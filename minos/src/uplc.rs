/// Apply a Plutus script parameter using the `uplc` crate's flat encoder/decoder.
///
/// Replaces the previous hand-rolled flat UPLC bit-buffer implementation.
use anyhow::Result;
use uplc::ast::{DeBruijn, Program};

#[cfg(test)]
mod tests {
    use super::*;

    const ONESHOT_HEX: &str = "59017e010100229800aba2aba1aba0aab9faab9eaab9dab9a4888888966002646465300130053754003300900398048012444b30013370e9001001c4c8cc892818068009806980700098049baa0048992cc004c00401226644b30013003300b375464660020026eb0c040c034dd5003112cc004006298103d87a80008992cc004cdd7980918079baa001015899ba548000cc0440052f5c1133003003301300240346022002807a264b300130110018acc004cdc39bad3010301130113758602000290014528c5900b45900e19198008009bab30103011301130113011300d375400c44b30010018a5eb82264664466446600400400244b30010018801c4c8cc058dd39980b1ba90053301630130013301630140014bd7019801801980c001180b000a028375660220066eb8c038004cc00c00cc04c008c04400500f45900a1bae300d300a375400ab300130013009375400515980098051baa0028a4d16402d1640211640206e1d2000401c30083009001300800130033754011149a26cac80081";

    #[test]
    fn roundtrip_oneshot() {
        let base = hex::decode(ONESHOT_HEX).unwrap();
        // Minimal param: Constr 0 [bstr(32 zeros), int(0)]
        // CBOR: d87982 + 4120*00 + 00
        let mut param = vec![0xd8, 0x79, 0x82]; // Constr 0, 2 fields
        param.push(0x58);
        param.push(0x20);
        param.extend_from_slice(&[0u8; 32]); // bstr(32)
        param.push(0x00); // int(0)

        let applied_cbor = apply_param(&base, &param).unwrap();

        let mut buf = Vec::new();
        Program::<DeBruijn>::from_cbor(&applied_cbor, &mut buf)
            .expect("applied script must be valid flat UPLC");

        let flat = crate::scripts::script_raw_bytes(&applied_cbor);
        eprintln!(
            "oneshot applied: {} bytes cbor, {} bytes flat",
            applied_cbor.len(),
            flat.len()
        );
        assert_eq!(
            &flat[..3],
            &[0x01, 0x01, 0x00],
            "flat bytes must start with version 1.1.0"
        );
        eprintln!(
            "flat hex (first 40 bytes): {}",
            hex::encode(&flat[..flat.len().min(40)])
        );
    }

