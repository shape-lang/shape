//! Stdlib metadata extractor for LSP introspection
//!
//! This module parses Shape stdlib files to extract function and pattern
//! metadata for use in LSP completions and hover information.

use crate::metadata::{FunctionCategory, FunctionInfo, ParameterInfo, TypeInfo};
use shape_ast::ast::{
    BuiltinFunctionDecl, BuiltinTypeDecl, DocComment, FunctionDef, Item, Program, TypeAnnotation,
};
use shape_ast::error::Result;
#[cfg(test)]
use shape_ast::parser::parse_program;
use std::path::{Path, PathBuf};

/// Metadata extracted from Shape stdlib
#[derive(Debug, Default)]
pub struct StdlibMetadata {
    /// Exported functions from stdlib
    pub functions: Vec<FunctionInfo>,
    /// Exported patterns from stdlib
    pub patterns: Vec<PatternInfo>,
    /// Declaration-only intrinsic functions from std/core
    pub intrinsic_functions: Vec<FunctionInfo>,
    /// Declaration-only intrinsic types from std/core
    pub intrinsic_types: Vec<TypeInfo>,
}

/// Pattern metadata for LSP
#[derive(Debug, Clone)]
pub struct PatternInfo {
    /// Pattern name
    pub name: String,
    /// Pattern signature
    pub signature: String,
    /// Description (if available from comments)
    pub description: String,
    /// Parameters (pattern variables)
    pub parameters: Vec<ParameterInfo>,
}

impl StdlibMetadata {
    /// Create empty stdlib metadata
    pub fn empty() -> Self {
        Self::default()
    }

    /// Load and parse all stdlib modules from the given path
    pub fn load(stdlib_path: &Path) -> Result<Self> {
        let mut functions = Vec::new();
        let mut patterns = Vec::new();
        let mut intrinsic_functions = Vec::new();
        let mut intrinsic_types = Vec::new();

        if !stdlib_path.exists() {
            return Ok(Self::empty());
        }

        // Use the unified module loader for stdlib discovery + parsing.
        let mut loader = crate::module_loader::ModuleLoader::new();
        loader.set_stdlib_path(stdlib_path.to_path_buf());

        for import_path in loader.list_stdlib_module_imports()? {
            let module_path = import_path
                .strip_prefix("std::")
                .unwrap_or(&import_path)
                .replace("::", "/");
            match loader.load_module(&import_path) {
                Ok(module) => {
                    Self::extract_from_program(
                        &module.ast,
                        &module_path,
                        &mut functions,
                        &mut patterns,
                        &mut intrinsic_functions,
                        &mut intrinsic_types,
                    );
                }
                Err(_) => {
                    // Skip modules that fail to parse/compile metadata.
                }
            }
        }

        Ok(Self {
            functions,
            patterns,
            intrinsic_functions,
            intrinsic_types,
        })
    }

    fn extract_from_program(
        program: &Program,
        module_path: &str,
        functions: &mut Vec<FunctionInfo>,
        _patterns: &mut Vec<PatternInfo>,
        intrinsic_functions: &mut Vec<FunctionInfo>,
        intrinsic_types: &mut Vec<TypeInfo>,
    ) {
        for item in &program.items {
            match item {
                Item::Function(func, span) => {
                    // All top-level functions are considered exports
                    functions.push(Self::function_to_info(
                        func,
                        module_path,
                        program.docs.comment_for_span(*span),
                    ));
                }
                Item::Export(export, span) => {
                    // Handle explicit exports
                    match &export.item {
                        shape_ast::ast::ExportItem::Function(func) => {
                            functions.push(Self::function_to_info(
                                func,
                                module_path,
                                program.docs.comment_for_span(*span),
                            ));
                        }
                        shape_ast::ast::ExportItem::TypeAlias(_) => {}
                        shape_ast::ast::ExportItem::Named(_) => {}
                        shape_ast::ast::ExportItem::Enum(_) => {}
                        shape_ast::ast::ExportItem::Struct(_) => {}
                        shape_ast::ast::ExportItem::Interface(_) => {}
                        shape_ast::ast::ExportItem::Trait(_) => {}
                        shape_ast::ast::ExportItem::ForeignFunction(_) => {
                            // Foreign functions are not stdlib intrinsics
                        }
                    }
                }
                Item::BuiltinTypeDecl(type_decl, span) => {
                    intrinsic_types
                        .push(Self::builtin_type_to_info(type_decl, program.docs.comment_for_span(*span)));
                }
                Item::BuiltinFunctionDecl(func_decl, span) => {
                    intrinsic_functions.push(Self::builtin_function_to_info(
                        func_decl,
                        module_path,
                        program.docs.comment_for_span(*span),
                    ));
                }
                _ => {}
            }
        }
    }

