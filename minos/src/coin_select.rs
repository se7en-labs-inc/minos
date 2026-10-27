use anyhow::{bail, ensure, Context, Result};

pub fn address_key_hash(address: &str, network: u8) -> Result<[u8; 28]> {
    let bytes = pallas_addresses::Address::from_bech32(address)?.to_vec();
    ensure!(
        network <= 1
            && bytes.len() >= 29
            && matches!(bytes[0] >> 4, 0 | 2 | 4 | 6)
            && bytes[0] & 15 == network,
        "expected a Shelley payment-key address on the selected network"
    );
    Ok(bytes[1..29]
        .try_into()
        .expect("checked payment credential length"))
}

use crate::koios::Utxo;

/// A wallet UTxO suitable for use as a fee input or collateral.
#[derive(Debug, Clone)]
pub struct WalletUtxo {
    pub tx_hash: String,
    pub output_index: u64,
    pub lovelace: u64,
    pub has_native_assets: bool,
    pub assets: std::collections::BTreeMap<String, u64>,
}

impl TryFrom<&Utxo> for WalletUtxo {
    type Error = anyhow::Error;

    fn try_from(u: &Utxo) -> Result<Self> {
        ensure!(
            u.tx_hash.len() == 64 && hex::decode(&u.tx_hash).is_ok(),
            "invalid wallet transaction hash"
        );
        let mut assets = std::collections::BTreeMap::new();
        for asset in u.asset_list.iter().flatten() {
            let id = format!("{}{}", asset.policy_id, asset.asset_name).to_ascii_lowercase();
            crate::tx::value::validate_asset_id(&id)?;
            ensure!(asset.policy_id.len() == 56, "invalid policy ID");
            let quantity: u64 = asset.quantity.parse().context("invalid asset quantity")?;
            ensure!(quantity > 0, "native asset quantity must be positive");
            ensure!(
                assets.insert(id, quantity).is_none(),
                "duplicate native asset"
            );
        }
        Ok(Self {
            tx_hash: u.tx_hash.to_ascii_lowercase(),
            output_index: u.tx_index,
            lovelace: u.value.parse().context("invalid wallet lovelace")?,
            has_native_assets: u.has_native_assets(),
            assets,
        })
    }
}

/// Preserve every native asset from selected wallet inputs in their change output.
pub fn change_output(
    address: pallas_addresses::Address,
    coin: u64,
    inputs: &[WalletUtxo],
) -> Result<pallas_txbuilder::Output> {
    let mut assets = std::collections::BTreeMap::<String, u64>::new();
    for input in inputs {
        ensure!(
            input.has_native_assets == !input.assets.is_empty(),
            "wallet asset summary is inconsistent"
        );
        let mut seen = std::collections::BTreeSet::new();
        for (id, amount) in &input.assets {
            crate::tx::value::validate_asset_id(id)?;
            ensure!(*amount > 0, "native asset quantity must be positive");
            let id = id.to_ascii_lowercase();
            ensure!(seen.insert(id.clone()), "duplicate wallet asset identifier");
            let total = assets.entry(id).or_default();
            *total = total
                .checked_add(*amount)
                .context("change asset overflow")?;
        }
    }
    let mut output = pallas_txbuilder::Output::new(address, coin);
    for (id, amount) in assets {
        let policy: [u8; 28] = hex::decode(&id[..56])?.try_into().unwrap();
        output = output
            .add_asset(policy.into(), hex::decode(&id[56..])?, amount)
            .map_err(|e| anyhow::anyhow!("change asset: {e}"))?;
    }
    Ok(output)
}

/// Select enough ADA for the reserve, contract top-ups and the actual change
/// asset bundle. Recompute the output minimum whenever another input is added.
pub fn select_with_change<'a>(
    utxos: &'a [WalletUtxo],
    address: &str,
    cost: u64,
    fee_reserve: u64,
    output_topups: u64,
) -> Result<(Vec<&'a WalletUtxo>, u64)> {
    let base = fee_reserve
        .checked_add(output_topups)
        .context("selection target overflow")?;
    let mut target = base;
    for _ in 0..utxos.len().saturating_add(8) {
        let (selected, _) = select_inputs(utxos, target)?;
        let owned: Vec<_> = selected.iter().map(|input| (*input).clone()).collect();
        let address = pallas_addresses::Address::from_bech32(address)?;
        let minimum = crate::tx::fees::minimum_builder_coin(cost, |coin| {
            change_output(address.clone(), coin, &owned)
        })?;
        let total = selected.iter().try_fold(0u64, |sum, input| {
            sum.checked_add(input.lovelace)
                .context("selected ADA overflow")
        })?;
        target = base
            .checked_add(minimum)
            .context("selection target overflow")?;
        if total >= target {
            return Ok((selected, total));
        }
    }
    bail!("change-aware coin selection did not converge")
}

