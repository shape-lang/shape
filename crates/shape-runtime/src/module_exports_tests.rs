use super::*;

/// Build a dummy `ModuleContext` for unit tests that don't need schema
/// lookup or callable invocation.
fn test_ctx() -> ModuleContext<'static> {
    // Leak a minimal registry so we get a `&'static` reference for tests.
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
    module.add_function("hello", |_args: &[ValueWord], _ctx: &ModuleContext| {
        Ok(ValueWord::from_string(Arc::new("world".to_string())))
    });

    assert_eq!(module.name, "test");
    assert!(module.has_export("hello"));
    assert!(!module.has_export("missing"));
}

#[test]
fn test_module_exports_call() {
    let mut module = ModuleExports::new("math");
    module.add_function("add", |args: &[ValueWord], _ctx: &ModuleContext| {
        let a = args
            .get(0)
            .and_then(|nb| nb.as_number_coerce())
            .unwrap_or(0.0);
        let b = args
            .get(1)
            .and_then(|nb| nb.as_number_coerce())
            .unwrap_or(0.0);
        Ok(ValueWord::from_f64(a + b))
    });

    let ctx = test_ctx();
    let func = module.get_export("add").unwrap();
    let result = func(&[ValueWord::from_f64(3.0), ValueWord::from_f64(4.0)], &ctx).unwrap();
    assert_eq!(result.as_number_coerce(), Some(7.0));
}

#[test]
fn test_registry() {
    let mut registry = ModuleExportRegistry::new();

    let mut files_module = ModuleExports::new("files");
    files_module.add_function("read", |_args: &[ValueWord], _ctx: &ModuleContext| {
        Ok(ValueWord::from_string(Arc::new("loaded".to_string())))
    });

    registry.register(files_module);

    assert!(registry.has("files"));
    assert!(!registry.has("json"));
    assert_eq!(registry.module_names(), vec!["files"]);

    let files = registry.get("files").unwrap();
    assert!(files.has_export("read"));
}

