//! Local Conway builder preflight. No wallet keys or submission required.
use crate::{
    koios::{ProtocolParams, Utxo},
    node::DataSource,
    references::{self, OutRef},
};
use anyhow::{ensure, Context, Result};
use pallas_addresses::Address;
use pallas_codec::minicbor::{self, data::Tag, Encoder};
use pallas_primitives::conway::{
    Redeemer, Redeemers, RedeemersKey, TransactionInput, TransactionOutput, Tx,
};
use pallas_traverse::MultiEraOutput;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct Report {
    pub redeemers: usize,
    pub memory: u64,
    pub steps: u64,
}

fn outref(input: &TransactionInput) -> OutRef {
    (input.transaction_id.to_string(), input.index)
}

pub async fn transaction(
    source: &DataSource,
    cbor_hex: &str,
    params: &ProtocolParams,
    genesis_offset_secs: u64,
    cached_references: &[Utxo],
) -> Result<Report> {
    let bytes = hex::decode(cbor_hex).context("invalid transaction hex")?;
    let tx = crate::metadata::transaction(&bytes)?;
    let refs = required_inputs(&tx);
    // Only reference inputs may be cached; spend/collateral inputs are rechecked.
    let reference_keys: BTreeSet<_> = tx
        .transaction_body
        .reference_inputs
        .iter()
        .flat_map(|r| r.iter())
        .map(outref)
        .collect();
    let cached: Vec<_> = cached_references
        .iter()
        .filter(|u| reference_keys.contains(&(u.tx_hash.clone(), u.tx_index)))
        .cloned()
        .collect();
    let cached_keys: BTreeSet<_> = cached
        .iter()
        .map(|u| (u.tx_hash.clone(), u.tx_index))
        .collect();
    let missing: Vec<_> = refs
        .iter()
        .filter(|r| !cached_keys.contains(*r))
        .cloned()
        .collect();
    let mut resolved = cached;
    if !missing.is_empty() {
        resolved.extend(
            source
                .utxo_info(&missing)
                .await
                .context("resolve preflight inputs")?,
        );
    }
    check_protocol(cbor_hex, &resolved, params)?;
    let params = params.evaluation_limits().and_then(|limits| {
        Ok((
            limits,
            params
                .v3_cost_model()
                .context("missing or malformed PlutusV3 cost model")?,
        ))
    })?;
    tokio::task::spawn_blocking(move || {
        evaluate(&bytes, &resolved, params.0, &params.1, genesis_offset_secs)
    })
    .await
    .context("preflight evaluator task failed")?
}

// Preserve the existing public entry point while all builders share validation.
pub use transaction as withdrawal;

/// Protocol checks shared by all current unsigned builders, independent of
/// script evaluation. Resolve spending/collateral inputs immediately before use.
pub fn check_protocol(cbor_hex: &str, utxos: &[Utxo], params: &ProtocolParams) -> Result<()> {
    let bytes = hex::decode(cbor_hex)?;
    let tx = crate::metadata::transaction(&bytes)?;
    let found = references::complete_utxos(&required_inputs(&tx), utxos.to_vec())?;
    let body = &tx.transaction_body;
    let network = body
        .network_id
        .as_ref()
        .context("transaction must declare its network")?;
    let network: u8 = minicbor::decode(&minicbor::to_vec(network)?)?;
    ensure!(network <= 1, "unsupported transaction network");
    for output in &body.outputs {
        check_output_network(output, network)?;
    }
    for input in found.values() {
        let raw = output_cbor(input)?;
        let output: TransactionOutput<'_> = minicbor::decode(&raw)?;
        check_output_network(&output, network)?;
    }
    let mut witnesses = body
        .required_signers
        .iter()
        .flat_map(|signers| signers.iter())
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let spending = body
        .inputs
        .iter()
        .chain(body.collateral.iter().flat_map(|inputs| inputs.iter()))
        .map(outref)
        .collect::<BTreeSet<_>>();
    for (key, utxo) in &found {
        if !spending.contains(key) {
            continue;
        }
        let raw = output_cbor(utxo)?;
        let output: TransactionOutput<'_> = minicbor::decode(&raw)?;
        let address = MultiEraOutput::from_conway(&output).address()?.to_vec();
        if address
            .first()
            .is_some_and(|header| matches!(header >> 4, 0 | 2 | 4 | 6))
            && address.len() >= 29
        {
            witnesses.insert(hex::encode(&address[1..29]));
        }
    }
    for key in crate::metadata::witnessed_authors(&tx)? {
        witnesses.insert(key);
    }
    let script_bytes =
        references::total_script_bytes(&found.values().cloned().collect::<Vec<_>>())?;
    let minimum =
        crate::tx::fees::minimum_fee(cbor_hex, params, script_bytes, witnesses.len() as u64)?;
    ensure!(
        body.fee >= minimum,
        "transaction fee is below the protocol minimum ({minimum})"
    );
    for output in &body.outputs {
        ensure!(
            encoded_value_size(output)? <= params.max_value_size()?,
            "output value exceeds protocol maximum size"
        );
        ensure!(
            MultiEraOutput::from_conway(output).value().coin()
                >= crate::tx::fees::minimum_output(output, params)?,
            "output below protocol minimum ADA"
        );
    }
    if let Some(collateral) = &body.collateral {
        ensure!(
            collateral.len() == 1,
            "current preflight supports one ADA-only collateral input"
        );
        let input = &collateral[0];
        let resolved = &found[&outref(input)];
        let raw = output_cbor(resolved)?;
        let output: TransactionOutput<'_> = minicbor::decode(&raw)?;
        let actual = utxo_from_output(&input.transaction_id.to_string(), input.index, &output)?;
        crate::tx::fees::validate_collateral(
            cbor_hex,
            params,
            &crate::coin_select::WalletUtxo::try_from(&actual)?,
        )?;
    } else {
        ensure!(
            tx.transaction_witness_set.redeemer.is_none(),
            "Plutus transaction has no collateral"
        );
    }
    Ok(())
}

