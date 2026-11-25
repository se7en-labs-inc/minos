//! Private invoice protocol. Never emits public evidence or a chain transaction.
use crate::{airgap::sha256, cose, portal::Portal};
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
};

pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 2 * MAX_FILE_BYTES as u64 + 128 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Claims {
    pub invoice_number: String,
    pub invoice_date: String,
    pub due_date: String,
    pub amount: String,
    pub unit: String,
    pub notes: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadPayload {
    pub instance: String,
    pub project: String,
    pub milestone: String,
    pub sha256: String,
    pub invoice_number: String,
    pub invoice_date: String,
    pub due_date: String,
    pub amount: String,
    pub unit: String,
    pub notes: String,
    pub nonce: String,
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlockPayload {
    pub instance: String,
    pub project: String,
    pub nonce: String,
    pub timestamp: String,
}

/// Order matches the public client's JSON.stringify, including action first.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum Payload {
    #[serde(rename = "create_invoice_upload_ticket")]
    Upload(UploadPayload),
    #[serde(rename = "unlock_invoice_session")]
    Unlock(UnlockPayload),
}

impl Payload {
    fn context(&self) -> (&str, &str, &str, &str) {
        match self {
            Self::Upload(p) => (&p.instance, &p.project, &p.nonce, &p.timestamp),
            Self::Unlock(p) => (&p.instance, &p.project, &p.nonce, &p.timestamp),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Package {
    pub version: u8,
    pub network: u8,
    pub signer: String,
    pub file_name: Option<String>,
    pub file_hex: Option<String>,
    pub payload_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedPackage {
    pub version: u8,
    pub request_hash: String,
    pub authorization: cose::DataSignature,
    /// Also binds filename/network fields absent from the portal payload.
    pub package_signature: cose::DataSignature,
}

fn nonempty(value: &str, max: usize, label: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.chars().count() <= max,
        "invalid {label}"
    );
    ensure!(
        !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')),
        "invalid {label} characters"
    );
    Ok(())
}

fn fixed_hex(value: &str, bytes: usize, label: &str) -> Result<()> {
    ensure!(
        value.len() == bytes * 2 && hex::decode(value).is_ok(),
        "invalid {label}"
    );
    Ok(())
}

fn date(value: &str) -> Result<NaiveDate> {
    let date =
        NaiveDate::parse_from_str(value, "%Y-%m-%d").context("invalid date; use YYYY-MM-DD")?;
    ensure!(
        date.format("%Y-%m-%d").to_string() == value,
        "date must use YYYY-MM-DD"
    );
    Ok(date)
}

impl Claims {
    pub fn validate(&self) -> Result<()> {
        nonempty(&self.invoice_number, 120, "invoice number")?;
        ensure!(
            date(&self.due_date)? >= date(&self.invoice_date)?,
            "due date precedes invoice date"
        );
        let mut parts = self.amount.split('.');
        let whole = parts.next().unwrap_or_default();
        ensure!(
            !whole.is_empty() && whole.bytes().all(|b| b.is_ascii_digit()),
            "invalid decimal amount"
        );
        if let Some(fraction) = parts.next() {
            ensure!(
                !fraction.is_empty()
                    && fraction.len() <= 6
                    && fraction.bytes().all(|b| b.is_ascii_digit()),
                "amount allows up to six decimal places"
            );
        }
        ensure!(
            parts.next().is_none() && self.amount.len() <= 100,
            "invalid decimal amount"
        );
        nonempty(&self.unit, 120, "unit")?;
        ensure!(
            self.notes.chars().count() <= 1000
                && !self
                    .notes
                    .chars()
                    .any(|c| matches!(c, '\0' | '\u{2028}' | '\u{2029}')),
            "invalid notes"
        );
        Ok(())
    }
}

pub fn content_type(file_name: &str, bytes: &[u8]) -> Result<&'static str> {
    nonempty(file_name, 255, "filename")?;
    ensure!(
        !file_name.contains(['/', '\\', '"']) && file_name != "." && file_name != "..",
        "expected a plain filename"
    );
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_FILE_BYTES,
        "invoice must contain 1 byte to 10 MiB"
    );
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".pdf") && bytes.starts_with(b"%PDF-") {
        return Ok("application/pdf");
    }
    if lower.ends_with(".png") && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok("image/png");
    }
    if (lower.ends_with(".jpg") || lower.ends_with(".jpeg")) && bytes.starts_with(b"\xff\xd8\xff") {
        return Ok("image/jpeg");
    }
    anyhow::bail!("invoice must be PDF, PNG or JPEG with matching file signature")
}