#[test]
fn test_function_schema() {
    let mut module = ModuleExports::new("test");
    module.add_function_with_schema(
        "compute",
        |_args: &[ValueWord], _ctx: &ModuleContext| Ok(ValueWord::from_f64(42.0)),
        ModuleFunction {
            description: "Compute something useful".into(),
            params: vec![
                ModuleParam {
                    name: "input".into(),
                    type_name: "number".into(),
                    required: true,
                    description: "The input value".into(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "scale".into(),
                    type_name: "number".into(),
                    required: false,
                    description: "Optional scale factor".into(),
                    default_snippet: Some("1.0".into()),
                    ..Default::default()
                },
            ],
            return_type: Some("number".into()),
        },
    );

    assert!(module.has_export("compute"));
    let schema = module.get_schema("compute").expect("schema should exist");
    assert_eq!(schema.description, "Compute something useful");
    assert_eq!(schema.params.len(), 2);
    assert_eq!(schema.params[0].name, "input");
    assert_eq!(schema.params[0].type_name, "number");
    assert!(schema.params[0].required);
    assert_eq!(schema.params[1].name, "scale");
    assert!(!schema.params[1].required);
    assert_eq!(schema.params[1].default_snippet.as_deref(), Some("1.0"));
    assert_eq!(schema.return_type.as_deref(), Some("number"));
}

#[test]
fn test_export_names() {
    let mut module = ModuleExports::new("multi");
    module.add_function("alpha", |_args: &[ValueWord], _ctx: &ModuleContext| {
        Ok(ValueWord::none())
    });
    module.add_function("beta", |_args: &[ValueWord], _ctx: &ModuleContext| {
        Ok(ValueWord::none())
    });

    let mut names = module.export_names();
    names.sort();
    assert_eq!(names, vec!["alpha", "beta"]);
}

#[test]
fn test_module_description() {
    let mut module = ModuleExports::new("described");
    assert!(module.description.is_empty());
    module.description = "A module with a description".into();
    assert_eq!(module.description, "A module with a description");
}

#[test]
fn test_module_clone() {
    let mut module = ModuleExports::new("original");
    module.description = "The original module".into();
    module.add_function_with_schema(
        "greet",
        |_args: &[ValueWord], _ctx: &ModuleContext| {
            Ok(ValueWord::from_string(Arc::new("hello".to_string())))
        },
        ModuleFunction {
            description: "Say hello".into(),
            params: vec![ModuleParam {
                name: "name".into(),
                type_name: "string".into(),
                required: true,
                description: "Who to greet".into(),
                ..Default::default()
            }],
            return_type: Some("string".into()),
        },
    );

    let cloned = module.clone();
    assert_eq!(cloned.name, "original");
    assert_eq!(cloned.description, "The original module");
    assert!(cloned.has_export("greet"));

    let mut cloned_names = cloned.export_names();
    cloned_names.sort();
    let mut original_names = module.export_names();
    original_names.sort();
    assert_eq!(cloned_names, original_names);

    let cloned_schema = cloned
        .get_schema("greet")
        .expect("cloned module should have schema for 'greet'");
    assert_eq!(cloned_schema.description, "Say hello");
    assert_eq!(cloned_schema.params.len(), 1);
    assert_eq!(cloned_schema.params[0].name, "name");
    assert_eq!(cloned_schema.return_type.as_deref(), Some("string"));
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
fn test_add_intrinsic() {
    let mut module = ModuleExports::new("test_ext");
    module.add_intrinsic(
        "MyType",
        "fast_method",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let val = args
                .first()
                .and_then(|nb| nb.as_number_coerce())
                .unwrap_or(0.0);
            Ok(ValueWord::from_f64(val * 2.0))
        },
    );
    module.add_intrinsic(
        "MyType",
        "another_method",
        |_args: &[ValueWord], _ctx: &ModuleContext| Ok(ValueWord::from_bool(true)),
    );
    module.add_intrinsic(
        "OtherType",
        "do_thing",
        |_args: &[ValueWord], _ctx: &ModuleContext| Ok(ValueWord::none()),
    );

    assert_eq!(module.method_intrinsics.len(), 2);
    assert!(module.method_intrinsics.contains_key("MyType"));
    assert!(module.method_intrinsics.contains_key("OtherType"));
    assert_eq!(module.method_intrinsics["MyType"].len(), 2);
    assert_eq!(module.method_intrinsics["OtherType"].len(), 1);

    // Verify the intrinsic actually works
    let ctx = test_ctx();
    let fast = &module.method_intrinsics["MyType"]["fast_method"];
    let result = fast(&[ValueWord::from_f64(21.0)], &ctx).unwrap();
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_shape_sources_clone() {
    let mut module = ModuleExports::new("cloneable");
    module.add_shape_source("test.shape", "let x = 1");
    module.add_intrinsic("T", "m", |_: &[ValueWord], _ctx: &ModuleContext| {
        Ok(ValueWord::none())
    });

    let cloned = module.clone();
    assert_eq!(cloned.shape_sources.len(), 1);
    assert_eq!(cloned.shape_sources[0].0, "test.shape");
    assert_eq!(cloned.method_intrinsics.len(), 1);
    assert!(cloned.method_intrinsics["T"].contains_key("m"));
}

#[test]
fn test_new_module_has_empty_extension_fields() {
    let module = ModuleExports::new("empty");
    assert!(module.shape_sources.is_empty());
    assert!(module.method_intrinsics.is_empty());
}

#[test]
fn test_export_visibility_defaults_to_public() {
    let mut module = ModuleExports::new("test");
    module.add_function("ping", |_args, _ctx: &ModuleContext| Ok(ValueWord::unit()));

    assert_eq!(
        module.export_visibility("ping"),
        ModuleExportVisibility::Public
    );
    assert!(module.is_export_available("ping", false));
    assert!(module.is_export_public_surface("ping", false));
}

#[test]
fn test_comptime_only_export_is_mode_gated() {
    let mut module = ModuleExports::new("test");
    module
        .add_function("connect_codegen", |_args, _ctx: &ModuleContext| {
            Ok(ValueWord::unit())
        })
        .set_export_visibility("connect_codegen", ModuleExportVisibility::ComptimeOnly);

    assert!(!module.is_export_available("connect_codegen", false));
    assert!(module.is_export_available("connect_codegen", true));

    let runtime_names = module.export_names_available(false);
    assert!(
        !runtime_names.contains(&"connect_codegen"),
        "runtime export surface must hide comptime-only names"
    );
    let comptime_names = module.export_names_available(true);
    assert!(
        comptime_names.contains(&"connect_codegen"),
        "comptime export surface must include comptime-only names"
    );
}

#[test]
fn test_internal_export_hidden_from_public_surface() {
    let mut module = ModuleExports::new("test");
    module
        .add_function("__internal", |_args, _ctx: &ModuleContext| {
            Ok(ValueWord::unit())
        })
        .set_export_visibility("__internal", ModuleExportVisibility::Internal);

    assert!(module.is_export_available("__internal", false));
    assert!(module.is_export_available("__internal", true));
    assert!(!module.is_export_public_surface("__internal", false));
    assert!(!module.is_export_public_surface("__internal", true));
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
    assert!(check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "api.example.com:443").is_ok());
    assert!(check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "sub.trusted.io:8080").is_ok());

    // Denied hosts
    assert!(check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "evil.com:80").is_err());
    assert!(check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "other.example.com:443").is_err());
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

    assert!(check_net_permission(&ctx, shape_abi_v1::Permission::NetConnect, "any.host.com:8080").is_ok());
}
