//! Static mapping of stdlib module functions to required permissions.
//!
//! Consulted at compile time (bytecode compilation) to determine whether a
//! call site requires specific permissions. Pure-computation modules (json,
//! crypto, math, testing, regex, log) require no permissions.

use shape_abi_v1::{Permission, PermissionSet};

/// Return the permissions required to call `module::function`.
///
/// Returns an empty set for pure-computation functions and for unknown
/// module/function pairs (the compiler may emit a separate "unknown function"
/// diagnostic).
pub fn required_permissions(module: &str, function: &str) -> PermissionSet {
    match module {
        "std::core::io" => io_permissions(function),
        "std::core::file" => file_permissions(function),
        "std::core::http" => http_permissions(function),
        "std::core::env" => env_permissions(function),
        "std::core::time" => time_permissions(function),
        "std::core::csv" => csv_permissions(function),
        // Pure computation — no permissions required.
        "std::core::json" | "std::core::crypto" | "std::core::testing" | "std::core::regex"
        | "std::core::math" => PermissionSet::pure(),
        _ => PermissionSet::pure(),
    }
}

/// Return the union of all permissions that any function in `module` might
/// require. Useful for whole-module gating (e.g., "does this import need any
/// capabilities at all?").
pub fn module_permissions(module: &str) -> PermissionSet {
    match module {
        "std::core::io" => [
            Permission::FsRead,
            Permission::FsWrite,
            Permission::NetConnect,
            Permission::NetListen,
            Permission::Process,
        ]
        .into_iter()
        .collect(),
        "std::core::file" => [Permission::FsRead, Permission::FsWrite]
            .into_iter()
            .collect(),
        "std::core::http" => [Permission::NetConnect].into_iter().collect(),
        "std::core::csv" => [Permission::FsRead].into_iter().collect(),
        "std::core::env" => [Permission::Env].into_iter().collect(),
        "std::core::time" => [Permission::Time].into_iter().collect(),
        // Pure computation modules.
        "std::core::json" | "std::core::crypto" | "std::core::testing" | "std::core::regex"
        | "std::core::math" => PermissionSet::pure(),
        _ => PermissionSet::pure(),
    }
}

// ---------------------------------------------------------------------------
// Per-module function mapping
// ---------------------------------------------------------------------------

fn io_permissions(function: &str) -> PermissionSet {
    match function {
        "open" | "read_file" => [Permission::FsRead].into_iter().collect(),
        "write_file" => [Permission::FsWrite].into_iter().collect(),
        "tcp_connect" => [Permission::NetConnect].into_iter().collect(),
        "listen" => [Permission::NetListen].into_iter().collect(),
        "spawn" | "exec" => [Permission::Process].into_iter().collect(),
        _ => PermissionSet::pure(),
    }
}

fn file_permissions(function: &str) -> PermissionSet {
    match function {
        "read_text" | "read_lines" | "read_bytes" => [Permission::FsRead].into_iter().collect(),
        "write_text" | "write_bytes" | "append" => [Permission::FsWrite].into_iter().collect(),
        _ => PermissionSet::pure(),
    }
}

fn http_permissions(function: &str) -> PermissionSet {
    match function {
        "get" | "post" | "put" | "delete" => [Permission::NetConnect].into_iter().collect(),
        _ => PermissionSet::pure(),
    }
}

fn env_permissions(function: &str) -> PermissionSet {
    match function {
        "get" | "has" | "all" | "args" | "cwd" => [Permission::Env].into_iter().collect(),
        _ => PermissionSet::pure(),
    }
}

fn csv_permissions(function: &str) -> PermissionSet {
    match function {
        "read_file" => [Permission::FsRead].into_iter().collect(),
        // parse, parse_records, stringify, stringify_records, is_valid are pure computation.
        _ => PermissionSet::pure(),
    }
}

