//! Portal adapters around the shared, bounded Pallas backend.
use crate::koios::{Utxo, UtxoAsset};
use anyhow::{Context, Result};
use minos::node::LocalNode;

pub struct PortalNode(LocalNode);
impl PortalNode {
    pub fn new(socket_path: String, network_magic: u64) -> Self {
        Self(LocalNode::new(socket_path, network_magic))
    }
    pub fn with_timeout(mut self, duration: std::time::Duration) -> Self {
        self.0 = self.0.with_timeout(duration);
        self
    }
    pub async fn session(&self) -> Result<NodeSession> {
        Ok(NodeSession(self.0.session().await?))
    }
}

pub struct NodeSession(minos::node::NodeSession);
impl NodeSession {
    pub async fn tip(&mut self) -> Result<(u64, u64)> {
        let tip = self.0.tip().await?;
        Ok((tip.abs_slot, tip.block_no))
    }
    pub async fn address_utxos(&mut self, address: &str) -> Result<Vec<Utxo>> {
        adapt(self.0.address_utxos(address).await?)
    }
    pub async fn outref_utxos(&mut self, refs: &[(String, u64)]) -> Result<Vec<Utxo>> {
        adapt(self.0.utxo_info(refs).await?)
    }
    pub async fn close(self) {
        self.0.close().await;
    }
}

fn adapt(utxos: Vec<minos::koios::Utxo>) -> Result<Vec<Utxo>> {
    utxos
        .into_iter()
        .map(|u| {
            let inline_datum = u.datum_hex();
            Ok(Utxo {
                tx_hash: u.tx_hash,
                output_index: u.tx_index,
                address: u.address,
                lovelace: u.value.parse().context("invalid UTxO lovelace")?,
                assets: u
                    .asset_list
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| {
                        Ok(UtxoAsset {
                            policy_id: a.policy_id,
                            asset_name: a.asset_name,
                            quantity: a.quantity.parse().context("invalid asset quantity")?,
                        })
                    })
                    .collect::<Result<_>>()?,
                inline_datum,
            })
        })
        .collect()
}
