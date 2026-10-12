/// Lightweight Koios client — only what minos can't get from the portal:
/// wallet UTxOs (collateral/fee inputs), protocol params, tx submission.
use anyhow::{Context, Result};
use serde::Deserialize;

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
            .query(&[("limit", "1")])
            .send()
            .await?
            .json()
            .await
            .context("protocol_params parse")?;
        resp.into_iter()
            .next()
            .context("empty epoch_params response")
    }

    pub async fn asset_utxos(&self, policy_id: &str, asset_name: &str) -> Result<Vec<Utxo>> {
        let asset_list = format!("{policy_id}{asset_name}");
        let resp: Vec<Utxo> = self
            .post_request("/asset_utxos")
            .json(
                &serde_json::json!({ "_asset_list": [[policy_id, asset_name]], "_extended": true }),
            )
            .send()
            .await?
            .json()
            .await
            .context("asset_utxos parse")?;
        let _ = asset_list; // suppress unused warning
        Ok(resp)
    }

    pub async fn address_utxos(&self, address: &str) -> Result<Vec<Utxo>> {
        let resp: Vec<Utxo> = self
            .post_request("/address_utxos")
            .json(&serde_json::json!({ "_addresses": [address], "_extended": true }))
            .send()
            .await?
            .json()
            .await
            .context("address_utxos parse")?;
        Ok(resp)
    }

    /// Fetch UTxO details for specific (tx_hash, output_index) pairs.
    /// Koios format: `_utxo_refs` = ["txhash#index", ...]
    pub async fn utxo_info(&self, utxos: &[(String, u64)]) -> Result<Vec<Utxo>> {
        let refs: Vec<String> = utxos
            .iter()
            .map(|(hash, idx)| format!("{}#{}", hash, idx))
            .collect();
        let resp: Vec<Utxo> = self
            .post_request("/utxo_info")
            .json(&serde_json::json!({ "_utxo_refs": refs, "_extended": true }))
            .send()
            .await?
            .json()
            .await
            .context("utxo_info parse")?;
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

/// Protocol params — we only destructure what we actually use.
/// Fields that Koios may return as either string or integer are stored as `serde_json::Value`.
#[derive(Debug, Deserialize)]
pub struct ProtocolParams {
    pub min_fee_a: u64,
    pub min_fee_b: u64,
    pub max_tx_size: u64,
    pub collateral_percent: u64,
    pub max_collateral_inputs: u64,
    /// Raw cost_models JSON from Koios (contains PlutusV1/V2/V3 sub-objects)
    pub cost_models: Option<serde_json::Value>,
    /// Catch-all for remaining fields so serde doesn't fail on unknown/mixed-type fields
    #[serde(flatten)]
    _extra: serde_json::Value,
}

impl ProtocolParams {
    /// Construct from typed protocol parameters queried through Pallas.
    #[allow(clippy::too_many_arguments)]
    pub fn from_fields(
        min_fee_a: u64,
        min_fee_b: u64,
        max_tx_size: u64,
        collateral_percent: u64,
        max_collateral_inputs: u64,
        cost_models: Option<serde_json::Value>,
        price_mem: f64,
        price_step: f64,
        ref_script_cost_per_byte: u64,
    ) -> Self {
        Self {
            min_fee_a,
            min_fee_b,
            max_tx_size,
            collateral_percent,
            max_collateral_inputs,
            cost_models,
            _extra: serde_json::json!({
                "price_mem": price_mem,
                "price_step": price_step,
                "min_fee_ref_script_cost_per_byte": ref_script_cost_per_byte,
            }),
        }
    }

    /// Execution price per memory unit (lovelace per mem unit).
    pub fn price_mem(&self) -> f64 {
        let v = &self._extra["price_mem"];
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0.0577)
    }

    /// Execution price per CPU step (lovelace per step).
    pub fn price_step(&self) -> f64 {
        let v = &self._extra["price_step"];
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0.0000721)
    }

    /// Minimum fee per reference script byte (lovelace).
    pub fn ref_script_cost_per_byte(&self) -> u64 {
        let v = &self._extra["min_fee_ref_script_cost_per_byte"];
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(15)
    }

    /// Extract PlutusV3 cost model as Vec<i64>.
    /// Koios may return it as an ordered array OR as a `{"key": value}` object.
    /// Array form: already in canonical order → use as-is.
    /// Object form: sort alphabetically by key name → Vec<i64>.
    pub fn v3_cost_model(&self) -> Option<Vec<i64>> {
        let models = self.cost_models.as_ref()?;
        let v3 = models.get("PlutusV3").or_else(|| models.get("plutusv3"))?;
        if let Some(arr) = v3.as_array() {
            // Array form: canonical ordering preserved
            let vals: Vec<i64> = arr
                .iter()
                .filter_map(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .collect();
            if vals.is_empty() {
                return None;
            }
            return Some(vals);
        }
        // Object form: sort alphabetically by key name
        let obj = v3.as_object()?;
        let mut pairs: Vec<(&str, i64)> = obj
            .iter()
            .filter_map(|(k, v)| {
                let n = v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))?;
                Some((k.as_str(), n))
            })
            .collect();
        pairs.sort_unstable_by_key(|(k, _)| *k);
        Some(pairs.into_iter().map(|(_, v)| v).collect())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Utxo {
    pub tx_hash: String,
    pub tx_index: u64,
    pub address: String,
    /// Lovelace amount as a string (Koios v1 current format)
    pub value: String,
    pub asset_list: Option<Vec<AssetListItem>>,
    pub inline_datum: Option<serde_json::Value>,
    /// Reference script info (Koios extended format: {bytes: "<hex>", ...})
    pub reference_script: Option<serde_json::Value>,
}

impl Utxo {
    pub fn lovelace(&self) -> u64 {
        self.value.parse().unwrap_or(0)
    }

    pub fn has_native_assets(&self) -> bool {
        self.asset_list.as_ref().map_or(false, |a| !a.is_empty())
    }

    /// Extract the inline datum CBOR hex (from Koios `inline_datum.bytes` field).
    pub fn datum_hex(&self) -> Option<String> {
        self.inline_datum
            .as_ref()
            .and_then(|d| d["bytes"].as_str())
            .map(|s| s.to_string())
    }

    /// Return the byte length of the reference script, or 0 if none.
    pub fn reference_script_size(&self) -> usize {
        let rs = match &self.reference_script {
            Some(v) => v,
            None => return 0,
        };
        // Koios and the Pallas adapter both expose the ledger script bytes.
        if let Some(hex) = rs["bytes"].as_str() {
            return hex.len() / 2;
        }
        0
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AssetListItem {
    pub policy_id: String,
    pub asset_name: String,
    pub quantity: String,
}
