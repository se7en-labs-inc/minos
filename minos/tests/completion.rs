use minos::{
    coin_select::WalletUtxo,
    koios::{ProtocolParams, Utxo},
    tx::complete,
};
use pallas_codec::minicbor;
use pallas_primitives::conway::{DatumOption, TransactionOutput, Tx};
use pallas_traverse::MultiEraOutput;

fn fixture() -> (complete::CompleteParams, Vec<Utxo>, ProtocolParams) {
    let mut inputs: Vec<Utxo> =
        serde_json::from_str(include_str!("fixtures/withdrawal/utxos.json")).unwrap();
    // A zero-withdraw completion is valid while no active payout is matured.
    let vendor = inputs
        .iter_mut()
        .find(|u| u.tx_hash.starts_with("1804"))
        .unwrap();
    let mut datum = minos::types::VendorDatum::from_cbor_hex(&vendor.datum_hex().unwrap()).unwrap();
    for payout in &mut datum.payouts {
        payout.status = minos::types::PayoutStatus::Paused;
    }
    vendor.inline_datum = Some(serde_json::json!({"bytes": hex::encode(datum.to_cbor_bytes())}));
    vendor.output_cbor = None;
    let vendor = inputs
        .iter()
        .find(|u| u.tx_hash.starts_with("1804"))
        .unwrap();
    let fee = inputs
        .iter()
        .find(|u| u.tx_hash.starts_with("70c6"))
        .unwrap();
    let collateral = inputs
        .iter()
        .find(|u| u.tx_hash.starts_with("ff9d"))
        .unwrap();
    let proto: ProtocolParams =
        serde_json::from_str(include_str!("fixtures/withdrawal/params.json")).unwrap();
    let datum = minos::types::VendorDatum::from_cbor_hex(&vendor.datum_hex().unwrap()).unwrap();
    let assets = vendor.asset_list.as_ref().unwrap();
    let params = complete::CompleteParams {
        coins_per_utxo_byte: 4310,
        vendor_utxo_hash: vendor.tx_hash.clone(),
        vendor_utxo_index: vendor.tx_index,
        vendor_address: vendor.address.clone(),
        vendor_datum_hex: vendor.datum_hex().unwrap(),
        vendor_value_lovelace: vendor.lovelace(),
        vendor_value_assets: assets
            .iter()
            .map(|a| {
                (
                    format!("{}{}", a.policy_id, a.asset_name),
                    a.quantity.parse().unwrap(),
                )
            })
            .collect(),
        registry_utxo_hash: "fe2a314561f787aa55f2cf375438a7c8d4f5d090c82292c0e0d6699aec11e722"
            .into(),
        registry_utxo_index: 0,
        vendor_script_ref_hash: "c307d0fcd0b63f6e31d44d7bb1726661b949f9f50eac60be7f81fb6748d35472"
            .into(),
        vendor_script_ref_index: 0,
        vendor_key_hashes: datum.vendor.key_hashes(),
        fee_key_hash: Some(vec![42; 28]),
        staking_script_hashes: vec![],
        staking_script_refs: vec![],
        staking_reward_mem: 2_500_000,
        staking_reward_steps: 900_000_000,
        fee_inputs: vec![WalletUtxo::try_from(fee).unwrap()],
        collateral: WalletUtxo::try_from(collateral).unwrap(),
        fee_addr: fee.address.clone(),
        fee: 1_000_000,
        change_lovelace: fee.lovelace() - 1_000_000,
        valid_from_slot: 200_000_000,
        invalid_from_slot: 200_007_200,
        v3_cost_model: proto.v3_cost_model().unwrap(),
        instance_id: "51486a2f1496d4d3a688a9b111971aa9b731ed045d900b601345ca4e".into(),
        project_id: "EXAMPLE-001".into(),
        milestone_id: "m-7".into(),
        description: "Signed MAF".into(),
        evidence_urls: vec![format!(
            "ipfs://{}",
            minos::evidence::raw_cid(b"synthetic evidence".as_slice()).unwrap()
        )],
        evidence_labels: vec![],
        network_id: 1,
    };
    (params, inputs, proto)
}

