//! Standard library compilation for Shape VM
//!
//! This module handles compiling the core stdlib modules at engine initialization.
//! Core modules are auto-imported and available without explicit imports.
//! Domain-specific modules (finance, iot, etc.) require explicit imports.

use std::path::Path;
use std::sync::OnceLock;

use shape_ast::error::{Result, ShapeError};
use shape_runtime::module_loader::ModuleLoader;

use crate::bytecode::BytecodeProgram;
use crate::compiler::BytecodeCompiler;

static CORE_STDLIB_CACHE: OnceLock<Result<BytecodeProgram>> = OnceLock::new();

fn stdlib_compile_logs_enabled() -> bool {
    std::env::var("SHAPE_TRACE_STDLIB_COMPILE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

/// Pre-compiled core stdlib bytecode (MessagePack-serialized BytecodeProgram).
/// Regenerate with: cargo run -p stdlib-gen
#[cfg(not(test))]
const EMBEDDED_CORE_STDLIB: Option<&[u8]> = Some(include_bytes!("../embedded/core_stdlib.msgpack"));

// Tests always recompile from source to validate compiler changes
#[cfg(test)]
const EMBEDDED_CORE_STDLIB: Option<&[u8]> = None;

/// Compile all core stdlib modules into a single BytecodeProgram
///
/// The core modules are those in `stdlib/core/` which are auto-imported
/// and available without explicit import statements.
///
/// Uses precompiled embedded bytecode when available, falling back to
/// source compilation. Set `SHAPE_FORCE_SOURCE_STDLIB=1` to force source.
///
/// # Returns
///
/// A merged BytecodeProgram containing all core functions, types, and metas.
pub fn compile_core_modules() -> Result<BytecodeProgram> {
    CORE_STDLIB_CACHE
        .get_or_init(load_core_modules_best_effort)
        .clone()
}

fn load_core_modules_best_effort() -> Result<BytecodeProgram> {
    // Env override: force source compilation (for debugging/development)
    if std::env::var("SHAPE_FORCE_SOURCE_STDLIB").is_ok() {
        return compile_core_modules_from_source();
    }

    // Try embedded precompiled artifact first
    if let Some(bytes) = EMBEDDED_CORE_STDLIB {
        match load_from_embedded(bytes) {
            Ok(program) => return Ok(program),
            Err(e) => {
                if stdlib_compile_logs_enabled() {
                    eprintln!(
                        "  Embedded stdlib deserialization failed: {}, falling back to source",
                        e
                    );
                }
            }
        }
    }

    // Fallback: compile from source
    compile_core_modules_from_source()
}

fn load_from_embedded(bytes: &[u8]) -> Result<BytecodeProgram> {
    let mut program: BytecodeProgram =
        rmp_serde::from_slice(bytes).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to deserialize embedded stdlib: {}", e),
            location: None,
        })?;
    program.ensure_string_index();
    Ok(program)
}

