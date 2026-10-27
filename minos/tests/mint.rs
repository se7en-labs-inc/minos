use minos::{
    koios::{ProtocolParams, Utxo},
    preflight,
    tx::mint,
};
use serde_json::json;
const ADDRESS: &str = "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh";
fn fixture() -> (mint::MintParams, Vec<Utxo>, ProtocolParams) {
    let mut protocol: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/withdrawal/params.json")).unwrap();
    protocol["coins_per_utxo_size"] = 4310.into();
    let utxos=serde_json::from_value(json!([{"tx_hash":"01".repeat(32),"tx_index":0,"address":ADDRESS,"value":"10000000","asset_list":[]}])).unwrap();
    (
        mint::MintParams {
            admin_key_hash: vec![2; 28],
            ft_amount: 100,
            mint_reference: true,
            token_ticker: "TEST".into(),
            token_decimals: 6,
            inputs: vec![mint::MintInput {
                tx_hash: "01".repeat(32),
                output_index: 0,
                lovelace: 10_000_000,
            }],
            dest_addr: ADDRESS.into(),
            fee_addr: ADDRESS.into(),
            fee: 200_000,
            coins_per_utxo_byte: 4310,
            valid_from_slot: 100,
            invalid_from_slot: 200,
            network_id: 0,
        },
        utxos,
        serde_json::from_value(protocol).unwrap(),
    )
}
#[test]
fn native_mint_conserves_value_and_checks_policy_fee_and_output_minimums() {
    let (params, utxos, protocol) = fixture();
    let built = mint::build(params.clone()).unwrap();
    preflight::check_protocol(&built.unsigned_tx.cbor_hex, &utxos, &protocol).unwrap();
    let evaluate = |built: &mint::MintResult| {
        preflight::evaluate(
            &hex::decode(&built.unsigned_tx.cbor_hex).unwrap(),
            &utxos,
            protocol.evaluation_limits().unwrap(),
            &protocol.v3_cost_model().unwrap(),
            1_666_656_000,
        )
    };
    assert_eq!(evaluate(&built).unwrap().redeemers, 0);
    let mut missing = utxos.clone();
    missing[0].value = "9999999".into();
    assert!(preflight::evaluate(
        &hex::decode(&built.unsigned_tx.cbor_hex).unwrap(),
        &missing,
        protocol.evaluation_limits().unwrap(),
        &protocol.v3_cost_model().unwrap(),
        1_666_656_000
    )
    .unwrap_err()
    .to_string()
    .contains("not conserved"));
    let mut low = params.clone();
    low.fee = 1;
    assert!(preflight::check_protocol(
        &mint::build(low).unwrap().unsigned_tx.cbor_hex,
        &utxos,
        &protocol
    )
    .unwrap_err()
    .to_string()
    .contains("fee is below"));
    let mut oversized = params;
    oversized.ft_amount = u64::MAX;
    assert!(mint::build(oversized).is_err());
}

#[test]
fn mint_minimum_changes_with_protocol_and_reference_datum_and_rebalances_change() {
    let (mut params, utxos, protocol) = fixture();
    let original = mint::minimum_output_coin(&params).unwrap();
    params.coins_per_utxo_byte = 20_000;
    let larger = mint::minimum_output_coin(&params).unwrap();
    assert!(larger > 3_000_000 && larger > original);
    let built = mint::build(params.clone()).unwrap();
    let bytes = hex::decode(&built.unsigned_tx.cbor_hex).unwrap();
    let tx: pallas_primitives::conway::Tx<'_> = pallas_codec::minicbor::decode(&bytes).unwrap();
    assert_eq!(
        pallas_traverse::MultiEraOutput::from_conway(&tx.transaction_body.outputs[0])
            .value()
            .coin(),
        larger
    );
    preflight::evaluate(
        &bytes,
        &utxos,
        protocol.evaluation_limits().unwrap(),
        &protocol.v3_cost_model().unwrap(),
        1_666_656_000,
    )
    .unwrap();
    params.mint_reference = false;
    assert!(mint::minimum_output_coin(&params).unwrap() < larger);
    params.inputs[0].lovelace = 1;
    assert!(mint::build(params).is_err());
}

#[test]
fn protocol_preflight_rejects_output_and_resolved_input_network_mismatches() {
    let (mut params, mut utxos, protocol) = fixture();
    params.network_id = 1;
    let built = mint::build(params.clone()).unwrap();
    assert!(
        preflight::check_protocol(&built.unsigned_tx.cbor_hex, &utxos, &protocol)
            .unwrap_err()
            .to_string()
            .contains("transaction network")
    );
    params.network_id = 0;
    let built = mint::build(params).unwrap();
    let mut raw = pallas_addresses::Address::from_bech32(ADDRESS)
        .unwrap()
        .to_vec();
    raw[0] |= 1;
    utxos[0].address = pallas_addresses::Address::from_bytes(&raw)
        .unwrap()
        .to_bech32()
        .unwrap();
    assert!(
        preflight::check_protocol(&built.unsigned_tx.cbor_hex, &utxos, &protocol)
            .unwrap_err()
            .to_string()
            .contains("transaction network")
    );
}

#[test]
fn protocol_preflight_rejects_an_oversized_native_asset_value() {
    let (params, utxos, protocol) = fixture();
    let built = mint::build(params).unwrap();
    assert!(preflight::check_protocol(
        &built.unsigned_tx.cbor_hex,
        &utxos,
        &protocol.with_max_value_size(10)
    )
    .unwrap_err()
    .to_string()
    .contains("value exceeds"));
}
