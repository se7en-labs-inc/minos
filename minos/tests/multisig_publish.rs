use minos::{coin_select::WalletUtxo, scripts, tx::publish, types::MultisigScript};
use pallas_codec::minicbor;
use pallas_primitives::conway::Tx;
use pallas_traverse::MultiEraOutput;
use serde_json::{json, Value};

fn normalize(v: &Value) -> Value {
    if let Some(text) = v.get("Text") {
        return text.clone();
    }
    if let Some(int) = v.get("Int") {
        return int.clone();
    }
    if let Some(array) = v.get("Array") {
        return Value::Array(array.as_array().unwrap().iter().map(normalize).collect());
    }
    if let Some(map) = v.get("Map") {
        return Value::Object(
            map.as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        normalize(&p[0]).as_str().unwrap().to_string(),
                        normalize(&p[1]),
                    )
                })
                .collect(),
        );
    }
    v.clone()
}

#[test]
fn administrator_threshold_rejects_invalid_or_duplicate_keys() {
    let keys = vec![vec![1; 28], vec![2; 28], vec![3; 28]];
    assert!(publish::admin_permission(&[], 1).is_err());
    assert!(publish::admin_permission(&keys, 0).is_err());
    assert!(publish::admin_permission(&keys, 4).is_err());
    assert!(publish::admin_permission(&[vec![1; 28], vec![1; 28]], 1).is_err());
    assert!(publish::admin_permission(&[vec![1; 27]], 1).is_err());
    assert!(matches!(
        publish::admin_permission(&keys[..1], 1).unwrap(),
        MultisigScript::Signature { .. }
    ));
}

