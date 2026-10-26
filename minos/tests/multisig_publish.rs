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
    let built = publish::build(publish::PublishParams {
        seed_utxo_hash: hex::encode(seed),
        seed_utxo_index: 0,
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
        },
        fee_addr: "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh".into(),
        fee: 3_000_000,
        change_lovelace: 934_000_000,
        valid_from_slot: 100,
        invalid_from_slot: 200,
        v3_cost_model: vec![0; 297],
        network_id: 0,
    })
    .unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
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
