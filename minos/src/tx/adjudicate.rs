/// Build an unsigned "adjudicate" transaction (pause or resume specific payouts).
///
/// Adjudicate lets the oversight committee (pause permission) or vendor (resume permission)
/// change the status of individual payouts without moving funds.
///
/// Invariants enforced by the script:
/// - Same number of payouts in / out
/// - Each payout's maturation and value are unchanged
/// - A matured, Active payout cannot be paused (validity_range must NOT be entirely past its maturation)
/// - Interval length ≤ 36 hours
use anyhow::{Context, Result};
use pallas_addresses::Address as PallasAddress;
use pallas_codec::minicbor;
use pallas_codec::utils::MaybeIndefArray;
use pallas_crypto::hash::Hash;
use pallas_primitives::conway::{Constr, PlutusData};
use pallas_txbuilder::{BuildConway, ExUnits, Input, Output, ScriptKind, StagingTransaction};

use super::cbor_util;
use super::UnsignedTx;
use crate::coin_select::WalletUtxo;
use crate::types::{PayoutStatus, VendorDatum};

#[derive(Clone)]
pub struct AdjudicateParams {
    // Vendor UTxO being spent
    pub vendor_utxo_hash: String,
    pub vendor_utxo_index: u64,
    pub vendor_address: String, // bech32 self-staking script address

    // Current on-chain datum (parsed); used to build the new datum
    pub current_datum: VendorDatum,

    // New status for each payout in order (length must equal payouts length)
    pub new_statuses: Vec<PayoutStatus>,

    // Vendor UTxO value — carried through unchanged to the output
    pub vendor_lovelace: u64,
    pub vendor_assets: Vec<(String, u64)>, // (assetId hex = policy(56) + name(hex), qty)

    // Reference inputs
    pub registry_utxo_hash: String,
    pub registry_utxo_index: u64,
    pub vendor_script_ref_hash: String,
    pub vendor_script_ref_index: u64,
    pub treasury_script_ref_hash: String,
    pub treasury_script_ref_index: u64,

    // Required signer — the pause (or resume) permission key hash
    pub admin_key_hash: Vec<u8>,

    // Fee wallet
    pub fee_inputs: Vec<WalletUtxo>,
    pub collateral: WalletUtxo,
    pub fee_addr: String,
    pub fee: u64,
    pub change_lovelace: u64,

    // Validity interval
    // valid_from_slot must correspond to a POSIX time ≤ the earliest maturation
    // of any Active payout we want to pause (so is_entirely_after is False).
    pub valid_from_slot: u64,
    pub invalid_from_slot: u64,

    // PlutusV3 cost model
    pub v3_cost_model: Vec<i64>,

    // Metadata
    pub instance_id: String,

    pub network_id: u8,
}

