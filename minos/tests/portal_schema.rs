use minos::portal::Portal;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn mock(replies: Vec<(bool, Value)>) -> (Portal, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let portal = Portal::new(&format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        for (legacy, response) in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let request: Value = loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                    }
                }
            };
            let query = request["query"].as_str().unwrap();
            if !query.contains("__type") {
                assert_eq!(!query.contains("apiVersion"), legacy);
                assert_eq!(!query.contains("registryReference"), legacy);
                assert_eq!(!query.contains("confirmed"), legacy);
            }
            let response = response.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        }
    });
    (portal, server)
}

fn schema(legacy: bool) -> Value {
    let mut fields = vec![json!({"name":"instances"}), json!({"name":"instanceById"})];
    if !legacy {
        fields.push(json!({"name":"apiVersion"}));
    }
    json!({"data":{"__type":{"fields":fields}}})
}

fn instance() -> Value {
    json!({"identifier":"example", "label":"Example", "description":"", "expiration":{"unixMilli":"2000"},
        "payoutUpperbound":{"unixMilli":"2000"}, "vendorExpiration":{"unixMilli":"2000"},
        "registry":{"treasuryScript":{"scriptHash":"aa"},"vendorScript":{"scriptHash":"bb"}}, "projects":[]})
}

#[tokio::test]
async fn legacy_reads_omit_new_fields_and_never_fabricate_confirmation() {
    let response = json!({"data":{"instanceById":instance()}});
    let (portal, server) = mock(vec![
        (true, schema(true)),
        (true, response.clone()),
        (true, response),
    ])
    .await;
    assert_eq!(
        portal.instance_by_id("example").await.unwrap().confirmed,
        None
    );
    assert_eq!(
        portal.instance_by_id("example").await.unwrap().confirmed,
        None
    );
    server.await.unwrap(); // Only one schema discovery despite two instance reads.
}

#[tokio::test]
async fn v2_still_requires_confirmation_and_supported_version() {
    for (version, confirmed, succeeds) in [
        (2, Some(true), true),
        (2, Some(false), false),
        (2, None, false),
        (3, Some(true), false),
    ] {
        let mut value = instance();
        if let Some(confirmed) = confirmed {
            value["confirmed"] = json!(confirmed);
        }
        let (portal, server) = mock(vec![
            (false, schema(false)),
            (
                false,
                json!({"data":{"apiVersion":version,"instanceById":value}}),
            ),
        ])
        .await;
        assert_eq!(portal.instance_by_id("example").await.is_ok(), succeeds);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn schema_errors_are_not_silently_downgraded() {
    for response in [
        json!({"errors":[{"message":"unavailable"}]}),
        json!({"data":{"__type":{"fields":[]}}}),
    ] {
        let (portal, server) = mock(vec![(false, response)]).await;
        assert!(portal.instance_by_id("example").await.is_err());
        server.await.unwrap();
    }
}
