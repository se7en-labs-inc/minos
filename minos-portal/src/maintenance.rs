//! Offline migration copies local registrations into a new database. The source
//! (including its DuckDB file) is never modified. Derived chain state is replayed.
use crate::{
    db::{RegisteredTx, TrackedInstance},
    types::Instance,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Serialize, Deserialize)]
struct Backup {
    format: u32,
    trees: Vec<(String, Vec<(String, String)>)>,
}
fn snapshot(db: &sled::Db) -> Result<Backup> {
    let mut trees = Vec::new();
    for name in db.tree_names() {
        let tree = db.open_tree(&name)?;
        let rows = tree
            .iter()
            .map(|row| {
                let (key, value) = row?;
                Ok((hex::encode(key), hex::encode(value)))
            })
            .collect::<Result<Vec<_>>>()?;
        trees.push((hex::encode(name), rows));
    }
    trees.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Backup { format: 1, trees })
}
fn write_backup(path: &Path, backup: &Backup) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("create new backup file (existing files are refused)")?;
    file.write_all(&serde_json::to_vec(backup)?)?;
    file.sync_all()?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
pub fn backup(source: &str, destination: &Path) -> Result<()> {
    ensure!(
        Path::new(source).is_dir(),
        "source database directory does not exist"
    );
    let db = sled::Config::default()
        .path(source)
        .flush_every_ms(None)
        .open()
        .context("open database exclusively; stop the portal first")?;
    write_backup(destination, &snapshot(&db)?)
}
pub fn migrate(source: &str, destination: &str, network: &str) -> Result<()> {
    ensure!(
        Path::new(source).is_dir(),
        "source database directory does not exist"
    );
    ensure!(
        source != destination,
        "migration requires a separate destination"
    );
    let original = sled::Config::default()
        .path(source)
        .flush_every_ms(None)
        .open()
        .context("open source exclusively; stop the portal first")?;
    if original
        .tree_names()
        .iter()
        .any(|name| name.as_ref() == b"metadata")
    {
        if let Some(schema) = original.open_tree("metadata")?.get("schema")? {
            let schema: serde_json::Value = serde_json::from_slice(&schema)?;
            ensure!(
                schema["network"] == network,
                "source database network differs"
            );
            ensure!(
                schema["version"]
                    .as_u64()
                    .is_some_and(|version| (2..=crate::db::SCHEMA_VERSION).contains(&version)),
                "unsupported source schema version"
            );
        }
    }
    let export = snapshot(&original)?;
    let marker = serde_json::to_vec(
        &serde_json::json!({"network":network,"sourceHash":pallas_crypto::hash::Hasher::<256>::hash(&serde_json::to_vec(&export)?).to_string()}),
    )?;
    // Validate retained records before creating the destination. No historical
    // event or current-output projection is carried across the trust boundary.
    let mut retained = Vec::new();
    for name in ["registered_txs", "tracked_instances", "instances"] {
        let mut rows = Vec::new();
        if original
            .tree_names()
            .iter()
            .any(|tree| tree.as_ref() == name.as_bytes())
        {
            for row in &original.open_tree(name)? {
                let (key, value) = row?;
                let bytes = match name {
                    "registered_txs" => {
                        let _: RegisteredTx = serde_json::from_slice(&value)
                            .context("invalid local transaction registration")?;
                        value.to_vec()
                    }
                    "tracked_instances" => {
                        let record: TrackedInstance = serde_json::from_slice(&value)?;
                        ensure!(
                            record.network == network,
                            "tracked instance network mismatch"
                        );
                        value.to_vec()
                    }
                    _ => {
                        let mut instance: Instance = serde_json::from_slice(&value)?;
                        ensure!(instance.network == network, "instance network mismatch");
                        instance.confirmed = false;
                        instance.treasury_ref = None;
                        instance.vendor_ref = None;
                        instance.registry_ref = None;
                        serde_json::to_vec(&instance)?
                    }
                };
                rows.push((key.to_vec(), bytes));
            }
        }
        retained.push((name, rows));
    }
    let destination_path = Path::new(destination);
    let existed = destination_path.exists();
    let target = sled::Config::default()
        .path(destination)
        .flush_every_ms(None)
        .open()
        .context("open migration destination exclusively")?;
    let meta = target.open_tree("metadata")?;
    if existed {
        ensure!(
            meta.get("migration_in_progress")?.is_some(),
            "destination already exists and is not an interrupted migration"
        );
    } else {
        meta.insert("migration_in_progress", marker.as_slice())?;
        target.flush()?;
    }
    ensure!(
        meta.get("migration_in_progress")?.as_deref() == Some(marker.as_slice()),
        "source or network changed since migration began; use a new destination"
    );
    let backup_path = destination_path.join("source-backup.json");
    if backup_path.exists() {
        let valid = std::fs::File::open(&backup_path)
            .ok()
            .and_then(|file| serde_json::from_reader::<_, Backup>(file).ok());
        if valid.is_none() {
            std::fs::remove_file(&backup_path)?;
        }
    }
    if !backup_path.exists() {
        write_backup(&backup_path, &export)?;
    }
    // On resume, compare the preserved export before accepting a changed source.
    let previous: Backup = serde_json::from_reader(std::fs::File::open(
        destination_path.join("source-backup.json"),
    )?)?;
    ensure!(
        serde_json::to_value(previous)? == serde_json::to_value(export)?,
        "source changed since migration began; use a new destination"
    );
    for (name, rows) in retained {
        let tree = target.open_tree(name)?;
        let mut batch = sled::Batch::default();
        for row in &tree {
            batch.remove(row?.0);
        }
        for (key, value) in rows {
            batch.insert(key, value);
        }
        tree.apply_batch(batch)?;
    }
    meta.insert(
        "schema",
        serde_json::to_vec(
            &serde_json::json!({"version":crate::db::SCHEMA_VERSION,"network":network}),
        )?,
    )?;
    target.flush()?;
    meta.remove("migration_in_progress")?;
    target.flush()?;
    drop(meta);
    drop(target);
    Ok(())
}

