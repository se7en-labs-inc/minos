use minos::{
    airgap::{self, Package},
    cose,
    tx::{metadata, sign},
};
use pallas_codec::minicbor::{Decoder, Encoder};
use pallas_crypto::{
    hash::Hasher,
    key::ed25519::{PublicKey, SecretKey, Signature},
};
use serde_json::json;

fn fixture() -> (Package, SecretKey, i64) {
    let key = SecretKey::from([7; 32]); // Public deterministic test key only.
    let signer = hex::encode(Hasher::<224>::hash(key.public_key().as_ref()));
    let now = chrono::Utc::now().timestamp();
    (package_for_signer(&signer, now), key, now)
}

fn package_for_signer(signer: &str, now: i64) -> Package {
    let file = b"%PDF-1.7\npublic test fixture\n%%EOF".to_vec();
    package_with_file(signer, now, file, false)
}

fn package_with_file(signer: &str, now: i64, file: Vec<u8>, unixfs: bool) -> Package {
    let url = format!(
        "ipfs://{}",
        if unixfs {
            minos::evidence::unixfs_cid(&file).unwrap()
        } else {
            minos::evidence::raw_cid(file.as_slice()).unwrap()
        }
    );
    let (metadata, aux_hash) = metadata::encode_complete(
        &"a".repeat(56),
        signer,
        "TEST",
        "m-7",
        "Evidence",
        &[url],
        &[],
    )
    .unwrap();
    let slot = (now - 1_591_566_291) as u64;
    let mut e = Encoder::new(Vec::new());
    e.array(4)
        .unwrap()
        .map(8)
        .unwrap()
        .u8(0)
        .unwrap()
        .array(0)
        .unwrap()
        .u8(1)
        .unwrap()
        .array(0)
        .unwrap()
        .u8(2)
        .unwrap()
        .u64(1_000_000)
        .unwrap()
        .u8(3)
        .unwrap()
        .u64(slot + 7200)
        .unwrap()
        .u8(7)
        .unwrap()
        .bytes(&aux_hash)
        .unwrap()
        .u8(8)
        .unwrap()
        .u64(slot - 120)
        .unwrap()
        .u8(14)
        .unwrap()
        .array(1)
        .unwrap()
        .bytes(&hex::decode(signer).unwrap())
        .unwrap()
        .u8(15)
        .unwrap()
        .u8(1)
        .unwrap()
        .map(0)
        .unwrap()
        .bool(true)
        .unwrap();
    let mut bytes = e.into_writer();
    bytes.extend(metadata);
    Package::prepare(
        hex::encode(bytes),
        "signed-maf.pdf".into(),
        file,
        1,
        "00".repeat(16),
        chrono::DateTime::from_timestamp(now, 0)
            .unwrap()
            .to_rfc3339(),
    )
    .unwrap()
}

#[test]
fn unixfs_package_preserves_both_signatures_and_rejects_changed_bytes_or_cid() {
    let key = SecretKey::from([7; 32]);
    let signer = hex::encode(Hasher::<224>::hash(key.public_key().as_ref()));
    let now = chrono::Utc::now().timestamp();
    let mut file = b"%PDF-1.7\npublic multichunk fixture\n".to_vec();
    file.resize(407_316, 42);
    let package = package_with_file(&signer, now, file, true);
    let signed = sign(&package, &key, now);
    package.verify(&signed, now).unwrap();
    assert_eq!(
        package.review().unwrap()["evidenceImportProfile"],
        "unixfs-cidv1-256k-raw-leaves"
    );
    let receipt = airgap::upload_receipt(
        &package,
        &json!({"cid": package.evidence_url.strip_prefix("ipfs://").unwrap()}),
    )
    .unwrap();
    assert_eq!(receipt["cidMatches"], true);
    let mut changed = package.clone();
    let mut bytes = hex::decode(&changed.file_hex).unwrap();
    bytes[300_000] ^= 1;
    changed.file_hex = hex::encode(&bytes);
    changed.file_sha256 = airgap::sha256(&bytes);
    assert!(changed.validate(now).is_err());
    assert!(changed.verify(&signed, now).is_err());
    let mut changed = package;
    changed.evidence_url = format!(
        "ipfs://{}",
        minos::evidence::raw_cid(&hex::decode(&changed.file_hex).unwrap()[..]).unwrap()
    );
    assert!(changed.validate(now).is_err());
}

