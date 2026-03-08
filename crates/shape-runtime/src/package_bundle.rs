//! Package bundle format for distributable .shapec files
//!
//! A package bundle contains pre-compiled bytecode for all modules in a Shape
//! package, plus metadata for versioning and freshness checks.
//!
//! File format: `[8 bytes "SHAPEPKG"] [4 bytes format_version LE] [MessagePack payload]`

use crate::doc_extract::DocItem;
use crate::module_manifest::ModuleManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const MAGIC: &[u8; 8] = b"SHAPEPKG";
const FORMAT_VERSION: u32 = 3;
/// Minimum version we can still load (v1 bundles lack blob_store/manifests).
const MIN_FORMAT_VERSION: u32 = 1;

fn default_bundle_kind() -> String {
    "portable-bytecode".to_string()
}

/// Metadata about a compiled package bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleMetadata {
    /// Package name from shape.toml [project].name
    pub name: String,
    /// Package version from shape.toml [project].version
    pub version: String,
    /// Shape compiler version that produced this bundle
    pub compiler_version: String,
    /// SHA-256 hash of all source files combined
    pub source_hash: String,
    /// Bundle compatibility kind.
    /// `portable-bytecode` bundles are cross-platform and contain no host-native machine code.
    #[serde(default = "default_bundle_kind")]
    pub bundle_kind: String,
    /// Host identifier of the build machine (for diagnostics only).
    #[serde(default)]
    pub build_host: String,
    /// Whether declared native dependencies are host-portable (no host-specific path/vendoring required).
    #[serde(default = "default_native_portable")]
    pub native_portable: bool,
    /// Entry module path, if any
    pub entry_module: Option<String>,
    /// Build timestamp (unix seconds from SystemTime)
    pub built_at: u64,
    /// README content (raw Markdown), read from README.md in project root.
    #[serde(default)]
    pub readme: Option<String>,
}

fn default_native_portable() -> bool {
    true
}

/// A single compiled module within a bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledModule {
    /// Module path using :: separator (e.g., "utils::helpers")
    pub module_path: String,
    /// MessagePack-serialized BytecodeProgram as raw bytes
    pub bytecode_bytes: Vec<u8>,
    /// Names of exported symbols
    pub export_names: Vec<String>,
    /// SHA-256 hash of the individual source file
    pub source_hash: String,
}

/// A compiled package bundle containing all modules and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageBundle {
    /// Bundle metadata
    pub metadata: BundleMetadata,
    /// Compiled modules
    pub modules: Vec<BundledModule>,
    /// Declared dependency versions (name -> version string)
    pub dependencies: HashMap<String, String>,
    /// Content-addressed blob store: hash -> raw blob bytes.
    /// Blobs are deduplicated across modules so shared functions are stored once.
    #[serde(default)]
    pub blob_store: HashMap<[u8; 32], Vec<u8>>,
    /// Module manifests for content-addressed resolution.
    /// Each manifest maps export names to blob hashes in `blob_store`.
    #[serde(default)]
    pub manifests: Vec<ModuleManifest>,
    /// Native dependency scopes for this package and all transitive dependencies.
    /// Used by consumers of `.shapec` bundles to lock/validate native prerequisites.
    #[serde(default)]
    pub native_dependency_scopes: Vec<BundledNativeDependencyScope>,
    /// Documentation items extracted from source code, keyed by module path.
    #[serde(default)]
    pub docs: HashMap<String, Vec<DocItem>>,
}

/// Native dependency scope embedded in a `.shapec` bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledNativeDependencyScope {
    /// Package name declaring the dependencies.
    pub package_name: String,
    /// Package version declaring the dependencies.
    pub package_version: String,
    /// Canonical package identity key (`name@version`).
    pub package_key: String,
    /// Native dependencies declared by this package.
    pub dependencies: HashMap<String, crate::project::NativeDependencySpec>,
}

impl PackageBundle {
    /// Serialize the bundle to bytes with magic header.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let payload =
            rmp_serde::to_vec(self).map_err(|e| format!("Failed to serialize bundle: {}", e))?;

        let mut buf = Vec::with_capacity(12 + payload.len());
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&payload);
        Ok(buf)
    }

    /// Deserialize a bundle from bytes, validating magic and version.
    ///
    /// Supports v1 (no blob_store/manifests), v2, and v3 (docs) bundles.
    /// Missing fields are filled with defaults via `#[serde(default)]`.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        if data.len() < 12 {
            return Err("Bundle too small: missing header".to_string());
        }

        if &data[..8] != MAGIC {
            return Err("Invalid bundle: bad magic bytes".to_string());
        }

        let version = u32::from_le_bytes(
            data[8..12]
                .try_into()
                .map_err(|_| "Invalid version bytes".to_string())?,
        );
        if version < MIN_FORMAT_VERSION || version > FORMAT_VERSION {
            return Err(format!(
                "Unsupported bundle format version: expected {}-{}, got {}",
                MIN_FORMAT_VERSION, FORMAT_VERSION, version
            ));
        }

        rmp_serde::from_slice(&data[12..])
            .map_err(|e| format!("Failed to deserialize bundle: {}", e))
    }

    /// Write the bundle to a file.
    pub fn write_to_file(&self, path: &Path) -> Result<(), String> {
        let bytes = self.to_bytes()?;
        std::fs::write(path, bytes)
            .map_err(|e| format!("Failed to write bundle to '{}': {}", path.display(), e))
    }

    /// Read a bundle from a file.
    pub fn read_from_file(path: &Path) -> Result<Self, String> {
        let data = std::fs::read(path)
            .map_err(|e| format!("Failed to read bundle from '{}': {}", path.display(), e))?;
        Self::from_bytes(&data)
    }
}