/// Largest-first coin selection. Returns (selected_utxos, change_lovelace).
pub fn select_inputs(
    utxos: &[WalletUtxo],
    target_lovelace: u64,
) -> Result<(Vec<&WalletUtxo>, u64)> {
    let mut candidates: Vec<&WalletUtxo> = utxos.iter().collect();
    let mut seen = std::collections::BTreeSet::new();
    for input in utxos {
        ensure!(
            seen.insert((input.tx_hash.to_ascii_lowercase(), input.output_index)),
            "duplicate wallet UTxO"
        );
    }
    candidates.sort_by_key(|u| std::cmp::Reverse(u.lovelace));

    let mut selected = Vec::new();
    let mut total = 0u64;

    for utxo in candidates {
        if total >= target_lovelace {
            break;
        }
        selected.push(utxo);
        total = total
            .checked_add(utxo.lovelace)
            .context("selected ADA overflow")?;
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

/// Pick a separate ADA-only output meeting the calculated collateral requirement.
pub fn select_separate_collateral<'a>(
    utxos: &'a [WalletUtxo],
    excluded: &[(String, u64)],
    minimum: u64,
) -> Result<&'a WalletUtxo> {
    utxos
        .iter()
        .filter(|u| {
            !u.has_native_assets
                && u.assets.is_empty()
                && u.lovelace >= minimum
                && !excluded
                    .iter()
                    .any(|(hash, index)| *hash == u.tx_hash && *index == u.output_index)
        })
        .min_by_key(|u| u.lovelace)
        .ok_or_else(|| anyhow::anyhow!("no separate ADA-only collateral UTxO"))
}

