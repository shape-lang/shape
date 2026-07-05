//! Permission-related types and logic for shape.toml `[permissions]`.

use serde::{Deserialize, Serialize};

/// Permission shorthand: a string like "pure", "readonly", or "full",
/// or an inline table with fine-grained booleans.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum PermissionPreset {
    /// Shorthand name: "pure", "readonly", or "full".
    Shorthand(String),
    /// Inline table with per-permission booleans.
    Table(PermissionsSection),
}

/// [permissions] section — declares what capabilities the project needs.
///
/// Missing fields default to `true` for backwards compatibility (unless
/// the `--sandbox` CLI flag overrides to `PermissionSet::pure()`).
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct PermissionsSection {
    #[serde(default, rename = "fs.read")]
    pub fs_read: Option<bool>,
    #[serde(default, rename = "fs.write")]
    pub fs_write: Option<bool>,
    #[serde(default, rename = "net.connect")]
    pub net_connect: Option<bool>,
    #[serde(default, rename = "net.listen")]
    pub net_listen: Option<bool>,
    #[serde(default)]
    pub process: Option<bool>,
    #[serde(default)]
    pub env: Option<bool>,
    #[serde(default)]
    pub time: Option<bool>,
    #[serde(default)]
    pub random: Option<bool>,

    /// Grant execution of foreign code (extern C / embedded python/typescript).
    ///
    /// Unlike the other coarse booleans (which default to `true` when a
    /// `[permissions]` section is present), an *unset* `ffi` defaults to
    /// `false`: `Ffi` grants full process authority, so an explicit
    /// allowlist never silently confers it (ffi-rebuild §4.8.2 — sandboxed /
    /// explicit-`[permissions]` contexts exclude `Ffi` unless granted).
    #[serde(default)]
    pub ffi: Option<bool>,
    /// Allowed foreign language ids (e.g. `["python"]`). Empty = all languages
    /// when `ffi = true` (parity with an unscoped grant).
    #[serde(default)]
    pub ffi_languages: Vec<String>,
    /// Allowed native-library path globs for `extern C` (post alias resolution).
    #[serde(default)]
    pub ffi_libraries: Vec<String>,
    /// Optional glob over symbols permitted within the allowed libraries.
    #[serde(default)]
    pub ffi_symbols: Vec<String>,

    /// Scoped filesystem constraints.
    #[serde(default)]
    pub fs: Option<FsPermissions>,
    /// Scoped network constraints.
    #[serde(default)]
    pub net: Option<NetPermissions>,
}

/// [permissions.fs] — path-level filesystem constraints.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct FsPermissions {
    /// Paths with full read/write access (glob patterns).
    #[serde(default)]
    pub allowed: Vec<String>,
    /// Paths with read-only access (glob patterns).
    #[serde(default)]
    pub read_only: Vec<String>,
}

/// [permissions.net] — host-level network constraints.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct NetPermissions {
    /// Allowed network hosts (host:port patterns, `*` wildcards).
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

impl PermissionsSection {
    /// Create a section from a shorthand name.
    ///
    /// - `"pure"` — all permissions false (no I/O).
    /// - `"readonly"` — fs.read + env + time, nothing else.
    /// - `"full"` — all permissions true.
    pub fn from_shorthand(name: &str) -> Option<Self> {
        match name {
            "pure" => Some(Self {
                fs_read: Some(false),
                fs_write: Some(false),
                net_connect: Some(false),
                net_listen: Some(false),
                process: Some(false),
                env: Some(false),
                time: Some(false),
                random: Some(false),
                ffi: Some(false),
                ffi_languages: Vec::new(),
                ffi_libraries: Vec::new(),
                ffi_symbols: Vec::new(),
                fs: None,
                net: None,
            }),
            "readonly" => Some(Self {
                fs_read: Some(true),
                fs_write: Some(false),
                net_connect: Some(false),
                net_listen: Some(false),
                process: Some(false),
                env: Some(true),
                time: Some(true),
                random: Some(false),
                ffi: Some(false),
                ffi_languages: Vec::new(),
                ffi_libraries: Vec::new(),
                ffi_symbols: Vec::new(),
                fs: None,
                net: None,
            }),
            "full" => Some(Self {
                fs_read: Some(true),
                fs_write: Some(true),
                net_connect: Some(true),
                net_listen: Some(true),
                process: Some(true),
                env: Some(true),
                time: Some(true),
                random: Some(true),
                // `full` is explicit total trust — it does confer Ffi.
                ffi: Some(true),
                ffi_languages: Vec::new(),
                ffi_libraries: Vec::new(),
                ffi_symbols: Vec::new(),
                fs: None,
                net: None,
            }),
            _ => None,
        }
    }

