//! Local withdrawal preflight. No keys, witnesses or network submission required.
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

pub async fn withdrawal(
    source: &DataSource,
    cbor_hex: &str,
    params: &ProtocolParams,
    genesis_offset_secs: u64,
    cached_references: &[Utxo],
) -> Result<Report> {
    let bytes = hex::decode(cbor_hex).context("invalid transaction hex")?;
    let tx: Tx<'_> = minicbor::decode(&bytes).context("invalid Conway transaction")?;
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
        body.mint.is_none()
            && body.certificates.is_none()
            && body.proposal_procedures.is_none()
            && body.voting_procedures.is_none()
            && body.donation.is_none(),
        "preflight supports withdrawal transactions only"
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
    for output in &body.outputs {
        add_value(&mut balance, output, -1);
    }
    *balance.entry((vec![], vec![])).or_default() -= i128::from(body.fee);
    ensure!(
        balance.values().all(|v| *v == 0),
        "withdrawal value is not conserved (inputs minus outputs and fee): {balance:?}"
    );
    let ws = &tx.transaction_witness_set;
    ensure!(
        ws.plutus_v1_script.is_none()
            && ws.plutus_v2_script.is_none()
            && ws.native_script.is_none(),
        "preflight supports V3 witnesses only"
    );
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
