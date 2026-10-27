//! Fail-closed resolution of registry and reference-script inputs.
use crate::{koios::Utxo, node::DataSource};
use anyhow::{ensure, Context, Result};
use pallas_codec::minicbor;
use pallas_crypto::hash::Hasher;
use pallas_primitives::conway::{NativeScript, PlutusData};
use std::collections::{BTreeMap, BTreeSet};

pub type OutRef = (String, u64);

#[derive(Default)]
pub struct ReferenceHints {
    pub registry: Option<OutRef>,
    pub vendor: Option<OutRef>,
    /// Explicit compatibility hint for the original atomic publication layout.
    pub legacy_publish_tx: Option<String>,
}
pub struct VendorReferenceInputs {
    pub registry: OutRef,
    pub vendor: OutRef,
    pub inputs: Vec<Utxo>,
}

/// Resolve an advertised NFT outref with filtered node queries. Koios may also
/// discover by asset when no location is known; no full-node UTxO crawl is used.
pub async fn registry_output(
    source: &DataSource,
    instance: &str,
    hint: Option<OutRef>,
) -> Result<Utxo> {
    let output = if let Some(reference) = hint {
        complete_utxos(
            std::slice::from_ref(&reference),
            source.utxo_info(std::slice::from_ref(&reference)).await?,
        )?
        .remove(&reference)
        .context("registry output not returned")?
    } else {
        let mut outputs = source
            .asset_utxos(instance, &hex::encode(b"REGISTRY"))
            .await?;
        ensure!(
            outputs.len() == 1,
            "expected exactly one live registry NFT output"
        );
        outputs.remove(0)
    };
    registry_hashes(&output, instance)?;
    Ok(output)
}

pub async fn vendor_inputs(
    source: &DataSource,
    instance: &str,
    hints: ReferenceHints,
    configured: &[ScriptReference],
    staking: &[(OutRef, String)],
) -> Result<VendorReferenceInputs> {
    let registry = registry_output(
        source,
        instance,
        hints.registry.or_else(|| {
            hints
                .legacy_publish_tx
                .as_ref()
                .map(|hash| (hash.clone(), 0))
        }),
    )
    .await?;
    let (_, hash) = registry_hashes(&registry, instance)?;
    let hash: [u8; 28] = hex::decode(hash)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid registered vendor hash"))?;
    let mut locations = BTreeMap::new();
    for reference in configured {
        ensure!(
            locations
                .insert(reference.script_hash, reference.output.clone())
                .is_none(),
            "duplicate configured script hash"
        );
    }
    let vendor = locations
        .get(&hash)
        .cloned()
        .or_else(|| hints.legacy_publish_tx.map(|hash| (hash, 2)))
        .or(hints.vendor)
        .context(
            "vendor reference is not confirmed; configure its verified --script-reference location",
        )?;
    let registry = (registry.tx_hash, registry.tx_index);
    let inputs =
        withdrawal_references(source, instance, registry.clone(), vendor.clone(), staking).await?;
    Ok(VendorReferenceInputs {
        registry,
        vendor,
        inputs,
    })
}

/// Operator-supplied location; script bytes are still verified against the hash.
#[derive(Debug, Clone)]
pub struct ScriptReference {
    pub script_hash: [u8; 28],
    pub output: OutRef,
}

impl std::str::FromStr for ScriptReference {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let (hash, output) = value
            .split_once('=')
            .context("expected SCRIPT_HASH=TX_HASH#INDEX")?;
        let (tx, index) = output.split_once('#').context("expected TX_HASH#INDEX")?;
        let script_hash = hex::decode(hash)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("script hash must be 28 bytes"))?;
        ensure!(
            hex::decode(tx)?.len() == 32,
            "transaction hash must be 32 bytes"
        );
        Ok(Self {
            script_hash,
            output: (tx.to_ascii_lowercase(), index.parse()?),
        })
    }
}

pub type StakingAuthorization = (Vec<[u8; 28]>, Vec<OutRef>);

