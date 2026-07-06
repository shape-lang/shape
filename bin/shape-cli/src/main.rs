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
pub mod diagnostics_json;
pub mod helpers;
pub mod module_loading;
pub mod registry_client;

use cli_args::{Cli, Commands};
use commands::{
    ProviderOptions, run_add, run_build, run_check, run_doctest, run_expand_comptime,
    run_ext_install, run_ext_list, run_ext_remove, run_info, run_jit_parity, run_keys_generate,
    run_keys_list, run_keys_trust, run_login, run_publish, run_register, run_remove, run_repl,
    run_schema_fetch, run_schema_status, run_script, run_search, run_serve, run_sign,
    run_snapshot_delete, run_snapshot_info, run_snapshot_list, run_tree, run_tui, run_verify,
    run_wire_serve,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Cluster-2 closure-wave-F (2026-05-16) — install the tracing subscriber
    // BEFORE env_logger so the `--trace-jit` flag wins the global logger /
    // dispatcher slot. Per ADR-006 §2.7.5 amendment: tracing is internal-
    // Rust-side only; the extension contract is unaffected. The subscriber
    // filter mirrors the legacy SHAPE_JIT_* env-var gating but with per-
    // target composability (`shape_jit::mir=trace,shape_jit::arc_counters=
    // info`). When the flag is absent the default env_logger path runs
    // unchanged below; the tracing macros at the emission sites still
    // compile away to no-ops under feature-OFF.
    #[cfg(feature = "jit-trace")]
    let trace_jit_installed = if let Some(filter_directive) = cli.trace_jit.as_ref() {
        use tracing_subscriber::{EnvFilter, fmt};
        let directive: String = if filter_directive.is_empty() {
            "shape_jit=debug".to_string()
        } else {
            filter_directive.clone()
        };
        let env_filter = EnvFilter::try_new(&directive)
            .with_context(|| format!("invalid --trace-jit filter directive: {directive}"))?;
        fmt()
            .with_env_filter(env_filter)
            .with_writer(std::io::stderr)
            .with_target(true)
            .compact()
            .try_init()
            .map_err(|e| anyhow::anyhow!("failed to install JIT tracing subscriber: {e}"))?;
        true
    } else {
        false
    };

    // env_logger initializes the `log` facade. Skip when tracing-subscriber
    // is installed so the two don't fight for the global logger slot.
    #[cfg(feature = "jit-trace")]
    {
        if !trace_jit_installed {
            env_logger::init();
        }
    }
    #[cfg(not(feature = "jit-trace"))]
    {
        env_logger::init();
    }

    if cli.expand && cli.file.is_none() && cli.command.is_none() {
        anyhow::bail!("--expand requires a script file: shape <file.shape> --expand");
    }
    if (cli.module.is_some() || cli.function.is_some()) && !cli.expand {
        anyhow::bail!(
            "--module/--function are only valid with --expand or the expand-comptime subcommand"
        );
    }

    if should_initialize_shared_runtime_before_dispatch(&cli) {
        initialize_shared_runtime().context("failed to initialize shared runtime")?;
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
        #[cfg(feature = "jit-trace")]
            trace_jit: _,
    } = cli;

    // Build provider options from top-level CLI args
    let provider_opts = ProviderOptions {
        config_path: providers_config,
        extension_dir,
        ..Default::default()
    };

    match (command, file) {
        // Explicit subcommands
        (Some(Commands::Run { script, opts }), _) => {
            let cli_args::RunCommandOptions {
                expand,
                diagnostics,
                resume,
                eager_link,
                runtime,
                expand_filter,
                limits,
            } = opts;
            // Select the process-wide diagnostic renderer before any compile
            // runs, so both the compile-error path and any non-fatal comptime
            // warning surface in the requested format.
            shape_diagnostics::set_output_format(diagnostics.into());
            let cli_args::RuntimeCommandOptions { mode, provider } = runtime;
            let cli_args::ProviderCommandOptions {
                extensions,
                providers_config,
                extension_dir,
            } = provider;
            let run_provider_opts = ProviderOptions {
                config_path: providers_config,
                extension_dir,
                ..Default::default()
            };

            if expand {
                let script = script
                    .ok_or_else(|| anyhow::anyhow!("shape run --expand requires a script path"))?;
                run_expand_comptime(script, expand_filter.module, expand_filter.function).await?;
            } else {
                let cli_limits = resource_limits_from_flags(&limits);
                run_script(
                    script,
                    mode,
                    extensions,
                    &run_provider_opts,
                    resume,
                    cli_limits,
                    eager_link,
                )
                .await?;
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
                ..Default::default()
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
                ..Default::default()
            };
            run_tui(mode, extensions, &provider_opts).await?;
        }
        (Some(Commands::Check { path, link }), _) => {
            run_check(path, link).await?;
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
                ..Default::default()
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
                ExtAction::Install { name, version } => run_ext_install(name, version).await?,
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

        (Some(Commands::Register { registry }), _) => {
            run_register(registry).await?;
        }
        (Some(Commands::Login { token, registry }), _) => {
            run_login(token, registry).await?;
        }
        (
            Some(Commands::Publish {
                registry,
                key,
                no_sign,
                no_source,
                native,
            }),
            _,
        ) => {
            run_publish(registry, key, no_sign, no_source, native).await?;
        }
        (Some(Commands::Add { name, version }), _) => {
            run_add(name, version).await?;
        }
        (Some(Commands::Remove { name }), _) => {
            run_remove(name).await?;
        }
        (Some(Commands::Search { query }), _) => {
            run_search(query).await;
        }
        (Some(Commands::Info { name }), _) => {
            run_info(name).await;
        }
        (Some(Commands::WireServe { address, opts }), _) => {
            let cli_args::RuntimeCommandOptions { mode, provider } = opts;
            let cli_args::ProviderCommandOptions {
                extensions,
                providers_config,
                extension_dir,
            } = provider;
            let provider_opts = ProviderOptions {
                config_path: providers_config,
                extension_dir,
                ..Default::default()
            };
            run_wire_serve(address, mode, extensions, &provider_opts).await?;
        }
        (
            Some(Commands::Serve {
                address,
                tls_cert,
                tls_key,
                auth_token,
                sandbox,
                max_concurrent,
                ffi_languages,
                opts,
            }),
            _,
        ) => {
            let cli_args::RuntimeCommandOptions { mode, provider } = opts;
            let cli_args::ProviderCommandOptions {
                extensions,
                providers_config,
                extension_dir,
            } = provider;
            let provider_opts = ProviderOptions {
                config_path: providers_config,
                extension_dir,
                ..Default::default()
            };
            run_serve(
                address,
                mode,
                extensions,
                &provider_opts,
                tls_cert,
                tls_key,
                auth_token,
                sandbox,
                max_concurrent,
                ffi_languages,
            )
            .await?;
        }

        // File mode: `shape foo.shape`
        (None, Some(file)) => {
            if expand {
                run_expand_comptime(file, module, function).await?;
            } else {
                run_script(
                    Some(file),
                    mode,
                    extensions,
                    &provider_opts,
                    resume,
                    shape_vm::resource_limits::ResourceLimits::unlimited(),
                    false,
                )
                .await?;
            }
        }

        // Resume-only mode: `shape --resume <hash>`
        (None, None) if resume.is_some() => {
            run_script(
                None,
                mode,
                extensions,
                &provider_opts,
                resume,
                shape_vm::resource_limits::ResourceLimits::unlimited(),
                    false,
            )
            .await?;
        }

        // No subcommand, no file: project mode or REPL
        (None, None) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let project_result = shape_runtime::project::try_find_project_root(&cwd)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            if let Some(project) = project_result {
                if let Some(entry) = &project.config.project.entry {
                    let entry_path = project.root_path.join(entry);
                    if entry_path.is_file() {
                        run_script(
                            Some(entry_path),
                            mode,
                            extensions,
                            &provider_opts,
                            resume,
                            shape_vm::resource_limits::ResourceLimits::unlimited(),
                    false,
                        )
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
                initialize_shared_runtime().context("failed to initialize shared runtime")?;
                run_repl(mode, extensions, &provider_opts).await?;
            }
        }
    }

    Ok(())
}

/// Build `ResourceLimits` from the `shape run` resource-limit flags (WF-1D).
/// Unset flags stay `None` (unlimited); a set flag installs a finite cap the
/// dispatch loop / alloc-budget enforce in-process.
fn resource_limits_from_flags(
    opts: &cli_args::ResourceLimitOptions,
) -> shape_vm::resource_limits::ResourceLimits {
    shape_vm::resource_limits::ResourceLimits {
        max_instructions: opts.max_instructions,
        max_memory_bytes: opts.max_memory_bytes,
        max_wall_time: opts.max_time_ms.map(std::time::Duration::from_millis),
        max_output_bytes: opts.max_output_bytes,
    }
}

fn should_initialize_shared_runtime_before_dispatch(cli: &Cli) -> bool {
    match &cli.command {
        Some(Commands::Run { .. }) | Some(Commands::ExpandComptime { .. }) => false,
        None => false,
        Some(_) => true,
    }
}