    #[test]
    fn verify_hash_format() {
        // Known working script from preview (instance 573af8c03...)
        // Koios script_info returns these bytes for the oneshot script:
        // script_hash = 573af8c03f1120ac874bf4632d1d570c61e6beace29899b34cf32f0d
        let koios_hex = "5901aa0101003229800aba2aba1aba0aab9faab9eaab9dab9a4888888966002646465300130053754003300900398048012444b30013370e9001001c4c8cc892818068009806980700098049baa0048992cc004c00401226644b30013003300b375464660020026eb0c040c034dd5003112cc0040062980103d87a80008992cc004cdd7980918079baa001015899ba548000cc0440052f5c1133003003301300240346022002807a264b300130110018acc004cdc39bad3010301130113758602000290014528c5900b45900e19198008009bab30103011301130113011300d375400c44b30010018a5eb82264664466446600400400244b30010018801c4c8cc058dd39980b1ba90053301630130013301630140014bd7019801801980c001180b000a028375660220066eb8c038004cc00c00cc04c008c04400500f45900a1bae300d300a375400ab300130013009375400515980098051baa0028a4d16402d1640211640206e1d2000401c30083009001300800130033754011149a26cac800898127d8799f5820a93769a6f696bc2e1b85f90afb547dcbdd178599681b8ac87497b363c53bee2102ff0001";
        let expected_hash = "573af8c03f1120ac874bf4632d1d570c61e6beace29899b34cf32f0d";

        let cbor_bytes = hex::decode(koios_hex).unwrap();
        eprintln!(
            "Koios bytes: {} total, starts with {:02x?}",
            cbor_bytes.len(),
            &cbor_bytes[..3]
        );

        // The first 3 bytes should be the CBOR bytestring header: 59 01 aa
        assert_eq!(
            &cbor_bytes[..3],
            &[0x59, 0x01, 0xaa],
            "should be CBOR bstr header"
        );
        let flat_bytes = &cbor_bytes[3..];
        eprintln!(
            "Flat bytes: {}, starts with {:02x?}",
            flat_bytes.len(),
            &flat_bytes[..3]
        );

        // Compute hash of CBOR-wrapped bytes (tag=3)
        use pallas_crypto::hash::Hasher;
        let hash_cbor: pallas_crypto::hash::Hash<28> = Hasher::<224>::hash_tagged(&cbor_bytes, 3);
        let hash_flat: pallas_crypto::hash::Hash<28> = Hasher::<224>::hash_tagged(flat_bytes, 3);

        eprintln!("Hash of CBOR-wrapped: {}", hex::encode(*hash_cbor));
        eprintln!("Hash of raw flat:     {}", hex::encode(*hash_flat));
        eprintln!("Expected hash:        {}", expected_hash);

        let cbor_matches = hex::encode(*hash_cbor) == expected_hash;
        let flat_matches = hex::encode(*hash_flat) == expected_hash;
        eprintln!("CBOR-wrapped matches: {}", cbor_matches);
        eprintln!("Raw flat matches:     {}", flat_matches);

        // One of them must match
        assert!(cbor_matches || flat_matches, "neither hash format matches!");
    }

    #[test]
    fn real_param_encoding() {
        use crate::scripts;
        // Simulate what parameterize_oneshot does with a real seed UTxO
        let seed_hash = [0xabu8; 32];
        let applied_cbor = scripts::parameterize_oneshot(&seed_hash, 0).unwrap();

        // The result must be valid flat UPLC
        let mut buf = Vec::new();
        Program::<DeBruijn>::from_cbor(&applied_cbor, &mut buf)
            .expect("parameterize_oneshot result must be valid flat UPLC");

        let flat = scripts::script_raw_bytes(&applied_cbor);
        eprintln!("parameterized oneshot: {} flat bytes", flat.len());
        assert_eq!(&flat[..3], &[0x01, 0x01, 0x00], "version must be 1.1.0");

        // First nibble of the term byte must be 0011 (Apply tag)
        let first_term_nibble = (flat[3] >> 4) & 0xf;
        eprintln!(
            "first term nibble: {:04b} (should be 0011 for Apply)",
            first_term_nibble
        );
    }
}

/// Apply a single PlutusData-encoded parameter to a compiled Plutus V3 script.
///
/// `base_script_cbor` — the outer CBOR-wrapped flat bytes (from plutus.json `compiledCode`,
///                       hex-decoded).
/// `param_cbor`       — the CBOR encoding of the parameter (PlutusData).
///
/// Returns the new outer CBOR-wrapped flat bytes.
pub fn apply_param(base_script_cbor: &[u8], param_cbor: &[u8]) -> Result<Vec<u8>> {
    let mut flat_buf = Vec::new();
    let program = Program::<DeBruijn>::from_cbor(base_script_cbor, &mut flat_buf)
        .map_err(|e| anyhow::anyhow!("uplc decode: {e}"))?;

    let param =
        uplc::plutus_data(param_cbor).map_err(|e| anyhow::anyhow!("plutus_data decode: {e}"))?;

    let applied = program.apply_data(param);

    applied
        .to_cbor()
        .map_err(|e| anyhow::anyhow!("uplc encode: {e}"))
}
