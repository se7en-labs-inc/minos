//! Self-contained request for the two vendor signatures in a completion workflow.
use crate::{cose, evidence, tx::sign};
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use cryptoxide::{digest::Digest, sha2::Sha256};
use pallas_codec::minicbor::{data::Type, Decoder};
use pallas_crypto::hash::Hasher;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadFile {
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: String,
}

/// Field order deliberately matches the web client's JSON.stringify payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadPayload {
    pub action: String,
    pub instance: String,
    pub project: String,
    pub milestone: String,
    pub files: Vec<UploadFile>,
    pub nonce: String,
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Package {
    pub version: u8,
    pub network: u8,
    pub signer: String,
    pub tx_cbor_hex: String,
    pub body_hash: String,
    pub file_hex: String,
    pub file_sha256: String,
    pub evidence_url: String,
    pub upload_payload_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedPackage {
    pub version: u8,
    pub request_hash: String,
    pub transaction_witness: Value,
    pub upload_signature: cose::DataSignature,
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.input(bytes);
    let mut out = [0; 32];
    h.result(&mut out);
    hex::encode(out)
}

fn metadata_value(d: &mut Decoder<'_>, depth: usize) -> Result<Value> {
    ensure!(depth < 20, "metadata nesting too deep");
    Ok(match d.datatype()? {
        Type::String => json!(d.str()?),
        Type::Array => {
            let n = d
                .array()?
                .context("indefinite metadata array unsupported")?;
            ensure!(n <= 10000, "metadata array too large");
            let mut a = Vec::new();
            for _ in 0..n {
                a.push(metadata_value(d, depth + 1)?);
            }
            Value::Array(a)
        }
        Type::Map => {
            let n = d.map()?.context("indefinite metadata map unsupported")?;
            ensure!(n <= 10000, "metadata map too large");
            let mut map = serde_json::Map::new();
            for _ in 0..n {
                let key = d.str()?.to_owned();
                ensure!(
                    map.insert(key, metadata_value(d, depth + 1)?).is_none(),
                    "duplicate metadata key"
                );
            }
            Value::Object(map)
        }
        _ => anyhow::bail!("unsupported completion metadata type"),
    })
}

fn text(value: &Value) -> Result<String> {
    if let Some(s) = value.as_str() {
        return Ok(s.to_owned());
    }
    let parts = value.as_array().context("expected metadata text/chunks")?;
    parts
        .iter()
        .map(|v| v.as_str().context("invalid text chunk"))
        .collect::<Result<Vec<_>>>()
        .map(|v| v.concat())
}

fn transaction(bytes: &[u8]) -> Result<(pallas_primitives::conway::Tx<'_>, Value)> {
    let mut d = Decoder::new(bytes);
    let tx: pallas_primitives::conway::Tx<'_> = d.decode()?;
    ensure!(d.position() == bytes.len(), "trailing transaction bytes");
    let mut d = Decoder::new(bytes);
    ensure!(d.array()? == Some(4), "expected Conway transaction");
    for _ in 0..3 {
        d.skip()?;
    }
    let start = d.position();
    d.skip()?;
    ensure!(
        tx.transaction_body.auxiliary_data_hash
            == Some(Hasher::<256>::hash(&bytes[start..d.position()])),
        "metadata hash does not match transaction body"
    );
    let mut aux = Decoder::new(&bytes[start..]);
    ensure!(
        aux.map()? == Some(1) && aux.u64()? == 1694,
        "expected completion metadata at 1694"
    );
    let metadata = metadata_value(&mut aux, 0)?;
    ensure!(
        aux.position() == bytes.len() - start,
        "trailing metadata bytes"
    );
    Ok((tx, metadata))
}

impl Package {
    pub fn prepare(
        tx_hex: String,
        file_name: String,
        bytes: Vec<u8>,
        network: u8,
        nonce: String,
        timestamp: String,
    ) -> Result<Self> {
        let raw = hex::decode(&tx_hex)?;
        let (_, meta) = transaction(&raw)?;
        let milestones = meta["body"]["milestones"]
            .as_object()
            .context("missing completion milestones")?;
        ensure!(milestones.len() == 1, "package supports one milestone");
        let milestone = milestones.keys().next().unwrap().clone();
        let payload = UploadPayload {
            action: "create_evidence_upload_ticket".into(),
            instance: text(&meta["instance"])?,
            project: text(&meta["body"]["identifier"])?,
            milestone,
            files: vec![UploadFile {
                file_name,
                content_type: "application/pdf".into(),
                size_bytes: bytes.len().to_string(),
            }],
            nonce,
            timestamp,
        };
        let package = Self {
            version: 1,
            network,
            signer: text(&meta["txAuthor"])?,
            body_hash: hex::encode(sign::tx_body_hash(&raw)?),
            file_sha256: sha256(&bytes),
            evidence_url: format!("ipfs://{}", evidence::raw_cid(bytes.as_slice())?),
            file_hex: hex::encode(bytes),
            tx_cbor_hex: tx_hex,
            upload_payload_hex: hex::encode(serde_json::to_vec(&payload)?),
        };
        package.validate(Utc::now().timestamp())?;
        Ok(package)
    }

    pub fn request_hash(&self) -> Result<String> {
        Ok(hex::encode(Hasher::<256>::hash(&serde_json::to_vec(self)?)))
    }

    pub fn address(&self) -> Result<Vec<u8>> {
        let signer = hex::decode(&self.signer)?;
        ensure!(
            signer.len() == 28 && self.network <= 1,
            "invalid signer/network"
        );
        let mut address = vec![0x60 | self.network];
        address.extend_from_slice(&signer);
        Ok(address)
    }

    pub fn payload(&self) -> Result<UploadPayload> {
        Ok(serde_json::from_slice(&hex::decode(
            &self.upload_payload_hex,
        )?)?)
    }

    pub fn validate(&self, now: i64) -> Result<()> {
        ensure!(
            self.version == 1 && self.network <= 1,
            "unsupported package version/network"
        );
        let raw = hex::decode(&self.tx_cbor_hex)?;
        let (tx, meta) = transaction(&raw)?;
        ensure!(tx.success, "transaction marked invalid");
        ensure!(
            hex::encode(sign::tx_body_hash(&raw)?) == self.body_hash,
            "body hash mismatch"
        );
        let address = self.address()?;
        ensure!(
            text(&meta["txAuthor"])? == self.signer,
            "author/signer mismatch"
        );
        ensure!(
            tx.transaction_body
                .required_signers
                .as_ref()
                .is_some_and(|s| s.iter().any(|h| h.as_ref() == &address[1..])),
            "vendor signer not required by body"
        );
        let tx_network = tx
            .transaction_body
            .network_id
            .context("transaction network missing")?;
        ensure!(
            matches!(
                (self.network, tx_network),
                (0, pallas_primitives::conway::NetworkId::Testnet)
                    | (1, pallas_primitives::conway::NetworkId::Mainnet)
            ),
            "transaction/package network mismatch"
        );
        let offset = if self.network == 1 {
            1_591_566_291
        } else {
            1_666_656_000
        };
        let slot = now.checked_sub(offset).context("invalid current time")?;
        ensure!(slot >= 0, "time precedes network genesis");
        let lower = tx
            .transaction_body
            .validity_interval_start
            .context("transaction validity start missing")?;
        let upper = tx
            .transaction_body
            .ttl
            .context("transaction expiry missing")?;
        ensure!(
            lower < upper && (slot as u64) >= lower && (slot as u64) < upper,
            "transaction expired or not yet valid"
        );
        let file = hex::decode(&self.file_hex)?;
        ensure!(
            file.len() <= 10 * 1024 * 1024 && file.starts_with(b"%PDF-"),
            "expected PDF at most 10 MiB"
        );
        ensure!(sha256(&file) == self.file_sha256, "PDF hash mismatch");
        ensure!(
            format!("ipfs://{}", evidence::raw_cid(file.as_slice())?) == self.evidence_url,
            "PDF/CID mismatch"
        );
        let payload = self.payload()?;
        ensure!(
            payload.action == "create_evidence_upload_ticket",
            "unexpected upload action"
        );
        ensure!(hex::decode(&payload.nonce)?.len() == 16, "invalid nonce");
        let timestamp = DateTime::parse_from_rfc3339(&payload.timestamp)?.timestamp();
        ensure!(
            timestamp <= now + 30 && timestamp >= lower as i64 + offset,
            "upload timestamp outside transaction window or in future"
        );
        ensure!(
            payload.instance == text(&meta["instance"])?
                && payload.project == text(&meta["body"]["identifier"])?
                && text(&meta["body"]["event"])? == "complete",
            "upload/transaction context mismatch"
        );
        ensure!(payload.files.len() == 1, "expected one PDF");
        let f = &payload.files[0];
        ensure!(
            !f.file_name.is_empty()
                && !f.file_name.contains(['/', '\\'])
                && !f.file_name.chars().any(char::is_control)
                && f.content_type == "application/pdf"
                && f.size_bytes == file.len().to_string(),
            "invalid upload file details"
        );
        let milestones = meta["body"]["milestones"]
            .as_object()
            .context("missing milestones")?;
        ensure!(milestones.len() == 1, "expected one completion milestone");
        let ms = milestones
            .get(&payload.milestone)
            .context("upload/transaction milestone mismatch")?;
        let urls = ms["evidence"].as_array().context("missing evidence")?;
        ensure!(
            urls.len() == 1 && text(&urls[0]["anchorUrl"])? == self.evidence_url,
            "signed evidence URL does not match PDF"
        );
        Ok(())
    }

    pub fn review(&self) -> Result<Value> {
        let raw = hex::decode(&self.tx_cbor_hex)?;
        let (tx, meta) = transaction(&raw)?;
        let outputs: Vec<_> = tx.transaction_body.outputs.iter().map(|output| {
            let output = pallas_traverse::MultiEraOutput::from_conway(output);
            Ok(json!({"address": output.address()?.to_bech32()?, "lovelace": output.value().coin(),
                "assets": output.value().assets().iter().map(|p| json!({"policy": p.policy().to_string(),
                    "assets": p.assets().iter().map(|a| json!({"name": hex::encode(a.name()), "quantity": a.any_coin().to_string()})).collect::<Vec<_>>() })).collect::<Vec<_>>() }))
        }).collect::<Result<_>>()?;
        let refs = |inputs: Vec<&pallas_primitives::conway::TransactionInput>| {
            inputs
                .iter()
                .map(|i| format!("{}#{}", i.transaction_id, i.index))
                .collect::<Vec<_>>()
        };
        let offset = if self.network == 1 {
            1_591_566_291
        } else {
            1_666_656_000
        };
        let expiry = tx
            .transaction_body
            .ttl
            .and_then(|s| DateTime::from_timestamp((s + offset) as i64, 0))
            .map(|d| d.to_rfc3339());
        let remaining_signers: Vec<_> = tx
            .transaction_body
            .required_signers
            .iter()
            .flat_map(|s| s.iter())
            .map(hex::encode)
            .filter(|s| s != &self.signer)
            .collect();
        Ok(
            json!({"requestHash": self.request_hash()?, "bodyHash": self.body_hash,
            "network": self.network, "signer": self.signer, "feeLovelace": tx.transaction_body.fee,
            "validFromSlot": tx.transaction_body.validity_interval_start, "expiresAtSlot": tx.transaction_body.ttl,
            "outputs": outputs, "metadata": meta, "pdfSha256": self.file_sha256,
            "evidenceUrl": self.evidence_url, "uploadAuthorization": self.payload()?,
            "uploadAuthorizationAgeSeconds": Utc::now().timestamp()-DateTime::parse_from_rfc3339(&self.payload()?.timestamp)?.timestamp(),
                "inputs": refs(tx.transaction_body.inputs.iter().collect()),
                "collateral": refs(tx.transaction_body.collateral.iter().flat_map(|s|s.iter()).collect()),
                "remainingSigners": remaining_signers, "expiresAtUtc":expiry,
                "note": "One transaction witness and one upload authorization. Upload CID must match before submission."}),
        )
    }

    pub fn sign(
        &self,
        now: i64,
        signing: impl Fn(&[u8]) -> Result<([u8; 32], [u8; 64])>,
    ) -> Result<SignedPackage> {
        self.validate(now)?;
        let hash: [u8; 32] = hex::decode(&self.body_hash)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid body hash"))?;
        let (vkey, sig) = signing(&hash)?;
        ensure!(
            hex::encode(Hasher::<224>::hash(&vkey)) == self.signer,
            "signing key does not match vendor"
        );
        let mut e = pallas_codec::minicbor::Encoder::new(Vec::new());
        e.array(2)?.bytes(&vkey)?.bytes(&sig)?;
        let witness = json!({"type":"TxWitness ConwayEra", "description":"Completion airgap package", "cborHex": hex::encode(e.into_writer())});
        let payload = hex::decode(&self.upload_payload_hex)?;
        let address = self.address()?;
        let (message_key, message_sig) = signing(&cose::signing_bytes(&payload, &address)?)?;
        ensure!(message_key == vkey, "signing key changed");
        let signed = SignedPackage {
            version: 1,
            request_hash: self.request_hash()?,
            transaction_witness: witness,
            upload_signature: cose::encode(&payload, &address, &vkey, &message_sig)?,
        };
        self.verify(&signed, now)?;
        Ok(signed)
    }

    pub fn verify(&self, signed: &SignedPackage, now: i64) -> Result<Vec<u8>> {
        self.validate(now)?;
        ensure!(
            signed.version == 1 && signed.request_hash == self.request_hash()?,
            "signed response/request mismatch"
        );
        cose::verify(
            &signed.upload_signature,
            &hex::decode(&self.upload_payload_hex)?,
            &self.address()?,
        )?;
        let hash: [u8; 32] = hex::decode(&self.body_hash)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid body hash"))?;
        let signer: [u8; 28] = hex::decode(&self.signer)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid signer"))?;
        sign::assemble_witness(
            &hex::decode(&self.tx_cbor_hex)?,
            &signed.transaction_witness.to_string(),
            &hash,
            &signer,
        )
    }

    pub fn ticket_input(&self, signed: &SignedPackage) -> Result<Value> {
        let p = self.payload()?;
        Ok(
            json!({"instanceId":p.instance,"projectId":p.project,"milestoneId":p.milestone,
            "files":p.files,"publicKeyHex":signed.upload_signature.key,"signatureHex":signed.upload_signature.signature,
            "payloadHex":self.upload_payload_hex}),
        )
    }
}

pub fn write_new(path: &std::path::Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&bytes)?;
    Ok(())
}

