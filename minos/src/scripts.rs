/// Treasury contract scripts — base compiled code and parameterization.
///
/// Base scripts from `plutus.json` in the SundaeSwap treasury-contracts repository.
/// Each script is the outer CBOR-encoded flat-UPLC bytes (as shipped in compiledCode).
use anyhow::Result;
use pallas_addresses::{
    Address, Network, ShelleyAddress, ShelleyDelegationPart, ShelleyPaymentPart,
};
use pallas_codec::minicbor;
use pallas_codec::utils::MaybeIndefArray;
use pallas_crypto::hash::{Hash, Hasher};
use pallas_primitives::conway::{BigInt, BoundedBytes, Constr, PlutusData};

use crate::types::MultisigScript;

// ---------------------------------------------------------------------------
// Base compiled scripts (from plutus.json, compiledCode field, hex-decoded)
// ---------------------------------------------------------------------------

const ONESHOT_HEX: &str = include_str!("contracts/oneshot.hex");

const TREASURY_HEX: &str = include_str!("contracts/treasury.hex");

const VENDOR_HEX: &str = include_str!("contracts/vendor.hex");

/// Decode the outer CBOR bytestring to get the raw flat UPLC bytes.
/// Cardano stores Plutus scripts as a CBOR bytestring; the VALUE (inner bytes)
/// is the flat-encoded UPLC program used for hashing and witness submission.
pub fn script_raw_bytes(outer_cbor: &[u8]) -> Vec<u8> {
    let flat: pallas_codec::minicbor::bytes::ByteVec =
        pallas_codec::minicbor::decode(outer_cbor).expect("script CBOR decode");
    flat.to_vec()
}

/// Compute the Plutus V3 script hash: blake2b-224(0x03 || outer_cbor_bytes)
///
/// Cardano hashes the CBOR-wrapped flat bytes (the full `59xxxx flat` encoding),
/// NOT the raw flat bytes. The node's deserialiseScript also CBOR-decodes the
/// bytes before flat-decoding, so the CBOR wrapper must be preserved everywhere.
pub fn script_hash(outer_cbor: &[u8]) -> Hash<28> {
    Hasher::<224>::hash_tagged(outer_cbor, 3)
}

// ---------------------------------------------------------------------------
// Plutus data encoding for script parameters
// ---------------------------------------------------------------------------

fn int_data(n: i128) -> PlutusData {
    use pallas_codec::utils::Int;
    PlutusData::BigInt(BigInt::Int(Int::try_from(n).expect("int fits i128")))
}

fn bytes_data(b: Vec<u8>) -> PlutusData {
    PlutusData::BoundedBytes(BoundedBytes::from(b))
}

fn constr(tag: u64, fields: Vec<PlutusData>) -> PlutusData {
    PlutusData::Constr(Constr {
        tag,
        any_constructor: None,
        fields: MaybeIndefArray::Def(fields),
    })
}

/// Encode OutputReference { transaction_id: ByteArray(32), output_index: Int }
pub fn encode_output_reference(tx_hash: &[u8; 32], output_index: u64) -> Vec<u8> {
    let data = constr(
        121,
        vec![bytes_data(tx_hash.to_vec()), int_data(output_index as i128)],
    );
    minicbor::to_vec(data).expect("OutputReference encode")
}

/// Permissions for all treasury actions.
pub struct TreasuryPermissions {
    pub reorganize: MultisigScript,
    pub sweep: MultisigScript,
    pub fund: MultisigScript,
    pub disburse: MultisigScript,
}

/// Permissions for all vendor actions.
pub struct VendorPermissions {
    pub pause: MultisigScript,
    pub resume: MultisigScript,
    pub modify: MultisigScript,
}

