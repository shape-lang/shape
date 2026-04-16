//! Unified `shape.lock` model for deterministic dependency resolution and
//! compile-time artifact caching.
//!
//! This is the single source of truth for:
//! - resolved package dependencies
//! - compile-time artifacts (schema inference, comptime outputs, generated metadata)

use serde::{Deserialize, Serialize};
use shape_value::ValueWordExt;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::project::DependencySpec;

/// Top-level lockfile structure written to `shape.lock`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageLock {
    /// Lockfile format version (currently "1").
    pub version: String,
    /// Locked packages in dependency order.
    pub packages: Vec<LockedPackage>,
    /// Cached compile-time artifacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<LockedArtifact>,
}

/// A single locked package entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockedPackage {
    /// Package name (matches the key in `[dependencies]`).
    pub name: String,
    /// Resolved version string.
    pub version: String,
    /// How the package was resolved.
    pub source: LockedSource,
    /// SHA-256 hash of the package contents for integrity verification.
    pub content_hash: String,
    /// Names of direct dependencies of this package.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

/// Source from which a locked package was resolved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum LockedSource {
    /// Local filesystem path dependency.
    Path { path: String },
    /// Git repository dependency, pinned to a specific revision.
    Git { url: String, rev: String },
    /// Registry dependency (future).
    Registry {
        version: String,
        #[serde(default)]
        registry: Option<String>,
        #[serde(default)]
        path: Option<String>,
    },
}

/// Reproducibility mode for a compile-time artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ArtifactDeterminism {
    /// Artifact depends only on compiler inputs (source/deps/config) and is hermetic.
    Hermetic,
    /// Artifact depends on external mutable inputs.
    /// Each entry must carry a fingerprint used for invalidation.
    External {
        fingerprints: BTreeMap<String, String>,
    },
}

impl ArtifactDeterminism {
    fn validate(&self) -> Result<(), String> {
        match self {
            ArtifactDeterminism::Hermetic => Ok(()),
            ArtifactDeterminism::External { fingerprints } => {
                if fingerprints.is_empty() {
                    Err(
                        "external artifact determinism requires at least one fingerprint"
                            .to_string(),
                    )
                } else {
                    Ok(())
                }
            }
        }
    }

    fn augment_inputs(&self, inputs: &mut BTreeMap<String, String>) {
        if let ArtifactDeterminism::External { fingerprints } = self {
            for (key, value) in fingerprints {
                let merged_key = format!("external::{key}");
                inputs.entry(merged_key).or_insert_with(|| value.clone());
            }
        }
    }
}

/// A generic compile-time artifact cached in `shape.lock`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockedArtifact {
    /// Namespace identifying artifact kind (e.g., `schema.infer`).
    pub namespace: String,
    /// Logical key within namespace (e.g., source path).
    pub key: String,
    /// Producer identifier (e.g., `shape-runtime/schema_inference@v1`).
    pub producer: String,
    /// Determinism/invalidation policy.
    pub determinism: ArtifactDeterminism,
    /// Explicit input components used for invalidation and debugging.
    pub inputs: BTreeMap<String, String>,
    /// Stable hash derived from inputs + determinism policy.
    pub inputs_hash: String,
    /// RFC3339 timestamp of artifact creation.
    pub created_at: String,
    /// Shape-wire payload encoded as JSON for TOML compatibility.
    pub payload_json: String,
}

impl LockedArtifact {
    /// Build a new artifact entry from a shape-wire payload.
    pub fn new(
        namespace: impl Into<String>,
        key: impl Into<String>,
        producer: impl Into<String>,
        determinism: ArtifactDeterminism,
        mut inputs: BTreeMap<String, String>,
        payload: shape_wire::WireValue,
    ) -> Result<Self, String> {
        determinism.validate()?;
        determinism.augment_inputs(&mut inputs);

        let inputs_hash = PackageLock::hash_inputs(&inputs);
        let payload_json =
            serde_json::to_string(&payload).map_err(|e| format!("invalid wire payload: {e}"))?;

        Ok(Self {
            namespace: namespace.into(),
            key: key.into(),
            producer: producer.into(),
            determinism,
            inputs,
            inputs_hash,
            created_at: chrono::Utc::now().to_rfc3339(),
            payload_json,
        })
    }

