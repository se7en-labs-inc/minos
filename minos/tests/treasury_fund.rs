use minos::tx::fund::{self, FundInput, FundParams, MilestoneSpec, TreasurySpend};
use pallas_codec::minicbor;
use pallas_primitives::conway::Tx;
use pallas_traverse::MultiEraOutput;

fn fixture() -> FundParams {
    FundParams {
        treasury: None,
        instance_id: "11".repeat(28),
        vendor_script_hash: vec![2; 28],
        admin_key_hash: vec![3; 28],
        vendor_key_hash: None,
        co_signer_hashes: vec![],
        project_id: "test".into(),
        label: "Test".into(),
        description: "Description".into(),
        vendor_label: "Vendor".into(),
        milestones: vec![MilestoneSpec {
            id: "m-0".into(),
            label: "Initial".into(),
            description: None,
            acceptance_criteria: Some("Signed contract".into()),
            requires_evidence: false,
            maturation_ms: 1_000_000,
            lovelace: 1_189_560,
            token_amount: 100,
        }],
        token_policy: Some(vec![4; 28]),
        token_name: Some(b"USDM7".to_vec()),
        inputs: vec![FundInput {
            tx_hash: "55".repeat(32),
            output_index: 0,
            lovelace: 1_000_000_000,
            token_amount: 100,
        }],
        vendor_lovelace_total: 10_000_000,
        change_lovelace: 988_000_000,
        change_tokens: 0,
        extra_change_assets: vec![(vec![4; 28], b"REF".to_vec(), 1)],
        fee_addr: "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh".into(),
        fee: 2_000_000,
        valid_from_slot: 100,
        invalid_from_slot: 200,
        network_id: 0,
    }
}

#[test]
fn distinct_vendor_and_two_admins_are_required_and_preserved_when_signing() {
    let mut params = fixture();
    let mnemonic = bip39::Mnemonic::from_entropy(&[0; 16]).unwrap();
    let keys: Vec<_> = (0..4)
        .map(|a| minos::wallet::payment_key_hash(&mnemonic, a).to_vec())
        .collect();
    params.admin_key_hash = keys[0].clone();
    params.co_signer_hashes = vec![keys[1].clone()];
    params.vendor_key_hash = Some(keys[3].clone());
    let built = fund::build(params).unwrap();
    let mut bytes = hex::decode(built.cbor_hex).unwrap();
    let hash = minos::tx::sign::tx_body_hash(&bytes).unwrap();
    for account in [0, 1, 3] {
        let (key, signature) = minos::wallet::sign_tx_body(&mnemonic, account, &hash);
        bytes = minos::tx::sign::sign_tx(&bytes, &key, &signature)
            .unwrap()
            .0;
        assert_eq!(minos::tx::sign::tx_body_hash(&bytes).unwrap(), hash);
    }
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    let signers = tx.transaction_body.required_signers.as_ref().unwrap();
    assert_eq!(signers.len(), 3);
    for account in [0, 1, 3] {
        assert!(signers
            .iter()
            .any(|s| s.as_ref() == keys[account].as_slice()));
    }
    assert!(!signers.iter().any(|s| s.as_ref() == keys[2].as_slice()));
    assert_eq!(
        tx.transaction_witness_set
            .vkeywitness
            .as_ref()
            .unwrap()
            .len(),
        3
    );
    let output = MultiEraOutput::from_conway(&tx.transaction_body.outputs[0]);
    let pallas_primitives::conway::DatumOption::Data(datum) = output.datum().unwrap() else {
        panic!("expected inline datum")
    };
    let datum_bytes = minicbor::to_vec(&datum.0).unwrap();
    let vendor = minos::types::VendorDatum::from_cbor_hex(&hex::encode(datum_bytes)).unwrap();
    assert_eq!(vendor.vendor.key_hashes(), vec![keys[3].clone()]);
}

