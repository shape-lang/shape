//! Configuration loading for Shape CLI
//!
//! Handles loading extension module configurations from TOML files,
//! centralized constants, and config directory resolution.

use std::path::PathBuf;

pub mod extensions;

pub use extensions::{
    ExtensionEntry, ExtensionsConfig, load_extensions_config, load_extensions_config_from,
};

/// The default Shape package registry URL.
pub const DEFAULT_REGISTRY: &str = "https://pkg.shape-lang.dev";

/// Return the Shape configuration directory.
///
/// Resolution order:
/// 1. `SHAPE_CONFIG_DIR` environment variable (if set and non-empty).
/// 2. `~/.shape/` (via `dirs::home_dir()`).
pub fn shape_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SHAPE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::home_dir().map(|h| h.join(".shape"))
}

/// Token format validation beyond simple length checks.
pub fn validate_token_format(token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("API token must not be empty".to_string());
    }
    if token.len() < 16 {
        return Err(format!(
            "API token is too short ({} characters, minimum 16)",
            token.len()
        ));
    }
    if token.len() > 4096 {
        return Err(format!(
            "API token is suspiciously long ({} characters, maximum 4096)",
            token.len()
        ));
    }
    if !token.is_ascii() {
        return Err("API token must contain only ASCII characters".to_string());
    }
    for (i, ch) in token.chars().enumerate() {
        if ch.is_ascii_whitespace() {
            return Err(format!("API token contains whitespace at position {}", i));
        }
        if !ch.is_ascii_alphanumeric()
            && !matches!(ch, '-' | '_' | '.' | '~' | '+' | '/' | '=')
        {
            return Err(format!(
                "API token contains invalid character '{}' at position {}",
                ch, i
            ));
        }
    }
    Ok(())
}

/// Mask a token for display, showing only first 4 and last 4 characters.
pub fn mask_token(token: &str) -> String {
    if token.len() <= 12 {
        return "[redacted]".to_string();
    }
    let prefix = &token[..4];
    let suffix = &token[token.len() - 4..];
    format!("{}...{}", prefix, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry_constant() {
        assert_eq!(DEFAULT_REGISTRY, "https://pkg.shape-lang.dev");
    }

    #[test]
    fn test_shape_config_dir_uses_env_var() {
        let saved = std::env::var("SHAPE_CONFIG_DIR").ok();
        unsafe { std::env::set_var("SHAPE_CONFIG_DIR", "/tmp/shape-test-config"); }
        let result = shape_config_dir();
        assert_eq!(result, Some(PathBuf::from("/tmp/shape-test-config")));
        match saved {
            Some(val) => unsafe { std::env::set_var("SHAPE_CONFIG_DIR", val); },
            None => unsafe { std::env::remove_var("SHAPE_CONFIG_DIR"); },
        }
    }

    #[test]
    fn test_shape_config_dir_ignores_empty_env() {
        let saved = std::env::var("SHAPE_CONFIG_DIR").ok();
        unsafe { std::env::set_var("SHAPE_CONFIG_DIR", ""); }
        let result = shape_config_dir();
        assert_ne!(result, Some(PathBuf::from("")));
        match saved {
            Some(val) => unsafe { std::env::set_var("SHAPE_CONFIG_DIR", val); },
            None => unsafe { std::env::remove_var("SHAPE_CONFIG_DIR"); },
        }
    }

    #[test]
    fn test_validate_token_format_valid() {
        assert!(validate_token_format("abcdefgh12345678").is_ok());
        assert!(validate_token_format("shp_abcdefghijklmnop").is_ok());
    }

    #[test]
    fn test_validate_token_format_empty() {
        let err = validate_token_format("").unwrap_err();
        assert!(err.contains("empty"), "got: {}", err);
    }

    #[test]
    fn test_validate_token_format_too_short() {
        let err = validate_token_format("abc1234").unwrap_err();
        assert!(err.contains("too short"), "got: {}", err);
    }

    #[test]
    fn test_validate_token_format_whitespace() {
        let err = validate_token_format("abc defgh12345678").unwrap_err();
        assert!(err.contains("whitespace"), "got: {}", err);
    }

    #[test]
    fn test_validate_token_format_invalid_char() {
        let err = validate_token_format("abcdefgh1234567!").unwrap_err();
        assert!(err.contains("invalid character"), "got: {}", err);
    }

    #[test]
    fn test_mask_token_normal() {
        assert_eq!(mask_token("abcdefghijklmnop"), "abcd...mnop");
    }

    #[test]
    fn test_mask_token_short() {
        assert_eq!(mask_token("short"), "[redacted]");
    }
}
