//! Deterministic projection of confirmed transactions. Metadata describes an
//! operation only after its actual script inputs, redeemers and lineage agree.
use crate::{
    lineage, parser,
    types::{AssetAmount, Event, EventType, Instance, Project, TxOutRef, VendorUtxo},
};
use anyhow::{ensure, Context, Result};
use minos::{koios::Utxo, metadata, references, tx::value::Value, types::VendorDatum};
use pallas_addresses::Address;
use pallas_crypto::hash::Hasher;
use pallas_primitives::conway::{PlutusData, RedeemerTag, Redeemers, Tx};
use pallas_traverse::MultiEraTx;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Point {
    pub slot: u64,
    pub hash: String,
    pub height: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub point: Point,
    pub transactions: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Kind {
    Registry,
    Treasury,
    Vendor,
    Reference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Output {
    utxo: Utxo,
    kind: Kind,
    project: Option<String>,
    payout_ids: Vec<Option<String>>,
    spent_by: Option<String>,
    slot: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub tx_hash: String,
    pub slot: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub instance: Option<Instance>,
    pub projects: BTreeMap<String, Project>,
    pub current: BTreeMap<String, VendorUtxo>,
    outputs: BTreeMap<String, Output>,
    pub observations: BTreeMap<String, Observation>,
    seen: BTreeSet<String>,
}

pub struct Applied {
    pub state: Option<State>,
    pub events: Vec<Event>,
    pub relevant: Block,
}

fn outref(hash: &str, index: u64) -> String {
    format!("{hash}#{index}")
}

fn payment_hash(address: &str, network: u8) -> Result<Option<String>> {
    let address = Address::from_bech32(address).context("invalid ledger address")?;
    let bytes = address.to_vec();
    let Some(header) = bytes.first() else {
        anyhow::bail!("empty ledger address")
    };
    if matches!(header >> 4, 1 | 3 | 5 | 7) {
        ensure!(
            header & 15 == network && bytes.len() >= 29,
            "script output network mismatch"
        );
        Ok(Some(hex::encode(&bytes[1..29])))
    } else {
        Ok(None)
    }
}

fn network_id(network: &str) -> Result<u8> {
    match network {
        "mainnet" => Ok(1),
        "preview" | "preprod" => Ok(0),
        _ => anyhow::bail!("unsupported chain network"),
    }
}

fn lower_ms(tx: &Tx<'_>, network: &str) -> Result<u64> {
    let magic = match network {
        "mainnet" => 764824073,
        "preview" => 2,
        "preprod" => 1,
        _ => anyhow::bail!("unsupported chain network"),
    };
    let genesis = pallas_traverse::wellknown::GenesisValues::from_magic(magic)
        .context("unknown chain genesis")?;
    genesis
        .slot_to_wallclock(
            tx.transaction_body
                .validity_interval_start
                .context("missing lower validity bound")?,
        )
        .checked_mul(1000)
        .context("validity time overflow")
}

fn value(utxo: &Utxo) -> Result<Value> {
    let wallet = minos::coin_select::WalletUtxo::try_from(utxo)?;
    Ok(Value {
        coin: wallet.lovelace,
        assets: wallet.assets,
    })
}

fn valid_author(tx: &Tx<'_>, meta: &Json) -> Result<()> {
    if let Some(author) = meta.get("txAuthor") {
        let author = author
            .as_str()
            .context("metadata author must be a key hash")?;
        ensure!(
            author.len() == 56 && hex::decode(author).is_ok(),
            "invalid metadata author key hash"
        );
        ensure!(
            metadata::witnessed_authors(tx)?
                .iter()
                .any(|key| key.eq_ignore_ascii_case(author)),
            "metadata author has no valid witness for this body"
        );
    }
    Ok(())
}

fn spend_action(tx: &Tx<'_>, key: &str) -> Result<u64> {
    let mut inputs = tx.transaction_body.inputs.iter().collect::<Vec<_>>();
    inputs.sort_by(|a, b| {
        a.transaction_id
            .cmp(&b.transaction_id)
            .then(a.index.cmp(&b.index))
    });
    ensure!(
        inputs.windows(2).all(|pair| pair[0] != pair[1]),
        "duplicate spending input"
    );
    let index = inputs
        .iter()
        .position(|input| outref(&input.transaction_id.to_string(), input.index) == key)
        .context("linked output is not a spending input")? as u32;
    let redeemers = tx
        .transaction_witness_set
        .redeemer
        .as_deref()
        .context("script spend has no redeemers")?;
    let mut found = Vec::new();
    match redeemers {
        Redeemers::List(items) => {
            for item in items {
                if item.tag == RedeemerTag::Spend && item.index == index {
                    found.push(&item.data);
                }
            }
        }
        Redeemers::Map(items) => {
            for (key, item) in items.iter() {
                if key.tag == RedeemerTag::Spend && key.index == index {
                    found.push(&item.data);
                }
            }
        }
    }
    ensure!(found.len() == 1, "missing or duplicate spend redeemer");
    let PlutusData::Constr(constructor) = found[0] else {
        anyhow::bail!("unsupported spend redeemer")
    };
    match constructor.tag {
        121..=127 => Ok(constructor.tag - 121),
        102 => constructor
            .any_constructor
            .context("missing general constructor index"),
        _ => anyhow::bail!("unsupported spend redeemer constructor"),
    }
}

pub fn publication(bytes: &[u8], instance_id: &str, network: &str) -> Result<(Instance, Utxo)> {
    let tx = metadata::transaction(bytes)?;
    let meta =
        metadata::auxiliary_1694(&tx)?.context("publish transaction has no treasury metadata")?;
    ensure!(
        meta["instance"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(instance_id)),
        "publish instance mismatch"
    );
    ensure!(meta["body"]["event"] == "publish", "not a publish event");
    valid_author(&tx, &meta)?;
    let body = &meta["body"];
    let seed = body.get("seedUtxo").context("missing publish seed")?;
    let hash: [u8; 32] = hex::decode(
        seed["transaction_id"]
            .as_str()
            .context("missing seed transaction hash")?,
    )?
    .try_into()
    .map_err(|_| anyhow::anyhow!("seed hash must be 32 bytes"))?;
    let index = seed["output_index"]
        .as_u64()
        .or_else(|| seed["output_index"].as_str().and_then(|s| s.parse().ok()))
        .context("invalid seed output index")?;
    ensure!(
        tx.transaction_body
            .inputs
            .iter()
            .any(|input| input.transaction_id.as_ref() == hash && input.index == index),
        "publish does not consume its declared seed"
    );
    ensure!(
        minos::scripts::script_hash(&minos::scripts::parameterize_oneshot(&hash, index)?)
            .to_string()
            == instance_id,
        "instance does not match the pinned seed-parameterized oneshot policy"
    );
    let multi = MultiEraTx::from_conway(&tx);
    let mut minted = Vec::new();
    for policy in multi.mints() {
        for asset in policy.assets() {
            minted.push((
                policy.policy().to_string(),
                asset.name().to_vec(),
                asset.any_coin(),
            ));
        }
    }
    ensure!(
        minted == vec![(instance_id.into(), b"REGISTRY".to_vec(), 1)],
        "publish must mint exactly one REGISTRY NFT and no other assets"
    );
    let tx_hash = Hasher::<256>::hash(tx.transaction_body.raw_cbor()).to_string();
    let mut registries = Vec::new();
    for (index, output) in tx.transaction_body.outputs.iter().enumerate() {
        let candidate = pallas_traverse::MultiEraOutput::from_conway(output);
        if !candidate.value().assets().iter().any(|policy| {
            policy.policy().to_string() == instance_id
                && policy
                    .assets()
                    .iter()
                    .any(|asset| asset.name() == b"REGISTRY")
        }) {
            continue;
        }
        let utxo = minos::preflight::utxo_from_output(&tx_hash, index as u64, output)?;
        if utxo.asset_list.iter().flatten().any(|asset| {
            asset.policy_id == instance_id && asset.asset_name == hex::encode(b"REGISTRY")
        }) {
            registries.push(utxo);
        }
    }
    ensure!(
        registries.len() == 1,
        "publish has no unique registry output"
    );
    let registry = registries.remove(0);
    ensure!(
        payment_hash(&registry.address, network_id(network)?)?.as_deref() == Some(instance_id),
        "registry is not locked by the pinned always-fail oneshot credential"
    );
    let (treasury, vendor) = references::registry_hashes(&registry, instance_id)?;
    let expected = metadata::instance_scripts(body, instance_id)?;
    ensure!(
        treasury == expected.treasury_hash && vendor == expected.vendor_hash,
        "registry script hashes disagree with publish configuration"
    );
    let mut instance = Instance {
        id: instance_id.into(),
        label: body
            .get("label")
            .map(parser::rejoined_string)
            .unwrap_or_default(),
        description: body.get("description").map(parser::rejoined_string),
        expiration_ms: expected.expiration_ms,
        payout_upperbound_ms: expected.payout_upperbound_ms,
        vendor_expiration_ms: expected.vendor_expiration_ms,
        treasury_hash: treasury,
        vendor_hash: vendor,
        treasury_ref: None,
        vendor_ref: None,
        registry_ref: Some(TxOutRef {
            transaction: registry.tx_hash.clone(),
            index: registry.tx_index,
        }),
        publish_tx: tx_hash.clone(),
        network: network.into(),
        confirmed: false,
    };
    for (index, output) in tx.transaction_body.outputs.iter().enumerate() {
        if pallas_traverse::MultiEraOutput::from_conway(output)
            .script_ref()
            .is_none()
        {
            continue;
        }
        let utxo = minos::preflight::utxo_from_output(&tx_hash, index as u64, output)?;
        if utxo.reference_script.is_some() {
            let (tag, bytes) = references::script_bytes(&utxo)?;
            let hash = Hasher::<224>::hash_tagged(&bytes, tag).to_string();
            if hash == instance.treasury_hash {
                instance.treasury_ref = Some(TxOutRef {
                    transaction: tx_hash.clone(),
                    index: index as u64,
                });
            }
            if hash == instance.vendor_hash {
                instance.vendor_ref = Some(TxOutRef {
                    transaction: tx_hash.clone(),
                    index: index as u64,
                });
            }
        }
    }
    Ok((instance, registry))
}

fn event(
    hash: &str,
    point: &Point,
    instance: &str,
    kind: EventType,
    project: Option<String>,
    payload: Json,
) -> Event {
    Event {
        tx_hash: hash.into(),
        block_slot: point.slot,
        block_height: point.height,
        event_type: kind,
        instance_id: instance.into(),
        project_id: project,
        payload,
    }
}

fn vendor_output(output: &Output, project: &str, ids: Vec<Option<String>>) -> Result<VendorUtxo> {
    let amount = value(&output.utxo)?;
    let mut assets = vec![AssetAmount {
        asset_id: String::new(),
        quantity: amount.coin,
    }];
    assets.extend(
        amount
            .assets
            .into_iter()
            .map(|(asset_id, quantity)| AssetAmount { asset_id, quantity }),
    );
    Ok(VendorUtxo {
        instance_id: String::new(),
        tx_hash: output.utxo.tx_hash.clone(),
        output_index: output.utxo.tx_index,
        project_id: project.into(),
        address: output.utxo.address.clone(),
        datum_hex: output
            .utxo
            .datum_hex()
            .context("vendor output has no inline datum")?,
        lovelace: amount.coin,
        assets,
        slot: output.slot,
        payout_ids: ids,
    })
}

fn datum(output: &Output) -> Result<VendorDatum> {
    VendorDatum::from_cbor_hex(
        &output
            .utxo
            .datum_hex()
            .context("linked vendor output has no inline datum")?,
    )
}

fn observation(state: &mut State, hash: &str, point: &Point, error: impl std::fmt::Display) {
    let reason = match state.observations.get(hash) {
        Some(previous) => format!("{}; {error}", previous.reason),
        None => error.to_string(),
    };
    state.observations.insert(
        hash.into(),
        Observation {
            tx_hash: hash.into(),
            slot: point.slot,
            reason: reason.chars().take(512).collect(),
        },
    );
    if state.observations.len() > 4096 {
        if let Some(oldest) = state
            .observations
            .iter()
            .min_by_key(|(_, record)| record.slot)
            .map(|(key, _)| key.clone())
        {
            state.observations.remove(&oldest);
        }
    }
}

fn classify(instance: &Instance, utxo: &Utxo) -> Result<Option<Kind>> {
    let credential = payment_hash(&utxo.address, network_id(&instance.network)?)?;
    Ok(match credential.as_deref() {
        Some(hash)
            if hash == instance.id && references::registry_hashes(utxo, &instance.id).is_ok() =>
        {
            Some(Kind::Registry)
        }
        Some(hash) if hash == instance.treasury_hash => Some(Kind::Treasury),
        Some(hash) if hash == instance.vendor_hash => Some(Kind::Vendor),
        _ => {
            if utxo.reference_script.is_some() {
                let (tag, bytes) = references::script_bytes(utxo)?;
                let hash = Hasher::<224>::hash_tagged(&bytes, tag).to_string();
                if hash == instance.treasury_hash || hash == instance.vendor_hash {
                    Some(Kind::Reference)
                } else {
                    None
                }
            } else {
                None
            }
        }
    })
}

fn has_registry_reference(state: &State, tx: &Tx<'_>, instance: &str) -> Result<()> {
    let references = tx
        .transaction_body
        .reference_inputs
        .iter()
        .flat_map(|inputs| inputs.iter())
        .map(|input| outref(&input.transaction_id.to_string(), input.index))
        .collect::<BTreeSet<_>>();
    let registries = state
        .outputs
        .iter()
        .filter(|(key, output)| {
            output.kind == Kind::Registry && output.spent_by.is_none() && references.contains(*key)
        })
        .collect::<Vec<_>>();
    ensure!(
        registries.len() == 1,
        "script spend has no unique validated registry reference"
    );
    references::registry_hashes(&registries[0].1.utxo, instance)?;
    Ok(())
}

#[derive(Default)]
struct Effects {
    project: Option<Project>,
    binding: Option<(String, String, Vec<Option<String>>)>,
    consumed_project: Option<String>,
    events: Vec<Event>,
    action: Option<u64>,
    changes: BTreeMap<String, bool>,
}

#[derive(Clone, Copy)]
struct ChainContext<'a> {
    instance: &'a str,
    network: &'a str,
    point: &'a Point,
}

fn vendor_effects(
    state: &State,
    tx: &Tx<'_>,
    spent: &[(String, Output)],
    produced: &[(String, Output)],
    hash: &str,
    context: ChainContext<'_>,
) -> Result<Effects> {
    let ChainContext {
        instance,
        network,
        point,
    } = context;
    let linked = spent
        .iter()
        .filter(|(_, output)| output.kind == Kind::Vendor && output.project.is_some())
        .collect::<Vec<_>>();
    if linked.is_empty() {
        return Ok(Effects::default());
    }
    ensure!(
        linked.len() == 1,
        "multiple linked vendor inputs require an explicit association"
    );
    has_registry_reference(state, tx, instance)?;
    let (key, previous) = linked[0];
    let project_id = previous.project.as_ref().unwrap();
    let mut project = state
        .projects
        .get(project_id)
        .context("linked project is missing")?
        .clone();
    let before = datum(previous)?;
    let action = spend_action(tx, key)?;
    let candidates = produced
        .iter()
        .filter(|(_, output)| output.kind == Kind::Vendor && output.utxo.reference_script.is_none())
        .collect::<Vec<_>>();
    let mut binding = None;
    let mut events = Vec::new();
    let mut changes = BTreeMap::new();
    match action {
        0 => {
            let partition =
                lineage::withdraw(&before, &previous.payout_ids, None, lower_ms(tx, network)?)?;
            let residual = value(&previous.utxo)?.checked_sub(&partition.paid)?;
            if !residual.is_zero() || !partition.remaining.is_empty() {
                ensure!(
                    candidates.len() == 1,
                    "withdrawal has no unique continuing vendor output"
                );
                let (key, output) = candidates[0];
                let after = datum(output)?;
                let partition = lineage::withdraw(
                    &before,
                    &previous.payout_ids,
                    Some(&after),
                    lower_ms(tx, network)?,
                )?;
                let actual = value(&output.utxo)?;
                ensure!(
                    actual.assets == residual.assets && actual.coin >= residual.coin,
                    "withdrawal residual assets are not preserved"
                );
                lineage::synchronize(&mut project, &after, &partition.remaining)?;
                binding = Some((key.clone(), project_id.clone(), partition.remaining));
            }
            if !partition.withdrawn.is_empty()
                || partition.unresolved_removed > 0
                || !partition.paid.is_zero()
            {
                let milestones = partition
                    .withdrawn
                    .into_iter()
                    .map(|id| (id, Json::Bool(true)))
                    .collect::<serde_json::Map<_, _>>();
                events.push(event(hash, point, instance, EventType::Withdraw, Some(project_id.clone()),
                    json!({"event": "withdraw", "milestones": milestones, "unresolvedPayouts": partition.unresolved_removed, "derivedFromContract": true})));
            }
        }
        1 | 2 => {
            ensure!(candidates.len() <= 1, "ambiguous vendor successor outputs");
            if action == 1 {
                ensure!(
                    candidates.len() == 1,
                    "adjudication must preserve a vendor output"
                );
            }
            if let Some((key, output)) = candidates.first() {
                let after = datum(output)?;
                let ids = if action == 1 {
                    lineage::adjudicate(&before, &previous.payout_ids, &after)?
                } else {
                    lineage::modify(&before, &previous.payout_ids, &after)?
                };
                if action == 1 {
                    for ((before, after), id) in before.payouts.iter().zip(&after.payouts).zip(&ids)
                    {
                        let was_paused =
                            matches!(before.status, minos::types::PayoutStatus::Paused);
                        let paused = matches!(after.status, minos::types::PayoutStatus::Paused);
                        if was_paused != paused {
                            if let Some(id) = id {
                                changes.insert(id.clone(), paused);
                            }
                        }
                    }
                }
                lineage::synchronize(&mut project, &after, &ids)?;
                binding = Some((key.clone(), project_id.clone(), ids));
            }
            events.push(event(
                hash,
                point,
                instance,
                if action == 1 {
                    EventType::Adjudicate
                } else {
                    EventType::Modify
                },
                Some(project_id.clone()),
                json!({"derivedFromContract": true, "changes": changes}),
            ));
        }
        3 | 4 => events.push(event(
            hash,
            point,
            instance,
            EventType::Sweep,
            Some(project_id.clone()),
            json!({"derivedFromContract": true}),
        )),
        _ => anyhow::bail!("unsupported vendor redeemer constructor"),
    }
    Ok(Effects {
        project: Some(project),
        binding,
        consumed_project: Some(project_id.clone()),
        events,
        action: Some(action),
        changes,
    })
}

fn validate_claim(
    parsed: &parser::ParsedEvent,
    vendor_action: Option<u64>,
    treasury_actions: &[u64],
    project: Option<&str>,
    produced: &[(String, Output)],
) -> Result<()> {
    if let Some(claimed) = parsed.event.project_id.as_deref() {
        if parsed.event.event_type != EventType::Fund {
            ensure!(
                Some(claimed) == project,
                "metadata project disagrees with consumed output lineage"
            );
        }
    }
    let valid = match parsed.event.event_type {
        EventType::Fund => treasury_actions.contains(&2),
        EventType::Complete | EventType::Withdraw => vendor_action == Some(0),
        EventType::Pause | EventType::Resume | EventType::Adjudicate => vendor_action == Some(1),
        EventType::Modify => vendor_action == Some(2),
        EventType::Sweep => treasury_actions.contains(&1) || matches!(vendor_action, Some(3 | 4)),
        EventType::Reorganize => treasury_actions.contains(&0),
        EventType::Disburse => treasury_actions.contains(&3),
        EventType::Initialize => produced
            .iter()
            .any(|(_, output)| output.kind == Kind::Treasury),
        EventType::Publish | EventType::Cancel => false,
    };
    ensure!(
        valid,
        "metadata event has no matching contract execution or deposit"
    );
    Ok(())
}

fn apply_transaction(
    state: &mut State,
    bytes: &[u8],
    tx: &Tx<'_>,
    meta: Option<&Json>,
    meta_error: Option<&str>,
    context: ChainContext<'_>,
) -> Result<Vec<Event>> {
    let ChainContext {
        instance,
        network,
        point,
    } = context;
    let hash = Hasher::<256>::hash(tx.transaction_body.raw_cbor()).to_string();
    let mut events = Vec::new();
    if state.instance.is_none() {
        match publication(bytes, instance, network) {
            Ok((mut published, _)) => {
                published.confirmed = true;
                state.instance = Some(published);
                events.push(event(
                    &hash,
                    point,
                    instance,
                    EventType::Publish,
                    None,
                    meta.context("missing publish metadata")?["body"].clone(),
                ));
            }
            Err(error) => {
                observation(
                    state,
                    &hash,
                    point,
                    format!("unconfirmed publish claim: {error:#}"),
                );
                return Ok(events);
            }
        }
    }
    let published = state.instance.as_ref().unwrap().clone();
    let inputs = if tx.success {
        tx.transaction_body.inputs.iter().collect::<Vec<_>>()
    } else {
        tx.transaction_body
            .collateral
            .iter()
            .flat_map(|inputs| inputs.iter())
            .collect()
    };
    let mut spent = Vec::new();
    for input in inputs {
        let key = outref(&input.transaction_id.to_string(), input.index);
        if let Some(output) = state.outputs.get(&key) {
            ensure!(
                output.spent_by.is_none(),
                "confirmed data double-spends a tracked output"
            );
            ensure!(
                output.kind != Kind::Registry,
                "confirmed data spends the pinned always-fail registry"
            );
            spent.push((key, output.clone()));
        }
    }
    let multi = MultiEraTx::from_conway(tx);
    let mut produced = Vec::new();
    for (index, output) in multi.produces() {
        let address = output.address()?;
        let raw = address.to_vec();
        if output.script_ref().is_none()
            && !raw
                .first()
                .is_some_and(|header| matches!(header >> 4, 1 | 3 | 5 | 7))
        {
            continue;
        }
        let utxo = minos::preflight::utxo_from_output(
            &hash,
            index as u64,
            output.as_conway().context("expected Conway output")?,
        )?;
        if let Some(kind) = classify(&published, &utxo)? {
            produced.push((
                outref(&hash, index as u64),
                Output {
                    utxo,
                    kind,
                    project: None,
                    payout_ids: vec![],
                    spent_by: None,
                    slot: point.slot,
                },
            ));
        }
    }
    let mut effects = Effects::default();
    let mut treasury_actions = Vec::new();
    if tx.success {
        match vendor_effects(state, tx, &spent, &produced, &hash, context) {
            Ok(validated) => effects = validated,
            Err(error) => observation(
                state,
                &hash,
                point,
                format!("unresolved vendor transition: {error:#}"),
            ),
        }
        for (key, output) in &spent {
            if output.kind == Kind::Treasury {
                match has_registry_reference(state, tx, instance)
                    .and_then(|_| spend_action(tx, key))
                {
                    Ok(action @ 0..=3) => treasury_actions.push(action),
                    _ => observation(state, &hash, point, "unresolved treasury execution"),
                }
            }
        }
    }
    let mut parsed = None;
    if let Some(meta) = meta.filter(|meta| {
        meta["instance"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(instance))
    }) {
        if meta["body"]["event"] != "publish" || events.is_empty() {
            match valid_author(tx, meta).and_then(|_| {
                ensure!(
                    tx.success,
                    "script-invalid transaction metadata cannot confirm an event"
                );
                let parsed = parser::parse_metadata(&hash, point.slot, point.height, meta)
                    .context("unsupported or malformed event metadata")?;
                validate_claim(
                    &parsed,
                    effects.action,
                    &treasury_actions,
                    effects.consumed_project.as_deref(),
                    &produced,
                )?;
                if matches!(
                    parsed.event.event_type,
                    EventType::Pause | EventType::Resume
                ) {
                    let ids = parsed.event.payload["milestones"]
                        .as_object()
                        .context("pause/resume requires milestone identifiers")?;
                    ensure!(!ids.is_empty(), "pause/resume has no milestones");
                    let paused = parsed.event.event_type == EventType::Pause;
                    ensure!(
                        ids.keys()
                            .all(|id| effects.changes.get(id) == Some(&paused)),
                        "pause/resume claim disagrees with actual datum changes"
                    );
                }
                if parsed.event.event_type == EventType::Complete {
                    ensure!(
                        parsed.event.payload["milestones"]
                            .as_object()
                            .is_some_and(|ids| !ids.is_empty()),
                        "completion requires milestone evidence map"
                    );
                }
                if parsed.event.event_type == EventType::Initialize {
                    let outputs = parsed.event.payload["outputs"]
                        .as_object()
                        .context("initialize requires declared output indices")?;
                    ensure!(
                        !outputs.is_empty()
                            && outputs
                                .keys()
                                .all(|index| index.parse::<u64>().ok().is_some_and(|index| {
                                    produced.iter().any(|(_, output)| {
                                        output.kind == Kind::Treasury
                                            && output.utxo.tx_index == index
                                    })
                                })),
                        "initialize declares an unrelated output"
                    );
                }
                Ok(parsed)
            }) {
                Ok(validated) => parsed = Some(validated),
                Err(error) => observation(
                    state,
                    &hash,
                    point,
                    format!("rejected metadata claim: {error:#}"),
                ),
            }
        }
    }
    if let Some(error) = meta_error {
        observation(
            state,
            &hash,
            point,
            format!("rejected metadata encoding: {error}"),
        );
    }
    // Consumed outputs remain in lineage, while the current-output projection is removed.
    for (key, previous) in spent {
        state.outputs.get_mut(&key).unwrap().spent_by = Some(hash.clone());
        if let Some(project) = previous.project {
            state.current.remove(&project);
        }
        let instance = state.instance.as_mut().unwrap();
        for reference in [&mut instance.treasury_ref, &mut instance.vendor_ref] {
            if reference
                .as_ref()
                .is_some_and(|reference| outref(&reference.transaction, reference.index) == key)
            {
                *reference = None;
            }
        }
    }
    if let Some(project) = effects.project.take() {
        state.projects.insert(project.id.clone(), project);
    }
    if let Some((key, project, ids)) = effects.binding.take() {
        let output = produced
            .iter_mut()
            .find(|(output, _)| *output == key)
            .context("missing bound successor output")?;
        output.1.project = Some(project.clone());
        output.1.payout_ids = ids.clone();
        let mut current = vendor_output(&output.1, &project, ids)?;
        current.instance_id = instance.into();
        state.current.insert(project, current);
    }
    if let Some(mut parsed) = parsed {
        if parsed.event.event_type == EventType::Fund {
            let result = (|| -> Result<()> {
                let mut project = parsed
                    .project
                    .take()
                    .context("fund metadata has no complete project")?;
                ensure!(
                    !state.projects.contains_key(&project.id),
                    "duplicate project identifier within instance"
                );
                let candidates = produced
                    .iter_mut()
                    .filter(|(_, output)| {
                        output.kind == Kind::Vendor
                            && output.project.is_none()
                            && output.utxo.reference_script.is_none()
                    })
                    .collect::<Vec<_>>();
                ensure!(
                    candidates.len() == 1,
                    "funding has no unique unassigned vendor output"
                );
                let (_, output) = candidates.into_iter().next().unwrap();
                let datum = datum(output)?;
                let ids = lineage::initial(&project, &datum);
                lineage::synchronize(&mut project, &datum, &ids)?;
                let mut current = vendor_output(output, &project.id, ids.clone())?;
                current.instance_id = instance.into();
                output.project = Some(project.id.clone());
                output.payout_ids = ids;
                state.current.insert(project.id.clone(), current);
                parsed.event.project_id = Some(project.id.clone());
                state.projects.insert(project.id.clone(), project);
                Ok(())
            })();
            if let Err(error) = result {
                observation(
                    state,
                    &hash,
                    point,
                    format!("unresolved funding association: {error:#}"),
                );
            } else {
                events.push(parsed.event);
            }
        } else if matches!(
            parsed.event.event_type,
            EventType::Complete
                | EventType::Pause
                | EventType::Resume
                | EventType::Adjudicate
                | EventType::Modify
        ) {
            if let Some(project) = effects.consumed_project.as_ref() {
                let known = state
                    .projects
                    .get(project)
                    .context("missing project for authenticated event")?;
                let ids = parsed
                    .event
                    .payload
                    .get("milestones")
                    .and_then(Json::as_object);
                if ids.is_none_or(|ids| {
                    ids.keys().all(|id| {
                        known
                            .milestones
                            .iter()
                            .any(|milestone| &milestone.identifier == id)
                    })
                }) {
                    parsed.event.project_id = Some(project.clone());
                    events.push(parsed.event);
                } else {
                    observation(
                        state,
                        &hash,
                        point,
                        "metadata refers to an unknown milestone",
                    );
                }
            }
        } else if parsed.event.event_type != EventType::Withdraw {
            events.push(parsed.event);
        }
    }
    for derived in effects.events {
        if let Some(existing) = events.iter_mut().find(|event| {
            event.event_type == derived.event_type && event.project_id == derived.project_id
        }) {
            if let (Some(existing), Some(actual)) = (
                existing.payload.as_object_mut(),
                derived.payload.as_object(),
            ) {
                existing.extend(
                    actual
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            } else {
                existing.payload = derived.payload;
            }
        } else {
            events.push(derived);
        }
    }
    for (key, output) in produced {
        if output.kind == Kind::Vendor && output.utxo.reference_script.is_none() {
            if output.project.is_none() {
                observation(
                    state,
                    &hash,
                    point,
                    "vendor output has no proven project association",
                );
            } else if output.payout_ids.iter().any(Option::is_none) {
                observation(
                    state,
                    &hash,
                    point,
                    "one or more payouts have unresolved milestone identifiers",
                );
            }
        }
        if output.utxo.reference_script.is_some() {
            let (tag, bytes) = references::script_bytes(&output.utxo)?;
            let hash = Hasher::<224>::hash_tagged(&bytes, tag).to_string();
            let instance = state.instance.as_mut().unwrap();
            let reference = TxOutRef {
                transaction: output.utxo.tx_hash.clone(),
                index: output.utxo.tx_index,
            };
            if hash == instance.treasury_hash {
                instance.treasury_ref = Some(reference.clone());
            }
            if hash == instance.vendor_hash {
                instance.vendor_ref = Some(reference);
            }
        }
        state.outputs.insert(key, output);
    }
    state.seen.insert(hash);
    Ok(events)
}

pub fn apply_block(
    instance: &str,
    network: &str,
    previous: &State,
    block: &Block,
) -> Result<Applied> {
    ensure!(
        block.point.hash.len() == 64 && hex::decode(&block.point.hash).is_ok(),
        "invalid confirmed block hash"
    );
    let mut working: Option<State> = None;
    let mut events = Vec::new();
    let mut relevant = Vec::new();
    for bytes in &block.transactions {
        let tx = metadata::envelope(bytes)?;
        let hash = Hasher::<256>::hash(tx.transaction_body.raw_cbor()).to_string();
        let state = working.as_ref().unwrap_or(previous);
        if state.seen.contains(&hash) {
            continue;
        }
        let (meta, meta_error) = match metadata::auxiliary_1694(&tx) {
            Ok(meta) => (meta, None),
            Err(error) => (None, Some(error.to_string())),
        };
        let claim = meta.as_ref().is_some_and(|meta| {
            meta["instance"]
                .as_str()
                .is_some_and(|id| id.eq_ignore_ascii_case(instance))
        });
        let touches_input = tx
            .transaction_body
            .inputs
            .iter()
            .chain(
                tx.transaction_body
                    .collateral
                    .iter()
                    .flat_map(|inputs| inputs.iter()),
            )
            .any(|input| {
                state
                    .outputs
                    .contains_key(&outref(&input.transaction_id.to_string(), input.index))
            });
        let touches_output = if let Some(published) = &state.instance {
            MultiEraTx::from_conway(&tx)
                .produces()
                .iter()
                .any(|(_, output)| {
                    output
                        .address()
                        .ok()
                        .and_then(|address| address.to_bech32().ok())
                        .and_then(|address| {
                            payment_hash(&address, network_id(network).ok()?)
                                .ok()
                                .flatten()
                        })
                        .is_some_and(|hash| {
                            hash == published.treasury_hash || hash == published.vendor_hash
                        })
                })
        } else {
            false
        };
        if !claim && !touches_input && !touches_output {
            continue;
        }
        let state = working.get_or_insert_with(|| previous.clone());
        events.extend(apply_transaction(
            state,
            bytes,
            &tx,
            meta.as_ref(),
            meta_error.as_deref(),
            ChainContext {
                instance,
                network,
                point: &block.point,
            },
        )?);
        relevant.push(bytes.clone());
    }
    Ok(Applied {
        state: working,
        events,
        relevant: Block {
            point: block.point.clone(),
            transactions: relevant,
        },
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use minos::{coin_select::WalletUtxo, tx::publish};
    use pallas_crypto::key::ed25519::SecretKey;
    pub fn publication_fixture() -> (String, Vec<u8>, Vec<u8>) {
        publication_fixture_seed(5)
    }
    pub fn publication_fixture_seed(seed: u8) -> (String, Vec<u8>, Vec<u8>) {
        let (instance, params) = publication_params(seed);
        let raw = hex::decode(publish::build(params).unwrap().cbor_hex).unwrap();
        let signed = sign_fixture(&raw);
        (instance, signed, raw)
    }
    fn publication_params(seed: u8) -> (String, publish::PublishParams) {
        // Public deterministic test material. No wallet files or network effects.
        let key = SecretKey::from([71; 32]);
        let public: [u8; 32] = key.public_key().as_ref().try_into().unwrap();
        let author = Hasher::<224>::hash(&public).to_vec();
        let seed = [seed; 32];
        let instance =
            minos::scripts::script_hash(&minos::scripts::parameterize_oneshot(&seed, 0).unwrap())
                .to_string();
        let params = publish::PublishParams {
            seed_utxo_hash: hex::encode(seed),
            seed_utxo_index: 0,
            seed_lovelace: 1_000_000_000,
            seed_assets: Default::default(),
            admin_key_hash: author.clone(),
            admin_keys: vec![author],
            admin_threshold: 1,
            expiration_ms: 2_000_000_000_000,
            payout_upperbound_ms: 1_999_999_999_000,
            vendor_expiration_ms: 2_000_000_000_000,
            label: "Synthetic publication".into(),
            description: None,
            extra_fee_inputs: vec![],
            collateral: WalletUtxo {
                tx_hash: "06".repeat(32),
                output_index: 0,
                lovelace: 10_000_000,
                has_native_assets: false,
                assets: Default::default(),
            },
            fee_addr: "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh".into(),
            fee: 3_000_000,
            coins_per_utxo_byte: 4310,
            valid_from_slot: 100,
            invalid_from_slot: 200,
            v3_cost_model: vec![0; 297],
            network_id: 0,
        };
        (instance, params)
    }

    #[tokio::test]
    async fn staged_publication_confirms_registry_then_discovers_vendor_reference() {
        let (instance, mut params) = publication_params(5);
        let mut raw: serde_json::Value = serde_json::from_str(include_str!(
            "../../minos/tests/fixtures/withdrawal/params.json"
        ))
        .unwrap();
        raw["coins_per_utxo_size"] = 4310.into();
        let protocol: minos::koios::ProtocolParams = serde_json::from_value(raw).unwrap();
        params.v3_cost_model = protocol.v3_cost_model().unwrap();
        let bundle = publish::build_bundle(params, &protocol).unwrap();
        let store = std::sync::Arc::new(crate::db::Store::temporary());
        store.register_instance(&instance, "preview").unwrap();
        let first = Block {
            point: Point {
                slot: 100,
                hash: "01".repeat(32),
                height: 1,
            },
            transactions: vec![sign_fixture(
                &hex::decode(&bundle.publish.cbor_hex).unwrap(),
            )],
        };
        store.commit_block(&instance, &first).unwrap();
        let state = store.projection(&instance).unwrap().unwrap();
        assert!(state.state.instance.as_ref().unwrap().confirmed);
        assert!(state
            .state
            .instance
            .as_ref()
            .unwrap()
            .treasury_ref
            .is_some());
        assert!(state.state.instance.as_ref().unwrap().vendor_ref.is_none());
        let config: crate::config::Config =
            toml::from_str("koios_url='http://127.0.0.1:1'\ndevelopment_auth_bypass=true").unwrap();
        let schema = crate::schema::build(store.clone(), None, None, config);
        let query = format!("{{ instanceById(ID: \"{instance}\") {{ confirmed referencesReady registry {{ vendorScript {{ scriptReference {{ transaction index }} }} }} }} }}");
        let before = schema.execute(&query).await;
        assert!(before.errors.is_empty(), "{:?}", before.errors);
        assert_eq!(
            before.data.into_json().unwrap()["instanceById"]["referencesReady"],
            false
        );
        let second = Block {
            point: Point {
                slot: 101,
                hash: "02".repeat(32),
                height: 2,
            },
            transactions: vec![sign_fixture(
                &hex::decode(&bundle.vendor_reference.cbor_hex).unwrap(),
            )],
        };
        store.commit_block(&instance, &second).unwrap();
        let ready = schema.execute(&query).await;
        assert!(ready.errors.is_empty(), "{:?}", ready.errors);
        let ready = ready.data.into_json().unwrap();
        assert_eq!(ready["instanceById"]["referencesReady"], true);
        assert_eq!(
            ready["instanceById"]["registry"]["vendorScript"]["scriptReference"]["transaction"],
            bundle.manifest.vendor_reference_body_hash
        );
        assert_eq!(
            store.projection(&instance).unwrap().unwrap().events.len(),
            1
        );
        store.rollback_chain(&instance, Some(&first.point)).unwrap();
        assert_eq!(
            schema.execute(&query).await.data.into_json().unwrap()["instanceById"]
                ["referencesReady"],
            false
        );
    }
    pub fn lifecycle_fixture() -> (String, Vec<Block>) {
        use minos::tx::fund::{FundInput, FundParams, MilestoneSpec, TreasurySpend};
        use pallas_txbuilder::{
            BuildConway, ExUnits, Input, Output as BuiltOutput, ScriptKind, StagingTransaction,
        };
        let (instance, published, _) = publication_fixture();
        let author = Hasher::<224>::hash(SecretKey::from([71; 32]).public_key().as_ref()).to_vec();
        let (config, _) = publication(&published, &instance, "preview").unwrap();
        let publish_hash = minos::tx::sign::tx_body_hash(&published)
            .map(hex::encode)
            .unwrap();
        let fee_address = "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh";
        let protocol: minos::koios::ProtocolParams = serde_json::from_str(include_str!(
            "../../minos/tests/fixtures/withdrawal/params.json"
        ))
        .unwrap();
        let mut params = FundParams {
            treasury: None,
            instance_id: instance.clone(),
            vendor_script_hash: hex::decode(&config.vendor_hash).unwrap(),
            admin_key_hash: author.clone(),
            vendor_key_hash: Some(author.clone()),
            co_signer_hashes: vec![],
            project_id: "project".into(),
            label: "Synthetic".into(),
            description: "".into(),
            vendor_label: "Vendor".into(),
            milestones: ["m-1", "m-10", "m-2"]
                .into_iter()
                .map(|id| MilestoneSpec {
                    id: id.into(),
                    label: id.into(),
                    description: None,
                    acceptance_criteria: None,
                    requires_evidence: false,
                    maturation_ms: 1_666_656_100_000,
                    lovelace: 1_000_000,
                    token_amount: 0,
                })
                .collect(),
            token_policy: None,
            token_name: None,
            inputs: vec![FundInput {
                tx_hash: "11".repeat(32),
                output_index: 0,
                lovelace: 20_000_000,
                token_amount: 0,
            }],
            vendor_lovelace_total: 10_000_000,
            change_lovelace: 9_000_000,
            change_tokens: 0,
            extra_change_assets: vec![],
            fee_addr: fee_address.into(),
            fee: 1_000_000,
            coins_per_utxo_byte: 4310,
            valid_from_slot: 100,
            invalid_from_slot: 300,
            network_id: 0,
        };
        let mut spoof_params = params.clone();
        spoof_params.inputs[0].tx_hash = "14".repeat(32);
        let spoof = minos::tx::fund::build(spoof_params).unwrap();
        let spoof = sign_fixture(&hex::decode(spoof.cbor_hex).unwrap());
        let deposit = minos::tx::fund::build_deposit(
            params.clone(),
            &hex::decode(&config.treasury_hash).unwrap(),
        )
        .unwrap();
        let deposit = sign_fixture(&hex::decode(deposit.cbor_hex).unwrap());
        let deposit_hash = minos::tx::sign::tx_body_hash(&deposit)
            .map(hex::encode)
            .unwrap();
        params.inputs = vec![
            FundInput {
                tx_hash: deposit_hash.clone(),
                output_index: 0,
                lovelace: 10_000_000,
                token_amount: 0,
            },
            FundInput {
                tx_hash: "12".repeat(32),
                output_index: 0,
                lovelace: 9_000_000,
                token_amount: 0,
            },
        ];
        params.change_lovelace = 1_000_000;
        params.treasury = Some(TreasurySpend {
            input: (deposit_hash, 0),
            references: vec![(publish_hash.clone(), 0), (publish_hash.clone(), 1)],
            collateral: WalletUtxo {
                tx_hash: "13".repeat(32),
                output_index: 0,
                lovelace: 10_000_000,
                has_native_assets: false,
                assets: Default::default(),
            },
            cost_model: protocol.v3_cost_model().unwrap(),
            return_address: minos::scripts::base_script_addr(
                &<[u8; 28]>::try_from(hex::decode(&config.treasury_hash).unwrap())
                    .unwrap()
                    .into(),
                0,
            )
            .to_bech32()
            .unwrap(),
            return_lovelace: 7_000_000,
            return_assets: vec![],
        });
        let funded = minos::tx::fund::build(params).unwrap();
        let funded = sign_fixture(&hex::decode(funded.cbor_hex).unwrap());
        let funded_tx = metadata::transaction(&funded).unwrap();
        let vendor_index = funded_tx
            .transaction_body
            .outputs
            .iter()
            .position(|output| {
                let address = pallas_traverse::MultiEraOutput::from_conway(output)
                    .address()
                    .unwrap()
                    .to_bech32()
                    .unwrap();
                payment_hash(&address, 0).unwrap().as_deref() == Some(&config.vendor_hash)
            })
            .unwrap();
        let vendor_utxo = minos::preflight::utxo_from_output(
            &Hasher::<256>::hash(funded_tx.transaction_body.raw_cbor()).to_string(),
            vendor_index as u64,
            &funded_tx.transaction_body.outputs[vendor_index],
        )
        .unwrap();
        let mut paused = VendorDatum::from_cbor_hex(&vendor_utxo.datum_hex().unwrap()).unwrap();
        paused.payouts[0].status = minos::types::PayoutStatus::Paused;
        let transition = |previous: &Utxo,
                          after: &VendorDatum,
                          coin: u64,
                          action: u64,
                          lower: u64,
                          payment: u64| {
            use pallas_codec::utils::MaybeIndefArray;
            use pallas_primitives::conway::Constr;
            let fields = if action == 1 {
                vec![PlutusData::Array(MaybeIndefArray::Def(
                    after
                        .payouts
                        .iter()
                        .map(|payout| {
                            PlutusData::Constr(Constr {
                                tag: if matches!(payout.status, minos::types::PayoutStatus::Paused)
                                {
                                    122
                                } else {
                                    121
                                },
                                any_constructor: None,
                                fields: MaybeIndefArray::Def(vec![]),
                            })
                        })
                        .collect(),
                ))]
            } else {
                vec![]
            };
            let redeemer = pallas_codec::minicbor::to_vec(PlutusData::Constr(Constr {
                tag: 121 + action,
                any_constructor: None,
                fields: MaybeIndefArray::Def(fields),
            }))
            .unwrap();
            let input = Input::new(
                <[u8; 32]>::try_from(hex::decode(&previous.tx_hash).unwrap())
                    .unwrap()
                    .into(),
                previous.tx_index,
            );
            let mut staging = StagingTransaction::new()
                .input(input.clone())
                .input(Input::new([action as u8 + 40; 32].into(), 0))
                .collateral_input(Input::new([30; 32].into(), 0))
                .reference_input(Input::new(
                    <[u8; 32]>::try_from(hex::decode(&publish_hash).unwrap())
                        .unwrap()
                        .into(),
                    0,
                ))
                .reference_input(Input::new(
                    <[u8; 32]>::try_from(hex::decode(&publish_hash).unwrap())
                        .unwrap()
                        .into(),
                    2,
                ))
                .output(
                    BuiltOutput::new(
                        pallas_addresses::Address::from_bech32(&previous.address).unwrap(),
                        coin,
                    )
                    .set_inline_datum(after.to_cbor_bytes()),
                )
                .fee(1_000_000)
                .valid_from_slot(lower)
                .invalid_from_slot(300)
                .network_id(0)
                .disclosed_signer(<[u8; 28]>::try_from(author.as_slice()).unwrap().into())
                .add_spend_redeemer(
                    input,
                    redeemer,
                    Some(ExUnits {
                        mem: 5_000_000,
                        steps: 2_000_000_000,
                    }),
                )
                .add_language(ScriptKind::PlutusV3, protocol.v3_cost_model().unwrap());
            if payment > 0 {
                staging = staging.output(BuiltOutput::new(
                    pallas_addresses::Address::from_bech32(fee_address).unwrap(),
                    payment,
                ));
            }
            sign_fixture(&staging.build_conway_raw().unwrap().tx_bytes.0)
        };
        let adjudicate_params = minos::tx::adjudicate::AdjudicateParams {
            coins_per_utxo_byte: 4310,
            vendor_utxo_hash: vendor_utxo.tx_hash.clone(),
            vendor_utxo_index: vendor_utxo.tx_index,
            vendor_address: vendor_utxo.address.clone(),
            current_datum: VendorDatum::from_cbor_hex(&vendor_utxo.datum_hex().unwrap()).unwrap(),
            new_statuses: paused
                .payouts
                .iter()
                .map(|payout| payout.status.clone())
                .collect(),
            vendor_lovelace: 10_000_000,
            vendor_assets: value(&vendor_utxo).unwrap().assets.into_iter().collect(),
            registry_utxo_hash: publish_hash.clone(),
            registry_utxo_index: 0,
            vendor_script_ref_hash: publish_hash.clone(),
            vendor_script_ref_index: 2,
            admin_key_hash: author.clone(),
            co_signer_hashes: vec![],
            fee_inputs: vec![WalletUtxo {
                tx_hash: hex::encode([41; 32]),
                output_index: 0,
                lovelace: 2_000_000,
                has_native_assets: false,
                assets: Default::default(),
            }],
            collateral: WalletUtxo {
                tx_hash: hex::encode([30; 32]),
                output_index: 0,
                lovelace: 10_000_000,
                has_native_assets: false,
                assets: Default::default(),
            },
            fee_addr: fee_address.into(),
            fee: 1_000_000,
            change_lovelace: 1_000_000,
            valid_from_slot: 100,
            invalid_from_slot: 300,
            v3_cost_model: protocol.v3_cost_model().unwrap(),
            instance_id: instance.clone(),
            network_id: 0,
        };
        let mut invalid_statuses = adjudicate_params.clone();
        invalid_statuses.new_statuses.clear();
        assert!(minos::tx::adjudicate::build(invalid_statuses).is_err());
        let adjudicated = minos::tx::adjudicate::build(adjudicate_params).unwrap();
        let adjudicated = sign_fixture(&hex::decode(adjudicated.cbor_hex).unwrap());
        let tx = metadata::transaction(&adjudicated).unwrap();
        let current = minos::preflight::utxo_from_output(
            &Hasher::<256>::hash(tx.transaction_body.raw_cbor()).to_string(),
            0,
            &tx.transaction_body.outputs[0],
        )
        .unwrap();
        let mut remaining = paused;
        remaining.payouts.truncate(1);
        let withdrawn = transition(&current, &remaining, 8_000_000, 0, 200, 2_000_000);
        let withdrawn_tx = metadata::transaction(&withdrawn).unwrap();
        let current = minos::preflight::utxo_from_output(
            &Hasher::<256>::hash(withdrawn_tx.transaction_body.raw_cbor()).to_string(),
            0,
            &withdrawn_tx.transaction_body.outputs[0],
        )
        .unwrap();
        let mut resumed = remaining;
        resumed.payouts[0].status = minos::types::PayoutStatus::Active;
        let modify_params = minos::tx::modify::ModifyParams {
            coins_per_utxo_byte: 4310,
            vendor_utxo_hash: current.tx_hash.clone(),
            vendor_utxo_index: 0,
            vendor_address: current.address.clone(),
            new_datum_bytes: resumed.to_cbor_bytes(),
            vendor_lovelace: 8_000_000,
            vendor_assets: value(&current).unwrap().assets.into_iter().collect(),
            registry_utxo_hash: publish_hash.clone(),
            registry_utxo_index: 0,
            vendor_script_ref_hash: publish_hash.clone(),
            vendor_script_ref_index: 2,
            admin_key_hash: author.clone(),
            vendor_key_hashes: vec![author.clone()],
            staking_script_hashes: vec![],
            staking_script_refs: vec![],
            staking_reward_mem: 0,
            staking_reward_steps: 0,
            fee_inputs: vec![WalletUtxo {
                tx_hash: hex::encode([50; 32]),
                output_index: 0,
                lovelace: 2_000_000,
                has_native_assets: false,
                assets: Default::default(),
            }],
            collateral: WalletUtxo {
                tx_hash: hex::encode([30; 32]),
                output_index: 0,
                lovelace: 10_000_000,
                has_native_assets: false,
                assets: Default::default(),
            },
            fee_addr: fee_address.into(),
            fee: 1_000_000,
            change_lovelace: 1_000_000,
            valid_from_slot: 201,
            invalid_from_slot: 300,
            v3_cost_model: protocol.v3_cost_model().unwrap(),
            instance_id: instance.clone(),
            network_id: 0,
        };
        let modified = minos::tx::modify::build(modify_params).unwrap();
        let modified = sign_fixture(&hex::decode(modified.cbor_hex).unwrap());
        let mut blocks: Vec<Block> = [
            (100, published),
            (101, deposit),
            (102, funded),
            (103, adjudicated),
            (200, withdrawn),
            (201, modified),
        ]
        .into_iter()
        .enumerate()
        .map(|(height, (slot, bytes))| Block {
            point: Point {
                slot,
                hash: format!("{:064x}", height + 1),
                height: height as u64 + 1,
            },
            transactions: vec![bytes],
        })
        .collect();
        blocks[2].transactions.insert(0, spoof);
        (instance, blocks)
    }
    fn sign_fixture(bytes: &[u8]) -> Vec<u8> {
        let key = SecretKey::from([71; 32]);
        let public = key.public_key().as_ref().try_into().unwrap();
        let hash = minos::tx::sign::tx_body_hash(bytes).unwrap();
        minos::tx::sign::add_vkey_witness(
            bytes,
            &public,
            &key.sign(hash).as_ref().try_into().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn lifecycle_transactions_execute_the_pinned_contracts() {
        let (instance, blocks) = lifecycle_fixture();
        let protocol: minos::koios::ProtocolParams = serde_json::from_str(include_str!(
            "../../minos/tests/fixtures/withdrawal/params.json"
        ))
        .unwrap();
        let mut state = State::default();
        for block in blocks {
            for bytes in &block.transactions {
                let tx = metadata::transaction(bytes).unwrap();
                let keys = tx
                    .transaction_body
                    .inputs
                    .iter()
                    .chain(
                        tx.transaction_body
                            .collateral
                            .iter()
                            .flat_map(|inputs| inputs.iter()),
                    )
                    .chain(
                        tx.transaction_body
                            .reference_inputs
                            .iter()
                            .flat_map(|inputs| inputs.iter()),
                    )
                    .map(|input| (input.transaction_id.to_string(), input.index))
                    .collect::<BTreeSet<_>>();
                let mut resolved = Vec::new();
                for (hash, index) in keys {
                    if let Some(output) = state.outputs.get(&outref(&hash, index)) {
                        resolved.push(output.utxo.clone());
                    } else {
                        let first = hex::decode(&hash).unwrap()[0];
                        let coin = match first {
                            5 => 1_000_000_000,
                            6 | 19 | 30 => 10_000_000,
                            17 | 20 => 20_000_000,
                            18 => 9_000_000,
                            40 => 1_000_000,
                            41 | 50 => 2_000_000,
                            _ => panic!("unexpected fixture input {hash}"),
                        };
                        resolved.push(serde_json::from_value(json!({"tx_hash":hash,"tx_index":index,"address":"addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh","value":coin.to_string(),"asset_list":[]})).unwrap());
                    }
                }
                minos::preflight::evaluate(
                    bytes,
                    &resolved,
                    protocol.evaluation_limits().unwrap(),
                    &protocol.v3_cost_model().unwrap(),
                    1_666_656_000,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "fixture contract failure at slot {}: {error:#}",
                        block.point.slot
                    )
                });
                if matches!(block.point.slot, 103 | 201) {
                    // Exercise the actual adjudicate and modify builders' pinned
                    // contracts with their declared authorization removed.
                    let mut unauthorized = metadata::transaction(bytes).unwrap();
                    unauthorized.transaction_body.required_signers = None;
                    let altered = pallas_codec::minicbor::to_vec(&unauthorized).unwrap();
                    assert!(
                        minos::preflight::evaluate(
                            &altered,
                            &resolved,
                            protocol.evaluation_limits().unwrap(),
                            &protocol.v3_cost_model().unwrap(),
                            1_666_656_000,
                        )
                        .is_err(),
                        "missing authorization accepted at slot {}",
                        block.point.slot
                    );
                }
                let single = Block {
                    point: block.point.clone(),
                    transactions: vec![bytes.clone()],
                };
                if let Some(next) = apply_block(&instance, "preview", &state, &single)
                    .unwrap()
                    .state
                {
                    state = next;
                }
            }
        }
    }

    #[test]
    fn publication_requires_witness_registry_seed_policy_and_network() {
        let (instance, signed, unsigned) = publication_fixture();
        let (pending, registry) = publication(&signed, &instance, "preview").unwrap();
        assert!(!pending.confirmed);
        assert_eq!(registry.tx_index, 0);
        assert!(pending.treasury_ref.is_some() && pending.vendor_ref.is_some());
        assert!(publication(&unsigned, &instance, "preview").is_err());
        assert!(publication(&signed, &"09".repeat(28), "preview").is_err());
        assert!(publication(&signed, &instance, "mainnet").is_err());
        let block = Block {
            point: Point {
                slot: 100,
                hash: "aa".repeat(32),
                height: 1,
            },
            transactions: vec![unsigned, signed],
        };
        let applied = apply_block(&instance, "preview", &State::default(), &block).unwrap();
        let state = applied.state.unwrap();
        assert!(state.instance.unwrap().confirmed);
        assert_eq!(applied.events.len(), 1);
        assert!(state.projects.is_empty());
        assert!(state.current.is_empty());
        assert_eq!(state.observations.len(), 1);
    }
}
