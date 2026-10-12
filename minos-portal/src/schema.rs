/// async-graphql schema — compatible with the existing minos portal client.
use async_graphql::{Context, EmptySubscription, Object, Schema, SimpleObject};
use std::sync::Arc;

use minos::types::{PayoutStatus, VendorDatum};
use pallas_codec::minicbor;
use pallas_primitives::conway::Tx;
use pallas_primitives::Metadatum;

use crate::config::Config;
use crate::db::Store;
use crate::koios::Koios;
use crate::node::PortalNode;
use crate::types::MilestoneStatus;

pub type MinosSchema = Schema<Query, Mutation, EmptySubscription>;

pub fn build(
    store: Arc<Store>,
    koios: Option<Arc<Koios>>,
    node: Option<Arc<PortalNode>>,
    cfg: Config,
) -> MinosSchema {
    let mut builder = Schema::build(Query, Mutation, EmptySubscription)
        .data(store)
        .data(cfg);
    if let Some(k) = koios {
        builder = builder.data(k);
    }
    if let Some(n) = node {
        builder = builder.data(n);
    }
    builder.finish()
}

// ---------------------------------------------------------------------------
// GraphQL output types
// ---------------------------------------------------------------------------

#[derive(SimpleObject)]
struct DateTime {
    unix_milli: String,
}
impl DateTime {
    fn from_ms(ms: u64) -> Self {
        Self {
            unix_milli: ms.to_string(),
        }
    }
}

#[derive(SimpleObject)]
struct TxOutRef {
    transaction: String,
    index: u64,
}

#[derive(SimpleObject)]
struct ScriptInfo {
    script_hash: String,
    script_reference: Option<TxOutRef>,
}

#[derive(SimpleObject)]
struct Registry {
    treasury_script: ScriptInfo,
    vendor_script: ScriptInfo,
}

#[derive(SimpleObject)]
struct AssetAmount {
    asset_id: String,
    quantity: String,
}

#[derive(SimpleObject)]
struct ProjectOutput {
    transaction: String,
    index: u64,
    address: String,
    datum_hex: String,
    value: Vec<AssetAmount>,
}

#[derive(SimpleObject)]
struct Evidence {
    anchor_url: String,
    hash: Option<String>,
}

#[derive(SimpleObject)]
struct Completion {
    description: String,
    evidence: Vec<Evidence>,
}

#[derive(SimpleObject)]
struct Milestone {
    identifier: String,
    label: String,
    description: String,
    acceptance_criteria: String,
    requires_evidence: bool,
    maturation: DateTime,
    status: String,
    completions: Vec<Completion>,
}

#[derive(SimpleObject)]
struct Vendor {
    label: String,
}

#[derive(SimpleObject)]
struct Project {
    identifier: String,
    label: String,
    description: String,
    vendor: Vendor,
    milestones: Vec<Milestone>,
    unspent_output: Option<ProjectOutput>,
}

#[derive(SimpleObject)]
struct Instance {
    identifier: String,
    label: String,
    description: String,
    expiration: DateTime,
    payout_upperbound: DateTime,
    vendor_expiration: DateTime,
    registry: Registry,
    projects: Vec<Project>,
}

// ---------------------------------------------------------------------------
// Resolvers
// ---------------------------------------------------------------------------

pub struct Query;

#[Object]
impl Query {
    async fn instances(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Instance>> {
        let store = ctx.data::<Arc<Store>>()?;
        store
            .all_instances()
            .map_err(|e| e.to_string().into())
            .and_then(|insts| insts.iter().map(|i| build_instance(i, store)).collect())
    }

    async fn instance_by_id(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "ID")] id: String,
    ) -> async_graphql::Result<Option<Instance>> {
        let store = ctx.data::<Arc<Store>>()?;
        match store
            .get_instance(&id)
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        {
            Some(inst) => Ok(Some(build_instance(&inst, store)?)),
            None => Ok(None),
        }
    }
}

pub struct Mutation;

#[Object]
impl Mutation {
    async fn new_transaction(
        &self,
        ctx: &Context<'_>,
        tx: TransactionInput,
    ) -> async_graphql::Result<TransactionResult> {
        self.register_transaction(ctx, tx).await
    }

