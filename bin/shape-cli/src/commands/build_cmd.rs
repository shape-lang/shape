use anyhow::{Context, Result};
use std::path::PathBuf;

/// Run the `shape build` command: compile a package into a .shapec bundle.
pub async fn run_build(output: Option<PathBuf>, _opt_level: u8) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;

    let project = shape_runtime::project::try_find_project_root(&cwd)
        .map_err(|e| anyhow::anyhow!("{}", e))?
        .ok_or_else(|| {
            anyhow::anyhow!("No shape.toml found. Run `shape build` from within a Shape project.")
        })?;

    eprintln!(
        "Building package '{}' v{}...",
        project.config.project.name, project.config.project.version
    );

    let bundle = shape_vm::bundle_compiler::BundleCompiler::compile(&project)
        .map_err(|e| anyhow::anyhow!("Build failed: {}", e))?;

    // Determine output path: CLI flag > shape.toml [build].output > auto-generated name
    let bundle_filename = {
        let name = if project.config.project.name.is_empty() {
            "package"
        } else {
            &project.config.project.name
        };
        let version = if project.config.project.version.is_empty() {
            "0.0.0"
        } else {
            &project.config.project.version
        };
        if bundle.metadata.native_portable {
            format!("{}-{}.shapec", name, version)
        } else {
            let host = if bundle.metadata.build_host.trim().is_empty() {
                format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
            } else {
                bundle.metadata.build_host.clone()
            };
            format!("{}-{}-{}.shapec", name, version, host)
        }
    };

    let output_path = if let Some(path) = output {
        path
    } else if let Some(ref output_dir) = project.config.build.output {
        let dir = project.root_path.join(output_dir);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create output directory: {}", dir.display()))?;
        }
        dir.join(&bundle_filename)
    } else {
        PathBuf::from(&bundle_filename)
    };

    bundle
        .write_to_file(&output_path)
        .map_err(|e| anyhow::anyhow!("Failed to write bundle: {}", e))?;

    let file_size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    eprintln!(
        "Built {} modules into {} ({} bytes)",
        bundle.modules.len(),
        output_path.display(),
        file_size
    );

    Ok(())
}

/// Compute the output path for a build, given CLI flag, project config, and bundle metadata.
///
/// Priority: CLI flag > shape.toml `[build].output` > auto-generated filename.
#[cfg(test)]
fn compute_output_path(
    cli_output: Option<&PathBuf>,
    project_root: &std::path::Path,
    build_output: Option<&str>,
    bundle_filename: &str,
) -> PathBuf {
    if let Some(path) = cli_output {
        path.clone()
    } else if let Some(output_dir) = build_output {
        project_root.join(output_dir).join(bundle_filename)
    } else {
        PathBuf::from(bundle_filename)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_compute_output_path_cli_flag_wins() {
        let cli = PathBuf::from("/custom/output.shapec");
        let result = compute_output_path(
            Some(&cli),
            Path::new("/project"),
            Some("dist/"),
            "pkg-1.0.0.shapec",
        );
        assert_eq!(result, PathBuf::from("/custom/output.shapec"));
    }

    #[test]
    fn test_compute_output_path_build_output_used() {
        let result = compute_output_path(
            None,
            Path::new("/project"),
            Some("dist/"),
            "pkg-1.0.0.shapec",
        );
        assert_eq!(result, PathBuf::from("/project/dist/pkg-1.0.0.shapec"));
    }

    #[test]
    fn test_compute_output_path_fallback_to_filename() {
        let result = compute_output_path(None, Path::new("/project"), None, "pkg-1.0.0.shapec");
        assert_eq!(result, PathBuf::from("pkg-1.0.0.shapec"));
    }

    #[test]
    fn test_build_output_from_shape_toml() {
        let toml_str = r#"
[project]
name = "my-app"
version = "1.0.0"

[build]
output = "dist/"
"#;
        let config: shape_runtime::project::ShapeProject =
            shape_runtime::project::parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.build.output.as_deref(), Some("dist/"));
    }

    #[test]
    fn test_build_output_absent_is_none() {
        let toml_str = r#"
[project]
name = "my-app"
version = "1.0.0"
"#;
        let config: shape_runtime::project::ShapeProject =
            shape_runtime::project::parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.build.output, None);
    }
}
