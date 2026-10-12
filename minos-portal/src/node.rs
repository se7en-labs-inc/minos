/// Local-state-query backend via pallas-network (Cardano node Unix socket).
/// Replaces Koios for chain-tip and UTxO queries so we don't exhaust the
/// free-tier rate limit.
use anyhow::{Context, Result};
use pallas_addresses::Address as PallasAddr;
use pallas_codec::minicbor;
use pallas_codec::utils::AnyUInt;
use pallas_network::facades::NodeClient;
use pallas_network::miniprotocols::localstate::queries_v16::{
    self, DatumOption, TransactionInput, TransactionOutput, Value,
};
use pallas_network::miniprotocols::Point;
use std::collections::BTreeSet;

use crate::koios::{Utxo, UtxoAsset};

pub struct PortalNode {
    pub socket_path: String,
    pub network_magic: u64,
}

impl PortalNode {
    pub fn new(socket_path: String, network_magic: u64) -> Self {
        Self {
            socket_path,
            network_magic,
        }
    }

    /// Open a session that shares one connection across multiple LSQ queries.
    /// The caller must call `session.close().await` when done; that sends
    /// release + done so the node can clean up its side before the socket drops.
    pub async fn session(&self) -> Result<NodeSession> {
        let mut client = NodeClient::connect(&self.socket_path, self.network_magic)
            .await
            .context("connect to node socket")?;
        client
            .statequery()
            .send_acquire(None)
            .await
            .context("statequery send_acquire")?;
        client
            .statequery()
            .recv_while_acquiring()
            .await
            .context("statequery recv_while_acquiring")?;
        let era = queries_v16::get_current_era(client.statequery())
            .await
            .context("get_current_era")?;
        Ok(NodeSession { client, era })
    }

    /// Convenience wrapper: one connection for one tip query.
    pub async fn tip(&self) -> Result<(u64, u64)> {
        let mut s = self.session().await?;
        let result = s.tip().await;
        s.close().await;
        result
    }

    /// Convenience wrapper: one connection for one address_utxos query.
    pub async fn address_utxos(&self, bech32: &str) -> Result<Vec<Utxo>> {
        let mut s = self.session().await?;
        let result = s.address_utxos(bech32).await;
        s.close().await;
        result
    }

    /// Submit a signed Conway tx (raw CBOR hex) via local tx submission.
    /// Returns the tx hash on acceptance, or an error with the rejection reason.
    pub async fn submit_tx(&self, tx_cbor_hex: &str) -> Result<String> {
        use pallas_codec::minicbor;
        use pallas_crypto::hash::Hasher;
        use pallas_network::miniprotocols::localtxsubmission::{EraTx, Response};
        use pallas_primitives::conway::Tx;

        let tx_bytes = hex::decode(tx_cbor_hex).context("decode tx hex")?;
        let tx: Tx = minicbor::decode(&tx_bytes).context("decode tx cbor")?;
        let tx_hash = hex::encode(Hasher::<256>::hash(tx.transaction_body.raw_cbor()).as_ref());

        let mut client = NodeClient::connect(&self.socket_path, self.network_magic)
            .await
            .context("connect to node socket")?;

        let era_tx = EraTx(6, tx_bytes); // 6 = Conway
        let response = client
            .submission()
            .submit_tx(era_tx)
            .await
            .map_err(|e| anyhow::anyhow!("localtxsubmission: {e:?}"))?;

        match response {
            Response::Accepted => Ok(tx_hash),
            Response::Rejected(err) => anyhow::bail!("tx rejected: {err:?}"),
        }
    }

    /// Convenience wrapper: one connection for one outref_utxos query.
    pub async fn outref_utxos(&self, outrefs: &[(String, u64)]) -> Result<Vec<Utxo>> {
        let mut s = self.session().await?;
        let result = s.outref_utxos(outrefs).await;
        s.close().await;
        result
    }
}

// ---------------------------------------------------------------------------
// NodeSession — reusable single-connection LSQ session
// ---------------------------------------------------------------------------

/// Holds one open NodeClient connection with an active statequery acquire.
/// Use `session()` on `PortalNode` to create one; call `close()` when done.
pub struct NodeSession {
    client: NodeClient,
    era: u16,
}

impl NodeSession {
    pub async fn tip(&mut self) -> Result<(u64, u64)> {
        let point = queries_v16::get_chain_point(self.client.statequery())
            .await
            .context("get_chain_point")?;
        let block_no = queries_v16::get_chain_block_no(self.client.statequery())
            .await
            .context("get_chain_block_no")?;
        let slot = match point {
            Point::Specific(slot, _) => slot,
            Point::Origin => 0,
        };
        Ok((slot, block_no.block_number as u64))
    }