#[test]
fn evidence_can_preserve_active_payout_maturing_inside_validity_interval() {
    let (mut params, mut inputs, proto) = fixture();
    let vendor = inputs
        .iter_mut()
        .find(|u| u.tx_hash == params.vendor_utxo_hash)
        .unwrap();
    let mut datum = minos::types::VendorDatum::from_cbor_hex(&params.vendor_datum_hex).unwrap();
    datum.payouts[0].status = minos::types::PayoutStatus::Active;
    datum.payouts[0].maturation = (params.valid_from_slot + 1_591_566_291 + 3600) * 1000;
    let tip_after_maturation = params.valid_from_slot + 4000;
    let lower = complete::validity_start(tip_after_maturation, 1_591_566_291, &datum);
    assert!((lower + 1_591_566_291) * 1000 < datum.payouts[0].maturation);
    assert!(tip_after_maturation > lower);
    params.valid_from_slot = lower;
    params.invalid_from_slot = tip_after_maturation + 7200;
    params.vendor_datum_hex = hex::encode(datum.to_cbor_bytes());
    vendor.inline_datum = Some(serde_json::json!({"bytes": params.vendor_datum_hex}));
    vendor.output_cbor = None;
    params.staking_script_hashes =
        vec![
            hex::decode("326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb")
                .unwrap()
                .try_into()
                .unwrap(),
        ];
    params.staking_script_refs = vec![(
        "aaec1b8402f1d7951a86a13ed05114baf32549841a5c288d8580b367bb5063c8".into(),
        0,
    )];
    let built = complete::build(params).unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let needed: std::collections::BTreeSet<_> = tx
        .transaction_body
        .inputs
        .iter()
        .chain(
            tx.transaction_body
                .reference_inputs
                .iter()
                .flat_map(|v| v.iter()),
        )
        .chain(tx.transaction_body.collateral.iter().flat_map(|v| v.iter()))
        .map(|i| (i.transaction_id.to_string(), i.index))
        .collect();
    inputs.retain(|u| needed.contains(&(u.tx_hash.clone(), u.tx_index)));
    let report = minos::preflight::evaluate(
        &bytes,
        &inputs,
        proto.evaluation_limits().unwrap(),
        &proto.v3_cost_model().unwrap(),
        1_591_566_291,
    )
    .unwrap();
    assert_eq!(report.redeemers, 2);
}

#[test]
fn completion_labels_remain_bound_to_urls_and_support_long_filenames() {
    let (mut params, _, _) = fixture();
    params.evidence_urls = vec!["ipfs://first".into(), "ipfs://second".into()];
    let long_label = format!("{}.pdf", "証拠".repeat(20));
    params.evidence_labels = vec!["September MAF.pdf".into(), long_label.clone()];
    let built = complete::build(params.clone()).unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    fn norm(v: &serde_json::Value) -> serde_json::Value {
        if let Some(t) = v.get("Text") {
            return t.clone();
        }
        if let Some(a) = v.get("Array") {
            return serde_json::Value::Array(a.as_array().unwrap().iter().map(norm).collect());
        }
        if let Some(m) = v.get("Map") {
            return serde_json::Value::Object(
                m.as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (norm(&p[0]).as_str().unwrap().to_owned(), norm(&p[1])))
                    .collect(),
            );
        }
        v.clone()
    }
    let aux = serde_json::to_value(&tx.auxiliary_data).unwrap();
    let meta = norm(&aux["Shelley"]["1694"]);
    let evidence = &meta["body"]["milestones"]["m-7"]["evidence"];
    assert_eq!(
        evidence[0],
        serde_json::json!({"anchorUrl":"ipfs://first","label":"September MAF.pdf"})
    );
    assert_eq!(evidence[1]["anchorUrl"], "ipfs://second");
    let chunks = evidence[1]["label"].as_array().unwrap();
    assert!(chunks.iter().all(|v| v.as_str().unwrap().len() <= 64));
    assert_eq!(
        chunks
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<String>(),
        long_label
    );
    params.evidence_labels.pop();
    assert!(complete::build(params)
        .unwrap_err()
        .to_string()
        .contains("label count"));
}

#[test]
fn completion_references_only_vendor_and_registry_and_preserves_vendor_output() {
    let (params, inputs, _) = fixture();
    let vendor = inputs
        .iter()
        .find(|u| u.tx_hash.starts_with("1804"))
        .unwrap();
    let assets = vendor.asset_list.as_ref().unwrap();
    let built = complete::build(params).unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let refs: Vec<_> = tx
        .transaction_body
        .reference_inputs
        .as_ref()
        .unwrap()
        .iter()
        .map(|r| (r.transaction_id.to_string(), r.index))
        .collect();
    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&(
        "fe2a314561f787aa55f2cf375438a7c8d4f5d090c82292c0e0d6699aec11e722".into(),
        0
    )));
    assert!(refs.contains(&(
        "c307d0fcd0b63f6e31d44d7bb1726661b949f9f50eac60be7f81fb6748d35472".into(),
        0
    )));
    let output = MultiEraOutput::from_conway(&tx.transaction_body.outputs[0]);
    assert_eq!(
        output.address().unwrap().to_bech32().unwrap(),
        vendor.address
    );
    assert_eq!(output.value().coin(), vendor.lovelace());
    let mut actual_assets = std::collections::BTreeMap::new();
    for policy in output.value().assets() {
        for asset in policy.assets() {
            actual_assets.insert(
                (
                    hex::encode(policy.policy().as_ref()),
                    hex::encode(asset.name()),
                ),
                asset.any_coin(),
            );
        }
    }
    let expected_assets: std::collections::BTreeMap<_, i128> = assets
        .iter()
        .map(|asset| {
            (
                (asset.policy_id.clone(), asset.asset_name.clone()),
                asset.quantity.parse().unwrap(),
            )
        })
        .collect();
    assert_eq!(actual_assets, expected_assets);
    let TransactionOutput::PostAlonzo(output) = &tx.transaction_body.outputs[0] else {
        panic!("inline datum required")
    };
    let DatumOption::Data(data) = &**output.datum_option.as_ref().unwrap() else {
        panic!("inline datum required")
    };
    assert_eq!(
        minicbor::to_vec(&data.0).unwrap(),
        hex::decode(vendor.datum_hex().unwrap()).unwrap()
    );
    assert_eq!(tx.transaction_body.fee, 1_000_000);
}