fn encode_multisig(m: &MultisigScript) -> PlutusData {
    match m {
        MultisigScript::Signature { key_hash } => constr(121, vec![bytes_data(key_hash.clone())]),
        MultisigScript::AllOf { scripts } => constr(
            122,
            vec![PlutusData::Array(MaybeIndefArray::Def(
                scripts.iter().map(encode_multisig).collect(),
            ))],
        ),
        MultisigScript::AnyOf { scripts } => constr(
            123,
            vec![PlutusData::Array(MaybeIndefArray::Def(
                scripts.iter().map(encode_multisig).collect(),
            ))],
        ),
        MultisigScript::AtLeast { required, scripts } => constr(
            124,
            vec![
                int_data(*required as i128),
                PlutusData::Array(MaybeIndefArray::Def(
                    scripts.iter().map(encode_multisig).collect(),
                )),
            ],
        ),
        MultisigScript::Before { time } => constr(125, vec![int_data(*time as i128)]),
        MultisigScript::After { time } => constr(126, vec![int_data(*time as i128)]),
        MultisigScript::Script { script_hash } => {
            constr(127, vec![bytes_data(script_hash.clone())])
        }
    }
}

/// Encode TreasuryConfiguration as PlutusData CBOR bytes.
pub fn encode_treasury_config(
    registry_token: &[u8; 28],
    permissions: &TreasuryPermissions,
    expiration_ms: u64,
    payout_upperbound: u64,
) -> Vec<u8> {
    let perms = constr(
        121,
        vec![
            encode_multisig(&permissions.reorganize),
            encode_multisig(&permissions.sweep),
            encode_multisig(&permissions.fund),
            encode_multisig(&permissions.disburse),
        ],
    );
    let data = constr(
        121,
        vec![
            bytes_data(registry_token.to_vec()),
            perms,
            int_data(expiration_ms as i128),
            int_data(payout_upperbound as i128),
        ],
    );
    minicbor::to_vec(data).expect("TreasuryConfiguration encode")
}

/// Encode VendorConfiguration as PlutusData CBOR bytes.
pub fn encode_vendor_config(
    registry_token: &[u8; 28],
    permissions: &VendorPermissions,
    expiration_ms: u64,
) -> Vec<u8> {
    let perms = constr(
        121,
        vec![
            encode_multisig(&permissions.pause),
            encode_multisig(&permissions.resume),
            encode_multisig(&permissions.modify),
        ],
    );
    let data = constr(
        121,
        vec![
            bytes_data(registry_token.to_vec()),
            perms,
            int_data(expiration_ms as i128),
        ],
    );
    minicbor::to_vec(data).expect("VendorConfiguration encode")
}

/// Encode ScriptHashRegistry datum for the registry UTxO.
pub fn encode_script_hash_registry(treasury_hash: &[u8; 28], vendor_hash: &[u8; 28]) -> Vec<u8> {
    // ScriptHashRegistry = Constructor 0 { treasury: Credential, vendor: Credential }
    // Credential::Script = Constructor 1 [hash]
    let treasury_cred = constr(122, vec![bytes_data(treasury_hash.to_vec())]);
    let vendor_cred = constr(122, vec![bytes_data(vendor_hash.to_vec())]);
    let data = constr(121, vec![treasury_cred, vendor_cred]);
    minicbor::to_vec(data).expect("ScriptHashRegistry encode")
}

// ---------------------------------------------------------------------------
// Parameterized script builders
// ---------------------------------------------------------------------------

/// Apply the seed UTxO to the oneshot minting policy.
pub fn parameterize_oneshot(seed_tx: &[u8; 32], seed_index: u64) -> Result<Vec<u8>> {
    let base = hex::decode(ONESHOT_HEX.trim()).expect("oneshot hex");
    let param = encode_output_reference(seed_tx, seed_index);
    crate::uplc::apply_param(&base, &param)
}

/// Apply TreasuryConfiguration to the treasury spending script.
pub fn parameterize_treasury(
    registry_token: &[u8; 28],
    permissions: &TreasuryPermissions,
    expiration_ms: u64,
    payout_upperbound: u64,
) -> Result<Vec<u8>> {
    let base = hex::decode(TREASURY_HEX.trim()).expect("treasury hex");
    let param = encode_treasury_config(
        registry_token,
        permissions,
        expiration_ms,
        payout_upperbound,
    );
    crate::uplc::apply_param(&base, &param)
}