    /// Register authenticated transaction data without submitting to the chain.
    async fn register_transaction(
        &self,
        ctx: &Context<'_>,
        tx: TransactionInput,
    ) -> async_graphql::Result<TransactionResult> {
        let cfg = ctx.data::<Config>()?;
        let tx_hash = verified_registration(&tx.cbor_hex, &cfg.allowed_authors)
            .map_err(|e| async_graphql::Error::new(format!("{e:#}")))?;
        let store = ctx.data::<Arc<Store>>()?;
        let registered = crate::db::RegisteredTx {
            tx_hash: tx_hash.clone(),
            label: tx.label.clone(),
            cbor_hex: tx.cbor_hex.clone(),
            submitted_at_ms: chrono::Utc::now().timestamp_millis() as u64,
        };
        let instance = extract_publish_metadata_from_cbor(&tx.cbor_hex).map(|pub_info| {
            let network = cfg.network();
            crate::types::Instance {
                id: pub_info.instance_id.clone(),
                label: pub_info.label,
                description: pub_info.description,
                expiration_ms: pub_info.expiration_ms,
                payout_upperbound_ms: pub_info.payout_upperbound_ms,
                vendor_expiration_ms: pub_info.vendor_expiration_ms,
                treasury_hash: pub_info.treasury_hash.clone(),
                vendor_hash: pub_info.vendor_hash.clone(),
                treasury_ref: None,
                vendor_ref: None,
                publish_tx: tx_hash.clone(),
                network: network.to_string(),
            }
        });
        store
            .store_registered_tx(&registered, instance.as_ref())
            .map_err(|e| e.to_string())?;

        Ok(TransactionResult { ok: true })
    }

    /// Index an already-on-chain tx by hash. Fetches metadata + required signers
    /// from Koios, checks authz, and registers the instance if it's a publish event.
    /// Requires koios_url to be set in the portal config.
    async fn index_transaction(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "txHash")] tx_hash: String,
    ) -> async_graphql::Result<TransactionResult> {
        let cfg = ctx.data::<Config>()?;
        let store = ctx.data::<Arc<Store>>()?;
        let koios = ctx
            .data::<Arc<Koios>>()
            .map_err(|_| "indexTransaction requires koios_url to be configured")?;

        let infos = koios
            .tx_info(&[tx_hash.as_str()])
            .await
            .map_err(|e| e.to_string())?;
        let info = infos.first().ok_or("tx not found on chain")?;

        if !cfg.allowed_authors.is_empty() {
            // Decode required signers from the raw tx CBOR — Koios doesn't reliably
            // return required_signers in tx_info.
            let cborv = koios
                .tx_cbor(&[tx_hash.as_str()])
                .await
                .map_err(|e| e.to_string())?;
            let signers = cborv
                .first()
                .map(|c| extract_required_signers(&c.cbor))
                .unwrap_or_default();
            if !signers.iter().any(|s| cfg.allowed_authors.contains(s)) {
                return Err("no authorized transaction authors in the required signers".into());
            }
        }

        let metas = koios
            .tx_metadata(&[tx_hash.as_str()])
            .await
            .map_err(|e| e.to_string())?;
        if let Some(meta) = metas.first() {
            if let Some(parsed) = crate::parser::parse_metadata(
                &tx_hash,
                info.block_slot,
                info.block_height,
                &meta.metadata,
            ) {
                store
                    .append_event(&parsed.event)
                    .map_err(|e| e.to_string())?;
                if parsed.event.event_type == crate::types::EventType::Publish {
                    let network = cfg.network();
                    store
                        .register_instance(&parsed.event.instance_id, network)
                        .map_err(|e| e.to_string())?;
                    if let Some(hashes) = derive_script_hashes_from_metadata(
                        &parsed.event.payload,
                        &parsed.event.instance_id,
                    ) {
                        let label = parsed
                            .event
                            .payload
                            .get("label")
                            .map(crate::parser::rejoined_string)
                            .unwrap_or_default();
                        let description = parsed
                            .event
                            .payload
                            .get("description")
                            .map(crate::parser::rejoined_string);
                        let _ = store.upsert_instance(&crate::types::Instance {
                            id: parsed.event.instance_id.clone(),
                            label,
                            description,
                            expiration_ms: hashes.expiration_ms,
                            payout_upperbound_ms: hashes.payout_upperbound_ms,
                            vendor_expiration_ms: hashes.vendor_expiration_ms,
                            treasury_hash: hashes.treasury_hash.clone(),
                            vendor_hash: hashes.vendor_hash.clone(),
                            treasury_ref: None,
                            vendor_ref: None,
                            publish_tx: tx_hash.clone(),
                            network: network.to_string(),
                        });
                        tracing::info!(
                            "registered instance {} treasury={} vendor={} from indexTransaction",
                            parsed.event.instance_id,
                            hashes.treasury_hash,
                            hashes.vendor_hash
                        );
                    } else {
                        tracing::warn!(
                            "could not derive script hashes for instance {} from metadata",
                            parsed.event.instance_id
                        );
                    }
                }
                if let Some(proj) = parsed.project {
                    store.upsert_project(&proj).map_err(|e| e.to_string())?;
                }
            }
        }

        Ok(TransactionResult { ok: true })
    }
}

