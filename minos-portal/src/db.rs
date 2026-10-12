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
    sled: sled::Db,
    duck: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &str) -> Result<Self> {
        let sled = sled::open(path).with_context(|| format!("open sled at {path}"))?;

        let events_path = format!("{path}-events.duckdb");
        let duck = Connection::open(&events_path)
            .with_context(|| format!("open duckdb at {events_path}"))?;
        duck.execute_batch(EVENTS_DDL)
            .context("create events table")?;

        Ok(Self {
            sled,
            duck: Mutex::new(duck),
        })
    }

    /// Delete the chain-sync checkpoint and all indexed-tx dedup entries so the
    /// instance replays from the publish tx and reprocesses every transaction.
    pub fn reset_sync(&self, instance_id: &str) -> Result<()> {
        // Clear slot/hash checkpoint
        let tree = self.sled.open_tree(TREE_SYNC)?;
        tree.remove(format!("sync_slot_{instance_id}").as_bytes())?;
        tree.remove(format!("sync_hash_{instance_id}").as_bytes())?;

        // Clear the entire indexed-tx dedup set so every tx is reprocessed
        let ix_tree = self.sled.open_tree(TREE_INDEXED_TXS)?;
        ix_tree.clear()?;

        // Wipe and recreate the events table so it's rebuilt cleanly
        let duck = self.duck.lock().unwrap();
        duck.execute_batch("DROP TABLE IF EXISTS events;")?;
        duck.execute_batch(EVENTS_DDL)?;

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

    pub fn upsert_instance(&self, inst: &Instance) -> Result<()> {
        let tree = self.sled.open_tree(TREE_INSTANCES)?;
        let bytes = serde_json::to_vec(inst)?;
        tree.insert(inst.id.as_bytes(), bytes)?;
        Ok(())
    }

    pub fn get_instance(&self, id: &str) -> Result<Option<Instance>> {
        let tree = self.sled.open_tree(TREE_INSTANCES)?;
        match tree.get(id.as_bytes())? {
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn all_instances(&self) -> Result<Vec<Instance>> {
        let tree = self.sled.open_tree(TREE_INSTANCES)?;
        tree.iter()
            .map(|r| {
                let (_, v) = r?;
                Ok(serde_json::from_slice(&v)?)
            })
            .collect::<Result<Vec<_>>>()
    }

    // -----------------------------------------------------------------------
    // Projects
    // -----------------------------------------------------------------------

    pub fn upsert_project(&self, proj: &Project) -> Result<()> {
        let tree = self.sled.open_tree(TREE_PROJECTS)?;
        let bytes = serde_json::to_vec(proj)?;
        tree.insert(proj.id.as_bytes(), bytes)?;
        Ok(())
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let tree = self.sled.open_tree(TREE_PROJECTS)?;
        match tree.get(id.as_bytes())? {
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn projects_for_instance(&self, instance_id: &str) -> Result<Vec<Project>> {
        let tree = self.sled.open_tree(TREE_PROJECTS)?;
        tree.iter()
            .filter_map(|r| {
                let (_, v) = r.ok()?;
                let p: Project = serde_json::from_slice(&v).ok()?;
                if p.instance_id == instance_id {
                    Some(Ok(p))
                } else {
                    None
                }
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Current vendor UTxO (one per project)
    // -----------------------------------------------------------------------

    pub fn upsert_utxo(&self, utxo: &VendorUtxo) -> Result<()> {
        let tree = self.sled.open_tree(TREE_UTXOS)?;
        let bytes = serde_json::to_vec(utxo)?;
        tree.insert(utxo.project_id.as_bytes(), bytes)?;
        Ok(())
    }

    pub fn get_utxo(&self, project_id: &str) -> Result<Option<VendorUtxo>> {
        let tree = self.sled.open_tree(TREE_UTXOS)?;
        match tree.get(project_id.as_bytes())? {
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            None => Ok(None),
        }
    }

    // -----------------------------------------------------------------------
    // Indexed tx dedup
    // -----------------------------------------------------------------------

    pub fn is_indexed(&self, tx_hash: &str) -> Result<bool> {
        let tree = self.sled.open_tree(TREE_INDEXED_TXS)?;
        Ok(tree.contains_key(tx_hash.as_bytes())?)
    }

    pub fn mark_indexed(&self, tx_hash: &str, slot: u64) -> Result<()> {
        let tree = self.sled.open_tree(TREE_INDEXED_TXS)?;
        tree.insert(tx_hash.as_bytes(), &slot.to_be_bytes())?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Event log (DuckDB)
    // -----------------------------------------------------------------------

    pub fn rollback_events_after(&self, instance_id: &str, slot: u64) -> Result<()> {
        let duck = self.duck.lock().unwrap();
        duck.execute(
            "DELETE FROM events WHERE instance_id = ? AND block_slot > ?",
            duckdb::params![instance_id, slot as i64],
        )?;
        Ok(())
    }

    pub fn append_event(&self, ev: &Event) -> Result<()> {
        let duck = self.duck.lock().unwrap();
        duck.execute(
            "INSERT OR IGNORE INTO events
             (tx_hash, block_slot, block_height, event_type, instance_id, project_id, payload)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                ev.tx_hash,
                ev.block_slot as i64,
                ev.block_height as i64,
                ev.event_type.as_str(),
                ev.instance_id,
                ev.project_id.as_deref().unwrap_or(""),
                ev.payload.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn events_for_project(&self, project_id: &str) -> Result<Vec<Event>> {
        let duck = self.duck.lock().unwrap();
        let mut stmt = duck.prepare(
            "SELECT tx_hash, block_slot, block_height, event_type, instance_id, project_id, payload
             FROM events WHERE project_id = ? ORDER BY block_slot ASC",
        )?;
        let rows = stmt.query_map([project_id], row_to_event)?;
        rows.map(|r| r.context("event row")).collect()
    }

    pub fn events_of_type_for_project(
        &self,
        project_id: &str,
        event_type: EventType,
    ) -> Result<Vec<Event>> {
        let duck = self.duck.lock().unwrap();
        let mut stmt = duck.prepare(
            "SELECT tx_hash, block_slot, block_height, event_type, instance_id, project_id, payload
             FROM events WHERE project_id = ? AND event_type = ? ORDER BY block_slot ASC",
        )?;
        let rows = stmt.query_map(
            duckdb::params![project_id, event_type.as_str()],
            row_to_event,
        )?;
        rows.map(|r| r.context("event row")).collect()
    }

    /// Extract completion evidence for a specific milestone from the event log.
    pub fn completions_for_project(&self, project_id: &str) -> Result<Vec<Completion>> {
        let events = self.events_of_type_for_project(project_id, EventType::Complete)?;
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
    pub fn withdrawn_milestones_for_project(&self, project_id: &str) -> Result<Vec<String>> {
        let events = self.events_of_type_for_project(project_id, EventType::Withdraw)?;
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

    pub fn store_registered_tx(&self, tx: &RegisteredTx) -> Result<()> {
        let tree = self.sled.open_tree(TREE_REGISTERED_TXS)?;
        let bytes = serde_json::to_vec(tx)?;
        tree.insert(tx.tx_hash.as_bytes(), bytes)?;
        Ok(())
    }

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
        let tree = self.sled.open_tree(TREE_TRACKED_INSTANCES)?;
        let ti = TrackedInstance {
            id: id.to_string(),
            network: network.to_string(),
        };
        tree.insert(id.as_bytes(), serde_json::to_vec(&ti)?)?;
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
    PRIMARY KEY (tx_hash, event_type, project_id)
);
CREATE INDEX IF NOT EXISTS idx_events_project ON events (project_id, block_slot);
CREATE INDEX IF NOT EXISTS idx_events_instance ON events (instance_id, block_slot);
";

fn row_to_event(row: &duckdb::Row) -> duckdb::Result<Event> {
    let event_type_str: String = row.get(3)?;
    let project_id_str: String = row.get(5)?;
    let payload_str: String = row.get(6)?;
    Ok(Event {
        tx_hash: row.get(0)?,
        block_slot: row.get::<_, i64>(1)? as u64,
        block_height: row.get::<_, i64>(2)? as u64,
        event_type: EventType::from_str(&event_type_str).unwrap_or(EventType::Publish),
        instance_id: row.get(4)?,
        project_id: if project_id_str.is_empty() {
            None
        } else {
            Some(project_id_str)
        },
        payload: serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null),
    })
}
