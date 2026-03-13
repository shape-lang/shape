use anyhow::Result;
use std::io::{self, Write};

use crate::config::{self, DEFAULT_REGISTRY, mask_token};
use crate::registry_client::{Credentials, RegistryClient};

fn prompt(label: &str) -> Result<String> {
    eprint!("{label}");
    io::stderr().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

fn prompt_password(label: &str) -> Result<String> {
    eprint!("{label}");
    io::stderr().flush()?;
    let password = rpassword::read_password()?;
    Ok(password)
}

/// `shape register` -- create a new account on the package registry.
pub async fn run_register(registry: Option<String>) -> Result<()> {
    let registry_url = registry.unwrap_or_else(|| DEFAULT_REGISTRY.to_string());

    let username = prompt("Username: ")?;
    if username.is_empty() {
        anyhow::bail!("username must not be empty");
    }

    let email = prompt("Email: ")?;
    if email.is_empty() {
        anyhow::bail!("email must not be empty");
    }

    let password = prompt_password("Password: ")?;
    if password.len() < 8 {
        anyhow::bail!("password must be at least 8 characters");
    }

    let confirm = prompt_password("Confirm password: ")?;
    if password != confirm {
        anyhow::bail!("passwords do not match");
    }

    let client = RegistryClient::new(Some(registry_url.clone()));
    let response = client
        .register(&username, &email, &password)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Show token once (masked) for confirmation
    eprintln!("Token: {}", mask_token(&response.token));

    let credentials = Credentials {
        registry: registry_url.clone(),
        token: response.token,
    };
    RegistryClient::save_credentials(&credentials).map_err(|e| anyhow::anyhow!("{}", e))?;

    let creds_path = config::shape_config_dir()
        .map(|d| d.join("credentials.json").display().to_string())
        .unwrap_or_else(|| "~/.shape/credentials.json".to_string());
    eprintln!("Registered as {}", response.username);
    eprintln!("Credentials saved to {}", creds_path);

    Ok(())
}
