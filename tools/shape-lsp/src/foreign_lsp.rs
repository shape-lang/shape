//! Foreign language block LSP support.
//!
//! Provides position mapping, virtual document generation, type annotation mapping,
//! and child LSP process management for foreign function blocks. This enables
//! delegated LSP features (completions, diagnostics, hover) from child language
//! servers like pyright.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use shape_ast::ast::{ForeignFunctionDef, Item, Span};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, GotoDefinitionResponse,
    Hover, Location, LocationLink, Position, Range, SemanticTokens, SignatureHelp, Uri,
};

// ---------------------------------------------------------------------------
// PositionMap
// ---------------------------------------------------------------------------

/// Maps positions between a Shape source file and a virtual foreign document.
#[derive(Clone, Debug)]
pub struct PositionMap {
    /// For each line in the virtual document, a mapping back to Shape source
    /// coordinates (or None for synthetic lines like generated headers).
    line_mapping: Vec<Option<ForeignLineMapping>>,
    /// Byte offset of the foreign body start in the Shape source.
    body_start_offset: usize,
    /// Number of synthetic header lines prepended to the virtual document.
    header_lines: u32,
}

#[derive(Clone, Copy, Debug)]
struct ForeignLineMapping {
    source_line: u32,
    source_col_start: u32,
    virtual_col_start: u32,
}

impl PositionMap {
    pub fn new(body_span: Span, source: &str) -> Self {
        let body_start_offset = body_span.start;
        let mut line_mapping = Vec::new();

        let (body_start_line, body_start_col) =
            crate::util::offset_to_line_col(source, body_span.start.min(source.len()));

        let body_end = body_span.end.min(source.len());
        let body_text = &source[body_span.start.min(source.len())..body_end];
        for (i, _) in body_text.split('\n').enumerate() {
            line_mapping.push(Some(ForeignLineMapping {
                source_line: body_start_line + i as u32,
                source_col_start: if i == 0 { body_start_col } else { 0 },
                virtual_col_start: 0,
            }));
        }

        Self {
            line_mapping,
            body_start_offset,
            header_lines: 0,
        }
    }

    /// Map a Shape source position to virtual document position, if inside the body.
    pub fn shape_to_virtual(&self, pos: Position) -> Option<Position> {
        for (virtual_line, mapping) in self.line_mapping.iter().enumerate() {
            if let Some(mapping) = mapping {
                if mapping.source_line == pos.line {
                    let character = if pos.character >= mapping.source_col_start {
                        mapping.virtual_col_start + (pos.character - mapping.source_col_start)
                    } else {
                        mapping
                            .virtual_col_start
                            .saturating_sub(mapping.source_col_start - pos.character)
                    };
                    return Some(Position {
                        line: virtual_line as u32,
                        character,
                    });
                }
            }
        }
        None
    }

    /// Map a virtual document position back to Shape source position.
    pub fn virtual_to_shape(&self, pos: Position) -> Option<Position> {
        let virtual_line = pos.line as usize;
        if virtual_line < self.line_mapping.len() {
            if let Some(mapping) = self.line_mapping[virtual_line] {
                let character = if pos.character >= mapping.virtual_col_start {
                    mapping.source_col_start + (pos.character - mapping.virtual_col_start)
                } else {
                    mapping
                        .source_col_start
                        .saturating_sub(mapping.virtual_col_start - pos.character)
                };
                return Some(Position {
                    line: mapping.source_line,
                    character,
                });
            }
        }
        None
    }

    /// Map a virtual document range back to a Shape source range.
    pub fn virtual_range_to_shape(&self, range: Range) -> Option<Range> {
        let start = self.virtual_to_shape(range.start)?;
        let end = self.virtual_to_shape(range.end)?;
        Some(Range { start, end })
    }

    /// The byte offset where the foreign body starts in the Shape source.
    pub fn body_start_offset(&self) -> usize {
        self.body_start_offset
    }

    /// Number of synthetic header lines in the virtual document.
    pub fn header_lines(&self) -> u32 {
        self.header_lines
    }
}

// ---------------------------------------------------------------------------
// VirtualDocument
// ---------------------------------------------------------------------------

/// A virtual foreign language document generated from a foreign function block.
pub struct VirtualDocument {
    /// The virtual document content (e.g., a complete Python file).
    pub content: String,
    /// Position mapping between Shape source and virtual document.
    pub position_map: PositionMap,
    /// The foreign language identifier (e.g., "python").
    pub language: String,
    /// The function name in the Shape source.
    pub function_name: String,
    /// URI of the Shape source file this was extracted from.
    pub source_uri: String,
    /// Virtual file path used for the child LSP.
    pub virtual_path: PathBuf,
}

/// Generate a virtual document from a foreign function definition.
pub fn generate_virtual_document(
    def: &ForeignFunctionDef,
    source: &str,
    source_uri: &str,
    workspace_dir: &Path,
    file_extension: &str,
) -> VirtualDocument {
    let params: Vec<String> = def
        .params
        .iter()
        .flat_map(|p| p.get_identifiers())
        .collect();

    let header = if def.is_async {
        format!("async def {}({}):\n", def.name, params.join(", "))
    } else {
        format!("def {}({}):\n", def.name, params.join(", "))
    };
    let body = &def.body_text;
    let body_lines: Vec<&str> = body.lines().collect();

    let mut virtual_doc = header;
    for line in &body_lines {
        virtual_doc.push_str("    ");
        virtual_doc.push_str(line);
        virtual_doc.push('\n');
    }
    if body.trim().is_empty() {
        virtual_doc.push_str("    pass\n");
    }

    let mut line_mapping = Vec::new();
    // First line is the synthetic `def` header
    line_mapping.push(None);

    let (body_start_line, body_start_col) =
        crate::util::offset_to_line_col(source, def.body_span.start.min(source.len()));
    let body_raw_end = def.body_span.end.min(source.len());
    let body_raw = &source[def.body_span.start.min(source.len())..body_raw_end];
    let body_raw_lines: Vec<&str> = body_raw.lines().collect();

    for (i, body_line) in body_lines.iter().enumerate() {
        let raw_line = body_raw_lines.get(i).copied().unwrap_or_default();
        let source_line_base_col = if i == 0 { body_start_col } else { 0 };
        let dedent_prefix_chars = dedented_prefix_chars(raw_line, body_line);
        line_mapping.push(Some(ForeignLineMapping {
            source_line: body_start_line + i as u32,
            source_col_start: source_line_base_col + dedent_prefix_chars,
            virtual_col_start: 4,
        }));
    }

    let position_map = PositionMap {
        line_mapping,
        body_start_offset: def.body_span.start,
        header_lines: 1,
    };

    let extension = normalize_file_extension(file_extension);
    let virtual_path = workspace_dir.join(".shape-vdocs").join(format!(
        "{}_{}.{}",
        sanitize_filename(source_uri),
        def.name,
        extension
    ));

    VirtualDocument {
        content: virtual_doc,
        position_map,
        language: def.language.clone(),
        function_name: def.name.clone(),
        source_uri: source_uri.to_string(),
        virtual_path,
    }
}

fn dedented_prefix_chars(raw_line: &str, dedented_line: &str) -> u32 {
    if raw_line == dedented_line {
        return 0;
    }
    if dedented_line.is_empty() {
        return raw_line.chars().count() as u32;
    }
    for (byte_idx, _) in raw_line.char_indices() {
        if raw_line[byte_idx..] == *dedented_line {
            return raw_line[..byte_idx].chars().count() as u32;
        }
    }
    0
}

fn normalize_file_extension(file_extension: &str) -> String {
    let trimmed = file_extension.trim();
    if trimmed.is_empty() {
        return "txt".to_string();
    }
    trimmed.trim_start_matches('.').to_string()
}

// ---------------------------------------------------------------------------
// ForeignLspManager
// ---------------------------------------------------------------------------

/// State for a single child language server process.
struct ChildServer {
    _process: Child,
    /// Stdin handle for sending JSON-RPC messages.
    stdin: tokio::process::ChildStdin,
    /// Pending responses keyed by request ID.
    pending: HashMap<u64, tokio::sync::oneshot::Sender<serde_json::Value>>,
    /// Next JSON-RPC request ID.
    next_id: u64,
    /// Whether the server has been initialized.
    initialized: bool,
    /// Child semantic token type legend (`legend.tokenTypes`).
    semantic_token_types: Vec<String>,
    /// Child semantic token modifier legend (`legend.tokenModifiers`).
    semantic_token_modifiers: Vec<String>,
    /// Open virtual documents keyed by URI with their current version.
    open_documents: HashMap<String, i32>,
}

