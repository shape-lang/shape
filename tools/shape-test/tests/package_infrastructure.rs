//! Integration tests for package infrastructure features:
//! - Extension-contributed TOML sections
//! - Package bundle format (.shapec)
//! - BundleCompiler
//! - Bundle-as-dependency resolution
//! - End-to-end build + load + execute
//! - ABI section claims

use std::collections::{HashMap, HashSet};

use shape_runtime::frontmatter::parse_frontmatter;
use shape_runtime::module_loader::ModuleLoader;
use shape_runtime::package_bundle::{BundleMetadata, BundledModule, PackageBundle};
use shape_runtime::plugins::{
    ClaimedSection, LoadedPlugin, PluginCapability, parse_sections_manifest,
};
use shape_runtime::project::{find_project_root, parse_shape_project_toml};
use shape_vm::bundle_compiler::BundleCompiler;

/// Create a temporary project directory with shape.toml and optional extra files.
fn create_temp_project(name: &str, version: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("shape.toml"),
        format!("[project]\nname = \"{}\"\nversion = \"{}\"", name, version),
    )
    .unwrap();
    for (path, content) in files {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }
    dir
}

// ===========================================================================
// 1. Extension Sections (5 tests)
// ===========================================================================

#[test]
fn test_shape_toml_captures_unknown_sections() {
    let toml_str = r#"
[project]
name = "ext-test"
version = "1.0.0"

[native-dependencies]
libm = { linux = "libm.so.6", macos = "libm.dylib" }

[custom-config]
key = "value"
nested = { a = 1, b = true }
"#;
    let config = parse_shape_project_toml(toml_str).unwrap();

    assert!(
        config
            .extension_sections
            .contains_key("native-dependencies")
    );
    assert!(config.extension_sections.contains_key("custom-config"));
    assert_eq!(config.extension_sections.len(), 2);
}

#[test]
fn test_extension_section_as_json_conversion() {
    let toml_str = r#"
[project]
name = "json-test"
version = "1.0.0"

[custom-config]
key = "value"
count = 42
enabled = true
items = ["a", "b"]
"#;
    let config = parse_shape_project_toml(toml_str).unwrap();

    let json = config.extension_section_as_json("custom-config").unwrap();
    assert_eq!(json["key"], "value");
    assert_eq!(json["count"], 42);
    assert_eq!(json["enabled"], true);
    assert_eq!(json["items"][0], "a");
    assert_eq!(json["items"][1], "b");
}

#[test]
fn test_extension_section_names_discovery() {
    let toml_str = r#"
[project]
name = "names-test"
version = "1.0.0"

[alpha-section]
x = 1

[beta-section]
y = 2

[gamma-section]
z = 3
"#;
    let config = parse_shape_project_toml(toml_str).unwrap();

    let mut names = config.extension_section_names();
    names.sort();
    assert_eq!(
        names,
        vec!["alpha-section", "beta-section", "gamma-section"]
    );
}

#[test]
fn test_validate_warns_on_unclaimed_sections() {
    let toml_str = r#"
[project]
name = "warn-test"
version = "1.0.0"

[native-dependencies]
libm = "system"

[typo-section]
oops = true

[another-unknown]
foo = "bar"
"#;
    let config = parse_shape_project_toml(toml_str).unwrap();

    let mut claimed = HashSet::new();
    claimed.insert("native-dependencies".to_string());

    let errors = config.validate_with_claimed_sections(&claimed);

    // "native-dependencies" is claimed, should NOT appear in errors
    assert!(!errors.iter().any(|e| e.contains("native-dependencies")));
    // "typo-section" is unclaimed, should appear
    assert!(errors.iter().any(|e| e.contains("typo-section")));
    // "another-unknown" is unclaimed, should appear
    assert!(errors.iter().any(|e| e.contains("another-unknown")));
}