impl Package {
    #[allow(clippy::too_many_arguments)]
    pub fn upload(
        network: u8,
        signer: String,
        instance: String,
        project: String,
        milestone: String,
        file_name: String,
        bytes: Vec<u8>,
        claims: Claims,
        nonce: String,
        timestamp: String,
    ) -> Result<Self> {
        let payload = Payload::Upload(UploadPayload {
            instance,
            project,
            milestone,
            sha256: sha256(&bytes),
            invoice_number: claims.invoice_number,
            invoice_date: claims.invoice_date,
            due_date: claims.due_date,
            amount: claims.amount,
            unit: claims.unit,
            notes: claims.notes,
            nonce,
            timestamp,
        });
        let package = Self {
            version: 1,
            network,
            signer,
            file_name: Some(file_name),
            file_hex: Some(hex::encode(bytes)),
            payload_hex: hex::encode(serde_json::to_vec(&payload)?),
        };
        package.validate()?;
        Ok(package)
    }

    pub fn unlock(network: u8, signer: String, payload: UnlockPayload) -> Result<Self> {
        let package = Self {
            version: 1,
            network,
            signer,
            file_name: None,
            file_hex: None,
            payload_hex: hex::encode(serde_json::to_vec(&Payload::Unlock(payload))?),
        };
        package.validate()?;
        Ok(package)
    }