    /// Infer function category from module path (domain-agnostic)
    ///
    /// Uses directory structure to determine category:
    /// - core/math, core/statistics → Math
    /// - */indicators/*, */backtesting/*, */simulation/* → Simulation
    /// - */patterns/* → Utility
    /// - Default → Utility
    fn infer_category_from_path(module_path: &str) -> FunctionCategory {
        let path_lower = module_path.to_lowercase().replace("::", "/");

        // Check path components for categorization
        if path_lower.contains("/math") || path_lower.contains("/statistics") {
            FunctionCategory::Math
        } else if path_lower.contains("/indicators")
            || path_lower.contains("/backtesting")
            || path_lower.contains("/simulation")
        {
            FunctionCategory::Simulation
        } else if path_lower.contains("/patterns") {
            FunctionCategory::Utility
        } else {
            FunctionCategory::Utility
        }
    }

    fn function_to_info(
        func: &FunctionDef,
        module_path: &str,
        doc: Option<&DocComment>,
    ) -> FunctionInfo {
        let params: Vec<ParameterInfo> = func
            .params
            .iter()
            .map(|p| ParameterInfo {
                name: p.simple_name().unwrap_or("_").to_string(),
                param_type: p
                    .type_annotation
                    .as_ref()
                    .map(Self::format_type_annotation)
                    .unwrap_or_else(|| "any".to_string()),
                optional: p.default_value.is_some(),
                description: doc
                    .and_then(|comment| comment.param_doc(p.simple_name().unwrap_or("_")))
                    .unwrap_or_default()
                    .to_string(),
                constraints: None,
            })
            .collect();

        let return_type = func
            .return_type
            .as_ref()
            .map(Self::format_type_annotation)
            .unwrap_or_else(|| "any".to_string());

        let param_strs: Vec<String> = params
            .iter()
            .map(|p| {
                if p.optional {
                    format!("{}?: {}", p.name, p.param_type)
                } else {
                    format!("{}: {}", p.name, p.param_type)
                }
            })
            .collect();

        let signature = format!(
            "{}({}) -> {}",
            func.name,
            param_strs.join(", "),
            return_type
        );

        // Determine category based on path structure (domain-agnostic)
        let category = Self::infer_category_from_path(module_path);

        FunctionInfo {
            name: func.name.clone(),
            signature,
            description: doc.map(Self::doc_text).unwrap_or_default(),
            category,
            parameters: params,
            return_type,
            example: doc.and_then(|comment| comment.example_doc()).map(str::to_string),
            implemented: true,
            comptime_only: false,
        }
    }

    fn builtin_type_to_info(type_decl: &BuiltinTypeDecl, doc: Option<&DocComment>) -> TypeInfo {
        TypeInfo {
            name: type_decl.name.clone(),
            description: doc.map(Self::doc_text).unwrap_or_default(),
        }
    }

