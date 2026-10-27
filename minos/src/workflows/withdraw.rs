//! withdraw command workflow, independent of CLI argument parsing.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_withdraw(
    portal: &Portal,
    koios: &DataSource,
    script_references: &[references::ScriptReference],
    vendor_signers: &[Vec<u8>],
    instance: &str,
    project: &str,
    fee_addr: &str,
    dest_addr: &str,
    publish_tx: Option<&str>,
    registry_utxo: Option<&str>,
    upcoming: bool,
    ttl_hours: u64,
    out: Option<&std::path::Path>,
    network_id: u8,
    genesis_offset_secs: u64,
) -> Result<()> {
    // 1. Fetch instance + project data
    let inst = portal.instance_by_id(instance).await?;

    let proj = inst
        .projects
        .iter()
        .find(|p| p.identifier == project)
        .ok_or_else(|| anyhow::anyhow!("project '{project}' not found in instance"))?;

    let output = proj
        .unspent_output
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("project '{project}' has no unspent UTxO"))?;

    // 2. Decode datum and find matured payouts
    let datum = types::VendorDatum::from_cbor_hex(&output.datum_hex)?;
    let vendor_key_hashes = if vendor_signers.is_empty() {
        datum.vendor.key_hashes()
    } else {
        vendor_signers.to_vec()
    };
    anyhow::ensure!(
        !vendor_key_hashes.is_empty(),
        "declare --vendor-signer for script-only vendor authorization and metadata authorship"
    );

    let (staking_script_hashes, staking_script_refs) =
        references::staking_authorization(&datum.vendor, script_references)?;
    let staking_reward_mem: u64 = 2_500_000 * staking_script_hashes.len() as u64;
    let staking_reward_steps: u64 = 900_000_000 * staking_script_hashes.len() as u64;

    // Sanity check: re-encode the full datum and compare with the portal's datum_hex.
    // If these differ, the round-trip is lossy and the script's datum equality check will fail.
    let roundtrip_hex = hex::encode(datum.to_cbor_bytes());
    if roundtrip_hex != output.datum_hex {
        eprintln!("WARNING: datum round-trip mismatch!");
        eprintln!(
            "  portal  : {}",
            &output.datum_hex[..80.min(output.datum_hex.len())]
        );
        eprintln!(
            "  roundtrip: {}",
            &roundtrip_hex[..80.min(roundtrip_hex.len())]
        );
    } else {
        eprintln!("DEBUG: datum round-trip OK");
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;

    // Fetch tip for abs_slot; genesis_offset_secs is hardcoded per-network (not derived
    // from tip.block_time - tip.abs_slot, which drifts when block_time = SystemTime::now()).
    let tip = koios.tip().await?;

    // Determine the maturation threshold for this withdraw.
    // Normal mode: only payouts already past their maturation time.
    // --upcoming mode: find the earliest future-maturing Active payout and use that batch,
    //   so the tx can be pre-signed now and submitted by a colleague when it matures.
    let latest_maturation_ms = if upcoming {
        let next = datum
            .payouts
            .iter()
            .filter(|p| matches!(p.status, types::PayoutStatus::Active) && p.maturation > now_ms)
            .map(|p| p.maturation)
            .min()
            .ok_or_else(|| anyhow::anyhow!("no upcoming Active payouts; omit --upcoming to withdraw already-matured payouts"))?;
        // Include all Active payouts up to and including the earliest upcoming batch.
        datum
            .payouts
            .iter()
            .filter(|p| matches!(p.status, types::PayoutStatus::Active) && p.maturation <= next)
            .map(|p| p.maturation)
            .max()
            .unwrap_or(next)
    } else {
        let any_matured = datum
            .payouts
            .iter()
            .any(|p| matches!(p.status, types::PayoutStatus::Active) && p.maturation <= now_ms);
        if !any_matured {
            anyhow::bail!("no matured Active payouts available to withdraw");
        }
        datum
            .payouts
            .iter()
            .filter(|p| matches!(p.status, types::PayoutStatus::Active) && p.maturation <= now_ms)
            .map(|p| p.maturation)
            .max()
            .unwrap_or(0)
    };
    // Use the current tip slot as valid_from: the maturation is already in the past,
    // so "now" satisfies maturation_time <= lower_bound more clearly than the maturation slot itself.
    // For --upcoming (pre-signing a future payout), fall back to the maturation slot so the
    // tx is valid when submitted later.
    let valid_from_slot = if upcoming {
        (latest_maturation_ms / 1000 + 1).saturating_sub(genesis_offset_secs)
    } else {
        tip.abs_slot.saturating_sub(120)
    };
    let valid_from_posix_ms = (valid_from_slot + genesis_offset_secs) * 1000;

    eprintln!("DEBUG: now_ms={now_ms} tip.block_time={} tip.abs_slot={} genesis_offset={genesis_offset_secs}",
        tip.block_time, tip.abs_slot);
    eprintln!("DEBUG: latest_maturation_ms={latest_maturation_ms} valid_from_slot={valid_from_slot} valid_from_posix_ms={valid_from_posix_ms}");

    // Re-partition using the exact same predicate the Plutus script applies,
    // so our datum and value split always agrees with what the script enforces.
    type IndexedPayout<'a> = (usize, &'a types::Payout);
    let (matured_indexed, remaining_indexed): (Vec<IndexedPayout<'_>>, Vec<IndexedPayout<'_>>) =
        datum.payouts.iter().enumerate().partition(|(_, p)| {
            matches!(p.status, types::PayoutStatus::Active) && p.maturation < valid_from_posix_ms
        });

    let matured: Vec<types::Payout> = matured_indexed
        .iter()
        .map(|(i, _)| datum.payouts[*i].clone())
        .collect();
    let remaining: Vec<types::Payout> = remaining_indexed
        .iter()
        .map(|(i, _)| datum.payouts[*i].clone())
        .collect();

    eprintln!(
        "DEBUG: claiming {} payouts, {} remaining",
        matured.len(),
        remaining.len()
    );
    for p in &matured {
        eprintln!("  CLAIM: maturation={}", p.maturation);
    }
    if let Some(first_rem) = remaining.first() {
        eprintln!("  REMAINING starts at: {}", first_rem.maturation);
    }

    // Derive the payout and residual independently of the payout-list length.
    let paid = tx::value::Value::from_payouts(&matured)?;
    let matured_lovelace_total = paid.coin;
    let matured_assets: Vec<(String, u64)> = paid
        .assets
        .iter()
        .map(|(id, qty)| (id.clone(), *qty))
        .collect();
    let matured_lovelace_out = matured_lovelace_total;
    let vendor_value = tx::value::Value::from_quantities(
        output
            .value
            .iter()
            .map(|v| (v.asset_id.as_str(), v.quantity)),
    )?;
    let residual = vendor_value.checked_sub(&paid)?;
    let remaining_datum_bytes = tx::withdraw::continuing_datum(&datum, remaining, &residual);
    let remaining_lovelace = residual.coin;
    let remaining_assets: Vec<(String, u64)> = residual.assets.into_iter().collect();

    // 5. Fetch wallet UTxOs and protocol params (tip already fetched above)
    let utxos = koios.address_utxos(fee_addr).await?;
    let wallet_utxos: Vec<coin_select::WalletUtxo> = utxos
        .iter()
        .map(coin_select::WalletUtxo::try_from)
        .collect::<Result<Vec<_>>>()?;

    let proto = koios.protocol_params().await?;

    let v3_cost_model = proto
        .v3_cost_model()
        .ok_or_else(|| anyhow::anyhow!("no PlutusV3 cost model in protocol params"))?;

    // 6. Registry NFT UTxO + script ref inputs
    struct WithdrawRefInfo {
        registry_utxo_hash: String,
        registry_utxo_index: u64,
        vendor_script_ref_hash: String,
        vendor_script_ref_index: u64,
    }
    // Parse --registry-utxo override (format: "<txhash>#<index>")
    let registry_utxo_override: Option<(String, u64)> = registry_utxo
        .map(|s| {
            let (hash, idx) = s
                .split_once('#')
                .ok_or_else(|| anyhow::anyhow!("--registry-utxo must be <txhash>#<index>"))?;
            let idx: u64 = idx
                .parse()
                .map_err(|_| anyhow::anyhow!("--registry-utxo index must be a number"))?;
            Ok::<_, anyhow::Error>((hash.to_string(), idx))
        })
        .transpose()?;

    let ref_info: WithdrawRefInfo = if let Some(pub_tx) = publish_tx {
        let (reg_hash, reg_idx) = registry_utxo_override
            .clone()
            .unwrap_or_else(|| (pub_tx.to_string(), 0));
        WithdrawRefInfo {
            registry_utxo_hash: reg_hash,
            registry_utxo_index: reg_idx,
            vendor_script_ref_hash: pub_tx.to_string(),
            vendor_script_ref_index: 2,
        }
    } else {
        let vendor_ref = inst
            .registry
            .vendor_script
            .script_reference
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("vendor script has no scriptReference"))?;
        let registry = references::registry_output(
            koios,
            instance,
            registry_utxo_override.clone().or_else(|| {
                inst.registry
                    .registry_reference
                    .as_ref()
                    .map(|reference| (reference.transaction.clone(), reference.index))
            }),
        )
        .await?;
        let (reg_hash, reg_idx) = (registry.tx_hash, registry.tx_index);
        WithdrawRefInfo {
            registry_utxo_hash: reg_hash,
            registry_utxo_index: reg_idx,
            vendor_script_ref_hash: vendor_ref.transaction.clone(),
            vendor_script_ref_index: vendor_ref.index,
        }
    };

    // 7. Fetch ref script sizes for accurate fee calculation
    let staking_expected: Vec<_> = staking_script_refs
        .iter()
        .cloned()
        .zip(staking_script_hashes.iter().map(hex::encode))
        .collect();
    let ref_utxos = references::withdrawal_references(
        koios,
        instance,
        (
            ref_info.registry_utxo_hash.clone(),
            ref_info.registry_utxo_index,
        ),
        (
            ref_info.vendor_script_ref_hash.clone(),
            ref_info.vendor_script_ref_index,
        ),
        &staking_expected,
    )
    .await?;
    let ref_script_bytes = references::total_script_bytes(&ref_utxos)?;
    eprintln!("ref script bytes: {ref_script_bytes}");

    // 8. Coin selection
    let fee_reserve = tx::fees::maximum_fee(&proto, ref_script_bytes, true)?;
    let cost = tx::fees::utxo_cost(&proto)?;
    let destination_topup = tx::fees::minimum_value_coin(dest_addr, cost, &matured_assets, None)?
        .saturating_sub(matured_lovelace_total);
    let continuation_topup = if let Some(datum) = &remaining_datum_bytes {
        tx::fees::minimum_value_coin(&output.address, cost, &remaining_assets, Some(datum))?
            .saturating_sub(remaining_lovelace)
    } else {
        0
    };
    let output_topups = destination_topup
        .checked_add(continuation_topup)
        .context("output top-up overflow")?;
    let (selected, total_fee_selected) =
        coin_select::select_with_change(&wallet_utxos, fee_addr, cost, fee_reserve, output_topups)?;
    let fee_refs: Vec<_> = selected
        .iter()
        .map(|u| (u.tx_hash.clone(), u.output_index))
        .collect();
    let collateral = coin_select::select_separate_collateral(&wallet_utxos, &fee_refs, 0)?;

    // 9. Validity interval — computed above from maturation timestamps
    let invalid_from_slot = tip.abs_slot + ttl_hours * 3600;

    // 10. Build params — two passes: first to measure tx size, then with exact fee
    let mut params = tx::withdraw::WithdrawParams {
        coins_per_utxo_byte: tx::fees::utxo_cost(&proto)?,
        vendor_utxo_hash: output.transaction.clone(),
        vendor_utxo_index: output.index,
        vendor_address: output.address.clone(),
        matured_lovelace: matured_lovelace_out,
        matured_assets,
        remaining_datum_bytes,
        remaining_lovelace,
        remaining_assets,
        registry_utxo_hash: ref_info.registry_utxo_hash,
        registry_utxo_index: ref_info.registry_utxo_index,
        vendor_script_ref_hash: ref_info.vendor_script_ref_hash,
        vendor_script_ref_index: ref_info.vendor_script_ref_index,
        vendor_key_hashes,
        fee_inputs: selected.iter().map(|u| (*u).clone()).collect(),
        collateral: collateral.clone(),
        fee_addr: fee_addr.to_string(),
        fee: fee_reserve,
        change_lovelace: total_fee_selected.saturating_sub(fee_reserve),
        dest_addr: dest_addr.to_string(),
        valid_from_slot,
        invalid_from_slot,
        v3_cost_model,
        instance_id: instance.to_string(),
        network_id,
        staking_script_hashes,
        staking_script_refs,
        staking_reward_mem,
        staking_reward_steps,
    };
    let available = total_fee_selected;
    let witnesses = tx::fees::witness_count(
        params.vendor_key_hashes.iter().map(Vec::as_slice),
        fee_addr,
        network_id,
    )?;
    let unsigned_tx = tx::fees::balance(
        &proto,
        available,
        ref_script_bytes,
        witnesses,
        |fee, change| {
            params.collateral = coin_select::select_separate_collateral(
                &wallet_utxos,
                &fee_refs,
                tx::fees::minimum_collateral(fee, &proto)?,
            )?
            .clone();
            params.fee = fee;
            params.change_lovelace = change;
            tx::withdraw::build(params.clone())
        },
    )?;
    tx::fees::validate_collateral(&unsigned_tx.cbor_hex, &proto, &params.collateral)?;
    eprintln!("tx fee: {:.4} ADA", params.fee as f64 / 1_000_000.0);

    let report = minos::preflight::withdrawal(
        koios,
        &unsigned_tx.cbor_hex,
        &proto,
        genesis_offset_secs,
        &ref_utxos,
    )
    .await
    .context("withdrawal preflight failed; unsigned file was not written")?;
    eprintln!(
        "Withdrawal preflight passed: {} redeemers, {} memory, {} steps",
        report.redeemers, report.memory, report.steps
    );
    write_tx(unsigned_tx, out)
}