    pub fn payload(&self) -> Result<Payload> {
        ensure!(
            self.payload_hex.len() <= 32 * 1024,
            "authorization too large"
        );
        let bytes = hex::decode(&self.payload_hex).context("invalid invoice payload hex")?;
        let payload: Payload =
            serde_json::from_slice(&bytes).context("invalid invoice authorization")?;
        ensure!(
            serde_json::to_vec(&payload)? == bytes,
            "noncanonical invoice authorization"
        );
        Ok(payload)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.network <= 1,
            "unsupported invoice package version/network"
        );
        fixed_hex(&self.signer, 28, "signer")?;
        let payload = self.payload()?;
        let (instance, project, nonce, timestamp) = payload.context();
        fixed_hex(instance, 28, "instance")?;
        fixed_hex(nonce, 16, "nonce")?;
        nonempty(project, 256, "project")?;
        DateTime::parse_from_rfc3339(timestamp).context("invalid authorization timestamp")?;
        match payload {
            Payload::Upload(p) => {
                nonempty(&p.milestone, 256, "milestone")?;
                Claims {
                    invoice_number: p.invoice_number,
                    invoice_date: p.invoice_date,
                    due_date: p.due_date,
                    amount: p.amount,
                    unit: p.unit,
                    notes: p.notes,
                }
                .validate()?;
                let file = self.file_bytes()?;
                content_type(
                    self.file_name
                        .as_deref()
                        .context("missing invoice filename")?,
                    &file,
                )?;
                ensure!(sha256(&file) == p.sha256, "invoice SHA-256 mismatch");
            }
            Payload::Unlock(_) => ensure!(
                self.file_name.is_none() && self.file_hex.is_none(),
                "unlock request must not embed a file"
            ),
        }
        Ok(())
    }

    pub fn file_bytes(&self) -> Result<Vec<u8>> {
        let file = self.file_hex.as_deref().context("missing invoice bytes")?;
        ensure!(file.len() <= 2 * MAX_FILE_BYTES, "invoice exceeds 10 MiB");
        hex::decode(file).context("invalid invoice hex")
    }

    fn address(&self) -> Result<Vec<u8>> {
        let mut address = vec![0x60 | self.network];
        address.extend(hex::decode(&self.signer)?);
        Ok(address)
    }

    pub fn request_hash(&self) -> Result<String> {
        Ok(sha256(&serde_json::to_vec(self)?))
    }
    fn binding_bytes(&self) -> Result<Vec<u8>> {
        Ok(format!("minos-private-invoice-v1:{}", self.request_hash()?).into_bytes())
    }

    pub fn review(&self) -> Result<Value> {
        self.validate()?;
        Ok(
            json!({"kind":"private-invoice", "network":self.network, "signer":self.signer,
            "requestHash":self.request_hash()?, "fileName":self.file_name,
            "sizeBytes":self.file_hex.as_ref().map(|s| s.len()/2), "authorization":self.payload()?,
            "note":"Private invoice authorization only; no Cardano transaction. Online freshness is checked at upload/unlock."}),
        )
    }

    pub fn sign(
        &self,
        mut signing: impl FnMut(&[u8]) -> Result<([u8; 32], [u8; 64])>,
    ) -> Result<SignedPackage> {
        self.validate()?;
        let address = self.address()?;
        let mut sign = |payload: &[u8]| -> Result<cose::DataSignature> {
            let (key, signature) = signing(&cose::signing_bytes(payload, &address)?)?;
            let result = cose::encode(payload, &address, &key, &signature)?;
            cose::verify(&result, payload, &address)?;
            Ok(result)
        };
        let signed = SignedPackage {
            version: 1,
            request_hash: self.request_hash()?,
            authorization: sign(&hex::decode(&self.payload_hex)?)?,
            package_signature: sign(&self.binding_bytes()?)?,
        };
        self.verify(&signed)?;
        Ok(signed)
    }

    pub fn verify(&self, signed: &SignedPackage) -> Result<()> {
        self.validate()?;
        ensure!(
            signed.version == 1 && signed.request_hash == self.request_hash()?,
            "invoice response/request mismatch"
        );
        cose::verify(
            &signed.authorization,
            &hex::decode(&self.payload_hex)?,
            &self.address()?,
        )?;
        cose::verify(
            &signed.package_signature,
            &self.binding_bytes()?,
            &self.address()?,
        )?;
        Ok(())
    }

    pub fn fresh(&self, network: u8, now: DateTime<Utc>) -> Result<()> {
        self.validate()?;
        ensure!(
            self.network == network,
            "invoice package/CLI network mismatch"
        );
        let payload = self.payload()?;
        let timestamp = DateTime::parse_from_rfc3339(payload.context().3)?.with_timezone(&Utc);
        let age = now.signed_duration_since(timestamp).num_milliseconds();
        ensure!(
            (-30_000..=120_000).contains(&age),
            "invoice authorization is stale or in the future; prepare and sign again"
        );
        Ok(())
    }

    pub fn ticket_input(&self, signed: &SignedPackage) -> Result<Value> {
        self.verify(signed)?;
        let Payload::Upload(p) = self.payload()? else {
            anyhow::bail!("expected invoice upload package")
        };
        Ok(
            json!({"instanceId":p.instance,"projectId":p.project,"milestoneId":p.milestone,
            "fileName":self.file_name,"sha256":p.sha256,"invoiceNumber":p.invoice_number,
            "invoiceDate":p.invoice_date,"dueDate":p.due_date,"amount":p.amount,"unit":p.unit,"notes":p.notes,
            "publicKeyHex":signed.authorization.key,"signatureHex":signed.authorization.signature,"payloadHex":self.payload_hex}),
        )
    }
}

pub fn read_private<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&read_bounded(path, MAX_PACKAGE_BYTES)?)
        .context("invalid private invoice file")
}

pub fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut data)?;
    ensure!(
        data.len() as u64 <= max,
        "private invoice file exceeds size limit"
    );
    Ok(data)
}

pub fn create_private(path: &Path) -> Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .context("private output must not already exist")
}

