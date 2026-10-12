/// Post-processing utilities for pallas-txbuilder output.
///
/// pallas-txbuilder does not support auxiliary_data or staking withdrawals.
/// We post-process the CBOR 4-element tx array [body, witness_set, valid, null] to:
/// 1. Add key 7 (aux_data_hash) to the body map.
/// 2. Replace the null aux_data field with real aux_data CBOR.
/// 3. (withdraw only) Add key 5 (withdrawals) to body, reward redeemer to witness_set,
///    and recompute key 11 (script_data_hash).
use anyhow::{Context, Result};
use pallas_codec::minicbor::Decoder;
use pallas_crypto::hash::Hasher;

/// Add aux_data to a transaction built by pallas-txbuilder.
///
/// - `tx_cbor`      : raw CBOR from `build_conway_raw().tx_bytes`
/// - `aux_data_cbor`: CBOR-encoded auxiliary data (outer {1694: {...}})
/// - `aux_data_hash`: BLAKE2b-256 of aux_data_cbor
///
/// Returns the reassembled 4-element tx CBOR array with body sorted canonically.
pub fn add_aux_data(
    tx_cbor: &[u8],
    aux_data_cbor: &[u8],
    aux_data_hash: &[u8; 32],
) -> Result<Vec<u8>> {
    let mut dec = Decoder::new(tx_cbor);

    // array(4)
    dec.array().context("tx: outer array")?;

    // Extract body bytes
    let body_start = dec.position();
    dec.skip().context("tx: body skip")?;
    let body_bytes = &tx_cbor[body_start..dec.position()];

    // Extract witness set bytes
    let ws_start = dec.position();
    dec.skip().context("tx: witness_set skip")?;
    let ws_bytes = &tx_cbor[ws_start..dec.position()];

    // Skip `valid` (true)
    let valid_start = dec.position();
    dec.skip().context("tx: valid skip")?;
    let valid_bytes = &tx_cbor[valid_start..dec.position()];

    // Skip null (aux_data placeholder)
    dec.skip().context("tx: null skip")?;

    // -----------------------------------------------------------------------
    // Parse body map and insert key 7 (aux_data_hash)
    // -----------------------------------------------------------------------
    let mut bdec = Decoder::new(body_bytes);
    let n = bdec
        .map()
        .context("body: map header")?
        .context("body: indefinite-length map not supported")?;

    let mut entries: Vec<(u64, Vec<u8>)> = Vec::with_capacity(n as usize + 1);
    for _ in 0..n {
        let key = bdec.u64().context("body: entry key")?;
        let vs = bdec.position();
        bdec.skip().context("body: entry value")?;
        entries.push((key, body_bytes[vs..bdec.position()].to_vec()));
    }

    // Key 7 = auxiliary_data_hash (32 bytes)
    // CBOR encoding of a 32-byte bytestring: 0x5820 followed by the 32 bytes
    let mut hash_cbor = Vec::with_capacity(34);
    hash_cbor.push(0x58);
    hash_cbor.push(0x20);
    hash_cbor.extend_from_slice(aux_data_hash);

    // Remove existing key 7 if present (shouldn't be, but defensive)
    entries.retain(|(k, _)| *k != 7);
    entries.push((7, hash_cbor));
    entries.sort_unstable_by_key(|(k, _)| *k);

    // -----------------------------------------------------------------------
    // Re-encode body map canonically
    // -----------------------------------------------------------------------
    let m = entries.len();
    let mut canonical_body = Vec::with_capacity(body_bytes.len() + 36);
    if m <= 23 {
        canonical_body.push(0xa0 | m as u8);
    } else {
        canonical_body.push(0xb8);
        canonical_body.push(m as u8);
    }
    for (key, val) in &entries {
        if *key <= 23 {
            canonical_body.push(*key as u8);
        } else if *key <= 255 {
            canonical_body.push(0x18);
            canonical_body.push(*key as u8);
        } else {
            canonical_body.push(0x19);
            canonical_body.push((*key >> 8) as u8);
            canonical_body.push(*key as u8);
        }
        canonical_body.extend_from_slice(val);
    }

    // -----------------------------------------------------------------------
    // Reassemble: array(4) body witness valid aux_data
    // -----------------------------------------------------------------------
    let mut out = Vec::with_capacity(
        1 + canonical_body.len() + ws_bytes.len() + valid_bytes.len() + aux_data_cbor.len(),
    );
    out.push(0x84); // array(4)
    out.extend_from_slice(&canonical_body);
    out.extend_from_slice(ws_bytes);
    out.extend_from_slice(valid_bytes);
    out.extend_from_slice(aux_data_cbor);

    Ok(out)
}

