/// CIP-100 metadata encoding for treasury events (key 1694).
///
/// Publish and fund events add a 5th key "txAuthor"; complete/withdraw keep 4 keys.
use anyhow::Result;
use pallas_crypto::hash::Hasher;

const CONTEXT_URL: &str =
    "https://raw.githubusercontent.com/SundaeSwap-finance/treasury-contracts/refs/heads/main/offchain/src/metadata/context.jsonld";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct FundMilestone {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn encode_publish(
    instance_id: &str,
    seed_tx_hex: &str,
    seed_index: u64,
    expiration_ms: u64,
    payout_upperbound_ms: u64,
    vendor_expiration_ms: u64,
    admin_key_hex: &str,
    label: &str,
    description: Option<&str>,
) -> Result<(Vec<u8>, [u8; 32])> {
    let body = build_publish_body(
        label,
        description,
        expiration_ms,
        payout_upperbound_ms,
        vendor_expiration_ms,
        admin_key_hex,
        seed_tx_hex,
        seed_index,
    );
    encode_metadata(instance_id, body, Some(admin_key_hex))
}

pub fn encode_fund(
    instance_id: &str,
    tx_author_hex: &str,
    project_id: &str,
    label: &str,
    description: &str,
    vendor_label: &str,
    milestones: &[FundMilestone],
) -> Result<(Vec<u8>, [u8; 32])> {
    let body = build_fund_body(project_id, label, description, vendor_label, milestones);
    encode_metadata(instance_id, body, Some(tx_author_hex))
}

pub fn encode_complete(
    instance_id: &str,
    project_id: &str,
    milestone_id: &str,
    description: &str,
    evidence_urls: &[String],
) -> Result<(Vec<u8>, [u8; 32])> {
    let body = build_complete_body(project_id, milestone_id, description, evidence_urls);
    encode_metadata(instance_id, body, None)
}

pub fn encode_withdraw(instance_id: &str) -> Result<(Vec<u8>, [u8; 32])> {
    let body = build_withdraw_body();
    encode_metadata(instance_id, body, None)
}

pub fn encode_adjudicate(instance_id: &str) -> Result<(Vec<u8>, [u8; 32])> {
    let body = build_event_body("adjudicate");
    encode_metadata(instance_id, body, None)
}

pub fn encode_modify(instance_id: &str) -> Result<(Vec<u8>, [u8; 32])> {
    let body = build_event_body("modify");
    encode_metadata(instance_id, body, None)
}

// ---------------------------------------------------------------------------
// Core encoder
// ---------------------------------------------------------------------------

fn encode_metadata(
    instance_id: &str,
    body_cbor: Vec<u8>,
    tx_author: Option<&str>,
) -> Result<(Vec<u8>, [u8; 32])> {
    let n_keys: usize = if tx_author.is_some() { 5 } else { 4 };
    let mut entries = Vec::new();

    entries.extend_from_slice(&encode_text("@context"));
    entries.extend_from_slice(&encode_context_array());

    entries.extend_from_slice(&encode_text("hashAlgorithm"));
    entries.extend_from_slice(&encode_text("blake2b-256"));

    if let Some(author) = tx_author {
        entries.extend_from_slice(&encode_text("txAuthor"));
        entries.extend_from_slice(&encode_text(author));
    }

    entries.extend_from_slice(&encode_text("instance"));
    entries.extend_from_slice(&encode_text(instance_id));

    entries.extend_from_slice(&encode_text("body"));
    entries.extend_from_slice(&body_cbor);

    let inner = encode_map_cbor(n_keys, &entries);

    let mut outer = Vec::new();
    outer.push(0xa1); // map(1)
    outer.push(0x19);
    outer.push(0x06);
    outer.push(0x9e); // 1694
    outer.extend_from_slice(&inner);

    let hash_raw = Hasher::<256>::hash(&outer);
    let mut hash = [0u8; 32];
    hash.copy_from_slice(hash_raw.as_ref());
    Ok((outer, hash))
}

// ---------------------------------------------------------------------------
// Body builders
// ---------------------------------------------------------------------------

fn build_publish_body(
    label: &str,
    description: Option<&str>,
    expiration_ms: u64,
    payout_upperbound_ms: u64,
    vendor_expiration_ms: u64,
    admin_key_hex: &str,
    seed_tx_hex: &str,
    seed_index: u64,
) -> Vec<u8> {
    // permissions map: 7 entries, all same single-key signature
    let sig_inner_entries = {
        let mut e = Vec::new();
        e.extend_from_slice(&encode_text("keyHash"));
        e.extend_from_slice(&encode_text(admin_key_hex));
        e
    };
    let sig_map = encode_map_cbor(1, &sig_inner_entries);
    let mut sig_entry = Vec::new();
    sig_entry.extend_from_slice(&encode_text("signature"));
    sig_entry.extend_from_slice(&sig_map);
    let perm_val = encode_map_cbor(1, &sig_entry);

    let perm_names = [
        "reorganize",
        "sweep",
        "fund",
        "disburse",
        "pause",
        "resume",
        "modify",
    ];
    let mut perm_entries = Vec::new();
    for name in &perm_names {
        perm_entries.extend_from_slice(&encode_text(name));
        perm_entries.extend_from_slice(&perm_val);
    }
    let permissions_cbor = encode_map_cbor(7, &perm_entries);

    // seedUtxo: {"transaction_id": hex, "output_index": "n"}
    // Portal indexer expects string values for numeric fields (matching working instances).
    let mut seed_entries = Vec::new();
    seed_entries.extend_from_slice(&encode_text("transaction_id"));
    seed_entries.extend_from_slice(&encode_text(seed_tx_hex));
    seed_entries.extend_from_slice(&encode_text("output_index"));
    seed_entries.extend_from_slice(&encode_text(&seed_index.to_string()));
    let seed_utxo_cbor = encode_map_cbor(2, &seed_entries);

    let n_keys = if description.is_some() { 8 } else { 7 };
    let mut body_entries = Vec::new();
    body_entries.extend_from_slice(&encode_text("event"));
    body_entries.extend_from_slice(&encode_text("publish"));
    body_entries.extend_from_slice(&encode_text("label"));
    body_entries.extend_from_slice(&encode_text_chunked(label));
    if let Some(desc) = description {
        body_entries.extend_from_slice(&encode_text("description"));
        body_entries.extend_from_slice(&encode_text_chunked(desc));
    }
    body_entries.extend_from_slice(&encode_text("expiration"));
    body_entries.extend_from_slice(&encode_text(&expiration_ms.to_string()));
    body_entries.extend_from_slice(&encode_text("payoutUpperbound"));
    body_entries.extend_from_slice(&encode_text(&payout_upperbound_ms.to_string()));
    body_entries.extend_from_slice(&encode_text("vendorExpiration"));
    body_entries.extend_from_slice(&encode_text(&vendor_expiration_ms.to_string()));
    body_entries.extend_from_slice(&encode_text("permissions"));
    body_entries.extend_from_slice(&permissions_cbor);
    body_entries.extend_from_slice(&encode_text("seedUtxo"));
    body_entries.extend_from_slice(&seed_utxo_cbor);
    encode_map_cbor(n_keys, &body_entries)
}

fn build_fund_body(
    project_id: &str,
    label: &str,
    description: &str,
    vendor_label: &str,
    milestones: &[FundMilestone],
) -> Vec<u8> {
    // vendor: {"label": "..."}
    let mut vendor_entries = Vec::new();
    vendor_entries.extend_from_slice(&encode_text("label"));
    vendor_entries.extend_from_slice(&encode_text_chunked(vendor_label));
    let vendor_cbor = encode_map_cbor(1, &vendor_entries);

    // otherIdentifiers: []
    let other_ids_cbor = encode_array_cbor(0, &[]);

    // milestones map: indexed by milestone ID
    let mut ms_entries = Vec::new();
    for m in milestones {
        ms_entries.extend_from_slice(&encode_text(&m.id));
        let n_ms_keys = if m.description.is_some() { 2 } else { 1 };
        let mut ms_map_entries = Vec::new();
        ms_map_entries.extend_from_slice(&encode_text("label"));
        ms_map_entries.extend_from_slice(&encode_text_chunked(&m.label));
        if let Some(desc) = &m.description {
            ms_map_entries.extend_from_slice(&encode_text("description"));
            ms_map_entries.extend_from_slice(&encode_text_chunked(desc));
        }
        ms_entries.extend_from_slice(&encode_map_cbor(n_ms_keys, &ms_map_entries));
    }
    let milestones_cbor = encode_map_cbor(milestones.len(), &ms_entries);

    let mut body_entries = Vec::new();
    body_entries.extend_from_slice(&encode_text("event"));
    body_entries.extend_from_slice(&encode_text("fund"));
    body_entries.extend_from_slice(&encode_text("identifier"));
    body_entries.extend_from_slice(&encode_text(project_id));
    body_entries.extend_from_slice(&encode_text("label"));
    body_entries.extend_from_slice(&encode_text_chunked(label));
    body_entries.extend_from_slice(&encode_text("description"));
    body_entries.extend_from_slice(&encode_text_chunked(description));
    body_entries.extend_from_slice(&encode_text("vendor"));
    body_entries.extend_from_slice(&vendor_cbor);
    body_entries.extend_from_slice(&encode_text("otherIdentifiers"));
    body_entries.extend_from_slice(&other_ids_cbor);
    body_entries.extend_from_slice(&encode_text("milestones"));
    body_entries.extend_from_slice(&milestones_cbor);
    encode_map_cbor(7, &body_entries)
}

fn build_complete_body(
    project_id: &str,
    milestone_id: &str,
    description: &str,
    evidence_urls: &[String],
) -> Vec<u8> {
    let ev_count = evidence_urls.len();
    let mut evidence_items = Vec::new();
    for url in evidence_urls {
        let mut entry_entries = Vec::new();
        entry_entries.extend_from_slice(&encode_text("anchorUrl"));
        entry_entries.extend_from_slice(&encode_text_chunked(url));
        evidence_items.extend_from_slice(&encode_map_cbor(1, &entry_entries));
    }
    let evidence_arr = encode_array_cbor(ev_count, &evidence_items);

    let mut ms_entry_entries = Vec::new();
    ms_entry_entries.extend_from_slice(&encode_text("description"));
    ms_entry_entries.extend_from_slice(&encode_text_chunked(description));
    ms_entry_entries.extend_from_slice(&encode_text("evidence"));
    ms_entry_entries.extend_from_slice(&evidence_arr);
    let ms_entry = encode_map_cbor(2, &ms_entry_entries);

    let mut ms_map_entries = Vec::new();
    ms_map_entries.extend_from_slice(&encode_text(milestone_id));
    ms_map_entries.extend_from_slice(&ms_entry);
    let milestones_map = encode_map_cbor(1, &ms_map_entries);

    let mut body_entries = Vec::new();
    body_entries.extend_from_slice(&encode_text("event"));
    body_entries.extend_from_slice(&encode_text("complete"));
    body_entries.extend_from_slice(&encode_text("identifier"));
    body_entries.extend_from_slice(&encode_text(project_id));
    body_entries.extend_from_slice(&encode_text("milestones"));
    body_entries.extend_from_slice(&milestones_map);
    encode_map_cbor(3, &body_entries)
}

fn build_withdraw_body() -> Vec<u8> {
    build_event_body("withdraw")
}

fn build_event_body(event: &str) -> Vec<u8> {
    let mut entries = Vec::new();
    entries.extend_from_slice(&encode_text("event"));
    entries.extend_from_slice(&encode_text(event));
    encode_map_cbor(1, &entries)
}

// ---------------------------------------------------------------------------
// CBOR primitive helpers
// ---------------------------------------------------------------------------

/// Encode a CBOR unsigned integer (major type 0).
pub(crate) fn encode_uint(n: u64) -> Vec<u8> {
    if n <= 23 {
        vec![n as u8]
    } else if n <= 0xff {
        vec![0x18, n as u8]
    } else if n <= 0xffff {
        vec![0x19, (n >> 8) as u8, n as u8]
    } else if n <= 0xffff_ffff {
        vec![
            0x1a,
            (n >> 24) as u8,
            (n >> 16) as u8,
            (n >> 8) as u8,
            n as u8,
        ]
    } else {
        vec![
            0x1b,
            (n >> 56) as u8,
            (n >> 48) as u8,
            (n >> 40) as u8,
            (n >> 32) as u8,
            (n >> 24) as u8,
            (n >> 16) as u8,
            (n >> 8) as u8,
            n as u8,
        ]
    }
}

/// Encode a definite-length CBOR map header + pre-encoded key-value pairs.
fn encode_map_cbor(n: usize, entries: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    if n <= 23 {
        buf.push(0xa0 | n as u8);
    } else {
        buf.push(0xb8);
        buf.push(n as u8);
    }
    buf.extend_from_slice(entries);
    buf
}

/// Encode a definite-length CBOR array header + pre-encoded items.
fn encode_array_cbor(n: usize, items: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    if n <= 23 {
        buf.push(0x80 | n as u8);
    } else {
        buf.push(0x98);
        buf.push(n as u8);
    }
    buf.extend_from_slice(items);
    buf
}

/// Encode a text string (major type 3). Panics if s is longer than 64 KB.
fn encode_text(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut buf = Vec::with_capacity(1 + len);
    if len <= 23 {
        buf.push(0x60 | len as u8);
    } else if len <= 0xff {
        buf.push(0x78);
        buf.push(len as u8);
    } else if len <= 0xffff {
        buf.push(0x79);
        buf.push((len >> 8) as u8);
        buf.push(len as u8);
    } else {
        panic!("text too long: {len}");
    }
    buf.extend_from_slice(bytes);
    buf
}

/// Encode a text value: single string if ≤64 bytes, else CBOR array of ≤64-byte chunks.
/// Cardano metadata requires text strings to be at most 64 bytes.
fn encode_text_chunked(s: &str) -> Vec<u8> {
    if s.len() <= 64 {
        encode_text(s)
    } else {
        let chunks = chunk_text(s, 64);
        let n = chunks.len();
        let mut items = Vec::new();
        for chunk in &chunks {
            items.extend_from_slice(&encode_text(chunk));
        }
        encode_array_cbor(n, &items)
    }
}

/// Encode the @context URL as an array of ≤64-byte text chunks.
fn encode_context_array() -> Vec<u8> {
    let chunks = chunk_text(CONTEXT_URL, 64);
    let n = chunks.len();
    let mut items = Vec::new();
    for chunk in &chunks {
        items.extend_from_slice(&encode_text(chunk));
    }
    encode_array_cbor(n, &items)
}

/// Split a UTF-8 string into chunks of at most `max_bytes` bytes.
fn chunk_text(s: &str, max_bytes: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut remaining = s;
    while !remaining.is_empty() {
        let split_at = if remaining.len() <= max_bytes {
            remaining.len()
        } else {
            let mut pos = max_bytes;
            while !remaining.is_char_boundary(pos) {
                pos -= 1;
            }
            pos
        };
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }
    chunks
}