fn sign(p: &Package, key: &SecretKey, now: i64) -> airgap::SignedPackage {
    p.sign(now, |bytes| {
        Ok((
            key.public_key().as_ref().try_into().unwrap(),
            key.sign(bytes).as_ref().try_into().unwrap(),
        ))
    })
    .unwrap()
}

#[test]
fn offline_signatures_ignore_clock_drift_but_online_validation_rejects_it() {
    let (package, key, now) = fixture();
    let hash = package.request_hash().unwrap();
    package.validate_offline().unwrap();
    let signed = package
        .sign_offline(|message| {
            Ok((
                key.public_key().as_ref().try_into().unwrap(),
                key.sign(message).as_ref().try_into().unwrap(),
            ))
        })
        .unwrap();
    package.verify_offline(&signed).unwrap();
    package.verify(&signed, now).unwrap();
    for drift in [-86_400, 86_400] {
        assert!(package.validate(now + drift).is_err());
        assert!(package.verify(&signed, now + drift).is_err());
        package.validate_offline().unwrap();
        package.verify_offline(&signed).unwrap();
    }
    assert_eq!(package.request_hash().unwrap(), hash);
    let review = package.review().unwrap();
    assert!(review["validFromUtc"].is_string());
    assert!(review["expiresAtUtc"].is_string());
    assert!(review.get("uploadAuthorizationAgeSeconds").is_none());

    for offset in [-3600, 8000] {
        let mut invalid = package.clone();
        let mut payload = invalid.payload().unwrap();
        payload.timestamp = chrono::DateTime::from_timestamp(now + offset, 0)
            .unwrap()
            .to_rfc3339();
        invalid.upload_payload_hex = hex::encode(serde_json::to_vec(&payload).unwrap());
        assert!(invalid.validate_offline().is_err());
    }

    // A structurally invalid validity window remains invalid even offline.
    let mut raw = hex::decode(&package.tx_cbor_hex).unwrap();
    let (lower, upper) = {
        let tx = minos::metadata::transaction(&raw).unwrap();
        (
            tx.transaction_body.validity_interval_start.unwrap(),
            tx.transaction_body.ttl.unwrap(),
        )
    };
    let mut encoded = Encoder::new(Vec::new());
    encoded.u64(upper).unwrap();
    let encoded = encoded.into_writer();
    let positions: Vec<_> = raw
        .windows(encoded.len())
        .enumerate()
        .filter(|(_, value)| *value == encoded)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(positions.len(), 1);
    let mut replacement = Encoder::new(Vec::new());
    replacement.u64(lower).unwrap();
    let replacement = replacement.into_writer();
    assert_eq!(replacement.len(), encoded.len());
    raw[positions[0]..positions[0] + encoded.len()].copy_from_slice(&replacement);
    let mut invalid = package;
    invalid.body_hash = hex::encode(sign::tx_body_hash(&raw).unwrap());
    invalid.tx_cbor_hex = hex::encode(raw);
    assert!(invalid
        .validate_offline()
        .unwrap_err()
        .to_string()
        .contains("validity window"));
}

