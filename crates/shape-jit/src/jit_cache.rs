//! Content-addressed JIT code cache with dependency-based invalidation.
//!
//! Caches compiled native function pointers keyed by `FunctionHash`.
//! When the same function blob appears in a subsequent compilation
//! (same content hash), we skip recompilation and reuse the existing
//! native code pointer.
//!
//! Supports dependency tracking: when a function is inlined into another,
//! the caller records the callee hash as a dependency. If the callee
//! changes, all dependents can be invalidated via `invalidate_by_dependency()`.

use shape_vm::bytecode::FunctionHash;
use shape_value::ValueWordExt;
use std::collections::HashMap;

use crate::optimizer::Tier2CacheKey;

/// Extended cache entry with dependency tracking.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// Native code pointer.
    pub code_ptr: *const u8,
    /// Content hash of the function blob.
    pub function_hash: FunctionHash,
    /// Schema version at compilation time (for shape guard invalidation).
    pub schema_version: u32,
    /// Feedback epoch at compilation time (for speculation invalidation).
    pub feedback_epoch: u32,
    /// Hashes of functions this compiled code depends on (e.g., inlined callees).
    pub dependencies: Vec<FunctionHash>,
    /// Tier 2 cache key, present when this entry was produced by the
    /// optimizing compiler with cross-function inlining.
    pub tier2_key: Option<Tier2CacheKey>,
}

// SAFETY: CacheEntry contains a raw pointer produced by Cranelift.
// The same safety argument as JitCodeCache applies (see below).
unsafe impl Send for CacheEntry {}
unsafe impl Sync for CacheEntry {}

/// Cache of JIT-compiled function pointers, keyed by content hash.
///
/// Same blob hash = skip recompilation, reuse function pointer.
///
/// Tracks dependency edges so that when an inlined callee changes,
/// all callers that embedded it can be invalidated.
///
/// # Safety
///
/// The raw `*const u8` pointers stored here point into Cranelift
/// `JITModule` memory regions. Callers must ensure that the
/// `JITModule` that produced a pointer outlives any use of that
/// pointer through this cache.
pub struct JitCodeCache {
    entries: HashMap<FunctionHash, CacheEntry>,
    /// Reverse index: dependency_hash -> set of dependent function hashes.
    /// Used by `invalidate_by_dependency()` to find affected entries.
    dependents: HashMap<FunctionHash, Vec<FunctionHash>>,
}

// SAFETY: The function pointers are produced by Cranelift and are
// valid for the lifetime of the owning JITModule. The cache itself
// does not execute code, it only stores and returns pointers.
unsafe impl Send for JitCodeCache {}
unsafe impl Sync for JitCodeCache {}

impl JitCodeCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            dependents: HashMap::new(),
        }
    }

    /// Create a cache pre-sized for `capacity` entries.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            dependents: HashMap::new(),
        }
    }

    /// Look up a cached native code pointer by content hash.
    pub fn get(&self, hash: &FunctionHash) -> Option<*const u8> {
        self.entries.get(hash).map(|e| e.code_ptr)
    }

    /// Insert a compiled function pointer for the given content hash.
    ///
    /// Creates a minimal `CacheEntry` with no dependencies and zero
    /// version/epoch. If an entry with the same hash already exists
    /// it is overwritten.
    pub fn insert(&mut self, hash: FunctionHash, ptr: *const u8) {
        // Remove old dependency edges if overwriting.
        self.remove_dependency_edges(&hash);
        self.entries.insert(
            hash,
            CacheEntry {
                code_ptr: ptr,
                function_hash: hash,
                schema_version: 0,
                feedback_epoch: 0,
                dependencies: Vec::new(),
                tier2_key: None,
            },
        );
    }

    /// Insert a cache entry with full dependency information.
    ///
    /// Builds reverse-index edges so that `invalidate_by_dependency()`
    /// can find this entry when any of its dependencies change.
    pub fn insert_entry(&mut self, entry: CacheEntry) {
        let hash = entry.function_hash;
        // Remove stale dependency edges if overwriting.
        self.remove_dependency_edges(&hash);
        // Build reverse edges for the new entry.
        for dep in &entry.dependencies {
            self.dependents.entry(*dep).or_default().push(hash);
        }
        self.entries.insert(hash, entry);
    }

    /// Invalidate all entries that depend on the given function hash.
    ///
    /// Performs a transitive walk: if A depends on B and B depends on C,
    /// invalidating C will remove both B and A.
    ///
    /// Returns the list of invalidated function hashes.
    pub fn invalidate_by_dependency(&mut self, changed_hash: &FunctionHash) -> Vec<FunctionHash> {
        let mut invalidated = Vec::new();
        let mut worklist = vec![*changed_hash];

        while let Some(current) = worklist.pop() {
            if let Some(deps) = self.dependents.remove(&current) {
                for dep_hash in deps {
                    if self.entries.remove(&dep_hash).is_some() {
                        invalidated.push(dep_hash);
                        // Cascade: anything that depended on the now-removed
                        // entry must also be invalidated.
                        worklist.push(dep_hash);
                    }
                }
            }
        }

        // Clean up reverse edges for invalidated entries.
        for inv in &invalidated {
            self.remove_dependency_edges(inv);
        }

        invalidated
    }

    /// Get a cache entry with full metadata.
    pub fn get_entry(&self, hash: &FunctionHash) -> Option<&CacheEntry> {
        self.entries.get(hash)
    }

    /// Check whether a function with the given hash has been compiled.
    pub fn contains(&self, hash: &FunctionHash) -> bool {
        self.entries.contains_key(hash)
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remove all entries from the cache.
    ///
    /// This does **not** free the underlying native code memory (that
    /// is owned by the Cranelift `JITModule`).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.dependents.clear();
    }

    /// Remove reverse-index edges for a given function hash.
    fn remove_dependency_edges(&mut self, hash: &FunctionHash) {
        if let Some(entry) = self.entries.get(hash) {
            let deps: Vec<FunctionHash> = entry.dependencies.clone();
            for dep in &deps {
                if let Some(rev) = self.dependents.get_mut(dep) {
                    rev.retain(|h| h != hash);
                    if rev.is_empty() {
                        self.dependents.remove(dep);
                    }
                }
            }
        }
    }
}

