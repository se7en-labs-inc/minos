use anyhow::{Context, Result};
use serde_json::{json, Value};

pub struct Portal {
    client: reqwest::Client,
    url: String,
}

impl Portal {
    pub fn new(url: &str) -> Self {
        Self {
            client: crate::http::client_builder()
                .build()
                .expect("HTTP client configuration"),
            url: url.to_string(),
        }
    }

    pub async fn query(&self, query: &str, variables: Option<Value>) -> Result<Value> {
        let body = match variables {
            Some(vars) => json!({ "query": query, "variables": vars }),
            None => json!({ "query": query }),
        };
        let resp = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("portal request failed")?;
        let val: Value = crate::http::json(resp)
            .await
            .context("portal response parse failed")?;
        if let Some(errors) = val.get("errors") {
            anyhow::bail!("GraphQL errors: {errors}");
        }
        val.get("data")
            .filter(|value| !value.is_null())
            .cloned()
            .context("portal response has no data")
    }

    pub async fn instances(&self) -> Result<Vec<Instance>> {
        let q = r#"
            query {
                apiVersion
                instances {
                    identifier label description confirmed
                    expiration { unixMilli }
                    payoutUpperbound { unixMilli }
                    vendorExpiration { unixMilli }
                    registry {
                        registryReference { transaction index }
                        treasuryScript { scriptHash scriptReference { transaction index } }
                        vendorScript   { scriptHash scriptReference { transaction index } }
                    }
                }
            }
        "#;
        let data = self.query(q, None).await?;
        anyhow::ensure!(
            data["apiVersion"] == 2,
            "unsupported portal API version; version 2 is required"
        );
        let arr = data["instances"]
            .as_array()
            .context("instances: expected array")?;
        arr.iter().map(parse_instance).collect()
    }

    pub async fn instance_by_id(&self, id: &str) -> Result<Instance> {
        let q = r#"
            query InstanceById($id: String!) {
                apiVersion
                instanceById(ID: $id) {
                    identifier label description confirmed
                    expiration { unixMilli }
                    payoutUpperbound { unixMilli }
                    vendorExpiration { unixMilli }
                    registry {
                        registryReference { transaction index }
                        treasuryScript { scriptHash scriptReference { transaction index } }
                        vendorScript   { scriptHash scriptReference { transaction index } }
                    }
                    projects {
                        identifier label description
                        vendor { label }
                        milestones {
                            identifier label description acceptanceCriteria
                            maturation { unixMilli }
                            status
                            completions { description evidence { anchorUrl hash } }
                        }
                        unspentOutput {
                            transaction index address datumHex
                            value { assetId quantity }
                        }
                    }
                }
            }
        "#;
        let vars = json!({ "id": id });
        let data = self.query(q, Some(vars)).await?;
        anyhow::ensure!(
            data["apiVersion"] == 2,
            "unsupported portal API version; version 2 is required"
        );
        let instance = parse_instance(&data["instanceById"])?;
        anyhow::ensure!(
            instance.confirmed,
            "instance is registered but its publication is not confirmed"
        );
        Ok(instance)
    }

    /// Uses the explicit registration-only mutation. Never fall back to a remote
    /// newTransaction implementation, which may also submit to the chain.
    pub async fn register_transaction(&self, label: &str, cbor_hex: &str) -> Result<()> {
        let q = r#"
            mutation NewTx($tx: TransactionInput!) {
                registerTransaction(tx: $tx) { ok }
            }
        "#;
        let vars = json!({ "tx": { "label": label, "cborHex": cbor_hex } });
        let data = self.query(q, Some(vars)).await?;
        anyhow::ensure!(
            data["registerTransaction"]["ok"].as_bool() == Some(true),
            "portal did not confirm registration"
        );
        Ok(())
    }

