use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::config;
use crate::registry_client::RegistryClient;

/// Find the first `.key` file in the keys directory.
fn find_default_signing_key() -> Result<PathBuf> {
    let config_dir =
        config::shape_config_dir().context("could not determine config directory")?;
    let keys_dir = config_dir.join("keys");
    if !keys_dir.is_dir() {
        anyhow::bail!(
            "No keys directory found at {}. Run `shape keys generate` first.",
            keys_dir.display()
        );
    }
    for entry in std::fs::read_dir(&keys_dir)
        .with_context(|| format!("failed to read keys directory: {}", keys_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("key") {
            return Ok(path);
        }
    }
    anyhow::bail!(
        "No .key files found in {}. Run `shape keys generate` first.",
        keys_dir.display()
    );
}

/// Sign all manifests in a bundle using the given key file. Returns the public key hex.
fn sign_bundle(
    bundle: &mut shape_runtime::package_bundle::PackageBundle,
    key_path: &PathBuf,
) -> Result<String> {
    let key_content = std::fs::read_to_string(key_path)
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

    for manifest in &mut bundle.manifests {
        manifest.finalize();
        let sig_data =
            shape_runtime::crypto::sign_manifest_hash(&manifest.manifest_hash, &secret_bytes);
        manifest.signature = Some(shape_runtime::module_manifest::ModuleSignature {
            author_key: sig_data.author_key,
            signature: sig_data.signature,
            signed_at: sig_data.signed_at,
        });
    }

    let public_key = shape_runtime::crypto::public_key_from_secret(&secret_bytes);
    Ok(hex::encode(public_key))
}

/// Collect `.shape` source files into a tar.gz archive.
fn create_source_tarball(project_root: &std::path::Path) -> Result<Vec<u8>> {
    let mut archive = Vec::new();
    {
        let encoder =
            flate2::write::GzEncoder::new(&mut archive, flate2::Compression::default());
        let mut tar = tar::Builder::new(encoder);

        let src_dir = project_root.join("src");
        if src_dir.is_dir() {
            tar.append_dir_all("src", &src_dir)
                .context("failed to add src/ to source tarball")?;
        }

        // Include shape.toml
        let toml_path = project_root.join("shape.toml");
        if toml_path.is_file() {
            tar.append_path_with_name(&toml_path, "shape.toml")
                .context("failed to add shape.toml to source tarball")?;
        }

        tar.finish().context("failed to finalize source tarball")?;
    }
    Ok(archive)
}

/// `shape publish` -- build, sign, and publish a package to the registry.
pub async fn run_publish(
    registry: Option<String>,
    key: Option<PathBuf>,
    no_sign: bool,
    no_source: bool,
    native: Vec<String>,
) -> Result<()> {
    // Step 1: Find project and build
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let project = shape_runtime::project::try_find_project_root(&cwd)
        .map_err(|e| anyhow::anyhow!("{}", e))?
        .ok_or_else(|| {
            anyhow::anyhow!("No shape.toml found. Run `shape publish` from within a Shape project.")
        })?;

    let pkg_name = &project.config.project.name;
    let pkg_version = &project.config.project.version;

    if pkg_name.is_empty() {
        anyhow::bail!("shape.toml [project].name is required for publishing");
    }
    if pkg_version.is_empty() {
        anyhow::bail!("shape.toml [project].version is required for publishing");
    }

    eprintln!("Building {} v{}...", pkg_name, pkg_version);

    let mut bundle = shape_vm::bundle_compiler::BundleCompiler::compile(&project)
        .map_err(|e| anyhow::anyhow!("Build failed: {}", e))?;

    eprintln!(
        "Compiled {} module(s), {} manifest(s)",
        bundle.modules.len(),
        bundle.manifests.len()
    );

    // Step 2: Sign bundle manifests
    if !no_sign {
        let key_path = match key {
            Some(p) => p,
            None => find_default_signing_key()?,
        };
        eprintln!("Signing with key {}...", key_path.display());
        let public_hex = sign_bundle(&mut bundle, &key_path)?;
        eprintln!(
            "Signed {} manifest(s) with key {}",
            bundle.manifests.len(),
            public_hex
        );
    }

    // Step 3: Load and validate credentials
    let credentials = RegistryClient::load_credentials().map_err(|e| {
        anyhow::anyhow!(
            "{}\nRun `shape login` to authenticate with the registry.",
            e
        )
    })?;

    if credentials.token.trim().is_empty() {
        anyhow::bail!(
            "Registry token is empty.\nRun `shape login` to authenticate with the registry."
        );
    }

    let client = RegistryClient::new(registry).with_token(credentials.token);

    // Validate the token before uploading
    eprintln!("Authenticating...");
    client.validate_token().await.map_err(|e| {
        anyhow::anyhow!(
            "Authentication failed: {}\nRun `shape login` to re-authenticate.",
            e
        )
    })?;

    // Step 4: Serialize bundle
    let bundle_bytes = bundle
        .to_bytes()
        .map_err(|e| anyhow::anyhow!("failed to serialize bundle: {}", e))?;

    // Step 5: Collect source tarball (unless --no-source)
    let source_bytes = if no_source {
        None
    } else {
        eprintln!("Packaging source...");
        Some(create_source_tarball(&project.root_path)?)
    };

    // Step 6: Collect native blobs from --native flags
    let mut native_blobs: Vec<(String, Vec<u8>)> = Vec::new();
    for spec in &native {
        let (target, path) = spec.split_once('=').ok_or_else(|| {
            anyhow::anyhow!(
                "invalid --native format '{}': expected 'target=path' (e.g. 'linux-x86_64=./lib.tar.gz')",
                spec
            )
        })?;
        let data = std::fs::read(path)
            .with_context(|| format!("failed to read native blob from '{}'", path))?;
        native_blobs.push((target.to_string(), data));
    }

    // Step 7: Upload via multipart
    let bundle_size = bundle_bytes.len();
    eprintln!("Uploading {} ({} bytes)...", pkg_name, bundle_size);

    let response = client
        .publish_multipart(bundle_bytes, source_bytes, native_blobs)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Step 8: Show success
    eprintln!("Published {} v{}", pkg_name, pkg_version);
    if !response.is_empty() {
        eprintln!("{}", response);
    }
    eprintln!("  https://pkg.shape-lang.dev/packages/{}", pkg_name);

    Ok(())
}
