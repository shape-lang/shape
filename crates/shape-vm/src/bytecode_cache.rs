//! Bytecode cache for compiled Shape modules
//!
//! Caches compiled bytecode programs on disk as `.shapec` files,
//! keyed by the SHA-256 hash of the source content + compiler version.
//! This avoids redundant recompilation when source files haven't changed.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::bytecode::BytecodeProgram;

/// Compiler version embedded in cache keys to invalidate on upgrades
const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// On-disk bytecode cache stored under `~/.shape/cache/bytecode/`
pub struct BytecodeCache {
    cache_dir: PathBuf,
}

impl BytecodeCache {
    /// Create a new bytecode cache, creating the cache directory if needed.
    ///
    /// The cache lives at `~/.shape/cache/bytecode/`. Returns `None` if the
    /// home directory cannot be determined or the directory cannot be created.
    pub fn new() -> Option<Self> {
        let home = dirs::home_dir()?;
        let cache_dir = home.join(".shape").join("cache").join("bytecode");
        std::fs::create_dir_all(&cache_dir).ok()?;
        Some(Self { cache_dir })
    }

    /// Create a cache at a specific directory (for testing).
    pub fn with_dir(cache_dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Self { cache_dir })
    }

    /// Look up cached bytecode for the given source content.
    ///
    /// Returns `Some(program)` on cache hit, `None` on miss or deserialization error.
    pub fn get(&self, source: &str) -> Option<BytecodeProgram> {
        let key = Self::cache_key(source);
        let path = self.cache_path(&key);
        let data = std::fs::read(&path).ok()?;
        rmp_serde::from_slice(&data).ok()
    }

    /// Store compiled bytecode for the given source content.
    ///
    /// **W17-make-closure note.** Programs whose compiler produced any
    /// `closure_function_layouts` entries (i.e. the source contains at
    /// least one closure literal) are not cached. The `ClosureLayout`
    /// side-table is currently `#[serde(skip)]` on `BytecodeProgram`
    /// and `ContentAddressedProgram` — a deserialized program would
    /// load with an empty layouts vector, and `op_make_closure` would
    /// then surface `no ClosureLayout registered for function N`
    /// (`crates/shape-vm/src/executor/control_flow/mod.rs:447`). Until
    /// `ClosureLayout` (+ `ConcreteType` / `FieldInfo` / `NativeKind` /
    /// `CaptureKind`) grow `Serialize`/`Deserialize` derives, the safe
    /// disposition is to skip caching for closure-bearing programs.
    /// Caching for closure-free programs remains in force.
    pub fn put(&self, source: &str, program: &BytecodeProgram) -> std::io::Result<()> {
        let has_closure_layouts = program
            .closure_function_layouts
            .iter()
            .any(|opt| opt.is_some());
        let has_ca_closure_layouts = program
            .content_addressed
            .as_ref()
            .map(|ca| !ca.closure_function_layouts_by_name.is_empty())
            .unwrap_or(false);
        // ADR-006 §2.7.24 Q25.C: same `#[serde(skip)]` concern applies
        // to `trait_vtables` — vtables hold `Arc<VTable>` which is not
        // a stable wire shape; a deserialized program would load with
        // empty vtables and `op_box_trait_object` would fail with
        // "no vtable registered". Skip caching for trait-bearing
        // programs until a kinded vtable wire format lands.
        let has_trait_vtables = !program.trait_vtables.is_empty();
        let has_ca_trait_vtables = program
            .content_addressed
            .as_ref()
            .map(|ca| !ca.trait_vtables.is_empty())
            .unwrap_or(false);
        if has_closure_layouts || has_ca_closure_layouts
            || has_trait_vtables || has_ca_trait_vtables
        {
            // Skip caching for closure-bearing OR trait-bearing programs.
            return Ok(());
        }
        let key = Self::cache_key(source);
        let path = self.cache_path(&key);
        let data = rmp_serde::to_vec(program)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&path, data)
    }

    /// Remove all cached bytecode files.
    pub fn clear(&self) -> std::io::Result<()> {
        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .map_or(false, |ext| ext == "shapec")
            {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    /// Compute the cache key for a source string.
    ///
    /// Key = SHA-256(source_content + "\0" + compiler_version) as hex.
    fn cache_key(source: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(source.as_bytes());
        hasher.update(b"\0");
        hasher.update(COMPILER_VERSION.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Map a cache key to a file path: `<cache_dir>/<key>.shapec`
    fn cache_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.shapec", key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache() -> (BytecodeCache, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let cache = BytecodeCache::with_dir(tmp.path().join("bytecode")).unwrap();
        (cache, tmp)
    }

    #[test]
    fn test_put_get_roundtrip() {
        let (cache, _tmp) = temp_cache();
        let program = BytecodeProgram::new();
        cache.put("let x = 1", &program).unwrap();
        let cached = cache.get("let x = 1");
        assert!(cached.is_some(), "Cache hit expected after put");
    }

    #[test]
    fn test_cache_miss() {
        let (cache, _tmp) = temp_cache();
        let result = cache.get("nonexistent source");
        assert!(result.is_none(), "Cache miss expected for unknown source");
    }

    #[test]
    fn test_different_source_different_key() {
        let (cache, _tmp) = temp_cache();
        let program = BytecodeProgram::new();
        cache.put("let x = 1", &program).unwrap();
        let result = cache.get("let x = 2");
        assert!(result.is_none(), "Different source should miss cache");
    }

    #[test]
    fn test_clear() {
        let (cache, _tmp) = temp_cache();
        let program = BytecodeProgram::new();
        cache.put("source_a", &program).unwrap();
        cache.put("source_b", &program).unwrap();

        cache.clear().unwrap();

        assert!(
            cache.get("source_a").is_none(),
            "Cache should be empty after clear"
        );
        assert!(
            cache.get("source_b").is_none(),
            "Cache should be empty after clear"
        );
    }

    #[test]
    fn test_cache_key_deterministic() {
        let key1 = BytecodeCache::cache_key("hello");
        let key2 = BytecodeCache::cache_key("hello");
        assert_eq!(key1, key2, "Same source should produce same key");
    }

    #[test]
    fn test_cache_key_different_for_different_source() {
        let key1 = BytecodeCache::cache_key("hello");
        let key2 = BytecodeCache::cache_key("world");
        assert_ne!(key1, key2, "Different source should produce different key");
    }
}
