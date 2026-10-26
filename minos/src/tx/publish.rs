/// Build the three unsigned transactions that deploy a new treasury instance.
///
/// All three are combined into ONE transaction for atomicity:
///   1. Spends seed UTxO (consumed to guarantee oneshot NFT uniqueness)
///   2. Mints REGISTRY NFT under the oneshot policy
///   3. Sends REGISTRY NFT → oneshot enterprise address (datum = ScriptHashRegistry)
///   4. Sends min-ADA → treasury base address with treasury script as reference
///   5. Sends min-ADA → vendor base address with vendor script as reference
///   6. Change → fee_addr
use anyhow::{Context, Result};
use pallas_addresses::Address as PallasAddress;
use pallas_crypto::hash::Hash;
use pallas_txbuilder::{BuildConway, ExUnits, Input, Output, ScriptKind, StagingTransaction};

use super::UnsignedTx;
use crate::scripts::{
    base_script_addr, encode_script_hash_registry, enterprise_script_addr, parameterize_oneshot,
    parameterize_treasury, parameterize_vendor, script_hash, TreasuryPermissions,
    VendorPermissions,
};
use crate::types::MultisigScript;

// Asset name for the registry NFT: "REGISTRY" = 0x5245474953545259
const REGISTRY_ASSET_NAME: &[u8] = b"REGISTRY";

// Min-ADA for each reference script output scales with script size.
// Treasury script ≈ 5.3 kB flat → ~25 ADA; vendor ≈ 7.9 kB → ~36 ADA.
const TREASURY_REF_ADA: u64 = 25_000_000;
const VENDOR_REF_ADA: u64 = 36_000_000;
// Minimum lovelace for the registry NFT output (2 ADA)
const REGISTRY_OUTPUT_ADA: u64 = 2_000_000;

pub struct PublishParams {
    // Seed UTxO — MUST be included in inputs; parameterizes the oneshot minting policy.
    pub seed_utxo_hash: String,
    pub seed_utxo_index: u64,

    // Admin: single key hash used for ALL permissions (treasury + vendor).
    pub admin_key_hash: Vec<u8>,
    /// All administrator keys; txAuthor must be one of them.
    pub admin_keys: Vec<Vec<u8>>,
    pub admin_threshold: u64,

    // Instance timings (POSIX milliseconds)
    pub expiration_ms: u64,
    pub payout_upperbound_ms: u64,
    pub vendor_expiration_ms: u64,

    // Metadata (CIP-100, INewInstance body for portal auto-discovery)
    pub label: String,
    pub description: Option<String>,

    // Coin selection (fee inputs other than seed; seed is added separately)
    pub extra_fee_inputs: Vec<crate::coin_select::WalletUtxo>,
    pub collateral: crate::coin_select::WalletUtxo,
    pub fee_addr: String,
    pub fee: u64,
    pub change_lovelace: u64,

    // Validity interval
    pub valid_from_slot: u64,
    pub invalid_from_slot: u64,

    // PlutusV3 cost model
    pub v3_cost_model: Vec<i64>,

    // Network (0 = testnet/preview, 1 = mainnet)
    pub network_id: u8,
}

pub fn admin_permission(keys: &[Vec<u8>], threshold: u64) -> Result<MultisigScript> {
    anyhow::ensure!(!keys.is_empty(), "at least one administrator is required");
    anyhow::ensure!(
        threshold > 0 && threshold <= keys.len() as u64,
        "admin threshold must be between 1 and the number of administrators"
    );
    let mut unique = std::collections::BTreeSet::new();
    for key in keys {
        anyhow::ensure!(key.len() == 28, "admin keys must be 28 bytes");
        anyhow::ensure!(unique.insert(key), "duplicate administrator key");
    }
    let scripts: Vec<_> = keys
        .iter()
        .map(|key| MultisigScript::Signature {
            key_hash: key.clone(),
        })
        .collect();
    Ok(if scripts.len() == 1 {
        scripts[0].clone()
    } else {
        MultisigScript::AtLeast {
            required: threshold,
            scripts,
        }
    })
}

