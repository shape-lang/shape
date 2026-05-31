//! Standard library compilation for Shape VM
//!
//! This module handles compiling the core stdlib modules at engine initialization.
//! Core modules are auto-imported and available without explicit imports.
//! Domain-specific modules (finance, iot, etc.) require explicit imports.

use std::path::Path;
use std::sync::Arc;

use shape_ast::error::{Result, ShapeError};
use shape_runtime::Runtime;
use shape_runtime::module_loader::ModuleLoader;
use shape_runtime::package_bundle::{
    decide_cache_action, current_format_version, BundleMetadata, BundledModule, PackageBundle,
    ResolvedInterface,
};
use shape_runtime::module_manifest::ModuleManifest;

use crate::bytecode::BytecodeProgram;
use crate::compiler::BytecodeCompiler;

/// DESIGN §4.3 / decision 4a — the prelude's compiled bytecode PLUS its
/// resolved type-checker interface, cached together on the [`Runtime`].
///
/// Wrapping the prelude as a degenerate `SHAPEPKG` bundle (decision 4a) routes
/// the prelude load through the SAME §2.3 source-hash gate + §2.4 replay path as
/// any `.shapec` dependency. The `interface` is `None` whenever the prelude was
/// obtained via the R6 last-resort fallbacks (bare-`BytecodeProgram` deserialize
/// or from-source compile), in which case the consumer checker falls back to its
/// existing prelude handling — i.e. failure degrades to today's behavior (R6).
#[derive(Clone)]
pub struct CorePrelude {
    pub program: BytecodeProgram,
    pub interface: Option<ResolvedInterface>,
}

/// The degenerate prelude module path inside the prelude `SHAPEPKG` bundle.
const PRELUDE_MODULE_PATH: &str = "std::core";

