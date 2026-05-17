//! Native `env` module for environment variable and system info access.
//!
//! Phase 2b canary: migrated to the typed marshal layer
//! (`crate::marshal::register_typed_fn_N`). Native function bodies take
//! typed Rust args via [`shape_runtime::marshal::FromSlot`]; their Rust
//! signatures *are* the typed signatures. The Rust trait system rejects
//! registration whose body's parameter types don't match.
//!
//! Currently exports the all-scalar-return subset:
//!   `env.has`, `env.cwd`, `env.os`, `env.arch`.
//!
//! Pending Phase 2c marshal extensions:
//!   - `env.get`  (`Option<string>` return — needs `ToSlot` for Option/None)
//!   - `env.all`  (`HashMap<string, string>` return — needs Map marshal)
//!   - `env.args` (`Array<string>` return — needs Array marshal)
//!
//! Policy gated: requires `Env` permission at runtime.
//!
//! See `docs/defections.md` 2026-05-06 (Phase 2b unified marshal).

use crate::marshal::{register_typed_fn_0, register_typed_fn_1};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;

/// Create the `env` module with environment variable and system info functions.
pub fn create_env_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::env");
    module.description = "Environment variables and system information".to_string();

    // env.has(name: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "has",
        "Check if an environment variable is set",
        "name",
        "string",
        ConcreteType::Bool,
        |name, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(
                std::env::var(name.as_str()).is_ok(),
            )))
        },
    );

    // env.cwd() -> string
    register_typed_fn_0(
        &mut module,
        "cwd",
        "Get the current working directory",
        ConcreteType::String,
        |ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            let cwd = std::env::current_dir().map_err(|e| format!("env.cwd() failed: {}", e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::String(
                cwd.to_string_lossy().into_owned(),
            )))
        },
    );

    // env.os() -> string
    register_typed_fn_0(
        &mut module,
        "os",
        "Get the operating system name (e.g. linux, macos, windows)",
        ConcreteType::String,
        |ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::String(
                std::env::consts::OS.to_string(),
            )))
        },
    );

    // env.arch() -> string
    register_typed_fn_0(
        &mut module,
        "arch",
        "Get the CPU architecture (e.g. x86_64, aarch64)",
        ConcreteType::String,
        |ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Env)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::String(
                std::env::consts::ARCH.to_string(),
            )))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_module_creation() {
        let module = create_env_module();
        assert_eq!(module.name, "std::core::env");
        assert!(module.has_export("has"));
        assert!(module.has_export("cwd"));
        assert!(module.has_export("os"));
        assert!(module.has_export("arch"));
    }

    #[test]
    fn test_env_typed_registry_arg_kinds() {
        let module = create_env_module();
        let typed = module.typed_exports();

        // Phase 2b structural property: arg_kinds is derived from the body's
        // Rust parameter types via FromSlot::NATIVE_KIND. env.has takes
        // one Arc<String>, which is NativeKind::String.
        let has_entry = typed.get("has").unwrap();
        assert_eq!(has_entry.arg_kinds.len(), 1);
        assert_eq!(has_entry.arg_kinds[0], shape_value::NativeKind::String);
        assert_eq!(has_entry.return_type, ConcreteType::Bool);

        // Zero-arg functions populate empty arg_kinds.
        let cwd_entry = typed.get("cwd").unwrap();
        assert!(cwd_entry.arg_kinds.is_empty());
        assert_eq!(cwd_entry.return_type, ConcreteType::String);
    }

    #[test]
    fn test_env_schemas() {
        let module = create_env_module();
        let has_schema = module.get_schema("has").unwrap();
        assert_eq!(has_schema.params.len(), 1);
        assert_eq!(has_schema.return_type.as_deref(), Some("bool"));

        let os_schema = module.get_schema("os").unwrap();
        assert_eq!(os_schema.return_type.as_deref(), Some("string"));
    }
}