#[test]
fn single_run_returns_valid_tx_witness_and_independently_verified_cose() {
    let (package, key, now) = fixture();
    let signed = sign(&package, &key, now);
    let tx = package.verify(&signed, now).unwrap();
    assert_eq!(
        hex::encode(sign::tx_body_hash(&tx).unwrap()),
        package.body_hash
    );
    // Decode COSE independently of the production verifier and construct RFC Sig_structure.
    let bytes = hex::decode(&signed.upload_signature.signature).unwrap();
    let mut d = Decoder::new(&bytes);
    assert_eq!(d.array().unwrap(), Some(4));
    let protected = d.bytes().unwrap();
    let mut h = Decoder::new(protected);
    assert_eq!(h.map().unwrap(), Some(2));
    assert_eq!((h.i8().unwrap(), h.i8().unwrap()), (1, -8));
    assert_eq!(h.str().unwrap(), "address");
    assert_eq!(h.bytes().unwrap(), package.address().unwrap());
    assert_eq!(d.map().unwrap(), Some(1));
    assert_eq!(d.str().unwrap(), "hashed");
    assert!(!d.bool().unwrap());
    let payload = d.bytes().unwrap();
    assert_eq!(payload, hex::decode(&package.upload_payload_hex).unwrap());
    let sig: [u8; 64] = d.bytes().unwrap().try_into().unwrap();
    let mut e = Encoder::new(Vec::new());
    e.array(4)
        .unwrap()
        .str("Signature1")
        .unwrap()
        .bytes(protected)
        .unwrap()
        .bytes(&[])
        .unwrap()
        .bytes(payload)
        .unwrap();
    assert!(key
        .public_key()
        .verify(e.into_writer(), &Signature::from(sig)));
    let bytes = hex::decode(&signed.upload_signature.key).unwrap();
    let mut d = Decoder::new(&bytes);
    assert_eq!(d.map().unwrap(), Some(4));
    assert_eq!((d.i8().unwrap(), d.i8().unwrap()), (1, 1));
    assert_eq!((d.i8().unwrap(), d.i8().unwrap()), (3, -8));
    assert_eq!((d.i8().unwrap(), d.i8().unwrap()), (-1, 6));
    assert_eq!(d.i8().unwrap(), -2);
    assert_eq!(d.bytes().unwrap(), key.public_key().as_ref());
    assert_eq!(signed.request_hash, package.request_hash().unwrap());
    let input = package.ticket_input(&signed).unwrap();
    assert_eq!(input["payloadHex"], package.upload_payload_hex);
    assert_eq!(input["publicKeyHex"], signed.upload_signature.key);
    assert_eq!(
        input["files"][0]["sizeBytes"],
        hex::decode(&package.file_hex).unwrap().len().to_string()
    );
}

#[test]
fn rejects_tampered_packages_signatures_context_network_and_expiry() {
    let (p, key, now) = fixture();
    let signed = sign(&p, &key, now);
    let mut altered = p.clone();
    altered.file_hex.push_str("00");
    assert!(altered.verify(&signed, now).is_err());
    let mut altered = p.clone();
    altered.body_hash = "00".repeat(32);
    assert!(altered.verify(&signed, now).is_err());
    let mut altered = p.clone();
    altered.network = 0;
    assert!(altered.verify(&signed, now).is_err());
    for field in ["instance", "project", "milestone", "action", "timestamp"] {
        let mut altered = p.clone();
        let mut payload: serde_json::Value =
            serde_json::from_slice(&hex::decode(&p.upload_payload_hex).unwrap()).unwrap();
        payload[field] = json!("wrong");
        altered.upload_payload_hex = hex::encode(serde_json::to_vec(&payload).unwrap());
        assert!(altered.validate(now).is_err(), "{field}");
    }
    assert!(p.validate(now + 8000).is_err());
    let other = SecretKey::from([8; 32]);
    assert!(p
        .sign(now, |m| Ok((
            other.public_key().as_ref().try_into().unwrap(),
            other.sign(m).as_ref().try_into().unwrap()
        )))
        .is_err());
    let mut bad = signed.clone();
    bad.request_hash = "00".repeat(32);
    assert!(p.verify(&bad, now).is_err());
    let mut bad = signed.clone();
    bad.upload_signature.signature.push_str("00");
    assert!(p.verify(&bad, now).is_err());
    let mut bad = signed;
    bad.transaction_witness["cborHex"] = json!("00");
    assert!(p.verify(&bad, now).is_err());
}