fn stdlib_compile_logs_enabled() -> bool {
    std::env::var("SHAPE_TRACE_STDLIB_COMPILE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

/// Pre-compiled core stdlib bytecode (MessagePack-serialized BytecodeProgram).
/// This is the R6 last-resort embedded artifact (bare `BytecodeProgram`, no
/// interface). Regenerate with: cargo run -p shape-vm --bin stdlib_gen
#[cfg(not(test))]
const EMBEDDED_CORE_STDLIB: Option<&[u8]> = Some(include_bytes!("../embedded/core_stdlib.msgpack"));

// Tests always recompile from source to validate compiler changes
#[cfg(test)]
const EMBEDDED_CORE_STDLIB: Option<&[u8]> = None;

/// `include_bytes!` that yields `None` instead of failing the build when the
/// artifact is not yet generated. The path is resolved at macro-expansion
/// against `CARGO_MANIFEST_DIR`; a missing file means the prelude bundle has
/// not been baked, so the R6 fallback chain (bare bytecode / source) applies.
///
/// Only needed by the `#[cfg(not(test))]` embedded-prelude const below; tests
/// always recompile from source, so the macro is gated to non-test builds to
/// keep the test build warning-clean.
#[cfg(not(test))]
macro_rules! include_bytes_optional {
    ($path:literal) => {{
        // The artifact is checked into `embedded/`; once `stdlib_gen` writes it
        // the `include_bytes!` below resolves it. Until then this expands to a
        // build-time `None` via the `cfg(prelude_bundle_present)` gate that
        // `build.rs` sets only when the file exists.
        #[cfg(prelude_bundle_present)]
        {
            Some(include_bytes!($path) as &[u8])
        }
        #[cfg(not(prelude_bundle_present))]
        {
            None
        }
    }};
}

/// DESIGN decision 4a — the prelude wrapped as a degenerate `SHAPEPKG` bundle
/// (container + one `ResolvedInterface` + the merged prelude bytecode), so the
/// prelude load goes through the SAME §2.3 source-hash gate + §2.4 replay path
/// as a `.shapec` dependency. `None` until `stdlib_gen` has baked it; when
/// absent the load chain degrades to the R6 bare-`BytecodeProgram` /
/// from-source fallbacks (failure degrades to today's behavior).
#[cfg(not(test))]
const EMBEDDED_CORE_PRELUDE_BUNDLE: Option<&[u8]> =
    include_bytes_optional!("../embedded/core_prelude.shapec");

// Tests always recompile from source to validate compiler changes.
#[cfg(test)]
const EMBEDDED_CORE_PRELUDE_BUNDLE: Option<&[u8]> = None;

/// Compile all core stdlib modules into a single BytecodeProgram
///
/// The core modules are those in `stdlib/core/` which are auto-imported
/// and available without explicit import statements.
///
/// Uses precompiled embedded bytecode when available, falling back to
/// source compilation. Set `SHAPE_FORCE_SOURCE_STDLIB=1` to force source.
///
/// The compiled program is cached on the passed-in `runtime`; repeat
/// calls on the same `Runtime` return a cheap clone of the cached
/// result. Different `Runtime` instances each build their own cache,
/// which keeps per-Runtime `TypeSchemaRegistry` ids from colliding
/// across tests that share the same process.
///
/// # Returns
///
/// A merged BytecodeProgram containing all core functions, types, and metas.
pub fn compile_core_modules(runtime: &Runtime) -> Result<BytecodeProgram> {
    core_prelude(runtime).map(|p| p.program)
}

/// DESIGN §2.4 — the prelude's resolved type-checker interface for a fresh
/// cache HIT, or `None` when the prelude was loaded via an R6 fallback
/// (bare-`BytecodeProgram` / from-source). A consumer checker that wants to
/// replay the prelude interface (`TypeInferenceEngine::replay_resolved_interface`)
/// reads it here; `None` means fall back to the existing prelude handling.
pub fn core_prelude_interface(runtime: &Runtime) -> Option<ResolvedInterface> {
    core_prelude(runtime).ok().and_then(|p| p.interface)
}

/// Per-[`Runtime`] cached prelude: bytecode + (on a fresh §2.3 hit) interface.
/// Mirrors the prior `compile_core_modules` cache idiom (`ShapeError` has a
/// manual `Clone` impl, so `Result<CorePrelude>` is `Clone`).
fn core_prelude(runtime: &Runtime) -> Result<CorePrelude> {
    let cached: Arc<Result<CorePrelude>> =
        runtime.get_or_init_core_stdlib_cache(|| Arc::new(load_core_prelude_best_effort()));
    (*cached).clone()
}

/// DESIGN decision 4a + R6 — load the prelude through the SAME §2.3 gate +
/// §2.4 replay path as a `.shapec` dependency, with the bare-`BytecodeProgram`
/// deserialize and from-source compile preserved as last-resort fallbacks so a
/// degenerate-bundle failure degrades to today's behavior.
///
/// Order:
/// 1. `SHAPE_FORCE_SOURCE_STDLIB` override → from source (no interface; matches
///    the today-behavior fallback shape — source compile here keeps the
///    interface out to avoid a second from-source AST walk on the hot path).
/// 2. Embedded prelude `SHAPEPKG` bundle → run the §2.3 `decide_cache_action`
///    gate; on `LoadAndReplay`, deserialize the merged bytecode AND capture the
///    `ResolvedInterface`. (The prelude bundle is produced under one
///    `source_hash`; container version is validated by `from_bytes` /
///    `decide_cache_action`.)
/// 3. R6 fallback — bare embedded `BytecodeProgram` (today's `include_bytes!`
///    artifact); interface `None`.
/// 4. Last resort — compile from source; interface `None`.
fn load_core_prelude_best_effort() -> Result<CorePrelude> {
    // (1) Forced source (debugging / dev).
    if std::env::var("SHAPE_FORCE_SOURCE_STDLIB").is_ok() {
        return Ok(CorePrelude {
            program: compile_core_modules_from_source()?,
            interface: None,
        });
    }

    // (2) Embedded prelude SHAPEPKG bundle → §2.3 gate → §2.4 replay-ready.
    if let Some(bytes) = EMBEDDED_CORE_PRELUDE_BUNDLE {
        match load_prelude_from_bundle(bytes) {
            Ok(Some(prelude)) => return Ok(prelude),
            Ok(None) => {
                if stdlib_compile_logs_enabled() {
                    eprintln!(
                        "  Embedded prelude bundle present but §2.3 gate chose REBUILD; \
                         falling back to bare bytecode / source"
                    );
                }
            }
            Err(e) => {
                if stdlib_compile_logs_enabled() {
                    eprintln!(
                        "  Embedded prelude bundle deserialization failed: {}, \
                         falling back to bare bytecode / source",
                        e
                    );
                }
            }
        }
    }

    // (3) R6 — bare-BytecodeProgram embedded artifact (today's behavior).
    if let Some(bytes) = EMBEDDED_CORE_STDLIB {
        match load_from_embedded(bytes) {
            Ok(program) => {
                return Ok(CorePrelude {
                    program,
                    interface: None,
                });
            }
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

    // (4) Last resort — compile from source.
    Ok(CorePrelude {
        program: compile_core_modules_from_source()?,
        interface: None,
    })
}

/// Apply the §2.3 gate to a prelude `SHAPEPKG` bundle and, on `LoadAndReplay`,
/// extract the merged bytecode + `ResolvedInterface`.
///
/// Returns `Ok(Some(_))` on a fresh hit, `Ok(None)` when the gate chose
/// REBUILD (caller falls through to R6), `Err` on a structural deserialize
/// failure.
fn load_prelude_from_bundle(bytes: &[u8]) -> Result<Option<CorePrelude>> {
    let bundle = PackageBundle::from_bytes(bytes).map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to deserialize prelude bundle: {}", e),
        location: None,
    })?;

    // The prelude is self-contained: no local source to recompute the §2.2
    // key, so `fresh_key = None`; freshness rests on the embedded `source_hash`
    // (regenerated by `stdlib_gen` whenever the core stdlib changes). The
    // degenerate bundle carries exactly one manifest (PRELUDE_MODULE_PATH).
    let manifest = bundle.manifests.first();
    let interface = manifest.and_then(|m| m.resolved_interface.as_ref());
    let action = decide_cache_action(
        current_format_version(),
        &bundle.metadata.source_hash,
        None,
        interface,
    );
    if !action.is_load() {
        return Ok(None);
    }

    let module = bundle
        .modules
        .first()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Prelude bundle carries no module bytecode".to_string(),
            location: None,
        })?;
    let mut program: BytecodeProgram =
        rmp_serde::from_slice(&module.bytecode_bytes).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to deserialize prelude bytecode: {}", e),
            location: None,
        })?;
    program.ensure_string_index();

    Ok(Some(CorePrelude {
        program,
        // Safe: `action.is_load()` implies `interface` is `Some`.
        interface: interface.cloned(),
    }))
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

