use anyhow::{Context, Result};
use duckdb::Connection;
use std::sync::Mutex;

use crate::types::{Completion, Event, EventType, Instance, Project, VendorUtxo};

// Sled tree name constants
const TREE_INSTANCES: &str = "instances";
const TREE_PROJECTS: &str = "projects";
const TREE_UTXOS: &str = "utxos";
const TREE_INDEXED_TXS: &str = "indexed_txs";
const TREE_SYNC: &str = "sync";
const TREE_REGISTERED_TXS: &str = "registered_txs";
const TREE_TRACKED_INSTANCES: &str = "tracked_instances";
const TREE_METADATA: &str = "metadata";
pub(crate) const SCHEMA_VERSION: u64 = 3;

fn scoped_key(instance: &str, identifier: &str) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(instance, identifier))?)
}

/// An instance the indexer should watch, registered either from config seeds
/// or auto-detected from a newTransaction / indexTransaction call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrackedInstance {
    pub id: String,
    pub network: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegisteredTx {
    pub tx_hash: String,
    pub label: String,
    pub cbor_hex: String,
    pub submitted_at_ms: u64,
}

pub struct Store {
    pub(crate) sled: sled::Db,
    pub(crate) duck: Mutex<Connection>,
    pub(crate) network: String,
    pub(crate) chain_gate: Mutex<()>,
    pub(crate) chain_cache:
        Mutex<std::collections::BTreeMap<String, std::sync::Arc<crate::chain::State>>>,
    #[cfg(test)]
    pub(crate) failure_boundary: std::sync::atomic::AtomicU8,
}

impl Store {
    #[cfg(test)]
    pub fn temporary() -> Self {
        let duck = Connection::open_in_memory().unwrap();
        duck.execute_batch(EVENTS_DDL).unwrap();
        Self {
            sled: sled::Config::new().temporary(true).open().unwrap(),
            duck: Mutex::new(duck),
            network: "preview".into(),
            chain_gate: Mutex::new(()),
            chain_cache: Mutex::new(Default::default()),
            failure_boundary: std::sync::atomic::AtomicU8::new(0),
        }
    }

    pub fn open(path: &str, network: &str) -> Result<Self> {
        let sled = sled::open(path).with_context(|| format!("open sled at {path}"))?;

        let existing_trees = sled.tree_names();
        let exists = |name: &str| {
            existing_trees
                .iter()
                .any(|tree| tree.as_ref() == name.as_bytes())
        };
        let stored_schema = if exists(TREE_METADATA) {
            anyhow::ensure!(
                sled.open_tree(TREE_METADATA)?
                    .get("restore_in_progress")?
                    .is_none(),
                "restore interrupted; rerun the same offline restore command before startup"
            );
            anyhow::ensure!(
                sled.open_tree(TREE_METADATA)?
                    .get("migration_in_progress")?
                    .is_none(),
                "migration interrupted; rerun the offline migrate command before startup"
            );
            sled.open_tree(TREE_METADATA)?.get("schema")?
        } else {
            None
        };
        let expected = serde_json::json!({"version": SCHEMA_VERSION, "network": network});
        if let Some(bytes) = stored_schema {
            let actual: serde_json::Value =
                serde_json::from_slice(&bytes).context("invalid database schema metadata")?;
            anyhow::ensure!(actual == expected, "database schema/network mismatch: expected schema {SCHEMA_VERSION} on {network}; back up and migrate/reindex explicitly");
        } else {
            for name in [
                TREE_INSTANCES,
                TREE_PROJECTS,
                TREE_UTXOS,
                TREE_INDEXED_TXS,
                TREE_SYNC,
                TREE_REGISTERED_TXS,
                TREE_TRACKED_INSTANCES,
            ] {
                anyhow::ensure!(!exists(name) || sled.open_tree(name)?.is_empty(),
                    "legacy database has no schema version; back up both stores and migrate/reindex explicitly before startup");
            }
            anyhow::ensure!(
                !std::path::Path::new(&format!("{path}-events.duckdb")).exists(),
                "unversioned event database exists; back up and migrate/reindex explicitly"
            );
        }

        // Establish ownership before creating the rebuildable event projection;
        // an interrupted first open can then retry without a legacy-data guess.
        let metadata = sled.open_tree(TREE_METADATA)?;
        metadata.insert("schema", serde_json::to_vec(&expected)?)?;
        sled.flush().context("persist schema version")?;
        let events_path = format!("{path}-events.duckdb");
        let duck = Connection::open(&events_path)
            .with_context(|| format!("open duckdb at {events_path}"))?;
        duck.execute_batch(EVENTS_DDL)
            .context("create events table")?;

        let store = Self {
            sled,
            duck: Mutex::new(duck),
            network: network.into(),
            chain_gate: Mutex::new(()),
            chain_cache: Mutex::new(Default::default()),
            #[cfg(test)]
            failure_boundary: std::sync::atomic::AtomicU8::new(0),
        };
        if let Some(instance) = store
            .sled
            .open_tree(TREE_METADATA)?
            .get("reset_in_progress")?
        {
            let instance =
                std::str::from_utf8(&instance).context("invalid interrupted-reset marker")?;
            store
                .reset_sync(instance)
                .context("resume interrupted instance reset")?;
        }
        Ok(store)
    }

