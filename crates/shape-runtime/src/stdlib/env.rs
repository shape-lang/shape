//! Native `env` module for environment variable and system info access.
//!
//! Exports: env.get, env.has, env.all, env.args, env.cwd, env.os, env.arch
//!
//! Policy gated: requires Env permission at runtime.
//!
//! Phase 4c: all 7 exports migrated to `TypedModuleExports`. `env.get`
//! uses the new `TypedReturn::Some` / `TypedReturn::None` variants for
//! its `Option<string>` return.

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteType, TypedReturn, register_typed_function};
use shape_value::{ValueWord, ValueWordExt};
#[cfg(test)]
use std::sync::Arc;

/// Create the `env` module with environment variable and system info functions.
pub fn create_env_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::env");
    module.description = "Environment variables and system information".to_string();

    // env.get(name: string) -> Option<string>
    register_typed_function(
        &mut module,
        "get",
        "Get the value of an environment variable, or none if not set",
        vec![ModuleParam {
            name: "name".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Environment variable name".to_string(),
            ..Default::default()
        }],
        ConcreteType::Option(Box::new(ConcreteType::String)),
        |args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            let name = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "env.get() requires a variable name string".to_string())?;

            match std::env::var(name) {
                Ok(val) => Ok(TypedReturn::Some(Box::new(TypedReturn::String(val)))),
                Err(_) => Ok(TypedReturn::None),
            }
        },
    );

    // env.has(name: string) -> bool
    register_typed_function(
        &mut module,
        "has",
        "Check if an environment variable is set",
        vec![ModuleParam {
            name: "name".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Environment variable name".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        |args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            let name = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "env.has() requires a variable name string".to_string())?;
            Ok(TypedReturn::Bool(std::env::var(name).is_ok()))
        },
    );

    // env.all() -> HashMap<string, string>
    register_typed_function(
        &mut module,
        "all",
        "Get all environment variables as a HashMap",
        vec![],
        ConcreteType::HashMapStringString,
        |_args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            let pairs: Vec<(String, String)> = std::env::vars().collect();
            Ok(TypedReturn::HashMapStringString(pairs))
        },
    );

    // env.args() -> Array<string>
    register_typed_function(
        &mut module,
        "args",
        "Get command-line arguments as an array of strings",
        vec![],
        ConcreteType::ArrayString,
        |_args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            let args: Vec<String> = std::env::args().collect();
            Ok(TypedReturn::ArrayString(args))
        },
    );

    // env.cwd() -> string
    register_typed_function(
        &mut module,
        "cwd",
        "Get the current working directory",
        vec![],
        ConcreteType::String,
        |_args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            let cwd = std::env::current_dir().map_err(|e| format!("env.cwd() failed: {}", e))?;
            Ok(TypedReturn::String(cwd.to_string_lossy().into_owned()))
        },
    );

    // env.os() -> string
    register_typed_function(
        &mut module,
        "os",
        "Get the operating system name (e.g. linux, macos, windows)",
        vec![],
        ConcreteType::String,
        |_args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            Ok(TypedReturn::String(std::env::consts::OS.to_string()))
        },
    );

    // env.arch() -> string
    register_typed_function(
        &mut module,
        "arch",
        "Get the CPU architecture (e.g. x86_64, aarch64)",
        vec![],
        ConcreteType::String,
        |_args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            Ok(TypedReturn::String(std::env::consts::ARCH.to_string()))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(val: &str) -> ValueWord {
        ValueWord::from_string(Arc::new(val.to_string()))
    }

    fn test_ctx() -> crate::module_exports::ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        crate::module_exports::ModuleContext {
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
    fn test_env_module_creation() {
        let module = create_env_module();
        assert_eq!(module.name, "std::core::env");
        assert!(module.has_export("get"));
        assert!(module.has_export("has"));
        assert!(module.has_export("all"));
        assert!(module.has_export("args"));
        assert!(module.has_export("cwd"));
        assert!(module.has_export("os"));
        assert!(module.has_export("arch"));
    }

    #[test]
    fn test_env_get_path() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("get").unwrap();
        // PATH should always be set
        let result = f(&[s("PATH")], &ctx).unwrap();
        let inner = result.as_some_inner().expect("PATH should be set");
        assert!(!inner.as_str().unwrap().is_empty());
    }

    #[test]
    fn test_env_get_missing() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("get").unwrap();
        let result = f(&[s("__SHAPE_NONEXISTENT_VAR_12345__")], &ctx).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_env_get_requires_string() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("get").unwrap();
        assert!(f(&[ValueWord::from_f64(42.0)], &ctx).is_err());
    }

    #[test]
    fn test_env_has_path() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("has").unwrap();
        let result = f(&[s("PATH")], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_env_has_missing() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("has").unwrap();
        let result = f(&[s("__SHAPE_NONEXISTENT_VAR_12345__")], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_env_all_returns_hashmap() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("all").unwrap();
        let result = f(&[], &ctx).unwrap();
        let (keys, _values, _index) = result.as_hashmap().expect("should be hashmap");
        // Should have at least PATH
        assert!(!keys.is_empty());
    }

    #[test]
    fn test_env_args_returns_array() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("args").unwrap();
        let result = f(&[], &ctx).unwrap();
        let arr = result.as_any_array().expect("should be array").to_generic();
        // At least the binary name
        assert!(!arr.is_empty());
    }

    #[test]
    fn test_env_cwd_returns_string() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("cwd").unwrap();
        let result = f(&[], &ctx).unwrap();
        let cwd = result.as_str().expect("should be string");
        assert!(!cwd.is_empty());
    }

    #[test]
    fn test_env_os_returns_string() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("os").unwrap();
        let result = f(&[], &ctx).unwrap();
        let os = result.as_str().expect("should be string");
        assert!(!os.is_empty());
        // Should be one of the known OS values
        assert!(
            ["linux", "macos", "windows", "freebsd", "android", "ios"].contains(&os),
            "unexpected OS: {}",
            os
        );
    }

    #[test]
    fn test_env_arch_returns_string() {
        let module = create_env_module();
        let ctx = test_ctx();
        let f = module.get_export("arch").unwrap();
        let result = f(&[], &ctx).unwrap();
        let arch = result.as_str().expect("should be string");
        assert!(!arch.is_empty());
    }

    #[test]
    fn test_env_schemas() {
        let module = create_env_module();

        let get_schema = module.get_schema("get").unwrap();
        assert_eq!(get_schema.params.len(), 1);
        assert_eq!(get_schema.return_type.as_deref(), Some("Option<string>"));

        let all_schema = module.get_schema("all").unwrap();
        assert_eq!(all_schema.params.len(), 0);

        let os_schema = module.get_schema("os").unwrap();
        assert_eq!(os_schema.return_type.as_deref(), Some("string"));
    }

    #[test]
    fn test_env_typed_registry_populated() {
        let module = create_env_module();
        // All 7 exports are migrated to the typed registry as of Phase 4c.
        let typed = module.typed_exports();
        assert!(typed.get("get").is_some());
        assert!(typed.get("has").is_some());
        assert!(typed.get("all").is_some());
        assert!(typed.get("args").is_some());
        assert!(typed.get("cwd").is_some());
        assert!(typed.get("os").is_some());
        assert!(typed.get("arch").is_some());

        let has_entry = typed.get("has").unwrap();
        assert_eq!(has_entry.return_type, ConcreteType::Bool);

        let get_entry = typed.get("get").unwrap();
        assert_eq!(
            get_entry.return_type,
            ConcreteType::Option(Box::new(ConcreteType::String))
        );
    }
}