    /// Explicit opt-in to the remote portal's submission/signing workflow.
    /// Acceptance by this API does not itself prove chain confirmation.
    pub async fn submit_transaction(&self, label: &str, cbor_hex: &str) -> Result<()> {
        let bytes = hex::decode(cbor_hex).context("invalid transaction hex")?;
        let tx: pallas_primitives::conway::Tx<'_> =
            pallas_codec::minicbor::decode(&bytes).context("invalid Conway transaction")?;
        let witnesses = tx
            .transaction_witness_set
            .vkeywitness
            .as_ref()
            .filter(|w| !w.is_empty())
            .context("transaction has no vkey witnesses; witness it on the airgap first")?;
        let hash = crate::tx::sign::tx_body_hash(&bytes)?;
        for witness in witnesses.iter() {
            let key: [u8; 32] = witness
                .vkey
                .as_slice()
                .try_into()
                .context("invalid witness public key")?;
            let signature: [u8; 64] = witness
                .signature
                .as_slice()
                .try_into()
                .context("invalid witness signature")?;
            let key = pallas_crypto::key::ed25519::PublicKey::from(key);
            let signature = pallas_crypto::key::ed25519::Signature::from(signature);
            anyhow::ensure!(
                key.verify(hash, &signature),
                "invalid witness over transaction body"
            );
        }
        let data = self
            .query(
                "mutation SubmitTx($tx: TransactionInput!) { newTransaction(tx: $tx) { ok } }",
                Some(json!({"tx": {"label": label, "cborHex": cbor_hex}})),
            )
            .await?;
        anyhow::ensure!(
            data["newTransaction"]["ok"].as_bool() == Some(true)
                || data["newTransaction"]["ok"].as_str() == Some("Ok"),
            "portal did not accept the witnessed transaction"
        );
        Ok(())
    }

    pub async fn fee_inputs(&self) -> Result<Vec<FeeInput>> {
        let data = self
            .query(
                "query { feeInputs { transaction index address reservedUntil } }",
                None,
            )
            .await?;
        serde_json::from_value(data["feeInputs"].clone()).context("decode portal fee inputs")
    }

