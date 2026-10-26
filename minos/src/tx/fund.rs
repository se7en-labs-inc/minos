use std::collections::BTreeMap;

use anyhow::{Context, Result};
use pallas_addresses::Address as PallasAddress;
use pallas_codec::{
    minicbor,
    utils::{Int, MaybeIndefArray},
};
use pallas_crypto::hash::Hash;
use pallas_primitives::conway::{BigInt, BoundedBytes, Constr, PlutusData};
use pallas_txbuilder::{BuildConway, ExUnits, Input, Output, ScriptKind, StagingTransaction};

use super::cbor_util;
use super::metadata::{self, FundMilestone};
use super::UnsignedTx;
use crate::scripts::base_script_addr;
use crate::types::{AssetBundle, MultisigScript, Payout, PayoutStatus, VendorDatum};

pub struct MilestoneSpec {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub requires_evidence: bool,
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
    pub treasury: Option<TreasurySpend>,
    pub instance_id: String,
    pub vendor_script_hash: Vec<u8>, // 28 bytes
    pub admin_key_hash: Vec<u8>,     // 28 bytes (txAuthor + disclosed signer)
    pub vendor_key_hash: Option<Vec<u8>>,
    pub co_signer_hashes: Vec<Vec<u8>>,
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

pub struct TreasurySpend {
    pub input: crate::references::OutRef,
    pub references: Vec<crate::references::OutRef>,
    pub collateral: crate::coin_select::WalletUtxo,
    pub cost_model: Vec<i64>,
    pub return_address: String,
    pub return_lovelace: u64,
    pub return_assets: Vec<(Vec<u8>, Vec<u8>, u64)>,
}

pub fn select_ada_treasury_input(
    utxos: &[crate::koios::Utxo],
    needed: u64,
) -> Result<&crate::koios::Utxo> {
    utxos
        .iter()
        .filter(|u| {
            !u.has_native_assets() && u.reference_script.is_none() && u.lovelace() >= needed
        })
        .min_by_key(|u| u.lovelace())
        .context("no ADA-only treasury UTxO can fund all payouts; use deposit-project first")
}

pub fn select_funding_collateral<'a>(
    utxos: &'a [crate::coin_select::WalletUtxo],
    inputs: &[FundInput],
) -> Result<&'a crate::coin_select::WalletUtxo> {
    let excluded: Vec<_> = inputs
        .iter()
        .map(|i| (i.tx_hash.clone(), i.output_index))
        .collect();
    crate::coin_select::select_separate_collateral(utxos, &excluded)
}

