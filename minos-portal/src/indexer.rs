/// Background indexer: polls the node (or Koios) for live UTxOs, and optionally
/// scans historical transactions via Koios for CIP-100 events.
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

use minos::types::VendorDatum;
use pallas_addresses::{
    Address as PallasAddr, ShelleyAddress, ShelleyDelegationPart, ShelleyPaymentPart,
};
use pallas_crypto::hash::Hash;

use crate::config::Config;
use crate::db::{Store, TrackedInstance};
use crate::koios::Koios;
use crate::node::{NodeSession, PortalNode};
use crate::parser;
use crate::types::{AssetAmount, EventType, Instance, TxOutRef, VendorUtxo};

const REGISTRY_ASSET_NAME: &str = "5245474953545259"; // hex("REGISTRY")

pub async fn run(
    cfg: Config,
    portal_node: Option<Arc<PortalNode>>,
    koios: Option<Arc<Koios>>,
    store: Arc<Store>,
) {
    loop {
        let tracked = match store.get_tracked_instances() {
            Ok(t) => t,
            Err(e) => {
                error!("get_tracked_instances: {e}");
                vec![]
            }
        };

        // Open exactly one node connection per poll cycle and reuse it for
        // all instance queries + the tip update.  This prevents the FD leak
        // that occurs when every helper opens its own NodeClient.
        let session_result = if let Some(ref pn) = portal_node {
            match pn.session().await {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!("tip fetch failed: {e}");
                    None
                }
            }
        } else {
            None
        };

        let mut session = session_result;

        for inst_cfg in &tracked {
            if let Err(e) =
                index_instance(inst_cfg, session.as_mut(), koios.as_deref(), &store).await
            {
                error!("index_instance {}: {e:#}", inst_cfg.id);
            }
        }

        // Update chain tip
        let tip_result: anyhow::Result<(u64, u64)> = if let Some(ref mut s) = session {
            s.tip().await
        } else if let Some(ref k) = koios {
            k.tip().await.map(|t| (t.abs_slot, t.block_height))
        } else {
            Ok((0, 0))
        };
        match tip_result {
            Ok((slot, height)) => {
                let _ = store.set_sync("last_tip_slot", &slot.to_string());
                let _ = store.set_sync("last_tip_height", &height.to_string());
            }
            Err(e) => warn!("tip fetch failed: {e}"),
        }

        // Close session cleanly — releases the acquire and sends done before
        // dropping the socket, so the node doesn't see an abrupt disconnect.
        if let Some(s) = session {
            s.close().await;
        }

        sleep(Duration::from_secs(cfg.poll_interval_secs)).await;
    }
}

