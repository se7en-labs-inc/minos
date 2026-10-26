use minos::{
    koios::Utxo,
    portal::{sponsored_utxos, FeeInput, Portal},
    tx::sign,
};
use pallas_crypto::key::ed25519::SecretKey;
use serde_json::json;

fn witnessed_tx() -> String {
    let tx = hex::decode("84a3008001800200a0f5f6").unwrap();
    let key = SecretKey::from([7; 32]); // Public deterministic test key.
    let signature = key.sign(sign::tx_body_hash(&tx).unwrap());
    hex::encode(
        sign::add_vkey_witness(
            &tx,
            &key.public_key().as_ref().try_into().unwrap(),
            &signature.as_ref().try_into().unwrap(),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn explicit_portal_submission_preserves_witnessed_cbor() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for acknowledgement in [json!(true), json!("Ok")] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let portal = Portal::new(&format!("http://{}", listener.local_addr().unwrap()));
        let cbor = witnessed_tx();
        let expected = cbor.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 2048];
            let body = loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(p) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..p]).to_lowercase();
                    let len: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= p + 4 + len {
                        break bytes[p + 4..p + 4 + len].to_vec();
                    }
                }
            };
            let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert!(request["query"]
                .as_str()
                .unwrap()
                .contains("newTransaction"));
            assert!(!request["query"]
                .as_str()
                .unwrap()
                .contains("registerTransaction"));
            assert_eq!(request["variables"]["tx"]["cborHex"], expected);
            assert_eq!(request["variables"]["tx"]["label"], "Complete M2.1");
            let response = json!({"data":{"newTransaction":{"ok":acknowledgement}}}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        });
        portal
            .submit_transaction("Complete M2.1", &cbor)
            .await
            .unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn reject_missing_or_invalid_witness_before_contacting_portal() {
    let portal = Portal::new("http://127.0.0.1:1");
    let err = portal
        .submit_transaction("test", "84a3008001800200a0f5f6")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no vkey witnesses"));
    let invalid = witnessed_tx().replacen("a3008001800200", "a3008001800201", 1);
    let err = portal
        .submit_transaction("test", &invalid)
        .await
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("invalid witness over transaction body"));
}

#[test]
fn sponsor_pool_rejects_reserved_spent_foreign_and_asset_bearing_outputs() {
    let now = "2026-10-26T12:00:00Z".parse().unwrap();
    let rows: Vec<FeeInput> = serde_json::from_value(json!([
        {"transaction":"live", "index":0, "address":"sponsor", "reservedUntil":""},
        {"transaction":"reserved", "index":0, "address":"sponsor", "reservedUntil":"2026-10-26T13:00:00Z"},
        {"transaction":"spent", "index":0, "address":"sponsor", "reservedUntil":null},
        {"transaction":"assets", "index":0, "address":"sponsor", "reservedUntil":null}
    ])).unwrap();
    let output = |hash: &str, address: &str, assets: serde_json::Value| -> Utxo {
        serde_json::from_value(json!({"tx_hash":hash,"tx_index":0,"address":address,
            "value":"9000000","asset_list":assets}))
        .unwrap()
    };
    let live = vec![
        output("live", "sponsor", json!([])),
        output("reserved", "sponsor", json!([])),
        output(
            "assets",
            "sponsor",
            json!([{"policy_id":"ab", "asset_name":"cd", "quantity":"1"}]),
        ),
        output("other", "sponsor", json!([])),
        output("live", "foreign", json!([])),
    ];
    let selected = sponsored_utxos(&rows, live, "sponsor", now).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].tx_hash, "live");
    assert_eq!(selected[0].lovelace(), 9_000_000);
}

#[test]
fn fee_reservations_accept_portal_empty_values_and_reject_malformed_values() {
    let now = "2026-10-26T12:00:00Z".parse().unwrap();
    for (value, expected) in [
        (None, true),
        (Some(""), true),
        (Some("2026-10-25T12:00:00Z"), true),
        (Some("2026-10-27T12:00:00Z"), false),
    ] {
        let row = FeeInput {
            transaction: "test".into(),
            index: 0,
            address: "sponsor".into(),
            reserved_until: value.map(str::to_owned),
        };
        assert_eq!(row.available_at(now).unwrap(), expected);
    }
    let row = FeeInput {
        transaction: "test".into(),
        index: 0,
        address: "sponsor".into(),
        reserved_until: Some("invalid".into()),
    };
    assert!(row.available_at(now).is_err());
}