pub fn write_private(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = create_private(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn private_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .context("invoice receipt directory must not already exist")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ticket {
    pub invoice_id: String,
    pub upload_url: String,
    pub expires_at: String,
    pub confirm_token: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub version: u8,
    pub network: u8,
    pub request_hash: String,
    pub instance: String,
    pub project: String,
    pub milestone: String,
    pub invoice_id: String,
    pub confirm_token: String,
    pub cid: String,
    pub sha256: String,
}

const INVOICE_FIELDS: &str = "invoiceId milestoneId status fileName contentType sizeBytes sha256 uploadedAt confirmedAt uploaderRole uploaderLabel invoiceNumber invoiceDate dueDate amount unit notes";

struct FileClient {
    client: reqwest::Client,
    #[cfg(test)]
    test_address: Option<std::net::SocketAddr>,
}

fn private_client() -> Result<FileClient> {
    Ok(FileClient {
        client: crate::http::client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("create private transfer client")?,
        #[cfg(test)]
        test_address: None,
    })
}

fn secure_url(value: &str) -> Result<reqwest::Url> {
    let url =
        reqwest::Url::parse(value).map_err(|_| anyhow::anyhow!("invalid private transfer URL"))?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "private transfer requires HTTPS without credentials or fragment"
    );
    Ok(url)
}

async fn transfer(client: &FileClient, request: reqwest::Request) -> Result<Vec<u8>> {
    #[cfg(test)]
    let request = {
        let mut request = request;
        if let Some(address) = client.test_address {
            // Only tests replace an already HTTPS-validated capability URL with
            // their loopback server; production has no such override.
            request.url_mut().set_scheme("http").unwrap();
            request.url_mut().set_host(Some("127.0.0.1")).unwrap();
            request.url_mut().set_port(Some(address.port())).unwrap();
        }
        request
    };
    let response = client
        .client
        .execute(request)
        .await
        .map_err(|_| anyhow::anyhow!("private file transfer failed; outcome may be unknown"))?;
    ensure!(
        response.status().is_success(),
        "private file transfer returned HTTP {}",
        response.status().as_u16()
    );
    crate::http::body_bytes(response)
        .await
        .map_err(|_| anyhow::anyhow!("private transfer response could not be read within limits"))
}

pub fn multipart_request(
    client: &reqwest::Client,
    package: &Package,
    upload_url: &str,
) -> Result<reqwest::Request> {
    package.validate()?;
    let Payload::Upload(p) = package.payload()? else {
        anyhow::bail!("expected upload authorization")
    };
    let file = package.file_bytes()?;
    let name = package.file_name.as_deref().context("missing filename")?;
    let boundary = format!("minos-invoice-{}", p.nonce);
    ensure!(
        !file
            .windows(boundary.len())
            .any(|w| w == boundary.as_bytes()),
        "multipart boundary occurs in invoice"
    );
    let mime = content_type(name, &file)?;
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n").into_bytes();
    body.extend(file);
    body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok(client
        .post(secure_url(upload_url)?)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .build()?)
}

async fn server_now(portal: &Portal) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(&portal.server_timestamp().await?)?.with_timezone(&Utc))
}

async fn private_query(portal: &Portal, query: &str, variables: Value) -> Result<Value> {
    // Server/proxy errors may echo confirmation/session tokens from the input.
    // Keep stage context at the call site, but never print those response bodies.
    portal.query(query, Some(variables)).await.map_err(|_| {
        anyhow::anyhow!("private invoice portal request failed or was rejected; inspect the saved stage before retrying")
    })
}

/// Private CID is opaque; it need not match the public evidence UnixFS import.
fn uploaded_cid(response: &Value) -> Result<String> {
    let cid = response["data"]["cid"]
        .as_str()
        .or_else(|| response["cid"].as_str())
        .context("private upload response has no CID; inspect saved ticket before retrying")?;
    ensure!(
        !cid.is_empty() && cid.len() <= 256 && cid.bytes().all(|b| b.is_ascii_alphanumeric()),
        "invalid private storage CID"
    );
    Ok(cid.to_owned())
}

pub async fn upload(
    portal: &Portal,
    network: u8,
    package: &Package,
    signed: &SignedPackage,
    out: &Path,
) -> Result<()> {
    let client = private_client()?;
    upload_with_client(portal, network, package, signed, out, &client).await
}