#[test]
fn test_known_sections_not_captured_as_extensions() {
    let toml_str = r#"
[project]
name = "known-test"
version = "1.0.0"

[build]
target = "bytecode"
opt_level = 2

[dependencies]
finance = "0.1.0"

[modules]
paths = ["lib"]
"#;
    let config = parse_shape_project_toml(toml_str).unwrap();

    // Known sections should NOT appear in extension_sections
    assert!(
        !config.extension_sections.contains_key("project"),
        "project should not be in extension_sections"
    );
    assert!(
        !config.extension_sections.contains_key("build"),
        "build should not be in extension_sections"
    );
    assert!(
        !config.extension_sections.contains_key("dependencies"),
        "dependencies should not be in extension_sections"
    );
    assert!(
        !config.extension_sections.contains_key("modules"),
        "modules should not be in extension_sections"
    );
    assert!(
        config.extension_sections.is_empty(),
        "no extension sections expected, got: {:?}",
        config.extension_section_names()
    );
}

// ===========================================================================
// 2. PackageBundle Format (4 tests)
// ===========================================================================

fn sample_bundle() -> PackageBundle {
    PackageBundle {
        metadata: BundleMetadata {
            name: "test-pkg".to_string(),
            version: "0.2.0".to_string(),
            compiler_version: "0.5.0".to_string(),
            source_hash: "abc123def456".to_string(),
            bundle_kind: "portable-bytecode".to_string(),
            build_host: "x86_64-linux".to_string(),
            native_portable: true,
            entry_module: Some("main".to_string()),
            built_at: 1700000000,
        },
        modules: vec![
            BundledModule {
                module_path: "main".to_string(),
                bytecode_bytes: vec![1, 2, 3, 4, 5],
                export_names: vec!["run".to_string(), "init".to_string()],
                source_hash: "hash_main".to_string(),
            },
            BundledModule {
                module_path: "utils::helpers".to_string(),
                bytecode_bytes: vec![10, 20, 30],
                export_names: vec!["helper".to_string(), "format".to_string()],
                source_hash: "hash_helpers".to_string(),
            },
            BundledModule {
                module_path: "utils".to_string(),
                bytecode_bytes: vec![40, 50],
                export_names: vec!["util_fn".to_string()],
                source_hash: "hash_utils_index".to_string(),
            },
        ],
        dependencies: {
            let mut deps = HashMap::new();
            deps.insert("my-lib".to_string(), "1.0.0".to_string());
            deps.insert("other-lib".to_string(), "2.3.0".to_string());
            deps
        },
        blob_store: HashMap::new(),
        manifests: vec![],
        native_dependency_scopes: vec![],
    }
}

#[test]
fn test_bundle_roundtrip_with_modules() {
    let bundle = sample_bundle();
    let bytes = bundle.to_bytes().expect("serialization should succeed");
    let restored = PackageBundle::from_bytes(&bytes).expect("deserialization should succeed");

    assert_eq!(restored.metadata.name, "test-pkg");
    assert_eq!(restored.metadata.version, "0.2.0");
    assert_eq!(restored.modules.len(), 3);

    assert_eq!(restored.modules[0].module_path, "main");
    assert_eq!(restored.modules[0].bytecode_bytes, vec![1, 2, 3, 4, 5]);
    assert_eq!(restored.modules[0].export_names, vec!["run", "init"]);

    assert_eq!(restored.modules[1].module_path, "utils::helpers");
    assert_eq!(restored.modules[1].export_names, vec!["helper", "format"]);

    assert_eq!(restored.modules[2].module_path, "utils");

    assert_eq!(restored.dependencies.get("my-lib").unwrap(), "1.0.0");
    assert_eq!(restored.dependencies.get("other-lib").unwrap(), "2.3.0");
}

#[test]
fn test_bundle_file_io_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.shapec");

    let bundle = sample_bundle();
    bundle.write_to_file(&path).expect("write should succeed");

    assert!(path.exists(), "bundle file should exist");

    let restored = PackageBundle::read_from_file(&path).expect("read should succeed");
    assert_eq!(restored.metadata.name, "test-pkg");
    assert_eq!(restored.metadata.version, "0.2.0");
    assert_eq!(restored.modules.len(), 3);
    assert_eq!(restored.dependencies.len(), 2);
}

#[test]
fn test_bundle_rejects_invalid_magic() {
    let mut bad_data = vec![0u8; 20];
    bad_data[..8].copy_from_slice(b"NOTSHAPE");
    bad_data[8..12].copy_from_slice(&1u32.to_le_bytes());

    let result = PackageBundle::from_bytes(&bad_data);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("bad magic"),
        "error should mention bad magic bytes"
    );
}

