//! Hot-reload support for content-addressed function blobs.
//!
//! When source files change, only affected functions are recompiled.
//! Old blobs remain valid for in-flight frames. New calls use updated blobs.

use crate::bytecode::{FunctionBlob, FunctionHash};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Tracks which blobs are active and manages hot-reload updates.
pub struct HotReloader {
    /// Current name -> hash mapping.
    current_mappings: HashMap<String, FunctionHash>,
    /// All known blobs (old versions kept until GC).
    blob_store: HashMap<FunctionHash, FunctionBlob>,
    /// Blobs that are referenced by active frames (not GC-eligible).
    active_references: HashSet<FunctionHash>,
    /// History of updates for rollback support.
    update_history: Vec<ReloadEvent>,
    /// Optional file watch paths.
    watch_paths: Vec<PathBuf>,
}

/// Record of a hot-reload event.
#[derive(Debug, Clone)]
pub struct ReloadEvent {
    pub timestamp: std::time::Instant,
    pub old_hash: FunctionHash,
    pub new_hash: FunctionHash,
    pub function_name: String,
}

/// Result of applying a hot-reload update.
#[derive(Debug)]
pub struct ReloadResult {
    pub functions_updated: Vec<String>,
    pub functions_unchanged: Vec<String>,
    pub old_blobs_retained: usize,
}

/// Patch describing a single function update.
#[derive(Debug, Clone)]
pub struct FunctionPatch {
    pub function_name: String,
    pub old_hash: Option<FunctionHash>,
    pub new_blob: FunctionBlob,
}

impl HotReloader {
    pub fn new() -> Self {
        Self {
            current_mappings: HashMap::new(),
            blob_store: HashMap::new(),
            active_references: HashSet::new(),
            update_history: Vec::new(),
            watch_paths: Vec::new(),
        }
    }

    /// Register a function's current blob.
    pub fn register_function(&mut self, name: String, blob: FunctionBlob) {
        let hash = blob.content_hash;
        self.current_mappings.insert(name, hash);
        self.blob_store.insert(hash, blob);
    }

    /// Mark a blob hash as actively referenced (in a call frame).
    pub fn add_active_reference(&mut self, hash: FunctionHash) {
        self.active_references.insert(hash);
    }

    /// Remove an active reference (frame exited).
    pub fn remove_active_reference(&mut self, hash: FunctionHash) {
        self.active_references.remove(&hash);
    }

    /// Apply a set of function patches (updated blobs).
    pub fn apply_patches(&mut self, patches: Vec<FunctionPatch>) -> ReloadResult {
        let mut functions_updated = Vec::new();
        let mut functions_unchanged = Vec::new();
        let mut old_blobs_retained: usize = 0;

        for patch in patches {
            let new_hash = patch.new_blob.content_hash;

            // Check if the function already maps to this exact hash (unchanged).
            if let Some(&existing_hash) = self.current_mappings.get(&patch.function_name) {
                if existing_hash == new_hash {
                    functions_unchanged.push(patch.function_name);
                    continue;
                }

                // Record the reload event.
                self.update_history.push(ReloadEvent {
                    timestamp: std::time::Instant::now(),
                    old_hash: existing_hash,
                    new_hash,
                    function_name: patch.function_name.clone(),
                });

                // If the old blob is actively referenced, it stays in the store.
                if self.active_references.contains(&existing_hash) {
                    old_blobs_retained += 1;
                }
            }

            // Update the mapping and store the new blob.
            self.current_mappings
                .insert(patch.function_name.clone(), new_hash);
            self.blob_store.insert(new_hash, patch.new_blob);
            functions_updated.push(patch.function_name);
        }

        ReloadResult {
            functions_updated,
            functions_unchanged,
            old_blobs_retained,
        }
    }

    /// Get the current hash for a function name.
    pub fn current_hash(&self, name: &str) -> Option<&FunctionHash> {
        self.current_mappings.get(name)
    }

    /// Get a blob by hash (works for both current and retained old blobs).
    pub fn get_blob(&self, hash: &FunctionHash) -> Option<&FunctionBlob> {
        self.blob_store.get(hash)
    }

    /// Run garbage collection: remove old blobs with no active references.
    ///
    /// Returns the number of blobs removed.
    pub fn gc(&mut self) -> usize {
        // Collect the set of hashes that are currently mapped (live).
        let live_hashes: HashSet<FunctionHash> = self.current_mappings.values().copied().collect();

        // Find blobs that are neither live nor actively referenced.
        let to_remove: Vec<FunctionHash> = self
            .blob_store
            .keys()
            .filter(|hash| !live_hashes.contains(hash) && !self.active_references.contains(hash))
            .copied()
            .collect();

        let removed = to_remove.len();
        for hash in to_remove {
            self.blob_store.remove(&hash);
        }
        removed
    }

    /// Get reload history.
    pub fn history(&self) -> &[ReloadEvent] {
        &self.update_history
    }

    /// Add a path to watch for changes.
    pub fn watch_path(&mut self, path: PathBuf) {
        self.watch_paths.push(path);
    }

    /// Get all watched paths.
    pub fn watched_paths(&self) -> &[PathBuf] {
        &self.watch_paths
    }

    /// Compute which functions need recompilation given a set of changed source files.
    ///
    /// For now, returns all functions (conservative). Future: use source map to narrow down.
    pub fn compute_affected_functions(&self, _changed_files: &[PathBuf]) -> Vec<String> {
        self.current_mappings.keys().cloned().collect()
    }
}

