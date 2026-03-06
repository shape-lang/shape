//! Runtime policy and filesystem provider abstraction.
//!
//! [`RuntimePolicy`] captures resource limits and scoped access rules.
//! [`FileSystemProvider`] is the trait through which all stdlib filesystem
//! operations are dispatched, allowing the host to swap in a virtual FS,
//! a policy-enforced wrapper, or a routing layer without changing callers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

// ============================================================================
// Runtime Policy
// ============================================================================

/// Runtime-scoped policy governing what a Shape program may access.
///
/// Threaded through the VM execution context as `Option<Arc<RuntimePolicy>>`.
/// `None` means unrestricted (default for trusted programs).
#[derive(Debug, Clone)]
pub struct RuntimePolicy {
    /// Filesystem paths the program may access (glob patterns supported).
    /// Empty means all paths are allowed unless the program lacks `FsRead`/`FsWrite`.
    pub allowed_paths: Vec<PathBuf>,
    /// Paths that may only be read, never written.
    pub read_only_paths: Vec<PathBuf>,
    /// Network hosts the program may connect to (supports `*.example.com`).
    /// Empty means all hosts are allowed unless the program lacks `NetConnect`.
    pub allowed_hosts: Vec<String>,
    /// Maximum heap memory in bytes. `None` = unlimited.
    pub memory_limit: Option<usize>,
    /// Maximum wall-clock execution time. `None` = unlimited.
    pub time_limit: Option<Duration>,
    /// Maximum output bytes (stdout + sink). `None` = unlimited.
    pub output_limit: Option<usize>,
}

impl RuntimePolicy {
    /// Unrestricted policy (equivalent to not having a policy at all).
    pub fn unrestricted() -> Self {
        Self {
            allowed_paths: Vec::new(),
            read_only_paths: Vec::new(),
            allowed_hosts: Vec::new(),
            memory_limit: None,
            time_limit: None,
            output_limit: None,
        }
    }

    /// Check whether `path` is allowed for reading.
    ///
    /// Returns `true` when:
    /// - `allowed_paths` is empty (no path restrictions), or
    /// - `path` matches at least one entry in `allowed_paths` or `read_only_paths`.
    pub fn is_path_readable(&self, path: &Path) -> bool {
        if self.allowed_paths.is_empty() && self.read_only_paths.is_empty() {
            return true;
        }
        self.path_matches_any(path, &self.allowed_paths)
            || self.path_matches_any(path, &self.read_only_paths)
    }

    /// Check whether `path` is allowed for writing.
    ///
    /// Returns `true` when:
    /// - `allowed_paths` is empty **and** `read_only_paths` is empty, or
    /// - `path` matches at least one entry in `allowed_paths` **and** does NOT
    ///   match any entry in `read_only_paths`.
    pub fn is_path_writable(&self, path: &Path) -> bool {
        if self.allowed_paths.is_empty() && self.read_only_paths.is_empty() {
            return true;
        }
        // If in read-only list, deny writes.
        if self.path_matches_any(path, &self.read_only_paths) {
            return false;
        }
        self.path_matches_any(path, &self.allowed_paths)
    }

    /// Check whether a network host is allowed.
    ///
    /// Returns `true` when `allowed_hosts` is empty or `host` matches at
    /// least one pattern.
    pub fn is_host_allowed(&self, host: &str) -> bool {
        if self.allowed_hosts.is_empty() {
            return true;
        }
        self.allowed_hosts
            .iter()
            .any(|pattern| host_matches(host, pattern))
    }

    /// Does `path` match any entry in `patterns`?
    ///
    /// Matching is prefix-based: `/data` matches `/data/file.txt`.
    fn path_matches_any(&self, path: &Path, patterns: &[PathBuf]) -> bool {
        patterns.iter().any(|allowed| path.starts_with(allowed))
    }
}

/// Simple wildcard host matching: `*.example.com` matches `api.example.com`.
fn host_matches(host: &str, pattern: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host.ends_with(suffix) && host.len() > suffix.len()
    } else {
        host == pattern
    }
}

// ============================================================================
// Filesystem Provider Trait
// ============================================================================

/// Metadata about a filesystem entry.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// Total size in bytes.
    pub size: u64,
    /// True if this entry is a directory.
    pub is_dir: bool,
    /// True if this entry is a regular file.
    pub is_file: bool,
    /// True if the file/directory is read-only.
    pub readonly: bool,
}

/// A single entry returned by `list_dir`.
#[derive(Debug, Clone)]
pub struct PathEntry {
    /// Absolute path to the entry.
    pub path: PathBuf,
    /// True if the entry is a directory.
    pub is_dir: bool,
}