    /// Decode the shape-wire payload.
    pub fn payload(&self) -> Result<shape_wire::WireValue, String> {
        serde_json::from_str(&self.payload_json)
            .map_err(|e| format!("invalid artifact payload encoding: {e}"))
    }
}

impl PackageLock {
    const EXTERNAL_REQUIRED_NAMESPACES: [&'static str; 1] = ["schema.infer"];
    const EXTERNAL_REQUIRED_NAMESPACE_PREFIXES: [&'static str; 2] =
        ["external.", "comptime.external."];
    const EXTERNAL_REQUIRED_PRODUCERS: [&'static str; 1] = ["shape-runtime/schema_inference@v1"];

    /// Create a new empty lockfile.
    pub fn new() -> Self {
        Self {
            version: "1".to_string(),
            packages: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    fn requires_external_determinism(namespace: &str, producer: &str) -> bool {
        Self::EXTERNAL_REQUIRED_NAMESPACES.contains(&namespace)
            || Self::EXTERNAL_REQUIRED_NAMESPACE_PREFIXES
                .iter()
                .any(|prefix| namespace.starts_with(prefix))
            || Self::EXTERNAL_REQUIRED_PRODUCERS.contains(&producer)
    }

    /// Read a lockfile from the given path. Returns `None` if the file
    /// doesn't exist or cannot be parsed.
    pub fn read(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let mut lock: Self = toml::from_str(&content).ok()?;
        if lock.version.is_empty() {
            lock.version = "1".to_string();
        }
        Some(lock)
    }

    /// Write the lockfile to the given path.
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let content = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, content)
    }

    /// Check whether this lockfile is still fresh (matches the given deps).
    ///
    /// A lockfile is fresh if every dependency in the spec is present in the
    /// lockfile and every locked package corresponds to a declared dependency.
    pub fn is_fresh(&self, deps: &HashMap<String, DependencySpec>) -> bool {
        for (name, spec) in deps {
            let Some(locked) = self.packages.iter().find(|p| &p.name == name) else {
                return false;
            };

            match spec {
                DependencySpec::Version(req) => {
                    if !locked_version_matches_req(&locked.version, req) {
                        return false;
                    }
                }
                DependencySpec::Detailed(detail) => {
                    // Path/Git details are validated by source/path matching elsewhere.
                    // For semver requirements, enforce lock compatibility.
                    if detail.path.is_none()
                        && detail.git.is_none()
                        && let Some(req) = &detail.version
                        && !locked_version_matches_req(&locked.version, req)
                    {
                        return false;
                    }
                }
            }
        }
        for pkg in &self.packages {
            if !deps.contains_key(&pkg.name) {
                return false;
            }
        }
        true
    }

    fn validate_artifact(artifact: &LockedArtifact) -> Result<(), String> {
        artifact.determinism.validate()?;
        let expected_hash =
            Self::artifact_inputs_hash(artifact.inputs.clone(), &artifact.determinism)?;
        if artifact.inputs_hash != expected_hash {
            return Err(format!(
                "artifact inputs_hash mismatch for {}:{}",
                artifact.namespace, artifact.key
            ));
        }

        if Self::requires_external_determinism(&artifact.namespace, &artifact.producer)
            && !matches!(artifact.determinism, ArtifactDeterminism::External { .. })
        {
            return Err(format!(
                "artifact {}:{} must declare external determinism fingerprints",
                artifact.namespace, artifact.key
            ));
        }

        Ok(())
    }

    /// Upsert a compile-time artifact by `(namespace, key)`.
    pub fn upsert_artifact(&mut self, artifact: LockedArtifact) -> Result<(), String> {
        Self::validate_artifact(&artifact)?;
        if let Some(existing) = self
            .artifacts
            .iter_mut()
            .find(|a| a.namespace == artifact.namespace && a.key == artifact.key)
        {
            *existing = artifact;
        } else {
            self.artifacts.push(artifact);
        }
        Ok(())
    }

    /// Upsert a compile-time artifact by `(namespace, key, inputs_hash)`.
    ///
    /// This is used for host-bound artifacts such as native dependency locks,
    /// where multiple variants for the same logical key may coexist across
    /// targets or fingerprints in a single committed lockfile.
    pub fn upsert_artifact_variant(&mut self, artifact: LockedArtifact) -> Result<(), String> {
        Self::validate_artifact(&artifact)?;
        if let Some(existing) = self.artifacts.iter_mut().find(|a| {
            a.namespace == artifact.namespace
                && a.key == artifact.key
                && a.inputs_hash == artifact.inputs_hash
        }) {
            *existing = artifact;
        } else {
            self.artifacts.push(artifact);
        }
        Ok(())
    }

    /// Lookup artifact by `(namespace, key, inputs_hash)`.
    pub fn artifact(
        &self,
        namespace: &str,
        key: &str,
        inputs_hash: &str,
    ) -> Option<&LockedArtifact> {
        self.artifacts
            .iter()
            .find(|a| a.namespace == namespace && a.key == key && a.inputs_hash == inputs_hash)
    }

    /// Compute a stable SHA-256 hash for inputs map.
    pub fn hash_inputs(inputs: &BTreeMap<String, String>) -> String {
        let mut hasher = Sha256::new();
        for (key, value) in inputs {
            hasher.update(key.as_bytes());
            hasher.update([0]);
            hasher.update(value.as_bytes());
            hasher.update([0xff]);
        }
        format!("sha256:{:x}", hasher.finalize())
    }

    /// Compute artifact inputs hash after applying determinism policy rules.
    pub fn artifact_inputs_hash(
        mut inputs: BTreeMap<String, String>,
        determinism: &ArtifactDeterminism,
    ) -> Result<String, String> {
        determinism.validate()?;
        determinism.augment_inputs(&mut inputs);
        Ok(Self::hash_inputs(&inputs))
    }

    /// Compute a content hash for a directory or file at the given path.
    ///
    /// For files, hashes the file content. For directories, hashes the
    /// concatenation of all `.shape` file contents (sorted by name).
    pub fn hash_path(path: &Path) -> std::io::Result<String> {
        let mut hasher = Sha256::new();

        if path.is_file() {
            let data = std::fs::read(path)?;
            hasher.update(&data);
        } else if path.is_dir() {
            let mut entries: Vec<_> = walkdir::WalkDir::new(path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "shape"))
                .collect();
            entries.sort_by_key(|e| e.path().to_path_buf());
            for entry in entries {
                let data = std::fs::read(entry.path())?;
                hasher.update(&data);
            }
        }

        Ok(format!("{:x}", hasher.finalize()))
    }
}

fn locked_version_matches_req(locked: &str, req: &str) -> bool {
    let Ok(parsed_version) = semver::Version::parse(locked) else {
        return false;
    };
    let Ok(version_req) = semver::VersionReq::parse(req) else {
        return false;
    };
    version_req.matches(&parsed_version)
}

impl Default for PackageLock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::DetailedDependency;

    fn sample_lock() -> PackageLock {
        PackageLock {
            version: "1".to_string(),
            packages: vec![
                LockedPackage {
                    name: "utils".to_string(),
                    version: "0.1.0".to_string(),
                    source: LockedSource::Path {
                        path: "../utils".to_string(),
                    },
                    content_hash: "abc123".to_string(),
                    dependencies: vec![],
                },
                LockedPackage {
                    name: "finance".to_string(),
                    version: "0.2.0".to_string(),
                    source: LockedSource::Git {
                        url: "https://github.com/example/finance.git".to_string(),
                        rev: "deadbeef".to_string(),
                    },
                    content_hash: "def456".to_string(),
                    dependencies: vec!["utils".to_string()],
                },
            ],
            artifacts: vec![],
        }
    }

    #[test]
    fn test_write_read_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_path = tmp.path().join("shape.lock");

        let original = sample_lock();
        original.write(&lock_path).unwrap();

        let loaded = PackageLock::read(&lock_path);
        assert!(loaded.is_some(), "Lockfile should be readable after write");
        assert_eq!(loaded.unwrap(), original);
    }