/// Client injection permits deterministic local HTTP transport tests; production
/// always uses a redirect-disabled client and validates HTTPS ticket URLs.
async fn upload_with_client(
    portal: &Portal,
    network: u8,
    package: &Package,
    signed: &SignedPackage,
    out: &Path,
    client: &FileClient,
) -> Result<()> {
    package.verify(signed)?;
    let input = package.ticket_input(signed)?;
    ensure!(
        std::fs::symlink_metadata(out).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "receipt directory already exists or is inaccessible"
    );
    package.fresh(network, server_now(portal).await?)?;
    private_directory(out)?;
    write_private(
        &out.join("request.json"),
        &json!({"requestHash":package.request_hash()?,"network":network}),
    )?;
    let data = private_query(portal, "mutation InvoiceTicket($input: CreateInvoiceUploadTicketInput!) { createInvoiceUploadTicket(input: $input) { invoiceId uploadUrl expiresAt confirmToken } }", json!({"input":input})).await.context("invoice ticket failed; no file uploaded")?;
    let ticket: Ticket = serde_json::from_value(data["createInvoiceUploadTicket"].clone())
        .context("invalid private invoice ticket")?;
    write_private(&out.join("ticket.json"), &ticket)?;
    nonempty(&ticket.invoice_id, 256, "invoice ID")?;
    ensure!(
        !ticket.confirm_token.is_empty(),
        "invoice ticket has no confirmation token"
    );
    ensure!(
        DateTime::parse_from_rfc3339(&ticket.expires_at)? > Utc::now(),
        "private upload ticket expired"
    );
    let request = multipart_request(&client.client, package, &ticket.upload_url)?;
    let response: Value = serde_json::from_slice(&transfer(client, request).await?).context(
        "invalid private upload response; ticket saved, inspect outcome before retrying",
    )?;
    let Payload::Upload(p) = package.payload()? else {
        unreachable!()
    };
    let receipt = Receipt {
        version: 1,
        network,
        request_hash: package.request_hash()?,
        instance: p.instance,
        project: p.project,
        milestone: p.milestone,
        invoice_id: ticket.invoice_id,
        confirm_token: ticket.confirm_token,
        cid: uploaded_cid(&response)?,
        sha256: p.sha256,
    };
    write_private(&out.join("uploaded.json"), &receipt)?;
    confirm(portal, network, &receipt, &out.join("confirmed.json"))
        .await
        .context(
            "file uploaded; use invoice confirm with uploaded.json to finish without reuploading",
        )
}

pub async fn confirm(portal: &Portal, network: u8, receipt: &Receipt, out: &Path) -> Result<()> {
    ensure!(
        receipt.version == 1 && receipt.network == network,
        "receipt version/network mismatch"
    );
    fixed_hex(&receipt.instance, 28, "instance")?;
    fixed_hex(&receipt.sha256, 32, "invoice SHA-256")?;
    nonempty(&receipt.project, 256, "project")?;
    nonempty(&receipt.milestone, 256, "milestone")?;
    nonempty(&receipt.invoice_id, 256, "invoice ID")?;
    ensure!(
        !receipt.confirm_token.is_empty() && !receipt.cid.is_empty(),
        "incomplete invoice upload receipt"
    );
    // Reserve output before external mutation. A failed attempt leaves a private
    // unknown-outcome record; retry confirmation with a fresh output name.
    let mut file = create_private(out)?;
    file.write_all(b"{\"stage\":\"confirmation-outcome-unknown\"}")?;
    file.sync_all()?;
    let data = private_query(portal, "mutation ConfirmInvoice($input: ConfirmInvoiceUploadInput!) { confirmInvoiceUpload(input: $input) { ok } }", json!({"input": {"instanceId":receipt.instance,"projectId":receipt.project,"milestoneId":receipt.milestone,"invoiceId":receipt.invoice_id,"confirmToken":receipt.confirm_token,"cid":receipt.cid}})).await?;
    let ok = &data["confirmInvoiceUpload"]["ok"];
    ensure!(
        ok == true || ok == "Ok",
        "portal did not acknowledge invoice confirmation"
    );
    let summary = json!({"stage":"confirmed","network":network,"instance":receipt.instance,"project":receipt.project,"milestone":receipt.milestone,"invoiceId":receipt.invoice_id,"sha256":receipt.sha256,"requestHash":receipt.request_hash});
    file.set_len(0)?;
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&serde_json::to_vec_pretty(&summary)?)?;
    file.sync_all()?;
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
    pub version: u8,
    pub network: u8,
    pub instance: String,
    pub project: String,
    pub token: String,
    pub role: String,
    pub expires_at: String,
}

impl Session {
    pub fn validate(&self, network: u8, now: DateTime<Utc>) -> Result<()> {
        ensure!(
            self.version == 1 && self.network == network,
            "invoice session version/network mismatch"
        );
        fixed_hex(&self.instance, 28, "instance")?;
        nonempty(&self.project, 256, "project")?;
        ensure!(!self.token.is_empty(), "missing invoice session token");
        ensure!(
            DateTime::parse_from_rfc3339(&self.expires_at)? > now,
            "invoice session expired; unlock again"
        );
        Ok(())
    }
}

