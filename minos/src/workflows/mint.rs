//! Mint command workflow, independent of CLI argument parsing.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_mint(
    koios: &DataSource,
    fee_addr: &str,
    admin_key_hex: &str,
    ticker: &str,
    decimals: u64,
    amount: u64,
    existing_asset: bool,
    dest_addr: &str,
    out: Option<&std::path::Path>,
    network_id: u8,
) -> Result<()> {
    let admin_key_hash = hex::decode(admin_key_hex.trim()).context("admin_key hex")?;
    anyhow::ensure!(admin_key_hash.len() == 28, "admin_key must be 28 bytes");
    let protocol = koios.protocol_params().await?;
    let cost = tx::fees::utxo_cost(&protocol)?;
    let tip = koios.tip().await?;
    let mut params = tx::mint::MintParams {
        admin_key_hash,
        ft_amount: amount,
        mint_reference: !existing_asset,
        token_ticker: ticker.into(),
        token_decimals: decimals,
        inputs: vec![],
        dest_addr: dest_addr.into(),
        fee_addr: fee_addr.into(),
        fee: 0,
        coins_per_utxo_byte: cost,
        valid_from_slot: tip.abs_slot.saturating_sub(120),
        invalid_from_slot: tip
            .abs_slot
            .checked_add(7200)
            .context("validity slot overflow")?,
        network_id,
    };
    let output_coin = tx::mint::minimum_output_coin(&params)?;
    let source = pallas_addresses::Address::from_bech32(fee_addr)?;
    let minimum_change = tx::fees::minimum_builder_coin(cost, |coin| {
        Ok(pallas_txbuilder::Output::new(source.clone(), coin))
    })?;
    // Native minting has no execution/reference fee; reserve the protocol's
    // maximum linear fee while selecting, then measure the actual transaction.
    let fee_reserve = protocol
        .min_fee_a
        .checked_mul(protocol.max_tx_size)
        .and_then(|fee| fee.checked_add(protocol.min_fee_b))
        .context("fee reserve overflow")?;
    let target = output_coin
        .checked_add(minimum_change)
        .and_then(|coin| coin.checked_add(fee_reserve))
        .context("mint target overflow")?;
    let utxos = koios.address_utxos(fee_addr).await?;
    let candidates = utxos
        .iter()
        .filter(|utxo| !utxo.has_native_assets())
        .map(coin_select::WalletUtxo::try_from)
        .collect::<Result<Vec<_>>>()?;
    let (selected, _) = coin_select::select_inputs(&candidates, target)?;
    let total = selected.iter().try_fold(0u64, |total, input| {
        total
            .checked_add(input.lovelace)
            .context("mint input ADA overflow")
    })?;
    params.inputs = selected
        .iter()
        .map(|input| tx::mint::MintInput {
            tx_hash: input.tx_hash.clone(),
            output_index: input.output_index,
            lovelace: input.lovelace,
        })
        .collect();
    let mut result = tx::mint::build(params.clone())?;
    result.unsigned_tx = tx::fees::balance(
        &protocol,
        total
            .checked_sub(output_coin)
            .context("insufficient mint funds")?,
        0,
        2,
        |fee, _change| {
            params.fee = fee;
            Ok(tx::mint::build(params.clone())?.unsigned_tx)
        },
    )?;
    minos::preflight::transaction(
        koios,
        &result.unsigned_tx.cbor_hex,
        &protocol,
        if network_id == 0 {
            1_666_656_000
        } else {
            1_591_566_291
        },
        &[],
    )
    .await?;
    eprintln!("Policy ID:          {}", result.policy_id_hex);
    eprintln!("FT asset name:      {}", result.ft_asset_name_hex);
    eprintln!("Ref NFT asset name: {}", result.ref_nft_asset_name_hex);
    eprintln!(
        "Full FT asset ID:   {}{}",
        result.policy_id_hex, result.ft_asset_name_hex
    );
    write_tx(result.unsigned_tx, out)
}