fn time_permissions(function: &str) -> PermissionSet {
    // `millis` reads wall-clock time.
    // `now()` (monotonic) is always allowed and does not appear here.
    match function {
        "millis" => [Permission::Time].into_iter().collect(),
        _ => PermissionSet::pure(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- required_permissions --

    #[test]
    fn io_read_requires_fs_read() {
        let perms = required_permissions("std::core::io", "open");
        assert!(perms.contains(&Permission::FsRead));
        assert_eq!(perms.len(), 1);

        let perms = required_permissions("std::core::io", "read_file");
        assert!(perms.contains(&Permission::FsRead));
    }

    #[test]
    fn io_write_requires_fs_write() {
        let perms = required_permissions("std::core::io", "write_file");
        assert!(perms.contains(&Permission::FsWrite));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn io_net_permissions() {
        let perms = required_permissions("std::core::io", "tcp_connect");
        assert!(perms.contains(&Permission::NetConnect));

        let perms = required_permissions("std::core::io", "listen");
        assert!(perms.contains(&Permission::NetListen));
    }

    #[test]
    fn io_process_permissions() {
        let perms = required_permissions("std::core::io", "spawn");
        assert!(perms.contains(&Permission::Process));

        let perms = required_permissions("std::core::io", "exec");
        assert!(perms.contains(&Permission::Process));
    }

    #[test]
    fn file_read_permissions() {
        for func in &["read_text", "read_lines", "read_bytes"] {
            let perms = required_permissions("std::core::file", func);
            assert!(
                perms.contains(&Permission::FsRead),
                "std::core::file::{func} should require FsRead"
            );
            assert_eq!(perms.len(), 1);
        }
    }

    #[test]
    fn file_write_permissions() {
        for func in &["write_text", "write_bytes", "append"] {
            let perms = required_permissions("std::core::file", func);
            assert!(
                perms.contains(&Permission::FsWrite),
                "std::core::file::{func} should require FsWrite"
            );
            assert_eq!(perms.len(), 1);
        }
    }

    #[test]
    fn http_requires_net_connect() {
        for func in &["get", "post", "put", "delete"] {
            let perms = required_permissions("std::core::http", func);
            assert!(
                perms.contains(&Permission::NetConnect),
                "std::core::http::{func} should require NetConnect"
            );
            assert_eq!(perms.len(), 1);
        }
    }

    #[test]
    fn env_requires_env_permission() {
        for func in &["get", "has", "all", "args", "cwd"] {
            let perms = required_permissions("std::core::env", func);
            assert!(
                perms.contains(&Permission::Env),
                "std::core::env::{func} should require Env"
            );
            assert_eq!(perms.len(), 1);
        }
    }

    #[test]
    fn time_millis_requires_time() {
        let perms = required_permissions("std::core::time", "millis");
        assert!(perms.contains(&Permission::Time));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn time_now_is_free() {
        let perms = required_permissions("std::core::time", "now");
        assert!(perms.is_empty());
    }

    #[test]
    fn pure_modules_require_nothing() {
        for module in &[
            "std::core::json",
            "std::core::crypto",
            "std::core::testing",
            "std::core::regex",
            "std::core::math",
        ] {
            let perms = required_permissions(module, "any_function");
            assert!(
                perms.is_empty(),
                "{module}::any_function should require no permissions"
            );
        }
    }

    #[test]
    fn unknown_module_requires_nothing() {
        let perms = required_permissions("unknown_module", "whatever");
        assert!(perms.is_empty());
    }

    #[test]
    fn unknown_function_in_known_module_requires_nothing() {
        let perms = required_permissions("std::core::io", "nonexistent_function");
        assert!(perms.is_empty());
    }

    // -- module_permissions --

    #[test]
    fn io_module_permissions() {
        let perms = module_permissions("std::core::io");
        assert!(perms.contains(&Permission::FsRead));
        assert!(perms.contains(&Permission::FsWrite));
        assert!(perms.contains(&Permission::NetConnect));
        assert!(perms.contains(&Permission::NetListen));
        assert!(perms.contains(&Permission::Process));
        assert_eq!(perms.len(), 5);
    }

    #[test]
    fn file_module_permissions() {
        let perms = module_permissions("std::core::file");
        assert!(perms.contains(&Permission::FsRead));
        assert!(perms.contains(&Permission::FsWrite));
        assert_eq!(perms.len(), 2);
    }

    #[test]
    fn http_module_permissions() {
        let perms = module_permissions("std::core::http");
        assert!(perms.contains(&Permission::NetConnect));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn env_module_permissions() {
        let perms = module_permissions("std::core::env");
        assert!(perms.contains(&Permission::Env));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn time_module_permissions() {
        let perms = module_permissions("std::core::time");
        assert!(perms.contains(&Permission::Time));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn pure_module_permissions() {
        for module in &[
            "std::core::json",
            "std::core::crypto",
            "std::core::testing",
            "std::core::regex",
            "std::core::math",
        ] {
            let perms = module_permissions(module);
            assert!(perms.is_empty(), "{module} should require no permissions");
        }
    }

    #[test]
    fn function_perms_subset_of_module_perms() {
        // Every function's required permissions should be a subset of the module's.
        let test_cases = [
            (
                "std::core::io",
                vec![
                    "open",
                    "read_file",
                    "write_file",
                    "tcp_connect",
                    "listen",
                    "spawn",
                    "exec",
                ],
            ),
            (
                "std::core::file",
                vec![
                    "read_text",
                    "read_lines",
                    "read_bytes",
                    "write_text",
                    "write_bytes",
                    "append",
                ],
            ),
            ("std::core::http", vec!["get", "post", "put", "delete"]),
            ("std::core::env", vec!["get", "has", "all", "args", "cwd"]),
            ("std::core::time", vec!["millis", "now"]),
        ];
        for (module, functions) in &test_cases {
            let mod_perms = module_permissions(module);
            for func in functions {
                let fn_perms = required_permissions(module, func);
                assert!(
                    fn_perms.is_subset(&mod_perms),
                    "{module}::{func} permissions {:?} not subset of module permissions {:?}",
                    fn_perms,
                    mod_perms
                );
            }
        }
    }
}