fn encoded_value_size(output: &TransactionOutput<'_>) -> Result<u64> {
    let bytes = minicbor::to_vec(output)?;
    let mut decoder = minicbor::Decoder::new(&bytes);
    match decoder.datatype()? {
        minicbor::data::Type::Map => {
            let count = decoder.map()?.context("indefinite encoded output map")?;
            for _ in 0..count {
                if decoder.u64()? == 1 {
                    let start = decoder.position();
                    decoder.skip()?;
                    return Ok((decoder.position() - start) as u64);
                }
                decoder.skip()?;
            }
            anyhow::bail!("encoded output has no value")
        }
        minicbor::data::Type::Array => {
            ensure!(
                decoder.array()?.is_some_and(|count| count >= 2),
                "invalid encoded legacy output"
            );
            decoder.skip()?;
            let start = decoder.position();
            decoder.skip()?;
            Ok((decoder.position() - start) as u64)
        }
        _ => anyhow::bail!("unsupported encoded output"),
    }
}

fn check_output_network(output: &TransactionOutput<'_>, network: u8) -> Result<()> {
    let address = MultiEraOutput::from_conway(output).address()?.to_vec();
    ensure!(
        address
            .first()
            .is_some_and(|header| header >> 4 <= 7 && header & 15 == network),
        "expected a Shelley output on the transaction network"
    );
    Ok(())
}

