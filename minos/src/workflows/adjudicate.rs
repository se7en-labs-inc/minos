//! adjudicate command workflow, independent of CLI argument parsing.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_adjudicate(
    portal: &Portal,
    koios: &DataSource,
    script_references: &[references::ScriptReference],
    additional_signers: &[Vec<u8>],
    instance: &str,
    project: &str,
    pause_indices_str: Option<&str>,
    resume_indices_str: Option<&str>,
    admin_key_hex: &str,
    fee_addr: &str,
    publish_tx: Option<&str>,
    vendor_utxo_override: Option<&str>,
    out: Option<&std::path::Path>,
    network_id: u8,
    genesis_offset_secs: u64,
) -> Result<()> {
    let admin_key_hash = hex::decode(admin_key_hex.trim()).context("admin_key hex")?;
    anyhow::ensure!(admin_key_hash.len() == 28, "admin_key must be 28 bytes");

    // Parse pause / resume index lists
    let parse_indices = |s: Option<&str>| -> Result<std::collections::HashSet<usize>> {
        match s {
            None => Ok(std::collections::HashSet::new()),
            Some("") => Ok(std::collections::HashSet::new()),
            Some(s) => s
                .split(',')
                .map(|t| t.trim().parse::<usize>().context("index must be a number"))
                .collect(),
        }
    };
    let pause_set = parse_indices(pause_indices_str)?;
    let resume_set = parse_indices(resume_indices_str)?;
    anyhow::ensure!(
        !pause_set.is_empty() || !resume_set.is_empty(),
        "must specify at least one --pause-indices or --resume-indices"
    );
    anyhow::ensure!(
        pause_set.is_disjoint(&resume_set),
        "pause and resume index sets must not overlap"
    );

    // -------------------------------------------------------------------------
    // Resolve vendor UTxO (portal or Koios fallback)
    // -------------------------------------------------------------------------
    struct VendorInfo {
        utxo_hash: String,
        utxo_index: u64,
        address: String,
        datum_hex: String,
        lovelace: u64,
        assets: Vec<(String, u64)>,
    }

    let vendor_info: VendorInfo = if let Some(override_ref) = vendor_utxo_override {
        // Direct Koios lookup by UTxO ref
        let (hash, idx) = parse_utxo_ref(override_ref)?;
        let utxo_list = koios.utxo_info(&[(hash.clone(), idx)]).await?;
        anyhow::ensure!(
            utxo_list.len() == 1,
            "expected exactly one vendor output response"
        );
        let utxo = utxo_list
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("vendor UTxO {hash}#{idx} not found on chain"))?;
        let datum_hex = utxo
            .datum_hex()
            .ok_or_else(|| anyhow::anyhow!("vendor UTxO has no inline datum"))?;
        anyhow::ensure!(
            utxo.tx_hash == hash && utxo.tx_index == idx,
            "vendor output response mismatch"
        );
        let checked = coin_select::WalletUtxo::try_from(&utxo)?;
        let lovelace = checked.lovelace;
        let assets: Vec<(String, u64)> = checked.assets.into_iter().collect();
        VendorInfo {
            utxo_hash: utxo.tx_hash,
            utxo_index: utxo.tx_index,
            address: utxo.address,
            datum_hex,
            lovelace,
            assets,
        }
    } else {
        // Portal lookup
        let inst = portal.instance_by_id(instance).await?;
        let proj = inst
            .projects
            .iter()
            .find(|p| p.identifier == project)
            .ok_or_else(|| anyhow::anyhow!("project '{project}' not found"))?;
        let output = proj
            .unspent_output
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("project '{project}' has no unspent UTxO"))?;
        let lovelace = output
            .value
            .iter()
            .find(|v| v.asset_id.is_empty())
            .map(|v| v.quantity)
            .unwrap_or(0);
        let assets: Vec<(String, u64)> = output
            .value
            .iter()
            .filter(|v| !v.asset_id.is_empty())
            .map(|v| (v.asset_id.clone(), v.quantity))
            .collect();
        VendorInfo {
            utxo_hash: output.transaction.clone(),
            utxo_index: output.index,
            address: output.address.clone(),
            datum_hex: output.datum_hex.clone(),
            lovelace,
            assets,
        }
    };

    // -------------------------------------------------------------------------
    // Resolve registry / script reference UTxOs
    // -------------------------------------------------------------------------
    let hints = if let Some(publish) = publish_tx {
        references::ReferenceHints {
            legacy_publish_tx: Some(publish.into()),
            ..Default::default()
        }
    } else {
        let instance = portal.instance_by_id(instance).await?;
        references::ReferenceHints {
            registry: instance
                .registry
                .registry_reference
                .as_ref()
                .map(|reference| (reference.transaction.clone(), reference.index)),
            vendor: instance
                .registry
                .vendor_script
                .script_reference
                .as_ref()
                .map(|reference| (reference.transaction.clone(), reference.index)),
            legacy_publish_tx: None,
        }
    };
    let ref_info =
        references::vendor_inputs(koios, instance, hints, script_references, &[]).await?;

    // 2. Decode datum
    let datum = types::VendorDatum::from_cbor_hex(&vendor_info.datum_hex)?;
    let n = datum.payouts.len();
    for &i in &pause_set {
        anyhow::ensure!(i < n, "pause index {i} out of range (payouts len = {n})");
    }
    for &i in &resume_set {
        anyhow::ensure!(i < n, "resume index {i} out of range (payouts len = {n})");
    }

    // 3. Build new statuses
    let new_statuses: Vec<types::PayoutStatus> = datum
        .payouts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if pause_set.contains(&i) {
                types::PayoutStatus::Paused
            } else if resume_set.contains(&i) {
                types::PayoutStatus::Active
            } else {
                p.status.clone()
            }
        })
        .collect();

    // 4. Fetch tip; compute valid_from_slot to allow pausing matured Active payouts.
    //    For any Active payout we want to pause, valid_from_slot must correspond to
    //    a POSIX time ≤ that payout's maturation (so is_entirely_after returns False).
    let tip = koios.tip().await?;
    let invalid_from_slot = tip.abs_slot + 7200;

    // Find the earliest maturation among payouts we're pausing that are currently Active
    let earliest_maturation_ms: Option<u64> = pause_set
        .iter()
        .filter(|&&i| matches!(datum.payouts[i].status, types::PayoutStatus::Active))
        .map(|&i| datum.payouts[i].maturation)
        .min();

    let valid_from_slot = if let Some(mat_ms) = earliest_maturation_ms {
        // Use the slot corresponding to that maturation so is_entirely_after is False.
        // Integer division drops sub-second precision intentionally:
        // slot_posix_ms = (genesis + slot) * 1000 ≤ mat_ms  →  is_entirely_after = False.
        let slot_at_maturation = (mat_ms / 1000).saturating_sub(genesis_offset_secs);
        // 36-hour lower bound from invalid_from_slot (interval must fit within 36h)
        let min_slot = invalid_from_slot.saturating_sub(129_600);
        anyhow::ensure!(
            slot_at_maturation >= min_slot,
            "maturation is more than 36 hours in the past — cannot adjudicate within the interval constraint"
        );
        slot_at_maturation
    } else {
        // No Active payouts being paused (all already Paused, or only resuming)
        tip.abs_slot.saturating_sub(120)
    };

    // 5. Wallet UTxOs and protocol params
    let utxos = koios.address_utxos(fee_addr).await?;
    let wallet_utxos: Vec<coin_select::WalletUtxo> = utxos
        .iter()
        .map(coin_select::WalletUtxo::try_from)
        .collect::<Result<Vec<_>>>()?;
    let proto = koios.protocol_params().await?;
    let v3_cost_model = proto
        .v3_cost_model()
        .ok_or_else(|| anyhow::anyhow!("no PlutusV3 cost model"))?;

    let ref_utxos = ref_info.inputs;
    let ref_script_bytes = references::total_script_bytes(&ref_utxos)?;
    let fee_reserve = tx::fees::maximum_fee(&proto, ref_script_bytes, true)?;
    let output_minimum = tx::fees::minimum_value_coin(
        &vendor_info.address,
        tx::fees::utxo_cost(&proto)?,
        &vendor_info.assets,
        Some(&{
            let mut updated = datum.clone();
            for (payout, status) in updated.payouts.iter_mut().zip(&new_statuses) {
                payout.status = status.clone();
            }
            updated.to_cbor_bytes()
        }),
    )?;
    let (selected, total_selected) = coin_select::select_with_change(
        &wallet_utxos,
        fee_addr,
        tx::fees::utxo_cost(&proto)?,
        fee_reserve,
        output_minimum.saturating_sub(vendor_info.lovelace),
    )?;
    let fee_refs: Vec<_> = selected
        .iter()
        .map(|u| (u.tx_hash.clone(), u.output_index))
        .collect();
    let collateral = coin_select::select_separate_collateral(&wallet_utxos, &fee_refs, 0)?;

    let mut params = tx::adjudicate::AdjudicateParams {
        coins_per_utxo_byte: tx::fees::utxo_cost(&proto)?,
        vendor_utxo_hash: vendor_info.utxo_hash,
        vendor_utxo_index: vendor_info.utxo_index,
        vendor_address: vendor_info.address,
        current_datum: datum,
        new_statuses,
        vendor_lovelace: vendor_info.lovelace,
        vendor_assets: vendor_info.assets,
        registry_utxo_hash: ref_info.registry.0,
        registry_utxo_index: ref_info.registry.1,
        vendor_script_ref_hash: ref_info.vendor.0,
        vendor_script_ref_index: ref_info.vendor.1,
        admin_key_hash,
        co_signer_hashes: additional_signers.to_vec(),
        fee_inputs: selected.iter().map(|u| (*u).clone()).collect(),
        collateral: collateral.clone(),
        fee_addr: fee_addr.to_string(),
        fee: fee_reserve,
        change_lovelace: total_selected.saturating_sub(fee_reserve),
        valid_from_slot,
        invalid_from_slot,
        v3_cost_model,
        instance_id: instance.to_string(),
        network_id,
    };

    let witnesses = tx::fees::witness_count(
        params
            .co_signer_hashes
            .iter()
            .chain(std::iter::once(&params.admin_key_hash))
            .map(Vec::as_slice),
        fee_addr,
        network_id,
    )?;
    let unsigned_tx = tx::fees::balance(
        &proto,
        total_selected,
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
            tx::adjudicate::build(params.clone())
        },
    )?;
    tx::fees::validate_collateral(&unsigned_tx.cbor_hex, &proto, &params.collateral)?;
    minos::preflight::transaction(
        koios,
        &unsigned_tx.cbor_hex,
        &proto,
        genesis_offset_secs,
        &ref_utxos,
    )
    .await?;
    eprintln!("tx fee: {:.4} ADA", params.fee as f64 / 1_000_000.0);

    write_tx(unsigned_tx, out)
}