pub fn build(params: AdjudicateParams) -> Result<UnsignedTx> {
    anyhow::ensure!(
        params.new_statuses.len() == params.current_datum.payouts.len(),
        "new_statuses length ({}) must equal payouts length ({})",
        params.new_statuses.len(),
        params.current_datum.payouts.len()
    );

    // -----------------------------------------------------------------------
    // 1. Addresses
    // -----------------------------------------------------------------------
    let vendor_addr_bytes = PallasAddress::from_bech32(&params.vendor_address)
        .context("vendor address bech32")?
        .to_vec();
    let vendor_pallas_addr =
        PallasAddress::from_bytes(&vendor_addr_bytes).context("vendor address bytes")?;

    let fee_addr_bytes = PallasAddress::from_bech32(&params.fee_addr)
        .context("fee address bech32")?
        .to_vec();
    let fee_pallas_addr =
        PallasAddress::from_bytes(&fee_addr_bytes).context("fee address bytes")?;

    // -----------------------------------------------------------------------
    // 2. Build new VendorDatum (same payouts, updated statuses)
    // -----------------------------------------------------------------------
    let new_payouts: Vec<crate::types::Payout> = params
        .current_datum
        .payouts
        .into_iter()
        .zip(params.new_statuses.iter())
        .map(|(mut p, status)| {
            p.status = status.clone();
            p
        })
        .collect();

    let new_datum = VendorDatum {
        vendor: params.current_datum.vendor.clone(),
        payouts: new_payouts,
    };
    let new_datum_bytes = new_datum.to_cbor_bytes();

    // -----------------------------------------------------------------------
    // 3. Build vendor output (same value, new datum)
    // -----------------------------------------------------------------------
    let mut vendor_output =
        Output::new(vendor_pallas_addr, params.vendor_lovelace).set_inline_datum(new_datum_bytes);

    for (asset_id, qty) in &params.vendor_assets {
        if asset_id.is_empty() || *qty == 0 {
            continue;
        }
        anyhow::ensure!(asset_id.len() >= 56, "asset_id too short: {asset_id}");
        let policy_bytes: [u8; 28] = hex::decode(&asset_id[..56])
            .context("asset policy decode")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("policy must be 28 bytes"))?;
        let asset_name = hex::decode(&asset_id[56..]).context("asset name decode")?;
        vendor_output = vendor_output
            .add_asset(Hash::from(policy_bytes), asset_name, *qty)
            .context("add asset to vendor output")?;
    }

    // -----------------------------------------------------------------------
    // 4. Change output
    // -----------------------------------------------------------------------
    let change_output = Output::new(fee_pallas_addr, params.change_lovelace);

    // -----------------------------------------------------------------------
    // 5. Parse vendor UTxO input
    // -----------------------------------------------------------------------
    let vendor_hash_bytes: [u8; 32] = hex::decode(&params.vendor_utxo_hash)
        .context("vendor utxo hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("vendor utxo hash must be 32 bytes"))?;
    let vendor_input = Input::new(Hash::from(vendor_hash_bytes), params.vendor_utxo_index);

    // -----------------------------------------------------------------------
    // 6. Parse reference inputs
    // -----------------------------------------------------------------------
    let registry_hash: [u8; 32] = hex::decode(&params.registry_utxo_hash)
        .context("registry hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("registry hash must be 32 bytes"))?;
    let registry_ref_input = Input::new(Hash::from(registry_hash), params.registry_utxo_index);

    let vendor_ref_hash: [u8; 32] = hex::decode(&params.vendor_script_ref_hash)
        .context("vendor script ref hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("vendor script ref hash must be 32 bytes"))?;
    let vendor_ref_input = Input::new(Hash::from(vendor_ref_hash), params.vendor_script_ref_index);

    let treasury_ref_hash: [u8; 32] = hex::decode(&params.treasury_script_ref_hash)
        .context("treasury script ref hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("treasury script ref hash must be 32 bytes"))?;
    let treasury_ref_input = Input::new(
        Hash::from(treasury_ref_hash),
        params.treasury_script_ref_index,
    );

    // -----------------------------------------------------------------------
    // 7. Collateral
    // -----------------------------------------------------------------------
    let coll_hash: [u8; 32] = hex::decode(&params.collateral.tx_hash)
        .context("collateral hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("collateral hash must be 32 bytes"))?;
    let collateral_input = Input::new(Hash::from(coll_hash), params.collateral.output_index);

    // -----------------------------------------------------------------------
    // 8. Adjudicate redeemer: Constr 1 [Array<statuses>]
    //    Adjudicate { statuses } is constructor 1 of VendorSpendRedeemer
    //    Active = Constr 0 (tag 121) = d87980
    //    Paused = Constr 1 (tag 122) = d87a80
    // -----------------------------------------------------------------------
    let status_items: Vec<PlutusData> = params
        .new_statuses
        .iter()
        .map(|s| {
            let tag: u64 = match s {
                PayoutStatus::Active => 121,
                PayoutStatus::Paused => 122,
            };
            PlutusData::Constr(Constr {
                tag,
                any_constructor: None,
                fields: MaybeIndefArray::Def(vec![]),
            })
        })
        .collect();

    let redeemer_data = PlutusData::Constr(Constr {
        tag: 122, // Constr 1 = Adjudicate
        any_constructor: None,
        fields: MaybeIndefArray::Def(vec![PlutusData::Array(MaybeIndefArray::Def(status_items))]),
    });
    let redeemer_bytes =
        minicbor::to_vec(redeemer_data).map_err(|e| anyhow::anyhow!("redeemer encode: {e}"))?;

    // -----------------------------------------------------------------------
    // 9. Required signer
    // -----------------------------------------------------------------------
    let admin_hash_arr: [u8; 28] = params
        .admin_key_hash
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("admin_key_hash must be 28 bytes"))?;
    let admin_hash: Hash<28> = Hash::from(admin_hash_arr);

    // -----------------------------------------------------------------------
    // 10. Build staging transaction
    // -----------------------------------------------------------------------
    let mut staging = StagingTransaction::new()
        .input(vendor_input.clone())
        .reference_input(registry_ref_input)
        .reference_input(vendor_ref_input)
        .reference_input(treasury_ref_input)
        .output(vendor_output)
        .output(change_output)
        .collateral_input(collateral_input)
        .fee(params.fee)
        .valid_from_slot(params.valid_from_slot)
        .invalid_from_slot(params.invalid_from_slot)
        .network_id(params.network_id)
        .disclosed_signer(admin_hash)
        .add_spend_redeemer(
            vendor_input,
            redeemer_bytes,
            Some(ExUnits {
                mem: 3_000_000,
                steps: 800_000_000,
            }),
        )
        .add_language(ScriptKind::PlutusV3, params.v3_cost_model);

    for fi in &params.fee_inputs {
        let h: [u8; 32] = hex::decode(&fi.tx_hash)
            .context("fee input hash")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("fee input hash must be 32 bytes"))?;
        staging = staging.input(Input::new(Hash::from(h), fi.output_index));
    }

    // -----------------------------------------------------------------------
    // 11. Build raw tx and inject metadata
    // -----------------------------------------------------------------------
    let built = staging
        .build_conway_raw()
        .map_err(|e| anyhow::anyhow!("build_conway_raw: {e}"))?;

    let (aux_cbor, aux_hash) = super::metadata::encode_adjudicate(&params.instance_id)?;
    let final_bytes = cbor_util::add_aux_data(&built.tx_bytes.0, &aux_cbor, &aux_hash)?;

    Ok(UnsignedTx::new(
        "adjudicate vendor payouts",
        hex::encode(final_bytes),
    ))
}
