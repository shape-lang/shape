use anyhow::{Context, Result};

use crate::registry_client::{Credentials, RegistryClient};

const DEFAULT_REGISTRY: &str = "https://pkg.shape-lang.dev";

/// `shape login` -- authenticate with the package registry.
///
/// Stores the API token in `~/.shape/credentials.json` (mode 0600).
/// The token is validated against the registry before saving.
pub async fn run_login(token: String, registry: Option<String>) -> Result<()> {
    let registry_url = registry.unwrap_or_else(|| DEFAULT_REGISTRY.to_string());

    // Basic token format validation
    let token = token.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("API token must not be empty");
    }
    if token.len() < 8 {
        anyhow::bail!("API token is too short (minimum 8 characters)");
    }

    // Validate the token against the registry by making a test request
    let client = RegistryClient::new(Some(registry_url.clone())).with_token(token.clone());
    client.validate_token().await.map_err(|e| {
        anyhow::anyhow!(
            "Token validation failed: {}\nCheck your token and try again.",
            e
        )
    })?;

    // Save credentials
    let credentials = Credentials {
        registry: registry_url.clone(),
        token,
    };
    RegistryClient::save_credentials(&credentials).map_err(|e| anyhow::anyhow!("{}", e))?;

    eprintln!("Logged in to {}", registry_url);
    eprintln!("Credentials saved to ~/.shape/credentials.json");

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
        assert!(msg.contains("must not be empty"), "got: {}", msg);
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
        assert!(msg.contains("must not be empty"), "got: {}", msg);
    }
}
