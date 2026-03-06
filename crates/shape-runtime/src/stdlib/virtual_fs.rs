//! In-memory virtual filesystem for sandbox mode.
//!
//! All reads and writes operate against an in-memory store. The host can
//! pre-seed read-only files before execution and extract written files after.
//! No real disk I/O is performed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use super::runtime_policy::{FileMetadata, FileSystemProvider, PathEntry};

// ============================================================================
// VFS Types
// ============================================================================

/// A single entry in the virtual filesystem.
#[derive(Debug, Clone)]
struct VfsEntry {
    content: Vec<u8>,
    is_dir: bool,
}

/// In-memory virtual filesystem implementing [`FileSystemProvider`].
///
/// Thread-safe via internal `RwLock`. Designed for sandbox mode where no real
/// disk access is permitted.
pub struct VirtualFilesystem {
    files: RwLock<HashMap<PathBuf, VfsEntry>>,
    read_only: RwLock<HashSet<PathBuf>>,
    total_written: RwLock<usize>,
    max_size: usize,
}

impl VirtualFilesystem {
    /// Create a new empty VFS with the given maximum total size in bytes.
    ///
    /// `max_size` of `0` means unlimited.
    pub fn new(max_size: usize) -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
            read_only: RwLock::new(HashSet::new()),
            total_written: RwLock::new(0),
            max_size,
        }
    }

    /// Pre-seed a read-only file. Typically called by the host before
    /// executing a sandboxed program (e.g., from `[sandbox.seed_files]`).
    pub fn seed_file(&self, path: impl Into<PathBuf>, content: Vec<u8>) {
        let path = path.into();
        // Ensure parent directories exist.
        self.ensure_parents(&path);
        let mut files = self.files.write().unwrap();
        files.insert(
            path.clone(),
            VfsEntry {
                content,
                is_dir: false,
            },
        );
        self.read_only.write().unwrap().insert(path);
    }

    /// Seed a directory (empty). Automatically seeds all parent directories.
    pub fn seed_dir(&self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.ensure_parents(&path);
        let mut files = self.files.write().unwrap();
        files.insert(
            path.clone(),
            VfsEntry {
                content: Vec::new(),
                is_dir: true,
            },
        );
        self.read_only.write().unwrap().insert(path);
    }

    /// Extract all files written by the sandboxed program (excludes seed files).
    ///
    /// Returns a map from path to file content.
    pub fn extract_written_files(&self) -> HashMap<PathBuf, Vec<u8>> {
        let files = self.files.read().unwrap();
        let ro = self.read_only.read().unwrap();
        files
            .iter()
            .filter(|(p, e)| !e.is_dir && !ro.contains(*p))
            .map(|(p, e)| (p.clone(), e.content.clone()))
            .collect()
    }

    /// Total bytes written by the sandboxed program (excludes seed files).
    pub fn total_bytes_written(&self) -> usize {
        *self.total_written.read().unwrap()
    }

    /// Ensure all ancestor directories of `path` exist as directory entries.
    fn ensure_parents(&self, path: &Path) {
        let mut files = self.files.write().unwrap();
        for ancestor in path.ancestors().skip(1) {
            if ancestor == Path::new("") || ancestor == Path::new("/") {
                // Always implicitly exists.
                continue;
            }
            files
                .entry(ancestor.to_path_buf())
                .or_insert_with(|| VfsEntry {
                    content: Vec::new(),
                    is_dir: true,
                });
        }
    }

    /// Check size limit, returning PermissionDenied if exceeded.
    fn check_size_limit(&self, additional: usize) -> std::io::Result<()> {
        if self.max_size == 0 {
            return Ok(());
        }
        let current = *self.total_written.read().unwrap();
        if current + additional > self.max_size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "VFS size limit exceeded: {} + {} > {}",
                    current, additional, self.max_size
                ),
            ));
        }
        Ok(())
    }
}

