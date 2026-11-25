use super::*;
use pallas_crypto::{hash::Hasher, key::ed25519::SecretKey};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn dir() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "minos-invoice-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    path
}
fn claims() -> Claims {
    Claims {
        invoice_number: "INV-42".into(),
        invoice_date: "2026-11-01".into(),
        due_date: "2026-11-30".into(),
        amount: "1250.75".into(),
        unit: "lovelace".into(),
        notes: "October maintenance".into(),
    }
}
fn fixture() -> (Package, SecretKey) {
    let key = SecretKey::from([29; 32]);
    let signer = hex::encode(Hasher::<224>::hash(key.public_key().as_ref()));
    let package = Package::upload(
        1,
        signer,
        "a".repeat(56),
        "TEST".into(),
        "m-8".into(),
        "invoice.pdf".into(),
        b"%PDF-1.7\npublic synthetic invoice".to_vec(),
        claims(),
        "00".repeat(16),
        Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    )
    .unwrap();
    (package, key)
}
fn signed(package: &Package, key: &SecretKey) -> SignedPackage {
    package
        .sign(|bytes| {
            Ok((
                key.public_key().as_ref().try_into().unwrap(),
                key.sign(bytes).as_ref().try_into().unwrap(),
            ))
        })
        .unwrap()
}

#[test]
fn payload_matches_browser_order_and_uses_decimal_amount_without_conversion() {
    let (mut package, _) = fixture();
    let Payload::Upload(mut p) = package.payload().unwrap() else {
        panic!()
    };
    p.timestamp = "2026-11-24T00:00:00.000Z".into();
    package.payload_hex = hex::encode(serde_json::to_vec(&Payload::Upload(p.clone())).unwrap());
    let payload = String::from_utf8(hex::decode(&package.payload_hex).unwrap()).unwrap();
    assert_eq!(
        payload,
        format!(
            r#"{{"action":"create_invoice_upload_ticket","instance":"{}","project":"TEST","milestone":"m-8","sha256":"{}","invoiceNumber":"INV-42","invoiceDate":"2026-11-01","dueDate":"2026-11-30","amount":"1250.75","unit":"lovelace","notes":"October maintenance","nonce":"{}","timestamp":"2026-11-24T00:00:00.000Z"}}"#,
            "a".repeat(56),
            p.sha256,
            "00".repeat(16)
        )
    );
    let unlock = Payload::Unlock(UnlockPayload {
        instance: "a".repeat(56),
        project: "TEST".into(),
        nonce: "00".repeat(16),
        timestamp: p.timestamp,
    });
    assert_eq!(
        serde_json::to_string(&unlock).unwrap(),
        format!(
            r#"{{"action":"unlock_invoice_session","instance":"{}","project":"TEST","nonce":"{}","timestamp":"2026-11-24T00:00:00.000Z"}}"#,
            "a".repeat(56),
            "00".repeat(16)
        )
    );
    package.validate().unwrap();
}

#[test]
fn signatures_bind_file_claims_filename_network_and_signer() {
    let (package, key) = fixture();
    let signed = signed(&package, &key);
    package.verify(&signed).unwrap();
    for mode in 0..7 {
        let mut changed = package.clone();
        match mode {
            0 => changed.file_hex = Some(hex::encode(b"%PDF-changed")),
            1 => changed.file_name = Some("renamed.pdf".into()),
            2 => changed.network = 0,
            3 => changed.signer = "11".repeat(28),
            _ => {
                let Payload::Upload(mut p) = changed.payload().unwrap() else {
                    panic!()
                };
                match mode {
                    4 => p.amount = "9999".into(),
                    5 => p.milestone = "m-9".into(),
                    _ => p.timestamp = Utc::now().to_rfc3339(),
                };
                changed.payload_hex = hex::encode(serde_json::to_vec(&Payload::Upload(p)).unwrap());
            }
        }
        // Even recomputing the unsigned response hash cannot forge the binding.
        let mut forged = signed.clone();
        forged.request_hash = changed.request_hash().unwrap();
        assert!(changed.verify(&forged).is_err(), "mode {mode}");
    }
    let other = SecretKey::from([30; 32]);
    assert!(package
        .sign(|b| Ok((
            other.public_key().as_ref().try_into().unwrap(),
            other.sign(b).as_ref().try_into().unwrap()
        )))
        .is_err());
    let mut corrupt = signed;
    corrupt.authorization.signature.replace_range(0..2, "ff");
    assert!(package.verify(&corrupt).is_err());
}