#[derive(async_graphql::InputObject)]
struct TransactionInput {
    label: String,
    cbor_hex: String,
}

#[derive(SimpleObject)]
struct TransactionResult {
    ok: bool,
}

// ---------------------------------------------------------------------------
// Builder helpers
// ---------------------------------------------------------------------------

fn build_instance(
    inst: &crate::types::Instance,
    store: &Arc<Store>,
) -> async_graphql::Result<Instance> {
    let projects_raw = store
        .projects_for_instance(&inst.id)
        .map_err(|e| e.to_string())?;
    let projects = projects_raw
        .iter()
        .map(|p| build_project(p, store))
        .collect::<async_graphql::Result<Vec<_>>>()?;
    Ok(Instance {
        identifier: inst.id.clone(),
        label: inst.label.clone(),
        description: inst.description.clone().unwrap_or_default(),
        expiration: DateTime::from_ms(inst.expiration_ms),
        payout_upperbound: DateTime::from_ms(inst.payout_upperbound_ms),
        vendor_expiration: DateTime::from_ms(inst.vendor_expiration_ms),
        registry: Registry {
            treasury_script: ScriptInfo {
                script_hash: inst.treasury_hash.clone(),
                script_reference: inst.treasury_ref.as_ref().map(|r| TxOutRef {
                    transaction: r.transaction.clone(),
                    index: r.index,
                }),
            },
            vendor_script: ScriptInfo {
                script_hash: inst.vendor_hash.clone(),
                script_reference: inst.vendor_ref.as_ref().map(|r| TxOutRef {
                    transaction: r.transaction.clone(),
                    index: r.index,
                }),
            },
        },
        projects,
    })
}

fn build_project(
    proj: &crate::types::Project,
    store: &Arc<Store>,
) -> async_graphql::Result<Project> {
    let utxo = store.get_utxo(&proj.id).map_err(|e| e.to_string())?;
    let completions_all = store
        .completions_for_project(&proj.id)
        .map_err(|e| e.to_string())?;
    let withdrawn = store
        .withdrawn_milestones_for_project(&proj.id)
        .map_err(|e| e.to_string())?;
    let now_ms = chrono::Utc::now().timestamp_millis() as u64;

    // Decode on-chain datum to find paused payout indices
    let paused_ids = utxo
        .as_ref()
        .map(|u| decode_paused_ids(&u.datum_hex, &proj.milestones))
        .unwrap_or_default();

    let milestones = proj
        .milestones
        .iter()
        .map(|ms| {
            let status = derive_status(ms, now_ms, &paused_ids, &withdrawn, utxo.is_none());
            let completions = completions_all
                .iter()
                .filter(|c| c.milestone_id == ms.identifier)
                .map(|c| Completion {
                    description: c.description.clone(),
                    evidence: c
                        .evidence
                        .iter()
                        .map(|e| Evidence {
                            anchor_url: e.anchor_url.clone(),
                            hash: e.hash.clone(),
                        })
                        .collect(),
                })
                .collect();
            Milestone {
                identifier: ms.identifier.clone(),
                label: ms.label.clone(),
                description: ms.description.clone().unwrap_or_default(),
                acceptance_criteria: ms.acceptance_criteria.clone().unwrap_or_default(),
                requires_evidence: ms.requires_evidence,
                maturation: DateTime::from_ms(ms.maturation_ms),
                status: status_str(status),
                completions,
            }
        })
        .collect();

    Ok(Project {
        identifier: proj.id.clone(),
        label: proj.label.clone(),
        description: proj.description.clone().unwrap_or_default(),
        vendor: Vendor {
            label: proj.vendor_label.clone(),
        },
        milestones,
        unspent_output: utxo.map(|u| ProjectOutput {
            transaction: u.tx_hash,
            index: u.output_index,
            address: u.address,
            datum_hex: u.datum_hex,
            value: u
                .assets
                .iter()
                .map(|a| AssetAmount {
                    asset_id: a.asset_id.clone(),
                    quantity: a.quantity.to_string(),
                })
                .collect(),
        }),
    })
}

