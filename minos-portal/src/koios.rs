use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

pub struct Koios {
    client: reqwest::Client,
    base_url: String,
}

impl Koios {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("Koios {path} returned {status}: {text}");
        }
        serde_json::from_str(&text).context("parse koios response")
    }

    // -----------------------------------------------------------------------
    // Tip
    // -----------------------------------------------------------------------

    pub async fn tip(&self) -> Result<Tip> {
        let v = self.post("tip", json!(null)).await?;
        let arr = v.as_array().context("tip: expected array")?;
        let t = arr.first().context("tip: empty")?;
        Ok(Tip {
            abs_slot: t["abs_slot"].as_u64().context("tip.abs_slot")?,
            block_height: t["block_no"].as_u64().context("tip.block_no")?,
            block_hash: t["hash"].as_str().unwrap_or("").to_string(),
        })
    }

    // -----------------------------------------------------------------------
    // Address UTxOs
    // -----------------------------------------------------------------------

    pub async fn address_utxos(&self, address: &str) -> Result<Vec<Utxo>> {
        let v = self
            .post(
                "address_utxos",
                json!({ "_addresses": [address], "_extended": true }),
            )
            .await?;
        let arr = v.as_array().context("address_utxos: expected array")?;
        arr.iter().map(parse_utxo).collect()
    }

    // -----------------------------------------------------------------------
    // Address transactions (for indexing event history)
    // -----------------------------------------------------------------------

    pub async fn address_txs(&self, address: &str, after_block: Option<u64>) -> Result<Vec<TxRef>> {
        let mut body = json!({ "_addresses": [address] });
        if let Some(block) = after_block {
            body["_after_block_height"] = json!(block);
        }
        let v = self.post("address_txs", body).await?;
        let arr = v.as_array().context("address_txs: expected array")?;
        arr.iter()
            .map(|t| {
                Ok(TxRef {
                    tx_hash: t["tx_hash"].as_str().context("tx_hash")?.to_string(),
                    block_height: t["block_height"].as_u64().unwrap_or(0),
                    block_slot: t["block_time"].as_u64().unwrap_or(0),
                })
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Transaction metadata (batch)
    // -----------------------------------------------------------------------

    pub async fn tx_metadata(&self, tx_hashes: &[&str]) -> Result<Vec<TxMetadata>> {
        if tx_hashes.is_empty() {
            return Ok(vec![]);
        }
        let v = self
            .post("tx_metadata", json!({ "_tx_hashes": tx_hashes }))
            .await?;
        let arr = v.as_array().context("tx_metadata: expected array")?;
        arr.iter()
            .map(|row| {
                Ok(TxMetadata {
                    tx_hash: row["tx_hash"].as_str().context("tx_hash")?.to_string(),
                    metadata: row["metadata"].clone(),
                })
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Transaction info (inputs + outputs, for UTxO lineage tracking)
    // -----------------------------------------------------------------------

    pub async fn tx_info(&self, tx_hashes: &[&str]) -> Result<Vec<TxInfo>> {
        if tx_hashes.is_empty() {
            return Ok(vec![]);
        }
        let v = self
            .post("tx_info", json!({ "_tx_hashes": tx_hashes }))
            .await?;
        let arr = v.as_array().context("tx_info: expected array")?;
        arr.iter().map(parse_tx_info).collect()
    }

    // -----------------------------------------------------------------------
    // Raw transaction CBOR
    // -----------------------------------------------------------------------

    pub async fn tx_cbor(&self, tx_hashes: &[&str]) -> Result<Vec<TxCbor>> {
        if tx_hashes.is_empty() {
            return Ok(vec![]);
        }
        let v = self
            .post("tx_cbor", json!({ "_tx_hashes": tx_hashes }))
            .await?;
        let arr = v.as_array().context("tx_cbor: expected array")?;
        arr.iter()
            .map(|row| {
                Ok(TxCbor {
                    tx_hash: row["tx_hash"].as_str().context("tx_hash")?.to_string(),
                    cbor: row["cbor"].as_str().context("cbor")?.to_string(),
                })
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Asset UTxOs (find registry NFT)
    // -----------------------------------------------------------------------

    pub async fn asset_utxos(&self, policy_id: &str, asset_name: &str) -> Result<Vec<Utxo>> {
        let v = self
            .post(
                "asset_utxos",
                json!({ "_asset_list": [[policy_id, asset_name]] }),
            )
            .await?;
        let arr = v.as_array().context("asset_utxos: expected array")?;
        arr.iter().map(parse_utxo).collect()
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Tip {
    pub abs_slot: u64,
    pub block_height: u64,
    pub block_hash: String,
}

#[derive(Debug, Clone)]
pub struct TxRef {
    pub tx_hash: String,
    pub block_height: u64,
    pub block_slot: u64,
}

#[derive(Debug, Clone)]
pub struct TxMetadata {
    pub tx_hash: String,
    /// Raw metadata value at key "1694" (may be null)
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct Utxo {
    pub tx_hash: String,
    pub output_index: u64,
    pub address: String,
    pub lovelace: u64,
    pub assets: Vec<UtxoAsset>,
    pub inline_datum: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UtxoAsset {
    pub policy_id: String,
    pub asset_name: String,
    pub quantity: u64,
}

#[derive(Debug, Clone)]
pub struct TxCbor {
    pub tx_hash: String,
    pub cbor: String,
}

#[derive(Debug, Clone)]
pub struct TxInfo {
    pub tx_hash: String,
    pub block_hash: String,
    pub block_slot: u64,
    pub block_height: u64,
    pub outputs: Vec<TxOutput>,
    /// Ed25519 key hashes required to sign this tx (hex, from tx body field 14).
    pub required_signers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TxOutput {
    pub payment_addr: String,
    pub tx_index: u64,
    pub lovelace: u64,
    pub assets: Vec<UtxoAsset>,
    pub inline_datum: Option<String>,
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

fn parse_utxo(v: &Value) -> Result<Utxo> {
    let lovelace = v["value"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v["value"].as_u64())
        .unwrap_or(0);

    let assets = v["asset_list"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|a| UtxoAsset {
                    policy_id: a["policy_id"].as_str().unwrap_or("").to_string(),
                    asset_name: a["asset_name"].as_str().unwrap_or("").to_string(),
                    quantity: a["quantity"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| a["quantity"].as_u64())
                        .unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Utxo {
        tx_hash: v["tx_hash"].as_str().context("utxo.tx_hash")?.to_string(),
        output_index: v["tx_index"].as_u64().context("utxo.tx_index")?,
        address: v["address"].as_str().unwrap_or("").to_string(),
        lovelace,
        assets,
        inline_datum: v["inline_datum"]["bytes"].as_str().map(str::to_string),
    })
}

fn parse_tx_info(v: &Value) -> Result<TxInfo> {
    let outputs = v["outputs"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|o| {
                    let lovelace = o["payment_addr"]["value"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| o["value"].as_u64())
                        .unwrap_or(0);
                    TxOutput {
                        payment_addr: o["payment_addr"]["bech32"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        tx_index: o["tx_index"].as_u64().unwrap_or(0),
                        lovelace,
                        assets: o["asset_list"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .map(|a| UtxoAsset {
                                        policy_id: a["policy_id"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        asset_name: a["asset_name"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        quantity: a["quantity"]
                                            .as_str()
                                            .and_then(|s| s.parse().ok())
                                            .or_else(|| a["quantity"].as_u64())
                                            .unwrap_or(0),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        inline_datum: o["inline_datum"]["bytes"].as_str().map(str::to_string),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let required_signers = v["required_signers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    Ok(TxInfo {
        tx_hash: v["tx_hash"]
            .as_str()
            .context("tx_info.tx_hash")?
            .to_string(),
        block_hash: v["block_hash"].as_str().unwrap_or("").to_string(),
        block_slot: v["absolute_slot"].as_u64().unwrap_or(0),
        block_height: v["block_height"].as_u64().unwrap_or(0),
        outputs,
        required_signers,
    })
}