/// Deposit is an ordinary wallet transfer; it does not create a project.
pub fn build_deposit(params: FundParams, treasury_hash: &[u8]) -> Result<UnsignedTx> {
    let hash = Hash::<28>::from(<[u8; 28]>::try_from(treasury_hash)?);
    let destination =
        PallasAddress::from_bytes(&base_script_addr(&hash, params.network_id).to_vec())?;
    let mut deposit = Output::new(destination, params.vendor_lovelace_total)
        .set_inline_datum(vec![0xd8, 0x79, 0x80]);
    if let (Some(policy), Some(name)) = (&params.token_policy, &params.token_name) {
        deposit = deposit.add_asset(
            Hash::from(<[u8; 28]>::try_from(policy.as_slice())?),
            name.clone(),
            params.milestones.iter().map(|m| m.token_amount).sum(),
        )?;
    }
    let mut change = Output::new(
        PallasAddress::from_bech32(&params.fee_addr)?,
        params.change_lovelace,
    );
    if params.change_tokens > 0 {
        change = change.add_asset(
            Hash::from(<[u8; 28]>::try_from(
                params
                    .token_policy
                    .as_ref()
                    .context("missing policy")?
                    .as_slice(),
            )?),
            params.token_name.clone().context("missing token name")?,
            params.change_tokens,
        )?;
    }
    for (policy, name, qty) in params.extra_change_assets {
        change = change.add_asset(
            Hash::from(<[u8; 28]>::try_from(policy.as_slice())?),
            name,
            qty,
        )?;
    }
    let mut staging = StagingTransaction::new()
        .output(deposit)
        .output(change)
        .fee(params.fee)
        .valid_from_slot(params.valid_from_slot)
        .invalid_from_slot(params.invalid_from_slot)
        .network_id(params.network_id)
        .disclosed_signer(Hash::from(<[u8; 28]>::try_from(
            params.admin_key_hash.as_slice(),
        )?));
    if let Some(treasury) = &params.treasury {
        anyhow::ensure!(
            treasury.return_lovelace == 0 && treasury.return_assets.is_empty(),
            "reuse requires an exact deposit value"
        );
        let input = Input::new(
            Hash::from(<[u8; 32]>::try_from(
                hex::decode(&treasury.input.0)?.as_slice(),
            )?),
            treasury.input.1,
        );
        staging = staging
            .collateral_input(Input::new(
                Hash::from(<[u8; 32]>::try_from(
                    hex::decode(&treasury.collateral.tx_hash)?.as_slice(),
                )?),
                treasury.collateral.output_index,
            ))
            .add_spend_redeemer(
                input,
                vec![0xd8, 0x79, 0x80],
                Some(ExUnits {
                    mem: 5_000_000,
                    steps: 2_000_000_000,
                }),
            )
            .add_language(ScriptKind::PlutusV3, treasury.cost_model.clone());
        for (hash, index) in &treasury.references {
            staging = staging.reference_input(Input::new(
                Hash::from(<[u8; 32]>::try_from(hex::decode(hash)?.as_slice())?),
                *index,
            ));
        }
    }
    for input in params.inputs {
        staging = staging.input(Input::new(
            Hash::from(<[u8; 32]>::try_from(
                hex::decode(input.tx_hash)?.as_slice(),
            )?),
            input.output_index,
        ));
    }
    let built = staging
        .build_conway_raw()
        .map_err(|e| anyhow::anyhow!("build deposit: {e}"))?;
    let (aux, hash) = metadata::encode_initialize(
        &params.instance_id,
        &hex::encode(&params.admin_key_hash),
        &format!("{}-funding", params.project_id),
    )?;
    let bytes = cbor_util::add_aux_data(&built.tx_bytes.0, &aux, &hash)?;
    Ok(UnsignedTx::new(
        "deposit project funds into budget treasury",
        hex::encode(bytes),
    ))
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
        key_hash: params
            .vendor_key_hash
            .clone()
            .unwrap_or_else(|| params.admin_key_hash.clone()),
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

    for key in params
        .co_signer_hashes
        .iter()
        .chain(params.vendor_key_hash.iter())
    {
        staging = staging.disclosed_signer(Hash::from(<[u8; 28]>::try_from(key.as_slice())?));
    }

    if let Some(treasury) = &params.treasury {
        let mut value: BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, i64>> = BTreeMap::new();
        let ada: u64 = params.milestones.iter().map(|m| m.lovelace).sum();
        if ada > 0 {
            value.insert(vec![], BTreeMap::from([(vec![], i64::try_from(ada)?)]));
        }
        if total_tokens > 0 {
            value.insert(
                params.token_policy.clone().context("missing policy")?,
                BTreeMap::from([(
                    params.token_name.clone().context("missing token name")?,
                    i64::try_from(total_tokens)?,
                )]),
            );
        }
        let data = PlutusData::Map(pallas_codec::utils::KeyValuePairs::Def(
            value
                .into_iter()
                .map(|(policy, assets)| {
                    (
                        PlutusData::BoundedBytes(BoundedBytes::from(policy)),
                        PlutusData::Map(pallas_codec::utils::KeyValuePairs::Def(
                            assets
                                .into_iter()
                                .map(|(name, qty)| {
                                    (
                                        PlutusData::BoundedBytes(BoundedBytes::from(name)),
                                        PlutusData::BigInt(BigInt::Int(Int::from(qty))),
                                    )
                                })
                                .collect(),
                        )),
                    )
                })
                .collect(),
        ));
        let redeemer = minicbor::to_vec(PlutusData::Constr(Constr {
            tag: 123,
            any_constructor: None,
            fields: MaybeIndefArray::Indef(vec![data]),
        }))?;
        let input = Input::new(
            Hash::from(<[u8; 32]>::try_from(
                hex::decode(&treasury.input.0)?.as_slice(),
            )?),
            treasury.input.1,
        );
        staging = staging
            .collateral_input(Input::new(
                Hash::from(<[u8; 32]>::try_from(
                    hex::decode(&treasury.collateral.tx_hash)?.as_slice(),
                )?),
                treasury.collateral.output_index,
            ))
            .add_spend_redeemer(
                input,
                redeemer,
                Some(ExUnits {
                    mem: 5_000_000,
                    steps: 2_000_000_000,
                }),
            )
            .add_language(ScriptKind::PlutusV3, treasury.cost_model.clone());
        for (hash, index) in &treasury.references {
            staging = staging.reference_input(Input::new(
                Hash::from(<[u8; 32]>::try_from(hex::decode(hash)?.as_slice())?),
                *index,
            ));
        }
        if treasury.return_lovelace > 0 || !treasury.return_assets.is_empty() {
            let mut output = Output::new(
                PallasAddress::from_bech32(&treasury.return_address)?,
                treasury.return_lovelace,
            )
            .set_inline_datum(vec![0xd8, 0x79, 0x80]);
            for (policy, name, qty) in &treasury.return_assets {
                output = output.add_asset(
                    Hash::from(<[u8; 28]>::try_from(policy.as_slice())?),
                    name.clone(),
                    *qty,
                )?;
            }
            staging = staging.output(output);
        }
    }

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
            acceptance_criteria: m.acceptance_criteria.clone(),
            requires_evidence: m.requires_evidence,
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