impl Default for JitCodeCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache() {
        let cache = JitCodeCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(cache.get(&FunctionHash::ZERO).is_none());
    }

    #[test]
    fn insert_and_get() {
        let mut cache = JitCodeCache::new();
        let hash = FunctionHash([0xAB; 32]);
        let fake_ptr = 0xDEAD_BEEF_usize as *const u8;

        cache.insert(hash, fake_ptr);
        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());
        assert!(cache.contains(&hash));
        assert_eq!(cache.get(&hash), Some(fake_ptr));
    }

    #[test]
    fn missing_hash_returns_none() {
        let mut cache = JitCodeCache::new();
        let hash_a = FunctionHash([1u8; 32]);
        let hash_b = FunctionHash([2u8; 32]);
        cache.insert(hash_a, 0x1 as *const u8);

        assert!(cache.get(&hash_b).is_none());
        assert!(!cache.contains(&hash_b));
    }

    #[test]
    fn overwrite_entry() {
        let mut cache = JitCodeCache::new();
        let hash = FunctionHash([0xCC; 32]);
        let ptr1 = 0x1000_usize as *const u8;
        let ptr2 = 0x2000_usize as *const u8;

        cache.insert(hash, ptr1);
        assert_eq!(cache.get(&hash), Some(ptr1));

        cache.insert(hash, ptr2);
        assert_eq!(cache.get(&hash), Some(ptr2));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn clear_removes_all() {
        let mut cache = JitCodeCache::new();
        cache.insert(FunctionHash([1; 32]), 0x1 as *const u8);
        cache.insert(FunctionHash([2; 32]), 0x2 as *const u8);
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn with_capacity() {
        let cache = JitCodeCache::with_capacity(64);
        assert!(cache.is_empty());
    }

    // --- Dependency-tracking tests ---

    fn make_entry(hash: FunctionHash, ptr: usize, deps: Vec<FunctionHash>) -> CacheEntry {
        CacheEntry {
            code_ptr: ptr as *const u8,
            function_hash: hash,
            schema_version: 1,
            feedback_epoch: 1,
            dependencies: deps,
            tier2_key: None,
        }
    }

    #[test]
    fn test_insert_entry_with_dependencies() {
        let mut cache = JitCodeCache::new();
        let callee = FunctionHash([0x01; 32]);
        let caller = FunctionHash([0x02; 32]);

        // Insert the callee (no deps).
        cache.insert_entry(make_entry(callee, 0x1000, vec![]));

        // Insert the caller which depends on the callee.
        cache.insert_entry(make_entry(caller, 0x2000, vec![callee]));

        assert_eq!(cache.len(), 2);
        assert!(cache.contains(&callee));
        assert!(cache.contains(&caller));

        // Verify metadata is accessible.
        let entry = cache.get_entry(&caller).unwrap();
        assert_eq!(entry.schema_version, 1);
        assert_eq!(entry.feedback_epoch, 1);
        assert_eq!(entry.dependencies, vec![callee]);
    }

    #[test]
    fn test_invalidate_by_dependency() {
        let mut cache = JitCodeCache::new();
        let callee = FunctionHash([0x01; 32]);
        let caller_a = FunctionHash([0x02; 32]);
        let caller_b = FunctionHash([0x03; 32]);
        let unrelated = FunctionHash([0x04; 32]);

        cache.insert_entry(make_entry(callee, 0x1000, vec![]));
        cache.insert_entry(make_entry(caller_a, 0x2000, vec![callee]));
        cache.insert_entry(make_entry(caller_b, 0x3000, vec![callee]));
        cache.insert_entry(make_entry(unrelated, 0x4000, vec![]));
        assert_eq!(cache.len(), 4);

        // Invalidate everything that depends on callee.
        let mut invalidated = cache.invalidate_by_dependency(&callee);
        invalidated.sort_by_key(|h| h.0);

        assert_eq!(invalidated.len(), 2);
        assert!(invalidated.contains(&caller_a));
        assert!(invalidated.contains(&caller_b));

        // callee itself is NOT removed (only its dependents are).
        assert!(cache.contains(&callee));
        assert!(cache.contains(&unrelated));
        assert!(!cache.contains(&caller_a));
        assert!(!cache.contains(&caller_b));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_invalidate_cascading() {
        // A depends on B, B depends on C. Invalidate C -> both B and A removed.
        let mut cache = JitCodeCache::new();
        let c = FunctionHash([0x01; 32]);
        let b = FunctionHash([0x02; 32]);
        let a = FunctionHash([0x03; 32]);

        cache.insert_entry(make_entry(c, 0x1000, vec![]));
        cache.insert_entry(make_entry(b, 0x2000, vec![c]));
        cache.insert_entry(make_entry(a, 0x3000, vec![b]));
        assert_eq!(cache.len(), 3);

        let mut invalidated = cache.invalidate_by_dependency(&c);
        invalidated.sort_by_key(|h| h.0);

        // Both B and A should be invalidated (B directly, A transitively).
        assert_eq!(invalidated.len(), 2);
        assert!(invalidated.contains(&b));
        assert!(invalidated.contains(&a));

        // Only C remains.
        assert!(cache.contains(&c));
        assert!(!cache.contains(&b));
        assert!(!cache.contains(&a));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_get_entry_returns_metadata() {
        let mut cache = JitCodeCache::new();
        let hash = FunctionHash([0xAA; 32]);
        let dep = FunctionHash([0xBB; 32]);

        cache.insert_entry(CacheEntry {
            code_ptr: 0x5000 as *const u8,
            function_hash: hash,
            schema_version: 42,
            feedback_epoch: 7,
            dependencies: vec![dep],
            tier2_key: None,
        });

        let entry = cache.get_entry(&hash).unwrap();
        assert_eq!(entry.code_ptr, 0x5000 as *const u8);
        assert_eq!(entry.function_hash, hash);
        assert_eq!(entry.schema_version, 42);
        assert_eq!(entry.feedback_epoch, 7);
        assert_eq!(entry.dependencies, vec![dep]);

        // get() still returns just the pointer.
        assert_eq!(cache.get(&hash), Some(0x5000 as *const u8));

        // Missing entry returns None.
        assert!(cache.get_entry(&FunctionHash([0xFF; 32])).is_none());
    }

    #[test]
    fn test_tier2_cache_key_stored_in_entry() {
        let mut cache = JitCodeCache::new();
        let root = FunctionHash([0x10; 32]);
        let inlined_callee = FunctionHash([0x20; 32]);

        let key = Tier2CacheKey::with_versions(
            root.0,
            vec![inlined_callee.0],
            1, // compiler_version
            5, // schema_version
            3, // feedback_epoch
        );

        cache.insert_entry(CacheEntry {
            code_ptr: 0x8000 as *const u8,
            function_hash: root,
            schema_version: 5,
            feedback_epoch: 3,
            dependencies: vec![inlined_callee],
            tier2_key: Some(key.clone()),
        });

        let entry = cache.get_entry(&root).unwrap();
        let stored_key = entry.tier2_key.as_ref().unwrap();
        assert_eq!(stored_key.root_hash, root.0);
        assert_eq!(stored_key.inlined_hashes, vec![inlined_callee.0]);
        assert_eq!(stored_key.schema_version, 5);
        assert_eq!(stored_key.feedback_epoch, 3);
        assert_eq!(stored_key.compiler_version, 1);

        // Verify combined_hash includes version metadata.
        let key_no_versions = Tier2CacheKey::new(root.0, vec![inlined_callee.0], 1);
        assert_ne!(stored_key.combined_hash(), key_no_versions.combined_hash());
    }

    #[test]
    fn test_invalidate_with_tier2_entries() {
        // Tier 2 entry with inlined callee: invalidating the callee
        // removes the tier 2 entry.
        let mut cache = JitCodeCache::new();
        let callee = FunctionHash([0x01; 32]);
        let optimized = FunctionHash([0x02; 32]);

        let key = Tier2CacheKey::with_versions(optimized.0, vec![callee.0], 1, 0, 0);

        cache.insert_entry(CacheEntry {
            code_ptr: 0x1000 as *const u8,
            function_hash: callee,
            schema_version: 0,
            feedback_epoch: 0,
            dependencies: vec![],
            tier2_key: None,
        });
        cache.insert_entry(CacheEntry {
            code_ptr: 0x2000 as *const u8,
            function_hash: optimized,
            schema_version: 0,
            feedback_epoch: 0,
            dependencies: vec![callee],
            tier2_key: Some(key),
        });

        let invalidated = cache.invalidate_by_dependency(&callee);
        assert_eq!(invalidated.len(), 1);
        assert_eq!(invalidated[0], optimized);
        assert!(!cache.contains(&optimized));
        assert!(cache.contains(&callee));
    }
}
