/// Build an unsigned "withdraw" transaction.
///
/// Spends the vendor UTxO, partitions payouts into matured+Active (sent to destination)
/// and remaining (sent back to vendor script with updated datum).
use anyhow::{Context, Result};
use pallas_addresses::Address as PallasAddress;
use pallas_crypto::hash::Hash;
use pallas_txbuilder::{BuildConway, ExUnits, Input, Output, ScriptKind, StagingTransaction};

use super::UnsignedTx;

/// The contract requires continuation whenever value remains, even after the
/// last scheduled payout. Unscheduled tokens belong to this residual value.
pub fn continuing_datum(
    original: &crate::types::VendorDatum,
    remaining: Vec<crate::types::Payout>,
    residual: &super::value::Value,
) -> Option<Vec<u8>> {
    (!remaining.is_empty() || !residual.is_zero()).then(|| {
        crate::types::VendorDatum {
            vendor: original.vendor.clone(),
            payouts: remaining,
        }
        .to_cbor_bytes()
    })
}

#[derive(Clone)]
pub struct WithdrawParams {
    // Portal data — vendor UTxO
    pub vendor_utxo_hash: String,
    pub vendor_utxo_index: u64,
    pub vendor_address: String, // bech32 script address (used for "remaining" output)

    // Matured payouts — sent to dest_addr
    pub matured_lovelace: u64,
    pub matured_assets: Vec<(String, u64)>, // (assetId, qty)

    // Remaining payouts — sent back to vendor script with updated datum
    /// None only when no payouts and no value remain.
    pub remaining_datum_bytes: Option<Vec<u8>>,
    pub remaining_lovelace: u64,
    pub remaining_assets: Vec<(String, u64)>,

    // Reference inputs
    pub registry_utxo_hash: String, // registry NFT UTxO (required by script)
    pub registry_utxo_index: u64,
    pub vendor_script_ref_hash: String,
    pub vendor_script_ref_index: u64,

    // Required signers (vendor multisig key hashes)
    pub vendor_key_hashes: Vec<Vec<u8>>,

    // Coin selection
    pub fee_inputs: Vec<crate::coin_select::WalletUtxo>,
    pub collateral: crate::coin_select::WalletUtxo,
    pub fee_addr: String, // bech32 change address
    pub fee: u64,
    pub change_lovelace: u64,
    pub coins_per_utxo_byte: u64,

    // Destination address for matured funds
    pub dest_addr: String, // bech32

    // Validity interval
    pub valid_from_slot: u64,
    pub invalid_from_slot: u64,

    // PlutusV3 cost model (sorted alphabetically by key name)
    pub v3_cost_model: Vec<i64>,

    // Metadata
    pub instance_id: String,

    // Network (0 = testnet/preview, 1 = mainnet)
    pub network_id: u8,

    // Staking withdrawal scripts (MultisigScript::Script variants, e.g. allowlist_vendor)
    pub staking_script_hashes: Vec<[u8; 28]>,
    /// Reference inputs that provide the bytecode for each staking script
    pub staking_script_refs: Vec<(String, u64)>,
    pub staking_reward_mem: u64,
    pub staking_reward_steps: u64,
}