#[test]
fn cose_rejects_wrong_address_payload_and_signatures() {
    let key = SecretKey::from([9; 32]);
    let vkey: [u8; 32] = key.public_key().as_ref().try_into().unwrap();
    let mut address = vec![0x61];
    address.extend_from_slice(Hasher::<224>::hash(&vkey).as_ref());
    let payload = b"abc";
    let sig = key.sign(cose::signing_bytes(payload, &address).unwrap());
    let value = cose::encode(payload, &address, &vkey, &sig.as_ref().try_into().unwrap()).unwrap();
    assert_eq!(cose::verify(&value, payload, &address).unwrap(), vkey);
    // COSE_Key CBOR prefix fixed by CIP-30, independently specified here.
    assert!(value.key.starts_with("a4010103272006215820"));
    assert!(cose::verify(&value, b"abd", &address).is_err());
    let mut wrong_address = address.clone();
    wrong_address[2] ^= 1;
    assert!(cose::verify(&value, payload, &wrong_address).is_err());
    let mut wrong = value.clone();
    wrong.key.push_str("00");
    assert!(cose::verify(&wrong, payload, &address).is_err());
    let wrong = cose::encode(payload, &address, &vkey, &[0; 64]).unwrap();
    assert!(cose::verify(&wrong, payload, &address).is_err());
    let public = PublicKey::from(vkey);
    assert!(public.verify(
        cose::signing_bytes(payload, &address).unwrap(),
        &Signature::from(*sig.as_ref().first_chunk::<64>().unwrap())
    ));
}

#[test]
fn upload_request_preserves_pdf_and_cid_mismatch_never_passes() {
    let (p, _, _) = fixture();
    let client = reqwest::Client::new();
    assert!(airgap::upload_request(&client, &p, "http://upload.invalid").is_err());
    let request =
        airgap::upload_request(&client, &p, "https://upload.invalid/path?token=test").unwrap();
    assert_eq!(request.method(), reqwest::Method::POST);
    let body = request.body().unwrap().as_bytes().unwrap();
    let pdf = hex::decode(&p.file_hex).unwrap();
    assert!(body.windows(pdf.len()).any(|w| w == pdf));
    assert!(String::from_utf8_lossy(body).contains("name=\"file\"; filename=\"signed-maf.pdf\""));
    assert!(
        airgap::upload_receipt(&p, &json!({"data":{"cid":"different"}})).unwrap()["cidMatches"]
            == false
    );
    assert!(
        airgap::upload_receipt(
            &p,
            &json!({"cid":p.evidence_url.strip_prefix("ipfs://").unwrap()})
        )
        .unwrap()["cidMatches"]
            == true
    );
    assert!(airgap::upload_receipt(&p, &json!({})).is_err());
}