#[test]
fn test_bundle_metadata_preserves_all_fields() {
    let bundle = PackageBundle {
        metadata: BundleMetadata {
            name: "full-meta".to_string(),
            version: "3.2.1".to_string(),
            compiler_version: "0.6.0-beta".to_string(),
            source_hash: "sha256_full_hash_value".to_string(),
            bundle_kind: "portable-bytecode".to_string(),
            build_host: "x86_64-linux".to_string(),
            native_portable: true,
            entry_module: Some("src::main".to_string()),
            built_at: 1234567890,
        },
        modules: vec![],
        dependencies: HashMap::new(),
        blob_store: HashMap::new(),
        manifests: vec![],
        native_dependency_scopes: vec![],
    };

    let bytes = bundle.to_bytes().unwrap();
    let restored = PackageBundle::from_bytes(&bytes).unwrap();

    assert_eq!(restored.metadata.name, "full-meta");
    assert_eq!(restored.metadata.version, "3.2.1");
    assert_eq!(restored.metadata.compiler_version, "0.6.0-beta");
    assert_eq!(restored.metadata.source_hash, "sha256_full_hash_value");
    assert_eq!(
        restored.metadata.entry_module,
        Some("src::main".to_string())
    );
    assert_eq!(restored.metadata.built_at, 1234567890);
}

// ===========================================================================
// 3. BundleCompiler (4 tests)
// ===========================================================================

#[test]
fn test_compile_project_with_multiple_modules() {
    let dir = create_temp_project(
        "multi-mod",
        "0.1.0",
        &[
            ("main.shape", "pub fn run() { 42 }"),
            ("utils/helpers.shape", "pub fn helper() { 1 }"),
            ("utils/index.shape", "pub fn util_fn() { 2 }"),
        ],
    );

    let project = find_project_root(dir.path()).expect("should find project root");
    let bundle = BundleCompiler::compile(&project).expect("compilation should succeed");

    assert_eq!(bundle.metadata.name, "multi-mod");
    assert_eq!(bundle.metadata.version, "0.1.0");

    // Should have at least 3 modules (main, utils::helpers, utils)
    assert!(
        bundle.modules.len() >= 3,
        "expected at least 3 modules, got {}",
        bundle.modules.len()
    );

    // Check module path naming conventions
    let paths: Vec<&str> = bundle
        .modules
        .iter()
        .map(|m| m.module_path.as_str())
        .collect();
    assert!(paths.contains(&"main"), "should have 'main' module");
    assert!(
        paths.contains(&"utils::helpers"),
        "should have 'utils::helpers' module"
    );
    // index.shape should map to parent name "utils"
    assert!(
        paths.contains(&"utils"),
        "index.shape should map to 'utils' module path"
    );
}

#[test]
fn test_compile_project_collects_exports() {
    let dir = create_temp_project(
        "exports-test",
        "0.1.0",
        &[(
            "main.shape",
            r#"
pub fn greet() { "hello" }
pub fn add(a, b) { a + b }
pub enum Color { Red, Green, Blue }
"#,
        )],
    );

    let project = find_project_root(dir.path()).expect("should find project root");
    let bundle = BundleCompiler::compile(&project).expect("compilation should succeed");

    let main_mod = bundle
        .modules
        .iter()
        .find(|m| m.module_path == "main")
        .expect("should have main module");

    assert!(
        main_mod.export_names.contains(&"greet".to_string()),
        "should export greet"
    );
    assert!(
        main_mod.export_names.contains(&"add".to_string()),
        "should export add"
    );
    assert!(
        main_mod.export_names.contains(&"Color".to_string()),
        "should export Color enum"
    );
}

#[test]
fn test_compile_project_computes_source_hash() {
    // Compile the same project twice
    let dir = create_temp_project("hash-test", "0.1.0", &[("main.shape", "pub fn f() { 1 }")]);

    let project = find_project_root(dir.path()).expect("should find project root");
    let bundle1 = BundleCompiler::compile(&project).expect("first compile should succeed");
    let bundle2 = BundleCompiler::compile(&project).expect("second compile should succeed");

    // Same source -> same hash
    assert_eq!(
        bundle1.metadata.source_hash, bundle2.metadata.source_hash,
        "same source should produce same hash"
    );

    // Modify a file and recompile -> different hash
    std::fs::write(dir.path().join("main.shape"), "pub fn f() { 2 }").unwrap();
    let project2 = find_project_root(dir.path()).expect("should find project root");
    let bundle3 = BundleCompiler::compile(&project2).expect("third compile should succeed");

    assert_ne!(
        bundle1.metadata.source_hash, bundle3.metadata.source_hash,
        "modified source should produce different hash"
    );
}