impl Default for HotReloader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{FunctionBlob, FunctionHash};

    /// Create a minimal test blob with a given name and unique hash.
    fn make_blob(name: &str, seed: u8) -> FunctionBlob {
        let mut hash_bytes = [0u8; 32];
        hash_bytes[0] = seed;
        FunctionBlob {
            content_hash: FunctionHash(hash_bytes),
            name: name.to_string(),
            arity: 0,
            param_names: Vec::new(),
            locals_count: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: Vec::new(),
            instructions: Vec::new(),
            constants: Vec::new(),
            strings: Vec::new(),
            required_permissions: Default::default(),
            dependencies: Vec::new(),
            callee_names: Vec::new(),
            type_schemas: Vec::new(),
            source_map: Vec::new(),
            foreign_dependencies: Vec::new(),
        }
    }

    #[test]
    fn test_register_and_lookup() {
        let mut hr = HotReloader::new();
        let blob = make_blob("foo", 1);
        let hash = blob.content_hash;
        hr.register_function("foo".into(), blob);

        assert_eq!(hr.current_hash("foo"), Some(&hash));
        assert!(hr.get_blob(&hash).is_some());
        assert_eq!(hr.get_blob(&hash).unwrap().name, "foo");
    }

    #[test]
    fn test_apply_patches_updates_mapping() {
        let mut hr = HotReloader::new();
        let blob_v1 = make_blob("bar", 10);
        hr.register_function("bar".into(), blob_v1);

        let blob_v2 = make_blob("bar", 20);
        let new_hash = blob_v2.content_hash;
        let result = hr.apply_patches(vec![FunctionPatch {
            function_name: "bar".into(),
            old_hash: Some(FunctionHash([10; 32])),
            new_blob: blob_v2,
        }]);

        assert_eq!(result.functions_updated, vec!["bar"]);
        assert!(result.functions_unchanged.is_empty());
        assert_eq!(hr.current_hash("bar"), Some(&new_hash));
        assert_eq!(hr.history().len(), 1);
    }

    #[test]
    fn test_apply_patches_unchanged_when_same_hash() {
        let mut hr = HotReloader::new();
        let blob = make_blob("baz", 5);
        hr.register_function("baz".into(), blob.clone());

        let result = hr.apply_patches(vec![FunctionPatch {
            function_name: "baz".into(),
            old_hash: None,
            new_blob: blob,
        }]);

        assert!(result.functions_updated.is_empty());
        assert_eq!(result.functions_unchanged, vec!["baz"]);
    }

    #[test]
    fn test_gc_removes_unreferenced_old_blobs() {
        let mut hr = HotReloader::new();
        let blob_v1 = make_blob("fn1", 1);
        let old_hash = blob_v1.content_hash;
        hr.register_function("fn1".into(), blob_v1);

        // Update to v2
        let blob_v2 = make_blob("fn1", 2);
        hr.apply_patches(vec![FunctionPatch {
            function_name: "fn1".into(),
            old_hash: Some(old_hash),
            new_blob: blob_v2,
        }]);

        // Old blob still in store before GC
        assert!(hr.get_blob(&old_hash).is_some());

        // GC should remove the old blob (no active references)
        let removed = hr.gc();
        assert_eq!(removed, 1);
        assert!(hr.get_blob(&old_hash).is_none());
    }

    #[test]
    fn test_gc_retains_actively_referenced_blobs() {
        let mut hr = HotReloader::new();
        let blob_v1 = make_blob("fn2", 10);
        let old_hash = blob_v1.content_hash;
        hr.register_function("fn2".into(), blob_v1);

        // Mark old blob as actively referenced (in-flight frame)
        hr.add_active_reference(old_hash);

        // Update to v2
        let blob_v2 = make_blob("fn2", 20);
        hr.apply_patches(vec![FunctionPatch {
            function_name: "fn2".into(),
            old_hash: Some(old_hash),
            new_blob: blob_v2,
        }]);

        // GC should NOT remove the old blob (active reference)
        let removed = hr.gc();
        assert_eq!(removed, 0);
        assert!(hr.get_blob(&old_hash).is_some());

        // Release the reference
        hr.remove_active_reference(old_hash);

        // Now GC should clean it up
        let removed = hr.gc();
        assert_eq!(removed, 1);
        assert!(hr.get_blob(&old_hash).is_none());
    }

    #[test]
    fn test_watch_paths() {
        let mut hr = HotReloader::new();
        assert!(hr.watched_paths().is_empty());

        hr.watch_path(PathBuf::from("/src/main.shape"));
        hr.watch_path(PathBuf::from("/src/lib.shape"));

        assert_eq!(hr.watched_paths().len(), 2);
    }

    #[test]
    fn test_compute_affected_functions_conservative() {
        let mut hr = HotReloader::new();
        hr.register_function("a".into(), make_blob("a", 1));
        hr.register_function("b".into(), make_blob("b", 2));
        hr.register_function("c".into(), make_blob("c", 3));

        let affected = hr.compute_affected_functions(&[PathBuf::from("/src/something.shape")]);
        // Conservative: returns all functions
        assert_eq!(affected.len(), 3);
    }

    #[test]
    fn test_old_blobs_retained_count() {
        let mut hr = HotReloader::new();
        let blob_v1 = make_blob("f", 1);
        let old_hash = blob_v1.content_hash;
        hr.register_function("f".into(), blob_v1);

        // Mark old version as active
        hr.add_active_reference(old_hash);

        let blob_v2 = make_blob("f", 2);
        let result = hr.apply_patches(vec![FunctionPatch {
            function_name: "f".into(),
            old_hash: Some(old_hash),
            new_blob: blob_v2,
        }]);

        assert_eq!(result.old_blobs_retained, 1);
    }
}