/// Extract top-level binding names from precompiled core bytecode.
/// Used to seed the compiler with known names without loading AST into persistent context.
pub fn core_binding_names() -> Vec<String> {
    match compile_core_modules() {
        Ok(program) => {
            let mut names: Vec<String> =
                program.functions.iter().map(|f| f.name.clone()).collect();
            for name in &program.module_binding_names {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            names
        }
        Err(_) => Vec::new(),
    }
}

/// Compile core stdlib from source (parse + compile). Used as fallback and for tests.
///
/// Each module is compiled independently (preserving its own scope for builtins
/// and intrinsics), then the bytecodes are merged via `merge_append`.
pub fn compile_core_modules_from_source() -> Result<BytecodeProgram> {
    let trace = stdlib_compile_logs_enabled();
    if trace {
        eprintln!("  Compiling core stdlib...");
    }
    let mut loader = ModuleLoader::new();
    let core_modules = loader.list_core_stdlib_module_imports()?;
    if core_modules.is_empty() {
        return Ok(BytecodeProgram::new());
    }

    let mut merged = BytecodeProgram::new();
    for import_path in core_modules {
        let file_name = import_path.strip_prefix("std.").unwrap_or(&import_path);
        match loader.load_module(&import_path).and_then(|module| {
            BytecodeCompiler::compile_module_ast(&module.ast).map(|(program, _)| program)
        }) {
            Ok(module_program) => {
                if trace {
                    eprintln!("    Compiled {}", file_name);
                }
                merged.merge_append(module_program);
            }
            Err(e) => {
                if trace {
                    eprintln!("    Warning: failed to compile {}: {}", file_name, e);
                }
            }
        }
    }

    if trace {
        eprintln!("  Finished core stdlib compilation");
    }
    Ok(merged)
}

/// Compile all Shape files in a directory (recursively) into a single BytecodeProgram.
/// Each file is compiled independently, then merged via `merge_append`.
pub fn compile_directory(dir: &Path) -> Result<BytecodeProgram> {
    let mut merged = BytecodeProgram::new();
    compile_directory_into(&mut merged, dir)?;
    Ok(merged)
}

/// Recursively compile all Shape files in a directory and merge into the given program.
fn compile_directory_into(program: &mut BytecodeProgram, dir: &Path) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| ShapeError::ModuleError {
        message: format!("Failed to read directory {:?}: {}", dir, e),
        module_path: Some(dir.to_path_buf()),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| ShapeError::ModuleError {
            message: format!("Failed to read directory entry: {}", e),
            module_path: Some(dir.to_path_buf()),
        })?;

        let path = entry.path();

        if path.is_dir() {
            compile_directory_into(program, &path)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("shape") {
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            match compile_file(&path) {
                Ok(file_program) => {
                    eprintln!("    Compiled {}", file_name);
                    program.merge_append(file_program);
                }
                Err(e) => {
                    eprintln!("    Warning: failed to compile {}: {}", file_name, e);
                }
            }
        }
    }

    Ok(())
}

/// Compile an in-memory Shape source string into a BytecodeProgram.
/// Used for extension-bundled Shape code (e.g., `include_str!("duckdb.shape")`).
pub fn compile_source(filename: &str, source: &str) -> Result<BytecodeProgram> {
    let program = shape_ast::parser::parse_program(source).map_err(|e| ShapeError::ParseError {
        message: format!("Failed to parse {}: {}", filename, e),
        location: None,
    })?;

    let mut compiler = BytecodeCompiler::new();
    compiler.set_source_with_file(source, filename);
    compiler.compile(&program)
}

