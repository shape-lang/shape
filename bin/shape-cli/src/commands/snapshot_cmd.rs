use anyhow::{Context, Result};
use shape_runtime::hashing::HashDigest;
use shape_runtime::snapshot::SnapshotStore;
use std::path::PathBuf;

/// Get the default snapshot store path
fn snapshot_store() -> Result<SnapshotStore> {
    let root = dirs::data_local_dir()
        .map(|dir| dir.join("shape").join("snapshots"))
        .unwrap_or_else(|| PathBuf::from(".shape").join("snapshots"));
    SnapshotStore::new(root).context("failed to open snapshot store")
}

/// List all saved snapshots
pub async fn run_snapshot_list() -> Result<()> {
    let store = snapshot_store()?;
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

/// Show detailed info about a snapshot
pub async fn run_snapshot_info(hash_str: String) -> Result<()> {
    let store = snapshot_store()?;
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

/// Delete a snapshot
pub async fn run_snapshot_delete(hash_str: String) -> Result<()> {
    let store = snapshot_store()?;
    let hash = resolve_hash(&store, &hash_str)?;
    store.delete_snapshot(&hash)?;
    println!("Deleted snapshot {}", hash.hex());
    Ok(())
}

/// Resolve a hash prefix to a full hash by scanning available snapshots.
fn resolve_hash(store: &SnapshotStore, prefix: &str) -> Result<HashDigest> {
    // Try exact match first
    let exact = HashDigest::from_hex(prefix);
    if store.get_snapshot(&exact).is_ok() {
        return Ok(exact);
    }

    // Try prefix match
    let snapshots = store.list_snapshots()?;
    let matches: Vec<_> = snapshots
        .iter()
        .filter(|(h, _)| h.hex().starts_with(prefix))
        .collect();

    match matches.len() {
        0 => anyhow::bail!("No snapshot found matching '{}'", prefix),
        1 => Ok(matches[0].0.clone()),
        n => anyhow::bail!("Ambiguous hash prefix '{}' matches {} snapshots", prefix, n),
    }
}