// ---------------------------------------------------------------------------
// Status derivation
// ---------------------------------------------------------------------------

fn derive_status(
    ms: &crate::types::Milestone,
    now_ms: u64,
    paused_ids: &[String],
    withdrawn_ids: &[String],
    utxo_gone: bool,
) -> MilestoneStatus {
    if withdrawn_ids.contains(&ms.identifier) || utxo_gone {
        return MilestoneStatus::Withdrawn;
    }
    if paused_ids.contains(&ms.identifier) {
        return MilestoneStatus::Paused;
    }
    if ms.maturation_ms > 0 && now_ms >= ms.maturation_ms {
        MilestoneStatus::Matured
    } else {
        MilestoneStatus::Active
    }
}

fn status_str(s: MilestoneStatus) -> String {
    match s {
        MilestoneStatus::Active => "Active",
        MilestoneStatus::Matured => "Matured",
        MilestoneStatus::Paused => "Paused",
        MilestoneStatus::Withdrawn => "Withdrawn",
        MilestoneStatus::Cancelled => "Cancelled",
    }
    .to_string()
}

/// Decode VendorDatum to find which milestone identifiers have Paused status.
/// Payouts align with milestones by position from the fund event.
fn decode_paused_ids(datum_hex: &str, milestones: &[crate::types::Milestone]) -> Vec<String> {
    let datum = match VendorDatum::from_cbor_hex(datum_hex) {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    datum
        .payouts
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.status, PayoutStatus::Paused))
        .filter_map(|(i, _)| milestones.get(i).map(|ms| ms.identifier.clone()))
        .collect()
}