async fn index_instance(
    inst_cfg: &TrackedInstance,
    mut session: Option<&mut NodeSession>,
    koios: Option<&Koios>,
    store: &Store,
) -> anyhow::Result<()> {
    // 1. Locate registry NFT UTxO
    //    - With node socket: use publish_tx#0 from stored instance record
    //    - With Koios: query asset UTxOs by policy ID
    let registry_utxo_opt = if let Some(s) = session.as_deref_mut() {
        if let Some(inst) = store.get_instance(&inst_cfg.id)? {
            let outrefs = vec![(inst.publish_tx.clone(), 0u64)];
            s.outref_utxos(&outrefs).await.ok().and_then(|mut v| {
                if v.is_empty() {
                    None
                } else {
                    Some(v.remove(0))
                }
            })
        } else {
            None
        }
    } else if let Some(k) = koios {
        k.asset_utxos(&inst_cfg.id, REGISTRY_ASSET_NAME)
            .await
            .ok()
            .and_then(|mut v| {
                if v.is_empty() {
                    None
                } else {
                    Some(v.remove(0))
                }
            })
    } else {
        None
    };

    // 2. Parse registry datum → script hashes
    let (treasury_hash, vendor_hash) = match registry_utxo_opt
        .as_ref()
        .and_then(|u| u.inline_datum.as_deref())
    {
        Some(hex) => match parse_script_hash_registry(hex) {
            Some(pair) => pair,
            None => {
                warn!("instance {}: bad ScriptHashRegistry datum", inst_cfg.id);
                match store.get_instance(&inst_cfg.id)? {
                    Some(inst) => (inst.treasury_hash, inst.vendor_hash),
                    None => {
                        warn!("instance {}: no datum, not in store", inst_cfg.id);
                        return Ok(());
                    }
                }
            }
        },
        None => match store.get_instance(&inst_cfg.id)? {
            Some(inst) => (inst.treasury_hash, inst.vendor_hash),
            None => {
                warn!("instance {}: registry NFT not found", inst_cfg.id);
                return Ok(());
            }
        },
    };

    // 3. Derive script addresses
    let network_id = if inst_cfg.network == "mainnet" {
        1u8
    } else {
        0u8
    };
    let treasury_addr = script_address_bech32(&treasury_hash, network_id)?;
    let vendor_addr = script_address_bech32(&vendor_hash, network_id)?;

    // 4. Load or init instance record
    let publish_tx = registry_utxo_opt
        .as_ref()
        .map(|u| u.tx_hash.clone())
        .or_else(|| {
            store
                .get_instance(&inst_cfg.id)
                .ok()
                .flatten()
                .map(|i| i.publish_tx.clone())
        })
        .unwrap_or_default();

    let mut instance = store
        .get_instance(&inst_cfg.id)?
        .unwrap_or_else(|| Instance {
            id: inst_cfg.id.clone(),
            label: String::new(),
            description: None,
            expiration_ms: 0,
            payout_upperbound_ms: 0,
            vendor_expiration_ms: 0,
            treasury_hash: treasury_hash.clone(),
            vendor_hash: vendor_hash.clone(),
            treasury_ref: None,
            vendor_ref: None,
            publish_tx: publish_tx.clone(),
            network: inst_cfg.network.clone(),
        });

    // 5. Index publish tx metadata once (only when Koios is available)
    if !publish_tx.is_empty() && !store.is_indexed(&publish_tx)? {
        if let Some(k) = koios {
            let metas = k.tx_metadata(&[publish_tx.as_str()]).await?;
            if let Some(meta) = metas.first() {
                if let Some(parsed) = parser::parse_metadata(&publish_tx, 0, 0, &meta.metadata) {
                    if parsed.event.event_type == EventType::Publish {
                        apply_publish_metadata(&mut instance, &parsed.event.payload);
                    }
                    store.append_event(&parsed.event)?;
                }
            }
            let tx_infos = k.tx_info(&[publish_tx.as_str()]).await?;
            if let Some(tx_info) = tx_infos.first() {
                for output in &tx_info.outputs {
                    if output.payment_addr == treasury_addr && instance.treasury_ref.is_none() {
                        instance.treasury_ref = Some(TxOutRef {
                            transaction: publish_tx.clone(),
                            index: output.tx_index,
                        });
                    }
                    if output.payment_addr == vendor_addr && instance.vendor_ref.is_none() {
                        instance.vendor_ref = Some(TxOutRef {
                            transaction: publish_tx.clone(),
                            index: output.tx_index,
                        });
                    }
                }
            }
        }
        store.mark_indexed(&publish_tx, 0)?;
    }

    // Set ref UTxOs from publish tx index positions when not already known
    if instance.treasury_ref.is_none() && !publish_tx.is_empty() {
        instance.treasury_ref = Some(TxOutRef {
            transaction: publish_tx.clone(),
            index: 1,
        });
    }
    if instance.vendor_ref.is_none() && !publish_tx.is_empty() {
        instance.vendor_ref = Some(TxOutRef {
            transaction: publish_tx.clone(),
            index: 2,
        });
    }

    instance.treasury_hash = treasury_hash;
    instance.vendor_hash = vendor_hash;
    store.upsert_instance(&instance)?;

    // Historical event scanning has been removed — chain-sync (sync.rs) handles
    // full replay from publish_slot so there is no need to poll Koios for tx history.

    // 6. Refresh live vendor UTxOs (node socket preferred, Koios fallback)
    let live_utxos = if let Some(s) = session {
        s.address_utxos(&vendor_addr).await?
    } else if let Some(k) = koios {
        k.address_utxos(&vendor_addr).await?
    } else {
        vec![]
    };

    let projects = store.projects_for_instance(&inst_cfg.id)?;
    for utxo in &live_utxos {
        let has_vendor_datum = utxo
            .inline_datum
            .as_deref()
            .map(|d| VendorDatum::from_cbor_hex(d).is_ok())
            .unwrap_or(false);
        if !has_vendor_datum {
            continue;
        }
        let project_id = match infer_project_id(&projects, utxo) {
            Some(id) => id,
            None => {
                warn!(
                    "could not infer project for UTxO {}#{}",
                    utxo.tx_hash, utxo.output_index
                );
                continue;
            }
        };
        if let Some(datum_hex) = &utxo.inline_datum {
            if let Ok(datum) = VendorDatum::from_cbor_hex(datum_hex) {
                if let Some(mut proj) = store.get_project(&project_id)? {
                    // Detect newly withdrawn payouts and record events so that
                    // derive_status() can return Withdrawn for those milestones.
                    let n_payouts = datum.payouts.len();
                    let n_milestones = proj.milestones.len();
                    if n_payouts < n_milestones {
                        let n_withdrawn = n_milestones - n_payouts;
                        let already: Vec<String> = store
                            .withdrawn_milestones_for_project(&project_id)
                            .unwrap_or_default();
                        if already.len() < n_withdrawn {
                            let mut ms_map = serde_json::Map::new();
                            for i in already.len()..n_withdrawn {
                                if let Some(ms) = proj.milestones.get(i) {
                                    ms_map.insert(
                                        ms.identifier.clone(),
                                        serde_json::Value::Bool(true),
                                    );
                                }
                            }
                            if !ms_map.is_empty() {
                                let ev = crate::types::Event {
                                    tx_hash: utxo.tx_hash.clone(),
                                    block_slot: 0,
                                    block_height: 0,
                                    event_type: crate::types::EventType::Withdraw,
                                    instance_id: inst_cfg.id.clone(),
                                    project_id: Some(project_id.clone()),
                                    payload: serde_json::json!({ "milestones": ms_map }),
                                };
                                let _ = store.append_event(&ev);
                            }
                        }
                    }
                    sync_maturation(&mut proj, &datum);
                    store.upsert_project(&proj)?;
                }
            }
        }
        let mut assets: Vec<AssetAmount> = utxo
            .assets
            .iter()
            .map(|a| AssetAmount {
                asset_id: format!("{}{}", a.policy_id, a.asset_name),
                quantity: a.quantity,
            })
            .collect();
        assets.insert(
            0,
            AssetAmount {
                asset_id: String::new(),
                quantity: utxo.lovelace,
            },
        );
        store.upsert_utxo(&VendorUtxo {
            tx_hash: utxo.tx_hash.clone(),
            output_index: utxo.output_index,
            project_id,
            address: utxo.address.clone(),
            datum_hex: utxo.inline_datum.clone().unwrap_or_default(),
            lovelace: utxo.lovelace,
            assets,
            slot: 0,
        })?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn apply_publish_metadata(instance: &mut Instance, body: &serde_json::Value) {
    if let Some(label) = body.get("label").map(parser::rejoined_string) {
        instance.label = label;
    }
    instance.description = body.get("description").map(parser::rejoined_string);
    instance.expiration_ms = parse_ts(body, "expiration");
    instance.payout_upperbound_ms = parse_ts(body, "payoutUpperbound");
    instance.vendor_expiration_ms = parse_ts(body, "vendorExpiration");
}

fn parse_ts(body: &serde_json::Value, key: &str) -> u64 {
    body.get(key)
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| v.as_u64())
        })
        .unwrap_or(0)
}