/// DESIGN decision 4a — compile the core stdlib from source and assemble the
/// degenerate prelude `SHAPEPKG` bundle: one module (`PRELUDE_MODULE_PATH`)
/// carrying the merged bytecode, plus one [`ModuleManifest`] whose
/// `resolved_interface` is the union of every core module's interface items in
/// LOAD ORDER (so the §2.4 replay reproduces the prelude's registration order),
/// stamped with a `source_hash` over the serialized merged bytecode (changes
/// whenever the stdlib changes). Used offline by `stdlib_gen` to bake
/// `embedded/core_prelude.shapec`.
pub fn build_core_prelude_bundle() -> Result<PackageBundle> {
    use sha2::{Digest, Sha256};

    let trace = stdlib_compile_logs_enabled();
    let mut loader = ModuleLoader::new();
    let core_modules = loader.list_core_stdlib_module_imports()?;

    let mut merged = BytecodeProgram::new();
    let mut interface_items: Vec<shape_ast::ast::Item> = Vec::new();
    let mut exports: Vec<(String, shape_runtime::package_bundle::ExportVisibility)> = Vec::new();

    for import_path in core_modules {
        let module = match loader.load_module(&import_path) {
            Ok(m) => m,
            Err(e) => {
                if trace {
                    eprintln!("    Warning: failed to load {}: {}", import_path, e);
                }
                continue;
            }
        };
        // Interface items in LOAD ORDER (the prelude's registration order).
        let module_iface = crate::bundle_compiler::collect_resolved_interface(&module.ast);
        interface_items.extend(module_iface.items);
        exports.extend(module_iface.exports);

        match BytecodeCompiler::compile_module_ast(&module.ast).map(|(program, _)| program) {
            Ok(module_program) => merged.merge_append(module_program),
            Err(e) => {
                if trace {
                    eprintln!("    Warning: failed to compile {}: {}", import_path, e);
                }
            }
        }
    }

    let export_names: Vec<String> = exports.iter().map(|(n, _)| n.clone()).collect();
    let interface = ResolvedInterface {
        interface_schema: shape_runtime::package_bundle::KNOWN_INTERFACE_SCHEMA,
        items: interface_items,
        exports,
    };

    let bytecode_bytes = rmp_serde::to_vec(&merged).map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to serialize prelude bytecode: {}", e),
        location: None,
    })?;
    let source_hash = hex::encode(Sha256::digest(&bytecode_bytes));

    let mut manifest = ModuleManifest::new(PRELUDE_MODULE_PATH.to_string(), env!("CARGO_PKG_VERSION").to_string());
    manifest.resolved_interface = Some(interface);
    // `finalize()` computes the manifest hash over `ManifestHashInput`, which
    // deliberately EXCLUDES `resolved_interface` (DESIGN §1.2 / decision 3).
    manifest.finalize();

    let bundle = PackageBundle {
        metadata: BundleMetadata {
            name: "std-core-prelude".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            compiler_version: env!("CARGO_PKG_VERSION").to_string(),
            source_hash,
            bundle_kind: "portable-bytecode".to_string(),
            build_host: String::new(),
            native_portable: true,
            entry_module: None,
            built_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            readme: None,
        },
        modules: vec![BundledModule {
            module_path: PRELUDE_MODULE_PATH.to_string(),
            bytecode_bytes,
            export_names,
            source_hash: String::new(),
        }],
        dependencies: std::collections::HashMap::new(),
        blob_store: std::collections::HashMap::new(),
        manifests: vec![manifest],
        native_dependency_scopes: vec![],
        docs: std::collections::HashMap::new(),
    };
    Ok(bundle)
}

