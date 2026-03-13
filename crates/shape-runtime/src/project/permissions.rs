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
        constraints
    }
}