#[derive(Clone, Debug)]
struct RuntimeLspConfig {
    server_command: Vec<String>,
    file_extension: String,
    extra_paths: Vec<String>,
}

#[derive(Clone, Debug)]
struct VirtualUriMapping {
    source_uri: Uri,
    position_map: PositionMap,
}

#[derive(Clone, Debug)]
struct ResolvedVirtualDocRequest {
    language: String,
    vdoc_uri: String,
    virtual_pos: Position,
    position_map: PositionMap,
}

/// Absolute semantic token in Shape document coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForeignSemanticToken {
    pub line: u32,
    pub start_char: u32,
    pub length: u32,
    pub token_type: u32,
    pub token_modifiers_bitset: u32,
}

/// Extension module spec configured directly by the LSP client.
#[derive(Clone, Debug)]
pub struct ConfiguredExtensionSpec {
    pub name: String,
    pub path: PathBuf,
    pub config: serde_json::Value,
}

/// Manages child LSP server processes for foreign language blocks.
///
/// Each foreign language gets at most one child server. The manager handles
/// lifecycle (start, shutdown), document synchronization, and request forwarding.
pub struct ForeignLspManager {
    /// Child servers keyed by language identifier.
    servers: Arc<Mutex<HashMap<String, ChildServer>>>,
    /// Virtual documents keyed by (source_uri, function_name).
    documents: Arc<Mutex<HashMap<(String, String), VirtualDocument>>>,
    /// Last `publishDiagnostics` payloads keyed by virtual document URI.
    published_diagnostics: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
    /// Runtime-declared child LSP config keyed by language identifier.
    runtime_configs: Arc<Mutex<HashMap<String, RuntimeLspConfig>>>,
    /// Shared extension registry used to discover language runtime LSP configs.
    extension_registry: shape_runtime::provider_registry::ProviderRegistry,
    /// Canonical extension identity keys loaded into `extension_registry`.
    loaded_extension_keys: Arc<Mutex<HashSet<String>>>,
    /// Extension specs that should always be loaded regardless of source context.
    configured_extensions: Arc<Mutex<Vec<ConfiguredExtensionSpec>>>,
    /// Workspace root for virtual document paths and child LSP rootUri.
    /// Updated from `initialize()` when the real workspace root is known.
    workspace_dir: std::sync::RwLock<PathBuf>,
}