/// Verify SHA-256 checksum of raw bundle bytes.
/// `expected` should be in format "sha256:hexdigest" or just the hex digest.
pub fn verify_bundle_checksum(bundle_bytes: &[u8], expected: &str) -> bool {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bundle_bytes);
    let digest = hex::encode(hasher.finalize());
    let expected_hex = expected.strip_prefix("sha256:").unwrap_or(expected);
    digest == expected_hex
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bundle() -> PackageBundle {
        PackageBundle {
            metadata: BundleMetadata {
                name: "test-pkg".to_string(),
                version: "0.1.0".to_string(),
                compiler_version: "0.5.0".to_string(),
                source_hash: "abc123".to_string(),
                bundle_kind: default_bundle_kind(),
                build_host: "x86_64-linux".to_string(),
                native_portable: true,
                entry_module: Some("main".to_string()),
                built_at: 1700000000,
                readme: None,
            },
            modules: vec![
                BundledModule {
                    module_path: "main".to_string(),
                    bytecode_bytes: vec![1, 2, 3, 4],
                    export_names: vec!["run".to_string()],
                    source_hash: "def456".to_string(),
                },
                BundledModule {
                    module_path: "utils::helpers".to_string(),
                    bytecode_bytes: vec![5, 6, 7],
                    export_names: vec!["helper".to_string(), "format".to_string()],
                    source_hash: "ghi789".to_string(),
                },
            ],
            dependencies: {
                let mut deps = HashMap::new();
                deps.insert("my-lib".to_string(), "1.0.0".to_string());
                deps
            },
            blob_store: HashMap::new(),
            manifests: vec![],
            native_dependency_scopes: vec![],
            docs: HashMap::new(),
        }
    }

    #[test]
    fn test_roundtrip_serialize_deserialize() {
        let bundle = sample_bundle();
        let bytes = bundle.to_bytes().expect("serialization should succeed");
        let restored = PackageBundle::from_bytes(&bytes).expect("deserialization should succeed");

        assert_eq!(restored.metadata.name, "test-pkg");
        assert_eq!(restored.metadata.version, "0.1.0");
        assert_eq!(restored.modules.len(), 2);
        assert_eq!(restored.modules[0].module_path, "main");
        assert_eq!(restored.modules[0].bytecode_bytes, vec![1, 2, 3, 4]);
        assert_eq!(restored.modules[1].module_path, "utils::helpers");
        assert_eq!(restored.dependencies.get("my-lib").unwrap(), "1.0.0");
        assert!(restored.blob_store.is_empty());
        assert!(restored.manifests.is_empty());
    }

    #[test]
    fn test_magic_bytes_validation() {
        let mut bad_data = vec![0u8; 20];
        bad_data[..8].copy_from_slice(b"BADMAGIC");
        let result = PackageBundle::from_bytes(&bad_data);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bad magic bytes"));
    }

    #[test]
    fn test_version_validation() {
        let mut data = vec![0u8; 20];
        data[..8].copy_from_slice(MAGIC);
        data[8..12].copy_from_slice(&99u32.to_le_bytes());
        let result = PackageBundle::from_bytes(&data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Unsupported bundle format version")
        );
    }

    #[test]
    fn test_too_small_data() {
        let result = PackageBundle::from_bytes(&[1, 2, 3]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too small"));
    }

    #[test]
    fn test_empty_bundle() {
        let bundle = PackageBundle {
            metadata: BundleMetadata {
                name: "empty".to_string(),
                version: "0.0.1".to_string(),
                compiler_version: "0.5.0".to_string(),
                source_hash: "empty".to_string(),
                bundle_kind: default_bundle_kind(),
                build_host: "x86_64-linux".to_string(),
                native_portable: true,
                entry_module: None,
                built_at: 0,
                readme: None,
            },
            modules: vec![],
            dependencies: HashMap::new(),
            blob_store: HashMap::new(),
            manifests: vec![],
            native_dependency_scopes: vec![],
            docs: HashMap::new(),
        };

        let bytes = bundle.to_bytes().expect("should serialize");
        let restored = PackageBundle::from_bytes(&bytes).expect("should deserialize");
        assert_eq!(restored.metadata.name, "empty");
        assert!(restored.modules.is_empty());
        assert!(restored.dependencies.is_empty());
    }

    #[test]
    fn test_file_roundtrip() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let path = tmp.path().join("test.shapec");

        let bundle = sample_bundle();
        bundle.write_to_file(&path).expect("write should succeed");
        let restored = PackageBundle::read_from_file(&path).expect("read should succeed");

        assert_eq!(restored.metadata.name, "test-pkg");
        assert_eq!(restored.modules.len(), 2);
    }

    #[test]
    fn test_bundle_with_blob_store_and_manifests() {
        let blob_hash = [0xAB; 32];
        let blob_data = vec![10, 20, 30, 40];

        let mut manifest = ModuleManifest::new("mymod".into(), "1.0.0".into());
        manifest.add_export("greet".into(), blob_hash);
        manifest.finalize();

        let bundle = PackageBundle {
            metadata: BundleMetadata {
                name: "ca-pkg".to_string(),
                version: "2.0.0".to_string(),
                compiler_version: "0.6.0".to_string(),
                source_hash: "ca_hash".to_string(),
                bundle_kind: default_bundle_kind(),
                build_host: "x86_64-linux".to_string(),
                native_portable: true,
                entry_module: None,
                built_at: 1700000001,
                readme: None,
            },
            modules: vec![],
            dependencies: HashMap::new(),
            blob_store: {
                let mut bs = HashMap::new();
                bs.insert(blob_hash, blob_data.clone());
                bs
            },
            manifests: vec![manifest],
            native_dependency_scopes: vec![],
            docs: HashMap::new(),
        };

        let bytes = bundle.to_bytes().expect("serialization should succeed");
        let restored = PackageBundle::from_bytes(&bytes).expect("deserialization should succeed");

        assert_eq!(restored.metadata.name, "ca-pkg");
        assert_eq!(restored.manifests.len(), 1);
        assert_eq!(restored.manifests[0].name, "mymod");
        assert!(restored.manifests[0].verify_integrity());
        assert_eq!(restored.blob_store.get(&blob_hash), Some(&blob_data));
        assert!(restored.modules.is_empty());
    }

    // --- verify_bundle_checksum tests ---

    fn sha256_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    #[test]
    fn test_verify_checksum_correct() {
        let data = b"hello world";
        let hash = sha256_hex(data);
        assert!(verify_bundle_checksum(data, &hash));
    }

    #[test]
    fn test_verify_checksum_wrong() {
        let data = b"hello world";
        assert!(!verify_bundle_checksum(data, "0000000000000000000000000000000000000000000000000000000000000000"));
    }

    #[test]
    fn test_verify_checksum_with_sha256_prefix() {
        let data = b"test data";
        let hash = sha256_hex(data);
        let prefixed = format!("sha256:{}", hash);
        assert!(verify_bundle_checksum(data, &prefixed));
    }

    #[test]
    fn test_verify_checksum_without_prefix() {
        let data = b"test data";
        let hash = sha256_hex(data);
        assert!(verify_bundle_checksum(data, &hash));
    }

    #[test]
    fn test_verify_checksum_empty_data() {
        let data = b"";
        let hash = sha256_hex(data);
        assert!(verify_bundle_checksum(data, &hash));
    }

    #[test]
    fn test_verify_checksum_case_sensitive() {
        let data = b"case test";
        let hash = sha256_hex(data).to_uppercase();
        // hex::encode produces lowercase; uppercase should fail
        assert!(!verify_bundle_checksum(data, &hash));
    }

    #[test]
    fn test_bundle_blob_deduplication() {
        let shared_hash = [0x01; 32];
        let shared_blob = vec![99, 88, 77];

        let mut m1 = ModuleManifest::new("mod_a".into(), "1.0.0".into());
        m1.add_export("fn_a".into(), shared_hash);
        m1.finalize();

        let mut m2 = ModuleManifest::new("mod_b".into(), "1.0.0".into());
        m2.add_export("fn_b".into(), shared_hash);
        m2.finalize();

        let bundle = PackageBundle {
            metadata: BundleMetadata {
                name: "dedup-pkg".to_string(),
                version: "1.0.0".to_string(),
                compiler_version: "0.6.0".to_string(),
                source_hash: "dedup".to_string(),
                bundle_kind: default_bundle_kind(),
                build_host: "x86_64-linux".to_string(),
                native_portable: true,
                entry_module: None,
                built_at: 0,
                readme: None,
            },
            modules: vec![],
            dependencies: HashMap::new(),
            blob_store: {
                let mut bs = HashMap::new();
                bs.insert(shared_hash, shared_blob.clone());
                bs
            },
            manifests: vec![m1, m2],
            native_dependency_scopes: vec![],
            docs: HashMap::new(),
        };

        let bytes = bundle.to_bytes().expect("serialize");
        let restored = PackageBundle::from_bytes(&bytes).expect("deserialize");

        // Both manifests reference the same hash, but blob_store has it once.
        assert_eq!(restored.blob_store.len(), 1);
        assert_eq!(restored.blob_store.get(&shared_hash), Some(&shared_blob));
        assert_eq!(restored.manifests.len(), 2);
    }
}