async fn enrich_project_from_fund_tx(
    fund_tx_hash: &str,
    vendor_addr: &str,
    proj: &mut crate::types::Project,
    koios: &Koios,
) {
    match koios.tx_info(&[fund_tx_hash]).await {
        Ok(infos) => {
            if let Some(tx_info) = infos.first() {
                for output in &tx_info.outputs {
                    if output.payment_addr == vendor_addr {
                        if let Some(datum_hex) = &output.inline_datum {
                            if let Ok(datum) = VendorDatum::from_cbor_hex(datum_hex) {
                                sync_maturation(proj, &datum);
                            }
                        }
                        break;
                    }
                }
            }
        }
        Err(e) => warn!("tx_info for fund tx {fund_tx_hash}: {e}"),
    }
}

pub fn sync_maturation(proj: &mut crate::types::Project, datum: &VendorDatum) {
    let n_ms = proj.milestones.len();
    let n_payouts = datum.payouts.len();

    if n_payouts >= n_ms {
        // Normal case: payouts align with milestones from the start.
        for (i, payout) in datum.payouts.iter().enumerate() {
            if let Some(ms) = proj.milestones.get_mut(i) {
                ms.maturation_ms = payout.maturation;
            }
        }
    } else {
        // Some payouts were withdrawn. The contract always withdraws the
        // earliest-matured Active payouts, which correspond to the first
        // n_withdrawn positions in the sorted milestone list.  Leave those
        // milestones' maturation_ms untouched and map the remaining payouts
        // to the milestones that still have on-chain payouts.
        let n_withdrawn = n_ms - n_payouts;
        for i in n_withdrawn..n_ms {
            if let Some(payout) = datum.payouts.get(i - n_withdrawn) {
                if let Some(ms) = proj.milestones.get_mut(i) {
                    ms.maturation_ms = payout.maturation;
                }
            }
        }
    }
}