#[test]
fn completion_evaluates_vendor_and_required_allowlist_authorization() {
    let (mut params, inputs, proto) = fixture();
    let evaluate = |params: complete::CompleteParams| {
        let bytes = hex::decode(complete::build(params).unwrap().cbor_hex).unwrap();
        let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
        let refs: std::collections::BTreeSet<_> = tx
            .transaction_body
            .inputs
            .iter()
            .chain(
                tx.transaction_body
                    .reference_inputs
                    .iter()
                    .flat_map(|v| v.iter()),
            )
            .chain(tx.transaction_body.collateral.iter().flat_map(|v| v.iter()))
            .map(|i| (i.transaction_id.to_string(), i.index))
            .collect();
        let resolved: Vec<_> = inputs
            .iter()
            .filter(|u| refs.contains(&(u.tx_hash.clone(), u.tx_index)))
            .cloned()
            .collect();
        minos::preflight::evaluate(
            &bytes,
            &resolved,
            proto.evaluation_limits().unwrap(),
            &proto.v3_cost_model().unwrap(),
            1_591_566_291,
        )
    };
    assert!(
        evaluate(params.clone()).is_err(),
        "missing allowlist authorization must fail"
    );
    params.staking_script_hashes =
        vec![
            hex::decode("326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb")
                .unwrap()
                .try_into()
                .unwrap(),
        ];
    params.staking_script_refs = vec![(
        "aaec1b8402f1d7951a86a13ed05114baf32549841a5c288d8580b367bb5063c8".into(),
        0,
    )];
    let report = evaluate(params).unwrap();
    assert_eq!(report.redeemers, 2);
}