#[test]
fn deposit_has_unit_datum_no_project_metadata_and_preserves_other_assets() {
    let built = fund::build_deposit(fixture(), &[9; 28]).unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    assert_eq!(tx.transaction_body.outputs.len(), 2);
    assert!(tx.transaction_witness_set.redeemer.is_none());
    assert!(tx.transaction_body.auxiliary_data_hash.is_some());
    let output = MultiEraOutput::from_conway(&tx.transaction_body.outputs[0]);
    assert_eq!(output.value().coin(), 10_000_000);
    assert_eq!(
        hex::encode(output.address().unwrap().to_vec()),
        format!("30{}{}", "09".repeat(28), "09".repeat(28))
    );
    let json = serde_json::to_value(&tx.transaction_body.outputs).unwrap();
    assert!(json[0].to_string().contains(&hex::encode(b"USDM7")));
    assert!(!json[0].to_string().contains(&hex::encode(b"REF")));
    assert!(json[1].to_string().contains(&hex::encode(b"REF")));
    assert_eq!(
        tx.transaction_body
            .outputs
            .iter()
            .map(|o| MultiEraOutput::from_conway(o).value().coin())
            .sum::<u64>()
            + tx.transaction_body.fee,
        1_000_000_000
    );
}

#[test]
fn existing_deposit_can_be_recreated_with_initialization_metadata_and_reorganize_redeemer() {
    let mut params = fixture();
    params.token_policy = None;
    params.token_name = None;
    params.milestones[0].token_amount = 0;
    params.extra_change_assets.clear();
    params.treasury = Some(TreasurySpend {
        input: ("55".repeat(32), 0),
        references: vec![("66".repeat(32), 0), ("77".repeat(32), 1)],
        collateral: minos::coin_select::WalletUtxo {
            tx_hash: "88".repeat(32),
            output_index: 0,
            lovelace: 10_000_000,
            has_native_assets: false,
        },
        cost_model: vec![0; 297],
        return_address: params.fee_addr.clone(),
        return_lovelace: 0,
        return_assets: vec![],
    });
    let built = fund::build_deposit(params, &[9; 28]).unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    assert_eq!(
        tx.transaction_body.reference_inputs.as_ref().unwrap().len(),
        2
    );
    assert!(tx.transaction_body.auxiliary_data_hash.is_some());
    assert!(tx.transaction_witness_set.redeemer.is_some());
    assert_eq!(tx.transaction_body.outputs.len(), 2);
    assert_eq!(
        MultiEraOutput::from_conway(&tx.transaction_body.outputs[0])
            .value()
            .coin(),
        10_000_000
    );
    assert!(format!("{:?}", tx.transaction_witness_set.redeemer).contains("121"));
}

#[test]
fn funding_spends_treasury_references_registry_and_returns_treasury_surplus() {
    let mut params = fixture();
    params.extra_change_assets.clear();
    params.change_lovelace -= 8_810_440;
    params.treasury = Some(TreasurySpend {
        input: ("55".repeat(32), 0),
        references: vec![("66".repeat(32), 0), ("77".repeat(32), 1)],
        collateral: minos::coin_select::WalletUtxo {
            tx_hash: "88".repeat(32),
            output_index: 0,
            lovelace: 10_000_000,
            has_native_assets: false,
        },
        cost_model: vec![0; 297],
        return_address: params.fee_addr.clone(),
        return_lovelace: 8_810_440,
        return_assets: vec![],
    });
    let built = fund::build(params).unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    assert_eq!(
        tx.transaction_body.reference_inputs.as_ref().unwrap().len(),
        2
    );
    assert_eq!(tx.transaction_body.collateral.as_ref().unwrap().len(), 1);
    assert!(tx.transaction_witness_set.redeemer.is_some());
    assert_eq!(tx.transaction_body.outputs.len(), 3);
    assert_eq!(
        MultiEraOutput::from_conway(&tx.transaction_body.outputs[2])
            .value()
            .coin(),
        8_810_440
    );
    assert_eq!(
        tx.transaction_body
            .outputs
            .iter()
            .map(|o| MultiEraOutput::from_conway(o).value().coin())
            .sum::<u64>()
            + tx.transaction_body.fee,
        1_000_000_000
    );
}

