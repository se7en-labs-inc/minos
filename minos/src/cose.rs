//! The CIP-30 subset of CIP-8: un-hashed payload, Ed25519, no external AAD.
use anyhow::{ensure, Context, Result};
use pallas_codec::minicbor::{Decoder, Encoder};
use pallas_crypto::{
    hash::Hasher,
    key::ed25519::{PublicKey, Signature},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSignature {
    pub key: String,
    pub signature: String,
}

fn protected(address: &[u8]) -> Result<Vec<u8>> {
    let mut e = Encoder::new(Vec::new());
    e.map(2)?.i8(1)?.i8(-8)?.str("address")?.bytes(address)?;
    Ok(e.into_writer())
}

pub fn signing_bytes(payload: &[u8], address: &[u8]) -> Result<Vec<u8>> {
    let mut e = Encoder::new(Vec::new());
    e.array(4)?
        .str("Signature1")?
        .bytes(&protected(address)?)?
        .bytes(&[])?
        .bytes(payload)?;
    Ok(e.into_writer())
}

pub fn encode(
    payload: &[u8],
    address: &[u8],
    vkey: &[u8; 32],
    signature: &[u8; 64],
) -> Result<DataSignature> {
    let mut key = Encoder::new(Vec::new());
    key.map(4)?
        .i8(1)?
        .i8(1)?
        .i8(3)?
        .i8(-8)?
        .i8(-1)?
        .i8(6)?
        .i8(-2)?
        .bytes(vkey)?;
    let mut sign1 = Encoder::new(Vec::new());
    sign1
        .array(4)?
        .bytes(&protected(address)?)?
        .map(1)?
        .str("hashed")?
        .bool(false)?
        .bytes(payload)?
        .bytes(signature)?;
    Ok(DataSignature {
        key: hex::encode(key.into_writer()),
        signature: hex::encode(sign1.into_writer()),
    })
}

/// Verify our narrowly specified COSE profile, including address/key binding.
pub fn verify(value: &DataSignature, payload: &[u8], address: &[u8]) -> Result<[u8; 32]> {
    let key = hex::decode(&value.key)?;
    let mut d = Decoder::new(&key);
    ensure!(d.map()? == Some(4), "expected COSE_Key map");
    ensure!(d.i8()? == 1 && d.i8()? == 1, "expected OKP key");
    ensure!(d.i8()? == 3 && d.i8()? == -8, "expected EdDSA algorithm");
    ensure!(d.i8()? == -1 && d.i8()? == 6, "expected Ed25519 curve");
    ensure!(d.i8()? == -2, "missing public key");
    let vkey: [u8; 32] = d.bytes()?.try_into().context("invalid COSE public key")?;
    ensure!(d.position() == key.len(), "trailing COSE key bytes");
    ensure!(
        address.len() == 29 && matches!(address[0], 0x60 | 0x61),
        "expected enterprise key address"
    );
    ensure!(
        Hasher::<224>::hash(&vkey).as_ref() == &address[1..],
        "COSE address/key mismatch"
    );
    let bytes = hex::decode(&value.signature)?;
    let mut d = Decoder::new(&bytes);
    ensure!(d.array()? == Some(4), "expected COSE_Sign1");
    ensure!(
        d.bytes()? == protected(address)?,
        "unexpected protected headers"
    );
    ensure!(
        d.map()? == Some(1) && d.str()? == "hashed" && !d.bool()?,
        "expected unhashed payload"
    );
    ensure!(d.bytes()? == payload, "COSE payload mismatch");
    let sig: [u8; 64] = d.bytes()?.try_into().context("invalid COSE signature")?;
    ensure!(d.position() == bytes.len(), "trailing COSE signature bytes");
    ensure!(
        PublicKey::from(vkey).verify(&signing_bytes(payload, address)?, &Signature::from(sig)),
        "invalid COSE signature"
    );
    Ok(vkey)
}
