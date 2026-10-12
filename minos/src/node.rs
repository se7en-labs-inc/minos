//! Direct Cardano node-to-client queries and submission through Pallas.
use anyhow::{ensure, Context, Result};
use pallas_addresses::Address;
use pallas_codec::{minicbor, utils::AnyUInt};
use pallas_crypto::hash::Hasher;
use pallas_network::{
    facades::NodeClient,
    miniprotocols::{
        handshake::{n2c, Confirmation},
        localstate::{self, queries_v16 as q},
        localtxsubmission::{primitives::ScriptRef, EraTx, Response},
        Point,
    },
    multiplexer::Bearer,
};
use pallas_traverse::wellknown::GenesisValues;
use std::{collections::BTreeSet, time::Duration};
use tokio::time::timeout;

use crate::koios::{AssetListItem, Koios, ProtocolParams, Tip, Utxo};

pub enum DataSource {
    Koios(Koios),
    Node(LocalNode),
}

impl DataSource {
    pub async fn tip(&self) -> Result<Tip> {
        match self {
            Self::Koios(k) => k.tip().await,
            Self::Node(n) => n.tip().await,
        }
    }
    pub async fn protocol_params(&self) -> Result<ProtocolParams> {
        match self {
            Self::Koios(k) => k.protocol_params().await,
            Self::Node(n) => n.protocol_params().await,
        }
    }
    pub async fn address_utxos(&self, address: &str) -> Result<Vec<Utxo>> {
        match self {
            Self::Koios(k) => k.address_utxos(address).await,
            Self::Node(n) => n.address_utxos(address).await,
        }
    }
    pub async fn asset_utxos(&self, policy_id: &str, asset_name: &str) -> Result<Vec<Utxo>> {
        match self {
            Self::Koios(k) => k.asset_utxos(policy_id, asset_name).await,
            Self::Node(_) => anyhow::bail!("asset-by-policy discovery is not supported by filtered node queries; supply explicit reference UTxOs or use Koios"),
        }
    }
    pub async fn utxo_info(&self, refs: &[(String, u64)]) -> Result<Vec<Utxo>> {
        match self {
            Self::Koios(k) => k.utxo_info(refs).await,
            Self::Node(n) => n.utxo_info(refs).await,
        }
    }
    pub async fn submit_tx(&self, cbor_hex: &str) -> Result<String> {
        match self {
            Self::Koios(k) => k.submit_tx(cbor_hex).await,
            Self::Node(n) => n.submit_tx(cbor_hex).await,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubmissionError {
    #[error("transaction {hash} rejected: {reason}")]
    Rejected { hash: String, reason: String },
    #[error("submission outcome unknown for transaction {hash}: {reason}; check its status before retrying")]
    Unknown { hash: String, reason: String },
}

pub struct LocalNode {
    socket_path: String,
    network_magic: u64,
    operation_timeout: Duration,
}

impl LocalNode {
    pub fn new(socket_path: String, network_magic: u64) -> Self {
        Self {
            socket_path,
            network_magic,
            operation_timeout: Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.operation_timeout = duration;
        self
    }

    pub async fn tip(&self) -> Result<Tip> {
        let mut s = self.session().await?;
        let result = s.tip().await;
        s.finish(result).await
    }
    pub async fn protocol_params(&self) -> Result<ProtocolParams> {
        let mut s = self.session().await?;
        let result = s.protocol_params().await;
        s.finish(result).await
    }
    pub async fn address_utxos(&self, bech32: &str) -> Result<Vec<Utxo>> {
        Address::from_bech32(bech32).context("invalid query address")?;
        let mut s = self.session().await?;
        let result = s.address_utxos(bech32).await;
        s.finish(result).await
    }
    pub async fn utxo_info(&self, refs: &[(String, u64)]) -> Result<Vec<Utxo>> {
        if parse_refs(refs)?.is_empty() {
            return Ok(vec![]);
        }
        let mut s = self.session().await?;
        let result = s.utxo_info(refs).await;
        s.finish(result).await
    }
    pub async fn submit_tx(&self, cbor_hex: &str) -> Result<String> {
        let bytes = hex::decode(cbor_hex).context("invalid transaction hex")?;
        let tx: pallas_primitives::conway::Tx =
            minicbor::decode(&bytes).context("invalid Conway transaction CBOR")?;
        let hash = Hasher::<256>::hash(tx.transaction_body.raw_cbor()).to_string();
        let mut s = self.session().await?;
        let result = async {
            ensure!(
                s.era == 6,
                "submission requires Conway (node era {})",
                s.era
            );
            let response = timeout(
                s.operation_timeout,
                s.client()?.submission().submit_tx(EraTx(6, bytes)),
            )
            .await;
            match response {
                Ok(Ok(Response::Accepted)) => Ok(hash.clone()),
                Ok(Ok(Response::Rejected(err))) => Err(SubmissionError::Rejected {
                    hash: hash.clone(),
                    reason: format!("{err:?}"),
                }
                .into()),
                other => Err(SubmissionError::Unknown {
                    hash: hash.clone(),
                    reason: format!("{other:?}"),
                }
                .into()),
            }
        }
        .await;
        // Cleanup is bounded and cannot replace a known acceptance or rejection.
        s.finish(result).await
    }

    pub async fn session(&self) -> Result<NodeSession> {
        let bearer = timeout(
            self.operation_timeout,
            Bearer::connect_unix(&self.socket_path),
        )
        .await
        .context("node connection timed out")?
        .with_context(|| format!("connect to node socket {}", self.socket_path))?;
        let mut s = NodeSession {
            client: Some(NodeClient::new(bearer)),
            era: 0,
            network_magic: self.network_magic,
            operation_timeout: self.operation_timeout,
            runtime: tokio::runtime::Handle::current(),
        };
        let result = timeout(self.operation_timeout, async {
            let confirmation = s
                .client()?
                .handshake()
                .handshake(n2c::VersionTable::v10_and_above(self.network_magic))
                .await
                .context("node handshake")?;
            ensure!(
                matches!(confirmation, Confirmation::Accepted(..)),
                "node handshake refused: {confirmation:?}"
            );
            s.query()?
                .acquire(None)
                .await
                .context("acquire node ledger state")?;
            s.era = q::get_current_era(s.query()?)
                .await
                .context("query current era")?;
            Ok(())
        })
        .await
        .context("node session setup timed out")
        .and_then(|r| r);
        match result {
            Ok(()) => Ok(s),
            Err(err) => s.finish(Err(err)).await,
        }
    }
}

/// Shared acquired ledger snapshot. Any protocol error invalidates the session.
/// Dropping or cancelling a session schedules multiplexer cancellation.
pub struct NodeSession {
    client: Option<NodeClient>,
    era: u16,
    network_magic: u64,
    operation_timeout: Duration,
    runtime: tokio::runtime::Handle,
}
impl NodeSession {
    fn client(&mut self) -> Result<&mut NodeClient> {
        self.client
            .as_mut()
            .context("node session closed after an earlier failure")
    }
    fn query(&mut self) -> Result<&mut localstate::Client> {
        Ok(self.client()?.statequery())
    }

    async fn checked<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            if let Some(client) = self.client.take() {
                client.abort().await;
            }
        }
        result
    }
    pub async fn tip(&mut self) -> Result<Tip> {
        let result = timeout(self.operation_timeout, async {
            let genesis = GenesisValues::from_magic(self.network_magic)
                .context("unknown network magic: no slot/time mapping available")?;
            let point = q::get_chain_point(self.query()?)
                .await
                .context("query chain point")?;
            let Point::Specific(abs_slot, _) = point else {
                anyhow::bail!("node is at origin; no chain tip available");
            };
            let block = q::get_chain_block_no(self.query()?)
                .await
                .context("query block number")?;
            Ok(Tip {
                block_no: u64::from(block.block_number),
                abs_slot,
                block_time: genesis.slot_to_wallclock(abs_slot),
            })
        })
        .await
        .context("node tip query timed out")
        .and_then(|r| r);
        self.checked(result).await
    }
    pub async fn protocol_params(&mut self) -> Result<ProtocolParams> {
        let result = timeout(self.operation_timeout, async {
            ensure!(
                self.era == 6,
                "protocol parameters require Conway (node era {})",
                self.era
            );
            let era = self.era;
            protocol_params(
                q::get_current_pparams(self.query()?, era)
                    .await
                    .context("query protocol parameters")?,
            )
        })
        .await
        .context("node protocol parameter query timed out")
        .and_then(|r| r);
        self.checked(result).await
    }
    pub async fn address_utxos(&mut self, bech32: &str) -> Result<Vec<Utxo>> {
        let address = Address::from_bech32(bech32).context("invalid query address")?;
        let era = self.era;
        let result = timeout(self.operation_timeout, async {
            decode_utxos(
                q::get_utxo_by_address(self.query()?, era, vec![address.to_vec().into()])
                    .await
                    .context("query address UTxOs")?,
            )
        })
        .await
        .context("node address query timed out")
        .and_then(|r| r);
        self.checked(result).await
    }
    pub async fn utxo_info(&mut self, refs: &[(String, u64)]) -> Result<Vec<Utxo>> {
        let inputs = parse_refs(refs)?;
        if inputs.is_empty() {
            return Ok(vec![]);
        }
        let era = self.era;
        let result = timeout(self.operation_timeout, async {
            decode_utxos(
                q::get_utxo_by_txin(self.query()?, era, inputs)
                    .await
                    .context("query UTxO references")?,
            )
        })
        .await
        .context("node reference query timed out")
        .and_then(|r| r);
        self.checked(result).await
    }
    pub async fn close(self) {
        self.finish(Ok(())).await.ok();
    }
    async fn finish<T>(mut self, result: Result<T>) -> Result<T> {
        // Pallas queues these messages; abort is still required to stop its tasks.
        let cleanup = async {
            if let Some(client) = self.client.as_mut() {
                if matches!(
                    client.submission().state(),
                    pallas_network::miniprotocols::localtxsubmission::State::Idle
                ) {
                    let _ = client.submission().terminate_gracefully().await;
                }
                if matches!(client.statequery().state(), localstate::State::Acquired) {
                    let _ = client.statequery().send_release().await;
                }
                if matches!(client.statequery().state(), localstate::State::Idle) {
                    let _ = client.statequery().send_done().await;
                }
            }
        };
        let _ = timeout(Duration::from_secs(1), cleanup).await;
        if let Some(client) = self.client.take() {
            client.abort().await;
        }
        result
    }
}
impl Drop for NodeSession {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            self.runtime.spawn(client.abort());
        }
    }
}
fn parse_refs(refs: &[(String, u64)]) -> Result<BTreeSet<q::TransactionInput>> {
    refs.iter()
        .map(|(hash, index)| {
            let bytes: [u8; 32] = hex::decode(hash)
                .context("invalid UTxO transaction hash")?
                .try_into()
                .map_err(|_| anyhow::anyhow!("UTxO transaction hash must be 32 bytes"))?;
            Ok(q::TransactionInput {
                transaction_id: bytes.into(),
                index: *index,
            })
        })
        .collect()
}