/// Trait for all filesystem operations used by the Shape stdlib.
///
/// Implementations include:
/// - [`RealFileSystem`] — delegates to `std::fs`
/// - [`PolicyEnforcedFs`] — wraps another provider with permission checks
/// - `VirtualFilesystem` (in `virtual_fs.rs`) — in-memory sandbox
pub trait FileSystemProvider: Send + Sync {
    /// Read the entire contents of a file.
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>>;
    /// Write `data` to a file, creating or truncating as needed.
    fn write(&self, path: &Path, data: &[u8]) -> std::io::Result<()>;
    /// Append `data` to a file.
    fn append(&self, path: &Path, data: &[u8]) -> std::io::Result<()>;
    /// Check whether a path exists.
    fn exists(&self, path: &Path) -> bool;
    /// Remove a file.
    fn remove(&self, path: &Path) -> std::io::Result<()>;
    /// List entries in a directory.
    fn list_dir(&self, path: &Path) -> std::io::Result<Vec<PathEntry>>;
    /// Query metadata for a path.
    fn metadata(&self, path: &Path) -> std::io::Result<FileMetadata>;
    /// Recursively create directories.
    fn create_dir_all(&self, path: &Path) -> std::io::Result<()>;
}

// ============================================================================
// RealFileSystem
// ============================================================================

/// Default filesystem provider that delegates to `std::fs`.
#[derive(Debug, Clone, Copy)]
pub struct RealFileSystem;

impl FileSystemProvider for RealFileSystem {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        std::fs::write(path, data)
    }

    fn append(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)?;
        f.write_all(data)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }

    fn list_dir(&self, path: &Path) -> std::io::Result<Vec<PathEntry>> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            entries.push(PathEntry {
                path: entry.path(),
                is_dir: entry.file_type()?.is_dir(),
            });
        }
        Ok(entries)
    }

    fn metadata(&self, path: &Path) -> std::io::Result<FileMetadata> {
        let m = std::fs::metadata(path)?;
        Ok(FileMetadata {
            size: m.len(),
            is_dir: m.is_dir(),
            is_file: m.is_file(),
            readonly: m.permissions().readonly(),
        })
    }

    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }
}

// ============================================================================
// PolicyEnforcedFs
// ============================================================================

/// A filesystem provider that wraps another and enforces a [`RuntimePolicy`].
///
/// All read operations check `policy.is_path_readable()`; all write operations
/// check `policy.is_path_writable()`. If the check fails,
/// `io::ErrorKind::PermissionDenied` is returned.
pub struct PolicyEnforcedFs {
    inner: Arc<dyn FileSystemProvider>,
    policy: Arc<RuntimePolicy>,
}

impl PolicyEnforcedFs {
    pub fn new(inner: Arc<dyn FileSystemProvider>, policy: Arc<RuntimePolicy>) -> Self {
        Self { inner, policy }
    }

    fn check_readable(&self, path: &Path) -> std::io::Result<()> {
        if self.policy.is_path_readable(path) {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("policy denies read access to {}", path.display()),
            ))
        }
    }

    fn check_writable(&self, path: &Path) -> std::io::Result<()> {
        if self.policy.is_path_writable(path) {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("policy denies write access to {}", path.display()),
            ))
        }
    }
}

impl FileSystemProvider for PolicyEnforcedFs {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.check_readable(path)?;
        self.inner.read(path)
    }

    fn write(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        self.check_writable(path)?;
        self.inner.write(path, data)
    }

    fn append(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        self.check_writable(path)?;
        self.inner.append(path, data)
    }

    fn exists(&self, path: &Path) -> bool {
        // exists is a read-like check — deny if not readable
        self.policy.is_path_readable(path) && self.inner.exists(path)
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        self.check_writable(path)?;
        self.inner.remove(path)
    }

    fn list_dir(&self, path: &Path) -> std::io::Result<Vec<PathEntry>> {
        self.check_readable(path)?;
        self.inner.list_dir(path)
    }

    fn metadata(&self, path: &Path) -> std::io::Result<FileMetadata> {
        self.check_readable(path)?;
        self.inner.metadata(path)
    }

    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        self.check_writable(path)?;
        self.inner.create_dir_all(path)
    }
}

// ============================================================================
// RoutingFileSystem
// ============================================================================

/// Routes filesystem operations to different providers based on path prefix.
///
/// Entries are checked in order; the first matching prefix wins. If no prefix
/// matches, the fallback provider is used.
pub struct RoutingFileSystem {
    routes: Vec<(PathBuf, Arc<dyn FileSystemProvider>)>,
    fallback: Arc<dyn FileSystemProvider>,
}