#[test]
fn offline_integrity_ignores_age_but_online_exchange_rejects_stale_future_and_network() {
    let (mut package, key) = fixture();
    let Payload::Upload(mut p) = package.payload().unwrap() else {
        panic!()
    };
    p.timestamp = "2000-01-01T00:00:00Z".into();
    package.payload_hex = hex::encode(serde_json::to_vec(&Payload::Upload(p)).unwrap());
    package.verify(&signed(&package, &key)).unwrap();
    assert!(package.fresh(1, Utc::now()).is_err());
    let (package, _) = fixture();
    assert!(package.fresh(0, Utc::now()).is_err());
    assert!(package
        .fresh(1, Utc::now() - chrono::Duration::minutes(1))
        .is_err());
    assert!(package
        .fresh(1, Utc::now() + chrono::Duration::minutes(3))
        .is_err());
    package.fresh(1, Utc::now()).unwrap();
}

#[test]
fn rejects_bad_claims_files_urls_and_oversized_packages() {
    for amount in ["", "1e6", "-1", "1.1234567", "1.", ".5", "1.2.3", "NaN"] {
        let mut c = claims();
        c.amount = amount.into();
        assert!(c.validate().is_err(), "{amount}");
    }
    for date in ["2026-02-30", "2026-1-01", "tomorrow"] {
        let mut c = claims();
        c.invoice_date = date.into();
        assert!(c.validate().is_err());
    }
    let mut c = claims();
    c.due_date = "2026-10-30".into();
    assert!(c.validate().is_err());
    c = claims();
    c.notes = "separator\u{2028}here".into();
    assert!(c.validate().is_err());
    assert!(content_type("a.pdf", b"not a pdf").is_err());
    assert!(content_type("bad\r\n.pdf", b"%PDF-").is_err());
    assert!(content_type("../a.pdf", b"%PDF-").is_err());
    assert!(content_type("a.pdf", &vec![b'0'; MAX_FILE_BYTES + 1]).is_err());
    assert_eq!(
        content_type("a.PNG", b"\x89PNG\r\n\x1a\nsynthetic").unwrap(),
        "image/png"
    );
    assert_eq!(
        content_type("a.jpeg", b"\xff\xd8\xffsynthetic").unwrap(),
        "image/jpeg"
    );
    for url in [
        "http://example.org",
        "https://user:password@example.org",
        "https://example.org/#secret",
        "not-url",
    ] {
        assert!(secure_url(url).is_err());
    }
}

