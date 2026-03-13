use anyhow::Result;

use crate::config::{self, DEFAULT_REGISTRY, mask_token, validate_token_format};
use crate::registry_client::{Credentials, RegistryClient};

/// `shape login` -- authenticate with the package registry.
///
/// Stores the API token in the credentials file (mode 0600).
/// The token is validated against the registry before saving.
pub async fn run_login(token: String, registry: Option<String>) -> Result<()> {
    let registry_url = registry.unwrap_or_else(|| DEFAULT_REGISTRY.to_string());

    // Token format validation
    let token = token.trim().to_string();
    validate_token_format(&token).map_err(|e| anyhow::anyhow!("{}", e))?;

    // Validate the token against the registry by making a test request
    let client = RegistryClient::new(Some(registry_url.clone())).with_token(token.clone());
    client.validate_token().await.map_err(|e| {
        anyhow::anyhow!(
            "Token validation failed: {}\nCheck your token and try again.",
            e
        )
    })?;

    // Show masked token once for confirmation
    eprintln!("Token: {}", mask_token(&token));

    // Save credentials
    let credentials = Credentials {
        registry: registry_url.clone(),
        token,
    };
    RegistryClient::save_credentials(&credentials).map_err(|e| anyhow::anyhow!("{}", e))?;

    let creds_path = config::shape_config_dir()
        .map(|d| d.join("credentials.json").display().to_string())
        .unwrap_or_else(|| "~/.shape/credentials.json".to_string());
    eprintln!("Logged in to {}", registry_url);
    eprintln!("Credentials saved to {}", creds_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_token_rejected() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_login("".to_string(), None));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("empty"), "got: {}", msg);
    }

    #[test]
    fn test_short_token_rejected() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_login("abc".to_string(), None));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("too short"), "got: {}", msg);
    }

    #[test]
    fn test_whitespace_only_token_rejected() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_login("   ".to_string(), None));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("empty"), "got: {}", msg);
    }

    #[test]
    fn test_invalid_char_token_rejected() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_login("abc!defgh12345678".to_string(), None));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("invalid character"), "got: {}", msg);
    }
}
