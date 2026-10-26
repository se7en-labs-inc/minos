use minos::{
    koios::{ProtocolParams, Utxo},
    preflight, references,
};
use pallas_codec::minicbor;
use pallas_primitives::conway::Tx;

fn inputs() -> Vec<Utxo> {
    serde_json::from_str(include_str!("fixtures/withdrawal/utxos.json")).unwrap()
}
fn params() -> ProtocolParams {
    serde_json::from_str(include_str!("fixtures/withdrawal/params.json")).unwrap()
}
fn original() -> Vec<u8> {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/withdrawal/original.json")).unwrap();
    hex::decode(json["cborHex"].as_str().unwrap()).unwrap()
}

fn original_inputs() -> Vec<Utxo> {
    inputs()
}

#[test]
fn missing_inputs_and_unbalanced_values_fail_before_evaluation() {
    let bytes = original();
    let params = params();
    let evaluate = |us: &[Utxo]| {
        preflight::evaluate(
            &bytes,
            us,
            params.evaluation_limits().unwrap(),
            &params.v3_cost_model().unwrap(),
            1_591_566_291,
        )
    };
    let mut us = original_inputs();
    us.retain(|u| !u.tx_hash.starts_with("aaec"));
    assert!(evaluate(&us)
        .unwrap_err()
        .to_string()
        .contains("missing UTxO"));
    let mut us = original_inputs();
    us.iter_mut()
        .find(|u| u.tx_hash.starts_with("70c6"))
        .unwrap()
        .value = "20000001".into();
    assert!(evaluate(&us)
        .unwrap_err()
        .to_string()
        .contains("not conserved"));
    let mut us = original_inputs();
    us.iter_mut()
        .find(|u| u.tx_hash.starts_with("1804"))
        .unwrap()
        .asset_list
        .as_mut()
        .unwrap()[0]
        .quantity = "1".into();
    assert!(evaluate(&us)
        .unwrap_err()
        .to_string()
        .contains("not conserved"));
}

#[test]
fn fixture_preserves_payouts_destination_and_staking_requirement() {
    let us = original_inputs();
    let vendor = us.iter().find(|u| u.tx_hash.starts_with("1804")).unwrap();
    let datum = minos::types::VendorDatum::from_cbor_hex(&vendor.datum_hex().unwrap()).unwrap();
    let bytes = original();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let output = pallas_traverse::MultiEraOutput::from_conway(&tx.transaction_body.outputs[0]);
    assert_eq!(
        output.address().unwrap().to_bech32().unwrap(),
        "addr1wystdkxqmsrhyxkzj90jaqhy0zq2m2ehjvmwjvmamsnth7ga8mhgs"
    );
    assert_eq!(output.value().coin(), 2_000_000);
    assert_eq!(datum.payouts.len(), 19);
    assert_eq!(
        datum.vendor.script_hashes(),
        vec![hex::decode("326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb").unwrap()]
    );
    assert_eq!(tx.transaction_body.withdrawals.as_ref().unwrap().len(), 1);
    let lower_ms = (tx.transaction_body.validity_interval_start.unwrap() + 1_591_566_291) * 1000;
    let expected = minos::types::VendorDatum {
        vendor: datum.vendor.clone(),
        payouts: datum
            .payouts
            .iter()
            .filter(|p| {
                !(matches!(p.status, minos::types::PayoutStatus::Active) && p.maturation < lower_ms)
            })
            .cloned()
            .collect(),
    };
    assert_eq!(expected.payouts.len(), 18);
    let pallas_primitives::conway::TransactionOutput::PostAlonzo(remaining) =
        &tx.transaction_body.outputs[1]
    else {
        panic!("inline remaining datum required");
    };
    let pallas_primitives::conway::DatumOption::Data(data) =
        &**remaining.datum_option.as_ref().unwrap()
    else {
        panic!("inline datum required");
    };
    assert_eq!(minicbor::to_vec(&data.0).unwrap(), expected.to_cbor_bytes());
}

#[test]
fn reference_data_is_complete_deduplicated_and_hash_checked() {
    let inputs = inputs();
    let refs: Vec<_> = inputs
        .iter()
        .map(|u| (u.tx_hash.clone(), u.tx_index))
        .collect();
    assert!(references::complete_utxos(&refs, inputs.clone()).is_ok());
    assert!(references::complete_utxos(&refs, inputs[1..].to_vec()).is_err());
    let mut duplicate = inputs.clone();
    duplicate.push(inputs[0].clone());
    assert!(references::complete_utxos(&refs, duplicate).is_err());
    let mut total = 0;
    for u in &inputs {
        if let Some(script) = &u.reference_script {
            let (tag, bytes) = references::script_bytes(u).unwrap();
            assert_eq!(
                pallas_crypto::hash::Hasher::<224>::hash_tagged(&bytes, tag).to_string(),
                script["hash"].as_str().unwrap()
            );
            total += bytes.len() as u64;
        }
    }
    assert_eq!(references::total_script_bytes(&inputs).unwrap(), total);
    assert_eq!(
        references::total_script_bytes(&[inputs.clone(), inputs.clone()].concat()).unwrap(),
        total
    );
    let mut malformed = inputs[0].clone();
    malformed.reference_script.as_mut().unwrap()["bytes"] = "zz".into();
    assert!(references::total_script_bytes(&[malformed]).is_err());
}

