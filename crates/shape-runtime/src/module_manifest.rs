//! Module manifest for content-addressed module distribution.
//!
//! A `ModuleManifest` describes a distributable module by mapping its exported
//! functions and type schemas to content-addressed hashes. This allows the
//! loader to fetch only the blobs it needs from a `BlobStore`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Manifest for a content-addressed module distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    /// Exported function names mapped to their content hashes.
    pub exports: HashMap<String, [u8; 32]>,
    /// Type schema names mapped to their content hashes.
    pub type_schemas: HashMap<String, [u8; 32]>,
    /// Permissions required by this module.
    pub required_permission_bits: u64,
    /// Transitive dependency closure for each export: export hash → list of dependency hashes.
    #[serde(default)]
    pub dependency_closure: HashMap<[u8; 32], Vec<[u8; 32]>>,
    /// SHA-256 hash of this manifest (excluding this field and signature).
    pub manifest_hash: [u8; 32],
    /// Optional cryptographic signature.
    pub signature: Option<ModuleSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSignature {
    pub author_key: [u8; 32],
    /// Ed25519 signature (64 bytes). Uses `Vec<u8>` because serde does not
    /// support `[u8; 64]` out of the box.
    pub signature: Vec<u8>,
    pub signed_at: u64,
}

/// Helper struct for deterministic manifest hashing.
/// We hash only the fields that define the manifest's identity,
/// excluding `manifest_hash` and `signature`.
#[derive(Serialize)]
struct ManifestHashInput<'a> {
    name: &'a str,
    version: &'a str,
    exports: Vec<(&'a String, &'a [u8; 32])>,
    type_schemas: Vec<(&'a String, &'a [u8; 32])>,
    required_permission_bits: u64,
    dependency_closure: Vec<(&'a [u8; 32], &'a Vec<[u8; 32]>)>,
}

impl ModuleManifest {
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            version,
            exports: HashMap::new(),
            type_schemas: HashMap::new(),
            required_permission_bits: 0,
            dependency_closure: HashMap::new(),
            manifest_hash: [0u8; 32],
            signature: None,
        }
    }

    pub fn add_export(&mut self, name: String, hash: [u8; 32]) {
        self.exports.insert(name, hash);
    }

    pub fn add_type_schema(&mut self, name: String, hash: [u8; 32]) {
        self.type_schemas.insert(name, hash);
    }

    /// Compute `manifest_hash` from the identity fields.
    ///
    /// Exports and type schemas are sorted by key for deterministic hashing.
    pub fn finalize(&mut self) {
        let mut exports: Vec<_> = self.exports.iter().collect();
        exports.sort_by_key(|(k, _)| *k);

        let mut type_schemas: Vec<_> = self.type_schemas.iter().collect();
        type_schemas.sort_by_key(|(k, _)| *k);

        let mut dep_closure: Vec<_> = self.dependency_closure.iter().collect();
        dep_closure.sort_by_key(|(k, _)| *k);

        let input = ManifestHashInput {
            name: &self.name,
            version: &self.version,
            exports,
            type_schemas,
            required_permission_bits: self.required_permission_bits,
            dependency_closure: dep_closure,
        };

        let bytes = rmp_serde::encode::to_vec(&input)
            .expect("ManifestHashInput serialization should not fail");
        let digest = Sha256::digest(&bytes);
        self.manifest_hash.copy_from_slice(&digest);
    }

    /// Verify that `manifest_hash` matches the current content.
    pub fn verify_integrity(&self) -> bool {
        let mut exports: Vec<_> = self.exports.iter().collect();
        exports.sort_by_key(|(k, _)| *k);

        let mut type_schemas: Vec<_> = self.type_schemas.iter().collect();
        type_schemas.sort_by_key(|(k, _)| *k);

        let mut dep_closure: Vec<_> = self.dependency_closure.iter().collect();
        dep_closure.sort_by_key(|(k, _)| *k);

        let input = ManifestHashInput {
            name: &self.name,
            version: &self.version,
            exports,
            type_schemas,
            required_permission_bits: self.required_permission_bits,
            dependency_closure: dep_closure,
        };

        let bytes = rmp_serde::encode::to_vec(&input)
            .expect("ManifestHashInput serialization should not fail");
        let digest = Sha256::digest(&bytes);
        let mut expected = [0u8; 32];
        expected.copy_from_slice(&digest);
        self.manifest_hash == expected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_manifest_has_zero_hash() {
        let m = ModuleManifest::new("test".into(), "0.1.0".into());
        assert_eq!(m.manifest_hash, [0u8; 32]);
        assert!(m.exports.is_empty());
        assert!(m.type_schemas.is_empty());
    }

    #[test]
    fn test_finalize_produces_nonzero_hash() {
        let mut m = ModuleManifest::new("mymod".into(), "1.0.0".into());
        m.add_export("greet".into(), [1u8; 32]);
        m.finalize();
        assert_ne!(m.manifest_hash, [0u8; 32]);
    }

    #[test]
    fn test_verify_integrity_passes_after_finalize() {
        let mut m = ModuleManifest::new("mymod".into(), "1.0.0".into());
        m.add_export("greet".into(), [1u8; 32]);
        m.add_type_schema("MyType".into(), [2u8; 32]);
        m.required_permission_bits = 0x03;
        m.finalize();
        assert!(m.verify_integrity());
    }

    #[test]
    fn test_verify_integrity_fails_after_mutation() {
        let mut m = ModuleManifest::new("mymod".into(), "1.0.0".into());
        m.add_export("greet".into(), [1u8; 32]);
        m.finalize();
        assert!(m.verify_integrity());

        m.add_export("farewell".into(), [3u8; 32]);
        assert!(!m.verify_integrity());
    }

    #[test]
    fn test_deterministic_hash() {
        let build = || {
            let mut m = ModuleManifest::new("det".into(), "0.0.1".into());
            m.add_export("b_fn".into(), [10u8; 32]);
            m.add_export("a_fn".into(), [20u8; 32]);
            m.add_type_schema("Z".into(), [30u8; 32]);
            m.add_type_schema("A".into(), [40u8; 32]);
            m.finalize();
            m.manifest_hash
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut m = ModuleManifest::new("serde_test".into(), "2.0.0".into());
        m.add_export("run".into(), [7u8; 32]);
        m.required_permission_bits = 0xFF;
        m.finalize();

        let json = serde_json::to_string(&m).expect("serialize");
        let restored: ModuleManifest = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.name, "serde_test");
        assert_eq!(restored.version, "2.0.0");
        assert_eq!(restored.exports.get("run"), Some(&[7u8; 32]));
        assert_eq!(restored.required_permission_bits, 0xFF);
        assert!(restored.verify_integrity());
    }
}
