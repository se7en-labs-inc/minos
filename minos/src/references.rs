//! Fail-closed resolution of registry and reference-script inputs.
use crate::{koios::Utxo, node::DataSource};
use anyhow::{ensure, Context, Result};
use pallas_codec::minicbor;
use pallas_crypto::hash::Hasher;
use pallas_primitives::conway::{NativeScript, PlutusData};
use std::collections::{BTreeMap, BTreeSet};

pub type OutRef = (String, u64);

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

fn registry_hashes(u: &Utxo, instance: &str) -> Result<(String, String)> {
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
    let refs: Vec<_> = [registry.clone(), vendor.clone(), treasury.clone()]
        .into_iter()
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
    for (r, expected) in [(vendor, vendor_hash), (treasury, treasury_hash)]
        .iter()
        .chain(staking.iter())
    {
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
