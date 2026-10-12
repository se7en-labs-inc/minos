use anyhow::{bail, Result};

use crate::koios::Utxo;

/// A wallet UTxO suitable for use as a fee input or collateral.
#[derive(Debug, Clone)]
pub struct WalletUtxo {
    pub tx_hash: String,
    pub output_index: u64,
    pub lovelace: u64,
    pub has_native_assets: bool,
}

impl From<&Utxo> for WalletUtxo {
    fn from(u: &Utxo) -> Self {
        Self {
            tx_hash: u.tx_hash.clone(),
            output_index: u.tx_index,
            lovelace: u.lovelace(),
            has_native_assets: u.has_native_assets(),
        }
    }
}

/// Largest-first coin selection. Returns (selected_utxos, change_lovelace).
pub fn select_inputs(
    utxos: &[WalletUtxo],
    target_lovelace: u64,
) -> Result<(Vec<&WalletUtxo>, u64)> {
    let mut candidates: Vec<&WalletUtxo> = utxos.iter().collect();
    candidates.sort_by(|a, b| b.lovelace.cmp(&a.lovelace));

    let mut selected = Vec::new();
    let mut total = 0u64;

    for utxo in candidates {
        if total >= target_lovelace {
            break;
        }
        selected.push(utxo);
        total = total.saturating_add(utxo.lovelace);
    }

    if total < target_lovelace {
        bail!(
            "insufficient funds: need {} lovelace ({:.2} ADA), wallet has {} lovelace ({:.2} ADA) across {} UTxOs",
            target_lovelace,
            target_lovelace as f64 / 1_000_000.0,
            total,
            total as f64 / 1_000_000.0,
            utxos.len()
        );
    }

    Ok((selected, total - target_lovelace))
}

/// Pick a collateral UTxO: ADA-only, >= 5 ADA.
/// Avoids UTxOs already chosen as fee inputs where possible.
pub fn select_collateral<'a>(
    utxos: &'a [WalletUtxo],
    exclude_hashes: &[&str],
) -> Result<&'a WalletUtxo> {
    const MIN_COLLATERAL: u64 = 5_000_000;

    // Prefer UTxOs not already selected as fee inputs
    let candidate = utxos
        .iter()
        .filter(|u| !u.has_native_assets && u.lovelace >= MIN_COLLATERAL)
        .filter(|u| !exclude_hashes.contains(&u.tx_hash.as_str()))
        .min_by_key(|u| u.lovelace);

    if let Some(c) = candidate {
        return Ok(c);
    }

    // Fall back to any ADA-only UTxO with enough value
    utxos
        .iter()
        .filter(|u| !u.has_native_assets && u.lovelace >= MIN_COLLATERAL)
        .min_by_key(|u| u.lovelace)
        .ok_or_else(|| {
            anyhow::anyhow!("no suitable collateral UTxO (need ADA-only UTxO with >= 5 ADA)")
        })
}
