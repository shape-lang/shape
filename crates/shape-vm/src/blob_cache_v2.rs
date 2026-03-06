//! Blob-level cache for content-addressed function blobs.
//! Caches individual FunctionBlobs by content hash on disk (~/.shape/cache/blobs/).

use crate::bytecode::{FunctionBlob, FunctionHash};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Cache for individual function blobs, keyed by content hash.
pub struct BlobCache {
    /// In-memory cache layer.
    memory: HashMap<FunctionHash, FunctionBlob>,
    /// Optional disk cache directory.
    disk_root: Option<PathBuf>,
    /// Cache statistics.
    stats: CacheStats,
}

#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    pub memory_hits: u64,
    pub disk_hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub evictions: u64,
}

impl BlobCache {
    /// Create a memory-only cache.
    pub fn memory_only() -> Self {
        Self {
            memory: HashMap::new(),
            disk_root: None,
            stats: CacheStats::default(),
        }
    }

    /// Create a cache with disk persistence.
    pub fn with_disk(root: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self {
            memory: HashMap::new(),
            disk_root: Some(root),
            stats: CacheStats::default(),
        })
    }

    /// Check if a blob is cached.
    pub fn has_blob(&self, hash: &FunctionHash) -> bool {
        if self.memory.contains_key(hash) {
            return true;
        }
        if let Some(ref root) = self.disk_root {
            let path = Self::blob_path(root, hash);
            return path.exists();
        }
        false
    }

    /// Get a cached blob by hash.
    pub fn get_blob(&mut self, hash: &FunctionHash) -> Option<FunctionBlob> {
        // Check memory first
        if let Some(blob) = self.memory.get(hash) {
            self.stats.memory_hits += 1;
            return Some(blob.clone());
        }

        // Check disk
        if let Some(ref root) = self.disk_root {
            let path = Self::blob_path(root, hash);
            if let Ok(data) = fs::read(&path) {
                if let Ok(blob) = rmp_serde::from_slice::<FunctionBlob>(&data) {
                    self.stats.disk_hits += 1;
                    self.memory.insert(*hash, blob.clone());
                    return Some(blob);
                }
            }
        }

        self.stats.misses += 1;
        None
    }

    /// Store a blob in the cache.
    pub fn put_blob(&mut self, blob: &FunctionBlob) {
        let hash = blob.content_hash;
        self.memory.insert(hash, blob.clone());
        self.stats.insertions += 1;

        // Write to disk if enabled
        if let Some(ref root) = self.disk_root {
            let path = Self::blob_path(root, &hash);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(data) = rmp_serde::to_vec(blob) {
                let _ = fs::write(&path, data);
            }
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Clear the in-memory cache.
    pub fn clear_memory(&mut self) {
        self.memory.clear();
    }

    /// Number of blobs in memory cache.
    pub fn memory_size(&self) -> usize {
        self.memory.len()
    }

    fn blob_path(root: &PathBuf, hash: &FunctionHash) -> PathBuf {
        let hex = hex::encode(hash.0);
        root.join(&hex[..2]).join(format!("{}.blob", &hex[2..]))
    }
}

/// JIT code cache - in-memory only (Cranelift JITModule can't serialize).
pub struct JitCodeCache {
    entries: HashMap<FunctionHash, *const u8>,
}

// SAFETY: Function pointers from JIT are valid for the lifetime of the JITModule
unsafe impl Send for JitCodeCache {}
unsafe impl Sync for JitCodeCache {}

impl JitCodeCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, hash: &FunctionHash) -> Option<*const u8> {
        self.entries.get(hash).copied()
    }

    pub fn insert(&mut self, hash: FunctionHash, ptr: *const u8) {
        self.entries.insert(hash, ptr);
    }

    pub fn contains(&self, hash: &FunctionHash) -> bool {
        self.entries.contains_key(hash)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
