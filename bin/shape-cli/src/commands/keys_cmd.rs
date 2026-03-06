use anyhow::{Context, Result};
use std::path::PathBuf;

/// Default directory for storing Shape key files.
fn keys_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".shape").join("keys"))
}

/// Default path for the trusted authors keychain file.
fn keychain_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".shape").join("trusted_authors.json"))
}

/// `shape keys generate` -- generate a new Ed25519 key pair.
pub async fn run_keys_generate(output: Option<PathBuf>, name: String) -> Result<()> {
    let (secret_bytes, public_bytes) = shape_runtime::crypto::generate_keypair_bytes();

    let output_path = match output {
        Some(p) => p,
        None => {
            let dir = keys_dir()?;
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create keys directory: {}", dir.display()))?;
            dir.join(format!("{}.key", name))
        }
    };

    // Serialize keys as hex
    let secret_hex = hex::encode(secret_bytes);
    let public_hex = hex::encode(public_bytes);

    // Write secret key to file
    let key_content = format!(
        "{{\n  \"name\": \"{}\",\n  \"secret_key\": \"{}\",\n  \"public_key\": \"{}\"\n}}\n",
        name, secret_hex, public_hex
    );
    std::fs::write(&output_path, &key_content)
        .with_context(|| format!("failed to write key file: {}", output_path.display()))?;

    // Set restrictive permissions on the key file (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&output_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| {
                format!(
                    "failed to set permissions on key file: {}",
                    output_path.display()
                )
            })?;
    }

    eprintln!("Generated Ed25519 key pair:");
    eprintln!("  Name:       {}", name);
    eprintln!("  Public key: {}", public_hex);
    eprintln!("  Key file:   {}", output_path.display());
    eprintln!();
    eprintln!("Share the public key above with others who want to verify your modules.");
    eprintln!("Keep the key file private.");

    Ok(())
}

/// `shape keys trust` -- add a public key to the trusted keychain.
pub async fn run_keys_trust(public_key_hex: String, name: String, scope: String) -> Result<()> {
    let public_key_bytes: [u8; 32] = hex::decode(&public_key_hex)
        .context("invalid hex for public key")?
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("expected 32 bytes, got {}", v.len()))?;

    let trust_level = if scope == "full" {
        shape_runtime::crypto::TrustLevel::Full
    } else {
        let prefixes: Vec<String> = scope.split(',').map(|s| s.trim().to_string()).collect();
        shape_runtime::crypto::TrustLevel::Scoped(prefixes)
    };

    let author = shape_runtime::crypto::TrustedAuthor {
        name: name.clone(),
        public_key: public_key_bytes,
        trust_level,
    };

    // Load existing keychain or create new one
    let kc_path = keychain_path()?;
    let mut authors: Vec<shape_runtime::crypto::TrustedAuthor> = if kc_path.is_file() {
        let content = std::fs::read_to_string(&kc_path)
            .with_context(|| format!("failed to read keychain: {}", kc_path.display()))?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Remove existing entry for same key, then add new one
    authors.retain(|a| a.public_key != public_key_bytes);
    authors.push(author);

    // Write back
    if let Some(parent) = kc_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&authors)?;
    std::fs::write(&kc_path, json)
        .with_context(|| format!("failed to write keychain: {}", kc_path.display()))?;

    eprintln!(
        "Trusted author '{}' added with key {}.",
        name, public_key_hex
    );
    eprintln!("  Scope: {}", scope);
    eprintln!("  Keychain: {}", kc_path.display());

    Ok(())
}

/// `shape keys list` -- list trusted keys.
pub async fn run_keys_list() -> Result<()> {
    let kc_path = keychain_path()?;

    if !kc_path.is_file() {
        eprintln!("No trusted authors configured.");
        eprintln!("  Keychain path: {}", kc_path.display());
        return Ok(());
    }

    let content = std::fs::read_to_string(&kc_path)
        .with_context(|| format!("failed to read keychain: {}", kc_path.display()))?;
    let authors: Vec<shape_runtime::crypto::TrustedAuthor> =
        serde_json::from_str(&content).context("failed to parse keychain file")?;

    if authors.is_empty() {
        eprintln!("No trusted authors configured.");
        return Ok(());
    }

    eprintln!("Trusted authors ({}):", authors.len());
    for author in &authors {
        let scope_str = match &author.trust_level {
            shape_runtime::crypto::TrustLevel::Full => "full".to_string(),
            shape_runtime::crypto::TrustLevel::Scoped(prefixes) => {
                format!("scoped({})", prefixes.join(", "))
            }
            shape_runtime::crypto::TrustLevel::Pinned(hash) => {
                format!("pinned({})", hex::encode(hash))
            }
        };
        eprintln!(
            "  {} [{}]  {}",
            author.name,
            scope_str,
            hex::encode(author.public_key)
        );
    }

    Ok(())
}