#[test]
fn historical_withdrawal_exposes_upstream_surplus_reference_check() {
    let bytes = original();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let params = params();
    let keys: Vec<_> = tx
        .transaction_body
        .inputs
        .iter()
        .chain(
            tx.transaction_body
                .reference_inputs
                .iter()
                .flat_map(|r| r.iter()),
        )
        .chain(tx.transaction_body.collateral.iter().flat_map(|r| r.iter()))
        .map(|i| (i.transaction_id.to_string(), i.index))
        .collect();
    let inputs: Vec<_> = inputs()
        .into_iter()
        .filter(|u| keys.contains(&(u.tx_hash.clone(), u.tx_index)))
        .collect();
    let error = preflight::evaluate(
        &bytes,
        &inputs,
        params.evaluation_limits().unwrap(),
        &params.v3_cost_model().unwrap(),
        1_591_566_291,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("Unexpected eb06997a94b339ee0b0dd0de7bfec2a184d1af577586654d44e90558"),
        "{error}"
    );
}

#[test]
fn real_withdrawal_without_treasury_reference_passes_preflight() {
    let bytes = original();
    let mut tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let mut body = (*tx.transaction_body).clone();
    body.reference_inputs = Some(
        body.reference_inputs
            .as_ref()
            .unwrap()
            .iter()
            .filter(|r| !r.transaction_id.to_string().starts_with("944e78"))
            .cloned()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap(),
    );
    tx.transaction_body = body.into();
    let bytes = minicbor::to_vec(&tx).unwrap();
    let us: Vec<_> = inputs()
        .into_iter()
        .filter(|u| !u.tx_hash.starts_with("944e78"))
        .collect();
    let p = params();
    let report = preflight::evaluate(
        &bytes,
        &us,
        p.evaluation_limits().unwrap(),
        &p.v3_cost_model().unwrap(),
        1_591_566_291,
    )
    .unwrap();
    assert_eq!(report.redeemers, 2);

    // Keep value conserved but send the payout to the fee payer. The real
    // allowlist validator must reject it after the surplus reference is removed.
    let mut body = (*tx.transaction_body).clone();
    let fee_address = pallas_addresses::Address::from_bech32(
        &us.iter()
            .find(|u| u.tx_hash.starts_with("70c6"))
            .unwrap()
            .address,
    )
    .unwrap()
    .to_vec();
    let pallas_primitives::conway::TransactionOutput::PostAlonzo(output) = &mut body.outputs[0]
    else {
        panic!("expected post-Alonzo payout");
    };
    output.address = fee_address.into();
    tx.transaction_body = body.into();
    let invalid = minicbor::to_vec(&tx).unwrap();
    assert!(preflight::evaluate(
        &invalid,
        &us,
        p.evaluation_limits().unwrap(),
        &p.v3_cost_model().unwrap(),
        1_591_566_291
    )
    .is_err());
}