    /// Convert to a `PermissionSet` from shape-abi-v1.
    ///
    /// Unset fields (`None`) default to `true` for backwards compatibility.
    pub fn to_permission_set(&self) -> shape_abi_v1::PermissionSet {
        use shape_abi_v1::Permission;
        let mut set = shape_abi_v1::PermissionSet::pure();
        if self.fs_read.unwrap_or(true) {
            set.insert(Permission::FsRead);
        }
        if self.fs_write.unwrap_or(true) {
            set.insert(Permission::FsWrite);
        }
        if self.net_connect.unwrap_or(true) {
            set.insert(Permission::NetConnect);
        }
        if self.net_listen.unwrap_or(true) {
            set.insert(Permission::NetListen);
        }
        if self.process.unwrap_or(true) {
            set.insert(Permission::Process);
        }
        if self.env.unwrap_or(true) {
            set.insert(Permission::Env);
        }
        if self.time.unwrap_or(true) {
            set.insert(Permission::Time);
        }
        if self.random.unwrap_or(true) {
            set.insert(Permission::Random);
        }
        // Foreign code: fail-closed default. Unlike the other coarse
        // permissions, an unset `ffi` in an explicit `[permissions]` section
        // defaults to `false` — `Ffi` confers process authority and must be
        // opted into explicitly (ffi-rebuild §4.8.2).
        if self.ffi.unwrap_or(false) {
            set.insert(Permission::Ffi);
        }
        // Scoped permissions
        if self.fs.as_ref().map_or(false, |fs| {
            !fs.allowed.is_empty() || !fs.read_only.is_empty()
        }) {
            set.insert(Permission::FsScoped);
        }
        if self
            .net
            .as_ref()
            .map_or(false, |net| !net.allowed_hosts.is_empty())
        {
            set.insert(Permission::NetScoped);
        }
        set
    }

    /// Build `ScopeConstraints` from the fs/net sub-sections.
    pub fn to_scope_constraints(&self) -> shape_abi_v1::ScopeConstraints {
        let mut constraints = shape_abi_v1::ScopeConstraints::none();
        if let Some(ref fs) = self.fs {
            let mut paths = fs.allowed.clone();
            paths.extend(fs.read_only.iter().cloned());
            constraints.allowed_paths = paths;
        }
        if let Some(ref net) = self.net {
            constraints.allowed_hosts = net.allowed_hosts.clone();
        }
        // Foreign-code scope (carried for WF-2A enforcement; ffi-rebuild §4.8.2).
        constraints.ffi_languages = self.ffi_languages.clone();
        constraints.ffi_libraries = self.ffi_libraries.clone();
        constraints.ffi_symbols = self.ffi_symbols.clone();
        constraints
    }
}

#[cfg(test)]
mod ffi_tests {
    use super::*;
    use shape_abi_v1::Permission;

    #[test]
    fn unset_ffi_defaults_closed_even_when_section_present() {
        // A `[permissions]` section that omits `ffi` must NOT confer it, even
        // though the other coarse permissions default-open (ffi-rebuild §4.8.2).
        let section: PermissionsSection = toml::from_str("").unwrap();
        assert!(!section.to_permission_set().contains(&Permission::Ffi));
    }

    #[test]
    fn ffi_true_grants_and_scope_lists_are_carried() {
        let section: PermissionsSection = toml::from_str(
            r#"
            ffi = true
            ffi_languages = ["python"]
            ffi_libraries = ["/usr/lib/*"]
            ffi_symbols = ["labs"]
            "#,
        )
        .unwrap();
        assert!(section.to_permission_set().contains(&Permission::Ffi));
        let scope = section.to_scope_constraints();
        assert_eq!(scope.ffi_languages, vec!["python".to_string()]);
        assert_eq!(scope.ffi_libraries, vec!["/usr/lib/*".to_string()]);
        assert_eq!(scope.ffi_symbols, vec!["labs".to_string()]);
    }

    #[test]
    fn ffi_false_excludes_grant() {
        let section: PermissionsSection = toml::from_str("ffi = false").unwrap();
        assert!(!section.to_permission_set().contains(&Permission::Ffi));
    }

    #[test]
    fn shorthands_map_ffi_correctly() {
        assert!(
            PermissionsSection::from_shorthand("full")
                .unwrap()
                .to_permission_set()
                .contains(&Permission::Ffi),
            "full is total trust — confers Ffi"
        );
        for name in ["pure", "readonly"] {
            assert!(
                !PermissionsSection::from_shorthand(name)
                    .unwrap()
                    .to_permission_set()
                    .contains(&Permission::Ffi),
                "{name} must not confer Ffi"
            );
        }
    }
}
