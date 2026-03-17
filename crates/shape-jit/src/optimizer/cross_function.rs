//! Cross-function JIT optimization for Tier 2 compilation.
//!
//! Provides Tier 2 cache key computation for content-addressed JIT code caching.

// ---------------------------------------------------------------------------
// Tier 2 Cache Key
// ---------------------------------------------------------------------------

/// Cache key for Tier 2 compiled functions.
///
/// Includes the function's own hash plus the hashes of all inlined callees,
/// since inlining changes the generated native code. Also tracks the schema
/// version and feedback epoch at compilation time for invalidation.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Tier2CacheKey {
    /// Hash of the root function blob.
    pub root_hash: [u8; 32],
    /// Sorted hashes of all inlined callee blobs.
    pub inlined_hashes: Vec<[u8; 32]>,
    /// Compiler version for invalidation.
    pub compiler_version: u32,
    /// Schema version at compilation time. When object shapes change
    /// (e.g., a property is added/removed), the schema version is bumped
    /// and compiled code that embedded shape guards becomes stale.
    pub schema_version: u32,
    /// Feedback epoch at compilation time. When speculation assumptions
    /// are invalidated (e.g., a type guard fails), the feedback epoch is
    /// bumped and code compiled under old assumptions must be discarded.
    pub feedback_epoch: u32,
}

impl Tier2CacheKey {
    pub fn new(root_hash: [u8; 32], mut inlined: Vec<[u8; 32]>, compiler_version: u32) -> Self {
        inlined.sort();
        Self {
            root_hash,
            inlined_hashes: inlined,
            compiler_version,
            schema_version: 0,
            feedback_epoch: 0,
        }
    }

    /// Create a cache key with full versioning metadata.
    pub fn with_versions(
        root_hash: [u8; 32],
        mut inlined: Vec<[u8; 32]>,
        compiler_version: u32,
        schema_version: u32,
        feedback_epoch: u32,
    ) -> Self {
        inlined.sort();
        Self {
            root_hash,
            inlined_hashes: inlined,
            compiler_version,
            schema_version,
            feedback_epoch,
        }
    }

    /// Compute a single combined hash for use as a map key.
    pub fn combined_hash(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.root_hash);
        for h in &self.inlined_hashes {
            hasher.update(h);
        }
        hasher.update(self.compiler_version.to_le_bytes());
        hasher.update(self.schema_version.to_le_bytes());
        hasher.update(self.feedback_epoch.to_le_bytes());
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[test]
    fn test_tier2_cache_key() {
        let k1 = Tier2CacheKey::new(hash(1), vec![hash(2), hash(3)], 1);
        let k2 = Tier2CacheKey::new(hash(1), vec![hash(3), hash(2)], 1);
        // Order shouldn't matter.
        assert_eq!(k1.combined_hash(), k2.combined_hash());
    }
}
