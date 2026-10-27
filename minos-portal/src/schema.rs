/// async-graphql schema — compatible with the existing minos portal client.
use async_graphql::{Context, EmptySubscription, Object, Schema, SimpleObject};
use std::sync::Arc;

use minos::types::{PayoutStatus, VendorDatum};
#[cfg(test)]
use pallas_codec::minicbor;

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
        .limit_depth(16)
        .limit_complexity(1000)
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
    registry_reference: Option<TxOutRef>,
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
    availability: String,
}

#[derive(SimpleObject)]
struct Instance {
    identifier: String,
    references_ready: bool,
    label: String,
    description: String,
    expiration: DateTime,
    payout_upperbound: DateTime,
    vendor_expiration: DateTime,
    registry: Registry,
    projects: Vec<Project>,
    confirmed: bool,
    observations: Vec<Observation>,
}

#[derive(SimpleObject)]
struct Observation {
    tx_hash: String,
    slot: u64,
    reason: String,
}

// ---------------------------------------------------------------------------
// Resolvers
// ---------------------------------------------------------------------------

pub struct Query;

#[Object]
impl Query {
    async fn api_version(&self) -> u32 {
        2
    }
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
        let authors = cfg.registration_authors().map_err(|e| e.to_string())?;
        let tx_hash = verified_registration(&tx.cbor_hex, authors)
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
                registry_ref: None,
                publish_tx: tx_hash.clone(),
                network: network.to_string(),
                confirmed: false,
            }
        });
        store
            .store_registered_tx(&registered, instance.as_ref())
            .map_err(|e| e.to_string())?;

        Ok(TransactionResult { ok: true })
    }

    /// Authorize tracking from a real witnessed transaction and queue history
    /// replay. Success acknowledges the request; chain sync confirms events.
    async fn index_transaction(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "txHash")] tx_hash: String,
    ) -> async_graphql::Result<TransactionResult> {
        let cfg = ctx.data::<Config>()?;
        cfg.registration_authors().map_err(|e| e.to_string())?;
        let store = ctx.data::<Arc<Store>>()?;
        let koios = ctx
            .data::<Arc<Koios>>()
            .map_err(|_| "indexTransaction requires koios_url")?;
        let cbors = koios
            .tx_cbor(&[&tx_hash])
            .await
            .map_err(|e| e.to_string())?;
        if cbors.len() != 1 || cbors[0].tx_hash != tx_hash {
            return Err("transaction not found or response mismatch".into());
        }
        let actual = verified_registration(&cbors[0].cbor, &cfg.allowed_authors)
            .map_err(|e| e.to_string())?;
        if actual != tx_hash {
            return Err("transaction body hash mismatch".into());
        }
        let bytes = hex::decode(&cbors[0].cbor).map_err(|e| e.to_string())?;
        let tx = minos::metadata::transaction(&bytes).map_err(|e| e.to_string())?;
        let metadata = minos::metadata::auxiliary_1694(&tx)
            .map_err(|e| e.to_string())?
            .ok_or("no treasury metadata")?;
        let instance = metadata["instance"]
            .as_str()
            .ok_or("missing instance")?
            .to_ascii_lowercase();
        if instance.len() != 56 || hex::decode(&instance).is_err() {
            return Err("invalid instance".into());
        }
        // The supervisor discovers this durable registration. Replaying an
        // entire history in the HTTP request can exceed the API deadline.
        store
            .register_instance(&instance, cfg.network())
            .map_err(|e| e.to_string())?;
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
        references_ready: inst.confirmed
            && inst.registry_ref.is_some()
            && inst.treasury_ref.is_some()
            && inst.vendor_ref.is_some(),
        label: inst.label.clone(),
        description: inst.description.clone().unwrap_or_default(),
        expiration: DateTime::from_ms(inst.expiration_ms),
        payout_upperbound: DateTime::from_ms(inst.payout_upperbound_ms),
        vendor_expiration: DateTime::from_ms(inst.vendor_expiration_ms),
        registry: Registry {
            registry_reference: inst.registry_ref.as_ref().map(|reference| TxOutRef {
                transaction: reference.transaction.clone(),
                index: reference.index,
            }),
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
        confirmed: inst.confirmed,
        observations: store
            .projection(&inst.id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .flat_map(|projection| projection.state.observations.into_values())
            .map(|record| Observation {
                tx_hash: record.tx_hash,
                slot: record.slot,
                reason: record.reason,
            })
            .collect(),
    })
}