// ---------------------------------------------------------------------------
// Treasury staking withdrawal
// ---------------------------------------------------------------------------

/// Post-process a tx (already had aux_data added) to insert a 0-ADA treasury
/// staking withdrawal.  Updates:
/// - Body key 5  (withdrawals)
/// - Body key 11 (script_data_hash — recomputed with reward redeemer included)
/// - Witness set key 5 (redeemers — reward redeemer appended)
///
/// The reward redeemer data is `d87980` (Constr 0 []).
/// ExUnits are the caller's estimate for the treasury staking script.
pub fn add_treasury_withdrawal(
    tx_cbor: &[u8],
    treasury_script_hash: &[u8; 28],
    network_id: u8,
    reward_mem: u64,
    reward_steps: u64,
    v3_cost_model: &[i64],
) -> Result<Vec<u8>> {
    let mut dec = Decoder::new(tx_cbor);
    dec.array().context("tx: outer array")?;

    let body_start = dec.position();
    dec.skip().context("tx: body")?;
    let body_bytes = &tx_cbor[body_start..dec.position()];

    let ws_start = dec.position();
    dec.skip().context("tx: witness_set")?;
    let ws_bytes = &tx_cbor[ws_start..dec.position()];

    let valid_start = dec.position();
    dec.skip().context("tx: valid")?;
    let valid_bytes = &tx_cbor[valid_start..dec.position()];

    let aux_start = dec.position();
    dec.skip().context("tx: aux_data")?;
    let aux_bytes = &tx_cbor[aux_start..dec.position()];

    // -------------------------------------------------------------------------
    // 1. Parse and update witness set (add reward redeemer to key 5)
    // -------------------------------------------------------------------------
    let mut wdec = Decoder::new(ws_bytes);
    let ws_n = wdec
        .map()
        .context("ws: map")?
        .context("ws: indefinite map not supported")? as usize;

    let mut ws_entries: Vec<(u64, Vec<u8>)> = Vec::with_capacity(ws_n);
    for _ in 0..ws_n {
        let key = wdec.u64().context("ws: key")?;
        let vs = wdec.position();
        wdec.skip().context("ws: value")?;
        ws_entries.push((key, ws_bytes[vs..wdec.position()].to_vec()));
    }

    let redeemer_idx = ws_entries
        .iter()
        .position(|(k, _)| *k == 5)
        .context("witness_set has no redeemers (key 5)")?;
    let old_redeemers = ws_entries[redeemer_idx].1.clone();
    let new_redeemers = append_reward_redeemer(&old_redeemers, reward_mem, reward_steps)?;
    ws_entries[redeemer_idx].1 = new_redeemers.clone();

    // -------------------------------------------------------------------------
    // 2. Compute new script_data_hash: blake2b-256(redeemers || language_views)
    // -------------------------------------------------------------------------
    let language_views = encode_v3_language_views(v3_cost_model);
    let script_data_hash = {
        let mut input = Vec::with_capacity(new_redeemers.len() + language_views.len());
        input.extend_from_slice(&new_redeemers);
        input.extend_from_slice(&language_views);
        Hasher::<256>::hash(&input)
    };

    // -------------------------------------------------------------------------
    // 3. Parse body and update key 5 (withdrawals) + key 11 (script_data_hash)
    // -------------------------------------------------------------------------
    let mut bdec = Decoder::new(body_bytes);
    let body_n = bdec
        .map()
        .context("body: map")?
        .context("body: indefinite map not supported")? as usize;

    let mut body_entries: Vec<(u64, Vec<u8>)> = Vec::with_capacity(body_n + 2);
    for _ in 0..body_n {
        let key = bdec.u64().context("body: key")?;
        let vs = bdec.position();
        bdec.skip().context("body: value")?;
        body_entries.push((key, body_bytes[vs..bdec.position()].to_vec()));
    }

    // Key 5: withdrawals = {treasury_reward_addr: 0}
    body_entries.retain(|(k, _)| *k != 5);
    body_entries.push((5, encode_withdrawals(treasury_script_hash, network_id)));

    // Key 11: script_data_hash (32-byte bstr: 0x5820 + hash)
    let mut sdh_cbor = Vec::with_capacity(34);
    sdh_cbor.push(0x58);
    sdh_cbor.push(0x20);
    sdh_cbor.extend_from_slice(script_data_hash.as_ref());
    body_entries.retain(|(k, _)| *k != 11);
    body_entries.push((11, sdh_cbor));

    body_entries.sort_unstable_by_key(|(k, _)| *k);

    // -------------------------------------------------------------------------
    // 4. Re-encode and reassemble
    // -------------------------------------------------------------------------
    let canonical_body = encode_cbor_uint_map(&body_entries);
    ws_entries.sort_unstable_by_key(|(k, _)| *k);
    let canonical_ws = encode_cbor_uint_map(&ws_entries);

    let mut out = Vec::new();
    out.push(0x84); // array(4)
    out.extend_from_slice(&canonical_body);
    out.extend_from_slice(&canonical_ws);
    out.extend_from_slice(valid_bytes);
    out.extend_from_slice(aux_bytes);
    Ok(out)
}

