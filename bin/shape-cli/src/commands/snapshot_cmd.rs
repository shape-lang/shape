use anyhow::{Context, Result};
use shape_runtime::hashing::HashDigest;
use shape_runtime::snapshot::SnapshotStore;
use std::path::PathBuf;

const SNAPSHOT_STORE_ENV: &str = "SHAPE_SNAPSHOT_STORE";

/// Resolve the snapshot store root shared by `run`, `snapshot`, and `serve`.
pub(crate) fn snapshot_store_root(explicit_root: Option<PathBuf>) -> PathBuf {
    explicit_root
        .or_else(|| std::env::var_os(SNAPSHOT_STORE_ENV).map(PathBuf::from))
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .map(|dir| dir.join("shape").join("snapshots"))
                .unwrap_or_else(|| PathBuf::from(".shape").join("snapshots"))
        })
}

/// Open the selected snapshot store.
pub(crate) fn open_snapshot_store(explicit_root: Option<PathBuf>) -> Result<SnapshotStore> {
    let root = snapshot_store_root(explicit_root);
    SnapshotStore::new(root).context("failed to open snapshot store")
}

/// List all saved snapshots.
pub async fn run_snapshot_list(snapshot_store_root: Option<PathBuf>) -> Result<()> {
    let store = open_snapshot_store(snapshot_store_root)?;
    let snapshots = store.list_snapshots()?;

    if snapshots.is_empty() {
        println!("No snapshots found.");
        return Ok(());
    }

    println!("{:<16}  {:<24}  {}", "HASH", "CREATED", "SCRIPT");
    for (hash, snap) in &snapshots {
        let created = chrono::DateTime::from_timestamp_millis(snap.created_at_ms)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let script = snap.script_path.as_deref().unwrap_or("-");
        let short_hash = &hash.hex()[..16.min(hash.hex().len())];
        println!("{:<16}  {:<24}  {}", short_hash, created, script);
    }

    println!("\n{} snapshot(s)", snapshots.len());
    Ok(())
}

/// Show detailed info about a snapshot.
pub async fn run_snapshot_info(
    hash_str: String,
    snapshot_store_root: Option<PathBuf>,
) -> Result<()> {
    let store = open_snapshot_store(snapshot_store_root)?;
    let hash = resolve_hash(&store, &hash_str)?;
    let snap = store.get_snapshot(&hash)?;

    println!("Hash:       {}", hash.hex());
    println!("Version:    {}", snap.version);
    let created = chrono::DateTime::from_timestamp_millis(snap.created_at_ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("Created:    {}", created);
    println!("Script:     {}", snap.script_path.as_deref().unwrap_or("-"));
    println!(
        "VM state:   {}",
        if snap.vm_hash.is_some() { "yes" } else { "no" }
    );
    println!(
        "Bytecode:   {}",
        if snap.bytecode_hash.is_some() {
            "yes"
        } else {
            "no"
        }
    );

    Ok(())
}

/// Delete a snapshot.
pub async fn run_snapshot_delete(
    hash_str: String,
    snapshot_store_root: Option<PathBuf>,
) -> Result<()> {
    let store = open_snapshot_store(snapshot_store_root)?;
    let hash = resolve_hash(&store, &hash_str)?;
    store.delete_snapshot(&hash)?;
    println!("Deleted snapshot {}", hash.hex());
    Ok(())
}

/// Resolve a hash prefix to a full hash by scanning available snapshots.
///
/// Delegates to [`SnapshotStore::resolve_hash`] so the `--resume` path and
/// the `snapshot info`/`rm` subcommands share one resolution rule.
fn resolve_hash(store: &SnapshotStore, prefix: &str) -> Result<HashDigest> {
    store.resolve_hash(prefix)
}