impl RoutingFileSystem {
    /// Create a routing FS with the given prefix-to-provider map and a fallback.
    pub fn new(
        routes: Vec<(PathBuf, Arc<dyn FileSystemProvider>)>,
        fallback: Arc<dyn FileSystemProvider>,
    ) -> Self {
        Self { routes, fallback }
    }

    fn resolve(&self, path: &Path) -> &dyn FileSystemProvider {
        for (prefix, provider) in &self.routes {
            if path.starts_with(prefix) {
                return provider.as_ref();
            }
        }
        self.fallback.as_ref()
    }
}

impl FileSystemProvider for RoutingFileSystem {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.resolve(path).read(path)
    }

    fn write(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        self.resolve(path).write(path, data)
    }

    fn append(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        self.resolve(path).append(path, data)
    }

    fn exists(&self, path: &Path) -> bool {
        self.resolve(path).exists(path)
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        self.resolve(path).remove(path)
    }

    fn list_dir(&self, path: &Path) -> std::io::Result<Vec<PathEntry>> {
        self.resolve(path).list_dir(path)
    }

    fn metadata(&self, path: &Path) -> std::io::Result<FileMetadata> {
        self.resolve(path).metadata(path)
    }

    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        self.resolve(path).create_dir_all(path)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- RuntimePolicy path checks --

    #[test]
    fn unrestricted_allows_everything() {
        let policy = RuntimePolicy::unrestricted();
        assert!(policy.is_path_readable(Path::new("/any/path")));
        assert!(policy.is_path_writable(Path::new("/any/path")));
        assert!(policy.is_host_allowed("any.host.com"));
    }

    #[test]
    fn allowed_paths_restrict_read() {
        let policy = RuntimePolicy {
            allowed_paths: vec![PathBuf::from("/data"), PathBuf::from("/tmp")],
            ..RuntimePolicy::unrestricted()
        };
        assert!(policy.is_path_readable(Path::new("/data/file.txt")));
        assert!(policy.is_path_readable(Path::new("/tmp/scratch")));
        assert!(!policy.is_path_readable(Path::new("/etc/passwd")));
    }

    #[test]
    fn allowed_paths_restrict_write() {
        let policy = RuntimePolicy {
            allowed_paths: vec![PathBuf::from("/data")],
            ..RuntimePolicy::unrestricted()
        };
        assert!(policy.is_path_writable(Path::new("/data/out.txt")));
        assert!(!policy.is_path_writable(Path::new("/etc/shadow")));
    }

    #[test]
    fn read_only_paths_deny_writes() {
        let policy = RuntimePolicy {
            allowed_paths: vec![PathBuf::from("/data")],
            read_only_paths: vec![PathBuf::from("/data/config")],
            ..RuntimePolicy::unrestricted()
        };
        // Can read both
        assert!(policy.is_path_readable(Path::new("/data/file.txt")));
        assert!(policy.is_path_readable(Path::new("/data/config/app.toml")));
        // Can write to /data but not to /data/config
        assert!(policy.is_path_writable(Path::new("/data/file.txt")));
        assert!(!policy.is_path_writable(Path::new("/data/config/app.toml")));
    }

    #[test]
    fn read_only_paths_are_readable_even_without_allowed_paths() {
        let policy = RuntimePolicy {
            read_only_paths: vec![PathBuf::from("/docs")],
            ..RuntimePolicy::unrestricted()
        };
        assert!(policy.is_path_readable(Path::new("/docs/readme.md")));
        assert!(!policy.is_path_writable(Path::new("/docs/readme.md")));
        // Outside both lists — denied when lists are non-empty
        assert!(!policy.is_path_readable(Path::new("/other")));
    }

    // -- Host matching --

    #[test]
    fn exact_host_match() {
        let policy = RuntimePolicy {
            allowed_hosts: vec!["api.example.com".into()],
            ..RuntimePolicy::unrestricted()
        };
        assert!(policy.is_host_allowed("api.example.com"));
        assert!(!policy.is_host_allowed("evil.com"));
    }

    #[test]
    fn wildcard_host_match() {
        let policy = RuntimePolicy {
            allowed_hosts: vec!["*.example.com".into()],
            ..RuntimePolicy::unrestricted()
        };
        assert!(policy.is_host_allowed("api.example.com"));
        assert!(policy.is_host_allowed("sub.example.com"));
        // The bare domain should NOT match *.example.com
        assert!(!policy.is_host_allowed("example.com"));
        assert!(!policy.is_host_allowed("evil.com"));
    }

    #[test]
    fn empty_allowed_hosts_allows_all() {
        let policy = RuntimePolicy::unrestricted();
        assert!(policy.is_host_allowed("anything.com"));
    }

    // -- RealFileSystem basic smoke test --

    #[test]
    fn real_fs_exists() {
        let fs = RealFileSystem;
        // Cargo.toml should exist in the workspace
        assert!(fs.exists(Path::new("/")));
    }

    // -- PolicyEnforcedFs --

    #[test]
    fn policy_enforced_fs_denies_unauthorized_read() {
        let inner: Arc<dyn FileSystemProvider> = Arc::new(RealFileSystem);
        let policy = Arc::new(RuntimePolicy {
            allowed_paths: vec![PathBuf::from("/allowed")],
            ..RuntimePolicy::unrestricted()
        });
        let enforced = PolicyEnforcedFs::new(inner, policy);
        let result = enforced.read(Path::new("/forbidden/file.txt"));
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn policy_enforced_fs_denies_unauthorized_write() {
        let inner: Arc<dyn FileSystemProvider> = Arc::new(RealFileSystem);
        let policy = Arc::new(RuntimePolicy {
            allowed_paths: vec![PathBuf::from("/allowed")],
            ..RuntimePolicy::unrestricted()
        });
        let enforced = PolicyEnforcedFs::new(inner, policy);
        let result = enforced.write(Path::new("/forbidden/file.txt"), b"data");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn policy_enforced_fs_hides_existence() {
        let inner: Arc<dyn FileSystemProvider> = Arc::new(RealFileSystem);
        let policy = Arc::new(RuntimePolicy {
            allowed_paths: vec![PathBuf::from("/nonexistent_prefix")],
            ..RuntimePolicy::unrestricted()
        });
        let enforced = PolicyEnforcedFs::new(inner, policy);
        // "/" exists on disk but the policy doesn't allow reading it
        assert!(!enforced.exists(Path::new("/")));
    }

    // -- RoutingFileSystem --

    /// A trivial in-memory FS for testing routing.
    struct ConstFs {
        data: Vec<u8>,
    }

    impl FileSystemProvider for ConstFs {
        fn read(&self, _path: &Path) -> std::io::Result<Vec<u8>> {
            Ok(self.data.clone())
        }
        fn write(&self, _path: &Path, _data: &[u8]) -> std::io::Result<()> {
            Ok(())
        }
        fn append(&self, _path: &Path, _data: &[u8]) -> std::io::Result<()> {
            Ok(())
        }
        fn exists(&self, _path: &Path) -> bool {
            true
        }
        fn remove(&self, _path: &Path) -> std::io::Result<()> {
            Ok(())
        }
        fn list_dir(&self, _path: &Path) -> std::io::Result<Vec<PathEntry>> {
            Ok(Vec::new())
        }
        fn metadata(&self, _path: &Path) -> std::io::Result<FileMetadata> {
            Ok(FileMetadata {
                size: self.data.len() as u64,
                is_dir: false,
                is_file: true,
                readonly: false,
            })
        }
        fn create_dir_all(&self, _path: &Path) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn routing_fs_dispatches_by_prefix() {
        let a: Arc<dyn FileSystemProvider> = Arc::new(ConstFs {
            data: vec![1, 2, 3],
        });
        let b: Arc<dyn FileSystemProvider> = Arc::new(ConstFs {
            data: vec![4, 5, 6],
        });
        let fallback: Arc<dyn FileSystemProvider> = Arc::new(ConstFs { data: vec![0] });

        let router = RoutingFileSystem::new(
            vec![(PathBuf::from("/a"), a), (PathBuf::from("/b"), b)],
            fallback,
        );

        assert_eq!(router.read(Path::new("/a/file")).unwrap(), vec![1, 2, 3]);
        assert_eq!(router.read(Path::new("/b/file")).unwrap(), vec![4, 5, 6]);
        assert_eq!(router.read(Path::new("/c/file")).unwrap(), vec![0]);
    }

    #[test]
    fn routing_fs_first_match_wins() {
        let first: Arc<dyn FileSystemProvider> = Arc::new(ConstFs { data: vec![1] });
        let second: Arc<dyn FileSystemProvider> = Arc::new(ConstFs { data: vec![2] });
        let fallback: Arc<dyn FileSystemProvider> = Arc::new(ConstFs { data: vec![0] });

        let router = RoutingFileSystem::new(
            vec![
                (PathBuf::from("/data"), first),
                (PathBuf::from("/data"), second),
            ],
            fallback,
        );

        assert_eq!(router.read(Path::new("/data/x")).unwrap(), vec![1]);
    }
}
