/// Add a VKey witness to an existing (possibly partially witnessed) tx CBOR.
///
/// Cardano Conway tx CBOR: array(4) [body, witness_set, is_valid, aux_data]
/// VKey witnesses live at key 0 in the witness_set map.
use anyhow::{Context, Result};
use pallas_codec::minicbor::Decoder;
use pallas_crypto::hash::{Hash, Hasher};

/// Compute the tx body hash (blake2b-256 of the body CBOR bytes).
pub fn tx_body_hash(tx_cbor: &[u8]) -> Result<[u8; 32]> {
    let mut dec = Decoder::new(tx_cbor);
    dec.array().context("outer array")?;
    let body_start = dec.position();
    dec.skip().context("body skip")?;
    let body_bytes = &tx_cbor[body_start..dec.position()];
    Ok(*Hasher::<256>::hash(body_bytes))
}

/// Add a VKey witness (vkey=32 bytes, sig=64 bytes) to a tx CBOR.
/// Returns new CBOR bytes with the witness appended to key 0 of the witness set.
pub fn add_vkey_witness(tx_cbor: &[u8], vkey: &[u8; 32], signature: &[u8; 64]) -> Result<Vec<u8>> {
    let mut dec = Decoder::new(tx_cbor);
    dec.array().context("outer array")?;

    let body_start = dec.position();
    dec.skip().context("body skip")?;
    let body_bytes = &tx_cbor[body_start..dec.position()];

    let ws_start = dec.position();
    dec.skip().context("witness_set skip")?;
    let ws_bytes = &tx_cbor[ws_start..dec.position()];

    let valid_start = dec.position();
    dec.skip().context("valid skip")?;
    let valid_bytes = &tx_cbor[valid_start..dec.position()];

    let aux_start = dec.position();
    dec.skip().context("aux_data skip")?;
    let aux_bytes = &tx_cbor[aux_start..dec.position()];

    // -----------------------------------------------------------------------
    // Parse witness_set map, collect existing entries
    // -----------------------------------------------------------------------
    let mut wsdec = Decoder::new(ws_bytes);
    let ws_map_type = wsdec.datatype().context("ws datatype")?;

    let mut ws_entries: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut existing_vkwitnesses: Vec<Vec<u8>> = Vec::new();

    if ws_map_type == pallas_codec::minicbor::data::Type::Map {
        let n = wsdec
            .map()
            .context("ws map")?
            .context("indefinite ws map")?;
        for _ in 0..n {
            let key = wsdec.u64().context("ws key")?;
            let vs = wsdec.position();
            wsdec.skip().context("ws value")?;
            let val_bytes = ws_bytes[vs..wsdec.position()].to_vec();
            if key == 0 {
                // Collect existing vkeywitnesses so we can append
                let mut vdec = Decoder::new(&val_bytes);
                let cnt = vdec.array().context("vkw array")?.unwrap_or(0);
                for _ in 0..cnt {
                    let ws_start2 = vdec.position();
                    vdec.skip().context("vkw entry skip")?;
                    existing_vkwitnesses.push(val_bytes[ws_start2..vdec.position()].to_vec());
                }
            } else {
                ws_entries.push((key, val_bytes));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Encode the new vkeywitness: array(2) [bstr(32), bstr(64)]
    // -----------------------------------------------------------------------
    let new_witness = encode_vkeywitness(vkey, signature);
    existing_vkwitnesses.push(new_witness);

    // Encode key 0 value: array(n) of witnesses
    let key0_val = encode_array_of_bytes(&existing_vkwitnesses);
    ws_entries.push((0, key0_val));
    ws_entries.sort_unstable_by_key(|(k, _)| *k);

    // -----------------------------------------------------------------------
    // Re-encode witness_set
    // -----------------------------------------------------------------------
    let new_ws = encode_map(&ws_entries);

    // -----------------------------------------------------------------------
    // Reassemble tx
    // -----------------------------------------------------------------------
    let mut out = Vec::new();
    out.push(0x84); // array(4)
    out.extend_from_slice(body_bytes);
    out.extend_from_slice(&new_ws);
    out.extend_from_slice(valid_bytes);
    out.extend_from_slice(aux_bytes);
    Ok(out)
}

// ---------------------------------------------------------------------------
// CBOR encoding helpers (minimal, for known-small values)
// ---------------------------------------------------------------------------

fn encode_vkeywitness(vkey: &[u8; 32], sig: &[u8; 64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + 34 + 66);
    out.push(0x82); // array(2)
    cbor_bstr(&mut out, vkey);
    cbor_bstr(&mut out, sig);
    out
}

fn cbor_bstr(out: &mut Vec<u8>, bytes: &[u8]) {
    let n = bytes.len();
    if n <= 23 {
        out.push(0x40 | n as u8);
    } else if n <= 255 {
        out.push(0x58);
        out.push(n as u8);
    } else {
        out.push(0x59);
        out.push((n >> 8) as u8);
        out.push(n as u8);
    }
    out.extend_from_slice(bytes);
}

fn encode_array_of_bytes(items: &[Vec<u8>]) -> Vec<u8> {
    let n = items.len();
    let mut out = Vec::new();
    if n <= 23 {
        out.push(0x80 | n as u8);
    } else if n <= 255 {
        out.push(0x98);
        out.push(n as u8);
    } else {
        out.push(0x99);
        out.push((n >> 8) as u8);
        out.push(n as u8);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn encode_map(entries: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let n = entries.len();
    let mut out = Vec::new();
    if n <= 23 {
        out.push(0xa0 | n as u8);
    } else if n <= 255 {
        out.push(0xb8);
        out.push(n as u8);
    } else {
        out.push(0xb9);
        out.push((n >> 8) as u8);
        out.push(n as u8);
    }
    for (key, val) in entries {
        if *key <= 23 {
            out.push(*key as u8);
        } else if *key <= 255 {
            out.push(0x18);
            out.push(*key as u8);
        } else {
            out.push(0x19);
            out.push((*key >> 8) as u8);
            out.push(*key as u8);
        }
        out.extend_from_slice(val);
    }
    out
}

/// Sign a tx CBOR with a VKey witness derived from the provided key material.
/// Returns the new tx CBOR and the tx hash (for user confirmation).
pub fn sign_tx(
    tx_cbor: &[u8],
    vkey: &[u8; 32],
    signature: &[u8; 64],
) -> Result<(Vec<u8>, Hash<32>)> {
    let body_hash = tx_body_hash(tx_cbor)?;
    let signed = add_vkey_witness(tx_cbor, vkey, signature)?;
    Ok((signed, Hash::from(body_hash)))
}