    pub async fn server_timestamp(&self) -> Result<String> {
        let data = self.query("query { now { unixMilli } }", None).await?;
        let millis: i64 = data["now"]["unixMilli"]
            .as_str()
            .context("missing server time")?
            .parse()?;
        Ok(chrono::DateTime::from_timestamp_millis(millis)
            .context("invalid server time")?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    }

    pub async fn evidence_upload_ticket(&self, input: Value) -> Result<String> {
        let data = self.query(
            "mutation UploadTicket($input: CreateEvidenceUploadTicketInput!) { createEvidenceUploadTicket(input: $input) { uploads { uploadUrl } } }",
            Some(json!({"input": input})),
        ).await?;
        let uploads = data["createEvidenceUploadTicket"]["uploads"]
            .as_array()
            .context("missing upload slots")?;
        anyhow::ensure!(uploads.len() == 1, "expected one upload slot");
        Ok(uploads[0]["uploadUrl"]
            .as_str()
            .context("missing upload URL")?
            .to_owned())
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeInput {
    pub transaction: String,
    pub index: u64,
    pub address: String,
    pub reserved_until: Option<String>,
}

impl FeeInput {
    pub fn available_at(&self, now: chrono::DateTime<chrono::Utc>) -> Result<bool> {
        self.reserved_until
            .as_deref()
            .filter(|v| !v.is_empty())
            .map_or(Ok(true), |value| {
                Ok(value
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .context("invalid fee input reservation timestamp")?
                    < now)
            })
    }
}

/// Intersect the advertised pool with current chain outputs; use chain values.
pub fn sponsored_utxos(
    rows: &[FeeInput],
    live: Vec<crate::koios::Utxo>,
    address: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<crate::koios::Utxo>> {
    let mut allowed = std::collections::BTreeSet::new();
    for row in rows {
        if row.address == address && row.available_at(now)? {
            allowed.insert((row.transaction.as_str(), row.index));
        }
    }
    Ok(live
        .into_iter()
        .filter(|u| {
            u.address == address
                && !u.has_native_assets()
                && allowed.contains(&(u.tx_hash.as_str(), u.tx_index))
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ScriptInfo {
    pub script_hash: String,
    pub script_reference: Option<TxOutRef>,
}

#[derive(Debug, Clone)]
pub struct TxOutRef {
    pub transaction: String,
    pub index: u64,
}

#[derive(Debug, Clone)]
pub struct RegistryInfo {
    pub registry_reference: Option<TxOutRef>,
    pub treasury_script: ScriptInfo,
    pub vendor_script: ScriptInfo,
}

#[derive(Debug, Clone)]
pub struct AssetValue {
    /// Empty string for lovelace; "<policyId><assetName>" for native assets
    pub asset_id: String,
    pub quantity: u64,
}

#[derive(Debug, Clone)]
pub struct ProjectOutput {
    pub transaction: String,
    pub index: u64,
    pub address: String,
    pub datum_hex: String,
    pub value: Vec<AssetValue>,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub description: String,
    pub evidence: Vec<CompletionEvidence>,
}

#[derive(Debug, Clone)]
pub struct CompletionEvidence {
    pub anchor_url: String,
    pub hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Milestone {
    pub identifier: String,
    pub label: String,
    pub description: String,
    pub acceptance_criteria: String,
    pub maturation_ms: u64,
    pub status: MilestoneStatus,
    pub completions: Vec<Completion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MilestoneStatus {
    Active,
    Paused,
    Withdrawn,
    Cancelled,
    Unresolved,
    Unavailable,
}

impl std::fmt::Display for MilestoneStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MilestoneStatus::Active => write!(f, "Active"),
            MilestoneStatus::Paused => write!(f, "Paused"),
            MilestoneStatus::Withdrawn => write!(f, "Withdrawn"),
            MilestoneStatus::Cancelled => write!(f, "Cancelled"),
            MilestoneStatus::Unresolved => write!(f, "Unresolved"),
            MilestoneStatus::Unavailable => write!(f, "Unavailable"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub identifier: String,
    pub label: String,
    pub description: String,
    pub vendor_label: String,
    pub milestones: Vec<Milestone>,
    pub unspent_output: Option<ProjectOutput>,
}

#[derive(Debug, Clone)]
pub struct Instance {
    pub identifier: String,
    pub label: String,
    pub description: String,
    pub expiration_ms: u64,
    pub payout_upperbound_ms: u64,
    pub vendor_expiration_ms: u64,
    pub registry: RegistryInfo,
    pub projects: Vec<Project>,
    pub confirmed: bool,
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

fn parse_instance(v: &Value) -> Result<Instance> {
    Ok(Instance {
        identifier: str(v, "identifier")?,
        label: str(v, "label")?,
        description: str(v, "description")?,
        confirmed: v["confirmed"]
            .as_bool()
            .context("missing instance confirmation state")?,
        expiration_ms: posix_ms(v, "expiration")?,
        payout_upperbound_ms: posix_ms(v, "payoutUpperbound")?,
        vendor_expiration_ms: posix_ms(v, "vendorExpiration")?,
        registry: parse_registry(&v["registry"])?,
        projects: v["projects"]
            .as_array()
            .map(|arr| arr.iter().map(parse_project).collect::<Result<Vec<_>>>())
            .transpose()?
            .unwrap_or_default(),
    })
}

fn parse_registry(v: &Value) -> Result<RegistryInfo> {
    Ok(RegistryInfo {
        registry_reference: if v["registryReference"].is_null() {
            None
        } else {
            Some(TxOutRef {
                transaction: str(&v["registryReference"], "transaction")?,
                index: v["registryReference"]["index"]
                    .as_u64()
                    .context("registry reference index")?,
            })
        },
        treasury_script: parse_script(&v["treasuryScript"])?,
        vendor_script: parse_script(&v["vendorScript"])?,
    })
}

fn parse_script(v: &Value) -> Result<ScriptInfo> {
    Ok(ScriptInfo {
        script_hash: str(v, "scriptHash")?,
        script_reference: {
            let r = &v["scriptReference"];
            if r.is_null() {
                None
            } else {
                Some(TxOutRef {
                    transaction: str(r, "transaction")?,
                    index: r["index"].as_u64().context("scriptReference.index")?,
                })
            }
        },
    })
}

fn parse_project(v: &Value) -> Result<Project> {
    Ok(Project {
        identifier: str(v, "identifier")?,
        label: str(v, "label")?,
        description: str(v, "description")?,
        vendor_label: v["vendor"]["label"].as_str().unwrap_or("").to_string(),
        milestones: v["milestones"]
            .as_array()
            .map(|arr| arr.iter().map(parse_milestone).collect::<Result<Vec<_>>>())
            .transpose()?
            .unwrap_or_default(),
        unspent_output: {
            let o = &v["unspentOutput"];
            if o.is_null() {
                None
            } else {
                Some(parse_project_output(o)?)
            }
        },
    })
}

fn parse_milestone(v: &Value) -> Result<Milestone> {
    let raw_status = v["status"].as_str().context("missing milestone status")?;
    let status = match raw_status {
        "ACTIVE" | "Active" | "MATURED" | "Matured" => MilestoneStatus::Active,
        "PAUSED" | "Paused" => MilestoneStatus::Paused,
        "WITHDRAWN" | "Withdrawn" => MilestoneStatus::Withdrawn,
        "CANCELLED" | "Cancelled" => MilestoneStatus::Cancelled,
        "UNRESOLVED" | "Unresolved" => MilestoneStatus::Unresolved,
        "UNAVAILABLE" | "Unavailable" => MilestoneStatus::Unavailable,
        other => anyhow::bail!("unsupported milestone status '{other}'; update the client"),
    };
    Ok(Milestone {
        identifier: str(v, "identifier")?,
        label: str(v, "label")?,
        description: str(v, "description")?,
        acceptance_criteria: str(v, "acceptanceCriteria")?,
        maturation_ms: posix_ms(v, "maturation")?,
        status,
        completions: v["completions"]
            .as_array()
            .map(|arr| arr.iter().map(parse_completion).collect::<Result<Vec<_>>>())
            .transpose()?
            .unwrap_or_default(),
    })
}

fn parse_completion(v: &Value) -> Result<Completion> {
    Ok(Completion {
        description: str(v, "description")?,
        evidence: v["evidence"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|e| {
                        Ok(CompletionEvidence {
                            anchor_url: str(e, "anchorUrl")?,
                            hash: e["hash"].as_str().map(str::to_string),
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
    })
}

fn parse_project_output(v: &Value) -> Result<ProjectOutput> {
    let output = ProjectOutput {
        transaction: str(v, "transaction")?,
        index: v["index"].as_u64().context("output.index")?,
        address: str(v, "address")?,
        datum_hex: str(v, "datumHex")?,
        value: v["value"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|a| {
                        let qty_raw = &a["quantity"];
                        let quantity = qty_raw
                            .as_u64()
                            .or_else(|| qty_raw.as_str().and_then(|s| s.parse().ok()))
                            .context("value.quantity")?;
                        Ok(AssetValue {
                            asset_id: normalize_asset_id(&str(a, "assetId")?),
                            quantity,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
    };
    crate::tx::value::Value::from_quantities(
        output
            .value
            .iter()
            .map(|value| (value.asset_id.as_str(), value.quantity)),
    )?;
    Ok(output)
}

/// Normalize portal assetId to our internal format.
/// Portal may return "ada.lovelace" for ADA or "policyHex.assetNameHex" for tokens.
/// We use "" for ADA and "policyHexassetNameHex" (concatenated) for tokens.
fn normalize_asset_id(raw: &str) -> String {
    if raw.is_empty() || raw == "ada.lovelace" {
        return String::new();
    }
    if let Some(dot) = raw.find('.') {
        return format!("{}{}", &raw[..dot], &raw[dot + 1..]).to_ascii_lowercase();
    }
    raw.to_ascii_lowercase()
}

fn str(v: &Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .map(str::to_string)
        .with_context(|| format!("missing string field '{key}'"))
}

fn posix_ms(v: &Value, key: &str) -> Result<u64> {
    // unixMilli is returned as a JSON string by the portal
    let raw = &v[key]["unixMilli"];
    if let Some(n) = raw.as_u64() {
        return Ok(n);
    }
    raw.as_str()
        .and_then(|s| s.parse().ok())
        .with_context(|| format!("missing unixMilli on '{key}'"))
}

#[cfg(test)]
mod status_tests {
    use super::*;
    #[test]
    fn uncertain_statuses_remain_explicit_and_unknown_statuses_fail() {
        let mut milestone = json!({"identifier":"m-10","label":"ten","description":"","acceptanceCriteria":"","maturation":{"unixMilli":"0"},"status":"Unresolved","completions":[]});
        assert_eq!(
            parse_milestone(&milestone).unwrap().status,
            MilestoneStatus::Unresolved
        );
        milestone["status"] = "Unavailable".into();
        assert_eq!(
            parse_milestone(&milestone).unwrap().status,
            MilestoneStatus::Unavailable
        );
        milestone["status"] = "FutureStatus".into();
        assert!(parse_milestone(&milestone).is_err());
        milestone.as_object_mut().unwrap().remove("status");
        assert!(parse_milestone(&milestone).is_err());
    }
    #[test]
    fn full_unsigned_asset_range_is_preserved_and_malformed_values_fail() {
        let mut output = json!({"transaction":"01".repeat(32),"index":0,"address":"synthetic","datumHex":"00","value":[{"assetId":"","quantity":"18446744073709551615"},{"assetId":"ab".repeat(28),"quantity":"18446744073709551615"}]});
        assert_eq!(
            parse_project_output(&output).unwrap().value[1].quantity,
            u64::MAX
        );
        output["value"][0]["quantity"] = "-1".into();
        assert!(parse_project_output(&output).is_err());
        output["value"][0]["quantity"] = "1".into();
        output["value"][1]["assetId"] = "".into();
        assert!(parse_project_output(&output).is_err());
    }
}
