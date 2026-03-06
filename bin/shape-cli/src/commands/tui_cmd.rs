use super::{ExecutionMode, ExecutionModeArg, ProviderOptions};
use crate::extension_loading;
use crate::repl;
use anyhow::{Context, Result};
use shape_runtime::engine::ShapeEngine;
use shape_runtime::snapshot::SnapshotStore;
use std::path::PathBuf;

/// Run the new TUI REPL with ratatui
pub async fn run_tui(
    mode: ExecutionModeArg,
    extensions: Vec<PathBuf>,
    provider_opts: &ProviderOptions,
) -> Result<()> {
    run_tui_repl(mode, extensions, provider_opts).await
}

/// Run the new TUI REPL with ratatui
async fn run_tui_repl(
    mode: ExecutionModeArg,
    extensions: Vec<PathBuf>,
    provider_opts: &ProviderOptions,
) -> Result<()> {
    let exec_mode = match mode {
        ExecutionModeArg::Vm => ExecutionMode::BytecodeVM,
        ExecutionModeArg::Jit => {
            #[cfg(feature = "jit")]
            {
                ExecutionMode::JIT
            }
            #[cfg(not(feature = "jit"))]
            {
                anyhow::bail!("JIT mode requires the 'jit' feature. Rebuild with --features jit");
            }
        }
    };
    // Create engine (data providers are loaded via extensions)
    let mut engine = ShapeEngine::new().context("failed to create Shape engine")?;

    engine
        .load_stdlib()
        .context("failed to load Shape stdlib")?;

    let mut project_root = None;
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(project) = shape_runtime::project::find_project_root(&cwd) {
            let module_paths = project.resolved_module_paths();
            engine
                .get_runtime_mut()
                .set_project_root(&project.root_path, &module_paths);
            project_root = Some(project);
        }
    }

    // Initialize REPL mode for persistent variable/function state
    engine.init_repl();

    // Enable snapshot store for checkpoint-based resumability
    let snapshot_root = dirs::data_local_dir()
        .map(|dir| dir.join("shape").join("snapshots"))
        .unwrap_or_else(|| PathBuf::from(".shape").join("snapshots"));
    let snapshot_store =
        SnapshotStore::new(snapshot_root).context("failed to create snapshot store")?;
    engine.enable_snapshot_store(snapshot_store);

    let startup_specs = extension_loading::collect_startup_specs(
        provider_opts,
        project_root.as_ref(),
        None,
        None,
        &extensions,
    );
    let _loaded = extension_loading::load_specs(
        &mut engine,
        &startup_specs,
        |spec, info| {
            eprintln!(
                "Loaded module: {} v{} (from {})",
                info.name,
                info.version,
                spec.source.label()
            );
        },
        |spec, err| {
            eprintln!("Failed to load module '{}': {}", spec.display_name(), err);
        },
    );

    let mut app = repl::ReplApp::new(engine, extensions, exec_mode);
    app.run().await
}
