use std::collections::BTreeMap;

use anyhow::{Context, Result};
use pallas_addresses::Address as PallasAddress;
use pallas_crypto::hash::Hash;
use pallas_txbuilder::{BuildConway, Input, Output, StagingTransaction};

use super::cbor_util;
use super::metadata::{self, FundMilestone};
use super::UnsignedTx;
use crate::scripts::base_script_addr;
use crate::types::{AssetBundle, MultisigScript, Payout, PayoutStatus, VendorDatum};

pub struct MilestoneSpec {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub maturation_ms: u64,
    pub lovelace: u64,
    pub token_amount: u64,
}

pub struct FundInput {
    pub tx_hash: String,
    pub output_index: u64,
    pub lovelace: u64,
    pub token_amount: u64,
}

pub struct FundParams {
    pub instance_id: String,
    pub vendor_script_hash: Vec<u8>, // 28 bytes
    pub admin_key_hash: Vec<u8>,     // 28 bytes (txAuthor + disclosed signer)
    pub project_id: String,
    pub label: String,
    pub description: String,
    pub vendor_label: String,
    pub milestones: Vec<MilestoneSpec>,
    pub token_policy: Option<Vec<u8>>, // 28 bytes
    pub token_name: Option<Vec<u8>>,   // asset name bytes
    pub inputs: Vec<FundInput>,
    // Total ADA to place in vendor output — must be ≥ sum of milestone lovelaces.
    // Caller sets this to max(sum_lovelaces, MIN_TOKEN_OUTPUT_ADA) to ensure UTxO validity.
    pub vendor_lovelace_total: u64,
    pub change_lovelace: u64,
    pub change_tokens: u64,
    /// Extra (non-target) native assets from inputs that must be returned to the fee address.
    pub extra_change_assets: Vec<(Vec<u8>, Vec<u8>, u64)>,
    pub fee_addr: String,
    pub fee: u64,
    pub valid_from_slot: u64,
    pub invalid_from_slot: u64,
    pub network_id: u8,
}