    /// Offline, restartable instance reset. Never clear another instance's state
    /// or local transaction registrations. A crash resumes the same reset at open.
    pub fn reset_sync(&self, instance_id: &str) -> Result<()> {
        self.rollback_chain(instance_id, None)?;
        let metadata = self.sled.open_tree(TREE_METADATA)?;
        metadata.insert("reset_in_progress", instance_id.as_bytes())?;
        self.sled.flush()?;
        let tree = self.sled.open_tree(TREE_SYNC)?;
        for prefix in [
            "sync_slot_",
            "sync_hash_",
            "publish_slot_",
            "publish_block_hash_",
        ] {
            tree.remove(format!("{prefix}{instance_id}").as_bytes())?;
        }
        for name in [TREE_PROJECTS, TREE_UTXOS, TREE_INDEXED_TXS] {
            let tree = self.sled.open_tree(name)?;
            let mut removals = sled::Batch::default();
            for entry in &tree {
                let (key, _) = entry?;
                let (instance, _): (String, String) =
                    serde_json::from_slice(&key).context("invalid scoped projection key")?;
                if instance == instance_id {
                    removals.remove(key);
                }
            }
            tree.apply_batch(removals)?;
        }
        let duck = self.duck.lock().unwrap();
        duck.execute(
            "DELETE FROM events WHERE instance_id = ?",
            duckdb::params![instance_id],
        )?;
        metadata.remove("reset_in_progress")?;
        self.sled.flush()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Sync state
    // -----------------------------------------------------------------------

    pub fn get_sync(&self, key: &str) -> Result<Option<String>> {
        let tree = self.sled.open_tree(TREE_SYNC)?;
        Ok(tree
            .get(key)?
            .map(|v| String::from_utf8_lossy(&v).into_owned()))
    }

    pub fn set_sync(&self, key: &str, value: &str) -> Result<()> {
        let tree = self.sled.open_tree(TREE_SYNC)?;
        tree.insert(key, value.as_bytes())?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Instances
    // -----------------------------------------------------------------------

    pub fn get_instance(&self, id: &str) -> Result<Option<Instance>> {
        if let Some(projection) = self.projection(id)? {
            if projection.state.instance.is_some() {
                return Ok(projection.state.instance);
            }
        }
        let tree = self.sled.open_tree(TREE_INSTANCES)?;
        match tree.get(id.as_bytes())? {
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn all_instances(&self) -> Result<Vec<Instance>> {
        let mut instances = std::collections::BTreeMap::new();
        for entry in &self.sled.open_tree(TREE_INSTANCES)? {
            let (_, bytes) = entry?;
            let instance: Instance = serde_json::from_slice(&bytes)?;
            instances.insert(instance.id.clone(), instance);
        }
        for tracked in self.get_tracked_instances()? {
            if let Some(projection) = self.projection(&tracked.id)? {
                if let Some(instance) = projection.state.instance {
                    instances.insert(instance.id.clone(), instance);
                }
            }
        }
        Ok(instances.into_values().collect())
    }

    // -----------------------------------------------------------------------
    // Projects
    // -----------------------------------------------------------------------

    #[cfg(test)]
    pub fn upsert_project(&self, proj: &Project) -> Result<()> {
        let tree = self.sled.open_tree(TREE_PROJECTS)?;
        let bytes = serde_json::to_vec(proj)?;
        tree.insert(scoped_key(&proj.instance_id, &proj.id)?, bytes)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn get_project(&self, instance: &str, id: &str) -> Result<Option<Project>> {
        if let Some(projection) = self.projection(instance)? {
            return Ok(projection.state.projects.get(id).cloned());
        }
        let tree = self.sled.open_tree(TREE_PROJECTS)?;
        match tree.get(scoped_key(instance, id)?)? {
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn projects_for_instance(&self, instance_id: &str) -> Result<Vec<Project>> {
        if let Some(projection) = self.projection(instance_id)? {
            return Ok(projection.state.projects.into_values().collect());
        }
        let tree = self.sled.open_tree(TREE_PROJECTS)?;
        tree.iter()
            .map(|r| {
                let (_, v) = r?;
                serde_json::from_slice::<Project>(&v).context("invalid stored project")
            })
            .collect::<Result<Vec<_>>>()
            .map(|projects| {
                projects
                    .into_iter()
                    .filter(|p| p.instance_id == instance_id)
                    .collect()
            })
    }

    // -----------------------------------------------------------------------
    // Current vendor UTxO (one per project)
    // -----------------------------------------------------------------------

    #[cfg(test)]
    pub fn upsert_utxo(&self, utxo: &VendorUtxo) -> Result<()> {
        let tree = self.sled.open_tree(TREE_UTXOS)?;
        let bytes = serde_json::to_vec(utxo)?;
        tree.insert(scoped_key(&utxo.instance_id, &utxo.project_id)?, bytes)?;
        Ok(())
    }

    pub fn get_utxo(&self, instance: &str, project_id: &str) -> Result<Option<VendorUtxo>> {
        if let Some(projection) = self.projection(instance)? {
            return Ok(projection.state.current.get(project_id).cloned());
        }
        let tree = self.sled.open_tree(TREE_UTXOS)?;
        match tree.get(scoped_key(instance, project_id)?)? {
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            None => Ok(None),
        }
    }

    // -----------------------------------------------------------------------
    // Indexed tx dedup
    // -----------------------------------------------------------------------

    #[cfg(test)]
    pub fn is_indexed(&self, instance: &str, tx_hash: &str) -> Result<bool> {
        let tree = self.sled.open_tree(TREE_INDEXED_TXS)?;
        Ok(tree.contains_key(scoped_key(instance, tx_hash)?)?)
    }

    #[cfg(test)]
    pub fn mark_indexed(&self, instance: &str, tx_hash: &str, slot: u64) -> Result<()> {
        let tree = self.sled.open_tree(TREE_INDEXED_TXS)?;
        tree.insert(scoped_key(instance, tx_hash)?, &slot.to_be_bytes())?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Event log (DuckDB)
    // -----------------------------------------------------------------------

    #[cfg(test)]
    pub fn append_event(&self, ev: &Event) -> Result<()> {
        let slot = i64::try_from(ev.block_slot).context("event slot exceeds database range")?;
        let height =
            i64::try_from(ev.block_height).context("event height exceeds database range")?;
        let duck = self.duck.lock().unwrap();
        duck.execute(
            "INSERT OR IGNORE INTO events
             (tx_hash, block_slot, block_height, event_type, instance_id, project_id, payload)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                ev.tx_hash,
                slot,
                height,
                ev.event_type.as_str(),
                ev.instance_id,
                ev.project_id.as_deref().unwrap_or(""),
                ev.payload.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn events_of_type_for_project(
        &self,
        instance: &str,
        project_id: &str,
        event_type: EventType,
    ) -> Result<Vec<Event>> {
        let _guard = self.chain_gate.lock().unwrap();
        if let Some(projection) = self.projection(instance)? {
            self.refresh_event_cache(instance, &projection.events)?;
        }
        let duck = self.duck.lock().unwrap();
        let mut stmt = duck.prepare(
            "SELECT tx_hash, block_slot, block_height, event_type, instance_id, project_id, payload
             FROM events WHERE instance_id = ? AND project_id = ? AND event_type = ? ORDER BY block_slot ASC, tx_hash ASC",
        )?;
        let rows = stmt.query_map(
            duckdb::params![instance, project_id, event_type.as_str()],
            row_to_event,
        )?;
        rows.map(|r| r.context("event row")).collect()
    }

    /// Extract completion evidence for a specific milestone from the event log.
    pub fn completions_for_project(
        &self,
        instance: &str,
        project_id: &str,
    ) -> Result<Vec<Completion>> {
        let events = self.events_of_type_for_project(instance, project_id, EventType::Complete)?;
        let mut out = Vec::new();
        for ev in &events {
            let milestones = match ev.payload.get("milestones").and_then(|m| m.as_object()) {
                Some(m) => m,
                None => continue,
            };
            for (ms_id, ms_val) in milestones {
                let description = ms_val
                    .get("description")
                    .map(crate::parser::rejoined_string)
                    .unwrap_or_default();
                let evidence = ms_val
                    .get("evidence")
                    .and_then(|e| e.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|e| crate::types::CompletionEvidence {
                                anchor_url: e
                                    .get("anchorUrl")
                                    .map(crate::parser::rejoined_string)
                                    .unwrap_or_default(),
                                hash: e
                                    .get("anchorDataHash")
                                    .and_then(|h| h.as_str())
                                    .map(str::to_string),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(Completion {
                    tx_hash: ev.tx_hash.clone(),
                    block_slot: ev.block_slot,
                    milestone_id: ms_id.clone(),
                    description,
                    evidence,
                });
            }
        }
        Ok(out)
    }

    /// Milestones withdrawn: parse Withdraw events to find which milestone IDs
    /// were included. Falls back to inferring from datum diff when metadata omits them.
    pub fn withdrawn_milestones_for_project(
        &self,
        instance: &str,
        project_id: &str,
    ) -> Result<Vec<String>> {
        let events = self.events_of_type_for_project(instance, project_id, EventType::Withdraw)?;
        let mut ids = Vec::new();
        for ev in &events {
            if let Some(ms) = ev.payload.get("milestones").and_then(|m| m.as_object()) {
                ids.extend(ms.keys().cloned());
            }
        }
        Ok(ids)
    }

    // -----------------------------------------------------------------------
    // Registered transactions (from newTransaction mutation)
    // -----------------------------------------------------------------------

    /// Atomically register once per body hash; retries preserve the first record.
    pub fn store_registered_tx(
        &self,
        tx: &RegisteredTx,
        instance: Option<&Instance>,
    ) -> Result<bool> {
        anyhow::ensure!(
            instance.is_none_or(|instance| instance.network == self.network),
            "instance network differs from database network"
        );
        use sled::transaction::Transactional;
        let tree = self.sled.open_tree(TREE_REGISTERED_TXS)?;
        let instances = self.sled.open_tree(TREE_INSTANCES)?;
        let tracked = self.sled.open_tree(TREE_TRACKED_INSTANCES)?;
        let bytes = serde_json::to_vec(tx)?;
        let instance_bytes = instance.map(serde_json::to_vec).transpose()?;
        let tracked_bytes = instance
            .map(|i| {
                serde_json::to_vec(&TrackedInstance {
                    id: i.id.clone(),
                    network: i.network.clone(),
                })
            })
            .transpose()?;
        let result: std::result::Result<bool, sled::transaction::TransactionError<sled::Error>> =
            (&tree, &instances, &tracked).transaction(|(txs, instances, tracked)| {
                if txs.get(tx.tx_hash.as_bytes())?.is_some() {
                    return Ok(false);
                }
                if let Some(inst) = instance {
                    // Registration must not overwrite an already indexed instance.
                    if instances.get(inst.id.as_bytes())?.is_none() {
                        instances.insert(
                            inst.id.as_bytes(),
                            instance_bytes.as_ref().unwrap().as_slice(),
                        )?;
                    }
                    if tracked.get(inst.id.as_bytes())?.is_none() {
                        tracked.insert(
                            inst.id.as_bytes(),
                            tracked_bytes.as_ref().unwrap().as_slice(),
                        )?;
                    }
                }
                txs.insert(tx.tx_hash.as_bytes(), bytes.as_slice())?;
                Ok(true)
            });
        Ok(result?)
    }

    #[cfg(test)]
    pub fn get_registered_txs(&self) -> Result<Vec<RegisteredTx>> {
        let tree = self.sled.open_tree(TREE_REGISTERED_TXS)?;
        tree.iter()
            .map(|r| {
                let (_, v) = r?;
                Ok(serde_json::from_slice(&v)?)
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Tracked instances (discovered from tx metadata)
    // -----------------------------------------------------------------------

    /// Idempotent: safe to call multiple times with the same id.
    pub fn register_instance(&self, id: &str, network: &str) -> Result<()> {
        anyhow::ensure!(
            network == self.network,
            "instance network differs from database network"
        );
        let tree = self.sled.open_tree(TREE_TRACKED_INSTANCES)?;
        let ti = TrackedInstance {
            id: id.to_string(),
            network: network.to_string(),
        };
        tree.insert(id.as_bytes(), serde_json::to_vec(&ti)?)?;
        self.sled.flush()?;
        Ok(())
    }

    pub fn get_tracked_instances(&self) -> Result<Vec<TrackedInstance>> {
        let tree = self.sled.open_tree(TREE_TRACKED_INSTANCES)?;
        tree.iter()
            .map(|r| {
                let (_, v) = r?;
                Ok(serde_json::from_slice(&v)?)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const EVENTS_DDL: &str = "
CREATE TABLE IF NOT EXISTS events (
    tx_hash      VARCHAR NOT NULL,
    block_slot   BIGINT  NOT NULL,
    block_height BIGINT  NOT NULL,
    event_type   VARCHAR NOT NULL,
    instance_id  VARCHAR NOT NULL,
    project_id   VARCHAR NOT NULL DEFAULT '',
    payload      JSON    NOT NULL,
    PRIMARY KEY (instance_id, tx_hash, event_type, project_id)
);
CREATE INDEX IF NOT EXISTS idx_events_project ON events (project_id, block_slot);
CREATE INDEX IF NOT EXISTS idx_events_instance ON events (instance_id, block_slot);
";

fn row_to_event(row: &duckdb::Row) -> duckdb::Result<Event> {
    let event_type_str: String = row.get(3)?;
    let project_id_str: String = row.get(5)?;
    let payload_str: String = row.get(6)?;
    let event_type = EventType::from_str(&event_type_str).ok_or_else(|| {
        duckdb::Error::FromSqlConversionFailure(
            3,
            duckdb::types::Type::Text,
            std::io::Error::new(std::io::ErrorKind::InvalidData, "unknown stored event type")
                .into(),
        )
    })?;
    let payload = serde_json::from_str(&payload_str).map_err(|error| {
        duckdb::Error::FromSqlConversionFailure(6, duckdb::types::Type::Text, Box::new(error))
    })?;
    Ok(Event {
        tx_hash: row.get(0)?,
        block_slot: row.get(1)?,
        block_height: row.get(2)?,
        event_type,
        instance_id: row.get(4)?,
        project_id: if project_id_str.is_empty() {
            None
        } else {
            Some(project_id_str)
        },
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(instance: &str, id: &str) -> Project {
        Project {
            id: id.into(),
            instance_id: instance.into(),
            label: instance.into(),
            description: None,
            vendor_label: "Synthetic".into(),
            milestones: vec![],
            fund_tx: "01".repeat(32),
            ordered_milestones: true,
        }
    }

    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("minos-store-{}-{id}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn database(&self) -> String {
            self.0.join("data").to_str().unwrap().to_owned()
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn composite_project_output_event_and_dedup_identity_isolated_by_instance() {
        let store = Store::temporary();
        for instance in ["a", "a:b"] {
            store
                .upsert_project(&project(instance, "b:project"))
                .unwrap();
            store
                .upsert_utxo(&VendorUtxo {
                    instance_id: instance.into(),
                    tx_hash: instance.into(),
                    output_index: 0,
                    project_id: "b:project".into(),
                    address: "synthetic".into(),
                    datum_hex: "".into(),
                    lovelace: 1,
                    assets: vec![],
                    slot: 1,
                    payout_ids: vec![],
                })
                .unwrap();
            store
                .append_event(&Event {
                    tx_hash: "same-transaction".into(),
                    block_slot: 1,
                    block_height: 1,
                    event_type: EventType::Complete,
                    instance_id: instance.into(),
                    project_id: Some("b:project".into()),
                    payload: serde_json::json!({"instance": instance}),
                })
                .unwrap();
        }
        store.mark_indexed("a", "same-transaction", 1).unwrap();
        assert!(!store.is_indexed("a:b", "same-transaction").unwrap());
        for instance in ["a", "a:b"] {
            assert_eq!(
                store
                    .get_project(instance, "b:project")
                    .unwrap()
                    .unwrap()
                    .label,
                instance
            );
            assert_eq!(
                store
                    .get_utxo(instance, "b:project")
                    .unwrap()
                    .unwrap()
                    .tx_hash,
                instance
            );
            let events = store
                .events_of_type_for_project(instance, "b:project", EventType::Complete)
                .unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].instance_id, instance);
        }
        assert_ne!(
            scoped_key("a", "b:c").unwrap(),
            scoped_key("a:b", "c").unwrap()
        );
    }

    #[test]
    fn reset_is_scoped_and_retains_local_registrations() {
        let store = Store::temporary();
        for instance in ["a", "b"] {
            store.upsert_project(&project(instance, "same-id")).unwrap();
            store.mark_indexed(instance, "tx", 1).unwrap();
            store
                .set_sync(&format!("sync_slot_{instance}"), "1")
                .unwrap();
            store
                .append_event(&Event {
                    tx_hash: "tx".into(),
                    block_slot: 1,
                    block_height: 1,
                    event_type: EventType::Complete,
                    instance_id: instance.into(),
                    project_id: Some("same-id".into()),
                    payload: serde_json::json!({}),
                })
                .unwrap();
        }
        store
            .store_registered_tx(
                &RegisteredTx {
                    tx_hash: "local".into(),
                    label: "Synthetic".into(),
                    cbor_hex: "00".into(),
                    submitted_at_ms: 1,
                },
                None,
            )
            .unwrap();
        store.reset_sync("a").unwrap();
        store.reset_sync("a").unwrap();
        assert!(store.get_project("a", "same-id").unwrap().is_none());
        assert!(store.get_project("b", "same-id").unwrap().is_some());
        assert!(!store.is_indexed("a", "tx").unwrap());
        assert!(store.is_indexed("b", "tx").unwrap());
        assert!(store.get_sync("sync_slot_a").unwrap().is_none());
        assert_eq!(store.get_sync("sync_slot_b").unwrap().unwrap(), "1");
        assert!(store
            .events_of_type_for_project("a", "same-id", EventType::Complete)
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .events_of_type_for_project("b", "same-id", EventType::Complete)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(store.get_registered_txs().unwrap().len(), 1);
    }

    #[test]
    fn legacy_schema_is_rejected_without_altering_registered_data() {
        let directory = Directory::new();
        let path = directory.database();
        let original = br#"{"tx_hash":"synthetic","cbor_hex":"00"}"#;
        {
            let db = sled::open(&path).unwrap();
            db.open_tree(TREE_REGISTERED_TXS)
                .unwrap()
                .insert("synthetic", original.as_slice())
                .unwrap();
            db.flush().unwrap();
        }
        let error = Store::open(&path, "preview").err().unwrap();
        assert!(error.to_string().contains("legacy database"));
        let db = sled::open(&path).unwrap();
        assert_eq!(
            db.open_tree(TREE_REGISTERED_TXS)
                .unwrap()
                .get("synthetic")
                .unwrap()
                .unwrap()
                .as_ref(),
            original
        );
        assert!(db
            .open_tree(TREE_METADATA)
            .unwrap()
            .get("schema")
            .unwrap()
            .is_none());
        assert!(!std::path::Path::new(&format!("{path}-events.duckdb")).exists());
    }

    #[test]
    fn schema_and_network_are_persisted_and_checked_before_serving() {
        let directory = Directory::new();
        let path = directory.database();
        drop(Store::open(&path, "preview").unwrap());
        drop(Store::open(&path, "preview").unwrap());
        assert!(Store::open(&path, "mainnet").is_err());
        {
            let db = sled::open(&path).unwrap();
            db.open_tree(TREE_METADATA)
                .unwrap()
                .insert(
                    "schema",
                    serde_json::to_vec(&serde_json::json!({"version": 999, "network": "preview"}))
                        .unwrap(),
                )
                .unwrap();
            db.flush().unwrap();
        }
        assert!(Store::open(&path, "preview").is_err());
    }

    #[test]
    fn event_json_is_built_in_and_round_trips_without_extension_installation() {
        let duck = Connection::open_in_memory().unwrap();
        duck.execute_batch("SET autoinstall_known_extensions = false;")
            .unwrap();
        let install_path: String = duck
            .query_row(
                "SELECT install_path FROM duckdb_extensions() WHERE extension_name = 'json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // A developer's cached extension must not make this regression pass.
        assert_eq!(install_path, "(BUILT-IN)");
        duck.execute_batch(EVENTS_DDL).unwrap();
        let store = Store {
            sled: sled::Config::new().temporary(true).open().unwrap(),
            duck: Mutex::new(duck),
            network: "preview".into(),
            chain_gate: Mutex::new(()),
            chain_cache: Mutex::new(Default::default()),
            failure_boundary: std::sync::atomic::AtomicU8::new(0),
        };
        let payload = serde_json::json!({
            "milestones": {"m-0": {"description": "Synthetic evidence ✓", "evidence": []}},
            "optional": null,
            "values": [true, 123, "text"],
        });
        store
            .append_event(&Event {
                tx_hash: "01".repeat(32),
                block_slot: 1,
                block_height: 1,
                event_type: EventType::Complete,
                instance_id: "02".repeat(28),
                project_id: Some("EXAMPLE-001".into()),
                payload: payload.clone(),
            })
            .unwrap();
        let events = store
            .events_of_type_for_project(&"02".repeat(28), "EXAMPLE-001", EventType::Complete)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload, payload);
    }
}
