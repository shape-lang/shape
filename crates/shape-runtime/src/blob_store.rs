//! Blob store abstraction for content-addressed function blobs.
//!
//! A `BlobStore` maps `[u8; 32]` content hashes to raw byte blobs. Two
//! implementations are provided:
//!
//! - `MemoryBlobStore` -- in-memory, suitable for testing and ephemeral use.
//! - `FsBlobStore` -- filesystem-based with a git-style two-level directory
//!   layout (`~/.shape/blobs/ab/cd1234...ef.blob`).

use std::collections::HashMap;
use std::path::PathBuf;

/// Content-addressed blob storage.
pub trait BlobStore: Send + Sync {
    /// Retrieve the blob for the given content hash, or `None` if absent.
    fn get(&self, hash: &[u8; 32]) -> Option<Vec<u8>>;

    /// Store a blob under the given content hash. Returns `true` if the blob
    /// was newly inserted, `false` if it already existed.
    fn put(&self, hash: [u8; 32], data: Vec<u8>) -> bool;

    /// Check whether a blob exists for the given hash.
    fn contains(&self, hash: &[u8; 32]) -> bool;
}

// ---------------------------------------------------------------------------
// MemoryBlobStore
// ---------------------------------------------------------------------------

/// In-memory blob store for testing and ephemeral use.
pub struct MemoryBlobStore {
    blobs: parking_lot::RwLock<HashMap<[u8; 32], Vec<u8>>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self {
            blobs: parking_lot::RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemoryBlobStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BlobStore for MemoryBlobStore {
    fn get(&self, hash: &[u8; 32]) -> Option<Vec<u8>> {
        self.blobs.read().get(hash).cloned()
    }

    fn put(&self, hash: [u8; 32], data: Vec<u8>) -> bool {
        use std::collections::hash_map::Entry;
        match self.blobs.write().entry(hash) {
            Entry::Occupied(_) => false,
            Entry::Vacant(e) => {
                e.insert(data);
                true
            }
        }
    }

    fn contains(&self, hash: &[u8; 32]) -> bool {
        self.blobs.read().contains_key(hash)
    }
}

// ---------------------------------------------------------------------------
// FsBlobStore
// ---------------------------------------------------------------------------

/// Filesystem-based blob store with git-style two-level directory layout.
///
/// Blobs are stored as `<root>/<first-2-hex-chars>/<remaining-hex>.blob`.
/// For example, hash `abcd12...ef` is stored at `<root>/ab/cd12...ef.blob`.
pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    /// Create (or open) a filesystem blob store rooted at `root`.
    ///
    /// The root directory is created if it does not exist.
    pub fn new(root: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Compute the path for a given content hash.
    fn blob_path(&self, hash: &[u8; 32]) -> PathBuf {
        let hex = hex::encode(hash);
        self.root
            .join(&hex[..2])
            .join(format!("{}.blob", &hex[2..]))
    }
}

impl BlobStore for FsBlobStore {
    fn get(&self, hash: &[u8; 32]) -> Option<Vec<u8>> {
        let path = self.blob_path(hash);
        std::fs::read(&path).ok()
    }

    fn put(&self, hash: [u8; 32], data: Vec<u8>) -> bool {
        let path = self.blob_path(&hash);
        if path.exists() {
            return false;
        }
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return false;
            }
        }
        std::fs::write(&path, &data).is_ok()
    }

    fn contains(&self, hash: &[u8; 32]) -> bool {
        self.blob_path(hash).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_blob_store_put_get() {
        let store = MemoryBlobStore::new();
        let hash = [0xAB; 32];
        let data = vec![1, 2, 3, 4, 5];

        assert!(!store.contains(&hash));
        assert!(store.put(hash, data.clone()));
        assert!(store.contains(&hash));
        assert_eq!(store.get(&hash), Some(data));
    }

    #[test]
    fn test_memory_blob_store_duplicate_put() {
        let store = MemoryBlobStore::new();
        let hash = [0xCD; 32];

        assert!(store.put(hash, vec![1, 2]));
        assert!(!store.put(hash, vec![3, 4]));
        // Original data preserved
        assert_eq!(store.get(&hash), Some(vec![1, 2]));
    }

    #[test]
    fn test_memory_blob_store_missing_key() {
        let store = MemoryBlobStore::new();
        assert_eq!(store.get(&[0xFF; 32]), None);
        assert!(!store.contains(&[0xFF; 32]));
    }

    #[test]
    fn test_fs_blob_store_put_get() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let store = FsBlobStore::new(tmp.path().to_path_buf()).expect("create store");

        let hash = [0x12; 32];
        let data = vec![10, 20, 30];

        assert!(!store.contains(&hash));
        assert!(store.put(hash, data.clone()));
        assert!(store.contains(&hash));
        assert_eq!(store.get(&hash), Some(data));
    }

    #[test]
    fn test_fs_blob_store_duplicate_put() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let store = FsBlobStore::new(tmp.path().to_path_buf()).expect("create store");

        let hash = [0x34; 32];
        assert!(store.put(hash, vec![1]));
        assert!(!store.put(hash, vec![2]));
        assert_eq!(store.get(&hash), Some(vec![1]));
    }

    #[test]
    fn test_fs_blob_store_path_layout() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let store = FsBlobStore::new(tmp.path().to_path_buf()).expect("create store");

        let mut hash = [0u8; 32];
        hash[0] = 0xAB;
        hash[1] = 0xCD;
        // Rest are zeros

        let path = store.blob_path(&hash);
        let path_str = path.to_string_lossy();

        // Should start with the 2-char prefix directory
        assert!(
            path_str.contains("/ab/"),
            "expected /ab/ in path, got: {}",
            path_str
        );
        assert!(
            path_str.ends_with(".blob"),
            "expected .blob suffix, got: {}",
            path_str
        );
    }

    #[test]
    fn test_fs_blob_store_missing_key() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let store = FsBlobStore::new(tmp.path().to_path_buf()).expect("create store");
        assert_eq!(store.get(&[0xFF; 32]), None);
        assert!(!store.contains(&[0xFF; 32]));
    }
}
