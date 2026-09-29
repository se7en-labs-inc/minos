/// Lightweight Koios client — only what minos can't get from the portal:
/// wallet UTxOs (collateral/fee inputs), protocol params, tx submission.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub struct Koios {
    client: reqwest::Client,
    base_url: String,
    /// Optional bearer token for paid Koios tiers
    api_key: Option<String>,
}

impl Koios {
    pub fn new(base_url: &str, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    fn request(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let req = self.client.get(&url);
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }

    fn post_request(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let req = self.client.post(&url);
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }

    pub async fn tip(&self) -> Result<Tip> {
        let resp: Vec<Tip> = self
            .request("/tip")
            .send()
            .await?
            .json()
            .await
            .context("tip parse")?;
        resp.into_iter().next().context("empty tip response")
    }

    pub async fn protocol_params(&self) -> Result<ProtocolParams> {
        let resp: Vec<ProtocolParams> = self
            .request("/epoch_params")
            .send()
            .await?
            .json()
            .await
            .context("protocol_params parse")?;
        resp.into_iter().next().context("empty epoch_params response")
    }

    pub async fn address_utxos(&self, address: &str) -> Result<Vec<Utxo>> {
        #[derive(Serialize)]
        struct Body {
            _addresses: Vec<String>,
        }
        let resp: Vec<Utxo> = self
            .post_request("/address_utxos")
            .json(&serde_json::json!({ "_addresses": [address] }))
            .send()
            .await?
            .json()
            .await
            .context("address_utxos parse")?;
        Ok(resp)
    }

    /// Submit a signed transaction. Returns the tx hash.
    pub async fn submit_tx(&self, tx_cbor_hex: &str) -> Result<String> {
        let resp = self
            .post_request("/submittx")
            .header("Content-Type", "application/cbor")
            .body(hex::decode(tx_cbor_hex).context("invalid tx hex")?)
            .send()
            .await
            .context("submittx request")?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("submittx failed ({status}): {body}");
        }
        Ok(body.trim_matches('"').to_string())
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct Tip {
    pub block_no: u64,
    pub abs_slot: u64,
    pub block_time: u64,
}

#[derive(Debug, Deserialize)]
pub struct ProtocolParams {
    pub min_fee_a: u64,
    pub min_fee_b: u64,
    pub key_deposit: String,
    pub pool_deposit: String,
    pub max_tx_size: u64,
    pub coins_per_utxo_size: String,
    pub price_mem: f64,
    pub price_step: f64,
    pub max_tx_ex_mem: String,
    pub max_tx_ex_steps: String,
    pub collateral_percent: u64,
    pub max_collateral_inputs: u64,
}

impl ProtocolParams {
    pub fn coins_per_utxo_byte(&self) -> u64 {
        self.coins_per_utxo_size.parse().unwrap_or(4310)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Utxo {
    pub tx_hash: String,
    pub tx_index: u64,
    pub address: String,
    pub value: Vec<AssetValue>,
    pub inline_datum: Option<String>,
}

impl Utxo {
    pub fn lovelace(&self) -> u64 {
        self.value
            .iter()
            .find(|v| v.unit == "lovelace")
            .and_then(|v| v.quantity.parse().ok())
            .unwrap_or(0)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AssetValue {
    pub unit: String,
    pub quantity: String,
}