#[test]
fn offline_cli_signs_both_with_gpg_encrypted_test_mnemonic() {
    use std::process::Command;
    let now = chrono::Utc::now().timestamp();
    let mnemonic = bip39::Mnemonic::from_entropy(&[0; 32]).unwrap(); // Public test mnemonic.
    let signer = hex::encode(minos::wallet::payment_key_hash(&mnemonic, 0).unwrap());
    let package = package_for_signer(&signer, now);
    let dir = std::env::temp_dir().join(format!("minos-airgap-test-{}-{now}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let request = dir.join("request.json");
    let wallet = dir.join("test-wallet.gpg");
    let signed = dir.join("signed.json");
    airgap::write_new(&request, &package).unwrap();
    // Encrypt only the public test mnemonic; isolate GPG state from real wallets.
    let gnupg = dir.join("gnupg");
    std::fs::create_dir(&gnupg).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gnupg, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    use std::io::Write;
    let generate = Command::new("gpg")
        .env("GNUPGHOME", &gnupg)
        .args([
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-generate-key",
            "Minos Airgap Test <airgap-test@example.invalid>",
            "future-default",
            "default",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        generate.status.success(),
        "{}",
        String::from_utf8_lossy(&generate.stderr)
    );
    let mut child = Command::new("gpg")
        .env("GNUPGHOME", &gnupg)
        .args([
            "--batch",
            "--trust-model",
            "always",
            "--encrypt",
            "--recipient",
            "airgap-test@example.invalid",
            "--output",
        ])
        .arg(&wallet)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(mnemonic.to_string().as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let binary = env!("CARGO_BIN_EXE_minos");
    let rejected_output = dir.join("wrong-hash-signed.json");
    let mismatch = Command::new(binary)
        .args(["airgap", "sign", "--package"])
        .arg(&request)
        .arg("--wallet")
        .arg(dir.join("absent-wallet.asc"))
        .arg("--expected-request-hash")
        .arg("00".repeat(32))
        .arg("--out")
        .arg(&rejected_output)
        .output()
        .unwrap();
    assert!(!mismatch.status.success());
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("reviewed request hash mismatch"));
    assert!(!rejected_output.exists());
    let review = Command::new(binary)
        .args(["airgap", "review", "--package"])
        .arg(&request)
        .output()
        .unwrap();
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stderr)
    );
    let run = Command::new(binary)
        .env("GNUPGHOME", &gnupg)
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .args(["airgap", "sign", "--package"])
        .arg(&request)
        .arg("--wallet")
        .arg(&wallet)
        .arg("--out")
        .arg(&signed)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let result: airgap::SignedPackage =
        serde_json::from_slice(&std::fs::read(&signed).unwrap()).unwrap();
    package.verify(&result, now).unwrap();
    // Independently verify the message signature with OpenSSL, not pallas/cryptoxide.
    let cose = hex::decode(result.upload_signature.signature).unwrap();
    let mut d = Decoder::new(&cose);
    d.array().unwrap();
    d.skip().unwrap();
    d.skip().unwrap();
    d.skip().unwrap();
    let sig = d.bytes().unwrap();
    let sig_file = dir.join("sig.bin");
    std::fs::write(&sig_file, sig).unwrap();
    let message = dir.join("message.bin");
    std::fs::write(
        &message,
        cose::signing_bytes(
            &hex::decode(&package.upload_payload_hex).unwrap(),
            &package.address().unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let key_bytes = hex::decode(result.upload_signature.key).unwrap();
    let mut d = Decoder::new(&key_bytes);
    d.map().unwrap();
    for _ in 0..3 {
        d.skip().unwrap();
        d.skip().unwrap();
    }
    d.skip().unwrap();
    let mut der = hex::decode("302a300506032b6570032100").unwrap();
    der.extend_from_slice(d.bytes().unwrap());
    let public = dir.join("public.der");
    std::fs::write(&public, der).unwrap();
    let verify = Command::new("openssl")
        .args(["pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey"])
        .arg(public)
        .args(["-rawin", "-in"])
        .arg(message)
        .arg("-sigfile")
        .arg(sig_file)
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    // Neither command may overwrite returned signatures.
    let repeat = Command::new(binary)
        .env("GNUPGHOME", &gnupg)
        .args(["airgap", "sign", "--package"])
        .arg(&request)
        .arg("--wallet")
        .arg(&wallet)
        .args([
            "--expected-request-hash",
            &package.request_hash().unwrap(),
            "--out",
        ])
        .arg(&signed)
        .output()
        .unwrap();
    assert!(!repeat.status.success());
    let _ = Command::new("gpgconf")
        .env("GNUPGHOME", &gnupg)
        .args(["--kill", "gpg-agent"])
        .status();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn portal_time_and_ticket_match_web_client_contract() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (package, key, now) = fixture();
    let signed = sign(&package, &key, now);
    let expected = package.ticket_input(&signed).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let portal = minos::portal::Portal::new(&format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        for turn in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
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
            let response = if turn == 0 {
                assert!(request["query"].as_str().unwrap().contains("now { unixMilli }"));
                json!({"data":{"now":{"unixMilli":"1793031557210"}}})
            } else {
                assert!(request["query"].as_str().unwrap().contains("createEvidenceUploadTicket"));
                assert_eq!(request["variables"]["input"],expected);
                json!({"data":{"createEvidenceUploadTicket":{"uploads":[{"uploadUrl":"https://uploads.example.invalid/signed"}]}}})
            }.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
        }
    });
    assert_eq!(
        portal.server_timestamp().await.unwrap(),
        "2026-10-26T16:19:17.210Z"
    );
    assert_eq!(
        portal
            .evidence_upload_ticket(package.ticket_input(&signed).unwrap())
            .await
            .unwrap(),
        "https://uploads.example.invalid/signed"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn invalid_signed_package_is_rejected_before_contacting_portal() {
    let (package, key, now) = fixture();
    let mut signed = sign(&package, &key, now);
    signed.upload_signature.signature = "00".into();
    let portal = minos::portal::Portal::new("http://127.0.0.1:1");
    let err = airgap::upload_package(
        &portal,
        &package,
        &signed,
        &std::env::temp_dir().join("unused-minos-receipt.json"),
        &std::env::temp_dir().join("unused-minos-tx.json"),
    )
    .await
    .unwrap_err();
    assert!(!err.to_string().contains("portal request failed"));
}