fn required_inputs(tx: &Tx<'_>) -> Vec<OutRef> {
    tx.transaction_body
        .inputs
        .iter()
        .chain(
            tx.transaction_body
                .reference_inputs
                .iter()
                .flat_map(|r| r.iter()),
        )
        .chain(tx.transaction_body.collateral.iter().flat_map(|r| r.iter()))
        .map(outref)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Reconstruct a ledger output from Koios fields, preserving inline datum CBOR.
/// Node queries retain the ledger output directly, including legacy datum hashes.
pub fn output_cbor(u: &Utxo) -> Result<Vec<u8>> {
    if let Some(raw) = &u.output_cbor {
        return Ok(raw.clone());
    }
    let mut e = Encoder::new(Vec::new());
    let datum = u
        .datum_hex()
        .map(hex::decode)
        .transpose()
        .context("invalid inline datum hex")?;
    let has_datum = datum.is_some() || u.datum_hash.is_some();
    e.map(2 + u64::from(has_datum) + u64::from(u.reference_script.is_some()))?
        .u8(0)?
        .bytes(
            &Address::from_bech32(&u.address)
                .context("invalid UTxO address")?
                .to_vec(),
        )?
        .u8(1)?;
    let mut assets: BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, u64>> = BTreeMap::new();
    for a in u.asset_list.iter().flatten() {
        let policy = hex::decode(&a.policy_id)?;
        let name = hex::decode(&a.asset_name)?;
        ensure!(
            policy.len() == 28 && name.len() <= 32,
            "invalid asset identifier"
        );
        let qty: u64 = a.quantity.parse().context("invalid asset quantity")?;
        ensure!(qty > 0, "nonpositive asset quantity");
        ensure!(
            assets
                .entry(policy)
                .or_default()
                .insert(name, qty)
                .is_none(),
            "duplicate asset"
        );
    }
    let lovelace: u64 = u.value.parse().context("invalid lovelace amount")?;
    if assets.is_empty() {
        e.u64(lovelace)?;
    } else {
        e.array(2)?.u64(lovelace)?.map(assets.len() as u64)?;
        for (policy, names) in assets {
            e.bytes(&policy)?.map(names.len() as u64)?;
            for (name, qty) in names {
                e.bytes(&name)?.u64(qty)?;
            }
        }
    }
    if let Some(datum) = datum {
        let _: pallas_primitives::PlutusData =
            minicbor::decode(&datum).context("invalid inline datum CBOR")?;
        e.u8(2)?.array(2)?.u8(1)?.tag(Tag::new(24))?.bytes(&datum)?;
    } else if let Some(hash) = &u.datum_hash {
        let bytes = hex::decode(hash)?;
        ensure!(bytes.len() == 32, "invalid datum hash");
        e.u8(2)?.array(2)?.u8(0)?.bytes(&bytes)?;
    }
    if u.reference_script.is_some() {
        let (tag, bytes) = references::script_bytes(u)?;
        let mut script = Encoder::new(Vec::new());
        script.array(2)?.u8(tag)?;
        if tag == 0 {
            script.writer_mut().extend(bytes);
        } else {
            script.bytes(&bytes)?;
        }
        e.u8(3)?.tag(Tag::new(24))?.bytes(&script.into_writer())?;
    }
    let raw = e.into_writer();
    let _: TransactionOutput<'_> = minicbor::decode(&raw).context("invalid resolved output")?;
    Ok(raw)
}

/// Retain the complete ledger output when journaling confirmed chain data.
pub fn utxo_from_output(tx_hash: &str, index: u64, output: &TransactionOutput<'_>) -> Result<Utxo> {
    use crate::koios::AssetListItem;
    use pallas_primitives::conway::{DatumOption, ScriptRef};
    let output = MultiEraOutput::from_conway(output);
    let value = output.value();
    let mut assets = Vec::new();
    for policy in value.assets() {
        for asset in policy.assets() {
            let quantity =
                u64::try_from(asset.any_coin()).context("invalid output asset quantity")?;
            ensure!(quantity > 0, "nonpositive output asset quantity");
            let policy_id = policy.policy().to_string();
            let asset_name = hex::encode(asset.name());
            crate::tx::value::validate_asset_id(&format!("{policy_id}{asset_name}"))?;
            assets.push(AssetListItem {
                policy_id,
                asset_name,
                quantity: quantity.to_string(),
            });
        }
    }
    let (inline_datum, datum_hash) = match output.datum() {
        Some(DatumOption::Data(data)) => (
            Some(serde_json::json!({"bytes": hex::encode(minicbor::to_vec(&data.0)?)})),
            None,
        ),
        Some(DatumOption::Hash(hash)) => (None, Some(hash.to_string())),
        None => (None, None),
    };
    let reference_script = output
        .script_ref()
        .map(|script| -> Result<_> {
            let (kind, bytes) = match script {
                ScriptRef::NativeScript(script) => ("native", minicbor::to_vec(script)?),
                ScriptRef::PlutusV1Script(script) => ("plutusV1", script.0.to_vec()),
                ScriptRef::PlutusV2Script(script) => ("plutusV2", script.0.to_vec()),
                ScriptRef::PlutusV3Script(script) => ("plutusV3", script.0.to_vec()),
            };
            Ok(serde_json::json!({"type": kind, "bytes": hex::encode(bytes)}))
        })
        .transpose()?;
    Ok(Utxo {
        tx_hash: tx_hash.into(),
        tx_index: index,
        address: output.address()?.to_bech32()?,
        value: value.coin().to_string(),
        asset_list: Some(assets),
        inline_datum,
        reference_script,
        datum_hash,
        output_cbor: Some(output.encode()),
    })
}

type Balance = BTreeMap<(Vec<u8>, Vec<u8>), i128>;
fn add_value(balance: &mut Balance, output: &TransactionOutput<'_>, sign: i128) {
    let output = MultiEraOutput::from_conway(output);
    let value = output.value();
    *balance.entry((vec![], vec![])).or_default() += sign * i128::from(value.coin());
    for policy in value.assets() {
        for asset in policy.assets() {
            *balance
                .entry((policy.policy().as_ref().to_vec(), asset.name().to_vec()))
                .or_default() += sign * asset.any_coin();
        }
    }
}

/// Pure offline entry point used by regression fixtures. Supports withdrawal-only
/// transactions and V3 scripts; it deliberately does not claim full ledger validation.
pub fn evaluate(
    bytes: &[u8],
    utxos: &[Utxo],
    (major, max_mem, max_steps): (u16, u64, u64),
    v3_cost_model: &[i64],
    genesis_offset_secs: u64,
) -> Result<Report> {
    crate::cbor::validate(bytes)?;
    ensure!(
        (9..=11).contains(&major),
        "unsupported preflight protocol version {major}"
    );
    ensure!(
        max_mem > 0 && max_steps > 0 && max_mem <= i64::MAX as u64 && max_steps <= i64::MAX as u64,
        "invalid execution limits"
    );
    ensure!(!v3_cost_model.is_empty(), "empty PlutusV3 cost model");
    let mut decoder = minicbor::Decoder::new(bytes);
    let tx: Tx<'_> = decoder.decode().context("invalid Conway transaction")?;
    ensure!(
        decoder.position() == bytes.len() && tx.success,
        "invalid transaction envelope"
    );
    let body = &tx.transaction_body;
    ensure!(
        body.certificates.is_none()
            && body.proposal_procedures.is_none()
            && body.voting_procedures.is_none()
            && body.donation.is_none(),
        "preflight does not support certificates or governance actions"
    );
    ensure!(
        body.withdrawals
            .as_ref()
            .is_none_or(|w| w.values().all(|v| *v == 0)),
        "preflight only supports zero-value script withdrawals"
    );
    let lower = body
        .validity_interval_start
        .context("withdrawal needs a lower validity bound")?;
    ensure!(
        body.ttl.is_some_and(|ttl| ttl > lower),
        "empty or missing validity interval"
    );
    let found = references::complete_utxos(&required_inputs(&tx), utxos.to_vec())?;
    let mut balance = Balance::new();
    let mut resolved = Vec::new();
    let mut script_spend = false;
    let spends: BTreeSet<_> = body.inputs.iter().map(outref).collect();
    ensure!(
        spends.len() == body.inputs.len(),
        "duplicate spending input"
    );
    ensure!(
        body.reference_inputs
            .as_ref()
            .is_none_or(|r| r.iter().all(|i| !spends.contains(&outref(i)))),
        "input is both spent and referenced"
    );
    for (key, u) in &found {
        let output = output_cbor(u)?;
        let parsed: TransactionOutput<'_> = minicbor::decode(&output)?;
        if spends.contains(key) {
            add_value(&mut balance, &parsed, 1);
            script_spend |= MultiEraOutput::from_conway(&parsed)
                .address()?
                .to_vec()
                .first()
                .is_some_and(|header| matches!(header >> 4, 1 | 3 | 5 | 7));
        }
        // Every referenced Plutus script must be V3 for this evaluator interface.
        if u.reference_script.is_some() {
            let (tag, _) = references::script_bytes(u)?;
            ensure!(
                tag == 3,
                "withdrawal preflight only supports PlutusV3 reference scripts"
            );
        }
        let hash: [u8; 32] = hex::decode(&key.0)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid input hash"))?;
        resolved.push((
            minicbor::to_vec(TransactionInput {
                transaction_id: hash.into(),
                index: key.1,
            })?,
            output,
        ));
    }
    let multi = pallas_traverse::MultiEraTx::from_conway(&tx);
    let mut mint_policies = BTreeSet::new();
    for policy in multi.mints() {
        mint_policies.insert(policy.policy().to_string());
        for asset in policy.assets() {
            *balance
                .entry((policy.policy().as_ref().to_vec(), asset.name().to_vec()))
                .or_default() += asset.any_coin();
        }
    }
    for output in &body.outputs {
        add_value(&mut balance, output, -1);
    }
    *balance.entry((vec![], vec![])).or_default() -= i128::from(body.fee);
    ensure!(
        balance.values().all(|v| *v == 0),
        "transaction value is not conserved (inputs plus mint minus outputs and fee): {balance:?}"
    );
    let ws = &tx.transaction_witness_set;
    ensure!(
        ws.plutus_v1_script.is_none() && ws.plutus_v2_script.is_none(),
        "preflight supports PlutusV3 and single-signature native minting only"
    );
    let mut native_policies = BTreeSet::new();
    for script in ws.native_script.iter().flat_map(|scripts| scripts.iter()) {
        let raw = minicbor::to_vec(script)?;
        let mut decoder = minicbor::Decoder::new(&raw);
        ensure!(
            decoder.array()? == Some(2) && decoder.u8()? == 0,
            "unsupported native script shape; only signature policies are supported"
        );
        let key = decoder.bytes()?;
        ensure!(
            key.len() == 28 && decoder.position() == raw.len(),
            "malformed native signature policy"
        );
        ensure!(
            body.required_signers
                .as_ref()
                .is_some_and(|signers| signers.iter().any(|signer| signer.as_ref() == key)),
            "native policy key is not a required signer"
        );
        native_policies
            .insert(pallas_crypto::hash::Hasher::<224>::hash_tagged(&raw, 0).to_string());
    }
    if ws.redeemer.is_none() {
        ensure!(
            !script_spend
                && body
                    .withdrawals
                    .as_ref()
                    .is_none_or(|withdrawals| withdrawals.is_empty()),
            "script execution has no redeemers"
        );
        ensure!(
            mint_policies.is_subset(&native_policies),
            "mint policy has neither a supported native witness nor Plutus redeemer"
        );
        return Ok(Report {
            redeemers: 0,
            memory: 0,
            steps: 0,
        });
    }
    let mut budgets = BTreeMap::new();
    match ws
        .redeemer
        .as_deref()
        .context("withdrawal has no redeemers")?
    {
        Redeemers::List(rs) => {
            for r in rs {
                ensure!(
                    budgets
                        .insert(
                            RedeemersKey {
                                tag: r.tag,
                                index: r.index
                            },
                            r.ex_units
                        )
                        .is_none(),
                    "duplicate redeemer"
                );
            }
        }
        Redeemers::Map(rs) => {
            for (k, r) in rs {
                budgets.insert(k.clone(), r.ex_units);
            }
        }
    }
    ensure!(!budgets.is_empty(), "withdrawal has no redeemers");
    let declared_mem: u128 = budgets.values().map(|b| u128::from(b.mem)).sum();
    let declared_steps: u128 = budgets.values().map(|b| u128::from(b.steps)).sum();
    ensure!(
        declared_mem <= u128::from(max_mem) && declared_steps <= u128::from(max_steps),
        "declared execution budget exceeds protocol limits"
    );
    let cost_models = minicbor::to_vec(BTreeMap::from([(2u8, v3_cost_model)]))?;
    let zero_time = genesis_offset_secs
        .checked_mul(1000)
        .context("slot timestamp overflow")?;
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        uplc::tx::eval_phase_two_raw_with_protocol(
            bytes,
            &resolved,
            Some(&cost_models),
            (max_steps, max_mem),
            (zero_time, 0, 1000),
            major,
            true,
            |_| {},
        )
    }))
    .map_err(|_| anyhow::anyhow!("UPLC evaluator panicked; preflight failed closed"))?
    .map_err(|e| anyhow::anyhow!("withdrawal script evaluation failed: {e}"))?;
    ensure!(
        evaluated.len() == budgets.len(),
        "not all redeemers were evaluated"
    );
    let mut report = Report {
        redeemers: 0,
        memory: 0,
        steps: 0,
    };
    for (raw, result) in evaluated {
        let actual: Redeemer = minicbor::decode(&raw)?;
        ensure!(
            !result.failed(false, &uplc::Language::PlutusV3),
            "PlutusV3 script returned failure for {:?}#{}: {:?}",
            actual.tag,
            actual.index,
            result.logs()
        );
        let key = RedeemersKey {
            tag: actual.tag,
            index: actual.index,
        };
        let declared = budgets
            .remove(&key)
            .context("unexpected or repeated evaluated redeemer")?;
        ensure!(
            actual.ex_units.mem <= declared.mem && actual.ex_units.steps <= declared.steps,
            "insufficient execution budget for {:?}#{}: need {}/{} mem/steps, declared {}/{}",
            actual.tag,
            actual.index,
            actual.ex_units.mem,
            actual.ex_units.steps,
            declared.mem,
            declared.steps
        );
        report.redeemers += 1;
        report.memory += actual.ex_units.mem;
        report.steps += actual.ex_units.steps;
    }
    Ok(report)
}