/// Build the same file-only multipart request as the portal UI.
pub fn upload_request(
    client: &reqwest::Client,
    package: &Package,
    upload_url: &str,
) -> Result<reqwest::Request> {
    let url = reqwest::Url::parse(upload_url)?;
    ensure!(
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
        "upload ticket must use HTTPS"
    );
    let payload = package.payload()?;
    let file = hex::decode(&package.file_hex)?;
    let boundary = format!("minos-{}", payload.nonce);
    ensure!(
        !file
            .windows(boundary.len())
            .any(|w| w == boundary.as_bytes()),
        "multipart boundary occurs in PDF"
    );
    let name = payload.files[0].file_name.replace('"', "%22");
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/pdf\r\n\r\n").into_bytes();
    body.extend_from_slice(&file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok(client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .build()?)
}

pub fn upload_receipt(package: &Package, response: &Value) -> Result<Value> {
    let cid = response["data"]["cid"]
        .as_str()
        .or_else(|| response["cid"].as_str())
        .context("upload response has no CID")?;
    ensure!(!cid.is_empty(), "upload response CID is empty");
    Ok(
        json!({"bodyHash": package.body_hash, "pdfSha256": package.file_sha256,
        "expectedEvidenceUrl": package.evidence_url, "uploadedEvidenceUrl": format!("ipfs://{cid}"),
        "cidMatches": package.evidence_url == format!("ipfs://{cid}")}),
    )
}

/// Validate every signature before requesting a ticket or transmitting the PDF.
/// This deliberately does not submit or register any transaction.
pub async fn upload_package(
    portal: &crate::portal::Portal,
    package: &Package,
    signed: &SignedPackage,
    receipt_path: &std::path::Path,
    tx_path: &std::path::Path,
) -> Result<()> {
    let assembled = package.verify(signed, Utc::now().timestamp())?;
    ensure!(
        receipt_path != tx_path && !receipt_path.exists() && !tx_path.exists(),
        "output paths must differ and must not exist"
    );
    let url = portal
        .evidence_upload_ticket(package.ticket_input(signed)?)
        .await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let request = upload_request(&client, package, &url)?;
    let response = client
        .execute(request)
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    let receipt = upload_receipt(package, &response)?;
    write_new(receipt_path, &receipt)?;
    ensure!(
        receipt["cidMatches"] == true,
        "PDF uploaded but returned CID differs; receipt saved; fresh transaction/witness needed"
    );
    // Recheck expiry after uploading; never emit a ready expired transaction.
    package.verify(signed, Utc::now().timestamp())?;
    write_new(
        tx_path,
        &json!({"type":"Witnessed Tx ConwayEra", "description":"Verified PDF upload and vendor signatures", "cborHex":hex::encode(assembled)}),
    )?;
    Ok(())
}
