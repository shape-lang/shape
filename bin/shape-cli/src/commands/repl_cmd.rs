use super::{ExecutionMode, ExecutionModeArg, ProviderOptions};
use crate::chart_renderer;
use crate::extension_loading;
use crate::helpers::{display_image_inline, theme_from_name};
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use rustyline::{
    Context as EditorContext, Editor, Helper,
    completion::{Completer, Pair},
    config::{Builder as ConfigBuilder, CompletionType, Config},
    error::ReadlineError,
    highlight::Highlighter,
    hint::{Hinter, HistoryHinter},
    history::FileHistory,
    validate::{ValidationContext, ValidationResult, Validator},
};
use shape_runtime::engine::{ExecutionResult, ShapeEngine};
use shape_viz_core::ChartConfig;
use shape_wire::{WireValue, render_wire_terminal};
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::{fs, task};

/// Run the REPL command
pub async fn run_repl(
    mode: ExecutionModeArg,
    extensions: Vec<PathBuf>,
    provider_opts: &ProviderOptions,
) -> Result<()> {
    Repl::bootstrap_with_options(mode, &extensions, provider_opts)?
        .run()
        .await
}

pub struct Repl {
    editor: Arc<Mutex<Editor<ReplHelper, FileHistory>>>,
    engine: ShapeEngine,
    history_path: Option<PathBuf>,
    last_result: Option<ExecutionResult>,
    execution_mode: ExecutionMode,
}

impl Repl {
    pub fn bootstrap_with_options(
        mode: ExecutionModeArg,
        extensions: &[PathBuf],
        provider_opts: &ProviderOptions,
    ) -> Result<Self> {
        let execution_mode = match mode {
            ExecutionModeArg::Vm => ExecutionMode::BytecodeVM,
            ExecutionModeArg::Jit => {
                #[cfg(feature = "jit")]
                {
                    ExecutionMode::JIT
                }
                #[cfg(not(feature = "jit"))]
                {
                    bail!(
                        "JIT mode requires the 'jit' feature. This is a Pro feature.\nRebuild with: cargo build --features jit"
                    );
                }
            }
        };

        let config = repl_editor_config();
        let helper = ReplHelper::default();
        let mut editor = Editor::<ReplHelper, FileHistory>::with_config(config)?;
        editor.set_helper(Some(helper));
        editor.bind_sequence(rustyline::KeyEvent::ctrl('l'), rustyline::Cmd::ClearScreen);

        let history_path = history_file();
        if let Some(path) = history_path.as_ref() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = editor.load_history(path);
        }

        // Create engine (data providers are loaded via extensions)
        let mut engine = ShapeEngine::new().context("failed to create Shape engine")?;

        engine
            .load_stdlib()
            .context("failed to load Shape stdlib")?;

        // Detect project root from current directory
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

        let startup_specs = extension_loading::collect_startup_specs(
            provider_opts,
            project_root.as_ref(),
            None,
            None,
            extensions,
        );
        let modules_loaded = extension_loading::load_specs(
            &mut engine,
            &startup_specs,
            |spec, info| {
                println!(
                    "  Loaded module: {} v{} (from {})",
                    info.name,
                    info.version,
                    spec.source.label()
                );
            },
            |spec, err| {
                eprintln!("  Failed to load module '{}': {}", spec.display_name(), err);
            },
        );

        let mode_str = match mode {
            ExecutionModeArg::Vm => "vm",
            ExecutionModeArg::Jit => "jit",
        };

        // Initialize REPL mode for persistent variable/function state
        engine.init_repl();

        if modules_loaded > 0 {
            println!(
                "Shape engine initialized (mode: {}, stdlib loaded, {} extension modules)",
                mode_str, modules_loaded
            );
        } else {
            println!(
                "Shape engine initialized (mode: {}, stdlib loaded)",
                mode_str
            );
        }

