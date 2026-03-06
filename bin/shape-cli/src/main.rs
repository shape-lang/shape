use anyhow::{Context, Result};
use clap::Parser;
use shape_runtime::initialize_shared_runtime;

// Generic chart data adapter (no market_data dependency)
pub mod chart_adapter;
pub mod chart_renderer;

// TUI REPL modules
pub mod repl;
pub mod ui;

// Configuration loading
pub mod config;
pub mod extension_loading;

// New modular structure
pub mod cli_args;
pub mod commands;
pub mod helpers;
pub mod module_loading;

use cli_args::{Cli, Commands};
use commands::{
    ProviderOptions, run_build, run_doctest, run_expand_comptime, run_ext_install, run_ext_list,
    run_ext_remove, run_jit_parity, run_keys_generate, run_keys_list, run_keys_trust, run_repl,
    run_schema_fetch, run_schema_status, run_script, run_sign, run_snapshot_delete,
    run_snapshot_info, run_snapshot_list, run_tree, run_tui, run_verify,
};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    initialize_shared_runtime().context("failed to initialize shared runtime")?;

    let cli = Cli::parse();

    if cli.expand && cli.file.is_none() && cli.command.is_none() {
        anyhow::bail!("--expand requires a script file: shape <file.shape> --expand");
    }
    if (cli.module.is_some() || cli.function.is_some()) && !cli.expand {
        anyhow::bail!(
            "--module/--function are only valid with --expand or the expand-comptime subcommand"
        );
    }

    let Cli {
        command,
        file,
        expand,
        module,
        function,
        mode,
        extensions,
        resume,
        providers_config,
        extension_dir,
    } = cli;

    // Build provider options from top-level CLI args
    let provider_opts = ProviderOptions {
        config_path: providers_config,
        extension_dir,
    };

    match (command, file) {
        // Explicit subcommands
        (Some(Commands::Run { script, opts }), _) => {
            let cli_args::RunCommandOptions {
                expand,
                resume,
                runtime,
                expand_filter,
            } = opts;
            let cli_args::RuntimeCommandOptions { mode, provider } = runtime;
            let cli_args::ProviderCommandOptions {
                extensions,
                providers_config,
                extension_dir,
            } = provider;
            let run_provider_opts = ProviderOptions {
                config_path: providers_config,
                extension_dir,
            };

            if expand {
                let script = script
                    .ok_or_else(|| anyhow::anyhow!("shape run --expand requires a script path"))?;
                run_expand_comptime(script, expand_filter.module, expand_filter.function).await?;
            } else {
                run_script(script, mode, extensions, &run_provider_opts, resume).await?;
            }
        }
        (Some(Commands::Repl { opts }), _) => {
            let cli_args::RuntimeCommandOptions { mode, provider } = opts;
            let cli_args::ProviderCommandOptions {
                extensions,
                providers_config,
                extension_dir,
            } = provider;
            let provider_opts = ProviderOptions {
                config_path: providers_config,
                extension_dir,
            };
            run_repl(mode, extensions, &provider_opts).await?;
        }
        (Some(Commands::Tui { opts }), _) => {
            let cli_args::RuntimeCommandOptions { mode, provider } = opts;
            let cli_args::ProviderCommandOptions {
                extensions,
                providers_config,
                extension_dir,
            } = provider;
            let provider_opts = ProviderOptions {
                config_path: providers_config,
                extension_dir,
            };
            run_tui(mode, extensions, &provider_opts).await?;
        }
        (Some(Commands::Doctest { path, verbose }), _) => {
            run_doctest(path, verbose).await?;
        }
        (Some(Commands::ExpandComptime { script, opts }), _) => {
            run_expand_comptime(script, opts.module, opts.function).await?;
        }
        (Some(Commands::Schema { action, opts }), _) => {
            let cli_args::ProviderCommandOptions {
                extensions,
                providers_config,
                extension_dir,
            } = opts;
            let schema_provider_opts = ProviderOptions {
                config_path: providers_config,
                extension_dir,
            };
            use cli_args::SchemaAction;
            match action {
                SchemaAction::Fetch { uri } => {
                    run_schema_fetch(uri, &schema_provider_opts, &extensions).await?
                }
                SchemaAction::Status => run_schema_status().await?,
            }
        }
        (Some(Commands::Snapshot { action }), _) => {
            use cli_args::SnapshotAction;
            match action {
                SnapshotAction::List => run_snapshot_list().await?,
                SnapshotAction::Info { hash } => run_snapshot_info(hash).await?,
                SnapshotAction::Delete { hash } => run_snapshot_delete(hash).await?,
            }
        }
        (Some(Commands::Tree { native }), _) => {
            run_tree(native).await?;
        }
        (Some(Commands::Ext { action }), _) => {
            use cli_args::ExtAction;
            match action {
                ExtAction::Install { name } => run_ext_install(name).await?,
                ExtAction::List => run_ext_list().await?,
                ExtAction::Remove { name } => run_ext_remove(name).await?,
            }
        }
        (Some(Commands::Jit { action }), _) => {
            use cli_args::JitAction;
            match action {
                JitAction::Parity {
                    builtins,
                    unsupported_only,
                } => run_jit_parity(builtins, unsupported_only).await?,
            }
        }
        (Some(Commands::Build { output, opt_level }), _) => {
            run_build(output, opt_level).await?;
        }
        (Some(Commands::Sign { bundle, key }), _) => {
            run_sign(bundle, key).await?;
        }
        (Some(Commands::Verify { bundle }), _) => {
            run_verify(bundle).await?;
        }
        (Some(Commands::Keys { action }), _) => {
            use cli_args::KeysAction;
            match action {
                KeysAction::Generate { output, name } => {
                    run_keys_generate(output, name).await?;
                }
                KeysAction::Trust {
                    public_key,
                    name,
                    scope,
                } => {
                    run_keys_trust(public_key, name, scope).await?;
                }
                KeysAction::List => {
                    run_keys_list().await?;
                }
            }
        }

        // File mode: `shape foo.shape`
        (None, Some(file)) => {
            if expand {
                run_expand_comptime(file, module, function).await?;
            } else {
                run_script(Some(file), mode, extensions, &provider_opts, resume).await?;
            }
        }

        // Resume-only mode: `shape --resume <hash>`
        (None, None) if resume.is_some() => {
            run_script(None, mode, extensions, &provider_opts, resume).await?;
        }

        // No subcommand, no file: project mode or REPL
        (None, None) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            if let Some(project) = shape_runtime::project::find_project_root(&cwd) {
                if let Some(entry) = &project.config.project.entry {
                    let entry_path = project.root_path.join(entry);
                    if entry_path.is_file() {
                        run_script(Some(entry_path), mode, extensions, &provider_opts, resume)
                            .await?;
                    } else {
                        anyhow::bail!(
                            "shape.toml entry '{}' not found (resolved to {})",
                            entry,
                            entry_path.display()
                        );
                    }
                } else {
                    anyhow::bail!(
                        "shape.toml is present at '{}' but [project].entry is missing; \
                         set `entry = \"src/main.shape\"` (or another script path) in [project]",
                        project.root_path.join("shape.toml").display()
                    );
                }
            } else {
                // No shape.toml — launch REPL
                run_repl(mode, extensions, &provider_opts).await?;
            }
        }
    }

    Ok(())
}
