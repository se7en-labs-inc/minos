use minos::tx::mint::{build, MintInput, MintParams};
use pallas_codec::minicbor;
use pallas_primitives::conway::Tx;

#[test]
fn additional_issuance_keeps_policy_and_does_not_duplicate_reference_nft() {
    let mut policy = None;
    for mint_reference in [true, false] {
        let result = build(MintParams {
            admin_key_hash: vec![0x33; 28],
            ft_amount: 400_000_000_000,
            mint_reference,
            token_ticker: "USDM7".into(),
            token_decimals: 6,
            inputs: vec![MintInput {
                tx_hash: "ab".repeat(32),
                output_index: 0,
                lovelace: 20_000_000,
            }],
            dest_addr: "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh".into(),
            fee_addr: "addr_test1vqe4ud7jajte2glw6mmhe7shmyhtewx0tmf3akesheyy3pcxhpteh".into(),
            fee: 1_000_000,
            coins_per_utxo_byte: 4310,
            valid_from_slot: 100,
            invalid_from_slot: 200,
            network_id: 0,
        })
        .unwrap();
        if let Some(previous) = &policy {
            assert_eq!(previous, &result.policy_id_hex);
        }
        policy = Some(result.policy_id_hex);
        let bytes = hex::decode(result.unsigned_tx.cbor_hex).unwrap();
        let tx: Tx<'_> = minicbor::decode(&bytes).unwrap();
        let mint = tx.transaction_body.mint.as_ref().unwrap();
        assert_eq!(mint.len(), 1);
        let assets = &mint.iter().next().unwrap().1;
        assert_eq!(assets.len(), if mint_reference { 2 } else { 1 });
        let mut total_ft = 0;
        let mut total_ref = 0;
        for (name, quantity) in assets.iter() {
            match hex::encode(name.as_slice()).as_str() {
                "0014df105553444d37" => total_ft += i64::from(*quantity),
                "000643b05553444d37" => total_ref += i64::from(*quantity),
                other => panic!("unexpected minted asset {other}"),
            }
        }
        assert_eq!(total_ft, 400_000_000_000);
        assert_eq!(total_ref, if mint_reference { 1 } else { 0 });
    }
}