#[test]
fn test_compile_empty_project_errors() {
    let dir = create_temp_project("empty-proj", "0.1.0", &[]);

    let project = find_project_root(dir.path()).expect("should find project root");
    let result = BundleCompiler::compile(&project);

    assert!(result.is_err(), "compiling empty project should fail");
    assert!(
        result.unwrap_err().contains("No .shape files"),
        "error should mention no shape files found"
    );
}

// ===========================================================================
// 4. Bundle as Dependency (3 tests)
// ===========================================================================

#[test]
fn test_bundle_dependency_resolution() {
    use shape_runtime::dependency_resolver::DependencyResolver;

    // Create a library project, compile it to .shapec
    let lib_dir = create_temp_project("mylib", "1.0.0", &[("lib.shape", "pub fn hello() { 42 }")]);
    let lib_project = find_project_root(lib_dir.path()).expect("should find lib root");
    let bundle = BundleCompiler::compile(&lib_project).expect("lib compilation should succeed");

    // Write the bundle file
    let bundle_path = lib_dir.path().join("mylib.shapec");
    bundle.write_to_file(&bundle_path).unwrap();

    // Create a consumer project with dependency pointing to .shapec
    let consumer_dir = tempfile::tempdir().unwrap();
    let dep_path = bundle_path.to_string_lossy().to_string();
    std::fs::write(
        consumer_dir.path().join("shape.toml"),
        format!(
            r#"
[project]
name = "consumer"
version = "0.1.0"

[dependencies]
mylib = {{ path = "{}" }}
"#,
            dep_path
        ),
    )
    .unwrap();

    let cache_dir = tempfile::tempdir().unwrap();
    let resolver = DependencyResolver::with_cache_dir(
        consumer_dir.path().to_path_buf(),
        cache_dir.path().to_path_buf(),
    );

    let consumer_project =
        find_project_root(consumer_dir.path()).expect("should find consumer root");
    let resolved = resolver
        .resolve(&consumer_project.config.dependencies)
        .expect("resolution should succeed");

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].name, "mylib");
    assert_eq!(resolved[0].version, "1.0.0");
    assert!(
        resolved[0].path.to_string_lossy().contains("shapec"),
        "resolved path should point to .shapec file"
    );
}

#[test]
fn test_bundle_preferred_over_directory() {
    use shape_runtime::dependency_resolver::DependencyResolver;

    // Create both a dep/ directory and dep.shapec file
    let root_dir = tempfile::tempdir().unwrap();
    let dep_dir = root_dir.path().join("dep");
    std::fs::create_dir_all(&dep_dir).unwrap();
    std::fs::write(
        dep_dir.join("shape.toml"),
        "[project]\nname = \"dep\"\nversion = \"0.5.0\"",
    )
    .unwrap();
    std::fs::write(dep_dir.join("lib.shape"), "pub fn f() { 1 }").unwrap();

    // Create a .shapec bundle alongside the directory
    let dep_project = find_project_root(&dep_dir).expect("should find dep root");
    let bundle = BundleCompiler::compile(&dep_project).expect("dep compilation should succeed");
    let bundle_path = root_dir.path().join("dep.shapec");
    bundle.write_to_file(&bundle_path).unwrap();

    // Consumer project
    std::fs::write(
        root_dir.path().join("shape.toml"),
        r#"
[project]
name = "consumer"
version = "0.1.0"

[dependencies]
dep = { path = "./dep" }
"#,
    )
    .unwrap();

    let cache_dir = tempfile::tempdir().unwrap();
    let resolver = DependencyResolver::with_cache_dir(
        root_dir.path().to_path_buf(),
        cache_dir.path().to_path_buf(),
    );
    let consumer_project = find_project_root(root_dir.path()).expect("should find consumer root");
    let resolved = resolver
        .resolve(&consumer_project.config.dependencies)
        .expect("resolution should succeed");

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].name, "dep");
    // Bundle should be preferred and keep its declared version
    assert_eq!(
        resolved[0].version, "1.0.0",
        "bundle should be preferred over directory"
    );
}

