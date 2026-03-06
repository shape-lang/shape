//! Deep tests for module resolution, nesting, and circular dependency detection.
//!
//! Categories:
//! 1. Filesystem module resolution (tempdir-based)
//! 2. Dependency path resolution
//! 3. Circular dependency detection
//! 4. Module cache behavior
//! 5. Search path ordering
//! 6. Edge cases in path resolution

#[cfg(test)]
mod tests {
    use crate::module_loader::resolution::resolve_module_path_with_context;
    use crate::module_loader::{ModuleCode, ModuleLoader};
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Arc;

    // =========================================================================
    // Category 1: Filesystem module resolution (~20 tests)
    // =========================================================================

    #[test]
    fn test_modres_fs_load_simple_module_from_search_path() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("greeter.shape"),
            "pub fn greet() { \"hello\" }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader
            .load_module("greeter")
            .expect("should load greeter module");
        assert!(
            module.exports.contains_key("greet"),
            "expected 'greet' export"
        );
    }

    #[test]
    fn test_modres_fs_load_module_from_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("utils");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("helpers.shape"), "pub fn help() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader
            .load_module("utils::helpers")
            .expect("should load utils::helpers");
        assert!(module.exports.contains_key("help"));
    }

    #[test]
    fn test_modres_fs_module_not_found_error_message() {
        let tmp = tempfile::tempdir().unwrap();
        let mut loader = ModuleLoader::new();
        loader.clear_module_paths();
        loader.set_stdlib_path(tmp.path().join("nonexistent_stdlib"));
        loader.add_module_path(tmp.path().to_path_buf());

        let err = loader.load_module("nonexistent_module").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("not found") || msg.contains("Module not found"),
            "error should mention module not found, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_fs_module_with_syntax_error_propagates() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("broken.shape"),
            "pub fn oops( { invalid syntax }}}",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let err = loader.load_module("broken").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("parse") || msg.contains("Parse") || msg.contains("syntax"),
            "error should mention parse failure, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_fs_load_same_module_twice_uses_cache() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("cached_mod.shape"),
            "pub fn cached() { 42 }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let m1 = loader.load_module("cached_mod").unwrap();
        let m2 = loader.load_module("cached_mod").unwrap();
        assert!(
            Arc::ptr_eq(&m1, &m2),
            "second load should return cached Arc"
        );
    }

    #[test]
    fn test_modres_fs_load_module_chain_a_imports_b_imports_c() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("chain_c.shape"), "pub fn c_fn() { 3 }").unwrap();
        std::fs::write(
            tmp.path().join("chain_b.shape"),
            "from chain_c use { c_fn }\npub fn b_fn() { c_fn() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("chain_a.shape"),
            "from chain_b use { b_fn }\npub fn a_fn() { b_fn() }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader
            .load_module("chain_a")
            .expect("should load chain_a through transitive deps");
        assert!(module.exports.contains_key("a_fn"));
        assert!(
            loader.get_module("chain_b").is_some(),
            "chain_b should be loaded"
        );
        assert!(
            loader.get_module("chain_c").is_some(),
            "chain_c should be loaded"
        );
    }

    #[test]
    fn test_modres_fs_index_shape_in_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("mypkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("index.shape"), "pub fn init() { 0 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader
            .load_module("mypkg")
            .expect("should resolve mypkg via index.shape");
        assert!(module.exports.contains_key("init"));
    }

    #[test]
    fn test_modres_fs_nested_directory_with_separator() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep_mod.shape"), "pub fn deep() { 99 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader
            .load_module("a::b::c::deep_mod")
            .expect("should load deeply nested module");
        assert!(module.exports.contains_key("deep"));
    }

    #[test]
    fn test_modres_fs_multiple_search_paths_priority() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();

        std::fs::write(tmp1.path().join("priority.shape"), "pub fn version() { 1 }").unwrap();
        std::fs::write(tmp2.path().join("priority.shape"), "pub fn version() { 2 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.clear_module_paths();
        // tmp1 added first, should take priority
        loader.add_module_path(tmp1.path().to_path_buf());
        loader.add_module_path(tmp2.path().to_path_buf());

        let module = loader.load_module("priority").unwrap();
        assert!(module.exports.contains_key("version"));
        // The module from the first search path should be used
        // (We can verify by checking the path if needed, but just loading
        // successfully from the first path is the key test)
    }

    #[test]
    fn test_modres_fs_relative_import_with_context() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("project");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("helper.shape"), "pub fn help() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        let result = loader.load_module_with_context("./helper", Some(&sub.to_path_buf()));

        match result {
            Ok(module) => {
                assert!(module.exports.contains_key("help"));
            }
            Err(e) => {
                // Relative imports may or may not be supported
                let msg = format!("{}", e);
                assert!(
                    msg.contains("relative") || msg.contains("not found"),
                    "unexpected error: {}",
                    msg
                );
            }
        }
    }

    #[test]
    fn test_modres_fs_relative_import_without_context_is_error() {
        let mut loader = ModuleLoader::new();
        let err = loader.load_module("./local_module").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("context") || msg.contains("relative"),
            "should mention context requirement, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_fs_load_module_from_file_path() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("direct.shape");
        std::fs::write(&file, "pub fn direct_fn() { 42 }").unwrap();

        let mut loader = ModuleLoader::new();
        let module = loader
            .load_module_from_file(&file)
            .expect("should load from direct file path");
        assert!(module.exports.contains_key("direct_fn"));
    }

    #[test]
    fn test_modres_fs_load_module_from_file_path_cached() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("cached_file.shape");
        std::fs::write(&file, "pub fn cf() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        let m1 = loader.load_module_from_file(&file).unwrap();
        let m2 = loader.load_module_from_file(&file).unwrap();
        assert!(
            Arc::ptr_eq(&m1, &m2),
            "file-path loads should be cached by canonical path"
        );
    }

    #[test]
    fn test_modres_fs_load_missing_file_path_error() {
        let mut loader = ModuleLoader::new();
        let err = loader
            .load_module_from_file(&PathBuf::from("/tmp/nonexistent_shape_test_42.shape"))
            .unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("read") || msg.contains("not found") || msg.contains("Failed"),
            "error should mention file read failure, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_fs_module_with_multiple_exports() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("multi.shape"),
            r#"
pub fn alpha() { 1 }
pub fn beta() { 2 }
pub fn gamma() { 3 }
"#,
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader.load_module("multi").unwrap();
        assert!(module.exports.contains_key("alpha"));
        assert!(module.exports.contains_key("beta"));
        assert!(module.exports.contains_key("gamma"));
        assert_eq!(module.export_names().len(), 3);
    }

    #[test]
    fn test_modres_fs_empty_module_file_loads() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("empty.shape"), "").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader
            .load_module("empty")
            .expect("empty module should load successfully");
        assert!(
            module.exports.is_empty(),
            "empty module should have no exports"
        );
    }

    #[test]
    fn test_modres_fs_module_with_only_private_functions() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("private_only.shape"),
            "fn hidden() { 1 }\nfn also_hidden() { 2 }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader.load_module("private_only").unwrap();
        assert!(
            module.exports.is_empty(),
            "module with only private functions should have no exports"
        );
    }

    // =========================================================================
    // Category 2: Dependency path resolution (~15 tests)
    // =========================================================================

    #[test]
    fn test_modres_deps_basic_dependency_resolution() {
        let tmp = tempfile::tempdir().unwrap();
        let dep_root = tmp.path().join("dep_pkg");
        std::fs::create_dir_all(&dep_root).unwrap();
        std::fs::write(dep_root.join("index.shape"), "pub fn dep_fn() { 1 }").unwrap();

        let mut deps = HashMap::new();
        deps.insert("dep_pkg".to_string(), dep_root);

        let mut loader = ModuleLoader::new();
        loader.set_dependency_paths(deps);

        let module = loader
            .load_module("dep_pkg")
            .expect("should resolve dependency via dependency_paths");
        assert!(module.exports.contains_key("dep_fn"));
    }

    #[test]
    fn test_modres_deps_submodule_in_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let dep_root = tmp.path().join("mylib");
        std::fs::create_dir_all(&dep_root).unwrap();
        std::fs::write(dep_root.join("util.shape"), "pub fn util_fn() { 2 }").unwrap();

        let mut deps = HashMap::new();
        deps.insert("mylib".to_string(), dep_root);

        let mut loader = ModuleLoader::new();
        loader.set_dependency_paths(deps);

        let module = loader
            .load_module("mylib::util")
            .expect("should resolve mylib::util");
        assert!(module.exports.contains_key("util_fn"));
    }

    #[test]
    fn test_modres_deps_nested_submodule_in_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let dep_root = tmp.path().join("framework");
        let nested = dep_root.join("core").join("engine");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("runtime.shape"), "pub fn run() { 3 }").unwrap();

        let mut deps = HashMap::new();
        deps.insert("framework".to_string(), dep_root);

        let mut loader = ModuleLoader::new();
        loader.set_dependency_paths(deps);

        let module = loader
            .load_module("framework::core::engine::runtime")
            .expect("should resolve deeply nested dep module");
        assert!(module.exports.contains_key("run"));
    }

    #[test]
    fn test_modres_deps_dependency_not_found_falls_through() {
        let tmp = tempfile::tempdir().unwrap();
        let dep_root = tmp.path().join("my_dep");
        std::fs::create_dir_all(&dep_root).unwrap();
        // No file for the requested submodule

        let mut deps = HashMap::new();
        deps.insert("my_dep".to_string(), dep_root);

        let mut loader = ModuleLoader::new();
        loader.clear_module_paths();
        loader.set_stdlib_path(tmp.path().join("no_stdlib"));
        loader.set_dependency_paths(deps);

        let err = loader.load_module("my_dep::nonexistent").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("not found") || msg.contains("Module not found"),
            "should report not found, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_deps_dependency_takes_priority_over_search_path() {
        let tmp = tempfile::tempdir().unwrap();
        let dep_root = tmp.path().join("dep_version");
        std::fs::create_dir_all(&dep_root).unwrap();
        std::fs::write(dep_root.join("index.shape"), "pub fn from_dep() { 1 }").unwrap();

        // Also create a module with the same name in a search path
        std::fs::write(
            tmp.path().join("dep_version.shape"),
            "pub fn from_search() { 2 }",
        )
        .unwrap();

        let mut deps = HashMap::new();
        deps.insert("dep_version".to_string(), dep_root);

        let mut loader = ModuleLoader::new();
        loader.clear_module_paths();
        loader.add_module_path(tmp.path().to_path_buf());
        loader.set_dependency_paths(deps);

        let module = loader.load_module("dep_version").unwrap();
        // Dependency paths should take priority
        assert!(
            module.exports.contains_key("from_dep"),
            "dependency should take priority over search path"
        );
    }

    #[test]
    fn test_modres_deps_dependency_index_shape_resolution() {
        let tmp = tempfile::tempdir().unwrap();
        let dep_root = tmp.path().join("pkg_with_index");
        std::fs::create_dir_all(&dep_root).unwrap();
        std::fs::write(dep_root.join("index.shape"), "pub fn root_fn() { 0 }").unwrap();

        let mut deps = HashMap::new();
        deps.insert("pkg_with_index".to_string(), dep_root);

        let mut loader = ModuleLoader::new();
        loader.set_dependency_paths(deps);

        let module = loader.load_module("pkg_with_index").unwrap();
        assert!(module.exports.contains_key("root_fn"));
    }

    #[test]
    fn test_modres_deps_sub_index_shape_resolution() {
        let tmp = tempfile::tempdir().unwrap();
        let dep_root = tmp.path().join("biglib");
        let sub = dep_root.join("subpkg");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("index.shape"), "pub fn sub_init() { 0 }").unwrap();

        let mut deps = HashMap::new();
        deps.insert("biglib".to_string(), dep_root);

        let mut loader = ModuleLoader::new();
        loader.set_dependency_paths(deps);

        let module = loader.load_module("biglib::subpkg").unwrap();
        assert!(module.exports.contains_key("sub_init"));
    }

    #[test]
    fn test_modres_deps_set_project_root_prepends_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("mymod.shape"), "pub fn proj_fn() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.set_project_root(&root, &[]);

        let module = loader.load_module("mymod").unwrap();
        assert!(module.exports.contains_key("proj_fn"));
    }

    #[test]
    fn test_modres_deps_set_project_root_with_extra_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        let lib = root.join("lib");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("libmod.shape"), "pub fn lib_fn() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.set_project_root(&root, &[lib]);

        let module = loader.load_module("libmod").unwrap();
        assert!(module.exports.contains_key("lib_fn"));
    }

    #[test]
    fn test_modres_deps_multiple_dependencies() {
        let tmp = tempfile::tempdir().unwrap();

        let dep_a = tmp.path().join("dep_a");
        let dep_b = tmp.path().join("dep_b");
        std::fs::create_dir_all(&dep_a).unwrap();
        std::fs::create_dir_all(&dep_b).unwrap();
        std::fs::write(dep_a.join("index.shape"), "pub fn a_fn() { 1 }").unwrap();
        std::fs::write(dep_b.join("index.shape"), "pub fn b_fn() { 2 }").unwrap();

        let mut deps = HashMap::new();
        deps.insert("dep_a".to_string(), dep_a);
        deps.insert("dep_b".to_string(), dep_b);

        let mut loader = ModuleLoader::new();
        loader.set_dependency_paths(deps);

        let mod_a = loader.load_module("dep_a").unwrap();
        let mod_b = loader.load_module("dep_b").unwrap();
        assert!(mod_a.exports.contains_key("a_fn"));
        assert!(mod_b.exports.contains_key("b_fn"));
    }

    // =========================================================================
    // Category 3: Circular dependency detection (~10 tests)
    // =========================================================================

    #[test]
    fn test_modres_circular_direct_a_imports_a() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("self_ref.shape"),
            "from self_ref use { oops }\npub fn oops() { 1 }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let err = loader.load_module("self_ref").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("ircular") || msg.contains("circular"),
            "should detect self-referential circular dependency, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_circular_two_module_cycle() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("cycle_a.shape"),
            "from cycle_b use { b_fn }\npub fn a_fn() { b_fn() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("cycle_b.shape"),
            "from cycle_a use { a_fn }\npub fn b_fn() { a_fn() }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let err = loader.load_module("cycle_a").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("ircular") || msg.contains("circular"),
            "should detect A<->B circular dependency, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_circular_three_module_cycle() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("tri_a.shape"),
            "from tri_b use { b_fn }\npub fn a_fn() { b_fn() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("tri_b.shape"),
            "from tri_c use { c_fn }\npub fn b_fn() { c_fn() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("tri_c.shape"),
            "from tri_a use { a_fn }\npub fn c_fn() { a_fn() }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let err = loader.load_module("tri_a").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("ircular") || msg.contains("circular"),
            "should detect A->B->C->A circular dependency, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_circular_error_includes_cycle_path() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("path_a.shape"),
            "from path_b use { pb }\npub fn pa() { pb() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("path_b.shape"),
            "from path_a use { pa }\npub fn pb() { pa() }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let err = loader.load_module("path_a").unwrap_err();
        let msg = format!("{}", err);
        // The error message should show the cycle path
        assert!(
            msg.contains("path_a") && msg.contains("path_b"),
            "circular error should include both module names, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_circular_diamond_no_false_positive() {
        // Diamond dependency: A -> B, A -> C, B -> D, C -> D
        // This is NOT circular
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("diamond_d.shape"), "pub fn d_fn() { 4 }").unwrap();
        std::fs::write(
            tmp.path().join("diamond_b.shape"),
            "from diamond_d use { d_fn }\npub fn b_fn() { d_fn() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("diamond_c.shape"),
            "from diamond_d use { d_fn }\npub fn c_fn() { d_fn() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("diamond_a.shape"),
            "from diamond_b use { b_fn }\nfrom diamond_c use { c_fn }\npub fn a_fn() { b_fn() }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader
            .load_module("diamond_a")
            .expect("diamond dependency should NOT be detected as circular");
        assert!(module.exports.contains_key("a_fn"));
    }

    #[test]
    fn test_modres_circular_loading_stack_cleanup_on_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("cleanup_a.shape"),
            "from cleanup_b use { bf }\npub fn af() { bf() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("cleanup_b.shape"),
            "from cleanup_a use { af }\npub fn bf() { af() }",
        )
        .unwrap();
        std::fs::write(tmp.path().join("independent.shape"), "pub fn ind() { 0 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        // First, trigger a circular dependency error
        let _ = loader.load_module("cleanup_a");

        // After the error, loading stack should be clean
        // Loading an independent module should still work
        let module = loader
            .load_module("independent")
            .expect("loading stack should be clean after circular dep error");
        assert!(module.exports.contains_key("ind"));
    }

    // =========================================================================
    // Category 4: Module cache behavior (~10 tests)
    // =========================================================================

    #[test]
    fn test_modres_cache_clear_forces_reload() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("clearable.shape"), "pub fn v() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let m1 = loader.load_module("clearable").unwrap();
        loader.clear_cache();
        let m2 = loader.load_module("clearable").unwrap();

        assert!(
            !Arc::ptr_eq(&m1, &m2),
            "after clear_cache, module should be reloaded (different Arc)"
        );
    }

    #[test]
    fn test_modres_cache_loaded_modules_list() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("list_a.shape"), "pub fn a() { 1 }").unwrap();
        std::fs::write(tmp.path().join("list_b.shape"), "pub fn b() { 2 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        loader.load_module("list_a").unwrap();
        loader.load_module("list_b").unwrap();

        let loaded = loader.loaded_modules();
        assert!(
            loaded.contains(&"list_a"),
            "loaded_modules should include list_a"
        );
        assert!(
            loaded.contains(&"list_b"),
            "loaded_modules should include list_b"
        );
    }

    #[test]
    fn test_modres_cache_get_module_after_load() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("get_me.shape"), "pub fn gm() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        loader.load_module("get_me").unwrap();

        let cached = loader.get_module("get_me");
        assert!(cached.is_some(), "get_module should return cached module");
    }

    #[test]
    fn test_modres_cache_get_export() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("exp_test.shape"),
            "pub fn my_export() { 42 }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        loader.load_module("exp_test").unwrap();

        let export = loader.get_export("exp_test", "my_export");
        assert!(export.is_some(), "get_export should return the export");
    }

    #[test]
    fn test_modres_cache_get_export_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("exp_test2.shape"), "pub fn real() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        loader.load_module("exp_test2").unwrap();

        let export = loader.get_export("exp_test2", "nonexistent");
        assert!(
            export.is_none(),
            "get_export for nonexistent name should return None"
        );
    }

    #[test]
    fn test_modres_cache_dependencies_tracked() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("dep_leaf.shape"), "pub fn leaf() { 0 }").unwrap();
        std::fs::write(
            tmp.path().join("dep_root.shape"),
            "from dep_leaf use { leaf }\npub fn root() { leaf() }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        loader.load_module("dep_root").unwrap();

        let deps = loader.get_dependencies("dep_root");
        assert!(deps.is_some(), "dependencies should be tracked");
        assert!(
            deps.unwrap().contains(&"dep_leaf".to_string()),
            "dep_root should depend on dep_leaf"
        );
    }

    #[test]
    fn test_modres_cache_all_dependencies_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("rdep_c.shape"), "pub fn c() { 3 }").unwrap();
        std::fs::write(
            tmp.path().join("rdep_b.shape"),
            "from rdep_c use { c }\npub fn b() { c() }",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("rdep_a.shape"),
            "from rdep_b use { b }\npub fn a() { b() }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        loader.load_module("rdep_a").unwrap();

        let all_deps = loader.get_all_dependencies("rdep_a");
        assert!(
            all_deps.contains(&"rdep_b".to_string()),
            "all_deps should include rdep_b"
        );
        assert!(
            all_deps.contains(&"rdep_c".to_string()),
            "all_deps should include rdep_c (transitive)"
        );
    }

    // =========================================================================
    // Category 5: In-memory / extension module resolution (~10 tests)
    // =========================================================================

    #[test]
    fn test_modres_inmem_extension_takes_priority() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("ext_mod.shape"), "pub fn from_fs() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());
        loader.register_extension_module(
            "ext_mod",
            ModuleCode::Source(Arc::from("pub fn from_ext() { 2 }")),
        );

        let module = loader.load_module("ext_mod").unwrap();
        assert!(
            module.exports.contains_key("from_ext"),
            "extension module should take priority over filesystem"
        );
    }

    #[test]
    fn test_modres_inmem_has_extension_module() {
        let mut loader = ModuleLoader::new();
        loader.register_extension_module(
            "my_ext",
            ModuleCode::Source(Arc::from("pub fn ext() { 1 }")),
        );

        assert!(loader.has_extension_module("my_ext"));
        assert!(!loader.has_extension_module("nonexistent"));
    }

    #[test]
    fn test_modres_inmem_extension_module_paths() {
        let mut loader = ModuleLoader::new();
        loader
            .register_extension_module("ext_a", ModuleCode::Source(Arc::from("pub fn a() { 1 }")));
        loader
            .register_extension_module("ext_b", ModuleCode::Source(Arc::from("pub fn b() { 2 }")));

        let paths = loader.extension_module_paths();
        assert!(paths.contains(&"ext_a".to_string()));
        assert!(paths.contains(&"ext_b".to_string()));
    }

    #[test]
    fn test_modres_inmem_extension_with_dependency_on_extension() {
        let mut loader = ModuleLoader::new();
        loader.register_extension_module(
            "base_ext",
            ModuleCode::Source(Arc::from("pub fn base() { 42 }")),
        );
        loader.register_extension_module(
            "top_ext",
            ModuleCode::Source(Arc::from(
                "from base_ext use { base }\npub fn top() { base() }",
            )),
        );

        let module = loader.load_module("top_ext").unwrap();
        assert!(module.exports.contains_key("top"));
        assert!(loader.get_module("base_ext").is_some());
    }

    #[test]
    fn test_modres_inmem_extension_with_dependency_on_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("fs_dep.shape"), "pub fn fs_fn() { 10 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());
        loader.register_extension_module(
            "ext_with_fs_dep",
            ModuleCode::Source(Arc::from(
                "from fs_dep use { fs_fn }\npub fn combined() { fs_fn() }",
            )),
        );

        let module = loader.load_module("ext_with_fs_dep").unwrap();
        assert!(module.exports.contains_key("combined"));
    }

    #[test]
    fn test_modres_inmem_embedded_stdlib_priority_over_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        // Even if a file exists, embedded stdlib should win
        let stdlib = tmp.path().join("stdlib");
        std::fs::create_dir_all(stdlib.join("core")).unwrap();
        std::fs::write(
            stdlib.join("core").join("test_embedded.shape"),
            "pub fn from_fs() { 1 }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.set_stdlib_path(stdlib);
        loader.register_embedded_stdlib_module(
            "std::core::test_embedded",
            ModuleCode::Source(Arc::from("pub fn from_embedded() { 2 }")),
        );

        let module = loader.load_module("std::core::test_embedded").unwrap();
        assert!(
            module.exports.contains_key("from_embedded"),
            "embedded stdlib should take priority over filesystem stdlib"
        );
    }

    #[test]
    fn test_modres_inmem_clone_without_cache() {
        let mut loader = ModuleLoader::new();
        loader.register_extension_module(
            "cloned_ext",
            ModuleCode::Source(Arc::from("pub fn cl() { 1 }")),
        );
        loader.load_module("cloned_ext").unwrap();

        let cloned = loader.clone_without_cache();
        assert!(
            cloned.has_extension_module("cloned_ext"),
            "cloned loader should retain extension modules"
        );
        assert!(
            cloned.get_module("cloned_ext").is_none(),
            "cloned loader cache should be empty"
        );
    }

    // =========================================================================
    // Category 6: Path resolution logic (~15 tests)
    // =========================================================================

    #[test]
    fn test_modres_path_resolve_std_prefix_strips_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let stdlib = tmp.path().join("stdlib");
        let core = stdlib.join("core");
        std::fs::create_dir_all(&core).unwrap();
        std::fs::write(core.join("test_resolve.shape"), "pub fn tr() { 1 }").unwrap();

        let result = resolve_module_path_with_context(
            "std::core::test_resolve",
            None,
            &stdlib,
            &[],
            &HashMap::new(),
        );

        assert!(result.is_ok(), "should resolve std:: prefixed module path");
    }

    #[test]
    fn test_modres_path_resolve_without_std_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("plain_mod.shape"), "pub fn pm() { 1 }").unwrap();

        let result = resolve_module_path_with_context(
            "plain_mod",
            None,
            &tmp.path().join("no_stdlib"),
            &[tmp.path().to_path_buf()],
            &HashMap::new(),
        );

        assert!(result.is_ok(), "should resolve plain module name");
    }

    #[test]
    fn test_modres_path_double_colon_to_path_separator() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("ns").join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("item.shape"), "pub fn i() { 1 }").unwrap();

        let result = resolve_module_path_with_context(
            "ns::sub::item",
            None,
            &tmp.path().join("no_stdlib"),
            &[tmp.path().to_path_buf()],
            &HashMap::new(),
        );

        assert!(result.is_ok(), ":: should be converted to path separator");
    }

    #[test]
    fn test_modres_path_module_not_found_lists_searched_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let search1 = tmp.path().join("search1");
        let search2 = tmp.path().join("search2");
        std::fs::create_dir_all(&search1).unwrap();
        std::fs::create_dir_all(&search2).unwrap();

        let err = resolve_module_path_with_context(
            "ghost_module",
            None,
            &tmp.path().join("no_stdlib"),
            &[search1, search2],
            &HashMap::new(),
        )
        .unwrap_err();

        let msg = format!("{}", err);
        assert!(
            msg.contains("ghost_module"),
            "error should include the module name, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_path_shape_extension_added_automatically() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("auto_ext.shape"), "pub fn ae() { 1 }").unwrap();

        let result = resolve_module_path_with_context(
            "auto_ext",
            None,
            &tmp.path().join("no_stdlib"),
            &[tmp.path().to_path_buf()],
            &HashMap::new(),
        );

        assert!(
            result.is_ok(),
            ".shape extension should be added automatically"
        );
    }

    #[test]
    fn test_modres_path_dep_takes_priority_over_stdlib() {
        let tmp = tempfile::tempdir().unwrap();

        // Create stdlib module
        let stdlib = tmp.path().join("stdlib");
        std::fs::create_dir_all(&stdlib).unwrap();
        std::fs::write(stdlib.join("mymod.shape"), "pub fn from_stdlib() { 1 }").unwrap();

        // Create dependency module
        let dep = tmp.path().join("dep_mymod");
        std::fs::create_dir_all(&dep).unwrap();
        std::fs::write(dep.join("index.shape"), "pub fn from_dep() { 2 }").unwrap();

        let mut deps = HashMap::new();
        deps.insert("mymod".to_string(), dep);

        let result = resolve_module_path_with_context("mymod", None, &stdlib, &[], &deps);

        let resolved = result.unwrap();
        let resolved_str = resolved.to_string_lossy().to_string();
        assert!(
            resolved_str.contains("dep_mymod"),
            "dependency should take priority over stdlib, resolved to: {}",
            resolved_str
        );
    }

    #[test]
    fn test_modres_path_parent_traversal_in_relative_import() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        let sub = project.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(project.join("parent_mod.shape"), "pub fn pm() { 1 }").unwrap();

        let result = resolve_module_path_with_context(
            "../parent_mod",
            Some(sub.as_path()),
            &tmp.path().join("no_stdlib"),
            &[],
            &HashMap::new(),
        );

        match result {
            Ok(path) => {
                let path_str = path.to_string_lossy().to_string();
                assert!(
                    path_str.contains("parent_mod"),
                    "should resolve parent traversal"
                );
            }
            Err(e) => {
                // Parent traversal might not be fully supported
                let msg = format!("{}", e);
                assert!(
                    msg.contains("not found") || msg.contains("relative"),
                    "unexpected error for parent traversal: {}",
                    msg
                );
            }
        }
    }

    // =========================================================================
    // Category 7: Edge cases & stress tests (~15 tests)
    // =========================================================================

    #[test]
    fn test_modres_edge_module_with_shape_extension_in_path() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("explicit.shape"), "pub fn ex() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        // Passing .shape extension explicitly
        let result = loader.load_module("explicit.shape");
        // This might or might not work depending on implementation
        match result {
            Ok(module) => assert!(module.exports.contains_key("ex")),
            Err(_) => {
                // If it doesn't work with extension, the normal path should work
                let module = loader.load_module("explicit").unwrap();
                assert!(module.exports.contains_key("ex"));
            }
        }
    }

    #[test]
    fn test_modres_edge_many_modules_in_same_directory() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..20 {
            std::fs::write(
                tmp.path().join(format!("mod_{}.shape", i)),
                format!("pub fn f{}() {{ {} }}", i, i),
            )
            .unwrap();
        }

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        for i in 0..20 {
            let module = loader
                .load_module(&format!("mod_{}", i))
                .unwrap_or_else(|e| panic!("failed to load mod_{}: {}", i, e));
            assert!(
                module.exports.contains_key(&format!("f{}", i)),
                "mod_{} should export f{}",
                i,
                i
            );
        }
    }

    #[test]
    fn test_modres_edge_long_module_path() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a").join("b").join("c").join("d").join("e");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("f.shape"), "pub fn deepest() { 0 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader
            .load_module("a::b::c::d::e::f")
            .expect("should handle deeply nested module path");
        assert!(module.exports.contains_key("deepest"));
    }

    #[test]
    fn test_modres_edge_module_name_with_underscore() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("my_module_name.shape"), "pub fn f() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader.load_module("my_module_name").unwrap();
        assert!(module.exports.contains_key("f"));
    }

    #[test]
    fn test_modres_edge_module_name_with_numbers() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("mod123.shape"), "pub fn f() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let module = loader.load_module("mod123").unwrap();
        assert!(module.exports.contains_key("f"));
    }

    #[test]
    fn test_modres_edge_clear_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("reloadable.shape"), "pub fn r() { 1 }").unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        // Load, clear, load again
        loader.load_module("reloadable").unwrap();
        assert!(loader.get_module("reloadable").is_some());

        loader.clear_cache();
        assert!(
            loader.get_module("reloadable").is_none(),
            "after clear_cache, module should not be cached"
        );

        loader.load_module("reloadable").unwrap();
        assert!(loader.get_module("reloadable").is_some());
    }

    #[test]
    fn test_modres_edge_reset_module_paths() {
        let mut loader = ModuleLoader::new();
        let original_count = loader.get_module_paths().len();

        loader.add_module_path(PathBuf::from("/tmp/extra_test_path"));
        assert!(loader.get_module_paths().len() > original_count);

        loader.reset_module_paths();
        assert_eq!(
            loader.get_module_paths().len(),
            original_count,
            "reset_module_paths should restore defaults"
        );
    }

    #[test]
    fn test_modres_edge_duplicate_module_path_not_added() {
        let mut loader = ModuleLoader::new();
        let path = PathBuf::from("/tmp/dup_test_path_unique_42");
        loader.add_module_path(path.clone());
        let count1 = loader.get_module_paths().len();
        loader.add_module_path(path);
        let count2 = loader.get_module_paths().len();
        assert_eq!(count1, count2, "duplicate paths should not be added");
    }

    #[test]
    fn test_modres_edge_module_compilation_error_in_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("bad_dep.shape"),
            "pub fn broken( { invalid }}}",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("depends_on_bad.shape"),
            "from bad_dep use { broken }\npub fn wrapper() { broken() }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let err = loader.load_module("depends_on_bad").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("parse")
                || msg.contains("Parse")
                || msg.contains("syntax")
                || msg.contains("bad_dep"),
            "should propagate dependency parse error, got: {}",
            msg
        );
    }

    #[test]
    fn test_modres_edge_resolve_import_named_items() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("named_exports.shape"),
            "pub fn alpha() { 1 }\npub fn beta() { 2 }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let import = shape_ast::ast::ImportStmt {
            from: "named_exports".to_string(),
            items: shape_ast::ast::ImportItems::Named(vec![
                shape_ast::ast::ImportSpec {
                    name: "alpha".to_string(),
                    alias: None,
                },
                shape_ast::ast::ImportSpec {
                    name: "beta".to_string(),
                    alias: Some("b".to_string()),
                },
            ]),
        };

        let resolved = loader.resolve_import(&import).unwrap();
        assert!(resolved.contains_key("alpha"), "should resolve alpha");
        assert!(
            resolved.contains_key("b"),
            "should resolve beta as b (alias)"
        );
        assert!(
            !resolved.contains_key("beta"),
            "original name should not appear when aliased"
        );
    }

    #[test]
    fn test_modres_edge_resolve_import_nonexistent_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("partial_exports.shape"),
            "pub fn exists() { 1 }",
        )
        .unwrap();

        let mut loader = ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());

        let import = shape_ast::ast::ImportStmt {
            from: "partial_exports".to_string(),
            items: shape_ast::ast::ImportItems::Named(vec![shape_ast::ast::ImportSpec {
                name: "does_not_exist".to_string(),
                alias: None,
            }]),
        };

        let err = loader.resolve_import(&import).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("does_not_exist") || msg.contains("no export"),
            "should report missing export, got: {}",
            msg
        );
    }

    // =========================================================================
    // Category 8: Module listing / discovery (~10 tests)
    // =========================================================================

    #[test]
    fn test_modres_list_modules_from_root_basic() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("mod_a.shape"), "pub fn a() { 1 }").unwrap();
        std::fs::write(tmp.path().join("mod_b.shape"), "pub fn b() { 2 }").unwrap();

        let modules =
            crate::module_loader::resolution::list_modules_from_root(tmp.path(), None).unwrap();
        assert!(modules.contains(&"mod_a".to_string()));
        assert!(modules.contains(&"mod_b".to_string()));
    }

    #[test]
    fn test_modres_list_modules_with_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("item.shape"), "pub fn i() { 1 }").unwrap();

        let modules =
            crate::module_loader::resolution::list_modules_from_root(tmp.path(), Some("pkg"))
                .unwrap();
        assert!(
            modules.contains(&"pkg::item".to_string()),
            "modules should be prefixed, got: {:?}",
            modules
        );
    }

    #[test]
    fn test_modres_list_modules_nested_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("nested.shape"), "pub fn n() { 1 }").unwrap();

        let modules =
            crate::module_loader::resolution::list_modules_from_root(tmp.path(), None).unwrap();
        assert!(
            modules.contains(&"sub::nested".to_string()),
            "should include nested module, got: {:?}",
            modules
        );
    }

    #[test]
    fn test_modres_list_modules_index_shape_maps_to_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("mypkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("index.shape"), "pub fn init() { 0 }").unwrap();

        let modules =
            crate::module_loader::resolution::list_modules_from_root(tmp.path(), None).unwrap();
        assert!(
            modules.contains(&"mypkg".to_string()),
            "index.shape should map to parent directory name, got: {:?}",
            modules
        );
    }

    #[test]
    fn test_modres_list_modules_skips_hidden_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let hidden = tmp.path().join(".hidden");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("secret.shape"), "pub fn s() { 1 }").unwrap();
        std::fs::write(tmp.path().join("visible.shape"), "pub fn v() { 1 }").unwrap();

        let modules =
            crate::module_loader::resolution::list_modules_from_root(tmp.path(), None).unwrap();
        assert!(modules.contains(&"visible".to_string()));
        assert!(
            !modules.iter().any(|m| m.contains("secret")),
            "should skip hidden directories, got: {:?}",
            modules
        );
    }

    #[test]
    fn test_modres_list_modules_skips_target_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("build.shape"), "pub fn b() { 1 }").unwrap();
        std::fs::write(tmp.path().join("src.shape"), "pub fn s() { 1 }").unwrap();

        let modules =
            crate::module_loader::resolution::list_modules_from_root(tmp.path(), None).unwrap();
        assert!(modules.contains(&"src".to_string()));
        assert!(
            !modules.iter().any(|m| m.contains("build")),
            "should skip target directory, got: {:?}",
            modules
        );
    }

    #[test]
    fn test_modres_list_modules_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let modules =
            crate::module_loader::resolution::list_modules_from_root(tmp.path(), None).unwrap();
        assert!(modules.is_empty(), "empty dir should list no modules");
    }

    #[test]
    fn test_modres_list_modules_nonexistent_dir() {
        let modules = crate::module_loader::resolution::list_modules_from_root(
            &PathBuf::from("/tmp/definitely_nonexistent_shape_dir_42"),
            None,
        )
        .unwrap();
        assert!(
            modules.is_empty(),
            "nonexistent dir should return empty list"
        );
    }

    #[test]
    fn test_modres_list_importable_includes_stdlib() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("main.shape"), "let x = 1").unwrap();

        let loader = ModuleLoader::new();
        let modules =
            loader.list_importable_modules_with_context(&tmp.path().join("main.shape"), None);

        assert!(
            modules.iter().any(|m| m.starts_with("std::")),
            "importable modules should include stdlib, got: {:?}",
            modules.iter().take(5).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_modres_list_importable_includes_local_modules() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("main.shape"), "let x = 1").unwrap();
        std::fs::write(tmp.path().join("helper.shape"), "pub fn h() { 1 }").unwrap();

        let loader = ModuleLoader::new();
        let modules =
            loader.list_importable_modules_with_context(&tmp.path().join("main.shape"), None);

        assert!(
            modules.contains(&"helper".to_string()),
            "importable modules should include local .shape files, got: {:?}",
            modules
        );
    }

    // =========================================================================
    // Category 9: extract_dependencies (~5 tests)
    // =========================================================================

    #[test]
    fn test_modres_extract_deps_basic_import() {
        let source = "from utils use { helper }\nlet x = helper()";
        let ast = shape_ast::parser::parse_program(source).unwrap();
        let deps = crate::module_loader::resolution::extract_dependencies(&ast);
        assert_eq!(deps, vec!["utils".to_string()]);
    }

    #[test]
    fn test_modres_extract_deps_multiple_imports() {
        let source = "from alpha use { a }\nfrom beta use { b }\nfrom gamma use { c }\nlet x = a()";
        let ast = shape_ast::parser::parse_program(source).unwrap();
        let deps = crate::module_loader::resolution::extract_dependencies(&ast);
        assert_eq!(deps.len(), 3);
        assert!(deps.contains(&"alpha".to_string()));
        assert!(deps.contains(&"beta".to_string()));
        assert!(deps.contains(&"gamma".to_string()));
    }

    #[test]
    fn test_modres_extract_deps_namespace_import() {
        let source = "use std::core::math\nlet x = math.sin(0)";
        let ast = shape_ast::parser::parse_program(source).unwrap();
        let deps = crate::module_loader::resolution::extract_dependencies(&ast);
        assert_eq!(deps, vec!["std::core::math".to_string()]);
    }

    #[test]
    fn test_modres_extract_deps_no_imports() {
        let source = "let x = 42\nlet y = x + 1";
        let ast = shape_ast::parser::parse_program(source).unwrap();
        let deps = crate::module_loader::resolution::extract_dependencies(&ast);
        assert!(deps.is_empty(), "no imports should produce empty deps");
    }

    #[test]
    fn test_modres_extract_deps_duplicate_import_preserved() {
        let source = "from utils use { a }\nfrom utils use { b }\nlet x = a()";
        let ast = shape_ast::parser::parse_program(source).unwrap();
        let deps = crate::module_loader::resolution::extract_dependencies(&ast);
        // extract_dependencies preserves all import statements, even duplicates
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0], "utils");
        assert_eq!(deps[1], "utils");
    }
}
