//! Environment identity for the performance charter suite (ADR-018 §1).
//!
//! ADR-016 discipline: "a number without a committed harness, exact revisions,
//! and environment identity is not evidence". This module produces the
//! environment half of that. The identity is a hash over a fixed, ordered set
//! of fields; when the captured identity differs from the one pinned in the
//! suite manifest, the report refuses to render a comparison and names the
//! fields that moved.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// The ordered field set that constitutes an environment identity. A field
/// present here and absent from a captured environment is itself a difference.
pub const IDENTITY_FIELDS: &[&str] = &[
    "arch",
    "cpu_logical_cores",
    "cpu_model",
    "extensions_fingerprint",
    "node_binary",
    "node_v8_version",
    "node_version",
    "os_kernel",
    "rustc_version",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Environment {
    #[serde(flatten)]
    pub fields: BTreeMap<String, String>,
}

/// One field that differs between a captured environment and a pinned one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldDiff {
    pub field: String,
    pub pinned: Option<String>,
    pub captured: Option<String>,
}

impl Environment {
    pub fn get(&self, field: &str) -> Option<&str> {
        self.fields.get(field).map(String::as_str)
    }

    fn canonical_form(&self) -> String {
        let mut out = String::new();
        for field in IDENTITY_FIELDS {
            let value = self
                .fields
                .get(*field)
                .map(String::as_str)
                .unwrap_or("<absent>");
            out.push_str(field);
            out.push('=');
            out.push_str(value);
            out.push('\n');
        }
        out
    }

    /// Identity hash over `IDENTITY_FIELDS` only. Fields outside that list are
    /// recorded in the report for context but do not move the identity.
    pub fn identity(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.canonical_form().as_bytes());
        format!("sha256:{:x}", hasher.finalize())
    }

    /// Every identity field whose value differs from `pinned`.
    pub fn diff(&self, pinned: &Environment) -> Vec<FieldDiff> {
        let mut diffs = Vec::new();
        for field in IDENTITY_FIELDS {
            let captured = self.fields.get(*field).cloned();
            let expected = pinned.fields.get(*field).cloned();
            if captured != expected {
                diffs.push(FieldDiff {
                    field: (*field).to_string(),
                    pinned: expected,
                    captured,
                });
            }
        }
        diffs
    }
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn first_cpu_model() -> Option<String> {
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("model name") {
            return Some(rest.trim_start_matches([' ', '\t', ':']).trim().to_string());
        }
    }
    None
}

fn logical_cores() -> Option<String> {
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let n = text.lines().filter(|l| l.starts_with("processor")).count();
    (n > 0).then(|| n.to_string())
}

/// Hash of the extension shared libraries the CLI loads at startup. These are
/// on the startup path (the interpreter loads them before user code runs), so
/// a machine with different extensions installed is not a comparable machine.
pub fn extensions_fingerprint(dir: &Path) -> String {
    let mut entries: Vec<(String, u64)> = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("so") {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                entries.push((name, size));
            }
        }
    }
    if entries.is_empty() {
        return "none".to_string();
    }
    entries.sort();
    let mut hasher = Sha256::new();
    for (name, size) in &entries {
        hasher.update(format!("{name}:{size}\n").as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// Where the CLI looks for extensions (`shape_config_dir()/extensions`).
pub fn default_extensions_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".shape").join("extensions"))
}