pub fn restore(backup: &Path, destination: &str) -> Result<()> {
    let snapshot: Backup = serde_json::from_reader(std::fs::File::open(backup)?)?;
    ensure!(snapshot.format == 1, "unsupported backup format");
    let had_metadata = snapshot
        .trees
        .iter()
        .any(|(name, _)| name == &hex::encode(b"metadata"));
    let digest =
        pallas_crypto::hash::Hasher::<256>::hash(&serde_json::to_vec(&snapshot)?).to_string();
    let existed = Path::new(destination).exists();
    let db = sled::Config::default()
        .path(destination)
        .flush_every_ms(None)
        .open()?;
    let meta = db.open_tree("metadata")?;
    if existed {
        ensure!(
            meta.get("restore_in_progress")?.as_deref() == Some(digest.as_bytes()),
            "restore destination exists or belongs to a different backup"
        );
    } else {
        meta.insert("restore_in_progress", digest.as_bytes())?;
        db.flush()?;
    }
    let mut metadata = None;
    for (name, rows) in snapshot.trees {
        let name = hex::decode(name)?;
        if name == b"metadata" {
            metadata = Some(rows);
            continue;
        }
        let tree = db.open_tree(&name)?;
        let mut batch = sled::Batch::default();
        for row in &tree {
            batch.remove(row?.0);
        }
        for (key, value) in rows {
            batch.insert(hex::decode(key)?, hex::decode(value)?);
        }
        tree.apply_batch(batch)?;
    }
    if let Some(rows) = metadata {
        let mut batch = sled::Batch::default();
        for row in &meta {
            let (key, _) = row?;
            if key.as_ref() != b"restore_in_progress" {
                batch.remove(key);
            }
        }
        for (key, value) in rows {
            let key = hex::decode(key)?;
            ensure!(
                key != b"restore_in_progress",
                "cannot restore an unfinished restore backup"
            );
            batch.insert(key, hex::decode(value)?);
        }
        meta.apply_batch(batch)?;
    }
    db.flush()?;
    meta.remove("restore_in_progress")?;
    db.flush()?;
    drop(meta);
    if !had_metadata {
        db.drop_tree("metadata")?;
        db.flush()?;
    }
    drop(db);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Store;

    fn operation(root: &Path, destination: &str, name: &str, expected_error: &str) {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "maintenance::tests::maintenance_operation_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("MINOS_TEST_MAINTENANCE_ROOT", root)
            .env("MINOS_TEST_MAINTENANCE_DESTINATION", destination)
            .env("MINOS_TEST_MAINTENANCE_OPERATION", name)
            .env("MINOS_TEST_MAINTENANCE_ERROR", expected_error)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    // Every operation owns its database handles until process exit, just as the
    // offline CLI does. Reopen and failure assertions run in fresh processes.
    #[test]
    fn maintenance_operation_child() {
        let Ok(root) = std::env::var("MINOS_TEST_MAINTENANCE_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let source = root.join("legacy");
        let destination = root.join(std::env::var("MINOS_TEST_MAINTENANCE_DESTINATION").unwrap());
        let backup_path = root.join("manual-backup.json");
        let name = std::env::var("MINOS_TEST_MAINTENANCE_OPERATION").unwrap();
        let run = || -> Result<()> {
            match name.as_str() {
                "create" => {
                    let db = sled::open(&source)?;
                    let registered = RegisteredTx {
                        tx_hash: "local".into(),
                        label: "synthetic".into(),
                        cbor_hex: "00".into(),
                        submitted_at_ms: 1,
                    };
                    db.open_tree("registered_txs")?
                        .insert("local", serde_json::to_vec(&registered)?)?;
                    db.open_tree("tracked_instances")?.insert(
                        "one",
                        serde_json::to_vec(&TrackedInstance {
                            id: "one".into(),
                            network: "preview".into(),
                        })?,
                    )?;
                    db.open_tree("projects")?
                        .insert("ambiguous-legacy-id", b"must replay")?;
                    db.flush()?;
                    std::fs::write(
                        root.join("expected-source.json"),
                        serde_json::to_vec(&snapshot(&db)?)?,
                    )?;
                }
                "backup" => backup(source.to_str().unwrap(), &backup_path)?,
                "migrate" => migrate(
                    source.to_str().unwrap(),
                    destination.to_str().unwrap(),
                    "preview",
                )?,
                "verify-migration" => {
                    let store = Store::open(destination.to_str().unwrap(), "preview")?;
                    assert_eq!(store.get_registered_txs()?[0].cbor_hex, "00");
                    assert_eq!(store.get_registered_txs()?.len(), 1);
                    assert_eq!(store.get_tracked_instances()?.len(), 1);
                    assert!(store.projects_for_instance("one")?.is_empty());
                    let original = sled::open(&source)?;
                    let expected: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(root.join("expected-source.json"))?)?;
                    assert_eq!(serde_json::to_value(snapshot(&original)?)?, expected);
                    let preserved: serde_json::Value = serde_json::from_slice(&std::fs::read(
                        destination.join("source-backup.json"),
                    )?)?;
                    assert_eq!(preserved, expected);
                }
                "restore" => restore(&backup_path, destination.to_str().unwrap())?,
                "verify-restore" => {
                    let db = sled::open(&destination)?;
                    let expected: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(root.join("expected-source.json"))?)?;
                    assert_eq!(serde_json::to_value(snapshot(&db)?)?, expected);
                }
                "interrupt" => {
                    let source_db = sled::open(&source)?;
                    let export = snapshot(&source_db)?;
                    let marker = serde_json::to_vec(
                        &serde_json::json!({"network":"preview","sourceHash":pallas_crypto::hash::Hasher::<256>::hash(&serde_json::to_vec(&export)?).to_string()}),
                    )?;
                    let db = sled::open(&destination)?;
                    db.open_tree("metadata")?
                        .insert("migration_in_progress", marker)?;
                    db.open_tree("registered_txs")?
                        .insert("partial", b"interrupted")?;
                    std::fs::write(destination.join("source-backup.json"), b"partial JSON")?;
                    db.flush()?;
                }
                "startup" => {
                    Store::open(destination.to_str().unwrap(), "preview")?;
                }
                _ => anyhow::bail!("unknown test operation"),
            }
            Ok(())
        };
        let expected = std::env::var("MINOS_TEST_MAINTENANCE_ERROR").unwrap();
        if expected.is_empty() {
            run().unwrap();
        } else {
            let error = run().unwrap_err();
            assert!(
                format!("{error:#}").contains(&expected),
                "unexpected {name} error: {error:#}"
            );
        }
    }

    #[test]
    fn migration_preserves_local_records_and_source_and_discards_projections() {
        let root = std::env::temp_dir().join(format!(
            "minos-migrate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        operation(&root, "new", "create", "");
        operation(&root, "new", "backup", "");
        operation(&root, "new", "backup", "exists");
        operation(&root, "new", "migrate", "");
        operation(&root, "new", "verify-migration", "");
        operation(&root, "new", "migrate", "destination already exists");
        operation(&root, "restored", "restore", "");
        operation(&root, "restored", "verify-restore", "");
        operation(&root, "restored", "restore", "restore destination exists");
        operation(&root, "interrupted", "interrupt", "");
        operation(&root, "interrupted", "startup", "migration interrupted");
        operation(&root, "interrupted", "migrate", "");
        operation(&root, "interrupted", "verify-migration", "");
        std::fs::remove_dir_all(root).unwrap();
    }
}