fn parse_u64_value(v: &serde_json::Value) -> Option<u64> {
    match v {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Parse a CIP-100 multisig script node. Handles camelCase and snake_case field
/// names, and SundaeSwap-style string references (e.g. `"pause": "reorganize"`).
fn parse_multisig_script(
    v: &serde_json::Value,
    permissions_root: Option<&serde_json::Value>,
) -> Option<minos::types::MultisigScript> {
    use minos::types::MultisigScript;

    if let Some(s) = v.as_str() {
        let target = permissions_root?.get(s)?;
        return parse_multisig_script(target, permissions_root);
    }

    if let Some(sig) = v.get("signature") {
        let hash_str = sig
            .get("keyHash")
            .or_else(|| sig.get("key_hash"))
            .and_then(|h| h.as_str())?;
        return Some(MultisigScript::Signature {
            key_hash: hex::decode(hash_str).ok()?,
        });
    }

    if let Some(arr) = v.get("allOf") {
        let scripts = arr
            .as_array()?
            .iter()
            .filter_map(|s| parse_multisig_script(s, permissions_root))
            .collect();
        return Some(MultisigScript::AllOf { scripts });
    }

    if let Some(arr) = v.get("anyOf") {
        let scripts = arr
            .as_array()?
            .iter()
            .filter_map(|s| parse_multisig_script(s, permissions_root))
            .collect();
        return Some(MultisigScript::AnyOf { scripts });
    }

    if let Some(at_least) = v.get("atLeast") {
        let required = parse_u64_value(at_least.get("required")?)?;
        let scripts = at_least
            .get("scripts")?
            .as_array()?
            .iter()
            .filter_map(|s| parse_multisig_script(s, permissions_root))
            .collect();
        return Some(MultisigScript::AtLeast { required, scripts });
    }

    if let Some(t) = v.get("before").and_then(|v| parse_u64_value(v)) {
        return Some(MultisigScript::Before { time: t });
    }
    if let Some(t) = v.get("after").and_then(|v| parse_u64_value(v)) {
        return Some(MultisigScript::After { time: t });
    }
    if let Some(hash_str) = v.get("script").and_then(|h| h.as_str()) {
        return Some(MultisigScript::Script {
            script_hash: hex::decode(hash_str).ok()?,
        });
    }

    None
}

struct DerivedHashes {
    treasury_hash: String,
    vendor_hash: String,
    expiration_ms: u64,
    payout_upperbound_ms: u64,
    vendor_expiration_ms: u64,
}

/// Re-derive treasury and vendor script hashes from the CIP-100 publish event body.
///
/// This is the correct approach for all instances: re-apply the permissions and
/// expiration parameters to the base scripts, exactly as the SundaeSwap portal
/// does. It does not rely on reference script outputs being present in the publish tx.
fn derive_script_hashes_from_metadata(
    body: &serde_json::Value,
    instance_id: &str,
) -> Option<DerivedHashes> {
    use minos::scripts::{
        parameterize_treasury, parameterize_vendor, script_hash, TreasuryPermissions,
        VendorPermissions,
    };

    let permissions = body.get("permissions")?;
    let get_perm = |key: &str| parse_multisig_script(permissions.get(key)?, Some(permissions));

    let treasury_perms = TreasuryPermissions {
        reorganize: get_perm("reorganize")?,
        sweep: get_perm("sweep")?,
        fund: get_perm("fund")?,
        disburse: get_perm("disburse")?,
    };
    let vendor_perms = VendorPermissions {
        pause: get_perm("pause")?,
        resume: get_perm("resume")?,
        modify: get_perm("modify")?,
    };

    let expiration_ms = body.get("expiration").and_then(|v| parse_u64_value(v))?;
    let payout_upperbound_ms = body
        .get("payoutUpperbound")
        .or_else(|| body.get("payout_upperbound"))
        .and_then(|v| parse_u64_value(v))?;
    let vendor_expiration_ms = body
        .get("vendorExpiration")
        .or_else(|| body.get("vendor_expiration"))
        .and_then(|v| parse_u64_value(v))?;

    let registry_token: [u8; 28] = hex::decode(instance_id).ok()?.try_into().ok()?;

    let treasury_cbor = parameterize_treasury(
        &registry_token,
        &treasury_perms,
        expiration_ms,
        payout_upperbound_ms,
    )
    .ok()?;
    let vendor_cbor =
        parameterize_vendor(&registry_token, &vendor_perms, vendor_expiration_ms).ok()?;

    Some(DerivedHashes {
        treasury_hash: hex::encode(script_hash(&treasury_cbor).as_ref()),
        vendor_hash: hex::encode(script_hash(&vendor_cbor).as_ref()),
        expiration_ms,
        payout_upperbound_ms,
        vendor_expiration_ms,
    })
}

struct PublishInfo {
    instance_id: String,
    label: String,
    description: Option<String>,
    treasury_hash: String,
    vendor_hash: String,
    expiration_ms: u64,
    payout_upperbound_ms: u64,
    vendor_expiration_ms: u64,
}

/// Decode a Conway tx CBOR, extract the CIP-100 publish event body, and
/// derive script hashes via metadata re-parameterization.
fn extract_publish_metadata_from_cbor(cbor_hex: &str) -> Option<PublishInfo> {
    let meta_json = extract_auxiliary_metadata_1694(cbor_hex)?;
    let parsed = crate::parser::parse_metadata("", 0, 0, &meta_json)?;
    if parsed.event.event_type != crate::types::EventType::Publish {
        return None;
    }
    let hashes =
        derive_script_hashes_from_metadata(&parsed.event.payload, &parsed.event.instance_id)?;
    let label = parsed
        .event
        .payload
        .get("label")
        .map(crate::parser::rejoined_string)
        .unwrap_or_default();
    let description = parsed
        .event
        .payload
        .get("description")
        .map(crate::parser::rejoined_string);
    Some(PublishInfo {
        instance_id: parsed.event.instance_id,
        label,
        description,
        treasury_hash: hashes.treasury_hash,
        vendor_hash: hashes.vendor_hash,
        expiration_ms: hashes.expiration_ms,
        payout_upperbound_ms: hashes.payout_upperbound_ms,
        vendor_expiration_ms: hashes.vendor_expiration_ms,
    })
}

/// Decode Conway tx CBOR and extract metadata label 1694 as a JSON value.
fn extract_auxiliary_metadata_1694(cbor_hex: &str) -> Option<serde_json::Value> {
    use pallas_codec::utils::KeepRaw;
    use pallas_primitives::alonzo::AuxiliaryData;

    let bytes = hex::decode(cbor_hex).ok()?;
    let tx: Tx<'_> = minicbor::decode(&bytes).ok()?;
    let aux_raw: KeepRaw<'_, AuxiliaryData> = Option::from(tx.auxiliary_data)?;

    let datum = match &*aux_raw {
        AuxiliaryData::Shelley(m) => m.get(&1694)?,
        AuxiliaryData::ShelleyMa(sma) => sma.transaction_metadata.get(&1694)?,
        AuxiliaryData::PostAlonzo(pa) => pa.metadata.as_ref()?.get(&1694)?,
    };

    Some(metadatum_to_json(datum))
}

fn metadatum_to_json(m: &Metadatum) -> serde_json::Value {
    match m {
        Metadatum::Map(kv) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in kv.iter() {
                let key = match k {
                    Metadatum::Text(s) => s.clone(),
                    Metadatum::Int(i) => i128::from(i.clone()).to_string(),
                    _ => continue,
                };
                obj.insert(key, metadatum_to_json(v));
            }
            serde_json::Value::Object(obj)
        }
        Metadatum::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(metadatum_to_json).collect())
        }
        Metadatum::Int(i) => {
            serde_json::json!(i128::from(i.clone()))
        }
        Metadatum::Bytes(b) => serde_json::Value::String(hex::encode(b.as_slice())),
        Metadatum::Text(s) => serde_json::Value::String(s.clone()),
    }
}

