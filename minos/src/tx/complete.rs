/// Build an unsigned "complete" transaction (zero-withdrawal with CIP-100 metadata).
///
/// Spends the vendor UTxO with a `Withdraw` redeemer, sends full value back to the
/// vendor script address with the same datum, and attaches on-chain evidence metadata.
use anyhow::{Context, Result};
use pallas_addresses::Address as PallasAddress;
use pallas_crypto::hash::Hash;
use pallas_txbuilder::{BuildConway, ExUnits, Input, Output, ScriptKind, StagingTransaction};

use super::UnsignedTx;

#[derive(Clone)]
pub struct CompleteParams {
    // Portal data — vendor UTxO
    pub vendor_utxo_hash: String,
    pub vendor_utxo_index: u64,
    pub vendor_address: String,   // bech32 script address
    pub vendor_datum_hex: String, // raw datum hex → inline datum on the output
    pub vendor_value_lovelace: u64,
    pub vendor_value_assets: Vec<(String, u64)>, // (assetId = policyId+assetName hex, qty)

    // Reference inputs
    pub registry_utxo_hash: String, // registry NFT UTxO (required by vendor script)
    pub registry_utxo_index: u64,
    pub vendor_script_ref_hash: String,
    pub vendor_script_ref_index: u64,
    pub treasury_script_ref_hash: String,
    pub treasury_script_ref_index: u64,

    // Required signers (vendor multisig key hashes)
    pub vendor_key_hashes: Vec<Vec<u8>>,

    // Coin selection
    pub fee_inputs: Vec<crate::coin_select::WalletUtxo>,
    pub collateral: crate::coin_select::WalletUtxo,
    pub fee_addr: String, // bech32 change address
    pub fee: u64,
    pub change_lovelace: u64,

    // Validity interval
    pub valid_from_slot: u64,
    pub invalid_from_slot: u64,

    // PlutusV3 cost model (sorted alphabetically by key name)
    pub v3_cost_model: Vec<i64>,

    // Metadata
    pub instance_id: String,
    pub project_id: String,
    pub milestone_id: String,
    pub description: String,
    pub evidence_urls: Vec<String>,

    // Network (0 = testnet/preview, 1 = mainnet)
    pub network_id: u8,
}

pub fn build(params: CompleteParams) -> Result<UnsignedTx> {
    // -----------------------------------------------------------------------
    // 1. Encode CIP-100 metadata and compute hash
    // -----------------------------------------------------------------------
    let (aux_data_cbor, aux_data_hash) = super::metadata::encode_complete(
        &params.instance_id,
        &params.project_id,
        &params.milestone_id,
        &params.description,
        &params.evidence_urls,
    )?;

    // -----------------------------------------------------------------------
    // 2. Parse addresses
    // -----------------------------------------------------------------------
    let vendor_addr_bytes = PallasAddress::from_bech32(&params.vendor_address)
        .context("invalid vendor address")?
        .to_vec();
    let vendor_addr =
        PallasAddress::from_bytes(&vendor_addr_bytes).context("re-parse vendor address")?;

    let fee_addr_bytes = PallasAddress::from_bech32(&params.fee_addr)
        .context("invalid fee address")?
        .to_vec();
    let fee_addr = PallasAddress::from_bytes(&fee_addr_bytes).context("re-parse fee address")?;

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

    let treasury_ref_hash_bytes: [u8; 32] = hex::decode(&params.treasury_script_ref_hash)
        .context("treasury script ref hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("treasury script ref hash must be 32 bytes"))?;
    let treasury_ref_input = Input::new(
        Hash::from(treasury_ref_hash_bytes),
        params.treasury_script_ref_index,
    );

    // -----------------------------------------------------------------------
    // 5. Build vendor output (full value, same datum back to script)
    // -----------------------------------------------------------------------
    let datum_bytes = hex::decode(&params.vendor_datum_hex).context("vendor datum hex decode")?;

    let mut vendor_output =
        Output::new(vendor_addr, params.vendor_value_lovelace).set_inline_datum(datum_bytes);

    for (asset_id, qty) in &params.vendor_value_assets {
        if asset_id.is_empty() {
            continue; // lovelace already handled
        }
        anyhow::ensure!(asset_id.len() >= 56, "asset_id too short: {asset_id}");
        let policy_bytes: [u8; 28] = hex::decode(&asset_id[..56])
            .context("asset policy_id decode")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("policy_id must be 28 bytes"))?;
        let asset_name = hex::decode(&asset_id[56..]).context("asset name decode")?;
        vendor_output = vendor_output
            .add_asset(Hash::from(policy_bytes), asset_name, *qty)
            .context("add_asset on vendor output")?;
    }

    // -----------------------------------------------------------------------
    // 6. Build change output
    // -----------------------------------------------------------------------
    let change_output = Output::new(fee_addr, params.change_lovelace);

    // -----------------------------------------------------------------------
    // 7. Parse collateral
    // -----------------------------------------------------------------------
    let coll_hash_bytes: [u8; 32] = hex::decode(&params.collateral.tx_hash)
        .context("collateral hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("collateral hash must be 32 bytes"))?;
    let collateral_input = Input::new(Hash::from(coll_hash_bytes), params.collateral.output_index);

    // -----------------------------------------------------------------------
    // 8. Redeemer — Withdraw = Constr 0 [] = d87980
    // -----------------------------------------------------------------------
    let redeemer_bytes = hex::decode("d87980").unwrap();

    // -----------------------------------------------------------------------
    // 9. Build staging transaction
    // -----------------------------------------------------------------------
    let mut staging = StagingTransaction::new()
        // Script input
        .input(vendor_input.clone())
        // Reference inputs
        .reference_input(registry_ref_input)
        .reference_input(vendor_ref_input)
        .reference_input(treasury_ref_input)
        // Vendor output (full value back to script)
        .output(vendor_output)
        // Change output
        .output(change_output)
        // Collateral
        .collateral_input(collateral_input)
        // Validity
        .valid_from_slot(params.valid_from_slot)
        .invalid_from_slot(params.invalid_from_slot)
        // Fee
        .fee(params.fee)
        // Network
        .network_id(params.network_id)
        // Redeemer for vendor input
        .add_spend_redeemer(
            vendor_input,
            redeemer_bytes,
            Some(ExUnits {
                mem: 3_000_000,
                steps: 800_000_000,
            }),
        )
        // Language view for script_data_hash
        .add_language(ScriptKind::PlutusV3, params.v3_cost_model);

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
    // 10. Build raw tx, then post-process to add aux_data
    // -----------------------------------------------------------------------
    let built = staging
        .build_conway_raw()
        .map_err(|e| anyhow::anyhow!("build_conway_raw: {e}"))?;

    let tx_cbor = &built.tx_bytes.0;

    let final_cbor = super::cbor_util::add_aux_data(tx_cbor, &aux_data_cbor, &aux_data_hash)
        .context("add_aux_data")?;

    Ok(UnsignedTx::new(
        format!("complete milestone {}", params.milestone_id),
        hex::encode(final_cbor),
    ))
}
