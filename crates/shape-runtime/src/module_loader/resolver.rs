//! Unified module artifact resolver abstractions.
//!
//! Module resolution is origin-agnostic:
//! - filesystem modules
//! - embedded stdlib modules
//! - extension-bundled modules

use shape_ast::error::{Result, ShapeError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Resolved module payload content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleCode {
    Source(Arc<str>),
    Compiled(Arc<[u8]>),
    Both {
        source: Arc<str>,
        compiled: Arc<[u8]>,
    },
    /// Content-addressed module: exports are resolved by hash through a
    /// manifest and blobs are fetched from a `BlobStore` on demand.
    ContentAddressed {
        /// Serialized `ModuleManifest` (MessagePack).
        manifest_bytes: Arc<[u8]>,
        /// Pre-fetched blob cache: content hash -> raw blob bytes.
        /// Blobs not in this map will be fetched from the ambient blob store.
        blob_cache: Arc<HashMap<[u8; 32], Vec<u8>>>,
    },
}

impl ModuleCode {
    /// Return source text if present.
    pub fn source(&self) -> Option<&str> {
        match self {
            Self::Source(source) => Some(source),
            Self::Compiled(_) => None,
            Self::Both { source, .. } => Some(source),
            Self::ContentAddressed { .. } => None,
        }
    }

    /// Return compiled payload if present.
    pub fn compiled(&self) -> Option<&[u8]> {
        match self {
            Self::Source(_) => None,
            Self::Compiled(compiled) => Some(compiled),
            Self::Both { compiled, .. } => Some(compiled),
            Self::ContentAddressed { .. } => None,
        }
    }

    /// Return manifest bytes if this is a content-addressed module.
    pub fn manifest_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::ContentAddressed { manifest_bytes, .. } => Some(manifest_bytes),
            _ => None,
        }
    }

    /// Return the blob cache if this is a content-addressed module.
    pub fn blob_cache(&self) -> Option<&HashMap<[u8; 32], Vec<u8>>> {
        match self {
            Self::ContentAddressed { blob_cache, .. } => Some(blob_cache),
            _ => None,
        }
    }
}

/// A resolved module artifact with optional filesystem origin metadata.
#[derive(Debug, Clone)]
pub struct ResolvedModuleArtifact {
    pub module_path: String,
    pub code: ModuleCode,
    pub origin_path: Option<PathBuf>,
}

/// Trait implemented by all module resolvers.
pub trait ModuleResolver {
    fn resolve(
        &self,
        module_path: &str,
        context_path: Option<&Path>,
    ) -> Result<Option<ResolvedModuleArtifact>>;

    fn list_modules(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// In-memory resolver used for embedded stdlib and extension modules.
#[derive(Debug, Clone, Default)]
pub struct InMemoryResolver {
    modules: HashMap<String, ModuleCode>,
}

impl InMemoryResolver {
    pub fn register(&mut self, module_path: impl Into<String>, code: ModuleCode) {
        self.modules.insert(module_path.into(), code);
    }

    pub fn clear(&mut self) {
        self.modules.clear();
    }

    pub fn has(&self, module_path: &str) -> bool {
        self.modules.contains_key(module_path)
    }

    pub fn module_paths(&self) -> Vec<String> {
        let mut items: Vec<String> = self.modules.keys().cloned().collect();
        items.sort();
        items
    }
}

impl ModuleResolver for InMemoryResolver {
    fn resolve(
        &self,
        module_path: &str,
        _context_path: Option<&Path>,
    ) -> Result<Option<ResolvedModuleArtifact>> {
        Ok(self
            .modules
            .get(module_path)
            .cloned()
            .map(|code| ResolvedModuleArtifact {
                module_path: module_path.to_string(),
                code,
                origin_path: None,
            }))
    }

    fn list_modules(&self) -> Result<Vec<String>> {
        Ok(self.module_paths())
    }
}

/// Filesystem resolver using standard module path rules.
pub struct FilesystemResolver<'a> {
    pub stdlib_path: &'a Path,
    pub module_paths: &'a [PathBuf],
    pub dependency_paths: &'a HashMap<String, PathBuf>,
}

impl<'a> ModuleResolver for FilesystemResolver<'a> {
    fn resolve(
        &self,
        module_path: &str,
        context_path: Option<&Path>,
    ) -> Result<Option<ResolvedModuleArtifact>> {
        let resolved = super::resolution::resolve_module_path_with_context(
            module_path,
            context_path,
            self.stdlib_path,
            self.module_paths,
            self.dependency_paths,
        )?;

        let source = std::fs::read_to_string(&resolved).map_err(|e| ShapeError::ModuleError {
            message: format!("Failed to read module file: {}: {}", resolved.display(), e),
            module_path: Some(resolved.clone()),
        })?;

        Ok(Some(ResolvedModuleArtifact {
            module_path: module_path.to_string(),
            code: ModuleCode::Source(Arc::from(source)),
            origin_path: Some(resolved),
        }))
    }
}
