//! Tests for `module_exports.rs`.
//!
//! Phase 1.B (ADR-006 §2.7.4 ruling): the pre-bulldozer fixtures used
//! `register_test_function` / `register_test_function_with_schema` /
//! `module.invoke_export` (all deleted alongside the legacy `ValueWord`
//! ABI). The structural tests that depended on those helpers are
//! removed; the permission / visibility tests that exercise the
//! [`ModuleContext`] surface directly remain.

use super::*;
use crate::marshal::register_typed_fn_0;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};

/// Build a dummy `ModuleContext` for unit tests that don't need schema
/// lookup or callable invocation.
fn test_ctx() -> ModuleContext<'static> {
    let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
    ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: None,
        scope_constraints: None,
        set_pending_resume: None,
        set_pending_frame_resume: None,
    }
}

#[test]
fn test_module_exports_creation() {
    let mut module = ModuleExports::new("test");
    register_typed_fn_0(
        &mut module,
        "hello",
        "Return a greeting",
        ConcreteType::String,
        |_ctx| Ok(TypedReturn::Concrete(ConcreteReturn::String("world".to_string()))),
    );

    assert_eq!(module.name, "test");
    assert!(module.has_export("hello"));
    assert!(!module.has_export("missing"));
}

#[test]
fn test_registry() {
    let mut registry = ModuleExportRegistry::new();

    let mut files_module = ModuleExports::new("files");
    register_typed_fn_0(
        &mut files_module,
        "read",
        "Read a file (test stub)",
        ConcreteType::String,
        |_ctx| Ok(TypedReturn::Concrete(ConcreteReturn::String("loaded".to_string()))),
    );

    registry.register(files_module);

    assert!(registry.has("files"));
    assert!(!registry.has("json"));
    assert_eq!(registry.module_names(), vec!["files"]);

    let files = registry.get("files").unwrap();
    assert!(files.has_export("read"));
}

#[test]
fn test_module_description() {
    let mut module = ModuleExports::new("described");
    assert!(module.description.is_empty());
    module.description = "A module with a description".into();
    assert_eq!(module.description, "A module with a description");
}

#[test]
fn test_add_shape_source() {
    let mut module = ModuleExports::new("test_ext");
    module.add_shape_source("helpers.shape", "fn double(x) { x * 2 }");
    module.add_shape_source("types.shape", "enum Color { Red, Green, Blue }");

    assert_eq!(module.shape_sources.len(), 2);
    assert_eq!(module.shape_sources[0].0, "helpers.shape");
    assert_eq!(module.shape_sources[0].1, "fn double(x) { x * 2 }");
    assert_eq!(module.shape_sources[1].0, "types.shape");
}

#[test]
fn test_new_module_has_empty_extension_fields() {
    let module = ModuleExports::new("empty");
    assert!(module.shape_sources.is_empty());
    assert!(module.method_intrinsics.is_empty());
}

// -- Permission checking tests --

#[test]
fn test_check_permission_allows_when_no_permissions_set() {
    let ctx = test_ctx();
    // When granted_permissions is None, all permissions are allowed
    assert!(check_permission(&ctx, shape_abi_v1::Permission::FsRead).is_ok());
    assert!(check_permission(&ctx, shape_abi_v1::Permission::NetConnect).is_ok());
    assert!(check_permission(&ctx, shape_abi_v1::Permission::Process).is_ok());
}

#[test]
fn test_check_permission_denies_when_not_granted() {
    let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
    let mut perms = shape_abi_v1::PermissionSet::pure();
    perms.insert(shape_abi_v1::Permission::FsRead);
    let ctx = ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: Some(perms),
        scope_constraints: None,
        set_pending_resume: None,
        set_pending_frame_resume: None,
    };
    assert!(check_permission(&ctx, shape_abi_v1::Permission::FsRead).is_ok());
    assert!(check_permission(&ctx, shape_abi_v1::Permission::FsWrite).is_err());
    assert!(check_permission(&ctx, shape_abi_v1::Permission::NetConnect).is_err());
}

#[test]
fn test_check_fs_permission_enforces_scope_constraints() {
    let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
    let mut perms = shape_abi_v1::PermissionSet::pure();
    perms.insert(shape_abi_v1::Permission::FsRead);
    let constraints = shape_abi_v1::ScopeConstraints {
        allowed_paths: vec!["/data/**".to_string(), "/tmp/*".to_string()],
        ..Default::default()
    };
    let ctx = ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: Some(perms),
        scope_constraints: Some(constraints),
        set_pending_resume: None,
        set_pending_frame_resume: None,
    };

    // Allowed paths
    assert!(check_fs_permission(&ctx, shape_abi_v1::Permission::FsRead, "/data/file.txt").is_ok());
    assert!(check_fs_permission(&ctx, shape_abi_v1::Permission::FsRead, "/tmp/scratch").is_ok());

    // Denied paths
    assert!(check_fs_permission(&ctx, shape_abi_v1::Permission::FsRead, "/etc/passwd").is_err());
    assert!(check_fs_permission(&ctx, shape_abi_v1::Permission::FsRead, "/home/user/file").is_err());
}

#[test]
fn test_check_fs_permission_allows_all_when_no_constraints() {
    let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
    let mut perms = shape_abi_v1::PermissionSet::pure();
    perms.insert(shape_abi_v1::Permission::FsRead);
    let ctx = ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: Some(perms),
        scope_constraints: None,
        set_pending_resume: None,
        set_pending_frame_resume: None,
    };

    assert!(check_fs_permission(&ctx, shape_abi_v1::Permission::FsRead, "/any/path").is_ok());
}

#[test]
fn test_check_net_permission_enforces_scope_constraints() {
    let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
    let mut perms = shape_abi_v1::PermissionSet::pure();
    perms.insert(shape_abi_v1::Permission::NetConnect);
    let constraints = shape_abi_v1::ScopeConstraints {
        allowed_hosts: vec!["api.example.com".to_string(), "*.trusted.io".to_string()],
        ..Default::default()
    };
    let ctx = ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: Some(perms),
        scope_constraints: Some(constraints),
        set_pending_resume: None,
        set_pending_frame_resume: None,
    };

    // Allowed hosts
    assert!(
        check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "api.example.com:443")
            .is_ok()
    );
    assert!(
        check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "sub.trusted.io:8080")
            .is_ok()
    );

    // Denied hosts
    assert!(
        check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "evil.com:80").is_err()
    );
    assert!(
        check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "other.example.com:443")
            .is_err()
    );
}

#[test]
fn test_check_net_permission_allows_all_when_no_constraints() {
    let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
    let mut perms = shape_abi_v1::PermissionSet::pure();
    perms.insert(shape_abi_v1::Permission::NetConnect);
    let ctx = ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: Some(perms),
        scope_constraints: None,
        set_pending_resume: None,
        set_pending_frame_resume: None,
    };

    assert!(
        check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "any.host.com:8080")
            .is_ok()
    );
}

#[test]
fn test_all_stdlib_modules_populated() {
    let modules = crate::stdlib::all_stdlib_modules();
    // Should have at least 18 modules (all shape-runtime ones)
    assert!(
        modules.len() >= 18,
        "expected at least 18 stdlib modules, got {}",
        modules.len()
    );
    // All should have canonical names
    for m in &modules {
        assert!(
            m.name.starts_with("std::core::"),
            "module '{}' should have canonical name starting with 'std::core::'",
            m.name
        );
    }
}
