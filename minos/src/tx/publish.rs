/// Build an unsigned treasury deployment, including the bounded two-stage bundle.
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentReference {
    pub transaction: String,
    pub index: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentManifest {
    pub version: u8,
    pub network_id: u8,
    pub instance: String,
    pub treasury_script_hash: String,
    pub vendor_script_hash: String,
    pub publish_body_hash: String,
    pub vendor_reference_body_hash: String,
    /// Confirm this transaction/output before submitting vendor-reference.json.
    pub vendor_reference_depends_on: DeploymentReference,
    pub registry_reference: DeploymentReference,
    pub treasury_reference: DeploymentReference,
    pub vendor_reference: DeploymentReference,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentBundle {
    pub publish: UnsignedTx,
    pub vendor_reference: UnsignedTx,
    pub manifest: DeploymentManifest,
}

/// Both body hashes are stable under later witness collection. Stage two is
/// validated against the exact projected stage-one output, not a live UTxO claim.
pub fn build_bundle(
    params: PublishParams,
    protocol: &crate::koios::ProtocolParams,
) -> Result<DeploymentBundle> {
    let minimum = minimum_registry_total(&params)?;
    let input_coin =
        params
            .extra_fee_inputs
            .iter()
            .try_fold(params.seed_lovelace, |sum, input| {
                sum.checked_add(input.lovelace)
                    .context("deployment input ADA overflow")
            })?;
    let witnesses = super::fees::witness_count(
        std::iter::once(params.admin_key_hash.as_slice()),
        &params.fee_addr,
        params.network_id,
    )?;
    let publish = super::fees::balance(
        protocol,
        input_coin
            .checked_sub(minimum)
            .context("insufficient deployment funds")?,
        0,
        witnesses,
        |fee, _change| {
            let mut stage = params.clone();
            stage.fee = fee;
            build_registry_stage(stage)
        },
    )?;
    super::fees::validate_collateral(&publish.cbor_hex, protocol, &params.collateral)?;
    let bytes = hex::decode(&publish.cbor_hex)?;
    let tx = crate::metadata::transaction(&bytes)?;
    let publish_hash = hex::encode(super::sign::tx_body_hash(&bytes)?);
    let metadata = crate::metadata::auxiliary_1694(&tx)?.context("deployment metadata missing")?;
    let instance = metadata["instance"]
        .as_str()
        .context("deployment instance missing")?
        .to_owned();
    let expected = crate::metadata::instance_scripts(&metadata["body"], &instance)?;
    let registry: [u8; 28] = hex::decode(&instance)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid registry hash"))?;
    let admin = admin_permission(&params.admin_keys, params.admin_threshold)?;
    let script = parameterize_vendor(
        &registry,
        &VendorPermissions {
            pause: admin.clone(),
            resume: admin.clone(),
            modify: admin,
        },
        params.vendor_expiration_ms,
    )?;
    anyhow::ensure!(
        script_hash(&script).to_string() == expected.vendor_hash,
        "deployment vendor hash mismatch"
    );
    let vendor_address = base_script_addr(&script_hash(&script), params.network_id);
    let vendor_coin = super::fees::minimum_builder_coin(params.coins_per_utxo_byte, |coin| {
        Ok(Output::new(vendor_address.clone(), coin)
            .set_inline_script(ScriptKind::PlutusV3, script.clone()))
    })?;
    anyhow::ensure!(
        tx.transaction_body.outputs.len() == 3,
        "unexpected registry stage layout"
    );
    let funding =
        crate::preflight::utxo_from_output(&publish_hash, 2, &tx.transaction_body.outputs[2])?;
    let wallet_input = crate::coin_select::WalletUtxo::try_from(&funding)?;
    let fee_address = PallasAddress::from_bech32(&params.fee_addr)?;
    let fee_key = crate::coin_select::address_key_hash(&params.fee_addr, params.network_id)?;
    let input_hash: [u8; 32] = hex::decode(&publish_hash)?
        .try_into()
        .expect("32-byte body hash");
    let vendor_reference = super::fees::balance(
        protocol,
        wallet_input
            .lovelace
            .checked_sub(vendor_coin)
            .context("stage-one change cannot fund vendor reference")?,
        0,
        1,
        |fee, change| {
            let built = StagingTransaction::new()
                .input(Input::new(input_hash.into(), 2))
                .output(
                    Output::new(vendor_address.clone(), vendor_coin)
                        .set_inline_script(ScriptKind::PlutusV3, script.clone()),
                )
                .output(crate::coin_select::change_output(
                    fee_address.clone(),
                    change,
                    std::slice::from_ref(&wallet_input),
                )?)
                .fee(fee)
                .network_id(params.network_id)
                .valid_from_slot(params.valid_from_slot)
                .invalid_from_slot(params.invalid_from_slot)
                .disclosed_signer(fee_key.into())
                .build_conway_raw()
                .map_err(|error| anyhow::anyhow!("build vendor reference: {error}"))?;
            Ok(UnsignedTx::new(
                "vendor reference; requires confirmed publish change",
                hex::encode(built.tx_bytes.0),
            ))
        },
    )?;
    crate::preflight::check_protocol(
        &vendor_reference.cbor_hex,
        std::slice::from_ref(&funding),
        protocol,
    )?;
    crate::preflight::evaluate(
        &hex::decode(&vendor_reference.cbor_hex)?,
        &[funding],
        protocol.evaluation_limits()?,
        &protocol.v3_cost_model().context("missing V3 cost model")?,
        0,
    )?;
    let vendor_hash = hex::encode(super::sign::tx_body_hash(&hex::decode(
        &vendor_reference.cbor_hex,
    )?)?);
    let manifest = DeploymentManifest {
        version: 1,
        network_id: params.network_id,
        instance,
        treasury_script_hash: expected.treasury_hash,
        vendor_script_hash: expected.vendor_hash,
        publish_body_hash: publish_hash.clone(),
        vendor_reference_body_hash: vendor_hash.clone(),
        vendor_reference_depends_on: DeploymentReference {
            transaction: publish_hash.clone(),
            index: 2,
        },
        registry_reference: DeploymentReference {
            transaction: publish_hash.clone(),
            index: 0,
        },
        treasury_reference: DeploymentReference {
            transaction: publish_hash,
            index: 1,
        },
        vendor_reference: DeploymentReference {
            transaction: vendor_hash,
            index: 0,
        },
    };
    Ok(DeploymentBundle {
        publish,
        vendor_reference,
        manifest,
    })
}

// Asset name for the registry NFT: "REGISTRY" = 0x5245474953545259
const REGISTRY_ASSET_NAME: &[u8] = b"REGISTRY";

#[derive(Clone)]
pub struct PublishParams {
    // Seed UTxO — MUST be included in inputs; parameterizes the oneshot minting policy.
    pub seed_utxo_hash: String,
    pub seed_utxo_index: u64,
    pub seed_lovelace: u64,
    pub seed_assets: std::collections::BTreeMap<String, u64>,

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
    pub coins_per_utxo_byte: u64,

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

/// Plan the three deployment outputs using the same applied scripts and encoder
/// as the final builder. The sizing copy is never signed or submitted.
pub fn minimum_output_total(params: &PublishParams) -> Result<u64> {
    minimum_outputs(params, true)
}

pub fn minimum_registry_total(params: &PublishParams) -> Result<u64> {
    minimum_outputs(params, false)
}

fn minimum_outputs(params: &PublishParams, atomic: bool) -> Result<u64> {
    let mut sizing = params.clone();
    sizing.seed_lovelace = u64::MAX;
    sizing.seed_assets.clear();
    sizing.extra_fee_inputs.clear();
    sizing.fee = 0;
    let built = build_inner(sizing, atomic)?;
    let bytes = hex::decode(built.cbor_hex)?;
    let tx: pallas_primitives::conway::Tx<'_> = pallas_codec::minicbor::decode(&bytes)?;
    tx.transaction_body
        .outputs
        .iter()
        .take(if atomic { 3 } else { 2 })
        .try_fold(0u64, |total, output| {
            total
                .checked_add(
                    pallas_traverse::MultiEraOutput::from_conway(output)
                        .value()
                        .coin(),
                )
                .context("publication minimum ADA overflow")
        })
}

pub fn build(params: PublishParams) -> Result<UnsignedTx> {
    build_inner(params, true)
}

pub fn build_registry_stage(params: PublishParams) -> Result<UnsignedTx> {
    build_inner(params, false)
}

fn build_inner(params: PublishParams, atomic: bool) -> Result<UnsignedTx> {
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
    let registry_coin = super::fees::minimum_builder_coin(params.coins_per_utxo_byte, |coin| {
        Ok(Output::new(oneshot_addr.clone(), coin)
            .add_asset(oneshot_hash, REGISTRY_ASSET_NAME.to_vec(), 1)?
            .set_inline_datum(registry_datum.clone()))
    })?;
    let treasury_coin = super::fees::minimum_builder_coin(params.coins_per_utxo_byte, |coin| {
        Ok(Output::new(treasury_addr.clone(), coin)
            .set_inline_script(ScriptKind::PlutusV3, treasury_cbor.clone()))
    })?;
    let vendor_coin = super::fees::minimum_builder_coin(params.coins_per_utxo_byte, |coin| {
        Ok(Output::new(vendor_addr.clone(), coin)
            .set_inline_script(ScriptKind::PlutusV3, vendor_cbor.clone()))
    })?;
    let registry_output = Output::new(oneshot_addr, registry_coin)
        .add_asset(oneshot_hash, REGISTRY_ASSET_NAME.to_vec(), 1)
        .context("add REGISTRY asset to output")?
        .set_inline_datum(registry_datum);

    // Pass CBOR-wrapped flat bytes to pallas-txbuilder.
    // The Cardano node's deserialiseScript CBOR-decodes the bytes before flat-decoding,
    // so scripts must be passed in their CBOR-wrapped form (59xxxx flat_bytes), not raw flat.
    let treasury_ref_output = Output::new(treasury_addr, treasury_coin)
        .set_inline_script(ScriptKind::PlutusV3, treasury_cbor.clone());

    let vendor_ref_output = Output::new(vendor_addr, vendor_coin)
        .set_inline_script(ScriptKind::PlutusV3, vendor_cbor.clone());

    let mut wallet_inputs = params.extra_fee_inputs.clone();
    wallet_inputs.push(crate::coin_select::WalletUtxo {
        tx_hash: params.seed_utxo_hash.clone(),
        output_index: params.seed_utxo_index,
        lovelace: params.seed_lovelace,
        has_native_assets: !params.seed_assets.is_empty(),
        assets: params.seed_assets,
    });
    crate::coin_select::select_inputs(&wallet_inputs, 0)?;
    let input_coin = wallet_inputs.iter().try_fold(0u64, |sum, input| {
        sum.checked_add(input.lovelace)
            .context("publish input ADA overflow")
    })?;
    let change_coin = input_coin
        .checked_sub(registry_coin)
        .and_then(|coin| coin.checked_sub(treasury_coin))
        .and_then(|coin| coin.checked_sub(if atomic { vendor_coin } else { 0 }))
        .and_then(|coin| coin.checked_sub(params.fee))
        .context("insufficient ADA for publication outputs and fee")?;
    let change_output = crate::coin_select::change_output(fee_addr, change_coin, &wallet_inputs)?;

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

    if atomic {
        staging = staging.output(vendor_ref_output);
    }
    staging = staging.output(change_output);

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

    Ok(UnsignedTx::new("publish treasury instance", cbor_hex))
}
