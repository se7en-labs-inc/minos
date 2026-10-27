/// Add a VKey witness to an existing (possibly partially witnessed) tx CBOR.
///
/// Cardano Conway tx CBOR: array(4) [body, witness_set, is_valid, aux_data]
/// VKey witnesses live at key 0 in the witness_set map.
use anyhow::{Context, Result};
use pallas_codec::minicbor::Decoder;
use pallas_crypto::hash::{Hash, Hasher};

/// Split a supported envelope without re-encoding any signed fields.
fn transaction_parts(tx_cbor: &[u8]) -> Result<[&[u8]; 4]> {
    crate::cbor::validate(tx_cbor)?;
    let mut decoder = Decoder::new(tx_cbor);
    anyhow::ensure!(
        decoder.array()? == Some(4),
        "expected definite four-item transaction"
    );
    let mut parts = [&[][..]; 4];
    for part in &mut parts {
        let start = decoder.position();
        decoder.skip().context("transaction item")?;
        *part = &tx_cbor[start..decoder.position()];
    }
    anyhow::ensure!(
        decoder.position() == tx_cbor.len(),
        "trailing transaction bytes"
    );
    Ok(parts)
}

/// Attach an airgap witness only after checking its exact body and required signer.
pub fn assemble_witness(
    tx_cbor: &[u8],
    envelope: &str,
    expected_body_hash: &[u8; 32],
    required_signer: &[u8; 28],
) -> Result<Vec<u8>> {
    let mut decoder = Decoder::new(tx_cbor);
    let tx: pallas_primitives::conway::Tx<'_> = decoder.decode()?;
    anyhow::ensure!(
        decoder.position() == tx_cbor.len(),
        "trailing transaction bytes"
    );
    let hash = tx_body_hash(tx_cbor)?;
    anyhow::ensure!(
        &hash == expected_body_hash,
        "unexpected transaction body hash"
    );
    anyhow::ensure!(
        tx.transaction_body
            .required_signers
            .as_ref()
            .is_some_and(|s| s.iter().any(|h| h.as_ref() == required_signer)),
        "signer is not required by transaction body"
    );
    let json: serde_json::Value = serde_json::from_str(envelope)?;
    anyhow::ensure!(
        json["type"] == "TxWitness ConwayEra",
        "expected TxWitness ConwayEra"
    );
    let bytes = hex::decode(
        json["cborHex"]
            .as_str()
            .context("missing witness cborHex")?,
    )?;
    let mut dec = Decoder::new(&bytes);
    anyhow::ensure!(dec.array()? == Some(2), "expected witness pair");
    let vkey: [u8; 32] = dec
        .bytes()?
        .try_into()
        .context("invalid public key length")?;
    let signature: [u8; 64] = dec
        .bytes()?
        .try_into()
        .context("invalid signature length")?;
    anyhow::ensure!(dec.position() == bytes.len(), "trailing witness bytes");
    anyhow::ensure!(
        Hasher::<224>::hash(&vkey).as_ref() == required_signer,
        "wrong witness signer"
    );
    anyhow::ensure!(
        pallas_crypto::key::ed25519::PublicKey::from(vkey).verify(
            hash,
            &pallas_crypto::key::ed25519::Signature::from(signature)
        ),
        "invalid witness over transaction body"
    );
    let assembled = add_vkey_witness(tx_cbor, &vkey, &signature)?;
    let _: pallas_primitives::conway::Tx<'_> = pallas_codec::minicbor::decode(&assembled)?;
    anyhow::ensure!(
        tx_body_hash(&assembled)? == hash,
        "assembly changed transaction body"
    );
    Ok(assembled)
}

/// Compute the tx body hash (blake2b-256 of the body CBOR bytes).
pub fn tx_body_hash(tx_cbor: &[u8]) -> Result<[u8; 32]> {
    let [body_bytes, _, _, _] = transaction_parts(tx_cbor)?;
    Ok(*Hasher::<256>::hash(body_bytes))
}