// ---------------------------------------------------------------------------
// CBOR helpers
// ---------------------------------------------------------------------------

/// Encode a map with u64 keys in canonical order (keys already sorted by caller).
fn encode_cbor_uint_map(entries: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let n = entries.len();
    let mut buf = Vec::new();
    if n <= 23 {
        buf.push(0xa0 | n as u8);
    } else if n <= 0xff {
        buf.push(0xb8);
        buf.push(n as u8);
    } else {
        buf.push(0xb9);
        buf.push((n >> 8) as u8);
        buf.push(n as u8);
    }
    for (key, val) in entries {
        cbor_uint(*key, &mut buf);
        buf.extend_from_slice(val);
    }
    buf
}

fn cbor_uint(n: u64, buf: &mut Vec<u8>) {
    if n <= 23 {
        buf.push(n as u8);
    } else if n <= 0xff {
        buf.push(0x18);
        buf.push(n as u8);
    } else if n <= 0xffff {
        buf.push(0x19);
        buf.push((n >> 8) as u8);
        buf.push(n as u8);
    } else if n <= 0xffff_ffff {
        buf.push(0x1a);
        buf.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        buf.push(0x1b);
        buf.extend_from_slice(&n.to_be_bytes());
    }
}

fn cbor_i64(n: i64, buf: &mut Vec<u8>) {
    if n >= 0 {
        cbor_uint(n as u64, buf);
    } else {
        let m = (-1 - n) as u64;
        if m <= 23 {
            buf.push(0x20 | m as u8);
        } else if m <= 0xff {
            buf.push(0x38);
            buf.push(m as u8);
        } else if m <= 0xffff {
            buf.push(0x39);
            buf.push((m >> 8) as u8);
            buf.push(m as u8);
        } else if m <= 0xffff_ffff {
            buf.push(0x3a);
            buf.extend_from_slice(&(m as u32).to_be_bytes());
        } else {
            buf.push(0x3b);
            buf.extend_from_slice(&m.to_be_bytes());
        }
    }
}

