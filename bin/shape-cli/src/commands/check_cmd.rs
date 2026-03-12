use anyhow::Result;
use std::path::PathBuf;

/// Run `shape check [path]` — validate a Shape file or project without executing.
pub async fn run_check(path: Option<PathBuf>) -> Result<()> {
    let path = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };

    let (source, display_path) = if path.is_dir() {
        // Project directory — find entry point from shape.toml
        let project = shape_runtime::project::find_project_root(&path)
            .ok_or_else(|| anyhow::anyhow!("No shape.toml found in '{}'", path.display()))?;

        let entry = project.config.project.entry.as_ref()
            .ok_or_else(|| anyhow::anyhow!(
                "shape.toml at '{}' has no [project].entry field",
                project.root_path.join("shape.toml").display()
            ))?;

        let entry_path = project.root_path.join(entry);
        let src = std::fs::read_to_string(&entry_path)
            .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", entry_path.display(), e))?;
        (src, entry_path)
    } else {
        let src = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", path.display(), e))?;
        (src, path)
    };

    let mut errors = 0u32;
    let warnings = 0u32;

    // Parse
    match shape_ast::parse_program(&source) {
        Ok(ast) => {
            // Compile (type-check) without executing
            let compiler = shape_vm::compiler::BytecodeCompiler::new();
            if let Err(e) = compiler.compile(&ast) {
                errors += 1;
                eprintln!(
                    "\x1b[31merror\x1b[0m: {} ({})",
                    e, display_path.display()
                );
            }
        }
        Err(e) => {
            errors += 1;
            eprintln!(
                "\x1b[31merror\x1b[0m: {} ({})",
                e, display_path.display()
            );
        }
    }

    // Summary
    if errors == 0 && warnings == 0 {
        eprintln!(
            "\x1b[32mcheck passed\x1b[0m: {} (no errors)",
            display_path.display()
        );
        Ok(())
    } else {
        eprintln!(
            "\x1b[31mcheck failed\x1b[0m: {} error(s), {} warning(s)",
            errors, warnings
        );
        std::process::exit(1);
    }
}