/// Add a VKey witness (vkey=32 bytes, sig=64 bytes) to a tx CBOR.
/// Existing signed fields and non-key witnesses retain their original bytes.
/// Repeating the same witness is idempotent; conflicting/duplicate keys fail.
/// Indefinite witness maps/sets and non-four-item envelopes are unsupported.
pub fn add_vkey_witness(tx_cbor: &[u8], vkey: &[u8; 32], signature: &[u8; 64]) -> Result<Vec<u8>> {
    use pallas_codec::minicbor::{data::Tag, Encoder};
    let [body_bytes, ws_bytes, valid_bytes, aux_bytes] = transaction_parts(tx_cbor)?;
    let _: pallas_primitives::conway::Tx<'_> =
        pallas_codec::minicbor::decode(tx_cbor).context("invalid Conway transaction")?;

    // -----------------------------------------------------------------------
    // Parse witness_set map, collect existing entries
    // -----------------------------------------------------------------------
    let mut wsdec = Decoder::new(ws_bytes);
    let ws_map_type = wsdec.datatype().context("ws datatype")?;

    let mut ws_entries: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut existing_vkwitnesses: Vec<Vec<u8>> = Vec::new();
    let mut seen_fields = [false; 8];
    let mut seen_vkeys = Vec::new();
    let mut tagged_set = false;
    let mut already_signed = false;

    anyhow::ensure!(
        ws_map_type == pallas_codec::minicbor::data::Type::Map,
        "expected definite witness map"
    );
    if ws_map_type == pallas_codec::minicbor::data::Type::Map {
        let n = wsdec
            .map()
            .context("ws map")?
            .context("indefinite ws map")?;
        for _ in 0..n {
            let key = wsdec.u64().context("ws key")?;
            anyhow::ensure!(key < 8, "unsupported witness field {key}");
            anyhow::ensure!(!seen_fields[key as usize], "duplicate witness field {key}");
            seen_fields[key as usize] = true;
            let vs = wsdec.position();
            wsdec.skip().context("ws value")?;
            let val_bytes = ws_bytes[vs..wsdec.position()].to_vec();
            if key == 0 {
                // Collect existing vkeywitnesses so we can append
                let mut vdec = Decoder::new(&val_bytes);
                if vdec.datatype()? == pallas_codec::minicbor::data::Type::Tag {
                    anyhow::ensure!(vdec.tag()?.as_u64() == 258, "unexpected witness set tag");
                    tagged_set = true;
                }
                let cnt = vdec
                    .array()
                    .context("vkw array")?
                    .context("indefinite vkey set unsupported")?;
                for _ in 0..cnt {
                    let ws_start2 = vdec.position();
                    anyhow::ensure!(vdec.array()? == Some(2), "expected vkey witness pair");
                    let old_key: [u8; 32] =
                        vdec.bytes()?.try_into().context("invalid vkey length")?;
                    let old_signature: [u8; 64] = vdec
                        .bytes()?
                        .try_into()
                        .context("invalid signature length")?;
                    anyhow::ensure!(!seen_vkeys.contains(&old_key), "duplicate vkey witness");
                    seen_vkeys.push(old_key);
                    if &old_key == vkey {
                        anyhow::ensure!(
                            &old_signature == signature,
                            "conflicting signature for existing vkey"
                        );
                        already_signed = true;
                    }
                    existing_vkwitnesses.push(val_bytes[ws_start2..vdec.position()].to_vec());
                }
                anyhow::ensure!(
                    vdec.position() == val_bytes.len(),
                    "trailing vkey set bytes"
                );
            } else {
                ws_entries.push((key, val_bytes));
            }
        }
    }
    if already_signed {
        return Ok(tx_cbor.to_vec());
    }

    // -----------------------------------------------------------------------
    // Encode the new vkeywitness: array(2) [bstr(32), bstr(64)]
    // -----------------------------------------------------------------------
    let mut witness = Encoder::new(Vec::new());
    witness.array(2)?.bytes(vkey)?.bytes(signature)?;
    existing_vkwitnesses.push(witness.into_writer());

    // Encode key 0 value: array(n) of witnesses
    let mut key0 = Encoder::new(Vec::new());
    if tagged_set {
        key0.tag(Tag::new(258))?;
    }
    key0.array(existing_vkwitnesses.len() as u64)?;
    let mut key0_val = key0.into_writer();
    for witness in existing_vkwitnesses {
        key0_val.extend(witness);
    }
    ws_entries.push((0, key0_val));
    ws_entries.sort_unstable_by_key(|(k, _)| *k);

    // -----------------------------------------------------------------------
    // Re-encode witness_set
    // -----------------------------------------------------------------------
    let mut map = Encoder::new(Vec::new());
    map.map(ws_entries.len() as u64)?;
    let mut new_ws = map.into_writer();
    for (key, bytes) in ws_entries {
        let mut field = Encoder::new(Vec::new());
        field.u64(key)?;
        new_ws.extend(field.into_writer());
        new_ws.extend(bytes);
    }

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
