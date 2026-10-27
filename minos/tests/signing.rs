use minos::tx::sign::{add_vkey_witness, tx_body_hash};
use pallas_codec::minicbor::{Decoder, Encoder};
use pallas_crypto::hash::Hasher;

fn envelope(witnesses: &str) -> Vec<u8> {
    // Deliberately non-minimal fee encoding: signing must hash these exact bytes.
    hex::decode(format!(
        "84a300800180021b0000000000000000{witnesses}f4a100a10000"
    ))
    .unwrap()
}

fn parts(tx: &[u8]) -> Vec<&[u8]> {
    let mut decoder = Decoder::new(tx);
    assert_eq!(decoder.array().unwrap(), Some(4));
    (0..4)
        .map(|_| {
            let start = decoder.position();
            decoder.skip().unwrap();
            &tx[start..decoder.position()]
        })
        .collect()
}

fn fields(tx: &[u8]) -> Vec<(u64, Vec<u8>)> {
    let witness_map = parts(tx)[1];
    let mut decoder = Decoder::new(witness_map);
    (0..decoder.map().unwrap().unwrap())
        .map(|_| {
            let key = decoder.u64().unwrap();
            let start = decoder.position();
            decoder.skip().unwrap();
            (key, witness_map[start..decoder.position()].to_vec())
        })
        .collect()
}

#[test]
fn signing_preserves_raw_body_scripts_datums_redeemers_validity_and_auxiliary_data() {
    let bootstrap = format!(
        "81845820{}5840{}5820{}40",
        "11".repeat(32),
        "22".repeat(64),
        "33".repeat(32)
    );
    let tx = envelope(&format!(
        "a7018182018002{bootstrap}03814201020481d8798005a182000082d8798082010106814201020781420102"
    ));
    let original_parts = parts(&tx);
    let signed = add_vkey_witness(&tx, &[7; 32], &[8; 64]).unwrap();
    let signed_parts = parts(&signed);
    for index in [0, 2, 3] {
        assert_eq!(original_parts[index], signed_parts[index]);
    }
    assert_eq!(
        tx_body_hash(&signed).unwrap(),
        *Hasher::<256>::hash(original_parts[0])
    );
    assert_eq!(
        fields(&tx),
        fields(&signed)
            .into_iter()
            .filter(|(key, _)| *key != 0)
            .collect::<Vec<_>>()
    );
    let _: pallas_primitives::conway::Tx<'_> = pallas_codec::minicbor::decode(&signed).unwrap();
}

#[test]
fn repeated_signing_is_idempotent_and_tagged_partial_witnesses_survive() {
    let tx = add_vkey_witness(&envelope("a0"), &[7; 32], &[8; 64]).unwrap();
    let mut tx_parts = parts(&tx);
    let mut tagged = tx_parts.remove(1).to_vec();
    // Insert tag 258 before the existing witness array, after map(1), key 0.
    tagged.splice(2..2, [0xd9, 0x01, 0x02]);
    let mut rebuilt = vec![0x84];
    rebuilt.extend(tx_parts[0]);
    rebuilt.extend(&tagged);
    rebuilt.extend(tx_parts[1]);
    rebuilt.extend(tx_parts[2]);
    assert_eq!(
        add_vkey_witness(&rebuilt, &[7; 32], &[8; 64]).unwrap(),
        rebuilt
    );
    let added = add_vkey_witness(&rebuilt, &[9; 32], &[10; 64]).unwrap();
    let key0 = &fields(&added)[0].1;
    assert!(key0.starts_with(&[0xd9, 0x01, 0x02, 0x82]));
    let decoded: pallas_primitives::conway::Tx<'_> =
        pallas_codec::minicbor::decode(&added).unwrap();
    assert_eq!(
        decoded
            .transaction_witness_set
            .vkeywitness
            .as_ref()
            .unwrap()
            .len(),
        2
    );
    assert!(add_vkey_witness(&rebuilt, &[7; 32], &[0; 64]).is_err());
}

#[test]
fn unsupported_indefinite_duplicate_and_malformed_witnesses_fail_closed() {
    let witness = format!("825820{}5840{}", "07".repeat(32), "08".repeat(64));
    for map in [
        "bf05a182000082d87980820101ff".to_owned(), // historical witness-loss shape
        "a1009fff".to_owned(),                     // indefinite vkey set
        "a100d901029fff".to_owned(),               // tagged indefinite vkey set
        "a200800080".to_owned(),                   // duplicate witness field
        format!("a10082{witness}{witness}"),       // duplicate vkey
        "a10081824040".to_owned(),                 // wrong key/signature lengths
        "a10880".to_owned(),                       // unsupported witness key
        "a100d9010380".to_owned(),                 // unsupported tag
    ] {
        assert!(
            add_vkey_witness(&envelope(&map), &[9; 32], &[10; 64]).is_err(),
            "{map}"
        );
    }
    let valid = envelope("a0");
    let mut cases = vec![valid[..valid.len() - 1].to_vec()];
    for length in [0x83, 0x85, 0x9f] {
        let mut invalid = valid.clone();
        invalid[0] = length;
        cases.push(invalid);
    }
    let mut trailing = valid.clone();
    trailing.push(0);
    cases.push(trailing);
    for tx in cases {
        assert!(tx_body_hash(&tx).is_err());
        assert!(add_vkey_witness(&tx, &[9; 32], &[10; 64]).is_err());
    }
    assert!(add_vkey_witness(&hex::decode("8400a0f5f6").unwrap(), &[9; 32], &[10; 64]).is_err());
}

#[test]
fn public_withdrawal_fixture_keeps_existing_script_witness_bytes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/withdrawal/original.json")).unwrap();
    let tx = hex::decode(fixture["cborHex"].as_str().unwrap()).unwrap();
    let signed = add_vkey_witness(&tx, &[7; 32], &[8; 64]).unwrap();
    assert_eq!(tx_body_hash(&tx).unwrap(), tx_body_hash(&signed).unwrap());
    assert_eq!(
        fields(&tx)
            .into_iter()
            .filter(|(key, _)| *key != 0)
            .collect::<Vec<_>>(),
        fields(&signed)
            .into_iter()
            .filter(|(key, _)| *key != 0)
            .collect::<Vec<_>>()
    );
}

#[test]
fn rejected_cli_signing_never_creates_or_replaces_output() {
    let directory = std::env::temp_dir().join(format!("minos-signing-test-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let wallet = directory.join("synthetic-wallet");
    minos::wallet::save_mnemonic(
        &bip39::Mnemonic::from_entropy(&[0; 32]).unwrap(),
        &wallet,
        None,
    )
    .unwrap();
    let input = directory.join("input.json");
    let output = directory.join("output.json");
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .array(4)
        .unwrap()
        .map(0)
        .unwrap()
        .map(0)
        .unwrap()
        .bool(true)
        .unwrap()
        .null()
        .unwrap();
    std::fs::write(
        &input,
        serde_json::json!({"cborHex": hex::encode(encoder.into_writer())}).to_string(),
    )
    .unwrap();
    for existing in [false, true] {
        if existing {
            std::fs::write(&output, "preserve existing output").unwrap();
        }
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_minos"))
            .args(["wallet", "sign", "--tx"])
            .arg(&input)
            .arg("--wallet")
            .arg(&wallet)
            .arg("--out")
            .arg(&output)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        if existing {
            assert_eq!(
                std::fs::read_to_string(&output).unwrap(),
                "preserve existing output"
            );
        } else {
            assert!(!output.exists());
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
