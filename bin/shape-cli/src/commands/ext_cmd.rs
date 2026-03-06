use anyhow::{Context, Result};
use std::path::PathBuf;

/// Default directory for globally installed extensions: ~/.shape/extensions/
pub fn default_extensions_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".shape").join("extensions"))
}

pub async fn run_ext_install(name: String) -> Result<()> {
    let crate_name = format!("shape-ext-{name}");
    let lib_name = crate_name.replace('-', "_");

    let ext_dir =
        default_extensions_dir().context("could not determine home directory")?;
    std::fs::create_dir_all(&ext_dir)?;

    println!("Installing extension '{name}' (crate: {crate_name})...");

    // Temp build directory
    let build_dir = std::env::temp_dir().join(format!("shape-ext-build-{name}"));
    if build_dir.exists() {
        std::fs::remove_dir_all(&build_dir)?;
    }
    std::fs::create_dir_all(&build_dir)?;

    // Write Cargo.toml — the wrapper crate produces a cdylib that includes the
    // extension's #[no_mangle] symbols.
    let cargo_toml = format!(
        r#"[package]
name = "_shape-ext-build"
version = "0.0.0"
edition = "2021"

[lib]
name = "{lib_name}"
crate-type = ["cdylib"]
path = "lib.rs"

[dependencies]
{crate_name} = "*"
"#
    );
    std::fs::write(build_dir.join("Cargo.toml"), &cargo_toml)?;

    // Force linkage so #[no_mangle] symbols from the dep end up in the cdylib.
    let lib_rs = format!("extern crate {lib_name};\n");
    std::fs::write(build_dir.join("lib.rs"), &lib_rs)?;

    // Shared target dir so repeated installs reuse cached deps.
    let cache_dir = dirs::home_dir()
        .context("could not determine home directory")?
        .join(".shape")
        .join("cache")
        .join("ext-build");

    let status = std::process::Command::new("cargo")
        .args(["build", "--release"])
        .env("CARGO_TARGET_DIR", &cache_dir)
        .current_dir(&build_dir)
        .status()
        .context(
            "failed to run cargo — is the Rust toolchain installed? \
             Install via https://rustup.rs",
        )?;

    // Clean up temp source dir (keep shared target dir for caching)
    let _ = std::fs::remove_dir_all(&build_dir);

    if !status.success() {
        anyhow::bail!("failed to build extension '{name}'");
    }

    // Copy the built cdylib to the extensions directory.
    let so_filename = format!(
        "{}{}{}",
        std::env::consts::DLL_PREFIX,
        lib_name,
        std::env::consts::DLL_SUFFIX,
    );
    let built_so = cache_dir.join("release").join(&so_filename);
    let target_path = ext_dir.join(&so_filename);

    std::fs::copy(&built_so, &target_path).with_context(|| {
        format!(
            "failed to copy {} to {}",
            built_so.display(),
            target_path.display()
        )
    })?;

    println!("Extension '{name}' installed to {}", target_path.display());
    Ok(())
}

pub async fn run_ext_list() -> Result<()> {
    let ext_dir = match default_extensions_dir() {
        Some(d) if d.is_dir() => d,
        _ => {
            println!("No extensions installed.");
            return Ok(());
        }
    };

    let mut found = false;
    let mut entries: Vec<_> = std::fs::read_dir(&ext_dir)?
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();

    for path in entries {
        if is_shared_lib(&path) {
            if !found {
                println!("Installed extensions:");
                found = true;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?");
            let display_name = stem
                .strip_prefix("libshape_ext_")
                .or_else(|| stem.strip_prefix("shape_ext_"))
                .unwrap_or(stem);
            println!("  {display_name:20} {}", path.display());
        }
    }

    if !found {
        println!("No extensions installed.");
    }
    Ok(())
}

pub async fn run_ext_remove(name: String) -> Result<()> {
    let ext_dir =
        default_extensions_dir().context("could not determine home directory")?;

    let lib_name = format!("shape_ext_{name}");
    let so_filename = format!(
        "{}{}{}",
        std::env::consts::DLL_PREFIX,
        lib_name,
        std::env::consts::DLL_SUFFIX,
    );
    let target_path = ext_dir.join(&so_filename);

    if target_path.exists() {
        std::fs::remove_file(&target_path)?;
        println!("Removed extension '{name}'");
    } else {
        println!(
            "Extension '{name}' is not installed (looked for {})",
            target_path.display()
        );
    }
    Ok(())
}

fn is_shared_lib(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext == "so" || ext == "dylib" || ext == "dll")
        .unwrap_or(false)
}
