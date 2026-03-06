//! Unified Metadata API
//!
//! Combines metadata from all sources (Rust builtins + Shape stdlib)

use super::types::{FunctionInfo, PropertyInfo};
use crate::builtin_metadata::builtin_functions_from_macros;
use crate::stdlib_metadata::{StdlibMetadata, default_stdlib_path};

/// Unified metadata combining all sources (Rust builtins + Shape stdlib)
///
/// This provides a single API for the LSP to get all function/pattern metadata
/// regardless of whether they're implemented in Rust or Shape stdlib.
#[derive(Debug)]
pub struct UnifiedMetadata {
    /// Functions from #[shape_builtin] proc-macro
    rust_builtins: Vec<FunctionInfo>,
    /// Functions from Shape stdlib
    stdlib_functions: Vec<FunctionInfo>,
    /// Patterns from Shape stdlib
    stdlib_patterns: Vec<crate::stdlib_metadata::PatternInfo>,
    /// Type metadata from #[derive(ShapeType)]
    type_metadata: Vec<TypeMetadataInfo>,
}

/// Runtime type metadata info (converted from compile-time TypeMetadata)
#[derive(Debug, Clone)]
pub struct TypeMetadataInfo {
    pub name: String,
    pub description: String,
    pub properties: Vec<PropertyInfo>,
}

impl UnifiedMetadata {
    /// Load unified metadata from all sources
    pub fn load() -> Self {
        use crate::builtin_metadata::collect_type_metadata;

        // 1. Collect from proc-macro generated constants
        let mut rust_builtins = builtin_functions_from_macros();

        // 2. Parse stdlib files
        let stdlib_path = default_stdlib_path();
        let stdlib = StdlibMetadata::load(&stdlib_path).unwrap_or_else(|e| {
            eprintln!("Warning: Failed to load stdlib metadata: {}", e);
            StdlibMetadata::empty()
        });

        // If std/core provides declaration-only intrinsic function docs/signatures,
        // apply them as authoritative metadata for matching runtime builtins.
        if !stdlib.intrinsic_functions.is_empty() {
            let intrinsic_by_name: std::collections::HashMap<_, _> = stdlib
                .intrinsic_functions
                .iter()
                .map(|f| (f.name.clone(), f))
                .collect();
            for builtin in &mut rust_builtins {
                if let Some(intrinsic) = intrinsic_by_name.get(&builtin.name) {
                    builtin.signature = intrinsic.signature.clone();
                    builtin.description = intrinsic.description.clone();
                    builtin.parameters = intrinsic.parameters.clone();
                    builtin.return_type = intrinsic.return_type.clone();
                    builtin.comptime_only = intrinsic.comptime_only;
                }
            }
            for intrinsic in &stdlib.intrinsic_functions {
                if !rust_builtins.iter().any(|f| f.name == intrinsic.name) {
                    rust_builtins.push(intrinsic.clone());
                }
            }
        }

        // 3. Collect type metadata from derive macro
        let type_metadata = collect_type_metadata()
            .into_iter()
            .map(|tm| TypeMetadataInfo {
                name: tm.name.to_string(),
                description: tm.description.to_string(),
                properties: tm.to_property_infos(),
            })
            .collect();

        Self {
            rust_builtins,
            stdlib_functions: stdlib.functions,
            stdlib_patterns: stdlib.patterns,
            type_metadata,
        }
    }

    /// Get all functions from all sources (deduplicated)
    pub fn all_functions(&self) -> Vec<&FunctionInfo> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();

        // Priority: rust_builtins > stdlib
        for func in &self.rust_builtins {
            if seen.insert(&func.name) {
                result.push(func);
            }
        }
        for func in &self.stdlib_functions {
            if seen.insert(&func.name) {
                result.push(func);
            }
        }

        result
    }

    /// Get function by name
    pub fn get_function(&self, name: &str) -> Option<&FunctionInfo> {
        // Check rust builtins first
        if let Some(func) = self.rust_builtins.iter().find(|f| f.name == name) {
            return Some(func);
        }
        // Check stdlib
        self.stdlib_functions.iter().find(|f| f.name == name)
    }

    /// Get all patterns from stdlib
    pub fn all_patterns(&self) -> &[crate::stdlib_metadata::PatternInfo] {
        &self.stdlib_patterns
    }

    /// Get stdlib functions only
    pub fn stdlib_functions(&self) -> &[FunctionInfo] {
        &self.stdlib_functions
    }

    /// Get rust builtin functions only
    pub fn rust_builtins(&self) -> &[FunctionInfo] {
        &self.rust_builtins
    }

    /// Get type properties by type name (case-insensitive)
    pub fn get_type_properties(&self, type_name: &str) -> Option<&[PropertyInfo]> {
        self.type_metadata
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(type_name))
            .map(|t| t.properties.as_slice())
    }

    /// Get all type metadata
    pub fn all_types(&self) -> &[TypeMetadataInfo] {
        &self.type_metadata
    }
}

#[cfg(test)]
mod unified_metadata_tests {
    use super::*;

    #[test]
    fn test_unified_metadata_load() {
        let metadata = UnifiedMetadata::load();

        // Should have some functions from Rust builtins
        let all_funcs: Vec<_> = metadata.all_functions().into_iter().collect();
        assert!(!all_funcs.is_empty(), "Should have functions");

        // Check some core builtins exist
        assert!(
            metadata.get_function("abs").is_some(),
            "abs should be present"
        );
        assert!(
            metadata.get_function("sqrt").is_some(),
            "sqrt should be present"
        );

        // snapshot() is defined in stdlib/core/snapshot.shape
        assert!(
            metadata.get_function("snapshot").is_some(),
            "snapshot should be present from stdlib"
        );
    }

    #[test]
    fn test_metadata_coverage() {
        let metadata = UnifiedMetadata::load();

        println!("\n=== Metadata Coverage ===");
        println!(
            "Rust builtins (proc-macro): {}",
            metadata.rust_builtins().len()
        );
        for f in metadata.rust_builtins() {
            println!("  - {}", f.name);
        }

        println!("\nStdlib functions: {}", metadata.stdlib_functions().len());
        assert!(
            !metadata.stdlib_functions().is_empty(),
            "stdlib function metadata should not be empty"
        );

        let all_funcs = metadata.all_functions();
        println!("\nTotal functions available: {}", all_funcs.len());

        // Core builtins that should be present from Rust
        assert!(
            metadata.get_function("abs").is_some(),
            "abs should be present from builtins"
        );
        assert!(
            metadata.get_function("sqrt").is_some(),
            "sqrt should be present from builtins"
        );
        assert!(
            metadata.get_function("snapshot").is_some(),
            "snapshot should be present from stdlib"
        );
    }

    #[test]
    fn test_intrinsic_std_core_overrides_builtin_docs() {
        let metadata = UnifiedMetadata::load();
        let abs = metadata
            .get_function("abs")
            .expect("abs should be available in unified metadata");
        assert_eq!(abs.signature, "abs(value: number) -> number");
        assert!(
            abs.description.contains("absolute value"),
            "abs docs should be sourced from std::core intrinsic declarations"
        );
    }

    #[test]
    fn test_intrinsic_std_core_uses_table_signatures() {
        let metadata = UnifiedMetadata::load();
        let resample = metadata
            .get_function("resample")
            .expect("resample should be available in unified metadata");
        assert!(
            resample.signature.contains("Table<"),
            "resample signature should use Table<T>, got: {}",
            resample.signature
        );
    }
}
