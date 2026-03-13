//! Sandbox configuration for shape.toml `[sandbox]`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// [sandbox] section — isolation settings for deterministic/testing modes.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct SandboxSection {
    /// Whether sandbox mode is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Use a deterministic runtime (fixed time, seeded RNG).
    #[serde(default)]
    pub deterministic: bool,
    /// RNG seed for deterministic mode.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Memory limit (human-readable, e.g. "64MB").
    #[serde(default)]
    pub memory_limit: Option<String>,
    /// Execution time limit (human-readable, e.g. "10s").
    #[serde(default)]
    pub time_limit: Option<String>,
    /// Use a virtual filesystem instead of real I/O.
    #[serde(default)]
    pub virtual_fs: bool,
    /// Seed files for the virtual filesystem: vfs_path -> real_path.
    #[serde(default)]
    pub seed_files: HashMap<String, String>,
}

impl SandboxSection {
    /// Parse the memory_limit string (e.g. "64MB") into bytes.
    pub fn memory_limit_bytes(&self) -> Option<u64> {
        self.memory_limit.as_ref().and_then(|s| parse_byte_size(s))
    }

    /// Parse the time_limit string (e.g. "10s") into milliseconds.
    pub fn time_limit_ms(&self) -> Option<u64> {
        self.time_limit.as_ref().and_then(|s| parse_duration_ms(s))
    }
}

/// Parse a human-readable byte size like "64MB", "1GB", "512KB".
pub(crate) fn parse_byte_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, suffix) = split_numeric_suffix(s)?;
    let value: u64 = num_part.parse().ok()?;
    let multiplier = match suffix.to_uppercase().as_str() {
        "B" | "" => 1,
        "KB" | "K" => 1024,
        "MB" | "M" => 1024 * 1024,
        "GB" | "G" => 1024 * 1024 * 1024,
        _ => return None,
    };
    Some(value * multiplier)
}

/// Parse a human-readable duration like "10s", "500ms", "2m".
pub(crate) fn parse_duration_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, suffix) = split_numeric_suffix(s)?;
    let value: u64 = num_part.parse().ok()?;
    let multiplier = match suffix.to_lowercase().as_str() {
        "ms" => 1,
        "s" | "" => 1000,
        "m" | "min" => 60_000,
        _ => return None,
    };
    Some(value * multiplier)
}

/// Split "64MB" into ("64", "MB").
fn split_numeric_suffix(s: &str) -> Option<(&str, &str)> {
    let idx = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    if idx == 0 {
        return None;
    }
    Some((&s[..idx], &s[idx..]))
}