fn rational(value: &q::RationalNumber) -> Result<f64> {
    ensure!(
        value.denominator != 0,
        "protocol parameter has zero denominator"
    );
    Ok(value.numerator as f64 / value.denominator as f64)
}

fn protocol_params(p: q::ProtocolParam) -> Result<ProtocolParams> {
    let (major, _) = p.protocol_version.context("missing protocol version")?;
    let max_ex = p
        .max_tx_ex_units
        .as_ref()
        .context("missing transaction execution limits")?;
    let prices = p.execution_costs.context("missing execution prices")?;
    let models = p
        .cost_models_for_script_languages
        .context("missing cost models")?;
    let v3 = models.plutus_v3.context("missing PlutusV3 cost model")?;
    ensure!(!v3.is_empty(), "empty PlutusV3 cost model");
    let ref_price = p
        .minfee_refscript_cost_per_byte
        .context("missing reference script byte price")?;
    ensure!(
        ref_price.denominator != 0,
        "reference script price has zero denominator"
    );
    // The existing fee API uses integer lovelace/byte. Round up fractional prices.
    let ref_price = ref_price.numerator.div_ceil(ref_price.denominator);
    Ok(ProtocolParams::from_fields(
        p.minfee_a.context("missing minfee_a")?,
        p.minfee_b.context("missing minfee_b")?,
        p.max_transaction_size
            .context("missing maximum transaction size")?,
        p.collateral_percentage
            .context("missing collateral percentage")?,
        p.max_collateral_inputs
            .context("missing maximum collateral inputs")?,
        Some(serde_json::json!({ "PlutusV3": v3 })),
        rational(&prices.mem_price)?,
        rational(&prices.step_price)?,
        ref_price,
    )
    .with_evaluation_limits(major, max_ex.mem, max_ex.steps))
}