#[test]
fn withdrawal_builder_includes_only_required_references() {
    use minos::tx::withdraw::{build, WithdrawParams};
    let us = inputs();
    let get = |prefix: &str| us.iter().find(|u| u.tx_hash.starts_with(prefix)).unwrap();
    let vendor = get("1804");
    let fee = get("70c6");
    let built = build(WithdrawParams {
        vendor_utxo_hash: vendor.tx_hash.clone(),
        vendor_utxo_index: 0,
        vendor_address: vendor.address.clone(),
        matured_lovelace: 2_000_000,
        matured_assets: vec![],
        remaining_datum_bytes: Some(hex::decode(vendor.datum_hex().unwrap()).unwrap()),
        remaining_lovelace: vendor.lovelace(),
        remaining_assets: vec![],
        registry_utxo_hash: get("fe2a").tx_hash.clone(),
        registry_utxo_index: 0,
        vendor_script_ref_hash: get("c307").tx_hash.clone(),
        vendor_script_ref_index: 0,
        vendor_key_hashes: vec![vec![7; 28]],
        fee_inputs: vec![fee.into()],
        collateral: get("ff9d").into(),
        fee_addr: fee.address.clone(),
        fee: 1_000_000,
        change_lovelace: 17_000_000,
        dest_addr: fee.address.clone(),
        valid_from_slot: 1,
        invalid_from_slot: 2,
        v3_cost_model: params().v3_cost_model().unwrap(),
        instance_id: "51486a2f1496d4d3a688a9b111971aa9b731ed045d900b601345ca4e".into(),
        network_id: 1,
        staking_script_hashes: vec![hex::decode(
            "326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb",
        )
        .unwrap()
        .try_into()
        .unwrap()],
        staking_script_refs: vec![(get("aaec").tx_hash.clone(), 0)],
        staking_reward_mem: 2_500_000,
        staking_reward_steps: 900_000_000,
    })
    .unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let actual: std::collections::BTreeSet<_> = tx
        .transaction_body
        .reference_inputs
        .as_ref()
        .unwrap()
        .iter()
        .map(|r| (r.transaction_id.to_string(), r.index))
        .collect();
    let expected = ["fe2a", "c307", "aaec"]
        .into_iter()
        .map(|prefix| (get(prefix).tx_hash.clone(), 0))
        .collect();
    assert_eq!(actual, expected);
    let metadata = serde_json::to_value(&tx.auxiliary_data).unwrap();
    let fields = metadata["Shelley"]["1694"]["Map"].as_array().unwrap();
    let author = fields.iter().find(|p| p[0]["Text"] == "txAuthor").unwrap();
    assert_eq!(author[1]["Text"], "07".repeat(28));
    assert!(tx
        .transaction_body
        .required_signers
        .as_ref()
        .unwrap()
        .iter()
        .any(|s| s.as_ref() == [7; 28]));
    assert!(minos::tx::metadata::encode_withdraw("instance", "").is_err());
    assert!(minos::tx::metadata::encode_withdraw("instance", &"07".repeat(27)).is_err());
}

fn synthetic(script_source: &str, mem: u64) -> (Vec<u8>, Vec<Utxo>) {
    use pallas_txbuilder::{BuildConway, ExUnits, Input, Output, ScriptKind, StagingTransaction};
    let script = uplc::parser::program(script_source)
        .unwrap()
        .to_debruijn()
        .unwrap()
        .to_cbor()
        .unwrap();
    let hash = pallas_crypto::hash::Hasher::<224>::hash_tagged(&script, 3);
    let mut script_address = vec![0x71];
    script_address.extend_from_slice(hash.as_ref());
    let address = pallas_addresses::Address::from_bytes(&script_address)
        .unwrap()
        .to_bech32()
        .unwrap();
    let dest = "addr1wystdkxqmsrhyxkzj90jaqhy0zq2m2ehjvmwjvmamsnth7ga8mhgs";
    let spending = Input::new([1; 32].into(), 0);
    let params = params();
    let tx = StagingTransaction::new()
        .input(spending.clone())
        .reference_input(Input::new([2; 32].into(), 0))
        .output(Output::new(
            pallas_addresses::Address::from_bech32(dest).unwrap(),
            2_500_000,
        ))
        .fee(500_000)
        .valid_from_slot(200_000_000)
        .invalid_from_slot(200_000_100)
        .add_spend_redeemer(
            spending,
            hex::decode("d87980").unwrap(),
            Some(ExUnits {
                mem,
                steps: 900_000_000,
            }),
        )
        .add_language(ScriptKind::PlutusV3, params.v3_cost_model().unwrap())
        .build_conway_raw()
        .unwrap();
    let us = serde_json::from_value(serde_json::json!([
        {"tx_hash":hex::encode([1;32]), "tx_index":0, "address":address, "value":"3000000", "asset_list":[], "inline_datum":{"bytes":"d87980"}},
        {"tx_hash":hex::encode([2;32]), "tx_index":0, "address":dest, "value":"3000000", "asset_list":[], "reference_script":{"type":"plutusV3", "bytes":hex::encode(script)}}
    ])).unwrap();
    (tx.tx_bytes.0, us)
}

#[test]
fn unsigned_preflight_evaluates_scripts_and_enforces_declared_budgets() {
    let params = params();
    for (program, mem, expected) in [
        ("(program 1.1.0 (lam ctx (con unit ())))", 2_500_000, None),
        (
            "(program 1.1.0 (lam ctx (error)))",
            2_500_000,
            Some("script evaluation failed"),
        ),
        (
            "(program 1.1.0 (lam ctx (con unit ())))",
            1,
            Some("insufficient execution budget"),
        ),
        (
            "(program 1.1.0 (lam ctx (con integer 42)))",
            2_500_000,
            Some("returned failure"),
        ),
    ] {
        let (bytes, us) = synthetic(program, mem);
        let result = preflight::evaluate(
            &bytes,
            &us,
            params.evaluation_limits().unwrap(),
            &params.v3_cost_model().unwrap(),
            1_591_566_291,
        );
        if let Some(expected) = expected {
            assert!(result.unwrap_err().to_string().contains(expected));
        } else {
            let report = result.unwrap();
            assert_eq!(report.redeemers, 1);
            assert!(report.memory > 0 && report.steps > 0);
        }
    }
}