pub fn build(params: FundParams) -> Result<UnsignedTx> {
    // -----------------------------------------------------------------------
    // 1. Vendor address from script hash
    // -----------------------------------------------------------------------
    let vendor_hash_bytes: [u8; 28] = params
        .vendor_script_hash
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("vendor_script_hash must be 28 bytes"))?;
    let vendor_hash: Hash<28> = Hash::from(vendor_hash_bytes);
    let vendor_addr_obj = base_script_addr(&vendor_hash, params.network_id);
    let vendor_addr =
        PallasAddress::from_bytes(&vendor_addr_obj.to_vec()).context("vendor addr")?;

    let fee_addr_bytes = PallasAddress::from_bech32(&params.fee_addr)
        .context("fee addr bech32")?
        .to_vec();
    let fee_addr = PallasAddress::from_bytes(&fee_addr_bytes).context("fee addr bytes")?;

    // -----------------------------------------------------------------------
    // 2. Build VendorDatum
    // -----------------------------------------------------------------------
    let vendor_multisig = MultisigScript::Signature {
        key_hash: params.admin_key_hash.clone(),
    };

    let payouts: Vec<Payout> = params
        .milestones
        .iter()
        .map(|m| {
            let mut value: BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, i64>> = BTreeMap::new();
            if m.lovelace > 0 {
                let mut ada_inner = BTreeMap::new();
                ada_inner.insert(vec![], m.lovelace as i64);
                value.insert(vec![], ada_inner);
            }
            if m.token_amount > 0 {
                if let (Some(policy), Some(name)) = (&params.token_policy, &params.token_name) {
                    let mut token_inner = BTreeMap::new();
                    token_inner.insert(name.clone(), m.token_amount as i64);
                    value.insert(policy.clone(), token_inner);
                }
            }
            Payout {
                maturation: m.maturation_ms,
                value: AssetBundle(value),
                status: PayoutStatus::Active,
            }
        })
        .collect();

    let datum = VendorDatum {
        vendor: vendor_multisig,
        payouts,
    };
    let datum_bytes = datum.to_cbor_bytes();

    // -----------------------------------------------------------------------
    // 3. Outputs
    // -----------------------------------------------------------------------
    let total_tokens: u64 = params.milestones.iter().map(|m| m.token_amount).sum();
    let vendor_lovelace = params.vendor_lovelace_total;

    let vendor_output = if total_tokens > 0 {
        if let (Some(policy), Some(name)) = (&params.token_policy, &params.token_name) {
            let policy_arr: [u8; 28] = policy
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("token_policy must be 28 bytes"))?;
            let policy_hash: Hash<28> = Hash::from(policy_arr);
            Output::new(vendor_addr, vendor_lovelace)
                .add_asset(policy_hash, name.clone(), total_tokens)
                .context("add token to vendor output")?
                .set_inline_datum(datum_bytes)
        } else {
            Output::new(vendor_addr, vendor_lovelace).set_inline_datum(datum_bytes)
        }
    } else {
        Output::new(vendor_addr, vendor_lovelace).set_inline_datum(datum_bytes)
    };

    let mut change_output = Output::new(fee_addr, params.change_lovelace);
    if params.change_tokens > 0 {
        if let (Some(policy), Some(name)) = (&params.token_policy, &params.token_name) {
            let policy_arr: [u8; 28] = policy
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("token_policy must be 28 bytes"))?;
            let policy_hash: Hash<28> = Hash::from(policy_arr);
            change_output = change_output
                .add_asset(policy_hash, name.clone(), params.change_tokens)
                .context("add token change")?;
        }
    }
    for (policy, name, qty) in &params.extra_change_assets {
        let policy_arr: [u8; 28] = policy
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("extra change asset policy must be 28 bytes"))?;
        change_output = change_output
            .add_asset(Hash::from(policy_arr), name.clone(), *qty)
            .context("add extra change asset")?;
    }

    // -----------------------------------------------------------------------
    // 4. Admin disclosed signer
    // -----------------------------------------------------------------------
    let admin_hash_arr: [u8; 28] = params
        .admin_key_hash
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("admin_key_hash must be 28 bytes"))?;
    let admin_hash: Hash<28> = Hash::from(admin_hash_arr);

    // -----------------------------------------------------------------------
    // 5. Build staging transaction
    // -----------------------------------------------------------------------
    let mut staging = StagingTransaction::new()
        .output(vendor_output)
        .output(change_output)
        .fee(params.fee)
        .valid_from_slot(params.valid_from_slot)
        .invalid_from_slot(params.invalid_from_slot)
        .network_id(params.network_id)
        .disclosed_signer(admin_hash);

    for fi in &params.inputs {
        let h: [u8; 32] = hex::decode(&fi.tx_hash)
            .context("fund input hash")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("fund input hash must be 32 bytes"))?;
        staging = staging.input(Input::new(Hash::from(h), fi.output_index));
    }

    // -----------------------------------------------------------------------
    // 6. Build raw and inject metadata
    // -----------------------------------------------------------------------
    let built = staging
        .build_conway_raw()
        .map_err(|e| anyhow::anyhow!("build_conway_raw: {e}"))?;

    let admin_key_hex = hex::encode(&params.admin_key_hash);
    let fund_milestones: Vec<FundMilestone> = params
        .milestones
        .iter()
        .map(|m| FundMilestone {
            id: m.id.clone(),
            label: m.label.clone(),
            description: m.description.clone(),
        })
        .collect();

    let (aux_cbor, aux_hash) = metadata::encode_fund(
        &params.instance_id,
        &admin_key_hex,
        &params.project_id,
        &params.label,
        &params.description,
        &params.vendor_label,
        &fund_milestones,
    )?;
    let final_bytes = cbor_util::add_aux_data(&built.tx_bytes.0, &aux_cbor, &aux_hash)?;
    let cbor_hex = hex::encode(&final_bytes);

    Ok(UnsignedTx::new("fund treasury project", cbor_hex))
}
