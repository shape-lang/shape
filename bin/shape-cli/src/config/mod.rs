//! Configuration loading for Shape CLI
//!
//! Handles loading extension module configurations from TOML files.

pub mod extensions;

pub use extensions::{
    ExtensionEntry, ExtensionsConfig, load_extensions_config, load_extensions_config_from,
};