/// Apply VendorConfiguration to the vendor spending script.
pub fn parameterize_vendor(
    registry_token: &[u8; 28],
    permissions: &VendorPermissions,
    expiration_ms: u64,
) -> Result<Vec<u8>> {
    let base = hex::decode(VENDOR_HEX.trim()).expect("vendor hex");
    let param = encode_vendor_config(registry_token, permissions, expiration_ms);
    crate::uplc::apply_param(&base, &param)
}

// ---------------------------------------------------------------------------
// Address helpers
// ---------------------------------------------------------------------------

/// Build an enterprise script address (no staking) from a script hash.
pub fn enterprise_script_addr(hash: &Hash<28>, network_id: u8) -> Address {
    let network = if network_id == 1 {
        Network::Mainnet
    } else {
        Network::Testnet
    };
    Address::Shelley(ShelleyAddress::new(
        network,
        ShelleyPaymentPart::script_hash(*hash),
        ShelleyDelegationPart::Null,
    ))
}

/// Build a base script address where both payment and staking use the same script hash.
pub fn base_script_addr(hash: &Hash<28>, network_id: u8) -> Address {
    let network = if network_id == 1 {
        Network::Mainnet
    } else {
        Network::Testnet
    };
    Address::Shelley(ShelleyAddress::new(
        network,
        ShelleyPaymentPart::script_hash(*hash),
        ShelleyDelegationPart::script_hash(*hash),
    ))
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    #[test]
    fn embedded_base_scripts_match_pinned_upstream_blueprint_bytes_and_hashes() {
        let blueprint: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/contracts/blueprint.json"))
                .unwrap();
        for (title, embedded) in [
            ("oneshot.oneshot.mint", ONESHOT_HEX),
            ("treasury.treasury.spend", TREASURY_HEX),
            ("vendor.vendor.spend", VENDOR_HEX),
        ] {
            let validator = blueprint["validators"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["title"] == title)
                .unwrap();
            let expected = hex::decode(validator["compiledCode"].as_str().unwrap()).unwrap();
            assert_eq!(hex::decode(embedded.trim()).unwrap(), expected, "{title}");
            assert_eq!(
                script_hash(&expected).to_string(),
                validator["hash"].as_str().unwrap(),
                "{title}"
            );
        }
        assert_eq!(
            blueprint["preamble"]["compiler"]["version"],
            "v1.1.19+unknown"
        );
        assert_eq!(blueprint["preamble"]["plutusVersion"], "v3");
    }

    #[test]
    fn historical_applied_programs_use_the_pinned_base_and_expected_hashes() {
        use uplc::ast::{DeBruijn, Program, Term};
        let inputs: Vec<crate::koios::Utxo> =
            serde_json::from_str(include_str!("../tests/fixtures/withdrawal/utxos.json")).unwrap();
        for (prefix, base, expected_hash) in [
            (
                "944e78",
                TREASURY_HEX,
                "eb06997a94b339ee0b0dd0de7bfec2a184d1af577586654d44e90558",
            ),
            (
                "c307",
                VENDOR_HEX,
                "5bab145a5e92c865725365bc8fa345abea7f292a12c85c951baf5bfa",
            ),
        ] {
            let input = inputs
                .iter()
                .find(|u| u.tx_hash.starts_with(prefix))
                .unwrap();
            let (language, bytes) = crate::references::script_bytes(input).unwrap();
            assert_eq!(language, 3);
            assert_eq!(script_hash(&bytes).to_string(), expected_hash);
            let applied = Program::<DeBruijn>::from_cbor(&bytes, &mut vec![]).unwrap();
            let base =
                Program::<DeBruijn>::from_cbor(&hex::decode(base.trim()).unwrap(), &mut vec![])
                    .unwrap();
            assert_eq!(applied.version, base.version);
            let Term::Apply { function, .. } = applied.term else {
                panic!("expected one applied configuration")
            };
            assert_eq!(function.as_ref(), &base.term);
        }
    }
}
