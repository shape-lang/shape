use anyhow::{Context, Result};
use std::path::PathBuf;

/// Run the `shape build` command: compile a package into a .shapec bundle.
pub async fn run_build(output: Option<PathBuf>, _opt_level: u8) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;

    let project = shape_runtime::project::find_project_root(&cwd).ok_or_else(|| {
        anyhow::anyhow!("No shape.toml found. Run `shape build` from within a Shape project.")
    })?;

    eprintln!(
        "Building package '{}' v{}...",
        project.config.project.name, project.config.project.version
    );

    let bundle = shape_vm::bundle_compiler::BundleCompiler::compile(&project)
        .map_err(|e| anyhow::anyhow!("Build failed: {}", e))?;

    let output_path = output.unwrap_or_else(|| {
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
            PathBuf::from(format!("{}-{}.shapec", name, version))
        } else {
            let host = if bundle.metadata.build_host.trim().is_empty() {
                format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
            } else {
                bundle.metadata.build_host.clone()
            };
            PathBuf::from(format!("{}-{}-{}.shapec", name, version, host))
        }
    });

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
