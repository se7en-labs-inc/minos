use anyhow::{bail, Context, Result};
use pallas_codec::minicbor;
use pallas_primitives::conway::PlutusData;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// On-chain Plutus types (mirrors lib/types.ak)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum PayoutStatus {
    Active,
    Paused,
}

#[derive(Debug, Clone)]
pub struct AssetBundle(pub BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, i64>>);

#[derive(Debug, Clone)]
pub struct Payout {
    /// POSIX milliseconds
    pub maturation: u64,
    pub value: AssetBundle,
    pub status: PayoutStatus,
}

#[derive(Debug, Clone)]
pub enum MultisigScript {
    Signature {
        key_hash: Vec<u8>,
    },
    AllOf {
        scripts: Vec<MultisigScript>,
    },
    AnyOf {
        scripts: Vec<MultisigScript>,
    },
    AtLeast {
        required: u64,
        scripts: Vec<MultisigScript>,
    },
    Before {
        time: u64,
    },
    After {
        time: u64,
    },
    Script {
        script_hash: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct VendorDatum {
    pub vendor: MultisigScript,
    pub payouts: Vec<Payout>,
}

impl MultisigScript {
    /// Collect all leaf key hashes reachable from this script.
    pub fn key_hashes(&self) -> Vec<Vec<u8>> {
        match self {
            MultisigScript::Signature { key_hash } => vec![key_hash.clone()],
            MultisigScript::AllOf { scripts } | MultisigScript::AnyOf { scripts } => {
                scripts.iter().flat_map(|s| s.key_hashes()).collect()
            }
            MultisigScript::AtLeast { scripts, .. } => {
                scripts.iter().flat_map(|s| s.key_hashes()).collect()
            }
            MultisigScript::Before { .. }
            | MultisigScript::After { .. }
            | MultisigScript::Script { .. } => vec![],
        }
    }

    /// Collect all leaf script hashes reachable from this script.
    pub fn script_hashes(&self) -> Vec<Vec<u8>> {
        match self {
            MultisigScript::Script { script_hash } => vec![script_hash.clone()],
            MultisigScript::AllOf { scripts } | MultisigScript::AnyOf { scripts } => {
                scripts.iter().flat_map(|s| s.script_hashes()).collect()
            }
            MultisigScript::AtLeast { scripts, .. } => {
                scripts.iter().flat_map(|s| s.script_hashes()).collect()
            }
            MultisigScript::Signature { .. }
            | MultisigScript::Before { .. }
            | MultisigScript::After { .. } => vec![],
        }
    }
}

impl VendorDatum {
    pub fn from_cbor_hex(hex: &str) -> Result<Self> {
        let bytes = hex::decode(hex).context("invalid datum hex")?;
        let data: PlutusData = minicbor::decode(&bytes).context("cbor decode failed")?;
        decode_vendor_datum(&data)
    }

    pub fn matured_payouts(&self, now_ms: u64) -> Vec<&Payout> {
        self.payouts
            .iter()
            .filter(|p| matches!(p.status, PayoutStatus::Active) && p.maturation <= now_ms)
            .collect()
    }

    /// Re-encode to Plutus CBOR bytes (for inline datum on updated vendor output).
    pub fn to_cbor_bytes(&self) -> Vec<u8> {
        let data = encode_vendor_datum(self);
        minicbor::to_vec(data).expect("VendorDatum CBOR encoding failed")
    }
}

// ---------------------------------------------------------------------------
// Plutus data encoders (mirrors decoders above)
// ---------------------------------------------------------------------------

fn encode_vendor_datum(d: &VendorDatum) -> PlutusData {
    use pallas_codec::utils::MaybeIndefArray;
    use pallas_primitives::conway::Constr;
    PlutusData::Constr(Constr {
        tag: 121,
        any_constructor: None,
        fields: MaybeIndefArray::Indef(vec![
            encode_multisig(&d.vendor),
            PlutusData::Array(MaybeIndefArray::Indef(
                d.payouts.iter().map(encode_payout).collect(),
            )),
        ]),
    })
}

fn encode_payout(p: &Payout) -> PlutusData {
    use pallas_codec::utils::{Int, MaybeIndefArray};
    use pallas_primitives::conway::{BigInt, Constr};
    PlutusData::Constr(Constr {
        tag: 121,
        any_constructor: None,
        fields: MaybeIndefArray::Indef(vec![
            PlutusData::BigInt(BigInt::Int(
                Int::try_from(p.maturation as i128).expect("maturation fits i128"),
            )),
            encode_asset_bundle(&p.value),
            encode_payout_status(&p.status),
        ]),
    })
}

fn encode_payout_status(s: &PayoutStatus) -> PlutusData {
    use pallas_codec::utils::MaybeIndefArray;
    use pallas_primitives::conway::Constr;
    let tag: u64 = match s {
        PayoutStatus::Active => 121,
        PayoutStatus::Paused => 122,
    };
    PlutusData::Constr(Constr {
        tag,
        any_constructor: None,
        fields: MaybeIndefArray::Def(vec![]),
    })
}

fn encode_multisig(m: &MultisigScript) -> PlutusData {
    use pallas_codec::utils::{Int, MaybeIndefArray};
    use pallas_primitives::conway::{BigInt, BoundedBytes, Constr};
    match m {
        MultisigScript::Signature { key_hash } => PlutusData::Constr(Constr {
            tag: 121,
            any_constructor: None,
            fields: MaybeIndefArray::Indef(vec![PlutusData::BoundedBytes(BoundedBytes::from(
                key_hash.clone(),
            ))]),
        }),
        MultisigScript::AllOf { scripts } => PlutusData::Constr(Constr {
            tag: 122,
            any_constructor: None,
            fields: MaybeIndefArray::Indef(vec![PlutusData::Array(MaybeIndefArray::Indef(
                scripts.iter().map(encode_multisig).collect(),
            ))]),
        }),
        MultisigScript::AnyOf { scripts } => PlutusData::Constr(Constr {
            tag: 123,
            any_constructor: None,
            fields: MaybeIndefArray::Indef(vec![PlutusData::Array(MaybeIndefArray::Indef(
                scripts.iter().map(encode_multisig).collect(),
            ))]),
        }),
        MultisigScript::AtLeast { required, scripts } => PlutusData::Constr(Constr {
            tag: 124,
            any_constructor: None,
            fields: MaybeIndefArray::Indef(vec![
                PlutusData::BigInt(BigInt::Int(Int::try_from(*required as i128).expect("fits"))),
                PlutusData::Array(MaybeIndefArray::Indef(
                    scripts.iter().map(encode_multisig).collect(),
                )),
            ]),
        }),
        MultisigScript::Before { time } => PlutusData::Constr(Constr {
            tag: 125,
            any_constructor: None,
            fields: MaybeIndefArray::Indef(vec![PlutusData::BigInt(BigInt::Int(
                Int::try_from(*time as i128).expect("fits"),
            ))]),
        }),
        MultisigScript::After { time } => PlutusData::Constr(Constr {
            tag: 126,
            any_constructor: None,
            fields: MaybeIndefArray::Indef(vec![PlutusData::BigInt(BigInt::Int(
                Int::try_from(*time as i128).expect("fits"),
            ))]),
        }),
        MultisigScript::Script { script_hash } => PlutusData::Constr(Constr {
            tag: 127,
            any_constructor: None,
            fields: MaybeIndefArray::Indef(vec![PlutusData::BoundedBytes(BoundedBytes::from(
                script_hash.clone(),
            ))]),
        }),
    }
}

fn encode_asset_bundle(b: &AssetBundle) -> PlutusData {
    use pallas_codec::utils::{Int, KeyValuePairs};
    use pallas_primitives::conway::{BigInt, BoundedBytes};
    let pairs: Vec<(PlutusData, PlutusData)> =
        b.0.iter()
            .map(|(policy, assets)| {
                let inner_pairs: Vec<(PlutusData, PlutusData)> = assets
                    .iter()
                    .map(|(name, qty)| {
                        (
                            PlutusData::BoundedBytes(BoundedBytes::from(name.clone())),
                            PlutusData::BigInt(BigInt::Int(
                                Int::try_from(*qty as i128).expect("qty fits"),
                            )),
                        )
                    })
                    .collect();
                (
                    PlutusData::BoundedBytes(BoundedBytes::from(policy.clone())),
                    PlutusData::Map(KeyValuePairs::Def(inner_pairs)),
                )
            })
            .collect();
    PlutusData::Map(KeyValuePairs::Def(pairs))
}

// ---------------------------------------------------------------------------
// Plutus data decoders
// ---------------------------------------------------------------------------

fn decode_vendor_datum(data: &PlutusData) -> Result<VendorDatum> {
    let PlutusData::Constr(c) = data else {
        bail!("VendorDatum: expected Constr");
    };
    let fields: Vec<&PlutusData> = c.fields.iter().collect();
    if fields.len() != 2 {
        bail!("VendorDatum: expected 2 fields, got {}", fields.len());
    }
    let vendor = decode_multisig(fields[0]).context("VendorDatum.vendor")?;
    let payouts = decode_list(fields[1], decode_payout).context("VendorDatum.payouts")?;
    Ok(VendorDatum { vendor, payouts })
}

fn decode_payout(data: &PlutusData) -> Result<Payout> {
    let PlutusData::Constr(c) = data else {
        bail!("Payout: expected Constr");
    };
    let fields: Vec<&PlutusData> = c.fields.iter().collect();
    if fields.len() != 3 {
        bail!("Payout: expected 3 fields, got {}", fields.len());
    }
    let maturation = decode_int(fields[0]).context("Payout.maturation")? as u64;
    let value = decode_asset_bundle(fields[1]).context("Payout.value")?;
    let status = decode_payout_status(fields[2]).context("Payout.status")?;
    Ok(Payout {
        maturation,
        value,
        status,
    })
}

fn decode_payout_status(data: &PlutusData) -> Result<PayoutStatus> {
    let PlutusData::Constr(c) = data else {
        bail!("PayoutStatus: expected Constr");
    };
    let alt = constr_alternative(c);
    match alt {
        0 => Ok(PayoutStatus::Active),
        1 => Ok(PayoutStatus::Paused),
        _ => bail!("PayoutStatus: unknown constructor {alt}"),
    }
}

fn decode_multisig(data: &PlutusData) -> Result<MultisigScript> {
    let PlutusData::Constr(c) = data else {
        bail!("MultisigScript: expected Constr");
    };
    let fields: Vec<&PlutusData> = c.fields.iter().collect();
    let alt = constr_alternative(c);
    match alt {
        0 => {
            let key_hash = decode_bytes(fields[0]).context("Signature.key_hash")?;
            Ok(MultisigScript::Signature { key_hash })
        }
        1 => {
            let scripts = decode_list(fields[0], decode_multisig)?;
            Ok(MultisigScript::AllOf { scripts })
        }
        2 => {
            let scripts = decode_list(fields[0], decode_multisig)?;
            Ok(MultisigScript::AnyOf { scripts })
        }
        3 => {
            let required = decode_int(fields[0])? as u64;
            let scripts = decode_list(fields[1], decode_multisig)?;
            Ok(MultisigScript::AtLeast { required, scripts })
        }
        4 => {
            let time = decode_int(fields[0])? as u64;
            Ok(MultisigScript::Before { time })
        }
        5 => {
            let time = decode_int(fields[0])? as u64;
            Ok(MultisigScript::After { time })
        }
        6 => {
            let script_hash = decode_bytes(fields[0]).context("Script.script_hash")?;
            Ok(MultisigScript::Script { script_hash })
        }
        _ => bail!("MultisigScript: unknown constructor {alt}"),
    }
}

fn decode_asset_bundle(data: &PlutusData) -> Result<AssetBundle> {
    let PlutusData::Map(pairs) = data else {
        bail!("AssetBundle: expected Map");
    };
    let mut outer = BTreeMap::new();
    for (policy_data, assets_data) in pairs.iter() {
        let policy = decode_bytes(policy_data)?;
        let PlutusData::Map(asset_pairs) = assets_data else {
            bail!("AssetBundle: inner value expected Map");
        };
        let mut inner = BTreeMap::new();
        for (name_data, qty_data) in asset_pairs.iter() {
            let name = decode_bytes(name_data)?;
            let qty = decode_int(qty_data)?;
            inner.insert(name, qty);
        }
        outer.insert(policy, inner);
    }
    Ok(AssetBundle(outer))
}

fn decode_list<T>(data: &PlutusData, f: fn(&PlutusData) -> Result<T>) -> Result<Vec<T>> {
    let PlutusData::Array(arr) = data else {
        bail!("expected Array");
    };
    arr.iter().map(f).collect()
}

fn decode_int(data: &PlutusData) -> Result<i64> {
    match data {
        PlutusData::BigInt(bi) => {
            use pallas_primitives::conway::BigInt;
            match bi {
                BigInt::Int(i) => Ok(i128::from(*i) as i64),
                BigInt::BigUInt(b) => {
                    let n = u64::from_be_bytes(
                        b.iter()
                            .rev()
                            .copied()
                            .chain(std::iter::repeat(0))
                            .take(8)
                            .collect::<Vec<_>>()
                            .try_into()
                            .unwrap(),
                    );
                    Ok(n as i64)
                }
                BigInt::BigNInt(_) => bail!("negative BigInt not expected"),
            }
        }
        _ => bail!("expected BigInt, got {:?}", data),
    }
}

fn decode_bytes(data: &PlutusData) -> Result<Vec<u8>> {
    match data {
        PlutusData::BoundedBytes(b) => Ok(b.to_vec()),
        _ => bail!("expected BoundedBytes"),
    }
}

fn constr_alternative(c: &pallas_primitives::conway::Constr<PlutusData>) -> u64 {
    // Tags 121–127 map to alternatives 0–6; tag 102 uses any_constructor field
    match c.tag {
        121 => 0,
        122 => 1,
        123 => 2,
        124 => 3,
        125 => 4,
        126 => 5,
        127 => 6,
        102 => c.any_constructor.unwrap_or(0),
        t => t,
    }
}

// ---------------------------------------------------------------------------
// CIP-100 metadata types (key 1694)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TransactionMetadata<B> {
    #[serde(rename = "@context")]
    pub context: String,
    #[serde(rename = "hashAlgorithm")]
    pub hash_algorithm: String,
    pub body: B,
    pub instance: String,
    #[serde(rename = "txAuthor")]
    pub tx_author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CompleteBody {
    pub event: String,
    pub milestones: std::collections::HashMap<String, MilestoneCompletion>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MilestoneCompletion {
    pub description: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Evidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(rename = "anchorUrl")]
    pub anchor_url: String,
    #[serde(rename = "anchorDataHash", skip_serializing_if = "Option::is_none")]
    pub anchor_data_hash: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WithdrawBody {
    pub event: String,
    pub milestones: std::collections::HashMap<String, WithdrawMilestone>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WithdrawMilestone {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

pub const METADATA_CONTEXT: &str =
    "https://raw.githubusercontent.com/SundaeSwap-finance/treasury-contracts/refs/heads/main/offchain/src/metadata/context.jsonld";
