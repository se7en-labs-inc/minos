//! Milestone association follows initial ordered metadata and proven transitions.
//! It never derives identity from payout count, lexical IDs, or a withdrawn prefix.
use anyhow::{ensure, Result};
use minos::{
    tx::value::Value,
    types::{Payout, PayoutStatus, VendorDatum},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Fingerprint {
    maturation: u64,
    coin: u64,
    assets: BTreeMap<String, u64>,
}

fn fingerprint(payout: &Payout) -> Result<Fingerprint> {
    let value = Value::from_payouts(std::slice::from_ref(payout))?;
    Ok(Fingerprint {
        maturation: payout.maturation,
        coin: value.coin,
        assets: value.assets,
    })
}

pub fn initial(project: &crate::types::Project, datum: &VendorDatum) -> Vec<Option<String>> {
    if project.ordered_milestones
        && project.milestones.len() == datum.payouts.len()
        && project
            .milestones
            .iter()
            .map(|m| &m.identifier)
            .collect::<BTreeSet<_>>()
            .len()
            == project.milestones.len()
    {
        project
            .milestones
            .iter()
            .map(|milestone| Some(milestone.identifier.clone()))
            .collect()
    } else {
        vec![None; datum.payouts.len()]
    }
}

fn identifiers(previous: &VendorDatum, ids: &[Option<String>]) -> Result<Vec<Option<String>>> {
    if ids.is_empty() {
        return Ok(vec![None; previous.payouts.len()]);
    }
    ensure!(
        ids.len() == previous.payouts.len(),
        "stored payout association length mismatch"
    );
    let known = ids.iter().flatten().collect::<Vec<_>>();
    ensure!(
        known.iter().copied().collect::<BTreeSet<_>>().len() == known.len(),
        "duplicate stored milestone association"
    );
    Ok(ids.to_vec())
}

pub struct Withdrawal {
    pub remaining: Vec<Option<String>>,
    pub withdrawn: Vec<String>,
    pub paid: Value,
    pub unresolved_removed: usize,
}

pub fn withdraw(
    previous: &VendorDatum,
    ids: &[Option<String>],
    next: Option<&VendorDatum>,
    lower_ms: u64,
) -> Result<Withdrawal> {
    let ids = identifiers(previous, ids)?;
    let mut remaining = Vec::new();
    let mut retained = Vec::new();
    let mut removed = Vec::new();
    let mut withdrawn = Vec::new();
    let mut unresolved_removed = 0;
    for (payout, id) in previous.payouts.iter().zip(ids) {
        if matches!(payout.status, PayoutStatus::Active) && payout.maturation < lower_ms {
            removed.push(payout.clone());
            if let Some(id) = id {
                withdrawn.push(id);
            } else {
                unresolved_removed += 1;
            }
        } else {
            retained.push(payout.clone());
            remaining.push(id);
        }
    }
    if let Some(next) = next {
        let expected = VendorDatum {
            vendor: previous.vendor.clone(),
            payouts: retained,
        };
        ensure!(
            next.to_cbor_bytes() == expected.to_cbor_bytes(),
            "withdrawal successor does not preserve the actual non-matured/paused partition"
        );
    }
    Ok(Withdrawal {
        remaining,
        withdrawn,
        paid: Value::from_payouts(&removed)?,
        unresolved_removed,
    })
}

pub fn adjudicate(
    previous: &VendorDatum,
    ids: &[Option<String>],
    next: &VendorDatum,
) -> Result<Vec<Option<String>>> {
    let ids = identifiers(previous, ids)?;
    ensure!(
        previous.payouts.len() == next.payouts.len(),
        "adjudication changed payout count"
    );
    for (before, after) in previous.payouts.iter().zip(&next.payouts) {
        ensure!(
            fingerprint(before)? == fingerprint(after)?,
            "adjudication changed payout value or maturation"
        );
    }
    let expected = VendorDatum {
        vendor: previous.vendor.clone(),
        payouts: next.payouts.clone(),
    };
    ensure!(
        expected.to_cbor_bytes() == next.to_cbor_bytes(),
        "adjudication changed vendor authorization"
    );
    Ok(ids)
}

pub fn modify(
    previous: &VendorDatum,
    ids: &[Option<String>],
    next: &VendorDatum,
) -> Result<Vec<Option<String>>> {
    let ids = identifiers(previous, ids)?;
    let mut before = BTreeMap::<Fingerprint, Vec<Option<String>>>::new();
    for (payout, id) in previous.payouts.iter().zip(ids) {
        before.entry(fingerprint(payout)?).or_default().push(id);
    }
    let mut counts = BTreeMap::<Fingerprint, usize>::new();
    for payout in &next.payouts {
        *counts.entry(fingerprint(payout)?).or_default() += 1;
    }
    next.payouts
        .iter()
        .map(|payout| {
            let key = fingerprint(payout)?;
            Ok(match before.get(&key) {
                Some(ids) if ids.len() == 1 && counts[&key] == 1 => ids[0].clone(),
                _ => None,
            })
        })
        .collect()
}

pub fn synchronize(
    project: &mut crate::types::Project,
    datum: &VendorDatum,
    ids: &[Option<String>],
) -> Result<()> {
    let ids = identifiers(datum, ids)?;
    for (payout, id) in datum.payouts.iter().zip(ids) {
        if let Some(id) = id {
            let milestone = project
                .milestones
                .iter_mut()
                .find(|m| m.identifier == id)
                .ok_or_else(|| anyhow::anyhow!("payout references an unknown milestone"))?;
            milestone.maturation_ms = payout.maturation;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos::types::{AssetBundle, MultisigScript};

    fn payout(maturation: u64, paused: bool, quantity: i64) -> Payout {
        Payout {
            maturation,
            status: if paused {
                PayoutStatus::Paused
            } else {
                PayoutStatus::Active
            },
            value: AssetBundle(BTreeMap::from([(
                vec![],
                BTreeMap::from([(vec![], quantity)]),
            )])),
        }
    }
    fn datum(payouts: Vec<Payout>) -> VendorDatum {
        VendorDatum {
            vendor: MultisigScript::Signature {
                key_hash: vec![1; 28],
            },
            payouts,
        }
    }

    #[test]
    fn withdrawals_remove_the_actual_matured_partition_not_a_prefix() {
        let before = datum(vec![
            payout(10, true, 1),
            payout(50, false, 2),
            payout(5, false, 3),
            payout(20, false, 4),
        ]);
        let ids = ["m-1", "m-10", "m-2", "m-3"].map(|id| Some(id.into()));
        let after = datum(vec![
            before.payouts[0].clone(),
            before.payouts[1].clone(),
            before.payouts[3].clone(),
        ]);
        let transition = withdraw(&before, &ids, Some(&after), 20).unwrap();
        assert_eq!(transition.withdrawn, vec!["m-2"]);
        assert_eq!(
            transition.remaining,
            vec![Some("m-1".into()), Some("m-10".into()), Some("m-3".into())]
        );
        assert_eq!(transition.paid.coin, 3);
        assert_eq!(transition.unresolved_removed, 0);
        let mut wrong = after.clone();
        wrong.payouts.remove(0);
        assert!(withdraw(&before, &ids, Some(&wrong), 20).is_err());
        let unresolved = withdraw(&before, &[], Some(&after), 20).unwrap();
        assert_eq!(unresolved.unresolved_removed, 1);
        assert!(unresolved.withdrawn.is_empty());
    }

    #[test]
    fn adjudication_and_modification_preserve_or_explicitly_unresolve_identity() {
        let before = datum(vec![
            payout(10, false, 1),
            payout(30, false, 2),
            payout(20, false, 3),
        ]);
        let ids = ["m-10", "m-2", "m-1"].map(|id| Some(id.into()));
        let mut paused = before.clone();
        paused.payouts[1].status = PayoutStatus::Paused;
        assert_eq!(adjudicate(&before, &ids, &paused).unwrap(), ids);
        paused.payouts[1].maturation += 1;
        assert!(adjudicate(&before, &ids, &paused).is_err());
        let reordered = datum(vec![
            before.payouts[2].clone(),
            before.payouts[0].clone(),
            payout(40, false, 7),
        ]);
        assert_eq!(
            modify(&before, &ids, &reordered).unwrap(),
            vec![Some("m-1".into()), Some("m-10".into()), None]
        );
        let duplicate = datum(vec![before.payouts[0].clone(), before.payouts[0].clone()]);
        assert_eq!(modify(&before, &ids, &duplicate).unwrap(), vec![None, None]);
    }
}
