use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config;
use crate::registry_client::RegistryClient;
use shape_runtime::crypto::signing::ModuleSignatureData;
use shape_runtime::package_bundle::{PackageBundle, verify_bundle_checksum};

/// Registry index file format (mirrors dependency_resolver's private type).
#[derive(Debug, Deserialize)]
struct RegistryIndexFile {
    #[serde(default)]
    #[allow(dead_code)]
    package: Option<String>,
    #[serde(default)]
    versions: Vec<RegistryVersionRecord>,
}

#[derive(Debug, Deserialize)]
struct RegistryVersionRecord {
    version: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    checksum: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    author_key: Option<String>,
    #[serde(default)]
    required_permissions: Vec<String>,
    #[serde(default)]
    has_native_deps: bool,
    #[serde(default)]
    native_platforms: Vec<String>,
}

/// Run the `shape add` command: add a dependency to the current project.
pub async fn run_add(name: String, version: Option<String>) -> Result<()> {
    let client = RegistryClient::new(None);

    // 1. Fetch and cache index
    eprintln!("Fetching index for '{}'...", name);
    let index_text = client
        .fetch_index(&name)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let config_dir = config::shape_config_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    let index_dir = config_dir.join("registry").join("index");
    std::fs::create_dir_all(&index_dir)
        .with_context(|| format!("failed to create index directory: {}", index_dir.display()))?;
    let index_path = index_dir.join(format!("{}.toml", name));
    std::fs::write(&index_path, &index_text)
        .with_context(|| format!("failed to write index to {}", index_path.display()))?;

    // 2. Parse index and resolve version
    let index: RegistryIndexFile = toml::from_str(&index_text)
        .with_context(|| format!("failed to parse registry index for '{}'", name))?;

    let resolved_version = if let Some(ref v) = version {
        // Verify the requested version exists and is not yanked
        let record = index
            .versions
            .iter()
            .find(|r| r.version == *v)
            .ok_or_else(|| anyhow::anyhow!("version {} not found for package '{}'", v, name))?;
        if record.yanked {
            anyhow::bail!("version {} of '{}' has been yanked", v, name);
        }
        v.clone()
    } else {
        // Pick latest non-yanked version
        index
            .versions
            .iter()
            .rev()
            .find(|r| !r.yanked)
            .map(|r| r.version.clone())
            .ok_or_else(|| anyhow::anyhow!("no available (non-yanked) versions for '{}'", name))?
    };

    let version_record = index
        .versions
        .iter()
        .find(|r| r.version == resolved_version)
        .expect("resolved version must exist in index");

    // 3. Download bundle
    eprintln!("Downloading {} v{}...", name, resolved_version);
    let bundle_bytes = client
        .download_bundle(&name, &resolved_version)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // 4. Cache bundle
    let cache_dir = config_dir
        .join("registry")
        .join("cache")
        .join(&name);
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create cache directory: {}", cache_dir.display()))?;
    let bundle_path = cache_dir.join(format!("{}.shapec", resolved_version));
    std::fs::write(&bundle_path, &bundle_bytes)
        .with_context(|| format!("failed to write bundle to {}", bundle_path.display()))?;

    // 5. Verify checksum if present
    if let Some(ref checksum) = version_record.checksum {
        if !verify_bundle_checksum(&bundle_bytes, checksum) {
            anyhow::bail!(
                "checksum mismatch for {} v{}: expected {}",
                name,
                resolved_version,
                checksum
            );
        }
        eprintln!("Checksum verified.");
    }

    // 6. Deserialize and verify manifests
    let bundle = PackageBundle::from_bytes(&bundle_bytes)
        .map_err(|e| anyhow::anyhow!("failed to deserialize bundle: {}", e))?;

    for manifest in &bundle.manifests {
        if !manifest.verify_integrity() {
            anyhow::bail!(
                "manifest integrity check failed for module '{}' in {} v{}",
                manifest.name,
                name,
                resolved_version
            );
        }

        if let Some(ref sig) = manifest.signature {
            let sig_data = ModuleSignatureData {
                author_key: sig.author_key,
                signature: sig.signature.clone(),
                signed_at: sig.signed_at,
            };
            if !sig_data.verify(&manifest.manifest_hash) {
                anyhow::bail!(
                    "signature verification failed for module '{}' in {} v{}",
                    manifest.name,
                    name,
                    resolved_version
                );
            }
            // TOFU: warn if author key is unknown
            eprintln!(
                "Warning: module '{}' is signed by key {}. \
                 Run `shape keys trust` to add this key to your trusted keychain.",
                manifest.name,
                hex::encode(sig.author_key)
            );
        }
    }

    // 7. Check required permissions
    if !version_record.required_permissions.is_empty() {
        eprintln!(
            "Warning: {} v{} requires elevated permissions: {}",
            name,
            resolved_version,
            version_record.required_permissions.join(", ")
        );
    }

    // Check native dependencies
    if version_record.has_native_deps {
        eprintln!(
            "Warning: {} v{} has native dependencies (platforms: {})",
            name,
            resolved_version,
            version_record.native_platforms.join(", ")
        );
        let current = if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "unknown"
        };
        if !version_record.native_platforms.is_empty()
            && !version_record
                .native_platforms
                .contains(&current.to_string())
        {
            eprintln!("Warning: your platform ({}) may not be supported!", current);
        }
    }

    // 8. Update shape.toml
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let project = shape_runtime::project::try_find_project_root(&cwd)
        .map_err(|e| anyhow::anyhow!("{}", e))?
        .ok_or_else(|| {
            anyhow::anyhow!("No shape.toml found. Run `shape add` from within a Shape project.")
        })?;

    let toml_path = project.root_path.join("shape.toml");
    let toml_text = std::fs::read_to_string(&toml_path)
        .with_context(|| format!("failed to read {}", toml_path.display()))?;

    let updated = add_dependency_to_toml(&toml_text, &name, &resolved_version);
    std::fs::write(&toml_path, &updated)
        .with_context(|| format!("failed to write {}", toml_path.display()))?;

    eprintln!("Added {} v{} to dependencies.", name, resolved_version);
    Ok(())
}

