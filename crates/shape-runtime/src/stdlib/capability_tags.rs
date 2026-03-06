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
        "io" => io_permissions(function),
        "file" => file_permissions(function),
        "http" => http_permissions(function),
        "env" => env_permissions(function),
        "time" => time_permissions(function),
        // Pure computation — no permissions required.
        "json" | "crypto" | "testing" | "regex" | "log" | "math" => PermissionSet::pure(),
        _ => PermissionSet::pure(),
    }
}

/// Return the union of all permissions that any function in `module` might
/// require. Useful for whole-module gating (e.g., "does this import need any
/// capabilities at all?").
pub fn module_permissions(module: &str) -> PermissionSet {
    match module {
        "io" => [
            Permission::FsRead,
            Permission::FsWrite,
            Permission::NetConnect,
            Permission::NetListen,
            Permission::Process,
        ]
        .into_iter()
        .collect(),
        "file" => [Permission::FsRead, Permission::FsWrite]
            .into_iter()
            .collect(),
        "http" => [Permission::NetConnect].into_iter().collect(),
        "env" => [Permission::Env].into_iter().collect(),
        "time" => [Permission::Time].into_iter().collect(),
        // Pure computation modules.
        "json" | "crypto" | "testing" | "regex" | "log" | "math" => PermissionSet::pure(),
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
        let perms = required_permissions("io", "open");
        assert!(perms.contains(&Permission::FsRead));
        assert_eq!(perms.len(), 1);

        let perms = required_permissions("io", "read_file");
        assert!(perms.contains(&Permission::FsRead));
    }

    #[test]
    fn io_write_requires_fs_write() {
        let perms = required_permissions("io", "write_file");
        assert!(perms.contains(&Permission::FsWrite));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn io_net_permissions() {
        let perms = required_permissions("io", "tcp_connect");
        assert!(perms.contains(&Permission::NetConnect));

        let perms = required_permissions("io", "listen");
        assert!(perms.contains(&Permission::NetListen));
    }

    #[test]
    fn io_process_permissions() {
        let perms = required_permissions("io", "spawn");
        assert!(perms.contains(&Permission::Process));

        let perms = required_permissions("io", "exec");
        assert!(perms.contains(&Permission::Process));
    }

    #[test]
    fn file_read_permissions() {
        for func in &["read_text", "read_lines", "read_bytes"] {
            let perms = required_permissions("file", func);
            assert!(
                perms.contains(&Permission::FsRead),
                "file::{func} should require FsRead"
            );
            assert_eq!(perms.len(), 1);
        }
    }

    #[test]
    fn file_write_permissions() {
        for func in &["write_text", "write_bytes", "append"] {
            let perms = required_permissions("file", func);
            assert!(
                perms.contains(&Permission::FsWrite),
                "file::{func} should require FsWrite"
            );
            assert_eq!(perms.len(), 1);
        }
    }

    #[test]
    fn http_requires_net_connect() {
        for func in &["get", "post", "put", "delete"] {
            let perms = required_permissions("http", func);
            assert!(
                perms.contains(&Permission::NetConnect),
                "http::{func} should require NetConnect"
            );
            assert_eq!(perms.len(), 1);
        }
    }

    #[test]
    fn env_requires_env_permission() {
        for func in &["get", "has", "all", "args", "cwd"] {
            let perms = required_permissions("env", func);
            assert!(
                perms.contains(&Permission::Env),
                "env::{func} should require Env"
            );
            assert_eq!(perms.len(), 1);
        }
    }

    #[test]
    fn time_millis_requires_time() {
        let perms = required_permissions("time", "millis");
        assert!(perms.contains(&Permission::Time));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn time_now_is_free() {
        let perms = required_permissions("time", "now");
        assert!(perms.is_empty());
    }

    #[test]
    fn pure_modules_require_nothing() {
        for module in &["json", "crypto", "testing", "regex", "log", "math"] {
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
        let perms = required_permissions("io", "nonexistent_function");
        assert!(perms.is_empty());
    }

    // -- module_permissions --

    #[test]
    fn io_module_permissions() {
        let perms = module_permissions("io");
        assert!(perms.contains(&Permission::FsRead));
        assert!(perms.contains(&Permission::FsWrite));
        assert!(perms.contains(&Permission::NetConnect));
        assert!(perms.contains(&Permission::NetListen));
        assert!(perms.contains(&Permission::Process));
        assert_eq!(perms.len(), 5);
    }

    #[test]
    fn file_module_permissions() {
        let perms = module_permissions("file");
        assert!(perms.contains(&Permission::FsRead));
        assert!(perms.contains(&Permission::FsWrite));
        assert_eq!(perms.len(), 2);
    }

    #[test]
    fn http_module_permissions() {
        let perms = module_permissions("http");
        assert!(perms.contains(&Permission::NetConnect));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn env_module_permissions() {
        let perms = module_permissions("env");
        assert!(perms.contains(&Permission::Env));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn time_module_permissions() {
        let perms = module_permissions("time");
        assert!(perms.contains(&Permission::Time));
        assert_eq!(perms.len(), 1);
    }

    #[test]
    fn pure_module_permissions() {
        for module in &["json", "crypto", "testing", "regex", "log", "math"] {
            let perms = module_permissions(module);
            assert!(perms.is_empty(), "{module} should require no permissions");
        }
    }

    #[test]
    fn function_perms_subset_of_module_perms() {
        // Every function's required permissions should be a subset of the module's.
        let test_cases = [
            (
                "io",
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
                "file",
                vec![
                    "read_text",
                    "read_lines",
                    "read_bytes",
                    "write_text",
                    "write_bytes",
                    "append",
                ],
            ),
            ("http", vec!["get", "post", "put", "delete"]),
            ("env", vec!["get", "has", "all", "args", "cwd"]),
            ("time", vec!["millis", "now"]),
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