pub fn build(params: WithdrawParams) -> Result<UnsignedTx> {
    let mut wallet_change = params.change_lovelace;
    anyhow::ensure!(
        params.staking_script_hashes.len() <= 1,
        "multiple reward withdrawals are unsupported"
    );
    // -----------------------------------------------------------------------
    // 1. Encode CIP-100 metadata and compute hash
    // -----------------------------------------------------------------------
    let author = params
        .vendor_key_hashes
        .first()
        .context("withdrawal needs a vendor author key")?;
    let (aux_data_cbor, aux_data_hash) =
        super::metadata::encode_withdraw(&params.instance_id, &hex::encode(author))?;

    // -----------------------------------------------------------------------
    // 2. Parse addresses
    // -----------------------------------------------------------------------
    let dest_addr_bytes = PallasAddress::from_bech32(&params.dest_addr)
        .context("invalid dest address")?
        .to_vec();
    let dest_pallas_addr =
        PallasAddress::from_bytes(&dest_addr_bytes).context("re-parse dest address")?;

    let vendor_addr_bytes = PallasAddress::from_bech32(&params.vendor_address)
        .context("invalid vendor address")?
        .to_vec();
    let vendor_pallas_addr =
        PallasAddress::from_bytes(&vendor_addr_bytes).context("re-parse vendor address")?;

    let fee_addr_bytes = PallasAddress::from_bech32(&params.fee_addr)
        .context("invalid fee address")?
        .to_vec();
    let fee_pallas_addr =
        PallasAddress::from_bytes(&fee_addr_bytes).context("re-parse fee address")?;

    // -----------------------------------------------------------------------
    // 3. Parse the vendor UTxO input
    // -----------------------------------------------------------------------
    let vendor_hash_bytes: [u8; 32] = hex::decode(&params.vendor_utxo_hash)
        .context("vendor utxo hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("vendor utxo hash must be 32 bytes"))?;
    let vendor_input = Input::new(Hash::from(vendor_hash_bytes), params.vendor_utxo_index);

    // -----------------------------------------------------------------------
    // 4. Parse reference inputs
    // -----------------------------------------------------------------------
    let registry_ref_hash_bytes: [u8; 32] = hex::decode(&params.registry_utxo_hash)
        .context("registry utxo hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("registry utxo hash must be 32 bytes"))?;
    let registry_ref_input = Input::new(
        Hash::from(registry_ref_hash_bytes),
        params.registry_utxo_index,
    );

    let vendor_ref_hash_bytes: [u8; 32] = hex::decode(&params.vendor_script_ref_hash)
        .context("vendor script ref hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("vendor script ref hash must be 32 bytes"))?;
    let vendor_ref_input = Input::new(
        Hash::from(vendor_ref_hash_bytes),
        params.vendor_script_ref_index,
    );

    // -----------------------------------------------------------------------
    // 5. Build destination output (matured funds)
    // -----------------------------------------------------------------------
    let dest_output = super::fees::funded_output(
        params.coins_per_utxo_byte,
        params.matured_lovelace,
        &mut wallet_change,
        |coin| {
            super::value::add_assets(
                Output::new(dest_pallas_addr.clone(), coin),
                &params.matured_assets,
            )
        },
    )?;

    // -----------------------------------------------------------------------
    // 6. Optionally build "remaining" output (back to vendor script)
    // -----------------------------------------------------------------------
    let remaining_output: Option<Output> = if let Some(datum_bytes) = params.remaining_datum_bytes {
        let out = super::fees::funded_output(
            params.coins_per_utxo_byte,
            params.remaining_lovelace,
            &mut wallet_change,
            |coin| {
                super::value::add_assets(
                    Output::new(vendor_pallas_addr.clone(), coin)
                        .set_inline_datum(datum_bytes.clone()),
                    &params.remaining_assets,
                )
            },
        )?;
        Some(out)
    } else {
        None
    };

    // -----------------------------------------------------------------------
    // 7. Build change output
    // -----------------------------------------------------------------------
    let change_output =
        crate::coin_select::change_output(fee_pallas_addr, wallet_change, &params.fee_inputs)?;

    // -----------------------------------------------------------------------
    // 8. Parse collateral
    // -----------------------------------------------------------------------
    let coll_hash_bytes: [u8; 32] = hex::decode(&params.collateral.tx_hash)
        .context("collateral hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("collateral hash must be 32 bytes"))?;
    let collateral_input = Input::new(Hash::from(coll_hash_bytes), params.collateral.output_index);

    // -----------------------------------------------------------------------
    // 9. Redeemer — Withdraw = Constr 0 [] = d87980
    // -----------------------------------------------------------------------
    let redeemer_bytes = hex::decode("d87980").unwrap();

    // -----------------------------------------------------------------------
    // 10. Build staging transaction
    // -----------------------------------------------------------------------
    // Deduplicate reference inputs (registry and vendor_ref may be the same UTxO)
    let mut ref_inputs = vec![registry_ref_input];
    if params.vendor_script_ref_hash != params.registry_utxo_hash
        || params.vendor_script_ref_index != params.registry_utxo_index
    {
        ref_inputs.push(vendor_ref_input);
    }
    // Staking script reference inputs (allowlist_vendor bytecode)
    for (ref_hash_str, ref_idx) in &params.staking_script_refs {
        let h: [u8; 32] = hex::decode(ref_hash_str)
            .context("staking script ref hash")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("staking script ref hash must be 32 bytes"))?;
        ref_inputs.push(Input::new(Hash::from(h), *ref_idx));
    }

    let mut staging = StagingTransaction::new().input(vendor_input.clone());
    for ri in ref_inputs {
        staging = staging.reference_input(ri);
    }
    let mut staging = staging
        .output(dest_output)
        .collateral_input(collateral_input)
        .valid_from_slot(params.valid_from_slot)
        .invalid_from_slot(params.invalid_from_slot)
        .fee(params.fee)
        .network_id(params.network_id)
        .add_spend_redeemer(
            vendor_input,
            redeemer_bytes,
            Some(ExUnits {
                mem: 2_500_000,
                steps: 900_000_000,
            }),
        )
        .add_language(ScriptKind::PlutusV3, params.v3_cost_model.clone());

    if let Some(rem_out) = remaining_output {
        staging = staging.output(rem_out);
    }

    staging = staging.output(change_output);

    // Fee inputs
    for fi in &params.fee_inputs {
        let h: [u8; 32] = hex::decode(&fi.tx_hash)
            .context("fee input hash")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("fee input hash must be 32 bytes"))?;
        staging = staging.input(Input::new(Hash::from(h), fi.output_index));
    }

    // Required signers
    for kh in &params.vendor_key_hashes {
        let kh_arr: [u8; 28] = kh
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("key_hash must be 28 bytes, got {}", kh.len()))?;
        staging = staging.disclosed_signer(Hash::from(kh_arr));
    }

    // -----------------------------------------------------------------------
    // 11. Build raw tx and post-process to add aux_data
    // -----------------------------------------------------------------------
    let built = staging
        .build_conway_raw()
        .map_err(|e| anyhow::anyhow!("build_conway_raw: {e}"))?;

    let tx_cbor = &built.tx_bytes.0;
    let mut final_cbor = super::cbor_util::add_aux_data(tx_cbor, &aux_data_cbor, &aux_data_hash)
        .context("add_aux_data")?;

    // Only one reward script is supported, with redeemer index zero.
    for script_hash in &params.staking_script_hashes {
        final_cbor = super::cbor_util::add_treasury_withdrawal(
            &final_cbor,
            script_hash,
            params.network_id,
            params.staking_reward_mem,
            params.staking_reward_steps,
            &params.v3_cost_model,
        )
        .context("add_treasury_withdrawal")?;
    }

    Ok(UnsignedTx::new(
        "withdraw matured payouts".to_string(),
        hex::encode(final_cbor),
    ))
}