pub async fn unlock(
    portal: &Portal,
    network: u8,
    package: &Package,
    signed: &SignedPackage,
    out: &Path,
) -> Result<()> {
    package.verify(signed)?;
    let Payload::Unlock(p) = package.payload()? else {
        anyhow::bail!("expected invoice unlock package")
    };
    let mut file = create_private(out)?;
    package.fresh(network, server_now(portal).await?)?;
    let data = private_query(portal, "mutation UnlockInvoices($input: UnlockInvoiceSessionInput!) { unlockInvoiceSession(input: $input) { token role projectId expiresAt } }", json!({"input":{"instanceId":p.instance,"projectId":p.project,"publicKeyHex":signed.authorization.key,"signatureHex":signed.authorization.signature,"payloadHex":package.payload_hex}})).await?;
    let value = &data["unlockInvoiceSession"];
    ensure!(
        value["projectId"] == p.project,
        "portal returned a different invoice session scope"
    );
    let session = Session {
        version: 1,
        network,
        instance: p.instance,
        project: p.project,
        token: value["token"]
            .as_str()
            .context("missing invoice session token")?
            .into(),
        role: value["role"]
            .as_str()
            .context("missing invoice session role")?
            .into(),
        expires_at: value["expiresAt"]
            .as_str()
            .context("missing invoice session expiry")?
            .into(),
    };
    session.validate(network, Utc::now())?;
    file.write_all(&serde_json::to_vec_pretty(&session)?)?;
    file.sync_all()?;
    Ok(())
}

pub async fn list(portal: &Portal, network: u8, session: &Session) -> Result<Value> {
    session.validate(network, Utc::now())?;
    let query = format!("query Invoices($instance: String!, $project: String!, $token: String!) {{ projectInvoices(instanceId:$instance, projectId:$project, sessionToken:$token) {{ instanceId projectId viewerIsVendor invoices {{ {INVOICE_FIELDS} }} }} }}");
    let data = private_query(
        portal,
        &query,
        json!({"instance":session.instance,"project":session.project,"token":session.token}),
    )
    .await?;
    let result = &data["projectInvoices"];
    ensure!(
        result["instanceId"] == session.instance
            && result["projectId"] == session.project
            && result["invoices"].is_array(),
        "invoice list scope/shape mismatch"
    );
    Ok(result.clone())
}

pub async fn download(
    portal: &Portal,
    network: u8,
    session: &Session,
    milestone: &str,
    invoice: &str,
    out: &Path,
) -> Result<()> {
    let client = private_client()?;
    download_with_client(portal, network, session, milestone, invoice, out, &client).await
}

async fn download_with_client(
    portal: &Portal,
    network: u8,
    session: &Session,
    milestone: &str,
    invoice: &str,
    out: &Path,
    client: &FileClient,
) -> Result<()> {
    session.validate(network, Utc::now())?;
    let mut file = create_private(out)?;
    let invoices = list(portal, network, session).await?;
    let matches: Vec<_> = invoices["invoices"]
        .as_array()
        .context("missing invoices")?
        .iter()
        .filter(|i| i["invoiceId"] == invoice && i["milestoneId"] == milestone)
        .collect();
    ensure!(
        matches.len() == 1,
        "invoice not uniquely found on selected milestone"
    );
    let record = matches[0];
    ensure!(
        record["status"] == "Available",
        "invoice is not available for download"
    );
    let expected = record["sha256"]
        .as_str()
        .context("missing invoice SHA-256")?;
    fixed_hex(expected, 32, "invoice SHA-256")?;
    let data = private_query(portal, "mutation InvoiceDownload($input: CreateInvoiceDownloadLinkInput!) { createInvoiceDownloadLink(input:$input) { url expiresAt } }", json!({"input":{"instanceId":session.instance,"projectId":session.project,"milestoneId":milestone,"invoiceId":invoice,"sessionToken":session.token}})).await?;
    let link = &data["createInvoiceDownloadLink"];
    ensure!(
        DateTime::parse_from_rfc3339(link["expiresAt"].as_str().context("missing link expiry")?)?
            > Utc::now(),
        "invoice download link expired"
    );
    let url = secure_url(
        link["url"]
            .as_str()
            .context("missing private download URL")?,
    )?;
    let request = client.client.get(url).build()?;
    let bytes = transfer(client, request).await?;
    ensure!(
        bytes.len() <= MAX_FILE_BYTES && sha256(&bytes) == expected,
        "downloaded invoice SHA-256/size mismatch; no content saved"
    );
    if let Some(size) = record["sizeBytes"].as_u64() {
        ensure!(
            bytes.len() as u64 == size,
            "downloaded invoice size mismatch"
        );
    }
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
#[path = "invoice_tests.rs"]
mod tests;