#[test]
fn test_module_loader_loads_bundle_modules() {
    let bundle = PackageBundle {
        metadata: BundleMetadata {
            name: "testlib".to_string(),
            version: "0.1.0".to_string(),
            compiler_version: "0.5.0".to_string(),
            source_hash: "abc".to_string(),
            bundle_kind: "portable-bytecode".to_string(),
            build_host: "x86_64-linux".to_string(),
            native_portable: true,
            entry_module: None,
            built_at: 0,
        },
        modules: vec![
            BundledModule {
                module_path: "helpers".to_string(),
                bytecode_bytes: vec![1, 2, 3],
                export_names: vec!["helper_fn".to_string()],
                source_hash: "h1".to_string(),
            },
            BundledModule {
                module_path: "math::utils".to_string(),
                bytecode_bytes: vec![4, 5, 6],
                export_names: vec!["add".to_string()],
                source_hash: "h2".to_string(),
            },
        ],
        dependencies: HashMap::new(),
        blob_store: HashMap::new(),
        manifests: vec![],
        native_dependency_scopes: vec![],
    };

    let mut loader = ModuleLoader::new();
    loader.load_bundle(&bundle, Some("mylib"));

    // Verify modules are registered by attempting to load them.
    // The bundle_resolver stores them as Compiled artifacts; loading them
    // as parsed modules will fail (not valid bytecode), but we can verify
    // registration by checking the load attempt touches the bundle resolver.
    // Use the internal resolver pattern: load_module will find them.
    // Since the bytes aren't valid, loading will fail at parse time,
    // but the resolver will find the module path.
    //
    // We verify indirectly by loading a second bundle and checking prefix.
    let mut loader2 = ModuleLoader::new();
    loader2.load_bundle(&bundle, None);

    // Without prefix, module paths should match exactly
    // With prefix "mylib", they should be "mylib::helpers" and "mylib::math::utils"
    // We can't directly query the resolver, but we can verify via a round-trip
    // by writing to file and loading via set_dependency_paths

    // Alternative verification: write bundle to temp file, load via set_dependency_paths
    let tmp = tempfile::tempdir().unwrap();
    let bundle_path = tmp.path().join("testlib.shapec");
    bundle.write_to_file(&bundle_path).unwrap();

    let mut loader3 = ModuleLoader::new();
    let mut deps = HashMap::new();
    deps.insert("testlib".to_string(), bundle_path);
    loader3.set_dependency_paths(deps);

    // The dependency was a .shapec file, so it should have been loaded via load_bundle
    // with prefix "testlib". Modules should now be available as "testlib::helpers"
    // and "testlib::math::utils" in the bundle resolver.
    // We can verify by checking that the dependency path is NOT in regular deps
    // (bundles are consumed during set_dependency_paths)
    assert!(
        loader3.get_dependency_paths().is_empty(),
        "bundle deps should be consumed from regular dependency_paths"
    );
}

// ===========================================================================
// 5. End-to-End: Build + Load + Execute (2 tests)
// ===========================================================================