/// `shape sign` -- sign a .shapec bundle.
pub async fn run_sign(bundle_path: PathBuf, key_path: PathBuf) -> Result<()> {
    // Read the signing key
    let key_content = std::fs::read_to_string(&key_path)
        .with_context(|| format!("failed to read key file: {}", key_path.display()))?;
    let key_json: serde_json::Value =
        serde_json::from_str(&key_content).context("failed to parse key file as JSON")?;
    let secret_hex = key_json["secret_key"]
        .as_str()
        .context("key file missing 'secret_key' field")?;
    let secret_bytes: [u8; 32] = hex::decode(secret_hex)
        .context("invalid hex in secret_key")?
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("expected 32-byte secret key, got {}", v.len()))?;

    // Read the bundle
    let mut bundle = shape_runtime::package_bundle::PackageBundle::read_from_file(
        &bundle_path,
    )
    .map_err(|e| anyhow::anyhow!("failed to read bundle '{}': {}", bundle_path.display(), e))?;

    let mut signed_count = 0usize;
    for manifest in &mut bundle.manifests {
        // Ensure the manifest hash is up to date
        manifest.finalize();

        let sig_data =
            shape_runtime::crypto::sign_manifest_hash(&manifest.manifest_hash, &secret_bytes);
        manifest.signature = Some(shape_runtime::module_manifest::ModuleSignature {
            author_key: sig_data.author_key,
            signature: sig_data.signature,
            signed_at: sig_data.signed_at,
        });
        signed_count += 1;
    }

    // Write the bundle back
    bundle
        .write_to_file(&bundle_path)
        .map_err(|e| anyhow::anyhow!("failed to write signed bundle: {}", e))?;

    let public_key = shape_runtime::crypto::public_key_from_secret(&secret_bytes);
    let public_hex = hex::encode(public_key);
    eprintln!(
        "Signed {} manifest(s) in {} with key {}",
        signed_count,
        bundle_path.display(),
        public_hex
    );

    Ok(())
}

/// `shape verify` -- verify signatures on a .shapec bundle.
pub async fn run_verify(bundle_path: PathBuf) -> Result<()> {
    let bundle = shape_runtime::package_bundle::PackageBundle::read_from_file(&bundle_path)
        .map_err(|e| {
        anyhow::anyhow!("failed to read bundle '{}': {}", bundle_path.display(), e)
    })?;

    if bundle.manifests.is_empty() {
        eprintln!("Bundle contains no manifests.");
        return Ok(());
    }

    let mut all_ok = true;
    for manifest in &bundle.manifests {
        let integrity_ok = manifest.verify_integrity();

        let sig_status = if let Some(sig) = &manifest.signature {
            let sig_data = shape_runtime::crypto::ModuleSignatureData {
                author_key: sig.author_key,
                signature: sig.signature.clone(),
                signed_at: sig.signed_at,
            };
            if sig_data.verify(&manifest.manifest_hash) {
                format!("valid (author: {})", hex::encode(sig.author_key))
            } else {
                all_ok = false;
                "INVALID SIGNATURE".to_string()
            }
        } else {
            "unsigned".to_string()
        };

        let integrity_str = if integrity_ok { "ok" } else { "FAILED" };
        if !integrity_ok {
            all_ok = false;
        }

        eprintln!(
            "  {} v{}: integrity={}, signature={}",
            manifest.name, manifest.version, integrity_str, sig_status
        );
    }

    if all_ok {
        eprintln!("All manifests verified successfully.");
    } else {
        anyhow::bail!("One or more manifests failed verification.");
    }

    Ok(())
}
