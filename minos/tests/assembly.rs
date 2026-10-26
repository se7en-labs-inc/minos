use minos::tx::sign::{add_vkey_witness, assemble_witness, tx_body_hash};
use pallas_crypto::{hash::Hasher, key::ed25519::SecretKey};
use serde_json::json;

fn fixture() -> (Vec<u8>, String, [u8; 32], [u8; 28]) {
    let key = SecretKey::from([17; 32]);
    let signer = *Hasher::<224>::hash(key.public_key().as_ref());
    // Body includes required signer; preserve a Plutus V1 script and auxiliary metadata.
    let tx = hex::decode(format!(
        "84a40080018002000e81581c{}a10381420102f5a100a10000",
        hex::encode(signer)
    ))
    .unwrap();
    let hash = tx_body_hash(&tx).unwrap();
    let signature = key.sign(hash);
    let witness = format!(
        "825820{}5840{}",
        hex::encode(key.public_key()),
        hex::encode(signature)
    );
    let envelope = json!({"type": "TxWitness ConwayEra", "cborHex": witness}).to_string();
    (tx, envelope, hash, signer)
}

#[test]
fn assembly_preserves_original_parts_and_existing_vkeys() {
    let (tx, envelope, hash, signer) = fixture();
    let other = SecretKey::from([19; 32]);
    let tx = add_vkey_witness(
        &tx,
        &other.public_key().as_ref().try_into().unwrap(),
        &other.sign(hash).as_ref().try_into().unwrap(),
    )
    .unwrap();
    let assembled = assemble_witness(&tx, &envelope, &hash, &signer).unwrap();
    let original: pallas_primitives::conway::Tx<'_> = pallas_codec::minicbor::decode(&tx).unwrap();
    let result: pallas_primitives::conway::Tx<'_> =
        pallas_codec::minicbor::decode(&assembled).unwrap();
    assert_eq!(
        original.transaction_body.raw_cbor(),
        result.transaction_body.raw_cbor()
    );
    assert_eq!(original.auxiliary_data, result.auxiliary_data);
    assert_eq!(
        original.transaction_witness_set.plutus_v1_script,
        result.transaction_witness_set.plutus_v1_script
    );
    assert_eq!(
        result
            .transaction_witness_set
            .vkeywitness
            .as_ref()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        original
            .transaction_witness_set
            .vkeywitness
            .as_ref()
            .unwrap()[0],
        result.transaction_witness_set.vkeywitness.as_ref().unwrap()[0]
    );
    assert_eq!(tx_body_hash(&assembled).unwrap(), hash);
}

#[test]
fn rejects_wrong_body_signer_signature_and_malformed_envelopes() {
    let (tx, envelope, hash, signer) = fixture();
    assert!(assemble_witness(&tx, &envelope, &[0; 32], &signer).is_err());
    assert!(assemble_witness(&tx, &envelope, &hash, &[0; 28]).is_err());
    let mut value: serde_json::Value = serde_json::from_str(&envelope).unwrap();
    let original = value["cborHex"].as_str().unwrap().to_owned();
    for bad in [
        format!("{original}00"),
        original[..original.len() - 2].to_string(),
        format!("{}ff", &original[..original.len() - 2]),
        "9fff".to_string(),
    ] {
        value["cborHex"] = json!(bad);
        assert!(assemble_witness(&tx, &value.to_string(), &hash, &signer).is_err());
    }
    value["cborHex"] = json!(original);
    value["type"] = json!("Tx ConwayEra");
    assert!(assemble_witness(&tx, &value.to_string(), &hash, &signer).is_err());
    let mut trailing = tx;
    trailing.push(0);
    assert!(assemble_witness(&trailing, &envelope, &hash, &signer).is_err());
}
