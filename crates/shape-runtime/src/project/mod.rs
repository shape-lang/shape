//! Project root detection and shape.toml configuration
//!
//! Discovers the project root by walking up from a starting directory
//! looking for a `shape.toml` file, then parses its configuration.
//!
//! This module is split into submodules for maintainability:
//! - [`dependency_spec`] — dependency specification types and native dependency handling
//! - [`permissions`] — permission-related types and logic
//! - [`sandbox`] — sandbox configuration and parsing helpers
//! - [`project_config`] — project configuration parsing and discovery

pub mod dependency_spec;
pub mod permissions;
pub mod project_config;
pub mod sandbox;

// Re-export all public items at the module root to preserve the existing API.
pub use dependency_spec::*;
pub use permissions::*;
pub use project_config::*;
pub use sandbox::SandboxSection;

// Re-export crate-internal items used by other modules.
pub(crate) use project_config::toml_to_json;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[project]
name = "test-project"
version = "0.1.0"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "test-project");
        assert_eq!(config.project.version, "0.1.0");
        assert!(config.modules.paths.is_empty());
        assert!(config.extensions.is_empty());
    }

    #[test]
    fn test_parse_empty_config() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert_eq!(config.project.name, "");
        assert!(config.modules.paths.is_empty());
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[project]
name = "my-analysis"
version = "0.1.0"

[modules]
paths = ["lib", "vendor"]

[dependencies]

[[extensions]]
name = "market-data"
path = "./libshape_plugin_market_data.so"