    pub async fn address_utxos(&mut self, bech32: &str) -> Result<Vec<Utxo>> {
        let addr_bytes: pallas_codec::utils::Bytes = PallasAddr::from_bech32(bech32)
            .with_context(|| format!("parse address {bech32}"))?
            .to_vec()
            .into();
        let addrs = vec![addr_bytes];
        let utxo_map = queries_v16::get_utxo_by_address(self.client.statequery(), self.era, addrs)
            .await
            .context("get_utxo_by_address")?;
        kv_to_utxos(utxo_map)
    }

    pub async fn outref_utxos(&mut self, outrefs: &[(String, u64)]) -> Result<Vec<Utxo>> {
        if outrefs.is_empty() {
            return Ok(vec![]);
        }
        let txins: BTreeSet<TransactionInput> = outrefs
            .iter()
            .filter_map(|(hash, idx)| {
                let bytes: [u8; 32] = hex::decode(hash).ok()?.try_into().ok()?;
                Some(TransactionInput {
                    transaction_id: bytes.into(),
                    index: *idx,
                })
            })
            .collect();
        let utxo_map = queries_v16::get_utxo_by_txin(self.client.statequery(), self.era, txins)
            .await
            .context("get_utxo_by_txin")?;
        kv_to_utxos(utxo_map)
    }

    /// Send release + done and drop the connection cleanly.
    pub async fn close(mut self) {
        let _ = self.client.statequery().send_release().await;
        let _ = self.client.statequery().send_done().await;
        // NodeClient dropped here, socket closed
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn kv_to_utxos(
    kv: pallas_network::miniprotocols::localstate::queries_v16::UTxOByAddress,
) -> Result<Vec<Utxo>> {
    let mut out = Vec::new();
    for (utxo_key, tx_out) in kv.iter() {
        let tx_hash = hex::encode(utxo_key.transaction_id.as_ref());
        let output_index = any_uint_to_u64(&utxo_key.index);
        let (address, lovelace, assets, inline_datum) = decode_output(tx_out)?;
        out.push(Utxo {
            tx_hash,
            output_index,
            address,
            lovelace,
            assets,
            inline_datum,
        });
    }
    Ok(out)
}

fn decode_output(out: &TransactionOutput) -> Result<(String, u64, Vec<UtxoAsset>, Option<String>)> {
    match out {
        TransactionOutput::Current(o) => {
            let address = addr_bytes_to_bech32(&o.address)?;
            let (lovelace, assets) = decode_value(&o.amount);
            let inline_datum = o.inline_datum.as_ref().and_then(datum_to_hex);
            Ok((address, lovelace, assets, inline_datum))
        }
        TransactionOutput::Legacy(o) => {
            let address = addr_bytes_to_bech32(&o.address)?;
            let (lovelace, assets) = decode_value(&o.amount);
            Ok((address, lovelace, assets, None))
        }
    }
}

fn addr_bytes_to_bech32(bytes: &pallas_codec::utils::Bytes) -> Result<String> {
    PallasAddr::from_bytes(bytes.as_ref())
        .context("decode address bytes")?
        .to_bech32()
        .context("encode address bech32")
}

fn decode_value(v: &Value) -> (u64, Vec<UtxoAsset>) {
    match v {
        Value::Coin(c) => (any_uint_to_u64(c), vec![]),
        Value::Multiasset(c, assets) => {
            let lovelace = any_uint_to_u64(c);
            let mut out = Vec::new();
            for (policy_id, asset_map) in assets.iter() {
                let pid = hex::encode(policy_id.as_ref());
                for (asset_name, qty) in asset_map.iter() {
                    let name_bytes: &[u8] = asset_name.as_ref();
                    out.push(UtxoAsset {
                        policy_id: pid.clone(),
                        asset_name: hex::encode(name_bytes),
                        quantity: any_uint_to_u64(qty),
                    });
                }
            }
            (lovelace, out)
        }
    }
}

fn datum_to_hex(opt: &DatumOption) -> Option<String> {
    match opt {
        DatumOption::Data(wrap) => minicbor::to_vec(&wrap.0).ok().map(|b| hex::encode(b)),
        DatumOption::Hash(_) => None,
    }
}

fn any_uint_to_u64(v: &AnyUInt) -> u64 {
    match v {
        AnyUInt::MajorByte(n) => *n as u64,
        AnyUInt::U8(n) => *n as u64,
        AnyUInt::U16(n) => *n as u64,
        AnyUInt::U32(n) => *n as u64,
        AnyUInt::U64(n) => *n,
    }
}