/// Extract required signer key hashes from a Conway tx CBOR hex string.
/// Returns ed25519 key hashes as lowercase hex strings.
fn extract_required_signers(cbor_hex: &str) -> Vec<String> {
    let bytes = match hex::decode(cbor_hex) {
        Ok(b) => b,
        Err(_) => return vec![],
    };
    let tx: Tx<'_> = match minicbor::decode(&bytes) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    tx.transaction_body
        .required_signers
        .as_ref()
        .map(|s| s.iter().map(|h| hex::encode(h.as_ref())).collect())
        .unwrap_or_default()
}

/// Authenticate the original body, and bind metadata to its signed hash.
/// This proves authorship only, not ledger validity or confirmation.
fn verified_registration(cbor_hex: &str, allowed: &[String]) -> anyhow::Result<String> {
    use anyhow::{ensure, Context as _};
    use pallas_codec::utils::Nullable;
    use pallas_crypto::{
        hash::Hasher,
        key::ed25519::{PublicKey, Signature},
    };
    let bytes = hex::decode(cbor_hex).context("invalid transaction hex")?;
    let mut decoder = minicbor::Decoder::new(&bytes);
    let tx: Tx<'_> = decoder.decode().context("invalid Conway transaction")?;
    ensure!(
        decoder.position() == bytes.len(),
        "trailing transaction bytes"
    );
    ensure!(
        tx.success,
        "cannot register a transaction marked script-invalid"
    );
    match (&tx.auxiliary_data, &tx.transaction_body.auxiliary_data_hash) {
        (Nullable::Some(aux), Some(expected)) => ensure!(
            Hasher::<256>::hash(aux.raw_cbor()) == *expected,
            "auxiliary data hash mismatch"
        ),
        (Nullable::Null | Nullable::Undefined, None) => {}
        _ => anyhow::bail!("auxiliary data and its body hash must both be present"),
    }
    let hash = Hasher::<256>::hash(tx.transaction_body.raw_cbor());
    if !allowed.is_empty() {
        let mut authorized = false;
        if let Some(witnesses) = &tx.transaction_witness_set.vkeywitness {
            for witness in witnesses.iter() {
                let key = PublicKey::try_from(witness.vkey.as_slice())
                    .context("invalid witness public key")?;
                let signature = Signature::try_from(witness.signature.as_slice())
                    .context("invalid witness signature")?;
                let key_hash = Hasher::<224>::hash(witness.vkey.as_slice()).to_string();
                if allowed.iter().any(|a| a.eq_ignore_ascii_case(&key_hash))
                    && key.verify(hash.as_ref(), &signature)
                {
                    authorized = true;
                }
            }
        }
        ensure!(
            authorized,
            "no valid witness from an authorized transaction author"
        );
    }
    Ok(hash.to_string())
}

