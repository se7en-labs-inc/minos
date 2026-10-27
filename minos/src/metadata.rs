//! Strict, bounded interpretation of the pinned treasury metadata permission tree.
use crate::types::MultisigScript;

/// Decode a bounded Conway envelope and verify auxiliary-data binding.
pub fn envelope(bytes: &[u8]) -> Result<pallas_primitives::conway::Tx<'_>> {
    use pallas_codec::{minicbor::Decoder, utils::Nullable};
    use pallas_crypto::hash::Hasher;
    crate::cbor::validate(bytes)?;
    let mut decoder = Decoder::new(bytes);
    let tx: pallas_primitives::conway::Tx<'_> =
        decoder.decode().context("invalid Conway transaction")?;
    ensure!(
        decoder.position() == bytes.len(),
        "trailing transaction bytes"
    );
    match (&tx.auxiliary_data, &tx.transaction_body.auxiliary_data_hash) {
        (Nullable::Some(aux), Some(expected)) => ensure!(
            Hasher::<256>::hash(aux.raw_cbor()) == *expected,
            "auxiliary data hash mismatch"
        ),
        (Nullable::Null | Nullable::Undefined, None) => {}
        _ => anyhow::bail!("auxiliary data and its body hash must both be present"),
    }
    Ok(tx)
}

pub fn transaction(bytes: &[u8]) -> Result<pallas_primitives::conway::Tx<'_>> {
    let tx = envelope(bytes)?;
    ensure!(tx.success, "transaction is marked script-invalid");
    Ok(tx)
}

pub fn witnessed_authors(
    tx: &pallas_primitives::conway::Tx<'_>,
) -> Result<std::collections::BTreeSet<String>> {
    use pallas_crypto::{
        hash::Hasher,
        key::ed25519::{PublicKey, Signature},
    };
    let body = Hasher::<256>::hash(tx.transaction_body.raw_cbor());
    let mut authors = std::collections::BTreeSet::new();
    for witness in tx
        .transaction_witness_set
        .vkeywitness
        .iter()
        .flat_map(|witnesses| witnesses.iter())
    {
        let key =
            PublicKey::try_from(witness.vkey.as_slice()).context("invalid witness public key")?;
        let signature = Signature::try_from(witness.signature.as_slice())
            .context("invalid witness signature")?;
        ensure!(
            key.verify(body.as_ref(), &signature),
            "invalid witness signature for transaction body"
        );
        ensure!(
            authors.insert(Hasher::<224>::hash(witness.vkey.as_slice()).to_string()),
            "duplicate witness public key"
        );
    }
    Ok(authors)
}

pub fn auxiliary_1694(tx: &pallas_primitives::conway::Tx<'_>) -> Result<Option<Value>> {
    use pallas_primitives::alonzo::AuxiliaryData;
    let pallas_codec::utils::Nullable::Some(aux) = &tx.auxiliary_data else {
        return Ok(None);
    };
    let metadata = match &**aux {
        AuxiliaryData::Shelley(metadata) => metadata.get(&1694),
        AuxiliaryData::ShelleyMa(aux) => aux.transaction_metadata.get(&1694),
        AuxiliaryData::PostAlonzo(aux) => aux
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get(&1694)),
    };
    metadata.map(metadatum).transpose()
}

/// Preserve metadata arrays and reject ambiguous JSON keys rather than discard
/// fields during conversion. Call after bounded transaction decoding.
pub fn metadatum(metadata: &pallas_primitives::Metadatum) -> Result<Value> {
    use pallas_primitives::Metadatum;
    match metadata {
        Metadatum::Map(entries) => {
            let mut object = serde_json::Map::new();
            for (key, value) in entries.iter() {
                let key = match key {
                    Metadatum::Text(key) => key.clone(),
                    Metadatum::Int(key) => i128::from(*key).to_string(),
                    _ => anyhow::bail!("metadata object has an unsupported key"),
                };
                ensure!(
                    !object.contains_key(&key),
                    "duplicate or ambiguous metadata object key"
                );
                object.insert(key, metadatum(value)?);
            }
            Ok(Value::Object(object))
        }
        Metadatum::Array(items) => Ok(Value::Array(
            items.iter().map(metadatum).collect::<Result<Vec<_>>>()?,
        )),
        Metadatum::Int(value) => {
            serde_json::to_value(i128::from(*value)).context("metadata integer exceeds JSON range")
        }
        Metadatum::Bytes(value) => Ok(Value::String(hex::encode(value.as_slice()))),
        Metadatum::Text(value) => Ok(Value::String(value.clone())),
    }
}
use anyhow::{ensure, Context, Result};
use serde_json::Value;

