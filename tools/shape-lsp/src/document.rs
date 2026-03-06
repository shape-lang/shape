//! Document management for LSP
//!
//! Tracks open documents, their content, and version numbers for incremental updates.

use crate::module_cache::ModuleCache;
use crate::symbols::SymbolInfo;
use dashmap::DashMap;
use ropey::Rope;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp_server::ls_types::Uri;

/// A document in the workspace with its content and metadata
#[derive(Debug, Clone)]
pub struct Document {
    /// The URI of the document
    pub uri: Uri,
    /// The version number (from LSP protocol)
    pub version: i32,
    /// The text content as a Rope for efficient editing
    pub rope: Rope,
    /// Cached symbols from last successful parse (for completion fallback)
    pub cached_symbols: Vec<SymbolInfo>,
    /// Cached type info from last successful inference (for completion fallback)
    pub cached_types: HashMap<String, String>,
}

impl Document {
    /// Create a new document
    pub fn new(uri: Uri, version: i32, text: String) -> Self {
        Self {
            uri,
            version,
            rope: Rope::from_str(&text),
            cached_symbols: Vec::new(),
            cached_types: HashMap::new(),
        }
    }

    /// Update cached symbols from successful parse
    pub fn update_cached_symbols(&mut self, symbols: Vec<SymbolInfo>) {
        self.cached_symbols = symbols;
    }

    /// Update cached type info from successful inference
    pub fn update_cached_types(&mut self, types: HashMap<String, String>) {
        self.cached_types = types;
    }

    /// Get cached symbols
    pub fn get_cached_symbols(&self) -> &[SymbolInfo] {
        &self.cached_symbols
    }

    /// Get cached type info
    pub fn get_cached_types(&self) -> &HashMap<String, String> {
        &self.cached_types
    }

    /// Get the full text content
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Get the number of lines
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Get a specific line (0-indexed)
    pub fn line(&self, line_idx: usize) -> Option<String> {
        if line_idx >= self.line_count() {
            return None;
        }

        let start = self.rope.line_to_char(line_idx);
        let end = if line_idx + 1 < self.line_count() {
            self.rope.line_to_char(line_idx + 1)
        } else {
            self.rope.len_chars()
        };

        Some(self.rope.slice(start..end).to_string())
    }

    /// Convert LSP Position to byte offset
    pub fn position_to_offset(&self, line: u32, character: u32) -> Option<usize> {
        let line_idx = line as usize;
        if line_idx >= self.line_count() {
            return None;
        }

        let line_start = self.rope.line_to_char(line_idx);
        let offset = line_start + character as usize;

        if offset > self.rope.len_chars() {
            return None;
        }

        Some(offset)
    }

    /// Convert byte offset to LSP Position
    pub fn offset_to_position(&self, offset: usize) -> Option<(u32, u32)> {
        if offset > self.rope.len_chars() {
            return None;
        }

        let line = self.rope.char_to_line(offset);
        let line_start = self.rope.line_to_char(line);
        let column = offset - line_start;

        Some((line as u32, column as u32))
    }
}

/// Manages all open documents in the workspace
#[derive(Debug)]
pub struct DocumentManager {
    /// Map of URI to Document
    documents: DashMap<Uri, Document>,
    /// Module cache for cross-file navigation
    module_cache: Arc<ModuleCache>,
}

impl Default for DocumentManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentManager {
    /// Create a new document manager
    pub fn new() -> Self {
        Self {
            documents: DashMap::new(),
            module_cache: Arc::new(ModuleCache::new()),
        }
    }

    /// Get the module cache
    pub fn get_module_cache(&self) -> Arc<ModuleCache> {
        self.module_cache.clone()
    }

    /// Open a new document
    pub fn open(&self, uri: Uri, version: i32, text: String) {
        let doc = Document::new(uri.clone(), version, text);
        self.documents.insert(uri, doc);
    }

    /// Close a document
    pub fn close(&self, uri: &Uri) {
        // Invalidate module cache for this file
        let path = PathBuf::from(uri.path().as_str());
        self.module_cache.invalidate(&path);

        self.documents.remove(uri);
    }

    /// Update document content (full update)
    pub fn update(&self, uri: &Uri, version: i32, text: String) {
        // Invalidate module cache for this file since it changed
        let path = PathBuf::from(uri.path().as_str());
        self.module_cache.invalidate(&path);

        if let Some(mut doc) = self.documents.get_mut(uri) {
            doc.version = version;
            doc.rope = Rope::from_str(&text);
        }
    }

    /// Get a document by URI
    pub fn get(&self, uri: &Uri) -> Option<Document> {
        self.documents.get(uri).map(|doc| doc.clone())
    }

    /// Check if a document is open
    pub fn contains(&self, uri: &Uri) -> bool {
        self.documents.contains_key(uri)
    }

    /// Get all document URIs
    pub fn all_uris(&self) -> Vec<Uri> {
        self.documents
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Update cached symbols for a document
    pub fn update_cached_symbols(&self, uri: &Uri, symbols: Vec<SymbolInfo>) {
        if let Some(mut doc) = self.documents.get_mut(uri) {
            doc.update_cached_symbols(symbols);
        }
    }

    /// Update cached type info for a document
    pub fn update_cached_types(&self, uri: &Uri, types: HashMap<String, String>) {
        if let Some(mut doc) = self.documents.get_mut(uri) {
            doc.update_cached_types(types);
        }
    }

    /// Get cached symbols for a document
    pub fn get_cached_symbols(&self, uri: &Uri) -> Vec<SymbolInfo> {
        self.documents
            .get(uri)
            .map(|doc| doc.get_cached_symbols().to_vec())
            .unwrap_or_default()
    }

    /// Get cached type info for a document
    pub fn get_cached_types(&self, uri: &Uri) -> HashMap<String, String> {
        self.documents
            .get(uri)
            .map(|doc| doc.get_cached_types().clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_creation() {
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let doc = Document::new(uri.clone(), 1, "let x = 5;\nlet y = 10;".to_string());

        assert_eq!(doc.version, 1);
        assert_eq!(doc.line_count(), 2);
        assert_eq!(doc.text(), "let x = 5;\nlet y = 10;");
    }

    #[test]
    fn test_position_conversion() {
        let uri = Uri::from_file_path("/test.shape").unwrap();
        let doc = Document::new(uri, 1, "let x = 5;\nlet y = 10;".to_string());

        // Test position to offset
        let offset = doc.position_to_offset(0, 4).unwrap();
        assert_eq!(doc.text().chars().nth(offset), Some('x'));

        // Test offset to position
        let (line, col) = doc.offset_to_position(4).unwrap();
        assert_eq!(line, 0);
        assert_eq!(col, 4);
    }

    #[test]
    fn test_document_manager() {
        let manager = DocumentManager::new();
        let uri = Uri::from_file_path("/test.shape").unwrap();

        // Open document
        manager.open(uri.clone(), 1, "let x = 5;".to_string());
        assert!(manager.contains(&uri));

        // Get document
        let doc = manager.get(&uri).unwrap();
        assert_eq!(doc.version, 1);
        assert_eq!(doc.text(), "let x = 5;");

        // Update document
        manager.update(&uri, 2, "let x = 10;".to_string());
        let doc = manager.get(&uri).unwrap();
        assert_eq!(doc.version, 2);
        assert_eq!(doc.text(), "let x = 10;");

        // Close document
        manager.close(&uri);
        assert!(!manager.contains(&uri));
    }
}