#[test]
fn private_artifacts_are_exclusive_owner_only_and_multipart_is_exact() {
    let root = dir();
    let path = root.join("private.json");
    write_private(&path, &json!({"secret":"test"})).unwrap();
    assert!(write_private(&path, &json!({})).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = root.join("link.json");
        symlink(&path, &link).unwrap();
        assert!(write_private(&link, &json!({})).is_err());
        let directory = root.join("receipts");
        private_directory(&directory).unwrap();
        assert_eq!(
            std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let (package, _) = fixture();
    let client = private_client().unwrap();
    let request = multipart_request(
        &client.client,
        &package,
        "https://invoice.test/private?token=secret",
    )
    .unwrap();
    let body = request.body().unwrap().as_bytes().unwrap();
    assert!(body
        .windows(package.file_bytes().unwrap().len())
        .any(|b| b == package.file_bytes().unwrap()));
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("name=\"file\"; filename=\"invoice.pdf\""));
    assert!(text.contains("Content-Type: application/pdf\r\n"));
    assert!(!text.contains("pinataMetadata"));
    assert!(multipart_request(&client.client, &package, "http://invoice.test").is_err());
    std::fs::remove_dir_all(root).unwrap();
}

fn request_body(bytes: &[u8]) -> &[u8] {
    let start = bytes.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
    &bytes[start..]
}

async fn mock(
    count: usize,
    handler: impl Fn(usize, &[u8]) -> Value + Send + 'static,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        for step in 0..count {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut block = [0; 8192];
                let size = socket.read(&mut block).await.unwrap();
                assert!(size > 0);
                request.extend_from_slice(&block[..size]);
                if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    let length = header
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let response = handler(step, &request);
            let bytes = if let Some(raw) = response.get("raw").and_then(Value::as_str) {
                raw.as_bytes().to_vec()
            } else {
                serde_json::to_vec(&response).unwrap()
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n",bytes.len()).as_bytes()).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
        }
    });
    (address, task)
}

