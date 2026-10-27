//! Treasury deployment workflow, independent of CLI argument parsing.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_publish(
    koios: &DataSource,
    fee_addr: &str,
    admin_key_hex: &[String],
    admin_threshold: u64,
    expiry: &str,
    payout_upperbound: &str,
    vendor_expiry: &str,
    label: &str,
    description: Option<&str>,
    out: Option<&std::path::Path>,
    network_id: u8,
) -> Result<()> {
    let admin_keys = admin_key_hex
        .iter()
        .map(|key| hex::decode(key.trim()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    tx::publish::admin_permission(&admin_keys, admin_threshold)?;
    let protocol = koios.protocol_params().await?;
    let cost = tx::fees::utxo_cost(&protocol)?;
    let tip = koios.tip().await?;
    let utxos = koios.address_utxos(fee_addr).await?;
    let mut wallet = utxos
        .iter()
        .map(coin_select::WalletUtxo::try_from)
        .collect::<Result<Vec<_>>>()?;
    wallet.sort_by_key(|input| std::cmp::Reverse(input.lovelace));
    let seed = wallet
        .first()
        .context("no wallet outputs for publication")?;
    let collateral = coin_select::select_separate_collateral(
        &wallet,
        &[(seed.tx_hash.clone(), seed.output_index)],
        0,
    )?
    .clone();
    let mut params = tx::publish::PublishParams {
        seed_utxo_hash: seed.tx_hash.clone(),
        seed_utxo_index: seed.output_index,
        seed_lovelace: seed.lovelace,
        seed_assets: seed.assets.clone(),
        admin_key_hash: admin_keys[0].clone(),
        admin_keys,
        admin_threshold,
        expiration_ms: parse_time_ms(expiry)?,
        payout_upperbound_ms: parse_time_ms(payout_upperbound)?,
        vendor_expiration_ms: parse_time_ms(vendor_expiry)?,
        extra_fee_inputs: vec![],
        collateral,
        fee_addr: fee_addr.into(),
        fee: 0,
        coins_per_utxo_byte: cost,
        valid_from_slot: tip.abs_slot.saturating_sub(120),
        invalid_from_slot: tip
            .abs_slot
            .checked_add(7200)
            .context("validity slot overflow")?,
        v3_cost_model: protocol.v3_cost_model().context("no PlutusV3 cost model")?,
        network_id,
        label: label.into(),
        description: description.map(str::to_owned),
    };
    let minimum = tx::publish::minimum_output_total(&params)?;
    let fee_reserve = tx::fees::maximum_fee(&protocol, 0, true)?;
    let reference_fee_reserve = tx::fees::maximum_fee(&protocol, 0, false)?;
    let outputs_and_second_fee = minimum
        .checked_add(reference_fee_reserve)
        .context("deployment reserve overflow")?;
    let (selected, _) = coin_select::select_with_change(
        &wallet,
        fee_addr,
        cost,
        fee_reserve,
        outputs_and_second_fee,
    )?;
    let first = selected.first().context("no deployment seed selected")?;
    anyhow::ensure!(
        first.tx_hash == params.seed_utxo_hash && first.output_index == params.seed_utxo_index,
        "deployment seed changed during selection"
    );
    params.extra_fee_inputs = selected
        .iter()
        .skip(1)
        .map(|input| (*input).clone())
        .collect();
    let excluded: Vec<_> = selected
        .iter()
        .map(|input| (input.tx_hash.clone(), input.output_index))
        .collect();
    params.collateral = coin_select::select_separate_collateral(
        &wallet,
        &excluded,
        tx::fees::minimum_collateral(fee_reserve, &protocol)?,
    )?
    .clone();
    let bundle = tx::publish::build_bundle(params, &protocol)?;
    minos::preflight::transaction(
        koios,
        &bundle.publish.cbor_hex,
        &protocol,
        if network_id == 0 {
            1_666_656_000
        } else {
            1_591_566_291
        },
        &[],
    )
    .await?;
    eprintln!("Registry token (policy ID): {}", bundle.manifest.instance);
    eprintln!(
        "Treasury script hash:       {}",
        bundle.manifest.treasury_script_hash
    );
    eprintln!(
        "Vendor script hash:         {}",
        bundle.manifest.vendor_script_hash
    );
    eprintln!("Confirm publish.json before submitting vendor-reference.json; review manifest.json for hashes and dependency.");
    match out {
        Some(directory) => {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(directory)
                .context("create new deployment directory")?;
            minos::airgap::write_new(&directory.join("publish.json"), &bundle.publish)?;
            minos::airgap::write_new(
                &directory.join("vendor-reference.json"),
                &bundle.vendor_reference,
            )?;
            // Written last: absence of a manifest identifies incomplete output.
            minos::airgap::write_new(&directory.join("manifest.json"), &bundle.manifest)?;
            println!(
                "Wrote unsigned deployment bundle to {}",
                directory.display()
            );
        }
        None => println!("{}", serde_json::to_string_pretty(&bundle)?),
    }
    Ok(())
}