        Ok(Self {
            editor: Arc::new(Mutex::new(editor)),
            engine,
            history_path,
            last_result: None,
            execution_mode,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        println!("Shape REPL (type :help for commands)");
        loop {
            let line = Self::read_line(self.editor.clone()).await?;
            let Some(line) = line else {
                println!("bye");
                break;
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with(':') {
                if self.handle_command(trimmed).await? {
                    break;
                }
            } else if let Err(err) = self.execute_source(line.clone()).await {
                self.print_error(&err);
            } else if let Ok(mut editor) = self.editor.lock() {
                let _ = editor.add_history_entry(line);
            }
        }

        self.save_history();
        Ok(())
    }

    async fn read_line(
        editor: Arc<Mutex<Editor<ReplHelper, FileHistory>>>,
    ) -> Result<Option<String>> {
        let prompt = "shape> ".to_string();
        let line = task::spawn_blocking(move || {
            let mut editor = editor.lock().expect("editor mutex poisoned");
            editor.readline(&prompt)
        })
        .await?;

        match line {
            Ok(line) => Ok(Some(line)),
            Err(ReadlineError::Interrupted) => Ok(Some(String::new())),
            Err(ReadlineError::Eof) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    async fn execute_source(&mut self, source: String) -> Result<()> {
        // Still async to keep consistent interface, but execute() is sync internally
        let response = self.run_engine(&source).await?;
        self.print_execution_with_format(&response, &source);
        self.last_result = Some(response);
        Ok(())
    }

    pub async fn execute_file(&mut self, path: &Path) -> Result<()> {
        // Add the script's directory to module search paths
        if let Some(parent) = path.parent() {
            let parent = if parent.as_os_str().is_empty() {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            } else {
                parent
                    .canonicalize()
                    .unwrap_or_else(|_| parent.to_path_buf())
            };
            self.engine.get_runtime_mut().add_module_path(parent);
        }

        let content = fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        self.execute_source(content).await
    }

    async fn handle_command(&mut self, command: &str) -> Result<bool> {
        let mut parts = command.split_whitespace();
        let head = parts.next().unwrap_or("");
        match head {
            ":quit" | ":exit" => return Ok(true),
            ":help" => {
                Self::print_help();
            }
            ":load" => {
                if let Some(path) = parts.next() {
                    let path = PathBuf::from(path);
                    if let Err(err) = self.execute_file(&path).await {
                        eprintln!("failed to load {}: {err}", path.display());
                    }
                } else {
                    eprintln!("usage: :load <file>");
                }
            }
            ":reload-stdlib" => {
                if let Err(err) = self.reset_engine() {
                    eprintln!("failed to reload stdlib: {err}");
                } else {
                    println!("engine reset (stdlib reloaded)");
                }
            }
            ":plot" => {
                let remainder = command[5..].trim_start();
                if remainder.is_empty() {
                    if let Some(result) = self.last_result.clone() {
                        if let Err(err) = self.plot_execution(&result).await {
                            eprintln!("plot error: {err}");
                        }
                    } else {
                        eprintln!("no previous execution to plot");
                    }
                } else {
                    match self.run_engine(remainder).await {
                        Ok(exec_result) => {
                            self.print_execution_with_format(&exec_result, remainder);
                            if let Err(err) = self.plot_execution(&exec_result).await {
                                eprintln!("plot error: {err}");
                            }
                            self.last_result = Some(exec_result);
                        }
                        Err(err) => eprintln!("plot execution failed: {err}"),
                    }
                }
            }
            ":metrics" => {
                if let Some(result) = self.last_result.as_ref() {
                    Self::print_metrics(result);
                } else {
                    eprintln!("no previous execution to show metrics");
                }
            }
            ":equity" => {
                if let Some(result) = self.last_result.clone() {
                    if let Err(err) = self.plot_equity_curve(&result).await {
                        eprintln!("equity plot error: {err}");
                    }
                } else {
                    eprintln!("no previous execution to plot equity curve");
                }
            }
            ":load-extension" => {
                if let Some(path) = parts.next() {
                    let extension_path = PathBuf::from(path);
                    // Get optional config as JSON
                    let config: serde_json::Value = parts
                        .next()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_else(|| serde_json::json!({}));

                    match self.engine.load_extension(&extension_path, &config) {
                        Ok(info) => {
                            println!("Loaded extension: {} v{}", info.name, info.version);
                            println!("  Kind: {:?}", info.plugin_type);
                            println!("  Description: {}", info.description);
                            if !info.capabilities.is_empty() {
                                let caps = info
                                    .capabilities
                                    .iter()
                                    .map(|cap| {
                                        format!("{}:{} ({:?})", cap.contract, cap.version, cap.kind)
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                println!("  Capabilities: {}", caps);
                            }

                            // Show query schema
                            if let Some(schema) = self.engine.get_extension_query_schema(&info.name)
                            {
                                if !schema.params.is_empty() {
                                    println!("  Query parameters:");
                                    for param in &schema.params {
                                        let required =
                                            if param.required { " (required)" } else { "" };
                                        println!(
                                            "    - {}: {:?}{}",
                                            param.name, param.param_type, required
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to load extension: {}", e);
                        }
                    }
                } else {
                    eprintln!("usage: :load-extension <path> [config_json]");
                    eprintln!("example: :load-extension ./libshape_ext_csv.so");
                    eprintln!(
                        "example: :load-extension ./libmydata.so '{{\"base_dir\":\"/data\"}}'"
                    );
                }
            }
            ":unload-extension" => {
                if let Some(name) = parts.next() {
                    if self.engine.unload_extension(name) {
                        println!("Unloaded extension: {}", name);
                    } else {
                        eprintln!("Extension not found: {}", name);
                    }
                } else {
                    eprintln!("usage: :unload-extension <name>");
                }
            }
            ":extensions" => {
                let extensions = self.engine.list_extensions();
                if extensions.is_empty() {
                    println!("No extensions loaded");
                } else {
                    println!("Loaded extensions:");
                    for name in extensions {
                        if let Some(schema) = self.engine.get_extension_query_schema(&name) {
                            let params: Vec<_> = schema
                                .params
                                .iter()
                                .filter(|p| p.required)
                                .map(|p| p.name.as_str())
                                .collect();
                            if params.is_empty() {
                                println!("  {} (no required params)", name);
                            } else {
                                println!("  {} (required: {})", name, params.join(", "));
                            }
                        } else {
                            println!("  {}", name);
                        }
                    }
                }
            }
            _ => {
                eprintln!("unknown command: {head}");
            }
        }
        Ok(false)
    }

    fn save_history(&self) {
        if let Some(path) = self.history_path.as_ref() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(mut editor) = self.editor.lock() {
                let _ = editor.save_history(path);
            }
        }
    }

    fn print_error(&mut self, err: &anyhow::Error) {
        let runtime_error = self.engine.get_runtime_mut().take_last_runtime_error();

        // Check if this is a Shape error with rich formatting available
        if let Some(shape_err) = err.downcast_ref::<shape_runtime::error::ShapeError>() {
            match shape_err {
                shape_runtime::error::ShapeError::StructuredParse(structured) => {
                    // Use the CLI renderer for rich parse error output
                    use shape_runtime::error::{CliErrorRenderer, ErrorRenderer};
                    let renderer = CliErrorRenderer::with_colors();
                    eprintln!("{}", renderer.render(structured));
                    return;
                }
                shape_runtime::error::ShapeError::SemanticError { location, .. }
                    if location.is_some() =>
                {
                    // Use format_with_source for semantic errors with location info
                    eprintln!("{}", shape_err.format_with_source());
                    return;
                }
                shape_runtime::error::ShapeError::RuntimeError { location, .. } => {
                    if let Some(runtime_error) = runtime_error.as_ref()
                        && let Some(rendered) = render_wire_terminal(runtime_error)
                    {
                        eprintln!("{rendered}");
                        return;
                    }
                    if location.is_some() {
                        eprintln!("{}", shape_err.format_with_source());
                    } else {
                        eprintln!("error: {shape_err}");
                    }
                    return;
                }
                shape_runtime::error::ShapeError::MultiError(errors) => {
                    for (i, sub_err) in errors.iter().enumerate() {
                        if i > 0 {
                            eprintln!();
                        }
                        self.print_shape_error_inner(sub_err, runtime_error.as_ref());
                    }
                    return;
                }
                _ => {}
            }
        }
        // Fall back to simple error display
        eprintln!("error: {err}");
    }

    fn print_shape_error_inner(
        &self,
        shape_err: &shape_runtime::error::ShapeError,
        runtime_error: Option<&WireValue>,
    ) {
        use shape_runtime::error::{CliErrorRenderer, ErrorRenderer, ShapeError};

        match shape_err {
            ShapeError::StructuredParse(structured) => {
                let renderer = CliErrorRenderer::with_colors();
                eprintln!("{}", renderer.render(structured));
            }
            ShapeError::SemanticError { location, .. } if location.is_some() => {
                eprintln!("{}", shape_err.format_with_source());
            }
            ShapeError::RuntimeError { location, .. } => {
                if let Some(runtime_error) = runtime_error
                    && let Some(rendered) = render_wire_terminal(runtime_error)
                {
                    eprintln!("{rendered}");
                } else if location.is_some() {
                    eprintln!("{}", shape_err.format_with_source());
                } else {
                    eprintln!("error: {shape_err}");
                }
            }
            ShapeError::MultiError(errors) => {
                for (i, sub_err) in errors.iter().enumerate() {
                    if i > 0 {
                        eprintln!();
                    }
                    self.print_shape_error_inner(sub_err, runtime_error);
                }
            }
            _ => {
                eprintln!("error: {shape_err}");
            }
        }
    }

    fn reset_engine(&mut self) -> Result<()> {
        // Create engine (data providers are loaded via extensions)
        let mut engine = ShapeEngine::new().context("failed to create Shape engine")?;

        engine
            .load_stdlib()
            .context("failed to load Shape stdlib")?;
        self.engine = engine;
        self.last_result = None;
        Ok(())
    }

    async fn run_engine(&mut self, source: &str) -> Result<ExecutionResult> {
        // Use execute_repl to maintain persistent state across commands
        let response = match self.execution_mode {
            ExecutionMode::BytecodeVM => {
                let mut executor = shape_vm::BytecodeExecutor::new();
                extension_loading::register_extension_capability_modules(
                    &self.engine,
                    &mut executor,
                );
                let module_info = executor.module_schemas();
                self.engine.register_extension_modules(&module_info);
                self.engine.register_language_runtime_artifacts();
                let current_file = std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join("__shape_repl__.shape");
                crate::module_loading::wire_vm_executor_module_loading(
                    &mut self.engine,
                    &mut executor,
                    Some(&current_file),
                    Some(source),
                )?;
                self.engine.execute_repl(&mut executor, source).await?
            }
            #[cfg(feature = "jit")]
            ExecutionMode::JIT => {
                let mut executor = shape_jit::JITExecutor;
                self.engine.execute_repl(&mut executor, source).await?
            }
        };
        Ok(response)
    }

    fn print_execution_with_format(&mut self, response: &ExecutionResult, source: &str) {
        use shape_wire::{ValueEnvelope, WireValue};
        use std::collections::HashMap;

        for message in &response.messages {
            let level = match message.level {
                shape_runtime::engine::MessageLevel::Info => "info",
                shape_runtime::engine::MessageLevel::Warning => "warning",
                shape_runtime::engine::MessageLevel::Error => "error",
            };
            println!("[{}] {}", level, message.text);
        }

        // Create wire envelope for the value
        let wire_value = response.value.clone();

        // Suppress output for Null/Unit - these represent void/no value
        // This handles print() return values and other unit-returning statements
        if matches!(&wire_value, WireValue::Null) {
            return;
        }

        // Special handling for PrintResult - print raw string directly
        if let WireValue::PrintResult(result) = &wire_value {
            println!("{}", result.rendered);
            return;
        }

        let envelope = if let Some(type_info) = &response.type_info {
            // Use metadata from execution result
            // Build type registry for this type
            let type_registry = match type_info.name.as_str() {
                "Number" | "number" => shape_wire::metadata::TypeRegistry::for_number(),
                "Timestamp" => shape_wire::metadata::TypeRegistry::for_timestamp(),
                _ => shape_wire::metadata::TypeRegistry::default_for_primitives(),
            };
            ValueEnvelope::new(wire_value.clone(), type_info.clone(), type_registry)
        } else {
            ValueEnvelope::from_value(wire_value.clone())
        };

        // Check for format hint
        let format_hint = self.extract_format_hint_from_source(source);
        let format_to_use = format_hint.as_deref().or(Some(envelope.default_format()));

        // Try to format using Shape runtime (for Number types)
        let formatted = match &envelope.value {
            WireValue::Number(n) => {
                // Use Shape runtime formatting
                self.engine
                    .format_value_string(
                        *n,
                        &envelope.type_info.name,
                        format_to_use,
                        &HashMap::new(),
                    )
                    .ok()
            }
            _ => {
                // Use wire fallback for other types
                format_to_use.and_then(|fmt| envelope.format(fmt, &HashMap::new()).ok())
            }
        };

        // Print formatted value or fallback to JSON
        match formatted {
            Some(s) => println!("{}", s),
            None => match serde_json::to_string_pretty(&wire_value) {
                Ok(json) => println!("{}", json),
                Err(_) => println!("{:?}", wire_value),
            },
        }
    }

    /// Extract format hint from source code
    ///
    /// Handles:
    /// 1. Variable declarations: let rate: Number @ Percent = 0.0523
    /// 2. Variable references: rate
    /// 3. Print statements: print(rate)
    fn extract_format_hint_from_source(&self, source: &str) -> Option<String> {
        let trimmed = source.trim();

        // Case 1: Variable declaration - get hint from type annotation
        if trimmed.starts_with("let ")
            || trimmed.starts_with("var ")
            || trimmed.starts_with("const ")
        {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let var_name = parts[1].trim_end_matches(':');
                return self.engine.get_variable_format_hint(var_name);
            }
        }

        // Case 2: Simple variable reference
        let is_ident = trimmed.chars().all(|c| c.is_alphanumeric() || c == '_');
        if is_ident && !trimmed.is_empty() {
            return self.engine.get_variable_format_hint(trimmed);
        }

        // Case 3: print(varname)
        if trimmed.starts_with("print(") && trimmed.ends_with(')') {
            let arg = &trimmed[6..trimmed.len() - 1].trim();
            let is_arg_ident = arg.chars().all(|c| c.is_alphanumeric() || c == '_');
            if is_arg_ident && !arg.is_empty() {
                return self.engine.get_variable_format_hint(arg);
            }
        }

        None
    }

    fn print_metrics(response: &ExecutionResult) {
        println!("[{:?}]", response.execution_type);
        println!(
            "metrics: parse={}μs analyze={}μs runtime={}μs total={}μs",
            response.metrics.parse_time_ms * 1000,
            response.metrics.analysis_time_ms * 1000,
            response.metrics.runtime_time_ms * 1000,
            response.metrics.execution_time_ms * 1000
        );
    }

    fn print_help() {
        println!("commands:");
        println!("  :help                    show this help");
        println!("  :load <file>             execute a Shape script from file");
        println!("  :reload-stdlib           reload the bundled stdlib");
        println!("  :plot [query]            render latest or supplied query result as chart");
        println!("  :metrics                 show execution metrics from last result");
        println!("  :equity                  plot equity curve from last backtest");
        println!();
        println!("extension commands:");
        println!("  :load-extension <path>   load an extension from shared library");
        println!("  :unload-extension <name> unload an extension by name");
        println!("  :extensions              list loaded extensions");
        println!();
        println!("  :quit / :exit            exit the REPL");
        println!();
        println!("enter Shape statements to execute them immediately.");
    }

    async fn plot_execution(&self, response: &ExecutionResult) -> Result<()> {
        let value = &response.value;
        let type_info = response.type_info.clone();

        println!(
            "Plotting result of type: {:?}",
            type_info
                .as_ref()
                .map(|t| t.name.as_str())
                .unwrap_or("Unknown")
        );

        // Use dynamic renderer if metadata is present
        if let (Some(type_info), shape_wire::WireValue::Table(_)) = (&type_info, value) {
            if type_info.metadata.is_some() {
                let mut config = ChartConfig::default();
                config.width = 1000;
                config.height = 600;
                config.theme = theme_from_name("reference-dark");

                let renderer = chart_renderer::DynamicChartRenderer::new(config.clone());
                let buffer = renderer.render(value, type_info).await?;
                display_image_inline(buffer, config.width, config.height)?;
                return Ok(());
            }
        }

        Ok(())
    }

    async fn plot_equity_curve(&self, _response: &ExecutionResult) -> Result<()> {
        // Equity curve plotting should be handled via meta blocks now
        println!("Equity curve plotting is now handled via meta blocks. Use :plot instead.");
        Ok(())
    }
}

// =============================================================================
// REPL Helper - Completion, Highlighting, Validation
// =============================================================================

fn repl_editor_config() -> Config {
    ConfigBuilder::default()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .build()
}

fn history_file() -> Option<PathBuf> {
    dirs::data_dir()
        .or_else(dirs::home_dir)
        .map(|base| base.join("shape").join("history"))
}

struct ReplHelper {
    keywords: &'static [&'static str],
    commands: &'static [&'static str],
    hinter: HistoryHinter,
}

impl Default for ReplHelper {
    fn default() -> Self {
        Self {
            keywords: KEYWORDS,
            commands: COMMANDS,
            hinter: HistoryHinter {},
        }
    }
}

impl Helper for ReplHelper {}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &EditorContext<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (start, fragment) = current_fragment(line, pos);
        let mut matches = Vec::new();
        for candidate in self.keywords.iter().chain(self.commands.iter()) {
            if candidate.starts_with(fragment) {
                matches.push(Pair {
                    display: candidate.to_string(),
                    replacement: candidate.to_string(),
                });
            }
        }
        Ok((start, matches))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &EditorContext<'_>) -> Option<String> {
        self.hinter.hint(line, pos, ctx)
    }
}

impl Highlighter for ReplHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if line.is_empty() {
            return Cow::Borrowed(line);
        }

        let mut buf = String::with_capacity(line.len());
        let mut last = 0;

        for m in KEYWORD_REGEX.find_iter(line) {
            buf.push_str(&line[last..m.start()]);
            buf.push_str("\x1b[1;36m");
            buf.push_str(m.as_str());
            buf.push_str("\x1b[0m");
            last = m.end();
        }

        buf.push_str(&line[last..]);
        Cow::Owned(buf)
    }
}

impl Validator for ReplHelper {
    fn validate(&self, _ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(ValidationResult::Valid(None))
    }
}

fn current_fragment(line: &str, pos: usize) -> (usize, &str) {
    let slice = &line[..pos];
    let start = slice
        .rfind(|c: char| c.is_whitespace())
        .map(|idx| idx + 1)
        .unwrap_or(0);
    (start, &line[start..pos])
}

static KEYWORDS: &[&str] = &[
    "let", "var", "const", "fn", "return", "if", "else", "for", "while", "when", "match", "true",
    "false", "null", "query", "find", "scan", "analyze", "backtest", "alert", "module", "import",
    "export", "extend", "method", "stream", "pattern",
];

static COMMANDS: &[&str] = &[
    ":help",
    ":quit",
    ":exit",
    ":load",
    ":reload-stdlib",
    ":plot",
    ":metrics",
    ":equity",
    ":load-extension",
    ":unload-extension",
    ":extensions",
];

static KEYWORD_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(let|var|const|fn|query|find|scan|analyze|backtest|alert|module|import|export|extend|method|stream|pattern|return|if|else|when)\b").unwrap()
});