#[test]
fn published_scripts_and_metadata_agree_on_two_of_three_administrators() {
    let keys = vec![vec![1; 28], vec![2; 28], vec![3; 28]];
    let permission = publish::admin_permission(&keys, 2).unwrap();
    let seed = [5; 32];
    let registry = scripts::script_hash(&scripts::parameterize_oneshot(&seed, 0).unwrap());
    let expected_treasury = scripts::parameterize_treasury(
        &registry,
        &scripts::TreasuryPermissions {
            reorganize: permission.clone(),
            sweep: permission.clone(),
            fund: permission.clone(),
            disburse: permission.clone(),
        },
        10_000_000,
        9_000_000,
    )
    .unwrap();
    let mut raw_protocol: Value =
        serde_json::from_str(include_str!("fixtures/withdrawal/params.json")).unwrap();
    raw_protocol["coins_per_utxo_size"] = 4310.into();
    let protocol: minos::koios::ProtocolParams = serde_json::from_value(raw_protocol).unwrap();
    let params = publish::PublishParams {
        seed_utxo_hash: hex::encode(seed),
        seed_utxo_index: 0,
        seed_lovelace: 1_000_000_000,
        seed_assets: std::collections::BTreeMap::from([(format!("{}ff", "ab".repeat(28)), 7)]),
        admin_key_hash: keys[0].clone(),
        admin_keys: keys.clone(),
        admin_threshold: 2,
        expiration_ms: 10_000_000,
        payout_upperbound_ms: 9_000_000,
        vendor_expiration_ms: 10_000_000,
        label: "Multisig".into(),
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
        v3_cost_model: protocol.v3_cost_model().unwrap(),
        network_id: 0,
    };
    let atomic = publish::build(params.clone()).unwrap();
    let utxos: Vec<minos::koios::Utxo> = serde_json::from_value(json!([
        {"tx_hash":hex::encode(seed),"tx_index":0,"address":"addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh","value":"1000000000","asset_list":[{"policy_id":"ab".repeat(28),"asset_name":"ff","quantity":"7"}]},
        {"tx_hash":"06".repeat(32),"tx_index":0,"address":"addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh","value":"10000000","asset_list":[]}
    ])).unwrap();
    assert!(
        minos::preflight::check_protocol(&atomic.cbor_hex, &utxos, &protocol)
            .unwrap_err()
            .to_string()
            .contains("maximum size")
    );
    let bundle = publish::build_bundle(params.clone(), &protocol).unwrap();
    minos::preflight::check_protocol(&bundle.publish.cbor_hex, &utxos, &protocol).unwrap();
    let bytes = hex::decode(&bundle.publish.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    assert_eq!(tx.transaction_body.outputs.len(), 3);
    assert_eq!(
        bundle.manifest.publish_body_hash,
        hex::encode(minos::tx::sign::tx_body_hash(&bytes).unwrap())
    );
    assert_eq!(
        bundle.manifest.vendor_reference_depends_on.transaction,
        bundle.manifest.publish_body_hash
    );
    assert_eq!(bundle.manifest.vendor_reference_depends_on.index, 2);
    let funding = minos::preflight::utxo_from_output(
        &bundle.manifest.publish_body_hash,
        2,
        &tx.transaction_body.outputs[2],
    )
    .unwrap();
    minos::preflight::check_protocol(
        &bundle.vendor_reference.cbor_hex,
        std::slice::from_ref(&funding),
        &protocol,
    )
    .unwrap();
    assert!(
        minos::preflight::check_protocol(&bundle.vendor_reference.cbor_hex, &[], &protocol)
            .is_err()
    );
    let second_bytes = hex::decode(&bundle.vendor_reference.cbor_hex).unwrap();
    let second: Tx<'_> = minicbor::decode(&second_bytes).unwrap();
    assert_eq!(second.transaction_body.inputs.len(), 1);
    assert_eq!(
        second.transaction_body.inputs[0].transaction_id.to_string(),
        bundle.manifest.publish_body_hash
    );
    assert_eq!(second.transaction_body.inputs[0].index, 2);
    let returned = minos::preflight::utxo_from_output(
        &bundle.manifest.vendor_reference_body_hash,
        1,
        &second.transaction_body.outputs[1],
    )
    .unwrap();
    let wallet = WalletUtxo::try_from(&returned).unwrap();
    assert_eq!(wallet.assets, params.seed_assets);
    let reference = minos::preflight::utxo_from_output(
        &bundle.manifest.vendor_reference_body_hash,
        0,
        &second.transaction_body.outputs[0],
    )
    .unwrap();
    let (tag, script) = minos::references::script_bytes(&reference).unwrap();
    assert_eq!(
        pallas_crypto::hash::Hasher::<224>::hash_tagged(&script, tag).to_string(),
        bundle.manifest.vendor_script_hash
    );
    let first_size = minos::tx::fees::signed_size(&bytes, 2).unwrap();
    let second_size = minos::tx::fees::signed_size(&second_bytes, 1).unwrap();
    assert!(first_size <= protocol.max_tx_size);
    assert!(second_size <= protocol.max_tx_size);
    eprintln!("Deployment signed sizes: {first_size} and {second_size} bytes");
    let mut insufficient = params.clone();
    insufficient.seed_lovelace = 1;
    assert!(publish::build_bundle(insufficient, &protocol).is_err());
    let mut collateral = params;
    collateral.collateral.lovelace = 1;
    assert!(publish::build_bundle(collateral, &protocol).is_err());

    let report = minos::preflight::evaluate(
        &bytes,
        &utxos,
        protocol.evaluation_limits().unwrap(),
        &protocol.v3_cost_model().unwrap(),
        1_666_656_000,
    )
    .unwrap();
    assert_eq!(report.redeemers, 1);
    let treasury = MultiEraOutput::from_conway(&tx.transaction_body.outputs[1]);
    assert_eq!(
        treasury.address().unwrap().to_vec(),
        scripts::base_script_addr(&scripts::script_hash(&expected_treasury), 0).to_vec()
    );
    let aux = serde_json::to_value(&tx.auxiliary_data).unwrap();
    let metadata = normalize(&aux["Shelley"]["1694"]);
    for role in [
        "fund",
        "reorganize",
        "sweep",
        "disburse",
        "pause",
        "resume",
        "modify",
    ] {
        assert_eq!(
            metadata["body"]["permissions"][role],
            json!({
                "atLeast": {"required": 2, "scripts": keys.iter().map(|k|
                    json!({"signature":{"keyHash":hex::encode(k)}})).collect::<Vec<_>>()}
            })
        );
    }
}