/// Append a reward redeemer to the existing redeemers CBOR.
///
/// pallas-txbuilder encodes redeemers as a List (array of 4-tuples):
///   `[[tag, index, data, [mem, steps]], ...]`
/// Each reward entry: `[3, 0, d87980, [mem, steps]]`
///
/// The Conway Map format (`{[tag,index]: [data, ex_units]}`) is also supported
/// if encountered, but pallas-txbuilder 1.4.0 always emits List format.
fn append_reward_redeemer(old: &[u8], mem: u64, steps: u64) -> Result<Vec<u8>> {
    // Peek at first byte to determine array vs map encoding
    let first = *old.first().context("empty redeemers")?;
    let is_list = (first & 0xe0) == 0x80; // major type 4 = array

    if is_list {
        // List format: array of 4-tuple arrays
        let mut dec = Decoder::new(old);
        let n = dec
            .array()
            .context("redeemers: outer array")?
            .context("redeemers: indefinite array not supported")? as usize;

        let mut entries: Vec<Vec<u8>> = Vec::with_capacity(n + 1);
        for _ in 0..n {
            let s = dec.position();
            dec.skip().context("redeemer: entry")?;
            entries.push(old[s..dec.position()].to_vec());
        }

        // New reward entry: array(4) [3, 0, d87980, array(2) [mem, steps]]
        let mut reward_entry = vec![0x84]; // array(4)
        reward_entry.push(0x03); // tag = 3 (Rewarding)
        reward_entry.push(0x00); // index = 0
        reward_entry.extend_from_slice(&[0xd8, 0x79, 0x80]); // d87980 (Constr 0 [])
        reward_entry.push(0x82); // array(2) for ExUnits
        cbor_uint(mem, &mut reward_entry);
        cbor_uint(steps, &mut reward_entry);

        entries.push(reward_entry);

        let total = entries.len();
        let mut buf = Vec::new();
        if total <= 23 {
            buf.push(0x80 | total as u8);
        } else if total <= 0xff {
            buf.push(0x98);
            buf.push(total as u8);
        } else {
            buf.push(0x99);
            buf.push((total >> 8) as u8);
            buf.push(total as u8);
        }
        for e in &entries {
            buf.extend_from_slice(e);
        }
        Ok(buf)
    } else {
        // Map format: {[tag,index]: [data, ex_units]}
        let mut dec = Decoder::new(old);
        let n = dec
            .map()
            .context("redeemers: map")?
            .context("redeemers: indefinite map not supported")? as usize;

        let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(n + 1);
        for _ in 0..n {
            let ks = dec.position();
            dec.skip().context("redeemer: key")?;
            let key = old[ks..dec.position()].to_vec();
            let vs = dec.position();
            dec.skip().context("redeemer: val")?;
            let val = old[vs..dec.position()].to_vec();
            entries.push((key, val));
        }

        // Reward redeemer key: array(2) [3, 0] = 82 03 00
        let reward_key = vec![0x82, 0x03, 0x00];
        let mut reward_val = vec![0x82];
        reward_val.extend_from_slice(&[0xd8, 0x79, 0x80]);
        reward_val.push(0x82);
        cbor_uint(mem, &mut reward_val);
        cbor_uint(steps, &mut reward_val);
        entries.push((reward_key, reward_val));

        entries.sort_unstable_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then(a.cmp(b)));

        let total = entries.len();
        let mut buf = Vec::new();
        if total <= 23 {
            buf.push(0xa0 | total as u8);
        } else {
            buf.push(0xb8);
            buf.push(total as u8);
        }
        for (k, v) in &entries {
            buf.extend_from_slice(k);
            buf.extend_from_slice(v);
        }
        Ok(buf)
    }
}

/// Encode the treasury script reward address as a withdrawals map `{addr: 0}`.
fn encode_withdrawals(treasury_script_hash: &[u8; 28], network_id: u8) -> Vec<u8> {
    // Reward address header: 0xF1 (mainnet script) or 0xF0 (testnet script)
    let header = if network_id == 1 { 0xF1u8 } else { 0xF0u8 };
    let mut buf = vec![0xa1]; // map(1)
    buf.push(0x58);
    buf.push(29); // bstr(29)
    buf.push(header);
    buf.extend_from_slice(treasury_script_hash);
    buf.push(0x00); // 0 lovelace
    buf
}

/// Encode PlutusV3 language_views as `{2: [cost_model_values...]}`.
/// Key 2 = PlutusV3 in the Conway language_views map.
fn encode_v3_language_views(cost_model: &[i64]) -> Vec<u8> {
    let mut buf = vec![0xa1, 0x02]; // map(1), key=2
    let n = cost_model.len();
    if n <= 23 {
        buf.push(0x80 | n as u8);
    } else if n <= 0xff {
        buf.push(0x98);
        buf.push(n as u8);
    } else {
        buf.push(0x99);
        buf.push((n >> 8) as u8);
        buf.push(n as u8);
    }
    for &v in cost_model {
        cbor_i64(v, &mut buf);
    }
    buf
}