#[test]
fn ada_selection_excludes_scripts_assets_and_spent_output_but_allows_sibling_collateral() {
    use serde_json::json;
    let row = |index, value, assets, script| {
        serde_json::from_value::<minos::koios::Utxo>(json!({
            "tx_hash":"ab".repeat(32),"tx_index":index,"address":"address","value":value,
            "asset_list":assets,"reference_script":script
        }))
        .unwrap()
    };
    let utxos = vec![
        row(0, "1000000000", json!([]), json!({"bytes":"00"})),
        row(
            1,
            "1000000000",
            json!([{"policy_id":"04".repeat(28),"asset_name":"00","quantity":"1"}]),
            json!(null),
        ),
        row(2, "8810440", json!([]), json!(null)),
        row(3, "950000000", json!([]), json!(null)),
    ];
    assert_eq!(
        fund::select_ada_treasury_input(&utxos, 950_000_000)
            .unwrap()
            .tx_index,
        3
    );
    assert!(fund::select_ada_treasury_input(&utxos[..3], 950_000_000).is_err());
    let inputs = vec![FundInput {
        tx_hash: "ab".repeat(32),
        output_index: 0,
        lovelace: 950_000_000,
        token_amount: 0,
    }];
    let wallet = vec![
        minos::coin_select::WalletUtxo {
            tx_hash: "ab".repeat(32),
            output_index: 0,
            lovelace: 950_000_000,
            has_native_assets: false,
        },
        minos::coin_select::WalletUtxo {
            tx_hash: "ab".repeat(32),
            output_index: 1,
            lovelace: 10_000_000,
            has_native_assets: false,
        },
    ];
    assert_eq!(
        fund::select_funding_collateral(&wallet, &inputs)
            .unwrap()
            .output_index,
        1
    );
    assert!(fund::select_funding_collateral(&wallet[..1], &inputs).is_err());
}

#[test]
fn nineteen_fifty_ada_payouts_create_no_token_assets_or_treasury_surplus() {
    let mut params = fixture();
    params.token_policy = None;
    params.token_name = None;
    params.extra_change_assets.clear();
    params.inputs[0].token_amount = 0;
    params.vendor_lovelace_total = 950_000_000;
    params.change_lovelace = 48_000_000;
    params.milestones = (0..19)
        .map(|i| MilestoneSpec {
            id: format!("m-{i}"),
            label: format!("Milestone {i}"),
            description: None,
            acceptance_criteria: None,
            requires_evidence: i != 0,
            maturation_ms: 1_000_000 + i * 900_000,
            lovelace: 50_000_000,
            token_amount: 0,
        })
        .collect();
    params.treasury = Some(TreasurySpend {
        input: ("55".repeat(32), 0),
        references: vec![("66".repeat(32), 0), ("77".repeat(32), 1)],
        collateral: minos::coin_select::WalletUtxo {
            tx_hash: "88".repeat(32),
            output_index: 0,
            lovelace: 10_000_000,
            has_native_assets: false,
        },
        cost_model: vec![0; 297],
        return_address: params.fee_addr.clone(),
        return_lovelace: 0,
        return_assets: vec![],
    });
    let built = fund::build(params).unwrap();
    let bytes = hex::decode(built.cbor_hex).unwrap();
    let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
    assert!(tx.transaction_body.mint.is_none());
    assert_eq!(tx.transaction_body.outputs.len(), 2);
    assert_eq!(
        MultiEraOutput::from_conway(&tx.transaction_body.outputs[0])
            .value()
            .coin(),
        950_000_000
    );
    let json = serde_json::to_value(&tx.transaction_body.outputs).unwrap();
    assert!(json
        .as_array()
        .unwrap()
        .iter()
        .all(|o| !o.to_string().contains("Multiasset")));
    assert_eq!(
        tx.transaction_body
            .outputs
            .iter()
            .map(|o| MultiEraOutput::from_conway(o).value().coin())
            .sum::<u64>()
            + tx.transaction_body.fee,
        1_000_000_000
    );
}