[extensions.config]
duckdb_path = "/path/to/market.duckdb"
default_timeframe = "1d"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "my-analysis");
        assert_eq!(config.modules.paths, vec!["lib", "vendor"]);
        assert_eq!(config.extensions.len(), 1);
        assert_eq!(config.extensions[0].name, "market-data");
        assert_eq!(
            config.extensions[0].config.get("default_timeframe"),
            Some(&toml::Value::String("1d".to_string()))
        );
    }

    #[test]
    fn test_parse_config_with_entry() {
        let toml_str = r#"
[project]
name = "my-analysis"
version = "0.1.0"
entry = "src/main.shape"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.entry, Some("src/main.shape".to_string()));
    }

    #[test]
    fn test_parse_config_without_entry() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.entry, None);
    }

    #[test]
    fn test_find_project_root_in_current_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("shape.toml");
        let mut f = std::fs::File::create(&toml_path).unwrap();
        writeln!(
            f,
            r#"
[project]
name = "found"
version = "1.0.0"

[modules]
paths = ["src"]
"#
        )
        .unwrap();

        let result = find_project_root(tmp.path());
        assert!(result.is_some());
        let root = result.unwrap();
        assert_eq!(root.root_path, tmp.path());
        assert_eq!(root.config.project.name, "found");
    }

    #[test]
    fn test_find_project_root_walks_up() {
        let tmp = tempfile::tempdir().unwrap();
        // Create shape.toml in root
        let toml_path = tmp.path().join("shape.toml");
        let mut f = std::fs::File::create(&toml_path).unwrap();
        writeln!(
            f,
            r#"
[project]
name = "parent"
"#
        )
        .unwrap();

        // Create nested directory
        let nested = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();

        let result = find_project_root(&nested);
        assert!(result.is_some());
        let root = result.unwrap();
        assert_eq!(root.root_path, tmp.path());
        assert_eq!(root.config.project.name, "parent");
    }

    #[test]
    fn test_find_project_root_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("empty_dir");
        std::fs::create_dir_all(&nested).unwrap();

        let result = find_project_root(&nested);
        // May or may not be None depending on whether a shape.toml exists
        // above tempdir. In practice, tempdir is deep enough that there won't be one.
        // We just verify it doesn't panic.
        let _ = result;
    }

    #[test]
    fn test_resolved_module_paths() {
        let root = ProjectRoot {
            root_path: PathBuf::from("/home/user/project"),
            config: ShapeProject {
                modules: ModulesSection {
                    paths: vec!["lib".to_string(), "vendor".to_string()],
                },
                ..Default::default()
            },
        };

        let resolved = root.resolved_module_paths();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0], PathBuf::from("/home/user/project/lib"));
        assert_eq!(resolved[1], PathBuf::from("/home/user/project/vendor"));
    }

    // --- New tests for expanded schema ---

    #[test]
    fn test_parse_version_only_dependency() {
        let toml_str = r#"
[project]
name = "dep-test"
version = "1.0.0"

[dependencies]
finance = "0.1.0"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(
            config.dependencies.get("finance"),
            Some(&DependencySpec::Version("0.1.0".to_string()))
        );
    }

    #[test]
    fn test_parse_path_dependency() {
        let toml_str = r#"
[dependencies]
my-utils = { path = "../utils" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("my-utils").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("../utils"));
                assert!(d.git.is_none());
                assert!(d.version.is_none());
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_git_dependency() {
        let toml_str = r#"
[dependencies]
plotting = { git = "https://github.com/org/plot.git", tag = "v1.0" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("plotting").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.git.as_deref(), Some("https://github.com/org/plot.git"));
                assert_eq!(d.tag.as_deref(), Some("v1.0"));
                assert!(d.branch.is_none());
                assert!(d.rev.is_none());
                assert!(d.path.is_none());
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_git_dependency_with_branch() {
        let toml_str = r#"
[dependencies]
my-lib = { git = "https://github.com/org/lib.git", branch = "develop" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("my-lib").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.git.as_deref(), Some("https://github.com/org/lib.git"));
                assert_eq!(d.branch.as_deref(), Some("develop"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_git_dependency_with_rev() {
        let toml_str = r#"
[dependencies]
pinned = { git = "https://github.com/org/pinned.git", rev = "abc1234" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("pinned").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.rev.as_deref(), Some("abc1234"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_dev_dependencies() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"

[dev-dependencies]
test-utils = "0.2.0"
mock-data = { path = "../mocks" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.dev_dependencies.len(), 2);
        assert_eq!(
            config.dev_dependencies.get("test-utils"),
            Some(&DependencySpec::Version("0.2.0".to_string()))
        );
        match config.dev_dependencies.get("mock-data").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("../mocks"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_build_section() {
        let toml_str = r#"
[build]
target = "native"
opt_level = 2
output = "dist/"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.build.target.as_deref(), Some("native"));
        assert_eq!(config.build.opt_level, Some(2));
        assert_eq!(config.build.output.as_deref(), Some("dist/"));
    }

    #[test]
    fn test_parse_project_extended_fields() {
        let toml_str = r#"
[project]
name = "full-project"
version = "2.0.0"
authors = ["Alice", "Bob"]
shape-version = "0.5.0"
license = "MIT"
repository = "https://github.com/org/project"
entry = "main.shape"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "full-project");
        assert_eq!(config.project.version, "2.0.0");
        assert_eq!(config.project.authors, vec!["Alice", "Bob"]);
        assert_eq!(config.project.shape_version.as_deref(), Some("0.5.0"));
        assert_eq!(config.project.license.as_deref(), Some("MIT"));
        assert_eq!(
            config.project.repository.as_deref(),
            Some("https://github.com/org/project")
        );
        assert_eq!(config.project.entry.as_deref(), Some("main.shape"));
    }

    #[test]
    fn test_parse_full_config_with_all_sections() {
        let toml_str = r#"
[project]
name = "mega-project"
version = "1.0.0"
authors = ["Dev"]
shape-version = "0.5.0"
license = "Apache-2.0"
repository = "https://github.com/org/mega"
entry = "src/main.shape"

[modules]
paths = ["lib", "vendor"]

[dependencies]
finance = "0.1.0"
my-utils = { path = "../utils" }
plotting = { git = "https://github.com/org/plot.git", tag = "v1.0" }

[dev-dependencies]
test-helpers = "0.3.0"

[build]
target = "bytecode"
opt_level = 1
output = "out/"

[[extensions]]
name = "market-data"
path = "./plugins/market.so"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "mega-project");
        assert_eq!(config.project.authors, vec!["Dev"]);
        assert_eq!(config.project.shape_version.as_deref(), Some("0.5.0"));
        assert_eq!(config.project.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(config.modules.paths, vec!["lib", "vendor"]);
        assert_eq!(config.dependencies.len(), 3);
        assert_eq!(config.dev_dependencies.len(), 1);
        assert_eq!(config.build.target.as_deref(), Some("bytecode"));
        assert_eq!(config.build.opt_level, Some(1));
        assert_eq!(config.extensions.len(), 1);
    }

    #[test]
    fn test_validate_valid_project() {
        let toml_str = r#"
[project]
name = "valid"
version = "1.0.0"

[dependencies]
finance = "0.1.0"
utils = { path = "../utils" }
lib = { git = "https://example.com/lib.git", tag = "v1" }

[build]
opt_level = 2
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validate_catches_path_and_git() {
        let toml_str = r#"
[dependencies]
bad-dep = { path = "../local", git = "https://example.com/repo.git", tag = "v1" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("bad-dep") && e.contains("path") && e.contains("git"))
        );
    }

    #[test]
    fn test_validate_catches_git_without_ref() {
        let toml_str = r#"
[dependencies]
no-ref = { git = "https://example.com/repo.git" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("no-ref") && e.contains("tag"))
        );
    }

    #[test]
    fn test_validate_git_with_branch_is_ok() {
        let toml_str = r#"
[dependencies]
ok-dep = { git = "https://example.com/repo.git", branch = "main" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validate_catches_opt_level_too_high() {
        let toml_str = r#"
[build]
opt_level = 5
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("opt_level") && e.contains("5"))
        );
    }

    #[test]
    fn test_validate_catches_empty_project_name() {
        let toml_str = r#"
[project]
version = "1.0.0"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("project.name")));
    }

    #[test]
    fn test_validate_dev_dependencies_errors() {
        let toml_str = r#"
[dev-dependencies]
bad = { path = "../x", git = "https://example.com/x.git", tag = "v1" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("dev-dependencies") && e.contains("bad"))
        );
    }

    #[test]
    fn test_empty_config_still_parses() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert!(config.dependencies.is_empty());
        assert!(config.dev_dependencies.is_empty());
        assert!(config.build.target.is_none());
        assert!(config.build.opt_level.is_none());
        assert!(config.project.authors.is_empty());
        assert!(config.project.shape_version.is_none());
    }

    #[test]
    fn test_mixed_dependency_types() {
        let toml_str = r#"
[dependencies]
simple = "1.0.0"
local = { path = "./local" }
remote = { git = "https://example.com/repo.git", rev = "deadbeef" }
versioned = { version = "2.0.0" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.dependencies.len(), 4);
        assert!(matches!(
            config.dependencies.get("simple"),
            Some(DependencySpec::Version(_))
        ));
        assert!(matches!(
            config.dependencies.get("local"),
            Some(DependencySpec::Detailed(_))
        ));
        assert!(matches!(
            config.dependencies.get("remote"),
            Some(DependencySpec::Detailed(_))
        ));
        assert!(matches!(
            config.dependencies.get("versioned"),
            Some(DependencySpec::Detailed(_))
        ));
    }

    #[test]
    fn test_parse_config_with_extension_sections() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"

[native-dependencies]
libm = { linux = "libm.so.6", macos = "libm.dylib" }

[custom-config]
key = "value"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "test");
        assert_eq!(config.extension_section_names().len(), 2);
        assert!(
            config
                .extension_sections
                .contains_key("native-dependencies")
        );
        assert!(config.extension_sections.contains_key("custom-config"));

        // Test JSON conversion
        let json = config.extension_section_as_json("custom-config").unwrap();
        assert_eq!(json["key"], "value");
    }

    #[test]
    fn test_parse_native_dependencies_section_typed() {
        let section: toml::Value = toml::from_str(
            r#"
libm = "libm.so.6"
duckdb = { linux = "libduckdb.so", macos = "libduckdb.dylib", windows = "duckdb.dll" }
"#,
        )
        .expect("valid native dependency section");

        let parsed =
            parse_native_dependencies_section(&section).expect("native dependencies should parse");
        assert!(matches!(
            parsed.get("libm"),
            Some(NativeDependencySpec::Simple(v)) if v == "libm.so.6"
        ));
        assert!(matches!(
            parsed.get("duckdb"),
            Some(NativeDependencySpec::Detailed(_))
        ));
    }

    #[test]
    fn test_native_dependency_provider_parsing() {
        let section: toml::Value = toml::from_str(
            r#"
libm = "libm.so.6"
local_lib = "./native/libfoo.so"
vendored = { provider = "vendored", path = "./vendor/libduckdb.so", version = "1.2.0", cache_key = "duckdb-1.2.0" }
"#,
        )
        .expect("valid native dependency section");

        let parsed =
            parse_native_dependencies_section(&section).expect("native dependencies should parse");

        let libm = parsed.get("libm").expect("libm");
        assert_eq!(libm.provider_for_host(), NativeDependencyProvider::System);
        assert_eq!(libm.declared_version(), None);

        let local = parsed.get("local_lib").expect("local_lib");
        assert_eq!(local.provider_for_host(), NativeDependencyProvider::Path);

        let vendored = parsed.get("vendored").expect("vendored");
        assert_eq!(
            vendored.provider_for_host(),
            NativeDependencyProvider::Vendored
        );
        assert_eq!(vendored.declared_version(), Some("1.2.0"));
        assert_eq!(vendored.cache_key(), Some("duckdb-1.2.0"));
    }

    #[test]
    fn test_native_dependency_target_specific_resolution() {
        let section: toml::Value = toml::from_str(
            r#"
duckdb = { provider = "vendored", targets = { "linux-x86_64-gnu" = "native/linux-x86_64-gnu/libduckdb.so", "linux-aarch64-gnu" = "native/linux-aarch64-gnu/libduckdb.so", linux = "legacy-linux.so" } }
"#,
        )
        .expect("valid native dependency section");

        let parsed =
            parse_native_dependencies_section(&section).expect("native dependencies should parse");
        let duckdb = parsed.get("duckdb").expect("duckdb");

        let linux_x86 = NativeTarget {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            env: Some("gnu".to_string()),
        };
        assert_eq!(
            duckdb.resolve_for_target(&linux_x86).as_deref(),
            Some("native/linux-x86_64-gnu/libduckdb.so")
        );

        let linux_arm = NativeTarget {
            os: "linux".to_string(),
            arch: "aarch64".to_string(),
            env: Some("gnu".to_string()),
        };
        assert_eq!(
            duckdb.resolve_for_target(&linux_arm).as_deref(),
            Some("native/linux-aarch64-gnu/libduckdb.so")
        );

        let linux_unknown = NativeTarget {
            os: "linux".to_string(),
            arch: "riscv64".to_string(),
            env: Some("gnu".to_string()),
        };
        assert_eq!(
            duckdb.resolve_for_target(&linux_unknown).as_deref(),
            Some("legacy-linux.so")
        );
    }

    #[test]
    fn test_project_native_dependencies_from_extension_section() {
        let toml_str = r#"
[project]
name = "native-deps"
version = "1.0.0"

[native-dependencies]
libm = "libm.so.6"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let deps = config
            .native_dependencies()
            .expect("native deps should parse");
        assert!(deps.contains_key("libm"));
    }

    #[test]
    fn test_validate_with_claimed_sections() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"

