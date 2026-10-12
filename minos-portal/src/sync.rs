/// Chain-sync indexer: streams blocks from the local Cardano node starting from
/// the publish tx slot, extracting label-1694 metadata events and vendor-address
/// outputs for each tracked instance.
///
/// Two phases, one code path:
///   Catchup  — replay every block from publish_slot to current tip
///   Follow   — node pushes new blocks as they arrive; no timer needed
use std::sync::Arc;

use anyhow::{Context, Result};
use pallas_network::facades::NodeClient;
use pallas_network::miniprotocols::chainsync::NextResponse;
use pallas_network::miniprotocols::Point;
use pallas_primitives::alonzo::MetadatumLabel;
use pallas_traverse::MultiEraBlock;
use tracing::{debug, error, info, warn};

use crate::db::Store;
use crate::indexer::{apply_publish_metadata, script_address_bech32, sync_maturation};
use crate::koios::Koios;
use crate::parser;
use crate::types::EventType;

// ---------------------------------------------------------------------------
// Public entry point — spawned as a long-running tokio task per instance
// ---------------------------------------------------------------------------

pub async fn run(
    instance_id: String,
    publish_tx: String,
    socket_path: String,
    network_magic: u64,
    store: Arc<Store>,
    koios: Option<Arc<Koios>>,
) {
    loop {
        match sync_once(
            &instance_id,
            &publish_tx,
            &socket_path,
            network_magic,
            &store,
            koios.as_deref(),
        )
        .await
        {
            Ok(()) => {
                // Connection dropped cleanly (shouldn't happen in follow mode).
                warn!("chain sync for {instance_id}: connection closed, reconnecting...");
            }
            Err(e) => {
                error!("chain sync for {instance_id}: {e:#}, reconnecting in 10s");
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Single sync session
// ---------------------------------------------------------------------------

async fn sync_once(
    instance_id: &str,
    publish_tx: &str,
    socket_path: &str,
    network_magic: u64,
    store: &Store,
    koios: Option<&Koios>,
) -> Result<()> {
    // Resolve vendor address bytes for output filtering
    let vendor_addr_bytes = resolve_vendor_addr(instance_id, store)?;

    let start = resolve_start_point(instance_id, publish_tx, store, koios).await?;

    let mut client = NodeClient::connect(socket_path, network_magic)
        .await
        .context("connect to node socket")?;

    let (intersection, _tip) = client
        .chainsync()
        .find_intersect(vec![start.clone()])
        .await
        .context("find_intersect")?;

    if intersection.is_none() {
        warn!("instance {instance_id}: no intersection at {start:?}; node may not have that block yet");
    }

    info!("instance {instance_id}: chain sync started from {start:?}");

    let mut at_tip = false;
    let mut initial_rollback_done = false;
    loop {
        let next = if client.chainsync().has_agency() {
            client
                .chainsync()
                .request_next()
                .await
                .context("request_next")?
        } else {
            client
                .chainsync()
                .recv_while_must_reply()
                .await
                .context("recv_while_must_reply")?
        };

        match next {
            NextResponse::RollForward(block_cbor, _tip) => {
                let bytes = block_cbor.0.as_ref();
                let block = MultiEraBlock::decode(bytes).context("decode block")?;
                let slot = block.slot();
                let block_height = block.number();
                let block_hash = hex::encode(block.hash().as_ref());

                process_block(
                    instance_id,
                    &vendor_addr_bytes,
                    &block,
                    slot,
                    block_height,
                    store,
                )?;

                store.set_sync(&format!("sync_slot_{instance_id}"), &slot.to_string())?;
                store.set_sync(&format!("sync_hash_{instance_id}"), &block_hash)?;

                if slot % 10_000 == 0 {
                    info!("instance {instance_id}: catchup at slot {slot}");
                }
                debug!("instance {instance_id}: processed slot {slot}");
            }

            NextResponse::RollBackward(point, _tip) => {
                let rollback_slot = match &point {
                    Point::Specific(s, _) => *s,
                    Point::Origin => 0,
                };
                // The first RollBackward after find_intersect is always the
                // protocol confirming our start point — not a real chain rollback.
                if !initial_rollback_done {
                    initial_rollback_done = true;
                    debug!("instance {instance_id}: sync confirmed at slot {rollback_slot}");
                } else {
                    warn!("instance {instance_id}: rollback to slot {rollback_slot}");
                }
                store.rollback_events_after(instance_id, rollback_slot)?;
                if let Point::Specific(slot, hash_bytes) = &point {
                    store.set_sync(&format!("sync_slot_{instance_id}"), &slot.to_string())?;
                    store.set_sync(
                        &format!("sync_hash_{instance_id}"),
                        &hex::encode(hash_bytes),
                    )?;
                }
            }

            NextResponse::Await => {
                if !at_tip {
                    at_tip = true;
                    info!("instance {instance_id}: reached tip, following chain");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Block processing
// ---------------------------------------------------------------------------

fn process_block(
    instance_id: &str,
    vendor_addr_bytes: &[u8],
    block: &MultiEraBlock,
    slot: u64,
    block_height: u64,
    store: &Store,
) -> Result<()> {
    for tx in block.txs() {
        let tx_hash = hex::encode(tx.hash().as_ref());

        if store.is_indexed(&tx_hash).unwrap_or(false) {
            continue;
        }

        let touches_vendor = tx.outputs().iter().any(|o| {
            o.address()
                .map(|a| a.to_vec() == vendor_addr_bytes)
                .unwrap_or(false)
        });

        let meta_json = tx
            .metadata()
            .find(1694 as MetadatumLabel)
            .map(metadatum_to_json);

        if meta_json.is_none() && !touches_vendor {
            let _ = store.mark_indexed(&tx_hash, slot);
            continue;
        }

        if let Some(meta) = meta_json {
            match parser::parse_metadata(&tx_hash, slot, block_height, &meta) {
                Some(parsed) => {
                    info!(
                        "sync: slot {slot} tx {tx_hash:.16} event={:?} instance={} project={:?}",
                        parsed.event.event_type, parsed.event.instance_id, parsed.event.project_id
                    );
                    if parsed.event.instance_id == instance_id {
                        match parsed.event.event_type {
                            EventType::Publish => {
                                if let Some(mut inst) =
                                    store.get_instance(instance_id).ok().flatten()
                                {
                                    apply_publish_metadata(&mut inst, &parsed.event.payload);
                                    let _ = store.upsert_instance(&inst);
                                }
                            }
                            EventType::Fund => {
                                if let Some(ref proj) = parsed.project {
                                    store.upsert_project(proj)?;
                                    info!("sync: indexed project {}", proj.id);
                                }
                            }
                            _ => {}
                        }
                        let _ = store.append_event(&parsed.event);
                    }
                }
                None => {
                    info!("sync: slot {slot} tx {tx_hash:.16} has label-1694 metadata but failed to parse");
                }
            }
        }

        // Refresh vendor UTxO datum when an output lands at the vendor address
        if touches_vendor {
            refresh_vendor_utxo(instance_id, &tx_hash, vendor_addr_bytes, &tx, store);
        }

        let _ = store.mark_indexed(&tx_hash, slot);
    }
    Ok(())
}

fn refresh_vendor_utxo(
    instance_id: &str,
    tx_hash: &str,
    vendor_addr_bytes: &[u8],
    tx: &pallas_traverse::MultiEraTx,
    store: &Store,
) {
    use minos::types::VendorDatum;

    let projects = match store.projects_for_instance(instance_id) {
        Ok(p) => p,
        Err(_) => return,
    };

    for (idx, output) in tx.outputs().iter().enumerate() {
        let addr_matches = output
            .address()
            .map(|a| a.to_vec() == vendor_addr_bytes)
            .unwrap_or(false);
        if !addr_matches {
            continue;
        }

        let datum_hex = match output.datum() {
            Some(pallas_primitives::conway::DatumOption::Data(d)) => {
                hex::encode(pallas_codec::minicbor::to_vec(&d).unwrap_or_default())
            }
            _ => continue,
        };

        let datum = match VendorDatum::from_cbor_hex(&datum_hex) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let project_id = match crate::indexer::infer_project_id_pub(&projects, datum.payouts.len())
        {
            Some(id) => id,
            None => continue,
        };

        if let Ok(Some(mut proj)) = store.get_project(&project_id) {
            let n_payouts = datum.payouts.len();
            let n_milestones = proj.milestones.len();
            if n_payouts < n_milestones {
                let n_withdrawn = n_milestones - n_payouts;
                let already = store
                    .withdrawn_milestones_for_project(&project_id)
                    .unwrap_or_default();
                if already.len() < n_withdrawn {
                    let mut ms_map = serde_json::Map::new();
                    for i in already.len()..n_withdrawn {
                        if let Some(ms) = proj.milestones.get(i) {
                            ms_map.insert(ms.identifier.clone(), serde_json::Value::Bool(true));
                        }
                    }
                    if !ms_map.is_empty() {
                        let ev = crate::types::Event {
                            tx_hash: tx_hash.to_string(),
                            block_slot: 0,
                            block_height: 0,
                            event_type: EventType::Withdraw,
                            instance_id: instance_id.to_string(),
                            project_id: Some(project_id.clone()),
                            payload: serde_json::json!({ "milestones": ms_map }),
                        };
                        let _ = store.append_event(&ev);
                    }
                }
            }
            sync_maturation(&mut proj, &datum);
            let _ = store.upsert_project(&proj);
        }

        let address = output
            .address()
            .ok()
            .and_then(|a| pallas_addresses::Address::from_bytes(&a.to_vec()).ok())
            .and_then(|a| a.to_bech32().ok())
            .unwrap_or_default();
        let lovelace = output.lovelace_amount();
        let _ = store.upsert_utxo(&crate::types::VendorUtxo {
            tx_hash: tx_hash.to_string(),
            output_index: idx as u64,
            project_id,
            address,
            datum_hex,
            lovelace,
            assets: vec![crate::types::AssetAmount {
                asset_id: String::new(),
                quantity: lovelace,
            }],
            slot: 0,
        });
    }
}

// ---------------------------------------------------------------------------
// Start point resolution
// ---------------------------------------------------------------------------

async fn resolve_start_point(
    instance_id: &str,
    publish_tx: &str,
    store: &Store,
    koios: Option<&Koios>,
) -> Result<Point> {
    // Resume from last synced block if available
    let slot_key = format!("sync_slot_{instance_id}");
    let hash_key = format!("sync_hash_{instance_id}");
    if let (Some(s), Some(h)) = (store.get_sync(&slot_key)?, store.get_sync(&hash_key)?) {
        if let Ok(slot) = s.parse::<u64>() {
            let hash = hex::decode(&h).unwrap_or_default();
            info!("instance {instance_id}: resuming chain sync from slot {slot}");
            return Ok(Point::Specific(slot, hash));
        }
    }

    // Bootstrap: look up publish tx slot via Koios (one-time per instance)
    let pub_slot_key = format!("publish_slot_{instance_id}");
    let pub_hash_key = format!("publish_block_hash_{instance_id}");
    if let (Some(s), Some(h)) = (
        store.get_sync(&pub_slot_key)?,
        store.get_sync(&pub_hash_key)?,
    ) {
        if let Ok(slot) = s.parse::<u64>() {
            let hash = hex::decode(&h).unwrap_or_default();
            return Ok(Point::Specific(slot, hash));
        }
    }

    if let Some(k) = koios {
        let infos = k
            .tx_info(&[publish_tx])
            .await
            .context("koios tx_info for publish tx")?;
        if let Some(info) = infos.first() {
            let slot = info.block_slot;
            let hash_bytes = hex::decode(&info.block_hash).unwrap_or_default();
            store.set_sync(&pub_slot_key, &slot.to_string())?;
            store.set_sync(&pub_hash_key, &info.block_hash)?;
            info!("instance {instance_id}: bootstrapped publish slot {slot} from Koios");
            return Ok(Point::Specific(slot, hash_bytes));
        }
    }

    warn!("instance {instance_id}: no publish slot available, syncing from origin");
    Ok(Point::Origin)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_vendor_addr(instance_id: &str, store: &Store) -> Result<Vec<u8>> {
    let inst = store
        .get_instance(instance_id)?
        .with_context(|| format!("instance {instance_id} not in store"))?;
    let network_id = if inst.network == "mainnet" { 1u8 } else { 0u8 };
    let bech32 = script_address_bech32(&inst.vendor_hash, network_id)?;
    pallas_addresses::Address::from_bech32(&bech32)
        .context("parse vendor address")
        .map(|a| a.to_vec())
}

fn metadatum_to_json(m: &pallas_primitives::alonzo::Metadatum) -> serde_json::Value {
    use pallas_primitives::alonzo::Metadatum;
    match m {
        Metadatum::Map(kv) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in kv.iter() {
                let key = match k {
                    Metadatum::Text(s) => s.to_string(),
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
        Metadatum::Int(i) => serde_json::json!(i128::from(i.clone())),
        Metadatum::Bytes(b) => serde_json::Value::String(hex::encode(b.as_slice())),
        Metadatum::Text(s) => serde_json::Value::String(s.to_string()),
    }
}