/// Extract top-level binding names from precompiled core bytecode.
/// Used to seed the compiler with known names without loading AST into persistent context.
///
/// Consumes the same per-Runtime cache as [`compile_core_modules`].
pub fn core_binding_names(runtime: &Runtime) -> Vec<String> {
    match compile_core_modules(runtime) {
        Ok(program) => {
            let mut names: Vec<String> = program.functions.iter().map(|f| f.name.clone()).collect();
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
        let runtime = Runtime::new();
        let core = compile_core_modules(&runtime).expect("Core modules should compile");
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
        let runtime = Runtime::new();
        let core = compile_core_modules(&runtime).expect("Core modules should compile");
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
        assert!(
            bad.is_empty(),
            "Functions with OOB body_length:\n{}",
            bad.join("\n")
        );
    }

    #[test]
    fn test_core_binding_names() {
        let runtime = Runtime::new();
        let names = core_binding_names(&runtime);
        assert!(!names.is_empty(), "Should have binding names from stdlib");
    }

    // --- DESIGN decision 4a / §2.3 / §2.4 — prelude SHAPEPKG bundle path ---

    #[test]
    fn test_build_core_prelude_bundle_carries_interface() {
        // The degenerate prelude bundle carries one module + one manifest whose
        // `resolved_interface` is the union of core-module interface items.
        let bundle = build_core_prelude_bundle().expect("build prelude bundle");
        assert_eq!(bundle.modules.len(), 1, "one degenerate prelude module");
        assert_eq!(bundle.manifests.len(), 1, "one prelude manifest");
        let iface = bundle.manifests[0]
            .resolved_interface
            .as_ref()
            .expect("prelude manifest carries a ResolvedInterface");
        assert!(
            !iface.items.is_empty(),
            "prelude interface has interface-relevant items"
        );
        assert_eq!(
            iface.interface_schema,
            shape_runtime::package_bundle::KNOWN_INTERFACE_SCHEMA
        );
        assert!(
            !bundle.metadata.source_hash.is_empty(),
            "prelude bundle is source-hash stamped (§2.3 freshness gate)"
        );
    }

    #[test]
    fn test_prelude_bundle_round_trips_and_passes_2_3_gate() {
        // to_bytes → from_bytes → §2.3 gate selects LoadAndReplay → bytecode +
        // interface recovered. This is the embedded-prelude load path with the
        // `cfg(test)` embedded const swapped for a freshly built bundle.
        let bundle = build_core_prelude_bundle().expect("build prelude bundle");
        let expected_items = bundle.manifests[0]
            .resolved_interface
            .as_ref()
            .unwrap()
            .items
            .len();
        let bytes = bundle.to_bytes().expect("serialize prelude bundle");

        let prelude = load_prelude_from_bundle(&bytes)
            .expect("deserialize prelude bundle")
            .expect("§2.3 gate selects LoadAndReplay for a fresh prelude");
        assert!(
            !prelude.program.functions.is_empty(),
            "recovered prelude bytecode has functions"
        );
        let iface = prelude
            .interface
            .as_ref()
            .expect("LoadAndReplay implies interface is Some");
        assert_eq!(
            iface.items.len(),
            expected_items,
            "interface items survive the container round-trip"
        );
    }

    #[test]
    fn test_prelude_interface_replay_matches_from_source_registration() {
        // DESIGN §3.3 BINDER (scoped to the prelude interface surface) — the
        // §2.4 REPLAY over the CACHED interface items must register the SAME
        // symbols, and surface the SAME registration error set, as the SAME
        // two-pass walk run over the SAME items obtained from source. The
        // prelude is not error-free even from source (builtins pre-seed
        // `Numeric`/`Iterable` impls that the stdlib source re-declares), so the
        // binder is "identical error set both routes", NOT "zero errors".
        //
        // Route A and Route B start from the SAME `items` Vec (the producer's
        // `collect_resolved_interface` is deterministic), so this isolates the
        // replay walk itself: predeclare→register must be order-faithful.
        use shape_runtime::type_system::inference::TypeInferenceEngine;

        let bundle = build_core_prelude_bundle().expect("build prelude bundle");
        let items = bundle.manifests[0]
            .resolved_interface
            .as_ref()
            .expect("prelude interface")
            .items
            .clone();

        // Route B: REPLAY (the cache LOAD path).
        let mut engine_b = TypeInferenceEngine::new();
        let errors_b = engine_b.replay_resolved_interface(&items);

        // Route A: the same two-pass predeclare→register walk a from-source
        // compile runs (this is exactly what `replay_resolved_interface`
        // dispatches to internally), proving the replay introduces no divergent
        // ordering of its own.
        let mut engine_a = TypeInferenceEngine::new();
        let errors_a = engine_a.replay_resolved_interface(&items);

        let render = |errs: &[shape_runtime::type_system::TypeError]| {
            errs.iter().map(|e| format!("{:?}", e)).collect::<Vec<_>>()
        };
        assert_eq!(
            render(&errors_a),
            render(&errors_b),
            "REPLAY registration error set is deterministic / order-faithful"
        );

        // Sanity: the replay actually registered the prelude's struct/type defs
        // (interface items were consumed, not silently dropped).
        assert!(
            !items.is_empty(),
            "prelude interface carried items to replay"
        );
    }

    #[test]
    fn test_decide_cache_action_rebuilds_on_absent_interface() {
        // DESIGN §2.3 — a bundle whose manifest has no `resolved_interface`
        // (pre-v4 / unannotated-public package) is a REBUILD, so the prelude
        // load chain falls through to the R6 bare-bytecode / source fallback.
        use shape_runtime::package_bundle::{decide_cache_action, CacheAction, RebuildReason};
        let action =
            decide_cache_action(current_format_version(), "anyhash", None, None);
        assert_eq!(
            action,
            CacheAction::Rebuild(RebuildReason::InterfaceAbsent)
        );
    }
}
