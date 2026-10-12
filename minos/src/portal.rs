use anyhow::{Context, Result};
use serde_json::{json, Value};

pub struct Portal {
    client: reqwest::Client,
    url: String,
}

impl Portal {
    pub fn new(url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.to_string(),
        }
    }

    async fn query(&self, query: &str, variables: Option<Value>) -> Result<Value> {
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
        let val: Value = resp.json().await.context("portal response parse failed")?;
        if let Some(errors) = val.get("errors") {
            anyhow::bail!("GraphQL errors: {errors}");
        }
        Ok(val["data"].clone())
    }

    pub async fn instances(&self) -> Result<Vec<Instance>> {
        let q = r#"
            query {
                instances {
                    identifier label description
                    expiration { unixMilli }
                    payoutUpperbound { unixMilli }
                    vendorExpiration { unixMilli }
                    registry {
                        treasuryScript { scriptHash scriptReference { transaction index } }
                        vendorScript   { scriptHash scriptReference { transaction index } }
                    }
                }
            }
        "#;
        let data = self.query(q, None).await?;
        let arr = data["instances"]
            .as_array()
            .context("instances: expected array")?;
        arr.iter().map(parse_instance).collect()
    }

    pub async fn instance_by_id(&self, id: &str) -> Result<Instance> {
        let q = r#"
            query InstanceById($id: String!) {
                instanceById(ID: $id) {
                    identifier label description
                    expiration { unixMilli }
                    payoutUpperbound { unixMilli }
                    vendorExpiration { unixMilli }
                    registry {
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
        parse_instance(&data["instanceById"])
    }

    pub async fn new_transaction(&self, label: &str, cbor_hex: &str) -> Result<()> {
        let q = r#"
            mutation NewTx($tx: TransactionInput!) {
                newTransaction(tx: $tx) { ok }
            }
        "#;
        let vars = json!({ "tx": { "label": label, "cborHex": cbor_hex } });
        self.query(q, Some(vars)).await?;
        Ok(())
    }
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
    pub treasury_script: ScriptInfo,
    pub vendor_script: ScriptInfo,
}

#[derive(Debug, Clone)]
pub struct AssetValue {
    /// Empty string for lovelace; "<policyId><assetName>" for native assets
    pub asset_id: String,
    pub quantity: i64,
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
}

impl std::fmt::Display for MilestoneStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MilestoneStatus::Active => write!(f, "Active"),
            MilestoneStatus::Paused => write!(f, "Paused"),
            MilestoneStatus::Withdrawn => write!(f, "Withdrawn"),
            MilestoneStatus::Cancelled => write!(f, "Cancelled"),
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
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

fn parse_instance(v: &Value) -> Result<Instance> {
    Ok(Instance {
        identifier: str(v, "identifier")?,
        label: str(v, "label")?,
        description: str(v, "description")?,
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
    let raw_status = v["status"].as_str().unwrap_or("Active");
    let status = match raw_status {
        "ACTIVE" | "Active" | "MATURED" | "Matured" => MilestoneStatus::Active,
        "PAUSED" | "Paused" => MilestoneStatus::Paused,
        "WITHDRAWN" | "Withdrawn" => MilestoneStatus::Withdrawn,
        "CANCELLED" | "Cancelled" => MilestoneStatus::Cancelled,
        other => {
            eprintln!("warn: unknown milestone status '{other}', treating as Active");
            MilestoneStatus::Active
        }
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
    Ok(ProjectOutput {
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
                            .as_i64()
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
    })
}

/// Normalize portal assetId to our internal format.
/// Portal may return "ada.lovelace" for ADA or "policyHex.assetNameHex" for tokens.
/// We use "" for ADA and "policyHexassetNameHex" (concatenated) for tokens.
fn normalize_asset_id(raw: &str) -> String {
    if raw.is_empty() || raw == "ada.lovelace" {
        return String::new();
    }
    if let Some(dot) = raw.find('.') {
        return format!("{}{}", &raw[..dot], &raw[dot + 1..]);
    }
    raw.to_string()
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