impl FileSystemProvider for VirtualFilesystem {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        let files = self.files.read().unwrap();
        match files.get(path) {
            Some(entry) if !entry.is_dir => Ok(entry.content.clone()),
            Some(_) => Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{} is a directory", path.display()),
            )),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} not found in VFS", path.display()),
            )),
        }
    }

    fn write(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        if self.read_only.read().unwrap().contains(path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} is read-only (seed file)", path.display()),
            ));
        }
        self.check_size_limit(data.len())?;
        self.ensure_parents(path);
        let mut files = self.files.write().unwrap();

        // Subtract old size if overwriting a user-written file.
        let old_size = files
            .get(path)
            .filter(|e| !e.is_dir)
            .map(|e| e.content.len())
            .unwrap_or(0);

        files.insert(
            path.to_path_buf(),
            VfsEntry {
                content: data.to_vec(),
                is_dir: false,
            },
        );

        let mut total = self.total_written.write().unwrap();
        *total = total.saturating_sub(old_size) + data.len();
        Ok(())
    }

    fn append(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        if self.read_only.read().unwrap().contains(path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} is read-only (seed file)", path.display()),
            ));
        }
        self.check_size_limit(data.len())?;
        self.ensure_parents(path);
        let mut files = self.files.write().unwrap();
        let entry = files.entry(path.to_path_buf()).or_insert_with(|| VfsEntry {
            content: Vec::new(),
            is_dir: false,
        });
        if entry.is_dir {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{} is a directory", path.display()),
            ));
        }
        entry.content.extend_from_slice(data);
        *self.total_written.write().unwrap() += data.len();
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        // Root always exists.
        if path == Path::new("/") || path == Path::new("") {
            return true;
        }
        self.files.read().unwrap().contains_key(path)
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        if self.read_only.read().unwrap().contains(path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} is read-only (seed file)", path.display()),
            ));
        }
        let mut files = self.files.write().unwrap();
        match files.remove(path) {
            Some(entry) if !entry.is_dir => {
                // Reclaim written bytes.
                let mut total = self.total_written.write().unwrap();
                *total = total.saturating_sub(entry.content.len());
                Ok(())
            }
            Some(entry) => {
                // Put it back — can't remove non-empty dir through this API.
                files.insert(path.to_path_buf(), entry);
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{} is a directory", path.display()),
                ))
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} not found in VFS", path.display()),
            )),
        }
    }

    fn list_dir(&self, path: &Path) -> std::io::Result<Vec<PathEntry>> {
        let files = self.files.read().unwrap();
        // Check the directory exists (root is implicit).
        let is_root = path == Path::new("/") || path == Path::new("");
        if !is_root {
            match files.get(path) {
                Some(e) if e.is_dir => {}
                Some(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("{} is not a directory", path.display()),
                    ));
                }
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("{} not found in VFS", path.display()),
                    ));
                }
            }
        }

        let mut entries = Vec::new();
        for (p, e) in files.iter() {
            if let Some(parent) = p.parent() {
                if parent == path && p != path {
                    entries.push(PathEntry {
                        path: p.clone(),
                        is_dir: e.is_dir,
                    });
                }
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    fn metadata(&self, path: &Path) -> std::io::Result<FileMetadata> {
        let files = self.files.read().unwrap();
        match files.get(path) {
            Some(entry) => Ok(FileMetadata {
                size: entry.content.len() as u64,
                is_dir: entry.is_dir,
                is_file: !entry.is_dir,
                readonly: self.read_only.read().unwrap().contains(path),
            }),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} not found in VFS", path.display()),
            )),
        }
    }

    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        let mut files = self.files.write().unwrap();
        for ancestor in path.ancestors() {
            if ancestor == Path::new("") || ancestor == Path::new("/") {
                continue;
            }
            files
                .entry(ancestor.to_path_buf())
                .or_insert_with(|| VfsEntry {
                    content: Vec::new(),
                    is_dir: true,
                });
        }
        // Ensure the target itself is also created.
        files.entry(path.to_path_buf()).or_insert_with(|| VfsEntry {
            content: Vec::new(),
            is_dir: true,
        });
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn vfs() -> VirtualFilesystem {
        VirtualFilesystem::new(0) // unlimited
    }

    // -- Basic read/write --

    #[test]
    fn write_then_read() {
        let fs = vfs();
        fs.write(Path::new("/hello.txt"), b"world").unwrap();
        let data = fs.read(Path::new("/hello.txt")).unwrap();
        assert_eq!(data, b"world");
    }

    #[test]
    fn read_nonexistent() {
        let fs = vfs();
        let err = fs.read(Path::new("/nope")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn overwrite_file() {
        let fs = vfs();
        fs.write(Path::new("/f"), b"old").unwrap();
        fs.write(Path::new("/f"), b"new").unwrap();
        assert_eq!(fs.read(Path::new("/f")).unwrap(), b"new");
    }

    // -- Append --

    #[test]
    fn append_creates_and_extends() {
        let fs = vfs();
        fs.append(Path::new("/log.txt"), b"line1\n").unwrap();
        fs.append(Path::new("/log.txt"), b"line2\n").unwrap();
        assert_eq!(fs.read(Path::new("/log.txt")).unwrap(), b"line1\nline2\n");
    }

    // -- Seed files (read-only) --

    #[test]
    fn seed_file_is_readable() {
        let fs = vfs();
        fs.seed_file(PathBuf::from("/config.toml"), b"key = true".to_vec());
        assert_eq!(fs.read(Path::new("/config.toml")).unwrap(), b"key = true");
    }

    #[test]
    fn seed_file_is_not_writable() {
        let fs = vfs();
        fs.seed_file(PathBuf::from("/config.toml"), b"data".to_vec());
        let err = fs.write(Path::new("/config.toml"), b"hacked").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn seed_file_is_not_appendable() {
        let fs = vfs();
        fs.seed_file(PathBuf::from("/config.toml"), b"data".to_vec());
        let err = fs.append(Path::new("/config.toml"), b"extra").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn seed_file_is_not_removable() {
        let fs = vfs();
        fs.seed_file(PathBuf::from("/config.toml"), b"data".to_vec());
        let err = fs.remove(Path::new("/config.toml")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn seed_file_not_in_extract() {
        let fs = vfs();
        fs.seed_file(PathBuf::from("/seed.txt"), b"seed".to_vec());
        fs.write(Path::new("/output.txt"), b"output").unwrap();
        let written = fs.extract_written_files();
        assert!(written.contains_key(Path::new("/output.txt")));
        assert!(!written.contains_key(Path::new("/seed.txt")));
    }

    // -- Directories --

    #[test]
    fn create_dir_and_list() {
        let fs = vfs();
        fs.create_dir_all(Path::new("/data/subdir")).unwrap();
        fs.write(Path::new("/data/subdir/file.txt"), b"hi").unwrap();
        let entries = fs.list_dir(Path::new("/data/subdir")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/data/subdir/file.txt"));
        assert!(!entries[0].is_dir);
    }

    #[test]
    fn list_root() {
        let fs = vfs();
        fs.write(Path::new("/a.txt"), b"a").unwrap();
        fs.create_dir_all(Path::new("/dir")).unwrap();
        let entries = fs.list_dir(Path::new("/")).unwrap();
        assert!(entries.len() >= 2);
    }

    #[test]
    fn read_directory_fails() {
        let fs = vfs();
        fs.create_dir_all(Path::new("/mydir")).unwrap();
        let err = fs.read(Path::new("/mydir")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
    }

    // -- Size limits --

    #[test]
    fn size_limit_enforced() {
        let fs = VirtualFilesystem::new(10);
        fs.write(Path::new("/a"), b"12345").unwrap();
        assert_eq!(fs.total_bytes_written(), 5);
        // This should fail — would exceed limit
        let err = fs.write(Path::new("/b"), b"123456").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
    }

    #[test]
    fn overwrite_reclaims_space() {
        let fs = VirtualFilesystem::new(20);
        fs.write(Path::new("/f"), b"1234567890").unwrap(); // 10 bytes
        assert_eq!(fs.total_bytes_written(), 10);
        fs.write(Path::new("/f"), b"ab").unwrap(); // replace with 2 bytes
        assert_eq!(fs.total_bytes_written(), 2);
    }

    // -- Exists --

    #[test]
    fn exists_for_files_and_dirs() {
        let fs = vfs();
        assert!(!fs.exists(Path::new("/x")));
        fs.write(Path::new("/x"), b"data").unwrap();
        assert!(fs.exists(Path::new("/x")));
        fs.create_dir_all(Path::new("/d")).unwrap();
        assert!(fs.exists(Path::new("/d")));
    }

    #[test]
    fn root_always_exists() {
        let fs = vfs();
        assert!(fs.exists(Path::new("/")));
    }

    // -- Remove --

    #[test]
    fn remove_file() {
        let fs = vfs();
        fs.write(Path::new("/f"), b"data").unwrap();
        fs.remove(Path::new("/f")).unwrap();
        assert!(!fs.exists(Path::new("/f")));
    }

    #[test]
    fn remove_nonexistent_errors() {
        let fs = vfs();
        let err = fs.remove(Path::new("/nope")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn remove_reclaims_bytes() {
        let fs = VirtualFilesystem::new(100);
        fs.write(Path::new("/f"), b"12345").unwrap();
        assert_eq!(fs.total_bytes_written(), 5);
        fs.remove(Path::new("/f")).unwrap();
        assert_eq!(fs.total_bytes_written(), 0);
    }

    // -- Metadata --

    #[test]
    fn metadata_file() {
        let fs = vfs();
        fs.write(Path::new("/f"), b"hello").unwrap();
        let m = fs.metadata(Path::new("/f")).unwrap();
        assert!(m.is_file);
        assert!(!m.is_dir);
        assert_eq!(m.size, 5);
        assert!(!m.readonly);
    }

    #[test]
    fn metadata_seed_file_is_readonly() {
        let fs = vfs();
        fs.seed_file(PathBuf::from("/s"), b"data".to_vec());
        let m = fs.metadata(Path::new("/s")).unwrap();
        assert!(m.readonly);
    }

    #[test]
    fn metadata_dir() {
        let fs = vfs();
        fs.create_dir_all(Path::new("/d")).unwrap();
        let m = fs.metadata(Path::new("/d")).unwrap();
        assert!(m.is_dir);
        assert!(!m.is_file);
    }

    // -- Parent directory auto-creation --

    #[test]
    fn writing_deep_path_creates_parents() {
        let fs = vfs();
        fs.write(Path::new("/a/b/c/file.txt"), b"deep").unwrap();
        assert!(fs.exists(Path::new("/a")));
        assert!(fs.exists(Path::new("/a/b")));
        assert!(fs.exists(Path::new("/a/b/c")));
        let m = fs.metadata(Path::new("/a/b")).unwrap();
        assert!(m.is_dir);
    }
}
