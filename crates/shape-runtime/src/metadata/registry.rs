//! Metadata registry stub
//!
//! Meta definitions have been removed (replaced by Display trait + comptime fields).
//! This stub preserves the MetadataRegistry type used by context infrastructure.

use std::sync::{Arc, RwLock};

/// Registry for type metadata (legacy — being replaced by Display trait)
#[derive(Debug, Clone)]
pub struct MetadataRegistry {
    _placeholder: Arc<RwLock<()>>,
}

impl MetadataRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            _placeholder: Arc::new(RwLock::new(())),
        }
    }
}

impl Default for MetadataRegistry {
    fn default() -> Self {
        Self::new()
    }
}