    #[test]
    fn test_read_missing_file() {
        let result = PackageLock::read(Path::new("/nonexistent/shape.lock"));
        assert!(result.is_none(), "Missing lockfile should return None");
    }

    #[test]
    fn test_is_fresh_matching_deps() {
        let lock = sample_lock();
        let mut deps = HashMap::new();
        deps.insert(
            "utils".to_string(),
            DependencySpec::Detailed(DetailedDependency {
                version: None,
                path: Some("../utils".to_string()),
                git: None,
                tag: None,
                branch: None,
                rev: None,
                permissions: None,
            }),
        );
        deps.insert(
            "finance".to_string(),
            DependencySpec::Detailed(DetailedDependency {
                version: None,
                path: None,
                git: Some("https://github.com/example/finance.git".to_string()),
                tag: None,
                branch: None,
                rev: Some("deadbeef".to_string()),
                permissions: None,
            }),
        );

        assert!(lock.is_fresh(&deps), "Lock should be fresh when deps match");
    }

    #[test]
    fn test_is_fresh_missing_dep() {
        let lock = sample_lock();
        let mut deps = HashMap::new();
        deps.insert(
            "utils".to_string(),
            DependencySpec::Version("0.1.0".to_string()),
        );
        deps.insert(
            "finance".to_string(),
            DependencySpec::Version("0.2.0".to_string()),
        );
        deps.insert(
            "new-dep".to_string(),
            DependencySpec::Version("1.0.0".to_string()),
        );

        assert!(
            !lock.is_fresh(&deps),
            "Lock should be stale when a new dep is added"
        );
    }

