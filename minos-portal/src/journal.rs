//! Sled is authoritative. One batch commits a whole block, its projection,
//! events and checkpoint. DuckDB is a rebuildable query cache.
use crate::{
    chain::{self, Block, Point, State},
    db::Store,
    types::Event,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Projection {
    pub state: State,
    pub events: Vec<Event>,
}
fn key(kind: &str, instance: &str) -> Vec<u8> {
    format!("chain/{kind}/{instance}").into_bytes()
}
fn block_prefix(instance: &str) -> String {
    format!("chain/block/{instance}/")
}
fn block_key(instance: &str, point: &Point) -> Vec<u8> {
    format!(
        "{}{:016x}/{}",
        block_prefix(instance),
        point.slot,
        point.hash
    )
    .into_bytes()
}
impl Store {
    #[cfg(test)]
    fn fail_at(&self, boundary: u8) -> Result<()> {
        ensure!(
            self.failure_boundary
                .load(std::sync::atomic::Ordering::SeqCst)
                != boundary,
            "injected journal write failure at boundary {boundary}"
        );
        Ok(())
    }
    pub(crate) fn projection(&self, instance: &str) -> Result<Option<Projection>> {
        self.sled
            .get(key("projection", instance))?
            .map(|bytes| serde_json::from_slice(&bytes).context("invalid chain projection"))
            .transpose()
    }
    pub fn checkpoint(&self, instance: &str) -> Result<Option<Point>> {
        self.sled
            .get(key("point", instance))?
            .map(|bytes| serde_json::from_slice(&bytes).context("invalid chain checkpoint"))
            .transpose()
    }
    fn journal(&self, instance: &str) -> Result<Vec<Block>> {
        self.sled
            .scan_prefix(block_prefix(instance))
            .map(|entry| {
                let (_, bytes) = entry?;
                serde_json::from_slice(&bytes).context("invalid chain journal block")
            })
            .collect()
    }
    /// Replace a fully fetched, consistently anchored Koios history in one
    /// commit. Failed/incomplete upstream queries never erase confirmed state.
    pub fn replace_chain(&self, instance: &str, blocks: &[Block], tip: &Point) -> Result<()> {
        let _guard = self.chain_gate.lock().unwrap();
        let mut projection = Projection::default();
        let mut relevant = Vec::new();
        let mut previous = None;
        for block in blocks {
            ensure!(
                previous.is_none_or(|slot| block.point.slot > slot),
                "history blocks not strictly ordered"
            );
            ensure!(
                block.point.slot <= tip.slot,
                "history extends beyond captured tip"
            );
            previous = Some(block.point.slot);
            let applied = chain::apply_block(instance, &self.network, &projection.state, block)?;
            if let Some(state) = applied.state {
                projection.state = state;
                relevant.push(applied.relevant);
            }
            projection.events.extend(applied.events);
        }
        ensure!(
            projection.state.instance.is_some(),
            "history does not confirm the instance publication"
        );
        let mut batch = sled::Batch::default();
        batch.insert(
            key("projection", instance),
            serde_json::to_vec(&projection)?,
        );
        for entry in self.sled.scan_prefix(block_prefix(instance)) {
            batch.remove(entry?.0);
        }
        for block in &relevant {
            batch.insert(
                block_key(instance, &block.point),
                serde_json::to_vec(block)?,
            );
        }
        batch.insert(key("point", instance), serde_json::to_vec(tip)?);
        self.sled.apply_batch(batch)?;
        self.sled.flush()?;
        self.chain_cache.lock().unwrap().remove(instance);
        self.refresh_event_cache(instance, &projection.events)
    }
    pub fn commit_block(&self, instance: &str, block: &Block) -> Result<()> {
        let _guard = self.chain_gate.lock().unwrap();
        if let Some(previous) = self.checkpoint(instance)? {
            if previous == block.point {
                self.sled.flush()?;
                self.chain_cache.lock().unwrap().remove(instance);
                if let Some(projection) = self.projection(instance)? {
                    self.refresh_event_cache(instance, &projection.events)?;
                }
                return Ok(());
            }
            ensure!(
                block.point.slot > previous.slot,
                "out-of-order block: rollback before replay"
            );
        }
        let previous = {
            let mut cache = self.chain_cache.lock().unwrap();
            if !cache.contains_key(instance) {
                cache.insert(
                    instance.into(),
                    std::sync::Arc::new(self.projection(instance)?.unwrap_or_default().state),
                );
            }
            cache[instance].clone()
        };
        let applied = chain::apply_block(instance, &self.network, &previous, block)?;
        let mut batch = sled::Batch::default();
        let mut updated = None;
        if let Some(state) = applied.state {
            let mut projection = self.projection(instance)?.unwrap_or_default();
            projection.state = state;
            projection.events.extend(applied.events);
            batch.insert(
                block_key(instance, &block.point),
                serde_json::to_vec(&applied.relevant)?,
            );
            batch.insert(
                key("projection", instance),
                serde_json::to_vec(&projection)?,
            );
            updated = Some(projection);
        }
        batch.insert(key("point", instance), serde_json::to_vec(&block.point)?);
        if updated.is_some() {
            self.chain_cache.lock().unwrap().remove(instance);
        }
        #[cfg(test)]
        self.fail_at(1)?;
        self.sled.apply_batch(batch)?;
        #[cfg(test)]
        self.fail_at(2)?;
        self.sled.flush().context("persist complete chain block")?;
        #[cfg(test)]
        self.fail_at(3)?;
        // Queries also refresh this cache, so a cache failure never loses a
        // committed chain event and a restart always recovers it.
        if let Some(projection) = updated {
            self.chain_cache
                .lock()
                .unwrap()
                .insert(instance.into(), std::sync::Arc::new(projection.state));
            self.refresh_event_cache(instance, &projection.events)?;
        }
        #[cfg(test)]
        self.fail_at(4)?;
        Ok(())
    }
    pub fn rollback_chain(&self, instance: &str, point: Option<&Point>) -> Result<()> {
        let _guard = self.chain_gate.lock().unwrap();
        let mut journal = self.journal(instance)?;
        let mut batch = sled::Batch::default();
        for block in &journal {
            if !point.is_some_and(|point| {
                block.point.slot < point.slot
                    || (block.point.slot == point.slot && block.point.hash == point.hash)
            }) {
                batch.remove(block_key(instance, &block.point));
            }
        }
        journal.retain(|block| {
            point.is_some_and(|point| {
                block.point.slot < point.slot
                    || (block.point.slot == point.slot && block.point.hash == point.hash)
            })
        });
        let mut projection = Projection::default();
        for block in &journal {
            let applied = chain::apply_block(instance, &self.network, &projection.state, block)?;
            if let Some(state) = applied.state {
                projection.state = state;
            }
            projection.events.extend(applied.events);
        }
        batch.insert(
            key("projection", instance),
            serde_json::to_vec(&projection)?,
        );
        if let Some(point) = point {
            batch.insert(key("point", instance), serde_json::to_vec(point)?);
        } else {
            batch.remove(key("point", instance));
        }
        self.sled.apply_batch(batch)?;
        self.sled.flush()?;
        self.chain_cache.lock().unwrap().remove(instance);
        self.refresh_event_cache(instance, &projection.events)
    }
    pub(crate) fn refresh_event_cache(&self, instance: &str, events: &[Event]) -> Result<()> {
        let mut duck = self.duck.lock().unwrap();
        let tx = duck.transaction()?;
        tx.execute(
            "DELETE FROM events WHERE instance_id = ?",
            duckdb::params![instance],
        )?;
        for event in events {
            tx.execute("INSERT OR REPLACE INTO events (tx_hash,block_slot,block_height,event_type,instance_id,project_id,payload) VALUES (?,?,?,?,?,?,?)",
                duckdb::params![event.tx_hash, i64::try_from(event.block_slot)?, i64::try_from(event.block_height)?, event.event_type.as_str(), event.instance_id, event.project_id.as_deref().unwrap_or(""), event.payload.to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn block(slot: u64, hash: u8) -> Block {
        Block {
            point: Point {
                slot,
                hash: hex::encode([hash; 32]),
                height: slot,
            },
            transactions: vec![],
        }
    }
    #[test]
    fn injected_writer_child() {
        let Ok(path) = std::env::var("MINOS_TEST_JOURNAL_PATH") else {
            return;
        };
        let boundary = std::env::var("MINOS_TEST_JOURNAL_BOUNDARY")
            .unwrap()
            .parse()
            .unwrap();
        let (instance, bytes, _) = crate::chain::tests::publication_fixture();
        let store = Store::open(&path, "preview").unwrap();
        store
            .failure_boundary
            .store(boundary, std::sync::atomic::Ordering::SeqCst);
        let mut published = block(100, 7);
        published.transactions.push(bytes);
        assert!(store.commit_block(&instance, &published).is_err());
        // Deliberately bypass Drop: model process loss before acknowledgment,
        // including the interval between the atomic batch and explicit flush.
        std::process::exit(0);
    }
    #[test]
    fn process_loss_at_every_commit_boundary_never_exposes_partial_blocks() {
        let root = std::env::temp_dir().join(format!(
            "minos-fault-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let (instance, bytes, _) = crate::chain::tests::publication_fixture();
        let mut published = block(100, 7);
        published.transactions.push(bytes);
        for boundary in 1..=4 {
            let path = root.join(boundary.to_string());
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "journal::tests::injected_writer_child",
                    "--nocapture",
                ])
                .env("MINOS_TEST_JOURNAL_PATH", &path)
                .env("MINOS_TEST_JOURNAL_BOUNDARY", boundary.to_string())
                .output()
                .unwrap();
            assert!(
                child.status.success(),
                "child failed: {}",
                String::from_utf8_lossy(&child.stderr)
            );
            let store = Store::open(path.to_str().unwrap(), "preview").unwrap();
            let point = store.checkpoint(&instance).unwrap();
            let state = store.projection(&instance).unwrap();
            assert_eq!(
                point.is_some(),
                state.is_some(),
                "partial block at boundary {boundary}"
            );
            if boundary == 1 {
                assert!(point.is_none());
            }
            if boundary >= 3 {
                assert!(point.is_some());
            }
            if let Some(state) = state {
                assert!(state.state.instance.unwrap().confirmed);
                assert_eq!(state.events.len(), 1);
            }
            store.commit_block(&instance, &published).unwrap();
            assert_eq!(
                store.projection(&instance).unwrap().unwrap().events.len(),
                1
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checkpoint_is_whole_block_idempotent_and_rollback_accepts_fork() {
        let store = Store::temporary();
        let a = block(10, 1);
        let b = block(20, 2);
        store.commit_block("one", &a).unwrap();
        store.commit_block("one", &a).unwrap();
        store.commit_block("one", &b).unwrap();
        assert!(store.commit_block("one", &block(20, 3)).is_err());
        store.rollback_chain("one", Some(&a.point)).unwrap();
        store.commit_block("one", &block(20, 3)).unwrap();
        assert!(store.checkpoint("two").unwrap().is_none());
        store.rollback_chain("one", None).unwrap();
        assert!(store.checkpoint("one").unwrap().is_none());
        assert!(store.projection("one").unwrap().unwrap().events.is_empty());
    }
    #[test]
    fn malformed_block_never_advances_checkpoint() {
        let store = Store::temporary();
        let mut bad = block(10, 1);
        bad.transactions.push(vec![0xff]);
        assert!(store.commit_block("one", &bad).is_err());
        assert!(store.checkpoint("one").unwrap().is_none());
    }
    #[test]
    fn concurrent_instances_commit_and_rollback_independently() {
        let store = std::sync::Arc::new(Store::temporary());
        let mut threads = Vec::new();
        let mut instances = Vec::new();
        for seed in [5, 6] {
            let (instance, bytes, _) = crate::chain::tests::publication_fixture_seed(seed);
            instances.push(instance.clone());
            let store = store.clone();
            threads.push(std::thread::spawn(move || {
                let mut first = block(100, seed);
                first.transactions.push(bytes);
                store.commit_block(&instance, &first).unwrap();
                for slot in 101..120 {
                    store.commit_block(&instance, &block(slot, seed)).unwrap();
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        for instance in &instances {
            assert_eq!(store.get_instance(instance).unwrap().unwrap().id, *instance);
            assert_eq!(store.checkpoint(instance).unwrap().unwrap().slot, 119);
            assert_eq!(store.projection(instance).unwrap().unwrap().events.len(), 1);
        }
        store.rollback_chain(&instances[0], None).unwrap();
        assert!(store.get_instance(&instances[0]).unwrap().is_none());
        assert!(
            store
                .get_instance(&instances[1])
                .unwrap()
                .unwrap()
                .confirmed
        );
    }
    #[test]
    fn project_payout_state_and_events_match_clean_replay_across_rollback() {
        let (instance, blocks) = crate::chain::tests::lifecycle_fixture();
        let store = Store::temporary();
        for block in &blocks {
            store.commit_block(&instance, block).unwrap();
        }
        let complete = serde_json::to_value(store.projection(&instance).unwrap().unwrap()).unwrap();
        let project = store.get_project(&instance, "project").unwrap().unwrap();
        assert_eq!(
            project
                .milestones
                .iter()
                .map(|m| m.identifier.as_str())
                .collect::<Vec<_>>(),
            vec!["m-1", "m-10", "m-2"]
        );
        let current = store.get_utxo(&instance, "project").unwrap().unwrap();
        assert_eq!(current.payout_ids, vec![Some("m-1".into())]);
        let withdrawn = store
            .withdrawn_milestones_for_project(&instance, "project")
            .unwrap();
        assert_eq!(withdrawn, vec!["m-10", "m-2"]);
        assert_eq!(current.lovelace, 8_000_000);
        let projection = store.projection(&instance).unwrap().unwrap();
        assert_eq!(projection.events.len(), 6);
        assert_eq!(projection.state.observations.len(), 1);
        assert!(projection
            .state
            .observations
            .values()
            .next()
            .unwrap()
            .reason
            .contains("no matching contract execution"));
        assert_eq!(
            projection
                .events
                .iter()
                .filter(|event| event.event_type == crate::types::EventType::Fund)
                .count(),
            1
        );
        store
            .rollback_chain(&instance, Some(&blocks[2].point))
            .unwrap();
        let clean = Store::temporary();
        for block in &blocks[..3] {
            clean.commit_block(&instance, block).unwrap();
        }
        assert_eq!(
            serde_json::to_value(store.projection(&instance).unwrap()).unwrap(),
            serde_json::to_value(clean.projection(&instance).unwrap()).unwrap()
        );
        for block in &blocks[3..] {
            store.commit_block(&instance, block).unwrap();
        }
        assert_eq!(
            serde_json::to_value(store.projection(&instance).unwrap().unwrap()).unwrap(),
            complete
        );
    }
    #[test]
    fn committed_block_survives_cache_failure_restart_and_origin_replay() {
        let path = std::env::temp_dir().join(format!(
            "minos-journal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = path.to_str().unwrap();
        let (instance, signed, _) = crate::chain::tests::publication_fixture();
        let mut published = block(100, 7);
        published.transactions.push(signed);
        let expected;
        {
            let store = Store::open(path, "preview").unwrap();
            store.register_instance(&instance, "preview").unwrap();
            store
                .store_registered_tx(
                    &crate::db::RegisteredTx {
                        tx_hash: "local".into(),
                        label: "retained".into(),
                        cbor_hex: "00".into(),
                        submitted_at_ms: 1,
                    },
                    None,
                )
                .unwrap();
            store
                .duck
                .lock()
                .unwrap()
                .execute_batch("DROP TABLE events")
                .unwrap();
            assert!(store.commit_block(&instance, &published).is_err());
            assert_eq!(
                store.checkpoint(&instance).unwrap().unwrap(),
                published.point
            );
            expected = serde_json::to_value(store.projection(&instance).unwrap().unwrap()).unwrap();
        }
        {
            let store = Store::open(path, "preview").unwrap();
            assert!(store.get_instance(&instance).unwrap().unwrap().confirmed);
            store.commit_block(&instance, &published).unwrap();
            assert_eq!(
                store.projection(&instance).unwrap().unwrap().events.len(),
                1
            );
            store.rollback_chain(&instance, None).unwrap();
            assert!(store.get_instance(&instance).unwrap().is_none());
            assert_eq!(store.get_registered_txs().unwrap().len(), 1);
            store.commit_block(&instance, &published).unwrap();
            assert_eq!(
                serde_json::to_value(store.projection(&instance).unwrap().unwrap()).unwrap(),
                expected
            );
        }
        std::fs::remove_dir_all(path).unwrap();
        for suffix in ["-events.duckdb", "-events.duckdb.wal"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
    }
}