pub fn build(params: PublishParams) -> Result<UnsignedTx> {
    let admin_sig = admin_permission(&params.admin_keys, params.admin_threshold)?;
    anyhow::ensure!(
        params.admin_keys.contains(&params.admin_key_hash),
        "transaction author must be an administrator"
    );
    // -----------------------------------------------------------------------
    // 1. Parse seed UTxO hash
    // -----------------------------------------------------------------------
    let seed_hash_bytes: [u8; 32] = hex::decode(&params.seed_utxo_hash)
        .context("seed utxo hash hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("seed utxo hash must be 32 bytes"))?;

    // -----------------------------------------------------------------------
    // 2. Derive all scripts
    // -----------------------------------------------------------------------
    let treasury_perms = TreasuryPermissions {
        reorganize: admin_sig.clone(),
        sweep: admin_sig.clone(),
        fund: admin_sig.clone(),
        disburse: admin_sig.clone(),
    };
    let vendor_perms = VendorPermissions {
        pause: admin_sig.clone(),
        resume: admin_sig.clone(),
        modify: admin_sig.clone(),
    };

    let oneshot_cbor = parameterize_oneshot(&seed_hash_bytes, params.seed_utxo_index)
        .context("parameterize oneshot")?;
    let oneshot_hash = script_hash(&oneshot_cbor);
    let registry_token: [u8; 28] = *oneshot_hash;

    let treasury_cbor = parameterize_treasury(
        &registry_token,
        &treasury_perms,
        params.expiration_ms,
        params.payout_upperbound_ms,
    )
    .context("parameterize treasury")?;
    let treasury_hash = script_hash(&treasury_cbor);

    let vendor_cbor =
        parameterize_vendor(&registry_token, &vendor_perms, params.vendor_expiration_ms)
            .context("parameterize vendor")?;
    let vendor_hash = script_hash(&vendor_cbor);

    let registry_datum = encode_script_hash_registry(&treasury_hash, &vendor_hash);

    // -----------------------------------------------------------------------
    // 3. Build addresses
    // -----------------------------------------------------------------------
    let oneshot_addr_obj = enterprise_script_addr(&oneshot_hash, params.network_id);
    let oneshot_addr_bytes = oneshot_addr_obj.to_vec();
    let oneshot_addr = PallasAddress::from_bytes(&oneshot_addr_bytes).context("oneshot addr")?;

    let treasury_addr_obj = base_script_addr(&treasury_hash, params.network_id);
    let treasury_addr_bytes = treasury_addr_obj.to_vec();
    let treasury_addr = PallasAddress::from_bytes(&treasury_addr_bytes).context("treasury addr")?;

    let vendor_addr_obj = base_script_addr(&vendor_hash, params.network_id);
    let vendor_addr_bytes = vendor_addr_obj.to_vec();
    let vendor_addr = PallasAddress::from_bytes(&vendor_addr_bytes).context("vendor addr")?;

    let fee_addr_bytes = PallasAddress::from_bech32(&params.fee_addr)
        .context("fee addr bech32")?
        .to_vec();
    let fee_addr = PallasAddress::from_bytes(&fee_addr_bytes).context("fee addr bytes")?;

    // -----------------------------------------------------------------------
    // 4. Inputs
    // -----------------------------------------------------------------------
    let seed_input = Input::new(Hash::from(seed_hash_bytes), params.seed_utxo_index);

    // -----------------------------------------------------------------------
    // 5. Outputs
    //    (a) Registry NFT → oneshot address with ScriptHashRegistry datum
    //    (b) Treasury reference script output
    //    (c) Vendor reference script output
    //    (d) Change
    // -----------------------------------------------------------------------
    let registry_output = Output::new(oneshot_addr, REGISTRY_OUTPUT_ADA)
        .add_asset(oneshot_hash, REGISTRY_ASSET_NAME.to_vec(), 1)
        .context("add REGISTRY asset to output")?
        .set_inline_datum(registry_datum);

    // Pass CBOR-wrapped flat bytes to pallas-txbuilder.
    // The Cardano node's deserialiseScript CBOR-decodes the bytes before flat-decoding,
    // so scripts must be passed in their CBOR-wrapped form (59xxxx flat_bytes), not raw flat.
    let treasury_ref_output = Output::new(treasury_addr, TREASURY_REF_ADA)
        .set_inline_script(ScriptKind::PlutusV3, treasury_cbor.clone());

    let vendor_ref_output = Output::new(vendor_addr, VENDOR_REF_ADA)
        .set_inline_script(ScriptKind::PlutusV3, vendor_cbor.clone());

    let change_output = Output::new(fee_addr, params.change_lovelace);

    // -----------------------------------------------------------------------
    // 6. Collateral
    // -----------------------------------------------------------------------
    let coll_hash_bytes: [u8; 32] = hex::decode(&params.collateral.tx_hash)
        .context("collateral hash")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("collateral hash must be 32 bytes"))?;
    let collateral_input = Input::new(Hash::from(coll_hash_bytes), params.collateral.output_index);

    // -----------------------------------------------------------------------
    // 7. Redeemer for oneshot minting policy (Void = Constr 0 [])
    // -----------------------------------------------------------------------
    let mint_redeemer = hex::decode("d87980").unwrap();

    // -----------------------------------------------------------------------
    // 8. Build staging transaction
    // -----------------------------------------------------------------------
    let admin_hash_arr: [u8; 28] = params
        .admin_key_hash
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("admin_key_hash must be 28 bytes"))?;
    let admin_hash = Hash::from(admin_hash_arr);

    let mut staging = StagingTransaction::new()
        // Seed UTxO (consumed to validate oneshot mint)
        .input(seed_input)
        // Registry NFT output (with inline datum)
        .output(registry_output)
        // Treasury reference script output
        .output(treasury_ref_output)
        // Vendor reference script output
        .output(vendor_ref_output)
        // Change back to fee wallet
        .output(change_output)
        // Collateral (required for Plutus scripts)
        .collateral_input(collateral_input)
        // Validity
        .valid_from_slot(params.valid_from_slot)
        .invalid_from_slot(params.invalid_from_slot)
        // Fee
        .fee(params.fee)
        // Network
        .network_id(params.network_id)
        // Mint REGISTRY NFT
        .mint_asset(oneshot_hash, REGISTRY_ASSET_NAME.to_vec(), 1)
        .context("mint_asset")?
        // Minting redeemer
        .add_mint_redeemer(
            oneshot_hash,
            mint_redeemer,
            Some(ExUnits {
                mem: 14_000_000,
                steps: 10_000_000_000,
            }),
        )
        // Oneshot script in witness set (CBOR-wrapped flat bytes)
        .script(ScriptKind::PlutusV3, oneshot_cbor.clone())
        // Language view for PlutusV3
        .add_language(ScriptKind::PlutusV3, params.v3_cost_model)
        // Required signer (admin must sign)
        .disclosed_signer(admin_hash);

    // Extra fee inputs (if needed beyond seed)
    for fi in &params.extra_fee_inputs {
        let h: [u8; 32] = hex::decode(&fi.tx_hash)
            .context("fee input hash")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("fee input hash must be 32 bytes"))?;
        staging = staging.input(Input::new(Hash::from(h), fi.output_index));
    }

    // -----------------------------------------------------------------------
    // 9. Build and return
    // -----------------------------------------------------------------------
    let built = staging
        .build_conway_raw()
        .map_err(|e| anyhow::anyhow!("build_conway_raw: {e}"))?;

    // Inject CIP-100 INewInstance metadata for portal auto-discovery.
    let instance_id_hex = hex::encode(*oneshot_hash);
    let admin_key_hex_str = hex::encode(&params.admin_key_hash);
    let (aux_cbor, aux_hash) = super::metadata::encode_publish(
        &instance_id_hex,
        &params.seed_utxo_hash,
        params.seed_utxo_index,
        params.expiration_ms,
        params.payout_upperbound_ms,
        params.vendor_expiration_ms,
        &admin_key_hex_str,
        &admin_sig,
        &params.label,
        params.description.as_deref(),
    )?;
    let final_bytes = super::cbor_util::add_aux_data(&built.tx_bytes.0, &aux_cbor, &aux_hash)?;
    let cbor_hex = hex::encode(&final_bytes);

    // Print the script hashes and policy ID for reference
    eprintln!("Registry token (policy ID): {instance_id_hex}");
    eprintln!(
        "Treasury script hash:       {}",
        hex::encode(*treasury_hash)
    );
    eprintln!("Vendor script hash:         {}", hex::encode(*vendor_hash));

    Ok(UnsignedTx::new("publish treasury instance", cbor_hex))
}
