//! Bounded Koios replay. Hashes, CBOR and block positions must agree; history is
//! published only after every page is retrieved against an unchanged tip.
use crate::{
    chain::{self, Block, Point},
    db::Store,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub struct Koios {
    client: reqwest::Client,
    base_url: String,
}
pub struct Tip {
    pub abs_slot: u64,
    pub block_height: u64,
    pub hash: String,
}
pub struct TxInfo {
    pub tx_hash: String,
    pub block_hash: String,
    pub block_slot: u64,
    pub block_height: u64,
    pub index: u64,
}
pub struct TxCbor {
    pub tx_hash: String,
    pub cbor: String,
}
impl Koios {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: minos::http::client_builder()
                .build()
                .expect("HTTP client configuration"),
            base_url: base_url.trim_end_matches('/').into(),
        }
    }
    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        let response = self
            .client
            .post(format!("{}/{}", self.base_url, path))
            .json(&body)
            .send()
            .await
            .context("Koios request")?;
        minos::http::json(response)
            .await
            .with_context(|| format!("Koios {path}"))
    }
    pub async fn tip(&self) -> Result<Tip> {
        let value = self.post("tip", Value::Null).await?;
        let rows = value.as_array().context("tip must be an array")?;
        ensure!(rows.len() == 1, "tip must have exactly one row");
        let row = &rows[0];
        Ok(Tip {
            abs_slot: row["abs_slot"].as_u64().context("tip slot")?,
            block_height: row["block_no"].as_u64().context("tip height")?,
            hash: hash(&row["hash"])?,
        })
    }
    pub async fn tx_info(&self, hashes: &[&str]) -> Result<Vec<TxInfo>> {
        let value = self.post("tx_info", json!({"_tx_hashes": hashes})).await?;
        value
            .as_array()
            .context("tx_info array")?
            .iter()
            .map(|row| {
                Ok(TxInfo {
                    tx_hash: hash(&row["tx_hash"])?,
                    block_hash: hash(&row["block_hash"])?,
                    block_slot: row["absolute_slot"].as_u64().context("transaction slot")?,
                    block_height: row["block_height"].as_u64().context("transaction height")?,
                    index: row["tx_block_index"]
                        .as_u64()
                        .context("transaction block index")?,
                })
            })
            .collect()
    }
    pub async fn tx_cbor(&self, hashes: &[&str]) -> Result<Vec<TxCbor>> {
        let value = self.post("tx_cbor", json!({"_tx_hashes": hashes})).await?;
        value
            .as_array()
            .context("tx_cbor array")?
            .iter()
            .map(|row| {
                Ok(TxCbor {
                    tx_hash: hash(&row["tx_hash"])?,
                    cbor: row["cbor"].as_str().context("transaction CBOR")?.into(),
                })
            })
            .collect()
    }
    async fn confirmed(&self, requested: &str) -> Result<(TxInfo, Vec<u8>)> {
        let mut infos = self.tx_info(&[requested]).await?;
        let mut cbors = self.tx_cbor(&[requested]).await?;
        ensure!(
            infos.len() == 1 && cbors.len() == 1,
            "missing or duplicate transaction response"
        );
        let info = infos.remove(0);
        let raw = cbors.remove(0);
        ensure!(
            info.tx_hash == requested && raw.tx_hash == requested,
            "upstream returned a different transaction"
        );
        let bytes = hex::decode(raw.cbor)?;
        let tx = minos::metadata::envelope(&bytes)?;
        ensure!(
            pallas_crypto::hash::Hasher::<256>::hash(tx.transaction_body.raw_cbor()).to_string()
                == requested,
            "CBOR body hash differs from requested transaction"
        );
        Ok((info, bytes))
    }
    async fn credential_history(&self, credentials: &[&str]) -> Result<BTreeSet<String>> {
        let mut hashes = BTreeSet::new();
        let mut completed = false;
        for page in 0..10_000 {
            let response = self
                .client
                .post(format!(
                    "{}/credential_txs?order=tx_hash.asc&limit=1000&offset={}",
                    self.base_url,
                    page * 1000
                ))
                .json(&json!({"_payment_credentials": credentials, "_after_block_height": 0}))
                .send()
                .await?;
            let value: Value = minos::http::json(response).await?;
            let rows = value
                .as_array()
                .context("credential history must be an array")?;
            ensure!(rows.len() <= 1000, "history page exceeds requested limit");
            for row in rows {
                let tx = hash(&row["tx_hash"])?;
                ensure!(hashes.insert(tx), "duplicate/unstable history pagination");
            }
            if rows.len() < 1000 {
                completed = true;
                break;
            }
        }
        ensure!(
            completed,
            "history pagination limit reached; no partial state applied"
        );
        Ok(hashes)
    }
    pub async fn replay(&self, instance: &str, network: &str, store: &Store) -> Result<()> {
        let tip = self.tip().await?;
        let registry = self
            .post(
                "asset_utxos",
                json!({"_asset_list": [[instance, hex::encode(b"REGISTRY")]]}),
            )
            .await?;
        let rows = registry.as_array().context("registry UTxO array")?;
        ensure!(rows.len() == 1, "registry NFT has no unique live output");
        let publish = hash(&rows[0]["tx_hash"])?;
        let (info, bytes) = self.confirmed(&publish).await?;
        let (published, _) = chain::publication(&bytes, instance, network)?;
        let hashes = self
            .credential_history(&[instance, &published.treasury_hash, &published.vendor_hash])
            .await?;
        let mut transactions = Vec::new();
        transactions.push((info, bytes));
        for hash in hashes {
            if hash != publish {
                transactions.push(self.confirmed(&hash).await?);
            }
        }
        transactions.sort_by_key(|(info, _)| (info.block_slot, info.index));
        let mut blocks: BTreeMap<u64, Block> = BTreeMap::new();
        let mut positions = BTreeSet::new();
        for (info, bytes) in transactions {
            ensure!(
                positions.insert((info.block_slot, info.index)),
                "duplicate block transaction position"
            );
            let point = Point {
                slot: info.block_slot,
                hash: info.block_hash,
                height: info.block_height,
            };
            let block = blocks.entry(point.slot).or_insert_with(|| Block {
                point: point.clone(),
                transactions: vec![],
            });
            ensure!(
                block.point == point,
                "inconsistent block identity in history"
            );
            block.transactions.push(bytes);
        }
        let after = self.tip().await?;
        ensure!(
            tip.hash == after.hash && tip.abs_slot == after.abs_slot,
            "chain tip changed while fetching history; retry without changing state"
        );
        store.replace_chain(
            instance,
            &blocks.into_values().collect::<Vec<_>>(),
            &Point {
                slot: tip.abs_slot,
                hash: tip.hash,
                height: tip.block_height,
            },
        )
    }
}
fn hash(value: &Value) -> Result<String> {
    let text = value.as_str().context("missing hash")?;
    ensure!(
        text.len() == 64 && hex::decode(text).is_ok(),
        "invalid transaction/block hash"
    );
    Ok(text.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Query, response::IntoResponse, routing::post, Json, Router};
    use std::sync::{
        atomic::{AtomicU8, AtomicUsize, Ordering},
        Arc,
    };
    #[tokio::test]
    async fn full_replay_retries_failures_without_erasing_committed_state() {
        let (instance, bytes, _) = crate::chain::tests::publication_fixture();
        let tx_hash = minos::tx::sign::tx_body_hash(&bytes)
            .map(hex::encode)
            .unwrap();
        let mode = Arc::new(AtomicU8::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/tip", post({ let mode = mode.clone(); let calls = calls.clone(); move || { let mode = mode.clone(); let calls = calls.clone(); async move {
                let count = calls.fetch_add(1, Ordering::SeqCst);
                Json(json!([{"hash": if mode.load(Ordering::SeqCst)==2 && count % 2 == 1 { "bb".repeat(32) } else { "aa".repeat(32) }, "abs_slot": 200, "block_no": 2}]))
            }}}))
            .route("/asset_utxos", post({ let hash = tx_hash.clone(); move || { let hash = hash.clone(); async move { Json(json!([{"tx_hash":hash}])) }}}))
            .route("/tx_info", post({ let hash = tx_hash.clone(); move || { let hash = hash.clone(); async move { Json(json!([{"tx_hash":hash,"block_hash":"aa".repeat(32),"absolute_slot":100,"block_height":1,"tx_block_index":0}])) }}}))
            .route("/tx_cbor", post({ let hash = tx_hash.clone(); let cbor=hex::encode(bytes); move || { let hash=hash.clone(); let cbor=cbor.clone(); async move { Json(json!([{"tx_hash":hash,"cbor":cbor}])) }}}))
            .route("/credential_txs", post({ let hash=tx_hash.clone(); let mode=mode.clone(); move || { let hash=hash.clone(); let mode=mode.clone(); async move {
                if mode.load(Ordering::SeqCst)==1 { axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response() }
                else { Json(json!([{"tx_hash":hash}])).into_response() }
            }}}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let store = Store::temporary();
        let koios = Koios::new(&url);
        koios.replay(&instance, "preview", &store).await.unwrap();
        let expected = serde_json::to_value(store.projection(&instance).unwrap()).unwrap();
        for failure in [1, 2] {
            mode.store(failure, Ordering::SeqCst);
            calls.store(0, Ordering::SeqCst);
            assert!(koios.replay(&instance, "preview", &store).await.is_err());
            assert_eq!(
                serde_json::to_value(store.projection(&instance).unwrap()).unwrap(),
                expected
            );
        }
        mode.store(0, Ordering::SeqCst);
        koios.replay(&instance, "preview", &store).await.unwrap();
        assert_eq!(
            store.projection(&instance).unwrap().unwrap().events.len(),
            1
        );
        server.abort();
    }
    #[tokio::test]
    async fn credential_history_fetches_later_pages_and_rejects_duplicate_pages() {
        let duplicate = Arc::new(AtomicU8::new(0));
        let app = Router::new().route(
            "/credential_txs",
            post({
                let duplicate = duplicate.clone();
                move |Query(query): Query<BTreeMap<String, String>>| {
                    let duplicate = duplicate.clone();
                    async move {
                        let offset: usize = query["offset"].parse().unwrap();
                        let start = if duplicate.load(Ordering::SeqCst) == 1 {
                            0
                        } else {
                            offset
                        };
                        Json(Value::Array(
                            (start..(start + 1000).min(1001))
                                .map(|n| json!({"tx_hash":format!("{n:064x}")}))
                                .collect(),
                        ))
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let koios = Koios::new(&format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let history = koios.credential_history(&[&"01".repeat(28)]).await.unwrap();
        assert_eq!(history.len(), 1001);
        assert!(history.contains(&format!("{:064x}", 1000)));
        duplicate.store(1, Ordering::SeqCst);
        assert!(koios.credential_history(&[&"01".repeat(28)]).await.is_err());
        server.abort();
    }
}