/// Pick ADA-only collateral meeting the caller's protocol calculation.
/// Refuses UTxOs already chosen as fee inputs.
pub fn select_collateral<'a>(
    utxos: &'a [WalletUtxo],
    exclude_hashes: &[&str],
    minimum: u64,
) -> Result<&'a WalletUtxo> {
    // Prefer UTxOs not already selected as fee inputs
    let candidate = utxos
        .iter()
        .filter(|u| !u.has_native_assets && u.assets.is_empty() && u.lovelace >= minimum)
        .filter(|u| !exclude_hashes.contains(&u.tx_hash.as_str()))
        .min_by_key(|u| u.lovelace);

    candidate.context("no separate ADA-only output meets the collateral requirement")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utxo(coin: u64) -> Utxo {
        let mut inputs: Vec<Utxo> =
            serde_json::from_str(include_str!("../tests/fixtures/withdrawal/utxos.json")).unwrap();
        let mut input = inputs.remove(0);
        input.value = coin.to_string();
        input.asset_list = None;
        input
    }

    #[test]
    fn quantities_duplicates_and_selection_overflow_fail_closed() {
        let mut input = utxo(10);
        for quantity in ["-1", "18446744073709551616", "bogus", "0"] {
            input.asset_list = Some(vec![crate::koios::AssetListItem {
                policy_id: "01".repeat(28),
                asset_name: "".into(),
                quantity: quantity.into(),
            }]);
            assert!(WalletUtxo::try_from(&input).is_err());
        }
        input.asset_list.as_mut().unwrap()[0].quantity = "1".into();
        let duplicate = input.asset_list.as_ref().unwrap()[0].clone();
        input.asset_list.as_mut().unwrap().push(duplicate);
        assert!(WalletUtxo::try_from(&input).is_err());
        input.asset_list = None;
        input.value = "-1".into();
        assert!(WalletUtxo::try_from(&input).is_err());
        let first = WalletUtxo::try_from(&utxo(u64::MAX - 1)).unwrap();
        let mut second = WalletUtxo::try_from(&utxo(2)).unwrap();
        second.output_index += 1;
        assert!(select_inputs(&[first.clone(), second], u64::MAX).is_err());
        assert!(select_inputs(&[first.clone(), first], 1).is_err());
        assert!(select_inputs(&[], 1).is_err());
    }

    #[test]
    fn change_aggregates_every_asset_and_rejects_overflow_or_case_aliases() {
        use pallas_txbuilder::{BuildConway, Input, StagingTransaction};
        let address = pallas_addresses::Address::from_bech32(
            "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh",
        )
        .unwrap();
        let asset = format!("{}ff", "ab".repeat(28));
        let other = format!("{}01", "cd".repeat(28));
        let mut first = WalletUtxo::try_from(&utxo(5_000_000)).unwrap();
        first.has_native_assets = true;
        first.assets.insert(asset.clone(), 7);
        let mut second = first.clone();
        second.output_index += 1;
        second.assets.insert(asset.clone(), 9);
        second.assets.insert(other.clone(), 4);
        let output =
            change_output(address.clone(), 8_000_000, &[first.clone(), second.clone()]).unwrap();
        let built = StagingTransaction::new()
            .input(Input::new([1; 32].into(), 0))
            .output(output)
            .fee(0)
            .network_id(0)
            .valid_from_slot(1)
            .invalid_from_slot(2)
            .build_conway_raw()
            .unwrap();
        let tx: pallas_primitives::conway::Tx<'_> =
            pallas_codec::minicbor::decode(&built.tx_bytes.0).unwrap();
        let actual = crate::preflight::utxo_from_output(
            &"01".repeat(32),
            0,
            &tx.transaction_body.outputs[0],
        )
        .unwrap();
        let actual = WalletUtxo::try_from(&actual).unwrap();
        assert_eq!(actual.lovelace, 8_000_000);
        assert_eq!(
            actual.assets,
            std::collections::BTreeMap::from([(asset.clone(), 16), (other, 4)])
        );
        first.assets.insert(asset.clone(), u64::MAX);
        assert!(change_output(address.clone(), 1, &[first.clone(), second]).is_err());
        first.assets.insert(asset.to_ascii_uppercase(), 1);
        assert!(change_output(address, 1, &[first]).is_err());
    }

    #[test]
    fn collateral_cannot_reuse_a_fee_input() {
        let input = WalletUtxo::try_from(&utxo(10_000_000)).unwrap();
        assert!(select_collateral(std::slice::from_ref(&input), &[&input.tx_hash], 1).is_err());
        assert!(select_collateral(&[input], &[], 1).is_ok());
    }

    #[test]
    fn selection_rebalances_for_native_asset_change_minimum() {
        let address = "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh";
        let mut first = WalletUtxo::try_from(&utxo(10_000_000)).unwrap();
        first.has_native_assets = true;
        first.assets = (0..20)
            .map(|index| (format!("{}{:064x}", "ab".repeat(28), index), 1))
            .collect();
        let mut second = WalletUtxo::try_from(&utxo(9_000_000)).unwrap();
        second.output_index += 1;
        let inputs = [first.clone(), second];
        let (selected, total) =
            select_with_change(&inputs, address, 4310, 7_000_000, 1_000_000).unwrap();
        assert_eq!(
            selected.len(),
            2,
            "the first input cannot fund its token change minimum"
        );
        assert_eq!(total, 19_000_000);
        assert_eq!(selected[0].assets, first.assets);
        assert!(select_with_change(&inputs[..1], address, 4310, 7_000_000, 1_000_000).is_err());
        assert!(select_with_change(&inputs, address, 4310, u64::MAX, 1).is_err());
    }

    #[test]
    fn separate_collateral_can_share_a_transaction_but_not_an_output() {
        let input = WalletUtxo::try_from(&utxo(10_000_000)).unwrap();
        let excluded = [(input.tx_hash.clone(), input.output_index)];
        assert!(select_separate_collateral(std::slice::from_ref(&input), &excluded, 1).is_err());
        let mut sibling = input;
        sibling.output_index += 1;
        assert!(
            select_separate_collateral(std::slice::from_ref(&sibling), &excluded, 10_000_000)
                .is_ok()
        );
        assert!(select_separate_collateral(&[sibling], &excluded, 10_000_001).is_err());
    }
}