pub fn permission(value: &Value, root: Option<&Value>) -> Result<MultisigScript> {
    parse_permission(value, root, 0, &mut 512)
}

#[derive(Debug)]
pub struct InstanceScripts {
    pub treasury_hash: String,
    pub vendor_hash: String,
    pub expiration_ms: u64,
    pub payout_upperbound_ms: u64,
    pub vendor_expiration_ms: u64,
}

pub fn instance_scripts(body: &Value, instance_id: &str) -> Result<InstanceScripts> {
    use crate::scripts::{
        parameterize_treasury, parameterize_vendor, script_hash, TreasuryPermissions,
        VendorPermissions,
    };
    let permissions = body
        .get("permissions")
        .context("missing instance permissions")?;
    let get = |name: &str| {
        permission(
            permissions
                .get(name)
                .with_context(|| format!("missing {name} permission"))?,
            Some(permissions),
        )
    };
    let treasury = TreasuryPermissions {
        reorganize: get("reorganize")?,
        sweep: get("sweep")?,
        fund: get("fund")?,
        disburse: get("disburse")?,
    };
    let vendor = VendorPermissions {
        pause: get("pause")?,
        resume: get("resume")?,
        modify: get("modify")?,
    };
    let expiration_ms = integer(body.get("expiration").context("missing expiration")?)?;
    let payout_upperbound_ms = integer(
        body.get("payoutUpperbound")
            .or_else(|| body.get("payout_upperbound"))
            .context("missing payout upper bound")?,
    )?;
    let vendor_expiration_ms = integer(
        body.get("vendorExpiration")
            .or_else(|| body.get("vendor_expiration"))
            .context("missing vendor expiration")?,
    )?;
    let registry: [u8; 28] = hex::decode(instance_id)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("instance policy must be 28 bytes"))?;
    Ok(InstanceScripts {
        treasury_hash: script_hash(&parameterize_treasury(
            &registry,
            &treasury,
            expiration_ms,
            payout_upperbound_ms,
        )?)
        .to_string(),
        vendor_hash: script_hash(&parameterize_vendor(
            &registry,
            &vendor,
            vendor_expiration_ms,
        )?)
        .to_string(),
        expiration_ms,
        payout_upperbound_ms,
        vendor_expiration_ms,
    })
}

fn integer(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .context("permission integer must be an unsigned 64-bit value")
}

fn hash(value: &Value) -> Result<Vec<u8>> {
    let bytes = hex::decode(
        value
            .as_str()
            .context("permission hash must be hexadecimal text")?,
    )?;
    ensure!(bytes.len() == 28, "permission hash must be 28 bytes");
    Ok(bytes)
}