fn build_project(
    proj: &crate::types::Project,
    store: &Arc<Store>,
) -> async_graphql::Result<Project> {
    let utxo = store
        .get_utxo(&proj.instance_id, &proj.id)
        .map_err(|e| e.to_string())?;
    let completions_all = store
        .completions_for_project(&proj.instance_id, &proj.id)
        .map_err(|e| e.to_string())?;
    let withdrawn = store
        .withdrawn_milestones_for_project(&proj.instance_id, &proj.id)
        .map_err(|e| e.to_string())?;
    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
    let snapshot = store
        .get_sync(&format!("availability_{}_{}", proj.instance_id, proj.id))
        .map_err(|e| e.to_string())?
        .map(|value| serde_json::from_str::<serde_json::Value>(&value))
        .transpose()
        .map_err(|e| e.to_string())?;
    let unavailable = utxo.as_ref().is_some_and(|output| {
        snapshot.as_ref().is_some_and(|snapshot| {
            snapshot["tx"] == output.tx_hash
                && snapshot["index"].as_u64() == Some(output.output_index)
                && snapshot["status"] != "Available"
        })
    });

    // Decode on-chain datum to find paused payout indices
    let paused_ids = utxo
        .as_ref()
        .map(|u| decode_paused_ids(&u.datum_hex, &u.payout_ids))
        .unwrap_or_default();

    let milestones = proj
        .milestones
        .iter()
        .map(|ms| {
            let associated = utxo
                .as_ref()
                .is_some_and(|u| u.payout_ids.iter().flatten().any(|id| id == &ms.identifier));
            let status = if associated && unavailable {
                MilestoneStatus::Unavailable
            } else {
                derive_status(ms, now_ms, &paused_ids, &withdrawn, !associated)
            };
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
        availability: if unavailable {
            "Unavailable"
        } else if utxo.is_some() {
            "Available"
        } else {
            "Unresolved"
        }
        .into(),
        unspent_output: utxo.filter(|_| !unavailable).map(|u| ProjectOutput {
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
    unresolved: bool,
) -> MilestoneStatus {
    if withdrawn_ids.contains(&ms.identifier) {
        return MilestoneStatus::Withdrawn;
    }
    if unresolved {
        return MilestoneStatus::Unresolved;
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
        MilestoneStatus::Unresolved => "Unresolved",
        MilestoneStatus::Unavailable => "Unavailable",
    }
    .to_string()
}

/// Decode VendorDatum to find which milestone identifiers have Paused status.
/// Payouts align with milestones by position from the fund event.
fn decode_paused_ids(datum_hex: &str, ids: &[Option<String>]) -> Vec<String> {
    let datum = match VendorDatum::from_cbor_hex(datum_hex) {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    datum
        .payouts
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.status, PayoutStatus::Paused))
        .filter_map(|(i, _)| ids.get(i).cloned().flatten())
        .collect()
}

fn derive_script_hashes_from_metadata(
    body: &serde_json::Value,
    instance_id: &str,
) -> Option<minos::metadata::InstanceScripts> {
    minos::metadata::instance_scripts(body, instance_id).ok()
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
    let bytes = hex::decode(cbor_hex).ok()?;
    let tx = minos::metadata::transaction(&bytes).ok()?;
    minos::metadata::auxiliary_1694(&tx).ok()?
}

/// Authenticate the original body, and bind metadata to its signed hash.
/// This proves authorship only, not ledger validity or confirmation.
fn verified_registration(cbor_hex: &str, allowed: &[String]) -> anyhow::Result<String> {
    use anyhow::{ensure, Context as _};
    let bytes = hex::decode(cbor_hex).context("invalid transaction hex")?;
    let tx = minos::metadata::transaction(&bytes)?;
    let authors = minos::metadata::witnessed_authors(&tx)?;
    ensure!(
        allowed.is_empty()
            || authors.iter().any(|author| allowed
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(author))),
        "no valid witness from an authorized transaction author"
    );
    Ok(pallas_crypto::hash::Hasher::<256>::hash(tx.transaction_body.raw_cbor()).to_string())
}

#[cfg(test)]
mod registration_tests {
    use super::*;
    use pallas_crypto::{hash::Hasher, key::ed25519::SecretKey};

    #[test]
    fn missing_output_is_unresolved_and_paused_ids_follow_surviving_mapping() {
        let milestone = crate::types::Milestone {
            identifier: "m-10".into(),
            label: "ten".into(),
            description: None,
            acceptance_criteria: None,
            maturation_ms: 1,
            requires_evidence: true,
        };
        assert_eq!(
            derive_status(&milestone, 100, &[], &[], true),
            MilestoneStatus::Unresolved
        );
        assert_eq!(
            derive_status(&milestone, 100, &[], &["m-10".into()], true),
            MilestoneStatus::Withdrawn
        );
        let datum = minos::types::VendorDatum {
            vendor: minos::types::MultisigScript::AnyOf { scripts: vec![] },
            payouts: vec![minos::types::Payout {
                maturation: 1,
                value: minos::types::AssetBundle(Default::default()),
                status: PayoutStatus::Paused,
            }],
        };
        let paused = decode_paused_ids(&hex::encode(datum.to_cbor_bytes()), &[Some("m-10".into())]);
        assert_eq!(paused, vec!["m-10"]);
        assert_eq!(
            derive_status(&milestone, 100, &paused, &[], false),
            MilestoneStatus::Paused
        );
        assert!(decode_paused_ids(&hex::encode(datum.to_cbor_bytes()), &[None]).is_empty());
    }

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