/// Compile a single Shape file into a BytecodeProgram
pub fn compile_file(path: &Path) -> Result<BytecodeProgram> {
    let source = std::fs::read_to_string(path).map_err(|e| ShapeError::ModuleError {
        message: format!("Failed to read file {:?}: {}", path, e),
        module_path: Some(path.to_path_buf()),
    })?;

    let program =
        shape_ast::parser::parse_program(&source).map_err(|e| ShapeError::ParseError {
            message: format!("Failed to parse {:?}: {}", path, e),
            location: None,
        })?;

    let mut compiler = BytecodeCompiler::new();
    compiler.set_source_with_file(&source, &path.to_string_lossy());
    compiler.compile(&program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_bytecode_has_snapshot_schema() {
        let core = compile_core_modules().expect("Core modules should compile");
        let snapshot = core.type_schema_registry.get("Snapshot");
        assert!(
            snapshot.is_some(),
            "Core bytecode should contain Snapshot enum schema"
        );
        let enum_info = snapshot.unwrap().get_enum_info();
        assert!(enum_info.is_some(), "Snapshot should be an enum");
        let info = enum_info.unwrap();
        assert!(
            info.variant_by_name("Hash").is_some(),
            "Snapshot should have Hash variant"
        );
        assert!(
            info.variant_by_name("Resumed").is_some(),
            "Snapshot should have Resumed variant"
        );
    }

    #[test]
    fn test_core_bytecode_registers_queryable_trait_dispatch_symbols() {
        let core = compile_core_modules().expect("Core modules should compile");
        let filter = core.lookup_trait_method_symbol("Queryable", "Table", None, "filter");
        let map = core.lookup_trait_method_symbol("Queryable", "Table", None, "map");
        let execute = core.lookup_trait_method_symbol("Queryable", "Table", None, "execute");

        assert_eq!(filter, Some("Table::filter"));
        assert_eq!(map, Some("Table::map"));
        assert_eq!(execute, Some("Table::execute"));
    }

    #[test]
    fn test_compile_empty_directory() {
        // Create a temp directory and compile it
        let temp_dir = std::env::temp_dir().join("shape_test_empty");
        let _ = std::fs::create_dir_all(&temp_dir);

        let result = compile_directory(&temp_dir);
        assert!(result.is_ok());

        let program = result.unwrap();
        // Should have a Halt instruction at minimum
        assert!(
            program.instructions.is_empty()
                || program.instructions.last().map(|i| i.opcode)
                    == Some(crate::bytecode::OpCode::Halt)
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_compile_source_simple_function() {
        let source = r#"
            fn double(x) { x * 2 }
        "#;
        let result = compile_source("test.shape", source);
        assert!(
            result.is_ok(),
            "compile_source should succeed: {:?}",
            result.err()
        );

        let program = result.unwrap();
        assert!(
            !program.functions.is_empty(),
            "Should have at least one function"
        );
        assert!(
            program.functions.iter().any(|f| f.name == "double"),
            "Should contain 'double' function"
        );
    }

    #[test]
    fn test_compile_source_parse_error() {
        let source = "fn broken(( { }";
        let result = compile_source("broken.shape", source);
        assert!(result.is_err(), "Should fail on invalid syntax");
    }

    #[test]
    fn test_compile_source_enum_definition() {
        let source = r#"
            enum Direction {
                Up,
                Down,
                Left,
                Right
            }
        "#;
        let result = compile_source("enums.shape", source);
        assert!(
            result.is_ok(),
            "compile_source should handle enums: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_embedded_stdlib_round_trip() {
        // Compile from source, serialize, deserialize, and verify key properties match
        let source = compile_core_modules_from_source().expect("Source compilation should succeed");
        let bytes = rmp_serde::to_vec(&source).expect("Serialization should succeed");
        let deserialized = load_from_embedded(&bytes).expect("Deserialization should succeed");

        assert_eq!(
            source.functions.len(),
            deserialized.functions.len(),
            "Function count should match after round-trip"
        );
        assert_eq!(
            source.instructions.len(),
            deserialized.instructions.len(),
            "Instruction count should match after round-trip"
        );
        assert_eq!(
            source.constants.len(),
            deserialized.constants.len(),
            "Constant count should match after round-trip"
        );
        assert!(
            !deserialized.functions.is_empty(),
            "Deserialized should have functions"
        );
    }

    #[test]
    fn test_body_length_within_bounds() {
        let program = compile_core_modules_from_source().expect("compile");
        let total = program.instructions.len();
        let mut bad = Vec::new();
        for (i, f) in program.functions.iter().enumerate() {
            let end = f.entry_point + f.body_length;
            if end > total {
                bad.push(format!(
                    "func[{}] '{}' entry={} body_length={} end={} exceeds total={}",
                    i, f.name, f.entry_point, f.body_length, end, total
                ));
            }
        }
        assert!(bad.is_empty(), "Functions with OOB body_length:\n{}", bad.join("\n"));
    }

    #[test]
    fn test_core_binding_names() {
        let names = core_binding_names();
        assert!(!names.is_empty(), "Should have binding names from stdlib");
    }
}