    fn builtin_function_to_info(
        func: &BuiltinFunctionDecl,
        module_path: &str,
        doc: Option<&DocComment>,
    ) -> FunctionInfo {
        let params: Vec<ParameterInfo> = func
            .params
            .iter()
            .map(|p| ParameterInfo {
                name: p.simple_name().unwrap_or("_").to_string(),
                param_type: p
                    .type_annotation
                    .as_ref()
                    .map(Self::format_type_annotation)
                    .unwrap_or_else(|| "any".to_string()),
                optional: p.default_value.is_some(),
                description: doc
                    .and_then(|comment| comment.param_doc(p.simple_name().unwrap_or("_")))
                    .unwrap_or_default()
                    .to_string(),
                constraints: None,
            })
            .collect();
        let return_type = Self::format_type_annotation(&func.return_type);
        let type_params_str = func
            .type_params
            .as_ref()
            .filter(|params| !params.is_empty())
            .map(|params| {
                format!(
                    "<{}>",
                    params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .unwrap_or_default();
        let signature = format!(
            "{}{}({}) -> {}",
            func.name,
            type_params_str,
            params
                .iter()
                .map(|p| format!("{}: {}", p.name, p.param_type))
                .collect::<Vec<_>>()
                .join(", "),
            return_type
        );
        FunctionInfo {
            name: func.name.clone(),
            signature,
            description: doc.map(Self::doc_text).unwrap_or_default(),
            category: Self::infer_category_from_path(module_path),
            parameters: params,
            return_type,
            example: doc.and_then(|comment| comment.example_doc()).map(str::to_string),
            implemented: true,
            comptime_only: crate::builtin_metadata::is_comptime_builtin_function(&func.name),
        }
    }

    fn doc_text(comment: &DocComment) -> String {
        if !comment.body.is_empty() {
            comment.body.clone()
        } else {
            comment.summary.clone()
        }
    }

    fn format_type_annotation(ty: &TypeAnnotation) -> String {
        match ty {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => name.clone(),
            TypeAnnotation::Array(inner) => format!("{}[]", Self::format_type_annotation(inner)),
            TypeAnnotation::Tuple(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(Self::format_type_annotation)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeAnnotation::Object(fields) => {
                let inner = fields
                    .iter()
                    .map(|f| {
                        if f.optional {
                            format!(
                                "{}?: {}",
                                f.name,
                                Self::format_type_annotation(&f.type_annotation)
                            )
                        } else {
                            format!(
                                "{}: {}",
                                f.name,
                                Self::format_type_annotation(&f.type_annotation)
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ {} }}", inner)
            }
            TypeAnnotation::Function { params, returns } => {
                let param_list = params
                    .iter()
                    .map(|p| {
                        let ty = Self::format_type_annotation(&p.type_annotation);
                        if let Some(name) = &p.name {
                            if p.optional {
                                format!("{}?: {}", name, ty)
                            } else {
                                format!("{}: {}", name, ty)
                            }
                        } else {
                            ty
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "({}) -> {}",
                    param_list,
                    Self::format_type_annotation(returns)
                )
            }
            TypeAnnotation::Union(types) => types
                .iter()
                .map(Self::format_type_annotation)
                .collect::<Vec<_>>()
                .join(" | "),
            TypeAnnotation::Intersection(types) => types
                .iter()
                .map(Self::format_type_annotation)
                .collect::<Vec<_>>()
                .join(" + "),
            TypeAnnotation::Generic { name, args } => format!(
                "{}<{}>",
                name,
                args.iter()
                    .map(Self::format_type_annotation)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeAnnotation::Void => "void".to_string(),
            TypeAnnotation::Never => "never".to_string(),
            TypeAnnotation::Null => "null".to_string(),
            TypeAnnotation::Undefined => "undefined".to_string(),
            TypeAnnotation::Dyn(bounds) => format!("dyn {}", bounds.join(" + ")),
        }
    }
}

/// Get the default stdlib path
pub fn default_stdlib_path() -> PathBuf {
    // Explicit override for non-workspace environments (packaged installs, custom dev layouts).
    if let Ok(path) = std::env::var("SHAPE_STDLIB_PATH") {
        return PathBuf::from(path);
    }

    // Workspace builds use the canonical stdlib source of truth.
    let workspace_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../shape-core/stdlib");
    if workspace_path.is_dir() {
        return workspace_path;
    }

    // Published crates carry a vendored stdlib copy inside shape-runtime itself.
    let packaged_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("stdlib-src");
    if packaged_path.is_dir() {
        return packaged_path;
    }

    packaged_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn collect_shape_files(root: &Path) -> BTreeMap<PathBuf, String> {
        let mut files = BTreeMap::new();
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("shape") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .expect("vendored stdlib file should be under root")
                .to_path_buf();
            let content = std::fs::read_to_string(path)
                .unwrap_or_else(|err| panic!("failed to read {}: {}", path.display(), err));
            files.insert(rel, content);
        }
        files
    }

    #[test]
    fn test_load_stdlib() {
        let stdlib_path = default_stdlib_path();
        if stdlib_path.exists() {
            let metadata = StdlibMetadata::load(&stdlib_path).unwrap();
            // Stdlib may or may not have functions depending on development state
            println!("Stdlib path: {:?}", stdlib_path);
            println!("Found {} stdlib functions", metadata.functions.len());
            for func in &metadata.functions {
                println!("  - {}: {}", func.name, func.signature);
            }
            println!("Found {} stdlib patterns", metadata.patterns.len());
            // Note: stdlib functions (like sma, ema) will be added as stdlib is developed
        } else {
            println!("Stdlib path does not exist: {:?}", stdlib_path);
        }
    }

    #[test]
    fn test_empty_stdlib() {
        let metadata = StdlibMetadata::empty();
        assert!(metadata.functions.is_empty());
        assert!(metadata.patterns.is_empty());
    }

    #[test]
    fn test_parse_all_stdlib_files() {
        let stdlib_path = default_stdlib_path();
        println!("Stdlib path: {:?}", stdlib_path);

        // Test each file individually
        let files = [
            "core/snapshot.shape",
            "core/math.shape",
            "finance/indicators/moving_averages.shape",
        ];

        for file in &files {
            let path = stdlib_path.join(file);
            if path.exists() {
                let content = std::fs::read_to_string(&path).unwrap();
                match parse_program(&content) {
                    Ok(program) => {
                        let func_count = program
                            .items
                            .iter()
                            .filter(|i| {
                                matches!(
                                    i,
                                    shape_ast::ast::Item::Function(_, _)
                                        | shape_ast::ast::Item::Export(_, _)
                                )
                            })
                            .count();
                        println!("✓ {} parsed: {} items", file, func_count);
                    }
                    Err(e) => {
                        panic!("✗ {} FAILED to parse: {:?}", file, e);
                    }
                }
            } else {
                println!("⚠ {} not found", file);
            }
        }
    }

    #[test]
    fn test_intrinsic_declarations_loaded_from_std_core() {
        let stdlib_path = default_stdlib_path();
        if !stdlib_path.exists() {
            return;
        }

        let metadata = StdlibMetadata::load(&stdlib_path).unwrap();
        assert!(
            metadata
                .intrinsic_types
                .iter()
                .any(|t| t.name == "AnyError"),
            "expected AnyError intrinsic type from std::core declarations"
        );
        let abs = metadata
            .intrinsic_functions
            .iter()
            .find(|f| f.name == "abs")
            .expect("abs intrinsic declaration should exist");
        assert_eq!(abs.signature, "abs(value: number) -> number");
        assert!(
            abs.description.contains("absolute value"),
            "abs description should come from doc comments"
        );
    }

    #[test]
    fn test_vendored_stdlib_matches_workspace_copy() {
        let workspace_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../shape-core/stdlib");
        let packaged_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("stdlib-src");

        if !workspace_path.is_dir() || !packaged_path.is_dir() {
            return;
        }

        let workspace_files = collect_shape_files(&workspace_path);
        let packaged_files = collect_shape_files(&packaged_path);
        assert_eq!(
            packaged_files, workspace_files,
            "shape-runtime/stdlib-src is out of sync with crates/shape-core/stdlib"
        );
    }
}
