//! CLI-facing re-export of the shared extension configuration loader.
//!
//! Canonical parser/loader lives in `shape-runtime::extensions_config` and is
//! reused here to avoid split-brain config handling.

pub use shape_runtime::extensions_config::{
    ExtensionEntry, ExtensionsConfig, load_extensions_config, load_extensions_config_from,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
    }
}