fn parse_permission(
    value: &Value,
    root: Option<&Value>,
    depth: usize,
    remaining: &mut usize,
) -> Result<MultisigScript> {
    ensure!(
        depth <= 32 && *remaining > 0,
        "permission tree exceeds depth/node limit or contains an alias cycle"
    );
    *remaining -= 1;
    if value.is_null() {
        return Ok(MultisigScript::AnyOf { scripts: vec![] });
    }
    if let Some(alias) = value.as_str() {
        let target = root
            .and_then(|root| root.get(alias))
            .context("unknown permission alias")?;
        return parse_permission(target, root, depth + 1, remaining);
    }
    let object = value.as_object().context("permission must be an object")?;
    let actions = [
        "signature",
        "script",
        "allOf",
        "anyOf",
        "atLeast",
        "before",
        "after",
    ];
    let present = actions
        .into_iter()
        .filter(|action| object.contains_key(*action))
        .collect::<Vec<_>>();
    ensure!(
        present.len() == 1,
        "permission must contain exactly one supported action"
    );
    let action = present[0];
    let value = &object[action];
    match action {
        "signature" => Ok(MultisigScript::Signature {
            key_hash: hash(
                value
                    .get("keyHash")
                    .or_else(|| value.get("key_hash"))
                    .context("missing signature key hash")?,
            )?,
        }),
        "script" => Ok(MultisigScript::Script {
            script_hash: hash(
                value
                    .get("scriptHash")
                    .or_else(|| value.get("script_hash"))
                    .unwrap_or(value),
            )?,
        }),
        "before" | "after" => {
            let time = integer(value.get("time").unwrap_or(value))?;
            Ok(if action == "before" {
                MultisigScript::Before { time }
            } else {
                MultisigScript::After { time }
            })
        }
        "allOf" | "anyOf" | "atLeast" => {
            let children = value
                .get("scripts")
                .unwrap_or(value)
                .as_array()
                .context("permission scripts must be an array")?;
            ensure!(
                children.len() <= *remaining,
                "permission tree exceeds node limit"
            );
            let scripts = children
                .iter()
                .map(|child| parse_permission(child, root, depth + 1, remaining))
                .collect::<Result<Vec<_>>>()?;
            Ok(match action {
                "allOf" => MultisigScript::AllOf { scripts },
                "anyOf" => MultisigScript::AnyOf { scripts },
                _ => {
                    let required = integer(
                        value
                            .get("required")
                            .context("missing permission threshold")?,
                    )?;
                    ensure!(
                        required <= scripts.len() as u64,
                        "permission threshold exceeds script count"
                    );
                    MultisigScript::AtLeast { required, scripts }
                }
            })
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_pinned_shapes_aliases_null_and_legacy_arrays() {
        let root = json!({"fund": {"signature": {"keyHash": "01".repeat(28)}}, "pause": "fund"});
        assert_eq!(
            permission(&json!("pause"), Some(&root))
                .unwrap()
                .key_hashes(),
            vec![vec![1; 28]]
        );
        for value in [
            json!({"allOf": {"scripts": [root["fund"].clone()]}}),
            json!({"allOf": [root["fund"].clone()]}),
        ] {
            assert_eq!(
                permission(&value, None).unwrap().key_hashes(),
                vec![vec![1; 28]]
            );
        }
        assert_eq!(
            permission(&json!({"script": {"scriptHash": "02".repeat(28)}}), None)
                .unwrap()
                .script_hashes(),
            vec![vec![2; 28]]
        );
        assert!(
            matches!(permission(&Value::Null, None).unwrap(), MultisigScript::AnyOf { scripts } if scripts.is_empty())
        );
        assert!(matches!(
            permission(&json!({"before": {"time": "123"}}), None).unwrap(),
            MultisigScript::Before { time: 123 }
        ));
    }

    #[test]
    fn cycles_invalid_children_ambiguous_actions_and_bad_hashes_fail() {
        let cyclic = json!({"fund": "pause", "pause": "fund"});
        assert!(permission(&json!("fund"), Some(&cyclic)).is_err());
        for value in [
            json!({"allOf": [ {"unknown": true} ]}),
            json!({"signature": {"keyHash": "00"}}),
            json!({"before": 1, "after": 2}),
            json!({"atLeast": {"required": 2, "scripts": []}}),
            json!({"after": -1}),
            json!({"script": "zz"}),
        ] {
            assert!(permission(&value, None).is_err(), "{value}");
        }
        let wide = json!({"allOf": vec![Value::Null; 513]});
        assert!(permission(&wide, None).is_err());
    }
}
