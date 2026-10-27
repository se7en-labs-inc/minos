use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};

/// Unsigned ledger value. Zero native-asset entries are omitted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Value {
    pub coin: u64,
    pub assets: BTreeMap<String, u64>,
}

impl Value {
    pub fn from_quantities<'a>(values: impl IntoIterator<Item = (&'a str, u64)>) -> Result<Self> {
        let mut result = Self::default();
        let mut seen = std::collections::BTreeSet::new();
        for (asset, quantity) in values {
            ensure!(seen.insert(asset), "duplicate asset in value: {asset}");
            if asset.is_empty() {
                result.coin = quantity;
            } else {
                validate_asset_id(asset)?;
                ensure!(quantity > 0, "native asset quantity must be positive");
                result.assets.insert(asset.into(), quantity);
            }
        }
        Ok(result)
    }

    pub fn from_payouts(payouts: &[crate::types::Payout]) -> Result<Self> {
        let mut result = Self::default();
        for payout in payouts {
            for (policy, names) in &payout.value.0 {
                for (name, quantity) in names {
                    let quantity = u64::try_from(*quantity).context("negative payout quantity")?;
                    ensure!(quantity > 0, "payout quantity must be positive");
                    if policy.is_empty() && name.is_empty() {
                        result.coin = result
                            .coin
                            .checked_add(quantity)
                            .context("payout ADA overflow")?;
                    } else {
                        ensure!(
                            policy.len() == 28 && name.len() <= 32,
                            "invalid payout asset identifier"
                        );
                        let id = format!("{}{}", hex::encode(policy), hex::encode(name));
                        let total = result.assets.entry(id).or_default();
                        *total = total
                            .checked_add(quantity)
                            .context("payout asset overflow")?;
                    }
                }
            }
        }
        Ok(result)
    }

    pub fn checked_sub(&self, paid: &Self) -> Result<Self> {
        let mut result = self.clone();
        result.coin = result
            .coin
            .checked_sub(paid.coin)
            .context("payout exceeds vendor ADA")?;
        for (asset, quantity) in &paid.assets {
            let balance = result
                .assets
                .get_mut(asset)
                .context("payout asset absent from vendor input")?;
            *balance = balance
                .checked_sub(*quantity)
                .context("payout exceeds vendor asset balance")?;
        }
        result.assets.retain(|_, quantity| *quantity != 0);
        Ok(result)
    }

    pub fn is_zero(&self) -> bool {
        self.coin == 0 && self.assets.is_empty()
    }
}

pub fn validate_asset_id(id: &str) -> Result<()> {
    ensure!(
        (56..=120).contains(&id.len()) && id.len().is_multiple_of(2),
        "invalid native asset identifier length"
    );
    hex::decode(id).context("invalid native asset identifier hex")?;
    Ok(())
}

/// Add an explicitly native-asset list. ADA is supplied separately on the output.
pub fn add_assets(
    mut output: pallas_txbuilder::Output,
    assets: &[(String, u64)],
) -> Result<pallas_txbuilder::Output> {
    let mut seen = std::collections::BTreeSet::new();
    for (id, quantity) in assets {
        validate_asset_id(id)?;
        ensure!(*quantity > 0, "native asset quantity must be positive");
        let id = id.to_ascii_lowercase();
        ensure!(seen.insert(id.clone()), "duplicate output asset");
        let policy: [u8; 28] = hex::decode(&id[..56])?.try_into().unwrap();
        output = output
            .add_asset(policy.into(), hex::decode(&id[56..])?, *quantity)
            .map_err(|e| anyhow::anyhow!("output asset: {e}"))?;
    }
    Ok(output)
}
