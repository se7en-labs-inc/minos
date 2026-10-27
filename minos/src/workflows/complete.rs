//! complete command workflow, independent of CLI argument parsing.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_complete(
    portal: &Portal,
    koios: &DataSource,
    script_references: &[references::ScriptReference],
    vendor_signers: &[Vec<u8>],
    instance: &str,
    project: &str,
    milestone: &str,
    description: &str,
    evidence_urls: &[String],
    evidence_labels: &[String],
    fee_addr: Option<&str>,
    portal_fees: bool,
    publish_tx: Option<&str>,
    out: Option<&std::path::Path>,
    network_id: u8,
    genesis_offset_secs: u64,
) -> Result<()> {
    // 1. Fetch instance + project data from portal
    let inst = portal.instance_by_id(instance).await?;

    let proj = inst
        .projects
        .iter()
        .find(|p| p.identifier == project)
        .ok_or_else(|| anyhow::anyhow!("project '{project}' not found in instance"))?;

    let _ms = proj
        .milestones
        .iter()
        .find(|m| m.identifier == milestone)
        .ok_or_else(|| anyhow::anyhow!("milestone '{milestone}' not found in project"))?;

    let output = proj
        .unspent_output
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("project '{project}' has no unspent UTxO"))?;

    // 2. Decode vendor datum
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
    let staking_expected: Vec<_> = staking_script_refs
        .iter()
        .cloned()
        .zip(staking_script_hashes.iter().map(hex::encode))
        .collect();

    // 3. Extract vendor UTxO value
    let vendor_value_lovelace = output
        .value
        .iter()
        .find(|v| v.asset_id.is_empty())
        .map(|v| v.quantity)
        .unwrap_or(0);
    let vendor_value_assets: Vec<(String, u64)> = output
        .value
        .iter()
        .filter(|v| !v.asset_id.is_empty())
        .map(|v| (v.asset_id.clone(), v.quantity))
        .collect();

    // 4. Fetch wallet UTxOs
    let (sponsored_addr, sponsored_utxos) = if portal_fees {
        let rows = portal.fee_inputs().await?;
        let mut available = Vec::new();
        for row in rows {
            if row.available_at(Utc::now())? {
                available.push(row);
            }
        }
        let address = available
            .first()
            .context("no unreserved portal fee inputs")?
            .address
            .clone();
        anyhow::ensure!(
            available.iter().all(|r| r.address == address),
            "portal fee inputs use different addresses"
        );
        let live = koios.address_utxos(&address).await?;
        let live = minos::portal::sponsored_utxos(&available, live, &address, Utc::now())?;
        anyhow::ensure!(
            live.len() >= 2,
            "need two live unreserved ADA-only portal fee inputs for fee and collateral"
        );
        (Some(address), Some(live))
    } else {
        (None, None)
    };
    let fee_addr = sponsored_addr
        .as_deref()
        .or(fee_addr)
        .context("missing fee address")?;
    let fee_key_hash = if portal_fees {
        let address = pallas_addresses::Address::from_bech32(fee_addr)?.to_vec();
        anyhow::ensure!(
            matches!(address[0] >> 4, 0 | 2 | 4 | 6) && address.len() >= 29,
            "portal fee address must use a payment key"
        );
        anyhow::ensure!(
            address[0] & 15 == network_id,
            "fee address belongs to another network"
        );
        Some(address[1..29].to_vec())
    } else {
        None
    };
    let utxos = match sponsored_utxos {
        Some(utxos) => utxos,
        None => koios.address_utxos(fee_addr).await?,
    };
    let wallet_utxos: Vec<coin_select::WalletUtxo> = utxos
        .iter()
        .map(coin_select::WalletUtxo::try_from)
        .collect::<Result<Vec<_>>>()?;

    // 5. Fetch tip and protocol params
    let tip = koios.tip().await?;
    let proto = koios.protocol_params().await?;

    let v3_cost_model = proto
        .v3_cost_model()
        .ok_or_else(|| anyhow::anyhow!("no PlutusV3 cost model in protocol params"))?;

    // 7. Script reference inputs + registry NFT
    let vendor_ref = inst
        .registry
        .vendor_script
        .script_reference
        .as_ref()
        .map(|r| (r.transaction.clone(), r.index))
        .or_else(|| publish_tx.map(|h| (h.to_owned(), 2)))
        .context("vendor script has no scriptReference; provide --publish-tx")?;
    // Live deployments can publish registry and reference scripts independently.
    let registry = references::registry_output(
        koios,
        instance,
        inst.registry
            .registry_reference
            .as_ref()
            .map(|reference| (reference.transaction.clone(), reference.index)),
    )
    .await?;
    let registry_utxo_hash = registry.tx_hash;
    let registry_utxo_index = registry.tx_index;

    let ref_utxos = references::withdrawal_references(
        koios,
        instance,
        (registry_utxo_hash.clone(), registry_utxo_index),
        vendor_ref.clone(),
        &staking_expected,
    )
    .await?;
    let ref_script_bytes = references::total_script_bytes(&ref_utxos)?;
    let fee_reserve = tx::fees::maximum_fee(&proto, ref_script_bytes, true)?;
    let output_minimum = tx::fees::minimum_value_coin(
        &output.address,
        tx::fees::utxo_cost(&proto)?,
        &vendor_value_assets,
        Some(&hex::decode(&output.datum_hex)?),
    )?;
    let (selected, total_fee_selected) = coin_select::select_with_change(
        &wallet_utxos,
        fee_addr,
        tx::fees::utxo_cost(&proto)?,
        fee_reserve,
        output_minimum.saturating_sub(vendor_value_lovelace),
    )?;
    let fee_refs: Vec<_> = selected
        .iter()
        .map(|u| (u.tx_hash.clone(), u.output_index))
        .collect();
    let collateral = coin_select::select_separate_collateral(&wallet_utxos, &fee_refs, 0)?;

    // 8. Validity interval
    let valid_from_slot = tx::complete::validity_start(tip.abs_slot, genesis_offset_secs, &datum);
    let invalid_from_slot = tip.abs_slot + 7200;

    // 9. Build params — two passes: first to measure tx size, then with exact fee
    let mut params = tx::complete::CompleteParams {
        coins_per_utxo_byte: tx::fees::utxo_cost(&proto)?,
        vendor_utxo_hash: output.transaction.clone(),
        vendor_utxo_index: output.index,
        vendor_address: output.address.clone(),
        vendor_datum_hex: output.datum_hex.clone(),
        vendor_value_lovelace,
        vendor_value_assets,
        registry_utxo_hash,
        registry_utxo_index,
        vendor_script_ref_hash: vendor_ref.0.clone(),
        vendor_script_ref_index: vendor_ref.1,
        vendor_key_hashes,
        fee_key_hash,
        staking_script_hashes,
        staking_script_refs,
        staking_reward_mem: 2_500_000,
        staking_reward_steps: 900_000_000,
        fee_inputs: selected.iter().map(|u| (*u).clone()).collect(),
        collateral: collateral.clone(),
        fee_addr: fee_addr.to_string(),
        fee: fee_reserve,
        change_lovelace: total_fee_selected.saturating_sub(fee_reserve),
        valid_from_slot,
        invalid_from_slot,
        v3_cost_model,
        instance_id: instance.to_string(),
        project_id: project.to_string(),
        milestone_id: milestone.to_string(),
        description: description.to_string(),
        evidence_urls: evidence_urls.to_vec(),
        evidence_labels: evidence_labels.to_vec(),
        network_id,
    };

    let witnesses = tx::fees::witness_count(
        params
            .vendor_key_hashes
            .iter()
            .chain(params.fee_key_hash.iter())
            .map(Vec::as_slice),
        fee_addr,
        network_id,
    )?;
    let unsigned_tx = tx::fees::balance(
        &proto,
        total_fee_selected,
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
            tx::complete::build(params.clone())
        },
    )?;
    tx::fees::validate_collateral(&unsigned_tx.cbor_hex, &proto, &params.collateral)?;
    eprintln!(
        "tx fee: {} lovelace ({:.6} ADA)",
        params.fee,
        params.fee as f64 / 1_000_000.0
    );

    let report = minos::preflight::withdrawal(
        koios,
        &unsigned_tx.cbor_hex,
        &proto,
        genesis_offset_secs,
        &ref_utxos,
    )
    .await
    .context("completion preflight failed; unsigned file was not written")?;
    eprintln!(
        "Completion preflight passed: {} redeemers, {} memory, {} steps",
        report.redeemers, report.memory, report.steps
    );
    eprintln!(
        "Transaction body hash: {}",
        hex::encode(tx::sign::tx_body_hash(&hex::decode(
            &unsigned_tx.cbor_hex
        )?)?)
    );
    eprintln!("Fee/change address: {fee_addr}");
    eprintln!(
        "Valid until: {}",
        ms_to_utc((invalid_from_slot + genesis_offset_secs) * 1000)
    );
    eprintln!("Validity end slot: {invalid_from_slot}; no signing or submission performed");
    write_tx(unsigned_tx, out)
}