    #[test]
    fn test_is_fresh_removed_dep() {
        let lock = sample_lock();
        let mut deps = HashMap::new();
        deps.insert(
            "utils".to_string(),
            DependencySpec::Version("0.1.0".to_string()),
        );

        assert!(
            !lock.is_fresh(&deps),
            "Lock should be stale when a dep is removed"
        );
    }

    #[test]
    fn test_hash_path_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test.shape");
        std::fs::write(&file, "let x = 1").unwrap();

        let hash1 = PackageLock::hash_path(&file).unwrap();
        let hash2 = PackageLock::hash_path(&file).unwrap();
        assert_eq!(hash1, hash2, "Same content should produce same hash");
        assert!(!hash1.is_empty(), "Hash should not be empty");
    }

    #[test]
    fn test_hash_path_directory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.shape"), "let a = 1").unwrap();
        std::fs::write(tmp.path().join("b.shape"), "let b = 2").unwrap();
        std::fs::write(tmp.path().join("README.md"), "not shape").unwrap();

        let hash = PackageLock::hash_path(tmp.path()).unwrap();
        assert!(!hash.is_empty(), "Directory hash should not be empty");
    }

    #[test]
    fn test_artifact_external_requires_fingerprints() {
        let err = LockedArtifact::new(
            "schema.infer",
            "data.csv",
            "shape-runtime/schema_inference@v1",
            ArtifactDeterminism::External {
                fingerprints: BTreeMap::new(),
            },
            BTreeMap::new(),
            shape_wire::WireValue::Null,
        )
        .unwrap_err();
        assert!(err.contains("requires at least one fingerprint"));
    }

    #[test]
    fn test_artifact_roundtrip_and_lookup() {
        let mut inputs = BTreeMap::new();
        inputs.insert("source".to_string(), "data.csv".to_string());
        inputs.insert("file_hash".to_string(), "sha256:abc".to_string());

        let mut fingerprints = BTreeMap::new();
        fingerprints.insert("file:data.csv".to_string(), "sha256:abc".to_string());

        let payload = shape_wire::WireValue::Object(BTreeMap::from([(
            "ok".to_string(),
            shape_wire::WireValue::Bool(true),
        )]));
        let artifact = LockedArtifact::new(
            "schema.infer",
            "data.csv",
            "shape-runtime/schema_inference@v1",
            ArtifactDeterminism::External { fingerprints },
            inputs.clone(),
            payload.clone(),
        )
        .expect("artifact should build");

        let hash = artifact.inputs_hash.clone();
        let mut lock = PackageLock::new();
        lock.upsert_artifact(artifact).unwrap();

        let found = lock
            .artifact("schema.infer", "data.csv", &hash)
            .expect("artifact should be found");
        assert_eq!(found.payload().unwrap(), payload);
    }

    #[test]
    fn test_upsert_artifact_variant_keeps_multiple_fingerprints() {
        let mut inputs_a = BTreeMap::new();
        inputs_a.insert("target".to_string(), "linux-x86_64-gnu".to_string());
        let mut fp_a = BTreeMap::new();
        fp_a.insert(
            "native:linux-x86_64-gnu:duckdb@0.1.0:duckdb:system".to_string(),
            "system-name:libduckdb.so:version:1.0.0".to_string(),
        );
        let artifact_a = LockedArtifact::new(
            "external.native.library",
            "duckdb@0.1.0::duckdb",
            "shape-runtime/native_resolution@v1",
            ArtifactDeterminism::External { fingerprints: fp_a },
            inputs_a,
            shape_wire::WireValue::String("linux".to_string()),
        )
        .expect("artifact should build");
        let hash_a = artifact_a.inputs_hash.clone();

        let mut inputs_b = BTreeMap::new();
        inputs_b.insert("target".to_string(), "darwin-aarch64".to_string());
        let mut fp_b = BTreeMap::new();
        fp_b.insert(
            "native:darwin-aarch64:duckdb@0.1.0:duckdb:system".to_string(),
            "system-name:libduckdb.dylib:version:1.0.0".to_string(),
        );
        let artifact_b = LockedArtifact::new(
            "external.native.library",
            "duckdb@0.1.0::duckdb",
            "shape-runtime/native_resolution@v1",
            ArtifactDeterminism::External { fingerprints: fp_b },
            inputs_b,
            shape_wire::WireValue::String("darwin".to_string()),
        )
        .expect("artifact should build");
        let hash_b = artifact_b.inputs_hash.clone();

        let mut lock = PackageLock::new();
        lock.upsert_artifact_variant(artifact_a).unwrap();
        lock.upsert_artifact_variant(artifact_b).unwrap();

        assert!(
            lock.artifact("external.native.library", "duckdb@0.1.0::duckdb", &hash_a)
                .is_some()
        );
        assert!(
            lock.artifact("external.native.library", "duckdb@0.1.0::duckdb", &hash_b)
                .is_some()
        );
        assert_eq!(lock.artifacts.len(), 2);
    }

    #[test]
    fn test_schema_namespace_requires_external_determinism() {
        let mut lock = PackageLock::new();
        let artifact = LockedArtifact::new(
            "schema.infer",
            "data.csv",
            "shape-runtime/schema_inference@v1",
            ArtifactDeterminism::Hermetic,
            BTreeMap::new(),
            shape_wire::WireValue::Null,
        )
        .unwrap();

        let err = lock.upsert_artifact(artifact).unwrap_err();
        assert!(err.contains("must declare external determinism"));
    }

    #[test]
    fn test_external_namespace_prefix_requires_external_determinism() {
        let mut lock = PackageLock::new();
        let artifact = LockedArtifact::new(
            "external.datasource.schema",
            "orders.csv",
            "shape-ext/csv@v1",
            ArtifactDeterminism::Hermetic,
            BTreeMap::new(),
            shape_wire::WireValue::Null,
        )
        .unwrap();

        let err = lock.upsert_artifact(artifact).unwrap_err();
        assert!(err.contains("external.datasource.schema:orders.csv"));
    }

    #[test]
    fn test_artifacts_persist_through_lock_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_path = tmp.path().join("shape.lock");

        let mut inputs = BTreeMap::new();
        inputs.insert("source".to_string(), "prices.csv".to_string());
        inputs.insert("file_hash".to_string(), "sha256:def".to_string());

        let mut fingerprints = BTreeMap::new();
        fingerprints.insert("file:prices.csv".to_string(), "sha256:def".to_string());

        let artifact = LockedArtifact::new(
            "schema.infer",
            "prices.csv",
            "shape-runtime/schema_inference@v1",
            ArtifactDeterminism::External { fingerprints },
            inputs,
            shape_wire::WireValue::String("cached".to_string()),
        )
        .expect("artifact should build");
        let hash = artifact.inputs_hash.clone();

        let mut lock = sample_lock();
        lock.upsert_artifact(artifact).unwrap();
        lock.write(&lock_path).unwrap();

        let loaded = PackageLock::read(&lock_path).expect("lockfile should parse");
        let cached = loaded
            .artifact("schema.infer", "prices.csv", &hash)
            .expect("artifact should roundtrip");
        assert_eq!(
            cached.payload().unwrap(),
            shape_wire::WireValue::String("cached".to_string())
        );
    }
}