#[cfg(test)]
mod registration_tests {
    use super::*;
    use pallas_crypto::{hash::Hasher, key::ed25519::SecretKey};

    // Public deterministic test key, unrelated to any wallet.
    fn fixture(fee: u64, signed_fee: Option<u64>, key: &SecretKey) -> String {
        fn body(fee: u64, key: &SecretKey) -> Vec<u8> {
            let mut e = minicbor::Encoder::new(Vec::new());
            e.map(4)
                .unwrap()
                .u8(0)
                .unwrap()
                .array(0)
                .unwrap()
                .u8(1)
                .unwrap()
                .array(0)
                .unwrap()
                .u8(2)
                .unwrap()
                .u64(fee)
                .unwrap()
                .u8(14)
                .unwrap()
                .array(1)
                .unwrap()
                .bytes(Hasher::<224>::hash(key.public_key().as_ref()).as_ref())
                .unwrap();
            e.into_writer()
        }
        let mut tx = vec![0x84];
        tx.extend(body(fee, key));
        let mut e = minicbor::Encoder::new(Vec::new());
        if let Some(signed_fee) = signed_fee {
            let hash = Hasher::<256>::hash(&body(signed_fee, key));
            e.map(1)
                .unwrap()
                .u8(0)
                .unwrap()
                .array(1)
                .unwrap()
                .array(2)
                .unwrap()
                .bytes(key.public_key().as_ref())
                .unwrap()
                .bytes(key.sign(hash).as_ref())
                .unwrap();
        } else {
            e.map(0).unwrap();
        }
        tx.extend(e.into_writer());
        tx.extend([0xf5, 0xf6]);
        hex::encode(tx)
    }
    fn author(key: &SecretKey) -> String {
        Hasher::<224>::hash(key.public_key().as_ref()).to_string()
    }

    #[test]
    fn claimed_author_requires_valid_signature_for_this_body() {
        let key = SecretKey::from([7; 32]);
        let allowed = vec![author(&key)];
        assert!(verified_registration(&fixture(0, None, &key), &allowed).is_err());
        assert!(verified_registration(&fixture(1, Some(0), &key), &allowed).is_err());
        assert!(verified_registration(&fixture(0, Some(0), &key), &["00".repeat(28)]).is_err());
        assert!(verified_registration(&fixture(0, Some(0), &key), &allowed).is_ok());
        assert!(verified_registration("00", &[]).is_err());
        assert!(verified_registration(&(fixture(0, Some(0), &key) + "00"), &allowed).is_err());
        let mut tx = hex::decode(fixture(0, Some(0), &key)).unwrap();
        tx.pop();
        tx.push(0xa0); // Unbound auxiliary metadata instead of null.
        assert!(verified_registration(&hex::encode(tx), &allowed)
            .unwrap_err()
            .to_string()
            .contains("auxiliary"));
    }

    #[tokio::test]
    async fn registration_is_authenticated_idempotent_and_never_submits() {
        let key = SecretKey::from([7; 32]);
        let cfg: Config =
            toml::from_str(&format!("allowed_authors = [\"{}\"]", author(&key))).unwrap();
        let store = Arc::new(Store::temporary());
        // If registration ever tries to submit, this configured node will fail.
        let node = Arc::new(PortalNode::new("/nonexistent/minos-test.sock".into(), 2));
        let schema = build(store.clone(), None, Some(node), cfg);
        let register = |cbor: String, label: &str| {
            async_graphql::Request::new(
                "mutation($tx: TransactionInput!) { newTransaction(tx: $tx) { ok } }",
            )
            .variables(async_graphql::Variables::from_json(serde_json::json!({
                "tx": {"label": label, "cborHex": cbor}
            })))
        };
        let bad = schema
            .execute(register(fixture(0, None, &key), "spoofed"))
            .await;
        assert!(!bad.errors.is_empty());
        assert!(store.get_registered_txs().unwrap().is_empty());
        for label in ["first", "retry"] {
            let response = schema
                .execute(register(fixture(0, Some(0), &key), label))
                .await;
            assert!(response.errors.is_empty(), "{:?}", response.errors);
        }
        let records = store.get_registered_txs().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].label, "first");
    }
}