fn uint(n: &AnyUInt) -> u64 {
    match n {
        AnyUInt::MajorByte(v) | AnyUInt::U8(v) => u64::from(*v),
        AnyUInt::U16(v) => u64::from(*v),
        AnyUInt::U32(v) => u64::from(*v),
        AnyUInt::U64(v) => *v,
    }
}

fn decode_utxos(map: q::UTxOByAddress) -> Result<Vec<Utxo>> {
    map.iter()
        .map(|(key, output)| {
            let (address, value, datum, script) = match output {
                q::TransactionOutput::Current(o) => (
                    &o.address,
                    &o.amount,
                    o.inline_datum.as_ref(),
                    o.script_ref.as_ref(),
                ),
                q::TransactionOutput::Legacy(o) => (&o.address, &o.amount, None, None),
            };
            let address = Address::from_bytes(address)
                .context("decode UTxO address")?
                .to_bech32()
                .context("encode UTxO address")?;
            let (coin, assets) = match value {
                q::Value::Coin(coin) => (uint(coin), vec![]),
                q::Value::Multiasset(coin, policies) => {
                    let mut assets = vec![];
                    for (policy, names) in policies.iter() {
                        for (name, qty) in names.iter() {
                            assets.push(AssetListItem {
                                policy_id: hex::encode(policy),
                                asset_name: hex::encode(&name[..]),
                                quantity: uint(qty).to_string(),
                            });
                        }
                    }
                    (uint(coin), assets)
                }
            };
            let inline_datum = match datum {
                Some(q::DatumOption::Data(data)) => Some(serde_json::json!({
                    "bytes": hex::encode(minicbor::to_vec(&data.0).context("encode inline datum")?)
                })),
                _ => None,
            };
            let reference_script = script
                .map(|wrapped| -> Result<_> {
                    let (kind, bytes) = match &wrapped.0 {
                        ScriptRef::NativeScript(s) => (
                            "native",
                            minicbor::to_vec(s).context("encode native reference script")?,
                        ),
                        ScriptRef::PlutusV1Script(s) => ("plutusV1", s.0.to_vec()),
                        ScriptRef::PlutusV2Script(s) => ("plutusV2", s.0.to_vec()),
                        ScriptRef::PlutusV3Script(s) => ("plutusV3", s.0.to_vec()),
                    };
                    Ok(serde_json::json!({ "type": kind, "bytes": hex::encode(bytes) }))
                })
                .transpose()?;
            Ok(Utxo {
                tx_hash: hex::encode(key.transaction_id),
                tx_index: uint(&key.index),
                address,
                value: coin.to_string(),
                asset_list: Some(assets),
                inline_datum,
                reference_script,
                datum_hash: None,
                output_cbor: Some(minicbor::to_vec(output).context("encode resolved output")?),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