impl ForeignLspManager {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            servers: Arc::new(Mutex::new(HashMap::new())),
            documents: Arc::new(Mutex::new(HashMap::new())),
            published_diagnostics: Arc::new(Mutex::new(HashMap::new())),
            runtime_configs: Arc::new(Mutex::new(HashMap::new())),
            extension_registry: shape_runtime::provider_registry::ProviderRegistry::new(),
            loaded_extension_keys: Arc::new(Mutex::new(HashSet::new())),
            configured_extensions: Arc::new(Mutex::new(Vec::new())),
            workspace_dir: std::sync::RwLock::new(workspace_dir),
        }
    }

    /// Update the workspace root directory.
    ///
    /// Called from `initialize()` once the real workspace root is known from
    /// the client's `rootUri` / `workspaceFolders`. This ensures child LSP
    /// servers receive the correct `rootUri` so they can discover project
    /// config files (e.g. `pyrightconfig.json`, virtualenvs).
    pub fn set_workspace_dir(&self, dir: PathBuf) {
        *self.workspace_dir.write().unwrap() = dir;
    }

    /// Set extension specs that should always be loaded for foreign-LSP discovery.
    pub async fn set_configured_extensions(&self, specs: Vec<ConfiguredExtensionSpec>) {
        let mut configured = self.configured_extensions.lock().await;
        *configured = specs;
    }

    fn resolve_configured_extension_path(
        path: &Path,
        current_file: Option<&Path>,
        workspace_root: Option<&Path>,
    ) -> PathBuf {
        if path.is_absolute() {
            return path.to_path_buf();
        }
        if let Some(root) = workspace_root {
            return root.join(path);
        }
        if let Some(file) = current_file
            && let Some(parent) = file.parent()
        {
            return parent.join(path);
        }
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }

    /// Refresh language-runtime child-LSP configuration from declared extensions.
    async fn refresh_runtime_configs(
        &self,
        current_file: Option<&Path>,
        workspace_root: Option<&Path>,
        source: Option<&str>,
    ) {
        let specs = shape_runtime::extension_context::declared_extension_specs_for_context(
            current_file,
            workspace_root,
            source,
        );
        let configured_specs = self.configured_extensions.lock().await.clone();

        {
            let mut loaded_keys = self.loaded_extension_keys.lock().await;
            for spec in specs {
                let canonical = spec
                    .path
                    .canonicalize()
                    .unwrap_or_else(|_| spec.path.clone());
                let config_key = serde_json::to_string(&spec.config).unwrap_or_default();
                let identity = format!("{}|{}", canonical.to_string_lossy(), config_key);
                if loaded_keys.contains(&identity) {
                    continue;
                }

                match self
                    .extension_registry
                    .load_extension(&spec.path, &spec.config)
                {
                    Ok(_) => {
                        loaded_keys.insert(identity);
                    }
                    Err(err) => {
                        tracing::warn!(
                            "failed to load extension '{}' for foreign LSP config discovery: {}",
                            spec.name,
                            err
                        );
                    }
                }
            }

            for spec in configured_specs {
                let resolved_path = Self::resolve_configured_extension_path(
                    &spec.path,
                    current_file,
                    workspace_root,
                );
                let canonical = resolved_path
                    .canonicalize()
                    .unwrap_or_else(|_| resolved_path.clone());
                let config_key = serde_json::to_string(&spec.config).unwrap_or_default();
                let identity = format!("{}|{}", canonical.to_string_lossy(), config_key);
                if loaded_keys.contains(&identity) {
                    continue;
                }

                match self
                    .extension_registry
                    .load_extension(&resolved_path, &spec.config)
                {
                    Ok(_) => {
                        loaded_keys.insert(identity);
                    }
                    Err(err) => {
                        tracing::warn!(
                            "failed to load configured extension '{}' for foreign LSP config discovery: {}",
                            spec.name,
                            err
                        );
                    }
                }
            }
        }

        let runtime_configs = self.extension_registry.language_runtime_lsp_configs();
        let mut configs = self.runtime_configs.lock().await;
        configs.clear();
        for runtime in runtime_configs {
            configs.insert(
                runtime.language_id.clone(),
                RuntimeLspConfig {
                    server_command: runtime.server_command,
                    file_extension: runtime.file_extension,
                    extra_paths: runtime.extra_paths,
                },
            );
        }
    }

    async fn runtime_config_for_language(&self, language: &str) -> Option<RuntimeLspConfig> {
        let configs = self.runtime_configs.lock().await;
        configs.get(language).cloned()
    }

    /// Start a child language server for the given language, if not already running.
    pub async fn start_server(&self, language: &str) -> Result<(), String> {
        let runtime_cfg = self
            .runtime_config_for_language(language)
            .await
            .ok_or_else(|| format!("No language runtime LSP config for '{}'", language))?;
        let (cmd, args) = runtime_cfg
            .server_command
            .split_first()
            .ok_or_else(|| format!("Empty server command for '{}'", language))?;

        let mut servers = self.servers.lock().await;
        if servers.contains_key(language) {
            return Ok(());
        }

        let mut child = Command::new(cmd)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start {} for '{}': {}", cmd, language, e))?;

        let stdin = child.stdin.take().ok_or("Failed to capture child stdin")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Failed to capture child stdout")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Failed to capture child stderr")?;

        let server = ChildServer {
            _process: child,
            stdin,
            pending: HashMap::new(),
            next_id: 1,
            initialized: false,
            semantic_token_types: Vec::new(),
            semantic_token_modifiers: Vec::new(),
            open_documents: HashMap::new(),
        };

        servers.insert(language.to_string(), server);

        // Spawn a reader task for stdout
        let servers_ref = Arc::clone(&self.servers);
        let diagnostics_ref = Arc::clone(&self.published_diagnostics);
        let lang = language.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut header_buf = String::new();

            loop {
                header_buf.clear();
                match reader.read_line(&mut header_buf).await {
                    Ok(0) => break, // EOF
                    Err(_) => break,
                    Ok(_) => {}
                }

                // Parse Content-Length header
                let content_length = if header_buf.starts_with("Content-Length:") {
                    header_buf
                        .trim_start_matches("Content-Length:")
                        .trim()
                        .parse::<usize>()
                        .ok()
                } else {
                    None
                };

                // Read the empty separator line
                header_buf.clear();
                if reader.read_line(&mut header_buf).await.is_err() {
                    break;
                }

                if let Some(len) = content_length {
                    let mut body = vec![0u8; len];
                    if tokio::io::AsyncReadExt::read_exact(&mut reader, &mut body)
                        .await
                        .is_err()
                    {
                        break;
                    }

                    if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&body) {
                        if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                            if method == "textDocument/publishDiagnostics" {
                                if let Some(params) = msg.get("params")
                                    && let Some(uri) = params.get("uri").and_then(|v| v.as_str())
                                {
                                    let diagnostics = params
                                        .get("diagnostics")
                                        .cloned()
                                        .and_then(|v| {
                                            serde_json::from_value::<Vec<Diagnostic>>(v).ok()
                                        })
                                        .unwrap_or_default();
                                    diagnostics_ref
                                        .lock()
                                        .await
                                        .insert(uri.to_string(), diagnostics);
                                }
                            }
                        }

                        // If this is a response (has "id" but no "method"), route it
                        if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                            if msg.get("method").is_none() {
                                let mut servers = servers_ref.lock().await;
                                if let Some(server) = servers.get_mut(&lang) {
                                    if let Some(sender) = server.pending.remove(&id) {
                                        let _ = sender.send(msg);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let stderr_lang = language.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end_matches(['\r', '\n']);
                        if !trimmed.is_empty() {
                            tracing::info!("child-lsp[{stderr_lang}] stderr: {trimmed}");
                        }
                    }
                    Err(err) => {
                        tracing::warn!("child-lsp[{stderr_lang}] stderr read failed: {err}");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Send an LSP initialize request to a child server.
    async fn initialize_server(&self, language: &str) -> Result<(), String> {
        let runtime_cfg = self
            .runtime_config_for_language(language)
            .await
            .ok_or_else(|| format!("No language runtime LSP config for '{}'", language))?;

        let workspace_uri = format!("file://{}", self.workspace_dir.read().unwrap().display());
        let workspace_path = self.workspace_dir.read().unwrap().display().to_string();
        let mut init_params = serde_json::json!({
            "processId": std::process::id(),
            "capabilities": child_client_capabilities(),
            // Use the real workspace root so child servers (e.g., pyright) can
            // discover project config and Python environments.
            "rootUri": workspace_uri,
            "rootPath": workspace_path,
            "workspaceFolders": [
                {
                    "uri": workspace_uri,
                    "name": workspace_path,
                }
            ],
        });
        if !runtime_cfg.extra_paths.is_empty() {
            init_params["initializationOptions"] = serde_json::json!({
                "extraPaths": runtime_cfg.extra_paths,
            });
        }

        tracing::info!(
            "child-lsp[{}] initialize rootUri={} rootPath={}",
            language,
            workspace_uri,
            workspace_path,
        );

        let response = self
            .send_request(language, "initialize", init_params)
            .await?;

        // Send initialized notification
        self.send_notification(language, "initialized", serde_json::json!({}))
            .await?;

        let (semantic_token_types, semantic_token_modifiers) =
            extract_semantic_tokens_legend(&response);

        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get_mut(language) {
            server.initialized = true;
            server.semantic_token_types = semantic_token_types;
            server.semantic_token_modifiers = semantic_token_modifiers;
        }

        Ok(())
    }

    /// Send a JSON-RPC request to a child server and await the response.
    async fn send_request(
        &self,
        language: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        let request_id;
        {
            let mut servers = self.servers.lock().await;
            let server = servers
                .get_mut(language)
                .ok_or_else(|| format!("No server for '{}'", language))?;

            request_id = server.next_id;
            server.next_id += 1;
            server.pending.insert(request_id, tx);

            let msg = serde_json::json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": params,
            });

            let body = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
            let header = format!("Content-Length: {}\r\n\r\n", body.len());

            server
                .stdin
                .write_all(header.as_bytes())
                .await
                .map_err(|e| format!("Failed to write to child stdin: {}", e))?;
            server
                .stdin
                .write_all(body.as_bytes())
                .await
                .map_err(|e| format!("Failed to write to child stdin: {}", e))?;
            server
                .stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush child stdin: {}", e))?;
        }

        let response = rx
            .await
            .map_err(|_| "Child server response channel closed".to_string())?;
        if let Some(error) = response.get("error") {
            return Err(format!(
                "Child server '{}' request '{}' failed: {}",
                language, method, error
            ));
        }
        Ok(response)
    }

    /// Send a JSON-RPC notification (no response expected) to a child server.
    async fn send_notification(
        &self,
        language: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), String> {
        let mut servers = self.servers.lock().await;
        let server = servers
            .get_mut(language)
            .ok_or_else(|| format!("No server for '{}'", language))?;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let body = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        server
            .stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        server
            .stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        server.stdin.flush().await.map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn ensure_server_ready(&self, language: &str) -> Result<(), String> {
        let (is_running, is_initialized) = {
            let servers = self.servers.lock().await;
            (
                servers.contains_key(language),
                servers
                    .get(language)
                    .map(|server| server.initialized)
                    .unwrap_or(false),
            )
        };

        if !is_running {
            self.start_server(language).await?;
        }
        if !is_initialized {
            self.initialize_server(language).await?;
        }
        Ok(())
    }

    async fn close_virtual_document(&self, language: &str, vdoc_uri: &str) -> Result<(), String> {
        let should_close = {
            let servers = self.servers.lock().await;
            servers
                .get(language)
                .map(|server| server.open_documents.contains_key(vdoc_uri))
                .unwrap_or(false)
        };

        if !should_close {
            return Ok(());
        }

        self.send_notification(
            language,
            "textDocument/didClose",
            serde_json::json!({
                "textDocument": { "uri": vdoc_uri }
            }),
        )
        .await?;

        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get_mut(language) {
            server.open_documents.remove(vdoc_uri);
        }
        Ok(())
    }

    async fn sync_virtual_document(
        &self,
        language: &str,
        vdoc_uri: &str,
        content: &str,
    ) -> Result<(), String> {
        let previous_version = {
            let servers = self.servers.lock().await;
            servers
                .get(language)
                .and_then(|server| server.open_documents.get(vdoc_uri).copied())
                .unwrap_or(0)
        };
        let next_version = if previous_version > 0 {
            previous_version + 1
        } else {
            1
        };

        if previous_version > 0 {
            self.send_notification(
                language,
                "textDocument/didChange",
                serde_json::json!({
                    "textDocument": {
                        "uri": vdoc_uri,
                        "version": next_version,
                    },
                    "contentChanges": [
                        { "text": content }
                    ],
                }),
            )
            .await?;
        } else {
            self.send_notification(
                language,
                "textDocument/didOpen",
                serde_json::json!({
                    "textDocument": {
                        "uri": vdoc_uri,
                        "languageId": language,
                        "version": next_version,
                        "text": content,
                    }
                }),
            )
            .await?;
        }

        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get_mut(language) {
            server
                .open_documents
                .insert(vdoc_uri.to_string(), next_version);
        }
        Ok(())
    }

    /// Update virtual documents for all foreign function blocks in a parsed program.
    ///
    /// Call this from `analyze_document` whenever the program is successfully parsed.
    pub async fn update_documents(
        &self,
        source_uri: &str,
        source: &str,
        items: &[Item],
        current_file: Option<&Path>,
        workspace_root: Option<&Path>,
    ) -> Vec<Diagnostic> {
        self.refresh_runtime_configs(current_file, workspace_root, Some(source))
            .await;

        let runtime_configs = self.runtime_configs.lock().await.clone();
        let mut docs = self.documents.lock().await;
        let mut language_ranges: HashMap<String, Range> = HashMap::new();
        let mut missing_runtime_languages: HashSet<String> = HashSet::new();

        let removed_virtual_docs: Vec<(String, String)> = docs
            .iter()
            .filter(|((uri, _), _)| uri == source_uri)
            .map(|((_, _), doc)| {
                (
                    doc.language.clone(),
                    format!("file://{}", doc.virtual_path.display()),
                )
            })
            .collect();

        // Remove stale documents for this source URI
        docs.retain(|(uri, _), _| uri != source_uri);

        for item in items {
            if let Item::ForeignFunction(def, _) = item {
                language_ranges
                    .entry(def.language.clone())
                    .or_insert_with(|| span_to_range(source, def.name_span));
                let Some(runtime_cfg) = runtime_configs.get(&def.language) else {
                    missing_runtime_languages.insert(def.language.clone());
                    continue;
                };

                let vdoc = generate_virtual_document(
                    def,
                    source,
                    source_uri,
                    &self.workspace_dir.read().unwrap(),
                    &runtime_cfg.file_extension,
                );

                docs.insert((source_uri.to_string(), def.name.clone()), vdoc);
            }
        }

        // Write virtual documents to disk and notify child servers
        for ((uri, _fn_name), vdoc) in docs.iter() {
            if uri != source_uri {
                continue;
            }
            // Ensure parent directory exists
            if let Some(parent) = vdoc.virtual_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&vdoc.virtual_path, &vdoc.content);
        }

        // Drop lock before async operations
        let docs_snapshot: Vec<(String, String, String)> = docs
            .iter()
            .filter(|((uri, _), _)| uri == source_uri)
            .map(|((_, _fn_name), vdoc)| {
                (
                    vdoc.language.clone(),
                    format!("file://{}", vdoc.virtual_path.display()),
                    vdoc.content.clone(),
                )
            })
            .collect();
        drop(docs);

        if !removed_virtual_docs.is_empty() {
            let mut published = self.published_diagnostics.lock().await;
            for (_, uri) in &removed_virtual_docs {
                published.remove(uri.as_str());
            }
        }

        let mut startup_diagnostics = Vec::new();
        let mut seen_issue_keys = HashSet::new();

        let mut push_issue = |language: &str, message: String| {
            let key = format!("{language}|{message}");
            if !seen_issue_keys.insert(key) {
                return;
            }
            let range = language_ranges
                .get(language)
                .cloned()
                .unwrap_or_else(fallback_range);
            startup_diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("shape-foreign".to_string()),
                message,
                ..Default::default()
            });
        };

        for language in missing_runtime_languages {
            push_issue(
                &language,
                format!(
                    "No child LSP config for foreign language '{}' (load the matching extension).",
                    language
                ),
            );
        }

        for (language, vdoc_uri) in removed_virtual_docs {
            if let Err(err) = self.close_virtual_document(&language, &vdoc_uri).await {
                push_issue(
                    &language,
                    format!("Failed to close foreign virtual document: {}", err),
                );
            }
        }

        // Sync virtual documents to child servers with explicit startup errors.
        for (language, vdoc_uri, content) in docs_snapshot {
            if let Err(err) = self.ensure_server_ready(&language).await {
                push_issue(
                    &language,
                    format!(
                        "Failed to start/initialize child LSP '{}': {}",
                        language, err
                    ),
                );
                continue;
            }

            if let Err(err) = self
                .sync_virtual_document(&language, &vdoc_uri, &content)
                .await
            {
                push_issue(
                    &language,
                    format!("Failed to sync foreign virtual document: {}", err),
                );
            }
        }

        startup_diagnostics
    }

    async fn resolve_virtual_doc_request(
        &self,
        source_uri: &str,
        position: Position,
        items: &[Item],
        source: &str,
    ) -> Option<ResolvedVirtualDocRequest> {
        let offset = crate::util::position_to_offset(source, position)?;
        let def = find_foreign_block_at_offset(items, offset)?;
        let docs = self.documents.lock().await;
        let vdoc = docs.get(&(source_uri.to_string(), def.name.clone()))?;
        let virtual_pos = vdoc.position_map.shape_to_virtual(position)?;
        Some(ResolvedVirtualDocRequest {
            language: vdoc.language.clone(),
            vdoc_uri: format!("file://{}", vdoc.virtual_path.display()),
            virtual_pos,
            position_map: vdoc.position_map.clone(),
        })
    }

    async fn virtual_uri_mappings(&self) -> HashMap<String, VirtualUriMapping> {
        let docs = self.documents.lock().await;
        docs.iter()
            .filter_map(|((_, _), vdoc)| {
                let source_uri = Uri::from_str(&vdoc.source_uri).ok()?;
                Some((
                    format!("file://{}", vdoc.virtual_path.display()),
                    VirtualUriMapping {
                        source_uri,
                        position_map: vdoc.position_map.clone(),
                    },
                ))
            })
            .collect()
    }

    /// Handle a completion request that falls inside a foreign function body.
    ///
    /// Returns `None` if the position is not in a foreign block or delegation fails.
    pub async fn handle_completion(
        &self,
        source_uri: &str,
        position: Position,
        items: &[Item],
        source: &str,
    ) -> Option<Vec<CompletionItem>> {
        let resolved = self
            .resolve_virtual_doc_request(source_uri, position, items, source)
            .await?;

        let params = serde_json::json!({
            "textDocument": { "uri": resolved.vdoc_uri },
            "position": { "line": resolved.virtual_pos.line, "character": resolved.virtual_pos.character },
        });

        let response = self
            .send_request(&resolved.language, "textDocument/completion", params)
            .await
            .ok()?;

        // Extract completion items from the response
        parse_completion_response(response)
    }

    /// Handle a hover request that falls inside a foreign function body.
    pub async fn handle_hover(
        &self,
        source_uri: &str,
        position: Position,
        items: &[Item],
        source: &str,
    ) -> Option<Hover> {
        let resolved = self
            .resolve_virtual_doc_request(source_uri, position, items, source)
            .await?;

        let params = serde_json::json!({
            "textDocument": { "uri": resolved.vdoc_uri },
            "position": { "line": resolved.virtual_pos.line, "character": resolved.virtual_pos.character },
        });

        let response = self
            .send_request(&resolved.language, "textDocument/hover", params)
            .await
            .ok()?;

        let mut hover = parse_hover_response(response)?;
        if let Some(range) = hover.range {
            hover.range = resolved.position_map.virtual_range_to_shape(range);
        }
        Some(hover)
    }

    /// Handle a signature help request that falls inside a foreign function body.
    pub async fn handle_signature_help(
        &self,
        source_uri: &str,
        position: Position,
        items: &[Item],
        source: &str,
    ) -> Option<SignatureHelp> {
        let resolved = self
            .resolve_virtual_doc_request(source_uri, position, items, source)
            .await?;

        let params = serde_json::json!({
            "textDocument": { "uri": resolved.vdoc_uri },
            "position": { "line": resolved.virtual_pos.line, "character": resolved.virtual_pos.character },
        });

        let response = self
            .send_request(&resolved.language, "textDocument/signatureHelp", params)
            .await
            .ok()?;
        parse_signature_help_response(response)
    }

    /// Handle go-to-definition for positions inside foreign function bodies.
    pub async fn handle_definition(
        &self,
        source_uri: &str,
        position: Position,
        items: &[Item],
        source: &str,
    ) -> Option<GotoDefinitionResponse> {
        let resolved = self
            .resolve_virtual_doc_request(source_uri, position, items, source)
            .await?;

        let params = serde_json::json!({
            "textDocument": { "uri": resolved.vdoc_uri },
            "position": { "line": resolved.virtual_pos.line, "character": resolved.virtual_pos.character },
        });

        let response = self
            .send_request(&resolved.language, "textDocument/definition", params)
            .await
            .ok()?;
        let definition = parse_definition_response(response)?;
        let mappings = self.virtual_uri_mappings().await;
        Some(map_definition_response_to_shape(definition, &mappings))
    }

    /// Handle find-references for positions inside foreign function bodies.
    pub async fn handle_references(
        &self,
        source_uri: &str,
        position: Position,
        items: &[Item],
        source: &str,
    ) -> Option<Vec<Location>> {
        let resolved = self
            .resolve_virtual_doc_request(source_uri, position, items, source)
            .await?;

        let params = serde_json::json!({
            "textDocument": { "uri": resolved.vdoc_uri },
            "position": { "line": resolved.virtual_pos.line, "character": resolved.virtual_pos.character },
            "context": { "includeDeclaration": true },
        });

        let response = self
            .send_request(&resolved.language, "textDocument/references", params)
            .await
            .ok()?;
        let references = parse_references_response(response)?;
        let mappings = self.virtual_uri_mappings().await;
        Some(
            references
                .into_iter()
                .map(|location| map_location_to_shape(location, &mappings))
                .collect(),
        )
    }

    /// Collect semantic tokens for foreign blocks mapped into Shape coordinates.
    pub async fn collect_semantic_tokens(&self, source_uri: &str) -> Vec<ForeignSemanticToken> {
        let docs_snapshot: Vec<(String, String, PositionMap, String)> = {
            let docs = self.documents.lock().await;
            docs.iter()
                .filter(|((uri, _), _)| uri == source_uri)
                .map(|((_, _), vdoc)| {
                    (
                        vdoc.language.clone(),
                        format!("file://{}", vdoc.virtual_path.display()),
                        vdoc.position_map.clone(),
                        vdoc.content.clone(),
                    )
                })
                .collect()
        };

        let mut collected = Vec::new();
        for (language, vdoc_uri, position_map, content) in docs_snapshot {
            let (mut token_types, mut token_modifiers) = {
                let servers = self.servers.lock().await;
                let Some(server) = servers.get(&language) else {
                    continue;
                };
                (
                    server.semantic_token_types.clone(),
                    server.semantic_token_modifiers.clone(),
                )
            };
            if token_types.is_empty() {
                token_types = CHILD_SEMANTIC_TOKEN_TYPES
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect();
            }
            if token_modifiers.is_empty() {
                token_modifiers = CHILD_SEMANTIC_TOKEN_MODIFIERS
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect();
            }

            let mut tokens = match self
                .request_semantic_tokens(&language, &vdoc_uri, &content)
                .await
            {
                Ok(tokens) => tokens,
                Err(err) => {
                    tracing::info!(
                        "child-lsp[{language}] semantic tokens unavailable for {}: {err}",
                        vdoc_uri
                    );
                    continue;
                }
            };

            // First semantic-token request can race child-LSP document indexing.
            // Retry once after a short delay before giving up.
            if tokens
                .as_ref()
                .is_none_or(|token_set| token_set.data.is_empty())
            {
                tokio::time::sleep(Duration::from_millis(40)).await;
                if let Ok(retry_tokens) = self
                    .request_semantic_tokens(&language, &vdoc_uri, &content)
                    .await
                    && retry_tokens
                        .as_ref()
                        .is_some_and(|token_set| !token_set.data.is_empty())
                {
                    tokens = retry_tokens;
                }
            }

            let Some(tokens) = tokens else {
                continue;
            };

            collected.extend(map_foreign_semantic_tokens_to_shape(
                &tokens,
                &token_types,
                &token_modifiers,
                &position_map,
            ));
        }

        collected
    }

    async fn request_semantic_tokens(
        &self,
        language: &str,
        vdoc_uri: &str,
        content: &str,
    ) -> Result<Option<SemanticTokens>, String> {
        let full_params = serde_json::json!({
            "textDocument": { "uri": vdoc_uri },
        });
        let mut full_error: Option<String> = None;

        match self
            .send_request(language, "textDocument/semanticTokens/full", full_params)
            .await
        {
            Ok(response) => {
                if let Some(tokens) = parse_semantic_tokens_response(response) {
                    return Ok(Some(tokens));
                }
            }
            Err(err) => {
                full_error = Some(err);
            }
        }

        // Some older servers only implement the early draft method name
        // `textDocument/semanticTokens` instead of `/full`.
        let legacy_params = serde_json::json!({
            "textDocument": { "uri": vdoc_uri },
        });
        let mut legacy_error: Option<String> = None;
        match self
            .send_request(language, "textDocument/semanticTokens", legacy_params)
            .await
        {
            Ok(response) => {
                if let Some(tokens) = parse_semantic_tokens_response(response) {
                    return Ok(Some(tokens));
                }
            }
            Err(err) => {
                legacy_error = Some(err);
            }
        }

        let range = full_document_semantic_tokens_range(content);
        let range_params = serde_json::json!({
            "textDocument": { "uri": vdoc_uri },
            "range": {
                "start": { "line": range.start.line, "character": range.start.character },
                "end": { "line": range.end.line, "character": range.end.character },
            },
        });

        match self
            .send_request(language, "textDocument/semanticTokens/range", range_params)
            .await
        {
            Ok(response) => Ok(parse_semantic_tokens_response(response)),
            Err(range_err) => {
                if let (Some(full_err), Some(legacy_err)) =
                    (full_error.as_ref(), legacy_error.as_ref())
                {
                    Err(format!(
                        "full request failed ({full_err}); legacy request failed ({legacy_err}); range request failed ({range_err})"
                    ))
                } else if let Some(full_err) = full_error.as_ref() {
                    Err(format!(
                        "full request failed ({full_err}); range request failed ({range_err})"
                    ))
                } else if let Some(legacy_err) = legacy_error.as_ref() {
                    Err(format!(
                        "legacy request failed ({legacy_err}); range request failed ({range_err})"
                    ))
                } else {
                    Err(format!("range request failed ({range_err})"))
                }
            }
        }
    }

    /// Retrieve diagnostics from child language servers for foreign blocks in a source file.
    ///
    /// Maps virtual document diagnostics back to Shape source positions.
    pub async fn get_diagnostics(&self, source_uri: &str) -> Vec<Diagnostic> {
        let docs_snapshot: Vec<(String, String, PositionMap)> = {
            let docs = self.documents.lock().await;
            docs.iter()
                .filter(|((uri, _), _)| uri == source_uri)
                .map(|((_, _), vdoc)| {
                    (
                        vdoc.language.clone(),
                        format!("file://{}", vdoc.virtual_path.display()),
                        vdoc.position_map.clone(),
                    )
                })
                .collect()
        };

        let mut all_diagnostics = Vec::new();

        for (language, vdoc_uri, position_map) in docs_snapshot {
            let mut diagnostics = None;

            let request_params = serde_json::json!({
                "textDocument": { "uri": vdoc_uri },
                "identifier": "shape-foreign",
            });
            if let Ok(response) = self
                .send_request(&language, "textDocument/diagnostic", request_params)
                .await
            {
                diagnostics = parse_document_diagnostic_report(response);
            }

            if diagnostics.is_none() {
                let published = self.published_diagnostics.lock().await;
                diagnostics = published.get(&vdoc_uri).cloned();
            }

            let Some(diagnostics) = diagnostics else {
                continue;
            };
            for diagnostic in &diagnostics {
                if let Some(mapped) = Self::map_diagnostic_to_shape(diagnostic, &position_map) {
                    all_diagnostics.push(mapped);
                }
            }
        }

        all_diagnostics
    }

    /// Map a diagnostic from a virtual document back to Shape source coordinates.
    pub fn map_diagnostic_to_shape(
        diagnostic: &Diagnostic,
        position_map: &PositionMap,
    ) -> Option<Diagnostic> {
        let range = position_map.virtual_range_to_shape(diagnostic.range)?;
        Some(Diagnostic {
            range,
            severity: diagnostic.severity,
            code: diagnostic.code.clone(),
            code_description: diagnostic.code_description.clone(),
            source: diagnostic.source.clone(),
            message: diagnostic.message.clone(),
            related_information: None,
            tags: diagnostic.tags.clone(),
            data: diagnostic.data.clone(),
        })
    }

    /// Shut down all child language servers gracefully.
    pub async fn shutdown(&self) {
        let mut servers = self.servers.lock().await;
        for (language, server) in servers.iter_mut() {
            // Best-effort shutdown request
            let msg = serde_json::json!({
                "jsonrpc": "2.0",
                "id": server.next_id,
                "method": "shutdown",
                "params": null,
            });
            server.next_id += 1;

            if let Ok(body) = serde_json::to_string(&msg) {
                let header = format!("Content-Length: {}\r\n\r\n", body.len());
                let _ = server.stdin.write_all(header.as_bytes()).await;
                let _ = server.stdin.write_all(body.as_bytes()).await;
                let _ = server.stdin.flush().await;
            }

            // Send exit notification
            let exit_msg = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "exit",
                "params": null,
            });
            if let Ok(body) = serde_json::to_string(&exit_msg) {
                let header = format!("Content-Length: {}\r\n\r\n", body.len());
                let _ = server.stdin.write_all(header.as_bytes()).await;
                let _ = server.stdin.write_all(body.as_bytes()).await;
                let _ = server.stdin.flush().await;
            }

            tracing::info!("Shut down child LSP for '{}'", language);
        }
        servers.clear();
    }
}