#[tokio::test]
async fn private_upload_confirms_and_recovers_without_another_upload() {
    for fail_confirm in [false, true] {
        let (package, key) = fixture();
        let signed = signed(&package, &key);
        let sha = sha256(&package.file_bytes().unwrap());
        let expected_key = signed.authorization.key.clone();
        let (address,task)=mock(if fail_confirm {5}else{4}, move |step,bytes| {
            if step==2 {
                assert!(String::from_utf8_lossy(bytes).contains("%PDF-1.7\npublic synthetic invoice"));
                return json!({"data":{"cid":"bafyOpaquePrivateStorage"}});
            }
            let request:Value=serde_json::from_slice(request_body(bytes)).unwrap();
            match step {
                0=>{assert!(request["query"].as_str().unwrap().contains("now"));json!({"data":{"now":{"unixMilli":Utc::now().timestamp_millis().to_string()}}})},
                1=>{
                    let input=&request["variables"]["input"];
                    assert_eq!(input["publicKeyHex"],expected_key);
                    assert_eq!(input["sha256"],sha);
                    assert_eq!(input["amount"],"1250.75");
                    assert_eq!(input["milestoneId"],"m-8");
                    assert!(!request["query"].as_str().unwrap().contains("Evidence"));
                    json!({"data":{"createInvoiceUploadTicket":{"invoiceId":"inv-42","uploadUrl":"https://invoice.test/upload?secret=capability","expiresAt":(Utc::now()+chrono::Duration::minutes(5)).to_rfc3339(),"confirmToken":"private-confirm-token"}}})
                },
                _=>{
                    assert!(request["query"].as_str().unwrap().contains("confirmInvoiceUpload"));
                    let input=&request["variables"]["input"];
                    assert_eq!(input["cid"],"bafyOpaquePrivateStorage");
                    assert_eq!(input["confirmToken"],"private-confirm-token");
                    if fail_confirm&&step==3 {json!({"errors":[{"message":"rejected private-confirm-token"}]})} else {json!({"data":{"confirmInvoiceUpload":{"ok":"Ok"}}})}
                }
            }
        }).await;
        let portal = Portal::new(&format!("http://{address}/graphql"));
        let mut client = private_client().unwrap();
        client.test_address = Some(address);
        let root = dir();
        let out = root.join("receipts");
        let result = upload_with_client(&portal, 1, &package, &signed, &out, &client).await;
        let receipt: Receipt = read_private(&out.join("uploaded.json")).unwrap();
        assert_eq!(receipt.cid, "bafyOpaquePrivateStorage");
        assert!(out.join("ticket.json").exists());
        if fail_confirm {
            assert!(result.is_err());
            assert!(!format!("{:#}", result.unwrap_err()).contains("private-confirm-token"));
            let unknown: Value = read_private(&out.join("confirmed.json")).unwrap();
            assert_eq!(unknown["stage"], "confirmation-outcome-unknown");
            confirm(&portal, 1, &receipt, &out.join("confirmed-retry.json"))
                .await
                .unwrap();
        } else {
            result.unwrap();
        }
        task.await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn tampered_and_wrong_action_packages_make_no_network_calls() {
    let portal = Portal::new("http://127.0.0.1:1");
    let (package, key) = fixture();
    let response = signed(&package, &key);
    let root = dir();
    let out = root.join("out");
    let mut changed = package.clone();
    changed.network = 0;
    assert!(upload(&portal, 0, &changed, &response, &out)
        .await
        .unwrap_err()
        .to_string()
        .contains("mismatch"));
    let unlock_package = Package::unlock(
        1,
        package.signer,
        UnlockPayload {
            instance: "a".repeat(56),
            project: "TEST".into(),
            nonce: "00".repeat(16),
            timestamp: Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
    let response = signed(&unlock_package, &key);
    assert!(upload(&portal, 1, &unlock_package, &response, &out)
        .await
        .unwrap_err()
        .to_string()
        .contains("upload package"));
    assert!(!out.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unlock_list_download_checks_scope_and_content_before_saving() {
    for corrupt in [false, true] {
        let (p, key) = fixture();
        let package = Package::unlock(
            1,
            p.signer,
            UnlockPayload {
                instance: "a".repeat(56),
                project: "TEST".into(),
                nonce: "00".repeat(16),
                timestamp: Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        let signed = signed(&package, &key);
        let expected = sha256(b"%PDF-invoice");
        let (address,task)=mock(6, move |step,bytes| {
            if step==5 {return json!({"raw":if corrupt {"%PDF-wrong"}else{"%PDF-invoice"}});}
            let request:Value=serde_json::from_slice(request_body(bytes)).unwrap();
            match step {
                0=>json!({"data":{"now":{"unixMilli":Utc::now().timestamp_millis().to_string()}}}),
                1=>{
                    assert_eq!(request["variables"]["input"]["projectId"],"TEST");
                    json!({"data":{"unlockInvoiceSession":{"token":"private-session-token","role":"Vendor","projectId":"TEST","expiresAt":(Utc::now()+chrono::Duration::minutes(5)).to_rfc3339()}}})
                },
                2|3=>{
                    assert_eq!(request["variables"]["token"],"private-session-token");
                    json!({"data":{"projectInvoices":{"instanceId":"a".repeat(56),"projectId":"TEST","viewerIsVendor":true,"invoices":[{"invoiceId":"inv-42","milestoneId":"m-8","status":"Available","sha256":expected,"sizeBytes":12}]}}})
                },
                4=>{
                    assert_eq!(request["variables"]["input"]["invoiceId"],"inv-42");
                    json!({"data":{"createInvoiceDownloadLink":{"url":"https://invoice.test/download?capability=secret","expiresAt":(Utc::now()+chrono::Duration::minutes(1)).to_rfc3339()}}})
                },
                _=>panic!(),
            }
        }).await;
        let portal = Portal::new(&format!("http://{address}"));
        let root = dir();
        let session_path = root.join("session.json");
        unlock(&portal, 1, &package, &signed, &session_path)
            .await
            .unwrap();
        let session: Session = read_private(&session_path).unwrap();
        assert!(session.validate(0, Utc::now()).is_err());
        assert!(session
            .validate(1, Utc::now() + chrono::Duration::days(1))
            .is_err());
        list(&portal, 1, &session).await.unwrap();
        let mut client = private_client().unwrap();
        client.test_address = Some(address);
        let file = root.join("invoice.pdf");
        let result =
            download_with_client(&portal, 1, &session, "m-8", "inv-42", &file, &client).await;
        if corrupt {
            assert!(result.is_err());
            assert!(std::fs::read(&file).unwrap().is_empty());
        } else {
            result.unwrap();
            assert_eq!(std::fs::read(&file).unwrap(), b"%PDF-invoice");
        }
        task.await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