/// Insert or update a dependency in the TOML text, preserving formatting.
fn add_dependency_to_toml(toml_text: &str, name: &str, version: &str) -> String {
    let dep_line = format!("{} = \"{}\"", name, version);
    let lines: Vec<&str> = toml_text.lines().collect();

    // Find [dependencies] section
    let mut dep_section_idx = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" {
            dep_section_idx = Some(i);
            break;
        }
    }

    if let Some(section_start) = dep_section_idx {
        // Find end of [dependencies] section (next section header or EOF)
        let mut section_end = lines.len();
        for i in (section_start + 1)..lines.len() {
            let trimmed = lines[i].trim();
            if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
                section_end = i;
                break;
            }
        }

        // Check if dependency already exists; if so, replace it
        let prefix = format!("{} ", name);
        let prefix_eq = format!("{}=", name);
        for i in (section_start + 1)..section_end {
            let trimmed = lines[i].trim();
            if trimmed.starts_with(&prefix) || trimmed.starts_with(&prefix_eq) {
                // Check this is actually a key assignment for our dep name
                if let Some(eq_pos) = trimmed.find('=') {
                    let key = trimmed[..eq_pos].trim();
                    if key == name {
                        let mut result: Vec<String> =
                            lines[..i].iter().map(|s| s.to_string()).collect();
                        result.push(dep_line);
                        result.extend(lines[i + 1..].iter().map(|s| s.to_string()));
                        return result.join("\n")
                            + if toml_text.ends_with('\n') { "\n" } else { "" };
                    }
                }
            }
        }

        // Insert before the next section (or at the end of the section)
        let insert_at = section_end;
        let mut result: Vec<String> = lines[..insert_at].iter().map(|s| s.to_string()).collect();
        result.push(dep_line);
        result.extend(lines[insert_at..].iter().map(|s| s.to_string()));
        result.join("\n") + if toml_text.ends_with('\n') { "\n" } else { "" }
    } else {
        // No [dependencies] section exists; append one
        let mut result = toml_text.to_string();
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("\n[dependencies]\n");
        result.push_str(&dep_line);
        result.push('\n');
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_dep_to_existing_section() {
        let input = r#"[project]
name = "my-app"
version = "0.1.0"

[dependencies]
foo = "1.0.0"
"#;
        let result = add_dependency_to_toml(input, "bar", "2.0.0");
        assert!(result.contains("bar = \"2.0.0\""));
        assert!(result.contains("foo = \"1.0.0\""));
    }

    #[test]
    fn test_add_dep_creates_section() {
        let input = r#"[project]
name = "my-app"
version = "0.1.0"
"#;
        let result = add_dependency_to_toml(input, "bar", "1.0.0");
        assert!(result.contains("[dependencies]"));
        assert!(result.contains("bar = \"1.0.0\""));
    }

    #[test]
    fn test_replace_existing_dep() {
        let input = r#"[project]
name = "my-app"

[dependencies]
bar = "1.0.0"
"#;
        let result = add_dependency_to_toml(input, "bar", "2.0.0");
        assert!(result.contains("bar = \"2.0.0\""));
        assert!(!result.contains("bar = \"1.0.0\""));
    }

    #[test]
    fn test_add_dep_before_next_section() {
        let input = r#"[project]
name = "my-app"

[dependencies]
foo = "1.0.0"

[build]
target = "bytecode"
"#;
        let result = add_dependency_to_toml(input, "bar", "2.0.0");
        assert!(result.contains("bar = \"2.0.0\""));
        // bar should appear before [build]
        let bar_pos = result.find("bar = \"2.0.0\"").unwrap();
        let build_pos = result.find("[build]").unwrap();
        assert!(bar_pos < build_pos);
    }
}
