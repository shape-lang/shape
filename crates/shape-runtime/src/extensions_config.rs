//! Global extension configuration loading from TOML.
//!
//! Loads extension definitions from `extensions.toml` using the same schema
//! across CLI, runtime tooling, and LSP.
//!
//! Search order:
//! 1. `$SHAPE_CONFIG_DIR/extensions.toml` (if set)
//! 2. `~/.config/shape/extensions.toml`

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Root configuration structure for `extensions.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionsConfig {
    /// List of extension entries.
    #[serde(default)]
    pub extensions: Vec<ExtensionEntry>,
}

/// A single extension entry in the configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionEntry {
    /// Module name (for logs/diagnostics and namespace registration).
    pub name: String,
    /// Path to the shared library (.so/.dylib/.dll).
    pub path: PathBuf,
    /// Configuration passed to module initialization.
    #[serde(default)]
    pub config: HashMap<String, toml::Value>,
}

impl ExtensionEntry {
    /// Convert TOML config values to JSON for runtime plugin initialization.
    pub fn config_as_json(&self) -> serde_json::Value {
        toml_to_json(&toml::Value::Table(
            self.config
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ))
    }
}

/// Load extension configuration from the default location.
///
/// Returns an empty config if no file exists.
pub fn load_extensions_config() -> Result<ExtensionsConfig> {
    let config_path = get_config_path()?;
    if !config_path.exists() {
        return Ok(ExtensionsConfig::default());
    }
    load_extensions_config_from(&config_path)
}

/// Load extension configuration from a specific file path.
pub fn load_extensions_config_from(path: &Path) -> Result<ExtensionsConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read extension config from {:?}", path))?;
    let config: ExtensionsConfig = toml::from_str(&content)
        .with_context(|| format!("Failed to parse extension config from {:?}", path))?;
    Ok(config)
}

/// Resolve the default extension config path.
pub fn get_config_path() -> Result<PathBuf> {
    if let Ok(config_dir) = std::env::var("SHAPE_CONFIG_DIR") {
        return Ok(PathBuf::from(config_dir).join("extensions.toml"));
    }

    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    Ok(config_dir.join("shape").join("extensions.toml"))
}

fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        toml::Value::Array(arr) => serde_json::Value::Array(arr.iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => {
            let map: serde_json::Map<String, serde_json::Value> = table
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_config() {
        let config: ExtensionsConfig = toml::from_str("").unwrap();
        assert!(config.extensions.is_empty());
    }

    #[test]
    fn test_parse_single_module() {
        let toml_str = r#"
[[extensions]]
name = "files"
path = "./libshape_plugin_files.so"

[extensions.config]
base_dir = "./data"
"#;

        let config: ExtensionsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.extensions.len(), 1);
        assert_eq!(config.extensions[0].name, "files");
        assert_eq!(
            config.extensions[0].path,
            PathBuf::from("./libshape_plugin_files.so")
        );

        let json_config = config.extensions[0].config_as_json();
        assert_eq!(json_config["base_dir"], "./data");
    }

    #[test]
    fn test_parse_multiple_modules() {
        let toml_str = r#"
[[extensions]]
name = "market-data"
path = "./libshape_plugin_market_data.so"

[extensions.config]
duckdb_path = "/path/to/market.duckdb"
default_timeframe = "1d"
read_only = true

[[extensions]]
name = "files"
path = "./libshape_plugin_files.so"

[extensions.config]
base_dir = "./data"
"#;

        let config: ExtensionsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.extensions.len(), 2);
        assert_eq!(config.extensions[0].name, "market-data");
        let json0 = config.extensions[0].config_as_json();
        assert_eq!(json0["duckdb_path"], "/path/to/market.duckdb");
        assert_eq!(json0["default_timeframe"], "1d");
        assert_eq!(json0["read_only"], true);
        assert_eq!(config.extensions[1].name, "files");
    }
}