/// Capture the current environment. `node` is the reference-runtime binary; a
/// missing one leaves the node fields absent, which by construction differs
/// from any pinned identity that has them.
pub fn capture(node: Option<&Path>) -> Result<Environment> {
    let mut fields = BTreeMap::new();
    let mut put = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            fields.insert(k.to_string(), v);
        }
    };

    put("os_kernel", command_stdout("uname", &["-sr"]));
    put("arch", command_stdout("uname", &["-m"]));
    put("cpu_model", first_cpu_model());
    put("cpu_logical_cores", logical_cores());
    put("rustc_version", command_stdout("rustc", &["--version"]));

    if let Some(node) = node {
        let node_str = node.to_string_lossy().to_string();
        put("node_binary", Some(node_str.clone()));
        put("node_version", command_stdout(&node_str, &["--version"]));
        put(
            "node_v8_version",
            command_stdout(&node_str, &["-p", "process.versions.v8"]),
        );
    }

    let ext_dir = default_extensions_dir();
    put(
        "extensions_fingerprint",
        Some(match ext_dir {
            Some(dir) => extensions_fingerprint(&dir),
            None => "none".to_string(),
        }),
    );

    Ok(Environment { fields })
}

/// Resolve the reference runtime binary: an explicit override, else `node` on
/// PATH resolved to its absolute path (a nix store path is itself an exact
/// pin, which is why the resolved path is an identity field).
pub fn resolve_node(explicit: Option<&Path>) -> Option<std::path::PathBuf> {
    if let Some(path) = explicit {
        return path.exists().then(|| path.to_path_buf());
    }
    let resolved = command_stdout("sh", &["-c", "command -v node"])?;
    let path = std::path::PathBuf::from(resolved);
    path.exists().then_some(path)
}

/// One-minute load average. Recorded as report context, never as an identity
/// field: a busy machine is not a *different* machine, and the authoritative
/// contention signal is the reference runtime's own run-to-run stability.
pub fn load_average_1m() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/loadavg").ok()?;
    text.split_whitespace().next()?.parse().ok()
}

/// sha256 of a file's contents, used for the shape binary and the manifest.
pub fn file_sha256(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading {} for hashing", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> Environment {
        Environment {
            fields: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }

    #[test]
    fn identity_is_stable_across_insertion_order() {
        let a = env_of(&[("os_kernel", "Linux 6.12"), ("arch", "x86_64")]);
        let b = env_of(&[("arch", "x86_64"), ("os_kernel", "Linux 6.12")]);
        assert_eq!(a.identity(), b.identity());
    }

    #[test]
    fn identity_changes_when_an_identity_field_changes() {
        let a = env_of(&[("node_version", "v24.14.1")]);
        let b = env_of(&[("node_version", "v22.0.0")]);
        assert_ne!(a.identity(), b.identity());
    }

    #[test]
    fn identity_ignores_non_identity_fields() {
        let a = env_of(&[("os_kernel", "Linux 6.12")]);
        let mut b = a.clone();
        b.fields
            .insert("captured_at".to_string(), "2026-07-27".to_string());
        assert_eq!(a.identity(), b.identity());
    }

    #[test]
    fn absent_field_is_a_difference_not_a_match() {
        let pinned = env_of(&[("node_version", "v24.14.1")]);
        let captured = env_of(&[]);
        let diffs = captured.diff(&pinned);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].field, "node_version");
        assert_eq!(diffs[0].captured, None);
        assert_ne!(pinned.identity(), captured.identity());
    }

    #[test]
    fn diff_names_every_moved_field() {
        let pinned = env_of(&[("node_version", "v24.14.1"), ("arch", "x86_64")]);
        let captured = env_of(&[("node_version", "v22.0.0"), ("arch", "aarch64")]);
        let diffs = captured.diff(&pinned);
        let names: Vec<&str> = diffs.iter().map(|d| d.field.as_str()).collect();
        assert_eq!(names, vec!["arch", "node_version"]);
    }

    #[test]
    fn identical_environments_produce_no_diff() {
        let a = env_of(&[("node_version", "v24.14.1"), ("arch", "x86_64")]);
        assert!(a.diff(&a).is_empty());
    }

    #[test]
    fn extensions_fingerprint_of_missing_dir_is_none() {
        let dir = std::path::PathBuf::from("/nonexistent-extensions-dir-for-test");
        assert_eq!(extensions_fingerprint(&dir), "none");
    }
}