// ---------------------------------------------------------------------------
// Lookup helpers
// ---------------------------------------------------------------------------

/// Check if a byte offset falls inside any foreign function body in the given items.
pub fn find_foreign_block_at_offset(items: &[Item], offset: usize) -> Option<&ForeignFunctionDef> {
    for item in items {
        if let Item::ForeignFunction(def, _) = item {
            if offset >= def.body_span.start && offset < def.body_span.end {
                return Some(def);
            }
        }
    }
    None
}

/// Check if a position falls inside a foreign function body, given the source text.
pub fn is_position_in_foreign_block(items: &[Item], source: &str, position: Position) -> bool {
    if let Some(offset) = crate::util::position_to_offset(source, position) {
        find_foreign_block_at_offset(items, offset).is_some()
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Response parsing helpers
// ---------------------------------------------------------------------------

fn parse_completion_response(response: serde_json::Value) -> Option<Vec<CompletionItem>> {
    let result = response.get("result")?;

    // Handle both CompletionList and CompletionItem[] responses
    let items_val = if let Some(items) = result.get("items") {
        items
    } else if result.is_array() {
        result
    } else {
        return None;
    };

    let arr = items_val.as_array()?;
    let mut completions = Vec::new();

    for item in arr {
        let label = item.get("label")?.as_str()?.to_string();
        let kind = item
            .get("kind")
            .and_then(|k| k.as_u64())
            .and_then(map_completion_item_kind);
        let detail = item
            .get("detail")
            .and_then(|d| d.as_str())
            .map(String::from);
        let documentation = item.get("documentation").and_then(|d| {
            if let Some(s) = d.as_str() {
                Some(tower_lsp_server::ls_types::Documentation::String(
                    s.to_string(),
                ))
            } else {
                None
            }
        });

        completions.push(CompletionItem {
            label,
            kind,
            detail,
            documentation,
            ..Default::default()
        });
    }

    Some(completions)
}

fn map_completion_item_kind(kind: u64) -> Option<CompletionItemKind> {
    // LSP completion item kind values are standardized
    match kind {
        1 => Some(CompletionItemKind::TEXT),
        2 => Some(CompletionItemKind::METHOD),
        3 => Some(CompletionItemKind::FUNCTION),
        4 => Some(CompletionItemKind::CONSTRUCTOR),
        5 => Some(CompletionItemKind::FIELD),
        6 => Some(CompletionItemKind::VARIABLE),
        7 => Some(CompletionItemKind::CLASS),
        8 => Some(CompletionItemKind::INTERFACE),
        9 => Some(CompletionItemKind::MODULE),
        10 => Some(CompletionItemKind::PROPERTY),
        _ => None,
    }
}

fn parse_hover_response(response: serde_json::Value) -> Option<Hover> {
    let result = response.get("result")?;
    if result.is_null() {
        return None;
    }
    serde_json::from_value(result.clone()).ok()
}

fn parse_signature_help_response(response: serde_json::Value) -> Option<SignatureHelp> {
    let result = response.get("result")?;
    if result.is_null() {
        return None;
    }
    serde_json::from_value(result.clone()).ok()
}

fn parse_definition_response(response: serde_json::Value) -> Option<GotoDefinitionResponse> {
    let result = response.get("result")?;
    if result.is_null() {
        return None;
    }
    serde_json::from_value(result.clone()).ok()
}

fn parse_references_response(response: serde_json::Value) -> Option<Vec<Location>> {
    let result = response.get("result")?;
    if result.is_null() {
        return Some(Vec::new());
    }
    serde_json::from_value(result.clone()).ok()
}

fn parse_document_diagnostic_report(response: serde_json::Value) -> Option<Vec<Diagnostic>> {
    let result = response.get("result")?;
    if result.is_null() {
        return None;
    }

    let items = result
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| {
            result
                .pointer("/fullDocumentDiagnosticReport/items")
                .and_then(|v| v.as_array())
                .cloned()
        })?;

    let mut diagnostics = Vec::with_capacity(items.len());
    for item in items {
        let Ok(parsed) = serde_json::from_value::<Diagnostic>(item) else {
            continue;
        };
        diagnostics.push(parsed);
    }
    Some(diagnostics)
}

fn parse_semantic_tokens_response(response: serde_json::Value) -> Option<SemanticTokens> {
    let result = response.get("result")?;
    if result.is_null() {
        return None;
    }
    serde_json::from_value(result.clone()).ok()
}

fn extract_semantic_tokens_legend(
    initialize_response: &serde_json::Value,
) -> (Vec<String>, Vec<String>) {
    let Some(provider) = initialize_response.pointer("/result/capabilities/semanticTokensProvider")
    else {
        return (Vec::new(), Vec::new());
    };
    if provider.is_null() {
        return (Vec::new(), Vec::new());
    }

    let legend = provider.get("legend").unwrap_or(provider);

    let mut token_types: Vec<String> = legend
        .get("tokenTypes")
        .or_else(|| provider.get("tokenTypes"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut token_modifiers: Vec<String> = legend
        .get("tokenModifiers")
        .or_else(|| provider.get("tokenModifiers"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Some servers advertise semanticTokensProvider but omit legend fields.
    // Use the same baseline vocabulary we advertised in client capabilities.
    if token_types.is_empty() {
        token_types = CHILD_SEMANTIC_TOKEN_TYPES
            .iter()
            .map(|s| (*s).to_string())
            .collect();
    }
    if token_modifiers.is_empty() {
        token_modifiers = CHILD_SEMANTIC_TOKEN_MODIFIERS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
    }

    (token_types, token_modifiers)
}

const CHILD_SEMANTIC_TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
];

const CHILD_SEMANTIC_TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "definition",
    "readonly",
    "static",
    "deprecated",
    "abstract",
    "async",
    "modification",
    "documentation",
    "defaultLibrary",
];

fn child_client_capabilities() -> serde_json::Value {
    serde_json::json!({
        "textDocument": {
            "hover": {
                "contentFormat": ["markdown", "plaintext"]
            },
            "completion": {
                "completionItem": {
                    "documentationFormat": ["markdown", "plaintext"]
                }
            },
            "semanticTokens": {
                "dynamicRegistration": false,
                "requests": {
                    "range": true,
                    "full": true
                },
                "tokenTypes": CHILD_SEMANTIC_TOKEN_TYPES,
                "tokenModifiers": CHILD_SEMANTIC_TOKEN_MODIFIERS,
                "formats": ["relative"],
                "multilineTokenSupport": true,
                "overlappingTokenSupport": true,
                "augmentsSyntaxTokens": true
            }
        }
    })
}

fn map_definition_response_to_shape(
    definition: GotoDefinitionResponse,
    mappings: &HashMap<String, VirtualUriMapping>,
) -> GotoDefinitionResponse {
    match definition {
        GotoDefinitionResponse::Scalar(location) => {
            GotoDefinitionResponse::Scalar(map_location_to_shape(location, mappings))
        }
        GotoDefinitionResponse::Array(locations) => GotoDefinitionResponse::Array(
            locations
                .into_iter()
                .map(|location| map_location_to_shape(location, mappings))
                .collect(),
        ),
        GotoDefinitionResponse::Link(links) => GotoDefinitionResponse::Link(
            links
                .into_iter()
                .map(|link| map_location_link_to_shape(link, mappings))
                .collect(),
        ),
    }
}

fn map_location_to_shape(
    location: Location,
    mappings: &HashMap<String, VirtualUriMapping>,
) -> Location {
    let Some(mapping) = mappings.get(location.uri.as_str()) else {
        return location;
    };
    let Some(range) = mapping.position_map.virtual_range_to_shape(location.range) else {
        return location;
    };
    Location {
        uri: mapping.source_uri.clone(),
        range,
    }
}

fn map_location_link_to_shape(
    link: LocationLink,
    mappings: &HashMap<String, VirtualUriMapping>,
) -> LocationLink {
    let Some(mapping) = mappings.get(link.target_uri.as_str()) else {
        return link;
    };
    let Some(target_range) = mapping
        .position_map
        .virtual_range_to_shape(link.target_range)
    else {
        return link;
    };
    let Some(target_selection_range) = mapping
        .position_map
        .virtual_range_to_shape(link.target_selection_range)
    else {
        return link;
    };

    LocationLink {
        origin_selection_range: link.origin_selection_range,
        target_uri: mapping.source_uri.clone(),
        target_range,
        target_selection_range,
    }
}

fn map_foreign_semantic_tokens_to_shape(
    child_tokens: &SemanticTokens,
    child_token_types: &[String],
    child_token_modifiers: &[String],
    position_map: &PositionMap,
) -> Vec<ForeignSemanticToken> {
    let mut out = Vec::new();
    let mut line = 0u32;
    let mut col = 0u32;

    for token in &child_tokens.data {
        line += token.delta_line;
        if token.delta_line == 0 {
            col += token.delta_start;
        } else {
            col = token.delta_start;
        }

        let Some(type_name) = child_token_types.get(token.token_type as usize) else {
            continue;
        };
        let Some(mapped_type) = map_semantic_token_type_name_to_shape_index(type_name) else {
            continue;
        };

        let mapped_modifiers = map_semantic_token_modifier_bits_to_shape(
            token.token_modifiers_bitset,
            child_token_modifiers,
        );
        let Some(shape_pos) = position_map.virtual_to_shape(Position {
            line,
            character: col,
        }) else {
            continue;
        };

        out.push(ForeignSemanticToken {
            line: shape_pos.line,
            start_char: shape_pos.character,
            length: token.length,
            token_type: mapped_type,
            token_modifiers_bitset: mapped_modifiers,
        });
    }

    out
}

fn map_semantic_token_type_name_to_shape_index(name: &str) -> Option<u32> {
    match name {
        "namespace" => Some(0),
        "type" | "typeParameter" | "builtinType" => Some(1),
        "class" => Some(2),
        "enum" => Some(3),
        "function" | "builtinFunction" => Some(4),
        "variable" | "builtinVariable" => Some(5),
        "parameter" | "selfParameter" | "clsParameter" => Some(6),
        "property" | "member" => Some(7),
        "keyword" => Some(8),
        "string" | "regexp" => Some(9),
        "number" | "boolean" => Some(10),
        "operator" => Some(11),
        "comment" => Some(12),
        "macro" => Some(13),
        "decorator" => Some(14),
        "interface" => Some(15),
        "enumMember" => Some(16),
        "method" => Some(17),
        _ => {
            let normalized = name
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();

            if normalized.contains("namespace") {
                return Some(0);
            }
            if normalized.contains("interface") {
                return Some(15);
            }
            if normalized.contains("enummember") {
                return Some(16);
            }
            if normalized.contains("enum") {
                return Some(3);
            }
            if normalized.contains("classmethod") || normalized.contains("method") {
                return Some(17);
            }
            if normalized.contains("function") || normalized.contains("callable") {
                return Some(4);
            }
            if normalized.contains("typeparam")
                || normalized.contains("builtintype")
                || normalized == "type"
            {
                return Some(1);
            }
            if normalized.contains("class") {
                return Some(2);
            }
            if normalized.contains("parameter") || normalized.ends_with("param") {
                return Some(6);
            }
            if normalized.contains("property") || normalized.contains("member") {
                return Some(7);
            }
            if normalized.contains("keyword") {
                return Some(8);
            }
            if normalized.contains("string") || normalized.contains("regexp") {
                return Some(9);
            }
            if normalized.contains("number") || normalized.contains("boolean") {
                return Some(10);
            }
            if normalized.contains("operator") {
                return Some(11);
            }
            if normalized.contains("comment") {
                return Some(12);
            }
            if normalized.contains("decorator") {
                return Some(14);
            }
            if normalized.contains("variable") || normalized.contains("builtin") {
                return Some(5);
            }
            None
        }
    }
}

fn map_semantic_token_modifier_bits_to_shape(bits: u32, child_modifiers: &[String]) -> u32 {
    let mut mapped = 0u32;
    for (idx, name) in child_modifiers.iter().enumerate() {
        let bit = 1u32 << idx;
        if bits & bit == 0 {
            continue;
        }
        match name.as_str() {
            "declaration" => mapped |= 1,     // bit 0
            "definition" => mapped |= 1 << 1, // bit 1
            "readonly" => mapped |= 1 << 2,   // bit 2
            "static" => mapped |= 1 << 3,     // bit 3
            "deprecated" => mapped |= 1 << 4, // bit 4
            "defaultLibrary" | "defaultlibrary" | "builtin" => mapped |= 1 << 5, // bit 5
            "modification" => mapped |= 1 << 6, // bit 6
            _ => {}
        }
    }
    mapped
}

fn full_document_semantic_tokens_range(content: &str) -> Range {
    if content.is_empty() {
        return Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        };
    }

    let mut line = 0u32;
    let mut last_line_len = 0u32;
    for chunk in content.split('\n') {
        last_line_len = chunk.chars().count() as u32;
        line += 1;
    }
    let end_line = line.saturating_sub(1);

    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: end_line,
            character: last_line_len,
        },
    }
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

fn span_to_range(source: &str, span: Span) -> Range {
    let (start_line, start_col) = crate::util::offset_to_line_col(source, span.start);
    let (end_line, end_col) = crate::util::offset_to_line_col(source, span.end);
    Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    }
}