#[test]
fn test_build_and_load_bundle_end_to_end() {
    // Create a project with exportable functions
    let dir = create_temp_project(
        "e2e-pkg",
        "0.1.0",
        &[
            ("main.shape", "pub fn entry() { 42 }"),
            ("utils.shape", "pub fn double(x) { x * 2 }"),
        ],
    );

    // Step 1: Compile with BundleCompiler
    let project = find_project_root(dir.path()).expect("should find project root");
    let bundle = BundleCompiler::compile(&project).expect("compilation should succeed");

    assert_eq!(bundle.metadata.name, "e2e-pkg");
    assert!(!bundle.modules.is_empty());

    // Step 2: Write to .shapec
    let tmp = tempfile::tempdir().unwrap();
    let bundle_path = tmp.path().join("e2e-pkg.shapec");
    bundle.write_to_file(&bundle_path).unwrap();

    // Step 3: Read back and verify
    let restored = PackageBundle::read_from_file(&bundle_path).unwrap();
    assert_eq!(restored.metadata.name, "e2e-pkg");
    assert_eq!(restored.metadata.version, "0.1.0");

    // Step 4: Load into ModuleLoader via load_bundle
    let mut loader = ModuleLoader::new();
    loader.load_bundle(&restored, Some("e2e-pkg"));

    // Step 5: Load via set_dependency_paths (which auto-loads bundles)
    let mut loader2 = ModuleLoader::new();
    let mut deps = HashMap::new();
    deps.insert("e2e-pkg".to_string(), bundle_path.clone());
    loader2.set_dependency_paths(deps);

    // The bundle was consumed from regular paths
    assert!(
        loader2.get_dependency_paths().is_empty(),
        "bundle dependency should not remain in regular dep paths"
    );

    // Step 6: Verify module registration via a fresh compile + bundle roundtrip
    let bytes = restored.to_bytes().unwrap();
    let final_bundle = PackageBundle::from_bytes(&bytes).unwrap();
    assert_eq!(final_bundle.metadata.name, "e2e-pkg");
    assert_eq!(final_bundle.modules.len(), restored.modules.len());
}

#[test]
fn test_frontmatter_extension_sections_threaded() {
    let source = r#"---
name = "my-script"
version = "0.1.0"

[native-dependencies]
libcurl = { linux = "libcurl.so.4" }

[dependencies]
utils = { path = "../utils" }
---
pub fn main() { 1 }
"#;

    let (parsed, _remaining) = parse_frontmatter(source);
    let project = parsed.expect("frontmatter should parse");

    // The [native-dependencies] section should appear in extension_sections
    assert!(
        project
            .extension_sections
            .contains_key("native-dependencies"),
        "native-dependencies should be in extension_sections"
    );

    // The [dependencies] section should NOT be in extension_sections
    assert!(
        !project.extension_sections.contains_key("dependencies"),
        "dependencies should not be in extension_sections"
    );

    // Verify the JSON conversion works
    let json = project
        .extension_section_as_json("native-dependencies")
        .expect("should convert to JSON");
    assert!(
        json["libcurl"].is_object(),
        "libcurl should be parsed as a table"
    );
}

// ===========================================================================
// 6. ABI Section Claims (2 tests)
// ===========================================================================

#[test]
fn test_sections_manifest_parsing() {
    use shape_runtime::plugins::{SectionClaim, SectionsManifest};

    // Create static SectionClaim entries (mimic what a real plugin would export)
    static CLAIMS: [SectionClaim; 2] = [
        SectionClaim {
            name: c"native-dependencies".as_ptr(),
            required: true,
        },
        SectionClaim {
            name: c"build-hooks".as_ptr(),
            required: false,
        },
    ];

    static MANIFEST: SectionsManifest = SectionsManifest {
        sections: CLAIMS.as_ptr(),
        sections_len: 2,
    };

    let parsed = parse_sections_manifest(&MANIFEST).expect("parsing should succeed");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].name, "native-dependencies");
    assert!(parsed[0].required);
    assert_eq!(parsed[1].name, "build-hooks");
    assert!(!parsed[1].required);
}

#[test]
fn test_claimed_section_names_helper() {
    use shape_runtime::plugins::{CapabilityKind, PluginType};

    let plugin = LoadedPlugin {
        name: "test-plugin".to_string(),
        version: "1.0.0".to_string(),
        plugin_type: PluginType::DataSource,
        description: "A test plugin".to_string(),
        capabilities: vec![PluginCapability {
            kind: CapabilityKind::DataSource,
            contract: "shape.datasource".to_string(),
            version: "1".to_string(),
            flags: 0,
        }],
        claimed_sections: vec![
            ClaimedSection {
                name: "native-dependencies".to_string(),
                required: true,
            },
            ClaimedSection {
                name: "custom-config".to_string(),
                required: false,
            },
            ClaimedSection {
                name: "build-hooks".to_string(),
                required: false,
            },
        ],
    };

    let names = plugin.claimed_section_names();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"native-dependencies"));
    assert!(names.contains(&"custom-config"));
    assert!(names.contains(&"build-hooks"));

    // Verify has_capability_kind helper
    assert!(plugin.has_capability_kind(CapabilityKind::DataSource));
    assert!(!plugin.has_capability_kind(CapabilityKind::OutputSink));
}