fn infer_project_id(
    projects: &[crate::types::Project],
    utxo: &crate::koios::Utxo,
) -> Option<String> {
    let payout_count = utxo
        .inline_datum
        .as_deref()
        .and_then(|d| VendorDatum::from_cbor_hex(d).ok())
        .map(|d| d.payouts.len())
        .unwrap_or(0);
    infer_project_id_pub(projects, payout_count)
}

/// Infer project ID from payout count — used by both the poll indexer and the chain sync.
pub fn infer_project_id_pub(
    projects: &[crate::types::Project],
    payout_count: usize,
) -> Option<String> {
    if projects.len() == 1 {
        return Some(projects[0].id.clone());
    }
    projects
        .iter()
        .find(|p| p.milestones.len() == payout_count)
        .or_else(|| {
            projects
                .iter()
                .filter(|p| p.milestones.len() >= payout_count)
                .min_by_key(|p| p.milestones.len() - payout_count)
        })
        .map(|p| p.id.clone())
}

pub fn script_address_bech32(hash_hex: &str, network_id: u8) -> anyhow::Result<String> {
    let bytes: [u8; 28] = hex::decode(hash_hex)
        .map_err(|e| anyhow::anyhow!("hex: {e}"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("script hash not 28 bytes"))?;
    let hash: Hash<28> = bytes.into();
    let network = if network_id == 1 {
        pallas_addresses::Network::Mainnet
    } else {
        pallas_addresses::Network::Testnet
    };
    let addr = ShelleyAddress::new(
        network,
        ShelleyPaymentPart::Script(hash),
        ShelleyDelegationPart::Script(hash),
    );
    PallasAddr::from_bytes(&addr.to_vec())
        .map_err(|e| anyhow::anyhow!("addr: {e}"))?
        .to_bech32()
        .map_err(|e| anyhow::anyhow!("bech32: {e}"))
}

fn parse_script_hash_registry(datum_hex: &str) -> Option<(String, String)> {
    let bytes = hex::decode(datum_hex).ok()?;
    let mut pos = 0;
    let mut hashes = Vec::new();
    while pos + 2 < bytes.len() && hashes.len() < 2 {
        if bytes[pos] == 0x58 && bytes[pos + 1] == 0x1c {
            let start = pos + 2;
            let end = start + 28;
            if end <= bytes.len() {
                hashes.push(hex::encode(&bytes[start..end]));
                pos = end;
                continue;
            }
        }
        pos += 1;
    }
    if hashes.len() == 2 {
        Some((hashes[0].clone(), hashes[1].clone()))
    } else {
        None
    }
}