fn fallback_range() -> Range {
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 1,
        },
    }
}

/// Sanitize a URI string for use as a filename component.
fn sanitize_filename(uri: &str) -> String {
    uri.replace("://", "_")
        .replace('/', "_")
        .replace('\\', "_")
        .replace(':', "_")
        .replace('.', "_")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp_server::ls_types::{HoverContents, MarkupKind, SemanticToken, SemanticTokens};

    #[test]
    fn test_position_map_round_trip() {
        let source = "fn python analyze(data: DataTable) -> number {\n    import pandas\n    return data.mean()\n}\n";
        let body_start = source.find("import").unwrap();
        let body_end = source.rfind('}').unwrap();
        let span = Span::new(body_start, body_end);
        let map = PositionMap::new(span, source);

        let shape_pos = Position {
            line: 1,
            character: 4,
        };
        let virtual_pos = map.shape_to_virtual(shape_pos);
        assert!(virtual_pos.is_some());

        let back = map.virtual_to_shape(virtual_pos.unwrap());
        assert!(back.is_some());
        assert_eq!(back.unwrap().line, shape_pos.line);
    }

    #[test]
    fn test_find_foreign_block_at_offset() {
        // Just verify it returns None for empty items
        assert!(find_foreign_block_at_offset(&[], 10).is_none());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(
            sanitize_filename("file:///home/user/test.shape"),
            "file__home_user_test_shape"
        );
    }

    #[test]
    fn test_normalize_file_extension() {
        assert_eq!(normalize_file_extension(".py"), "py");
        assert_eq!(normalize_file_extension("jl"), "jl");
        assert_eq!(normalize_file_extension(""), "txt");
    }

    #[test]
    fn test_virtual_doc_mapping_preserves_source_columns_after_dedent() {
        let source = r#"fn python percentile(values: Array<number>, pct: number) -> number {
    sorted_v = sorted(values)
    k = (len(sorted_v) - 1) * (pct / 100.0)
    return k
}"#;
        let program = shape_ast::parser::parse_program(source).expect("program should parse");
        let def = match &program.items[0] {
            Item::ForeignFunction(def, _) => def,
            _ => panic!("expected first item to be foreign function"),
        };

        let vdoc = generate_virtual_document(
            def,
            source,
            "file:///tmp/test.shape",
            std::path::Path::new("/tmp"),
            ".py",
        );

        let first_line_shape = Position {
            line: 1,
            character: 4,
        };
        let first_line_virtual = vdoc
            .position_map
            .shape_to_virtual(first_line_shape)
            .expect("first body line should map");
        assert_eq!(first_line_virtual.line, 1);
        assert_eq!(first_line_virtual.character, 4);
        assert_eq!(
            vdoc.position_map
                .virtual_to_shape(first_line_virtual)
                .expect("roundtrip should map"),
            first_line_shape
        );

        let second_line_shape = Position {
            line: 2,
            character: 4,
        };
        let second_line_virtual = vdoc
            .position_map
            .shape_to_virtual(second_line_shape)
            .expect("second body line should map");
        assert_eq!(second_line_virtual.line, 2);
        assert_eq!(second_line_virtual.character, 4);
        assert_eq!(
            vdoc.position_map
                .virtual_to_shape(second_line_virtual)
                .expect("roundtrip should map"),
            second_line_shape
        );
    }

    #[test]
    fn test_virtual_doc_header_uses_async_def_for_async_foreign_functions() {
        let source = r#"async fn python fetch_json(url: string) -> Array<number> {
    import aiohttp
    async with aiohttp.ClientSession() as session:
        async with session.get(url) as response:
            data = await response.json()
    return data["values"]
}"#;
        let program = shape_ast::parser::parse_program(source).expect("program should parse");
        let def = match &program.items[0] {
            Item::ForeignFunction(def, _) => def,
            _ => panic!("expected first item to be foreign function"),
        };

        let vdoc = generate_virtual_document(
            def,
            source,
            "file:///tmp/test_async.shape",
            std::path::Path::new("/tmp"),
            ".py",
        );

        assert!(
            vdoc.content.starts_with("async def fetch_json(url):\n"),
            "unexpected virtual document header: {:?}",
            vdoc.content.lines().next()
        );
    }

    #[test]
    fn test_parse_hover_response_supports_markup_content_and_range() {
        let response = serde_json::json!({
            "result": {
                "contents": {
                    "kind": "markdown",
                    "value": "**sorted_v**: `list[float]`"
                },
                "range": {
                    "start": { "line": 3, "character": 2 },
                    "end": { "line": 3, "character": 10 }
                }
            }
        });

        let hover = parse_hover_response(response).expect("hover should parse");
        match hover.contents {
            HoverContents::Markup(markup) => {
                assert_eq!(markup.kind, MarkupKind::Markdown);
                assert!(markup.value.contains("sorted_v"));
            }
            other => panic!("expected markup hover, got {other:?}"),
        }
        assert_eq!(
            hover.range,
            Some(Range {
                start: Position {
                    line: 3,
                    character: 2,
                },
                end: Position {
                    line: 3,
                    character: 10,
                },
            })
        );
    }

    #[test]
    fn test_parse_document_diagnostic_report_extracts_items() {
        let response = serde_json::json!({
            "result": {
                "kind": "full",
                "items": [
                    {
                        "range": {
                            "start": { "line": 1, "character": 2 },
                            "end": { "line": 1, "character": 5 }
                        },
                        "severity": 1,
                        "message": "undefined name"
                    }
                ]
            }
        });

        let diagnostics =
            parse_document_diagnostic_report(response).expect("diagnostics should parse");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "undefined name");
        assert_eq!(diagnostics[0].range.start.line, 1);
        assert_eq!(diagnostics[0].range.start.character, 2);
    }

    #[test]
    fn test_parse_document_diagnostic_report_extracts_nested_full_report_items() {
        let response = serde_json::json!({
            "result": {
                "kind": "workspace",
                "fullDocumentDiagnosticReport": {
                    "kind": "full",
                    "items": [
                        {
                            "range": {
                                "start": { "line": 2, "character": 0 },
                                "end": { "line": 2, "character": 4 }
                            },
                            "severity": 1,
                            "message": "bad call"
                        }
                    ]
                }
            }
        });

        let diagnostics =
            parse_document_diagnostic_report(response).expect("nested diagnostics should parse");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "bad call");
        assert_eq!(diagnostics[0].range.start.line, 2);
    }

    #[test]
    fn test_extract_semantic_tokens_legend() {
        let init_response = serde_json::json!({
            "result": {
                "capabilities": {
                    "semanticTokensProvider": {
                        "legend": {
                            "tokenTypes": ["variable", "function"],
                            "tokenModifiers": ["declaration", "readonly"]
                        }
                    }
                }
            }
        });

        let (token_types, token_modifiers) = extract_semantic_tokens_legend(&init_response);
        assert_eq!(token_types, vec!["variable", "function"]);
        assert_eq!(token_modifiers, vec!["declaration", "readonly"]);
    }

    #[test]
    fn test_extract_semantic_tokens_legend_falls_back_when_provider_has_no_legend() {
        let init_response = serde_json::json!({
            "result": {
                "capabilities": {
                    "semanticTokensProvider": {}
                }
            }
        });

        let (token_types, token_modifiers) = extract_semantic_tokens_legend(&init_response);
        assert_eq!(
            token_types,
            CHILD_SEMANTIC_TOKEN_TYPES
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            token_modifiers,
            CHILD_SEMANTIC_TOKEN_MODIFIERS
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_semantic_tokens_legend_empty_when_provider_absent() {
        let init_response = serde_json::json!({
            "result": {
                "capabilities": {}
            }
        });

        let (token_types, token_modifiers) = extract_semantic_tokens_legend(&init_response);
        assert!(token_types.is_empty());
        assert!(token_modifiers.is_empty());
    }

    #[test]
    fn test_child_client_capabilities_request_markdown_hover_and_semantic_tokens() {
        let caps = child_client_capabilities();
        let hover_formats = caps
            .pointer("/textDocument/hover/contentFormat")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            hover_formats.iter().any(|v| v.as_str() == Some("markdown")),
            "child capabilities must advertise markdown hover support"
        );

        assert_eq!(
            caps.pointer("/textDocument/semanticTokens/requests/full")
                .and_then(|v| v.as_bool()),
            Some(true),
            "child capabilities must request semanticTokens/full support"
        );
        assert_eq!(
            caps.pointer("/textDocument/semanticTokens/formats/0")
                .and_then(|v| v.as_str()),
            Some("relative")
        );
    }

    #[test]
    fn test_map_foreign_semantic_tokens_to_shape_coordinates() {
        let source = "fn python test() {\n    x = 1\n}";
        let program = shape_ast::parser::parse_program(source).expect("program should parse");
        let def = match &program.items[0] {
            Item::ForeignFunction(def, _) => def,
            _ => panic!("expected first item to be foreign function"),
        };
        let vdoc = generate_virtual_document(
            def,
            source,
            "file:///tmp/test.shape",
            std::path::Path::new("/tmp"),
            ".py",
        );

        let tokens = SemanticTokens {
            result_id: None,
            data: vec![SemanticToken {
                delta_line: 1,
                delta_start: 4,
                length: 1,
                token_type: 0,
                token_modifiers_bitset: 1,
            }],
        };
        let mapped = map_foreign_semantic_tokens_to_shape(
            &tokens,
            &["variable".to_string()],
            &["declaration".to_string()],
            &vdoc.position_map,
        );

        assert_eq!(
            mapped,
            vec![ForeignSemanticToken {
                line: 1,
                start_char: 4,
                length: 1,
                token_type: 5,
                token_modifiers_bitset: 1,
            }]
        );
    }

    #[test]
    fn test_map_semantic_token_type_name_supports_common_builtin_aliases() {
        assert_eq!(
            map_semantic_token_type_name_to_shape_index("builtinFunction"),
            Some(4)
        );
        assert_eq!(
            map_semantic_token_type_name_to_shape_index("builtinType"),
            Some(1)
        );
        assert_eq!(
            map_semantic_token_type_name_to_shape_index("selfParameter"),
            Some(6)
        );
        assert_eq!(
            map_semantic_token_type_name_to_shape_index("builtInType"),
            Some(1)
        );
        assert_eq!(
            map_semantic_token_type_name_to_shape_index("builtin"),
            Some(5)
        );
    }

    #[test]
    fn test_full_document_semantic_tokens_range_covers_entire_document() {
        let range = full_document_semantic_tokens_range("a\nbc\n");
        assert_eq!(
            range,
            Range {
                start: Position {
                    line: 0,
                    character: 0
                },
                end: Position {
                    line: 2,
                    character: 0
                }
            }
        );

        let single_line = full_document_semantic_tokens_range("abc");
        assert_eq!(
            single_line,
            Range {
                start: Position {
                    line: 0,
                    character: 0
                },
                end: Position {
                    line: 0,
                    character: 3
                }
            }
        );
    }

    #[tokio::test]
    async fn test_update_documents_reports_missing_runtime_config_for_foreign_language() {
        let source = r#"fn python percentile(values: Array<number>, pct: number) -> number {
  return pct
}
"#;
        let program = shape_ast::parser::parse_program(source).expect("program should parse");
        let tmp = tempfile::tempdir().expect("tempdir");
        let manager = ForeignLspManager::new(tmp.path().to_path_buf());

        let diagnostics = manager
            .update_documents("file:///tmp/test.shape", source, &program.items, None, None)
            .await;

        assert_eq!(diagnostics.len(), 1, "expected one startup diagnostic");
        assert!(
            diagnostics[0]
                .message
                .contains("No child LSP config for foreign language 'python'"),
            "unexpected diagnostic message: {}",
            diagnostics[0].message
        );
    }

    #[tokio::test]
    async fn test_update_documents_dedupes_missing_runtime_config_diagnostics_per_language() {
        let source = r#"fn python p1() -> number {
  return 1
}
fn python p2() -> number {
  return 2
}
"#;
        let program = shape_ast::parser::parse_program(source).expect("program should parse");
        let tmp = tempfile::tempdir().expect("tempdir");
        let manager = ForeignLspManager::new(tmp.path().to_path_buf());

        let diagnostics = manager
            .update_documents("file:///tmp/test.shape", source, &program.items, None, None)
            .await;

        assert_eq!(
            diagnostics.len(),
            1,
            "expected deduped missing-runtime diagnostic, got {:?}",
            diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        );
    }
}