pub fn staking_authorization(
    vendor: &crate::types::MultisigScript,
    configured: &[ScriptReference],
) -> Result<StakingAuthorization> {
    let mut locations = BTreeMap::new();
    for reference in configured {
        ensure!(
            locations
                .insert(reference.script_hash, reference.output.clone())
                .is_none(),
            "duplicate configured script hash"
        );
    }
    let hashes = vendor
        .script_hashes()
        .into_iter()
        .map(|hash| {
            hash.try_into()
                .map_err(|_| anyhow::anyhow!("script hash must be 28 bytes"))
        })
        .collect::<Result<BTreeSet<[u8; 28]>>>()?
        .into_iter()
        .collect::<Vec<_>>();
    let outputs = hashes
        .iter()
        .map(|hash| {
            locations.get(hash).cloned().with_context(|| {
                format!(
                    "missing --script-reference {}=TX_HASH#INDEX",
                    hex::encode(hash)
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((hashes, outputs))
}

/// Require exactly one response for each requested output. Repeated requests are
/// deduplicated, but duplicate/unrequested provider responses are errors.
pub fn complete_utxos(refs: &[OutRef], utxos: Vec<Utxo>) -> Result<BTreeMap<OutRef, Utxo>> {
    let expected: BTreeSet<_> = refs.iter().cloned().collect();
    let mut found = BTreeMap::new();
    for u in utxos {
        let key = (u.tx_hash.clone(), u.tx_index);
        ensure!(
            expected.contains(&key),
            "unexpected UTxO {}#{}",
            key.0,
            key.1
        );
        ensure!(
            !found.contains_key(&key),
            "duplicate UTxO {}#{}",
            key.0,
            key.1
        );
        found.insert(key, u);
    }
    for key in expected {
        ensure!(
            found.contains_key(&key),
            "missing UTxO {}#{} (spent or incomplete query)",
            key.0,
            key.1
        );
    }
    Ok(found)
}

/// Return language tag and exact ledger script bytes; never infer missing bytes.
pub fn script_bytes(u: &Utxo) -> Result<(u8, Vec<u8>)> {
    let script = u
        .reference_script
        .as_ref()
        .context("missing reference script")?;
    let bytes = hex::decode(
        script["bytes"]
            .as_str()
            .context("missing reference script bytes")?,
    )
    .context("malformed reference script hex")?;
    ensure!(!bytes.is_empty(), "empty reference script");
    let kind = script["type"]
        .as_str()
        .context("missing reference script type")?;
    let tag = match kind.to_ascii_lowercase().as_str() {
        "native" | "timelock" | "multisig" => {
            let _: NativeScript = minicbor::decode(&bytes).context("invalid native script CBOR")?;
            0
        }
        "plutusv1" => 1,
        "plutusv2" => 2,
        "plutusv3" => 3,
        _ => anyhow::bail!("unsupported reference script type {kind}"),
    };
    Ok((tag, bytes))
}

pub fn registry_hashes(u: &Utxo, instance: &str) -> Result<(String, String)> {
    ensure!(
        u.asset_list
            .as_ref()
            .is_some_and(|assets| assets.iter().any(|a| a.policy_id == instance
                && a.asset_name == "5245474953545259"
                && a.quantity == "1")),
        "registry UTxO does not contain this instance's REGISTRY NFT"
    );
    let bytes = hex::decode(u.datum_hex().context("registry has no inline datum")?)?;
    let data: PlutusData = minicbor::decode(&bytes).context("invalid registry datum")?;
    let PlutusData::Constr(c) = data else {
        anyhow::bail!("registry datum must be a constructor");
    };
    ensure!(
        c.tag == 121 && c.fields.len() == 2,
        "invalid registry constructor"
    );
    let credential = |d: &PlutusData| -> Result<String> {
        let PlutusData::Constr(c) = d else {
            anyhow::bail!("invalid registry credential");
        };
        ensure!(
            c.tag == 122 && c.fields.len() == 1,
            "registry credential must be a script"
        );
        let PlutusData::BoundedBytes(hash) = &c.fields[0] else {
            anyhow::bail!("invalid registry hash");
        };
        ensure!(hash.len() == 28, "registry script hash must be 28 bytes");
        Ok(hex::encode(hash.as_slice()))
    };
    Ok((credential(&c.fields[0])?, credential(&c.fields[1])?))
}

/// All reference inputs are queried once and counted once, including any script
/// attached to the registry. Script hashes are checked against the registry datum.
pub async fn treasury_references(
    source: &DataSource,
    instance: &str,
    registry: OutRef,
    vendor: OutRef,
    treasury: OutRef,
    staking: &[(OutRef, String)],
) -> Result<Vec<Utxo>> {
    resolve_references(
        source,
        instance,
        registry,
        Some(vendor),
        Some(treasury),
        staking,
    )
    .await
}

/// Vendor withdrawals execute only the vendor and configured staking scripts.
pub async fn withdrawal_references(
    source: &DataSource,
    instance: &str,
    registry: OutRef,
    vendor: OutRef,
    staking: &[(OutRef, String)],
) -> Result<Vec<Utxo>> {
    resolve_references(source, instance, registry, Some(vendor), None, staking).await
}

/// Treasury funding/reorganization executes the treasury script and reads the
/// registry; the vendor script's bytes are not a reference input.
pub async fn funding_references(
    source: &DataSource,
    instance: &str,
    registry: OutRef,
    treasury: OutRef,
) -> Result<Vec<Utxo>> {
    resolve_references(source, instance, registry, None, Some(treasury), &[]).await
}

async fn resolve_references(
    source: &DataSource,
    instance: &str,
    registry: OutRef,
    vendor: Option<OutRef>,
    treasury: Option<OutRef>,
    staking: &[(OutRef, String)],
) -> Result<Vec<Utxo>> {
    let refs: Vec<_> = std::iter::once(registry.clone())
        .chain(vendor.iter().cloned())
        .chain(treasury.iter().cloned())
        .chain(staking.iter().map(|(r, _)| r.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let found = complete_utxos(
        &refs,
        source
            .utxo_info(&refs)
            .await
            .context("resolve reference inputs")?,
    )?;
    let (treasury_hash, vendor_hash) = registry_hashes(&found[&registry], instance)?;
    let expected = vendor
        .map(|vendor| (vendor, vendor_hash))
        .into_iter()
        .chain(treasury.map(|r| (r, treasury_hash)))
        .collect::<Vec<_>>();
    for (r, expected) in expected.iter().chain(staking.iter()) {
        let (tag, bytes) =
            script_bytes(&found[r]).with_context(|| format!("reference script {}#{}", r.0, r.1))?;
        ensure!(
            Hasher::<224>::hash_tagged(&bytes, tag).to_string() == *expected,
            "reference script hash mismatch at {}#{}; expected {}",
            r.0,
            r.1,
            expected
        );
    }
    Ok(found.into_values().collect())
}

pub fn total_script_bytes(utxos: &[Utxo]) -> Result<u64> {
    let mut total = 0u64;
    let mut seen = BTreeSet::new();
    for u in utxos {
        if seen.insert((&u.tx_hash, u.tx_index)) && u.reference_script.is_some() {
            total = total
                .checked_add(script_bytes(u)?.1.len() as u64)
                .context("reference script size overflow")?;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staking_locations_are_explicit_and_independent_of_deployment() {
        use crate::types::MultisigScript;
        let vendor = MultisigScript::AllOf {
            scripts: vec![
                MultisigScript::Script {
                    script_hash: vec![3; 28],
                },
                MultisigScript::Script {
                    script_hash: vec![3; 28],
                },
            ],
        };
        assert!(staking_authorization(&vendor, &[]).is_err());
        for transaction in ["01".repeat(32), "02".repeat(32)] {
            let reference: ScriptReference = format!("{}={transaction}#7", "03".repeat(28))
                .parse()
                .unwrap();
            let (hashes, outputs) =
                staking_authorization(&vendor, std::slice::from_ref(&reference)).unwrap();
            assert_eq!(hashes, vec![[3; 28]]);
            assert_eq!(outputs, vec![(transaction, 7)]);
            assert!(staking_authorization(&vendor, &[reference.clone(), reference]).is_err());
        }
        for malformed in ["", "abcd=ef#0", "03=abcd#x"] {
            assert!(malformed.parse::<ScriptReference>().is_err());
        }
    }

    #[tokio::test]
    async fn relocated_script_reference_is_hash_verified() {
        let mut all: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/fixtures/withdrawal/utxos.json")).unwrap();
        let staking = all
            .iter_mut()
            .find(|u| u["tx_hash"].as_str().unwrap().starts_with("aaec"))
            .unwrap();
        staking["tx_hash"] = "04".repeat(32).into();
        staking["tx_index"] = 7.into();
        let response: Vec<_> = all
            .into_iter()
            .filter(|u| {
                [REGISTRY, VENDOR, &"04".repeat(32)].contains(&u["tx_hash"].as_str().unwrap())
            })
            .collect();
        let (source, task) = source(serde_json::to_string(&response).unwrap(), 200).await;
        let outputs = withdrawal_references(
            &source,
            INSTANCE,
            (REGISTRY.into(), 0),
            (VENDOR.into(), 0),
            &[(
                ("04".repeat(32), 7),
                "326cec60e7bcf3673b02f8684802febdcf27cad1601d75182ad6a5bb".into(),
            )],
        )
        .await
        .unwrap();
        assert_eq!(outputs.len(), 3);
        task.await.unwrap();
    }
    const INSTANCE: &str = "51486a2f1496d4d3a688a9b111971aa9b731ed045d900b601345ca4e";
    const REGISTRY: &str = "fe2a314561f787aa55f2cf375438a7c8d4f5d090c82292c0e0d6699aec11e722";
    const VENDOR: &str = "c307d0fcd0b63f6e31d44d7bb1726661b949f9f50eac60be7f81fb6748d35472";
    const TREASURY: &str = "944e78fe313cf2f608b987bcedaa6a1e7ca3d3fe49844224ff8b31846ad265c1";
    // One deterministic local HTTP response; no Koios requests in tests.
    async fn source(body: String, status: u16) -> (DataSource, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut req = vec![0; 8192];
            let n = socket.read(&mut req).await.unwrap();
            assert!(String::from_utf8_lossy(&req[..n]).contains("POST /utxo_info"));
            socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        (
            DataSource::Koios(crate::koios::Koios::new(&format!("http://{addr}"), None)),
            task,
        )
    }
    #[tokio::test]
    async fn withdrawal_references_exclude_treasury_bytes() {
        let all: Vec<Utxo> =
            serde_json::from_str(include_str!("../tests/fixtures/withdrawal/utxos.json")).unwrap();
        let treasury_bytes = script_bytes(all.iter().find(|u| u.tx_hash == TREASURY).unwrap())
            .unwrap()
            .1
            .len() as u64;
        let response: Vec<serde_json::Value> = serde_json::from_str::<Vec<serde_json::Value>>(
            include_str!("../tests/fixtures/withdrawal/utxos.json"),
        )
        .unwrap()
        .into_iter()
        .filter(|u| [REGISTRY, VENDOR].contains(&u["tx_hash"].as_str().unwrap()))
        .collect();
        let (source, task) = source(serde_json::to_string(&response).unwrap(), 200).await;
        let outputs = withdrawal_references(
            &source,
            INSTANCE,
            (REGISTRY.into(), 0),
            (VENDOR.into(), 0),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(outputs.len(), 2);
        assert_eq!(
            total_script_bytes(&outputs).unwrap(),
            19190 - treasury_bytes
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn reference_queries_fail_closed_and_verify_registry_script_hashes() {
        let all: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/fixtures/withdrawal/utxos.json")).unwrap();
        let base: Vec<_> = all
            .into_iter()
            .filter(|u| [REGISTRY, VENDOR, TREASURY].contains(&u["tx_hash"].as_str().unwrap()))
            .collect();
        for case in 0..6 {
            let mut response = base.clone();
            let mut status = 200;
            match case {
                1 => {
                    response.pop();
                }
                2 => {
                    response
                        .iter_mut()
                        .find(|u| u["tx_hash"] == VENDOR)
                        .unwrap()["reference_script"]["bytes"] = "43010203".into();
                }
                3 => {
                    response
                        .iter_mut()
                        .find(|u| u["tx_hash"] == REGISTRY)
                        .unwrap()["asset_list"] = serde_json::json!([]);
                }
                4 => {
                    status = 429;
                }
                5 => {
                    response.push(response[0].clone());
                }
                _ => {}
            }
            let (source, task) = source(serde_json::to_string(&response).unwrap(), status).await;
            let result = treasury_references(
                &source,
                INSTANCE,
                (REGISTRY.into(), 0),
                (VENDOR.into(), 0),
                (TREASURY.into(), 0),
                &[],
            )
            .await;
            if case == 0 {
                let outputs = result.unwrap();
                assert_eq!(total_script_bytes(&outputs).unwrap(), 19190);
            } else {
                assert!(result.is_err(), "case {case} unexpectedly succeeded");
            }
            task.await.unwrap();
        }
    }
}