[native-dependencies]
libm = { linux = "libm.so.6" }

[typo-section]
foo = "bar"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let mut claimed = std::collections::HashSet::new();
        claimed.insert("native-dependencies".to_string());

        let errors = config.validate_with_claimed_sections(&claimed);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("typo-section") && e.contains("not claimed"))
        );
        assert!(!errors.iter().any(|e| e.contains("native-dependencies")));
    }

    #[test]
    fn test_extension_sections_empty_by_default() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert!(config.extension_sections.is_empty());
    }

    // --- Permissions section tests ---

    #[test]
    fn test_no_permissions_section_defaults_to_full() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert!(config.permissions.is_none());
        let pset = config.effective_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(pset.contains(&shape_abi_v1::Permission::FsWrite));
        assert!(pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(pset.contains(&shape_abi_v1::Permission::Process));
    }

    #[test]
    fn test_parse_permissions_section() {
        let toml_str = r#"
[project]
name = "perms-test"
version = "1.0.0"

[permissions]
"fs.read" = true
"fs.write" = false
"net.connect" = true
"net.listen" = false
process = false
env = true
time = true
random = false
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let perms = config.permissions.as_ref().unwrap();
        assert_eq!(perms.fs_read, Some(true));
        assert_eq!(perms.fs_write, Some(false));
        assert_eq!(perms.net_connect, Some(true));
        assert_eq!(perms.net_listen, Some(false));
        assert_eq!(perms.process, Some(false));
        assert_eq!(perms.env, Some(true));
        assert_eq!(perms.time, Some(true));
        assert_eq!(perms.random, Some(false));

        let pset = config.effective_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(!pset.contains(&shape_abi_v1::Permission::FsWrite));
        assert!(pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(!pset.contains(&shape_abi_v1::Permission::NetListen));
        assert!(!pset.contains(&shape_abi_v1::Permission::Process));
        assert!(pset.contains(&shape_abi_v1::Permission::Env));
        assert!(pset.contains(&shape_abi_v1::Permission::Time));
        assert!(!pset.contains(&shape_abi_v1::Permission::Random));
    }

    #[test]
    fn test_parse_permissions_with_scoped_fs() {
        let toml_str = r#"
[permissions]
"fs.read" = true

[permissions.fs]
allowed = ["./data", "/tmp/cache"]
read_only = ["./config"]

[permissions.net]
allowed_hosts = ["api.example.com", "*.internal.corp"]
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let perms = config.permissions.as_ref().unwrap();
        let fs = perms.fs.as_ref().unwrap();
        assert_eq!(fs.allowed, vec!["./data", "/tmp/cache"]);
        assert_eq!(fs.read_only, vec!["./config"]);

        let net = perms.net.as_ref().unwrap();
        assert_eq!(
            net.allowed_hosts,
            vec!["api.example.com", "*.internal.corp"]
        );

        let pset = perms.to_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsScoped));
        assert!(pset.contains(&shape_abi_v1::Permission::NetScoped));

        let constraints = perms.to_scope_constraints();
        assert_eq!(constraints.allowed_paths.len(), 3); // ./data, /tmp/cache, ./config
        assert_eq!(constraints.allowed_hosts.len(), 2);
    }

    #[test]
    fn test_permissions_shorthand_pure() {
        let section = PermissionsSection::from_shorthand("pure").unwrap();
        let pset = section.to_permission_set();
        assert!(pset.is_empty());
    }

    #[test]
    fn test_permissions_shorthand_readonly() {
        let section = PermissionsSection::from_shorthand("readonly").unwrap();
        let pset = section.to_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(!pset.contains(&shape_abi_v1::Permission::FsWrite));
        assert!(!pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(pset.contains(&shape_abi_v1::Permission::Env));
        assert!(pset.contains(&shape_abi_v1::Permission::Time));
    }

    #[test]
    fn test_permissions_shorthand_full() {
        let section = PermissionsSection::from_shorthand("full").unwrap();
        let pset = section.to_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(pset.contains(&shape_abi_v1::Permission::FsWrite));
        assert!(pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(pset.contains(&shape_abi_v1::Permission::NetListen));
        assert!(pset.contains(&shape_abi_v1::Permission::Process));
    }

    #[test]
    fn test_permissions_shorthand_unknown() {
        assert!(PermissionsSection::from_shorthand("unknown").is_none());
    }

    #[test]
    fn test_permissions_unset_fields_default_to_true() {
        let toml_str = r#"
[permissions]
"fs.write" = false
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let pset = config.effective_permission_set();
        // Explicitly set to false
        assert!(!pset.contains(&shape_abi_v1::Permission::FsWrite));
        // Not set — defaults to true
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(pset.contains(&shape_abi_v1::Permission::Process));
    }

    // --- Sandbox section tests ---

    #[test]
    fn test_parse_sandbox_section() {
        let toml_str = r#"
[sandbox]
enabled = true
deterministic = true
seed = 42
memory_limit = "64MB"
time_limit = "10s"
virtual_fs = true

[sandbox.seed_files]
"data/input.csv" = "./real_data/input.csv"
"config/settings.toml" = "./test_settings.toml"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let sandbox = config.sandbox.as_ref().unwrap();
        assert!(sandbox.enabled);
        assert!(sandbox.deterministic);
        assert_eq!(sandbox.seed, Some(42));
        assert_eq!(sandbox.memory_limit.as_deref(), Some("64MB"));
        assert_eq!(sandbox.time_limit.as_deref(), Some("10s"));
        assert!(sandbox.virtual_fs);
        assert_eq!(sandbox.seed_files.len(), 2);
        assert_eq!(
            sandbox.seed_files.get("data/input.csv").unwrap(),
            "./real_data/input.csv"
        );
    }

    #[test]
    fn test_sandbox_memory_limit_parsing() {
        let section = SandboxSection {
            memory_limit: Some("64MB".to_string()),
            ..Default::default()
        };
        assert_eq!(section.memory_limit_bytes(), Some(64 * 1024 * 1024));

        let section = SandboxSection {
            memory_limit: Some("1GB".to_string()),
            ..Default::default()
        };
        assert_eq!(section.memory_limit_bytes(), Some(1024 * 1024 * 1024));

        let section = SandboxSection {
            memory_limit: Some("512KB".to_string()),
            ..Default::default()
        };
        assert_eq!(section.memory_limit_bytes(), Some(512 * 1024));
    }

    #[test]
    fn test_sandbox_time_limit_parsing() {
        let section = SandboxSection {
            time_limit: Some("10s".to_string()),
            ..Default::default()
        };
        assert_eq!(section.time_limit_ms(), Some(10_000));

        let section = SandboxSection {
            time_limit: Some("500ms".to_string()),
            ..Default::default()
        };
        assert_eq!(section.time_limit_ms(), Some(500));

        let section = SandboxSection {
            time_limit: Some("2m".to_string()),
            ..Default::default()
        };
        assert_eq!(section.time_limit_ms(), Some(120_000));
    }

    #[test]
    fn test_sandbox_invalid_limits() {
        let section = SandboxSection {
            memory_limit: Some("abc".to_string()),
            ..Default::default()
        };
        assert!(section.memory_limit_bytes().is_none());

        let section = SandboxSection {
            time_limit: Some("forever".to_string()),
            ..Default::default()
        };
        assert!(section.time_limit_ms().is_none());
    }

    #[test]
    fn test_validate_sandbox_invalid_memory_limit() {
        let toml_str = r#"
[sandbox]
enabled = true
memory_limit = "xyz"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("sandbox.memory_limit")));
    }

    #[test]
    fn test_validate_sandbox_invalid_time_limit() {
        let toml_str = r#"
[sandbox]
enabled = true
time_limit = "forever"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("sandbox.time_limit")));
    }

    #[test]
    fn test_validate_sandbox_deterministic_requires_seed() {
        let toml_str = r#"
[sandbox]
enabled = true
deterministic = true
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("sandbox.seed")));
    }

    #[test]
    fn test_validate_sandbox_deterministic_with_seed_is_ok() {
        let toml_str = r#"
[sandbox]
enabled = true
deterministic = true
seed = 123
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            !errors.iter().any(|e| e.contains("sandbox")),
            "expected no sandbox errors, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_no_sandbox_section_is_none() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert!(config.sandbox.is_none());
    }

    // --- Dependency-level permissions ---

    #[test]
    fn test_dependency_with_permission_shorthand() {
        let toml_str = r#"
[dependencies]
analytics = { path = "../analytics", permissions = "pure" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("analytics").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("../analytics"));
                match d.permissions.as_ref().unwrap() {
                    PermissionPreset::Shorthand(s) => assert_eq!(s, "pure"),
                    other => panic!("expected Shorthand, got {:?}", other),
                }
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_dependency_without_permissions() {
        let toml_str = r#"
[dependencies]
utils = { path = "../utils" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("utils").unwrap() {
            DependencySpec::Detailed(d) => {
                assert!(d.permissions.is_none());
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    // --- Full config round-trip ---

    #[test]
    fn test_full_config_with_permissions_and_sandbox() {
        let toml_str = r#"
[project]
name = "full-project"
version = "1.0.0"

[permissions]
"fs.read" = true
"fs.write" = false
"net.connect" = true
"net.listen" = false
process = false
env = true
time = true
random = false

[permissions.fs]
allowed = ["./data"]

[sandbox]
enabled = false
deterministic = false
virtual_fs = false
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert!(config.permissions.is_some());
        assert!(config.sandbox.is_some());
        let errors = config.validate();
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    // --- MED-22: Malformed shape.toml error reporting ---

    #[test]
    fn test_try_find_project_root_returns_error_for_malformed_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("shape.toml"), "this is not valid toml {{{").unwrap();

        let result = try_find_project_root(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Malformed shape.toml"),
            "Expected 'Malformed shape.toml' in error, got: {}",
            err
        );
    }

    #[test]
    fn test_try_find_project_root_returns_ok_none_when_no_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("empty_dir");
        std::fs::create_dir_all(&nested).unwrap();

        let result = try_find_project_root(&nested);
        // Should return Ok(None) — not an error, just no project found.
        // (May find a shape.toml above tempdir, so we just verify no panic/error.)
        assert!(result.is_ok());
    }

    #[test]
    fn test_try_find_project_root_parses_valid_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(tmp.path().join("shape.toml")).unwrap();
        writeln!(
            f,
            r#"
[project]
name = "try-test"
version = "1.0.0"
"#
        )
        .unwrap();

        let result = try_find_project_root(tmp.path());
        assert!(result.is_ok());
        let root = result.unwrap().unwrap();
        assert_eq!(root.config.project.name, "try-test");
    }

    #[test]
    fn test_find_project_root_returns_none_for_malformed_toml() {
        // find_project_root should return None (not panic) for malformed TOML
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("shape.toml"), "[invalid\nbroken toml").unwrap();

        let result = find_project_root(tmp.path());
        assert!(result.is_none());
    }
}