#[test]
fn completion_metadata_binds_vendor_author_to_required_signer_and_body() {
    let (params, _, _) = fixture();
    let expected_author = hex::encode(&params.vendor_key_hashes[0]);
    let bytes = hex::decode(complete::build(params).unwrap().cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let mut decoder = minicbor::Decoder::new(&bytes);
    assert_eq!(decoder.array().unwrap(), Some(4));
    for _ in 0..3 {
        decoder.skip().unwrap();
    }
    let aux_start = decoder.position();
    assert_eq!(decoder.map().unwrap(), Some(1));
    assert_eq!(decoder.u64().unwrap(), 1694);
    let mut author = None;
    for _ in 0..decoder.map().unwrap().unwrap() {
        let name = decoder.str().unwrap();
        if name == "txAuthor" {
            assert!(author.is_none(), "duplicate author");
            author = Some(decoder.str().unwrap());
        } else {
            decoder.skip().unwrap();
        }
    }
    assert_eq!(author, Some(expected_author.as_str()));
    assert!(tx
        .transaction_body
        .required_signers
        .as_ref()
        .unwrap()
        .iter()
        .any(|key| hex::encode(key) == expected_author));
    let aux_hash = pallas_crypto::hash::Hasher::<256>::hash(&bytes[aux_start..]);
    assert_eq!(tx.transaction_body.auxiliary_data_hash, Some(aux_hash));
}

#[test]
fn completion_rejects_missing_or_malformed_vendor_author() {
    let (mut params, _, _) = fixture();
    params.vendor_key_hashes.clear();
    assert!(complete::build(params.clone())
        .unwrap_err()
        .to_string()
        .contains("vendor author"));
    for length in [0, 27, 29] {
        params.vendor_key_hashes = vec![vec![7; length]];
        assert!(complete::build(params.clone())
            .unwrap_err()
            .to_string()
            .contains("28-byte"));
    }
}

fn evaluate_authorization(
    params: complete::CompleteParams,
    inputs: &[Utxo],
    proto: &ProtocolParams,
) -> anyhow::Result<minos::preflight::Report> {
    let bytes = hex::decode(complete::build(params)?.cbor_hex)?;
    let tx: Tx<'_> = minicbor::decode(&bytes)?;
    let needed: std::collections::BTreeSet<_> = tx
        .transaction_body
        .inputs
        .iter()
        .chain(
            tx.transaction_body
                .reference_inputs
                .iter()
                .flat_map(|v| v.iter()),
        )
        .chain(tx.transaction_body.collateral.iter().flat_map(|v| v.iter()))
        .map(|i| (i.transaction_id.to_string(), i.index))
        .collect();
    let resolved: Vec<_> = inputs
        .iter()
        .filter(|u| needed.contains(&(u.tx_hash.clone(), u.tx_index)))
        .cloned()
        .collect();
    minos::preflight::evaluate(
        &bytes,
        &resolved,
        proto.evaluation_limits()?,
        &proto.v3_cost_model().expect("fixture cost model"),
        1_591_566_291,
    )
}

#[test]
fn completion_accepts_a_threshold_subset_and_rejects_insufficient_signers() {
    let (mut params, mut inputs, proto) = fixture();
    let mut datum = minos::types::VendorDatum::from_cbor_hex(&params.vendor_datum_hex).unwrap();
    datum.vendor = minos::types::MultisigScript::AtLeast {
        required: 2,
        scripts: (1..=3)
            .map(|key| minos::types::MultisigScript::Signature {
                key_hash: vec![key; 28],
            })
            .collect(),
    };
    params.vendor_datum_hex = hex::encode(datum.to_cbor_bytes());
    let vendor = inputs
        .iter_mut()
        .find(|u| u.tx_hash == params.vendor_utxo_hash)
        .unwrap();
    vendor.inline_datum = Some(serde_json::json!({"bytes": params.vendor_datum_hex}));
    vendor.output_cbor = None;
    params.vendor_key_hashes = vec![vec![1; 28], vec![2; 28]];
    params.staking_script_hashes =
        vec![
            hex::decode("326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb")
                .unwrap()
                .try_into()
                .unwrap(),
        ];
    params.staking_script_refs = vec![(
        "aaec1b8402f1d7951a86a13ed05114baf32549841a5c288d8580b367bb5063c8".into(),
        0,
    )];
    assert_eq!(
        evaluate_authorization(params.clone(), &inputs, &proto)
            .unwrap()
            .redeemers,
        2
    );
    params.vendor_key_hashes.pop();
    assert!(evaluate_authorization(params, &inputs, &proto).is_err());
}

#[test]
fn completion_supports_script_only_vendor_with_an_explicit_metadata_actor() {
    let (mut params, mut inputs, proto) = fixture();
    let script = hex::decode("326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb").unwrap();
    let mut datum = minos::types::VendorDatum::from_cbor_hex(&params.vendor_datum_hex).unwrap();
    datum.vendor = minos::types::MultisigScript::Script {
        script_hash: script.clone(),
    };
    params.vendor_datum_hex = hex::encode(datum.to_cbor_bytes());
    let vendor = inputs
        .iter_mut()
        .find(|u| u.tx_hash == params.vendor_utxo_hash)
        .unwrap();
    vendor.inline_datum = Some(serde_json::json!({"bytes": params.vendor_datum_hex}));
    vendor.output_cbor = None;
    assert!(evaluate_authorization(params.clone(), &inputs, &proto).is_err());
    params.staking_script_hashes = vec![script.try_into().unwrap()];
    params.staking_script_refs = vec![(
        "aaec1b8402f1d7951a86a13ed05114baf32549841a5c288d8580b367bb5063c8".into(),
        0,
    )];
    assert_eq!(
        evaluate_authorization(params, &inputs, &proto)
            .unwrap()
            .redeemers,
        2
    );
}

#[test]
fn completion_funds_large_datum_minimum_from_wallet_without_losing_assets() {
    let (mut params, _, _) = fixture();
    params.coins_per_utxo_byte = 20_000;
    params.vendor_value_lovelace = 1;
    params.change_lovelace = 50_000_000;
    let bytes = hex::decode(complete::build(params.clone()).unwrap().cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let vendor = MultiEraOutput::from_conway(&tx.transaction_body.outputs[0]);
    let change = MultiEraOutput::from_conway(&tx.transaction_body.outputs[1]);
    assert!(vendor.value().coin() > 2_000_000);
    assert_eq!(vendor.value().coin() + change.value().coin(), 50_000_001);
    assert_eq!(vendor.value().assets().len(), 1);
    params.change_lovelace = 0;
    assert!(complete::build(params)
        .unwrap_err()
        .to_string()
        .contains("minimum ADA"));
}
