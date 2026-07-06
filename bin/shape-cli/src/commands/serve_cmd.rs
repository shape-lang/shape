use anyhow::{Result, bail};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use shape_runtime::engine::ShapeEngine;
use shape_vm::BytecodeExecutor;
use shape_vm::remote::{
    AuthRequest, AuthResponse, BlobNegotiationRequest, BlobSidecar, ExecuteFileRequest,
    ExecuteProjectRequest, ExecuteRequest, ExecuteResponse, ExecutionMetrics, ServerInfo,
    ValidatePathRequest, ValidateRequest, ValidateResponse, WireDiagnostic, WireMessage,
};
use shape_wire::WireValue;
use shape_wire::transport::framing::{decode_framed, encode_framed};

use crate::cli_args::ExecutionModeArg;
use crate::commands::ProviderOptions;
use crate::extension_loading;

/// Pre-loaded language runtimes for polyglot remote execution.
type LanguageRuntimes =
    HashMap<String, Arc<shape_runtime::plugins::language_runtime::PluginLanguageRuntime>>;

/// Server configuration derived from CLI flags.
struct ServeConfig {
    auth_token: Option<String>,
    max_concurrent: usize,
    sandbox: SandboxLevel,
    /// WF-1D security wiring: the permission envelope actually enforced by
    /// every execution this server runs (derived from `sandbox` + bind class).
    security: SecurityPosture,
    _mode: ExecutionModeArg,
    extensions: Vec<std::path::PathBuf>,
    provider_opts: ProviderOptions,
}

#[derive(Debug, Clone, Copy)]
pub enum SandboxLevel {
    Strict,
    Permissive,
    None,
}

impl std::str::FromStr for SandboxLevel {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            // `moderate` is the ratified alias for the permissive envelope.
            "strict" => Ok(SandboxLevel::Strict),
            "permissive" | "moderate" => Ok(SandboxLevel::Permissive),
            "none" | "off" => Ok(SandboxLevel::None),
            _ => Err(format!(
                "unknown sandbox level: '{}' (expected strict|moderate|off)",
                s
            )),
        }
    }
}

/// The concrete security envelope a served execution runs under (WF-1D).
#[derive(Clone)]
struct SecurityPosture {
    granted: shape_abi_v1::PermissionSet,
    scope: shape_abi_v1::ScopeConstraints,
    limits: shape_vm::resource_limits::ResourceLimits,
}

/// Map a `--sandbox` level plus the bind class into a real
/// `PermissionSet` + `ScopeConstraints` + `ResourceLimits`, per the ratified
/// per-bind-class posture (project_design_ratification_2026_07_05 Q15/Q28/Q52):
///
/// - `strict`   → grant nothing (`pure`), sandboxed resource caps.
/// - `moderate` → read + env + time + random + outbound-connect; still no
///   `fs.write` / `net.listen` / `process`, sandboxed caps.
/// - `off`      → full permissions, unlimited resources (trusted).
///
/// Bind class: loopback binds the level's set; a non-loopback bind is clamped
/// to `pure` ("Pure-only until configured") regardless of level — fail closed.
///
/// Foreign code (`Permission::Ffi`): `shape serve` defaults to the STRICT-EMPTY
/// ffi posture (ffi-rebuild §4.8.2 / OQ-6, ratified 2026-07-05) — the deliberate
/// asymmetry vs. the local trusted-run unscoped grant. `strict`/`moderate` never
/// grant `Ffi`; only `off` (explicit total trust) confers it via `full()`. The
/// returned scope carries empty `ffi_languages`/`ffi_libraries`/`ffi_symbols`
/// (strict-empty) — WF-2A wires the pre-`dlopen` scope check against them.
fn derive_serve_security(level: SandboxLevel, is_loopback: bool) -> SecurityPosture {
    use shape_abi_v1::{Permission, PermissionSet, ScopeConstraints};
    use shape_vm::resource_limits::ResourceLimits;

    let (granted, limits) = match level {
        // Strict/moderate deliberately omit `Ffi` (fail closed on foreign code).
        SandboxLevel::Strict => (PermissionSet::pure(), ResourceLimits::sandboxed()),
        SandboxLevel::Permissive => {
            let mut set = PermissionSet::pure();
            set.insert(Permission::FsRead);
            set.insert(Permission::Env);
            set.insert(Permission::Time);
            set.insert(Permission::Random);
            set.insert(Permission::NetConnect);
            (set, ResourceLimits::sandboxed())
        }
        // `off` == operator-declared total trust: `full()` includes `Ffi`.
        SandboxLevel::None => (PermissionSet::full(), ResourceLimits::unlimited()),
    };

    // Non-loopback binds fail closed to Pure-only until explicitly configured.
    let granted = if is_loopback {
        granted
    } else {
        granted.intersection(&PermissionSet::pure())
    };

    SecurityPosture {
        granted,
        scope: ScopeConstraints::none(),
        limits,
    }
}

/// Per-connection state.
struct ConnectionState {
    authenticated: bool,
    blob_cache: shape_vm::remote::RemoteBlobCache,
    pending_sidecars: HashMap<u32, BlobSidecar>,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            authenticated: false,
            blob_cache: shape_vm::remote::RemoteBlobCache::default_cache(),
            pending_sidecars: HashMap::new(),
        }
    }
}

/// Entry point for `shape serve`.
pub async fn run_serve(
    address: String,
    mode: ExecutionModeArg,
    extensions: Vec<std::path::PathBuf>,
    provider_opts: &ProviderOptions,
    tls_cert: Option<std::path::PathBuf>,
    tls_key: Option<std::path::PathBuf>,
    auth_token: Option<String>,
    sandbox: String,
    max_concurrent: usize,
) -> Result<()> {
    let addr: SocketAddr = address.parse()?;

    // Non-loopback bind gate (distributed §4.7 / Q29 / OQ-4). Loopback stays
    // plain for dev; a non-loopback bind must present BOTH TLS material AND an
    // auth token before it will serve — a non-loopback server executes
    // arbitrary sender-supplied bytecode, so refusing (not warning) is the
    // honest posture.
    if !addr.ip().is_loopback() {
        if tls_cert.is_none() || tls_key.is_none() {
            bail!(
                "Refusing to start on non-loopback address {} without TLS.\n\
                 Provide --tls-cert and --tls-key, or bind to 127.0.0.1.",
                addr
            );
        }
        // §4.7: non-loopback binds REQUIRE an auth token (upgraded from a
        // warning to a refusal) — an unauthenticated non-loopback endpoint
        // lets any client run code on this host.
        if auth_token.is_none() {
            bail!(
                "Refusing to start on non-loopback address {} without --auth-token.\n\
                 A non-loopback server requires authentication; pass --auth-token, \
                 or bind to 127.0.0.1 for local development.",
                addr
            );
        }
    }

    let sandbox_level: SandboxLevel = sandbox.parse().map_err(|e: String| anyhow::anyhow!(e))?;

    // Transport security honesty (distributed §4.7 / Q29 / OQ-4): TLS-on-TCP
    // termination (tokio-rustls) is NOT yet wired into the serve accept loop.
    // The `--tls-cert`/`--tls-key` gate above proves the operator INTENDED
    // TLS, but the accept loop still speaks plaintext framing today. Rather
    // than silently pretending the connection is encrypted (the previous
    // `let _ = (tls_cert, tls_key); // future enhancement` lie), surface it
    // loudly so no operator mistakes cert presence for active encryption.
    if !addr.ip().is_loopback() && (tls_cert.is_some() || tls_key.is_some()) {
        eprintln!(
            "  WARNING: TLS termination is not yet active — traffic on {} is \
             NOT encrypted at the transport layer (auth token still enforced).",
            addr
        );
    }
    let _ = (tls_cert, tls_key);

    // Load language runtimes at startup for polyglot remote execution.
    // Extensions are loaded once via the full discovery + load path;
    // the runtimes are Arc-wrapped and shared across all connections.
    let language_runtimes: Arc<LanguageRuntimes> = {
        let mut engine = ShapeEngine::new()
            .map_err(|e| anyhow::anyhow!("failed to create engine for extension loading: {}", e))?;
        // Use the standard extension discovery path (auto-scans ~/.shape/extensions/)
        let specs =
            extension_loading::collect_startup_specs(provider_opts, None, None, None, &extensions);
        let loaded = extension_loading::load_specs(
            &mut engine,
            &specs,
            |spec, info| {
                eprintln!(
                    "  Loaded extension: {} ({})",
                    info.name,
                    spec.path.display()
                );
            },
            |spec, err| {
                eprintln!(
                    "  Failed to load extension {}: {}",
                    spec.path.display(),
                    err
                );
            },
        );
        if loaded > 0 {
            eprintln!("  {} extension(s) loaded", loaded);
        }
        let runtimes = engine.language_runtimes();
        if !runtimes.is_empty() {
            let names: Vec<&str> = runtimes.keys().map(|s| s.as_str()).collect();
            eprintln!("  language runtimes: {}", names.join(", "));
        }
        Arc::new(runtimes)
    };

    // WF-1D security wiring: derive the real permission + resource envelope
    // from the sandbox level and bind class ONCE at startup. Every execution
    // this server runs is gated by this posture.
    let security = derive_serve_security(sandbox_level, addr.ip().is_loopback());

    let config = Arc::new(ServeConfig {
        auth_token,
        max_concurrent,
        sandbox: sandbox_level,
        security,
        _mode: mode,
        extensions,
        provider_opts: provider_opts.clone(),
    });

    let semaphore = Arc::new(Semaphore::new(config.max_concurrent));

    let listener = TcpListener::bind(addr).await?;
    eprintln!("Shape serve listening on {}", addr);
    let granted_names: Vec<&str> = config.security.granted.iter().map(|p| p.name()).collect();
    eprintln!(
        "  sandbox: {:?}, max-concurrent: {}, auth: {}",
        config.sandbox,
        config.max_concurrent,
        if config.auth_token.is_some() {
            "required"
        } else {
            "none"
        },
    );
    eprintln!(
        "  granted: [{}]{}",
        granted_names.join(", "),
        if granted_names.is_empty() {
            " (pure — no I/O)"
        } else {
            ""
        },
    );

    loop {
        let (socket, peer) = listener.accept().await?;
        eprintln!("Connection from {}", peer);

        let config = config.clone();
        let semaphore = semaphore.clone();
        let language_runtimes = language_runtimes.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(socket, &config, &semaphore, &language_runtimes).await
            {
                eprintln!("Connection error from {}: {}", peer, e);
            }
        });
    }
}

async fn handle_connection(
    mut socket: tokio::net::TcpStream,
    config: &ServeConfig,
    semaphore: &Semaphore,
    language_runtimes: &LanguageRuntimes,
) -> Result<()> {
    let mut state = ConnectionState::new();

    loop {
        // Read 4-byte length prefix
        let mut len_buf = [0u8; 4];
        match socket.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let msg_len = u32::from_be_bytes(len_buf) as usize;

        if msg_len > 256 * 1024 * 1024 {
            bail!("message too large: {} bytes", msg_len);
        }

        // Read framed payload
        let mut payload = vec![0u8; msg_len];
        socket.read_exact(&mut payload).await?;

        // Decode framing (flags byte + optional zstd decompression)
        let decompressed =
            decode_framed(&payload).map_err(|e| anyhow::anyhow!("framing decode error: {}", e))?;

        // Deserialize from MessagePack
        let message: WireMessage = shape_wire::decode_message(&decompressed)
            .map_err(|e| anyhow::anyhow!("MessagePack decode error: {}", e))?;

        // Dispatch
        let response = match message {
            WireMessage::Auth(req) => Some(handle_auth(req, config, &mut state)),
            WireMessage::Ping(_) => Some(handle_ping()),
            WireMessage::Execute(req) => {
                if requires_auth(config) && !state.authenticated {
                    Some(WireMessage::ExecuteResponse(ExecuteResponse {
                        request_id: req.request_id,
                        success: false,
                        value: WireValue::Null,
                        stdout: None,
                        error: Some(
                            "Authentication required. Send Auth message first.".to_string(),
                        ),
                        content_terminal: None,
                        content_html: None,
                        diagnostics: vec![],
                        metrics: None,
                        print_output: None,
                    }))
                } else {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|_| anyhow::anyhow!("semaphore closed"))?;
                    Some(handle_execute(req, config).await)
                }
            }
            WireMessage::Validate(req) => {
                if requires_auth(config) && !state.authenticated {
                    Some(WireMessage::ValidateResponse(ValidateResponse {
                        request_id: req.request_id,
                        success: false,
                        diagnostics: vec![WireDiagnostic {
                            severity: "error".to_string(),
                            message: "Authentication required.".to_string(),
                            line: None,
                            column: None,
                        }],
                    }))
                } else {
                    Some(handle_validate(req))
                }
            }
            WireMessage::Call(req) => {
                if requires_auth(config) && !state.authenticated {
                    Some(WireMessage::CallResponse(
                        shape_vm::remote::RemoteCallResponse {
                            result: Err(shape_vm::remote::RemoteCallError::new(
                                shape_vm::remote::RemoteErrorKind::AuthRequired,
                                "Authentication required.",
                            )),
                        },
                    ))
                } else {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|_| anyhow::anyhow!("semaphore closed"))?;
                    Some(handle_call(
                        req,
                        &mut state,
                        language_runtimes,
                        &config.security.granted,
                    ))
                }
            }
            WireMessage::ExecuteFile(req) => {
                if requires_auth(config) && !state.authenticated {
                    Some(WireMessage::ExecuteResponse(ExecuteResponse {
                        request_id: req.request_id,
                        success: false,
                        value: WireValue::Null,
                        stdout: None,
                        error: Some(
                            "Authentication required. Send Auth message first.".to_string(),
                        ),
                        content_terminal: None,
                        content_html: None,
                        diagnostics: vec![],
                        metrics: None,
                        print_output: None,
                    }))
                } else {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|_| anyhow::anyhow!("semaphore closed"))?;
                    Some(handle_execute_file(req, config).await)
                }
            }
            WireMessage::ExecuteProject(req) => {
                if requires_auth(config) && !state.authenticated {
                    Some(WireMessage::ExecuteResponse(ExecuteResponse {
                        request_id: req.request_id,
                        success: false,
                        value: WireValue::Null,
                        stdout: None,
                        error: Some(
                            "Authentication required. Send Auth message first.".to_string(),
                        ),
                        content_terminal: None,
                        content_html: None,
                        diagnostics: vec![],
                        metrics: None,
                        print_output: None,
                    }))
                } else {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|_| anyhow::anyhow!("semaphore closed"))?;
                    Some(handle_execute_project(req, config).await)
                }
            }
            WireMessage::ValidatePath(req) => {
                if requires_auth(config) && !state.authenticated {
                    Some(WireMessage::ValidateResponse(ValidateResponse {
                        request_id: req.request_id,
                        success: false,
                        diagnostics: vec![WireDiagnostic {
                            severity: "error".to_string(),
                            message: "Authentication required.".to_string(),
                            line: None,
                            column: None,
                        }],
                    }))
                } else {
                    Some(handle_validate_path(req))
                }
            }
            WireMessage::BlobNegotiation(req) => Some(handle_negotiation(req, &state.blob_cache)),
            WireMessage::Sidecar(s) => {
                state.pending_sidecars.insert(s.sidecar_id, s);
                continue;
            }
            // Ignore response-type messages from clients
            WireMessage::CallResponse(_)
            | WireMessage::BlobNegotiationReply(_)
            | WireMessage::ExecuteResponse(_)
            | WireMessage::ValidateResponse(_)
            | WireMessage::AuthResponse(_)
            | WireMessage::Pong(_) => continue,
        };

        if let Some(resp) = response {
            // Encode response as MessagePack + framing
            let mp = shape_wire::encode_message(&resp)
                .map_err(|e| anyhow::anyhow!("response encode error: {}", e))?;
            let framed = encode_framed(&mp);

            let len = framed.len() as u32;
            socket.write_all(&len.to_be_bytes()).await?;
            socket.write_all(&framed).await?;
            socket.flush().await?;
        }
    }
}

fn requires_auth(config: &ServeConfig) -> bool {
    config.auth_token.is_some()
}

fn handle_auth(req: AuthRequest, config: &ServeConfig, state: &mut ConnectionState) -> WireMessage {
    match &config.auth_token {
        Some(expected) if req.token == *expected => {
            state.authenticated = true;
            WireMessage::AuthResponse(AuthResponse {
                authenticated: true,
                error: None,
            })
        }
        Some(_) => WireMessage::AuthResponse(AuthResponse {
            authenticated: false,
            error: Some("Invalid token.".to_string()),
        }),
        None => {
            // No auth configured — always succeed
            state.authenticated = true;
            WireMessage::AuthResponse(AuthResponse {
                authenticated: true,
                error: None,
            })
        }
    }
}

fn handle_ping() -> WireMessage {
    WireMessage::Pong(ServerInfo {
        shape_version: env!("CARGO_PKG_VERSION").to_string(),
        wire_protocol: shape_wire::WIRE_PROTOCOL_V2,
        capabilities: vec![
            "execute".to_string(),
            "execute-file".to_string(),
            "execute-project".to_string(),
            "validate".to_string(),
            "validate-path".to_string(),
            "call".to_string(),
            "blob-negotiation".to_string(),
        ],
    })
}

async fn handle_execute(req: ExecuteRequest, config: &ServeConfig) -> WireMessage {
    let code = req.code;
    let request_id = req.request_id;
    let extensions = config.extensions.clone();
    let provider_opts = config.provider_opts.clone();
    let security = config.security.clone();

    let result = tokio::task::spawn_blocking(move || {
        execute_code_in_process(&code, &extensions, &provider_opts, &security)
    })
    .await;

    match result {
        Ok(Ok(r)) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id,
            success: true,
            value: r.value,
            stdout: r.stdout,
            error: None,
            content_terminal: r.content_terminal,
            content_html: r.content_html,
            diagnostics: vec![],
            metrics: Some(ExecutionMetrics {
                instructions_executed: 0,
                wall_time_ms: r.wall_time_ms,
                memory_bytes_peak: 0,
            }),
            print_output: None,
        }),
        Ok(Err(err)) => {
            let (message, diagnostics) = format_error(&err);
            WireMessage::ExecuteResponse(ExecuteResponse {
                request_id,
                success: false,
                value: WireValue::Null,
                stdout: None,
                error: Some(message),
                content_terminal: None,
                content_html: None,
                diagnostics,
                metrics: None,
                print_output: None,
            })
        }
        Err(join_err) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some(format!("Execution panicked: {}", join_err)),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![],
            metrics: None,
            print_output: None,
        }),
    }
}

fn handle_validate(req: ValidateRequest) -> WireMessage {
    let diagnostics = match shape_ast::parse_program(&req.code) {
        Ok(_) => vec![],
        Err(e) => vec![WireDiagnostic {
            severity: "error".to_string(),
            message: e.to_string(),
            line: None,
            column: None,
        }],
    };

    let success = diagnostics.iter().all(|d| d.severity != "error");

    WireMessage::ValidateResponse(ValidateResponse {
        request_id: req.request_id,
        success,
        diagnostics,
    })
}

async fn handle_execute_file(req: ExecuteFileRequest, config: &ServeConfig) -> WireMessage {
    let request_id = req.request_id;
    let path = req.path.clone();
    let cwd = req.cwd.clone();
    let extensions = config.extensions.clone();
    let provider_opts = config.provider_opts.clone();
    let security = config.security.clone();

    let result = tokio::task::spawn_blocking(move || {
        execute_file_in_process(
            &path,
            cwd.as_deref(),
            &extensions,
            &provider_opts,
            &security,
        )
    })
    .await;

    match result {
        Ok(Ok(r)) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id,
            success: true,
            value: r.value,
            stdout: r.stdout,
            error: None,
            content_terminal: r.content_terminal,
            content_html: r.content_html,
            diagnostics: vec![],
            metrics: Some(ExecutionMetrics {
                instructions_executed: 0,
                wall_time_ms: r.wall_time_ms,
                memory_bytes_peak: 0,
            }),
            print_output: None,
        }),
        Ok(Err(err)) => {
            let (message, diagnostics) = format_error(&err);
            WireMessage::ExecuteResponse(ExecuteResponse {
                request_id,
                success: false,
                value: WireValue::Null,
                stdout: None,
                error: Some(message),
                content_terminal: None,
                content_html: None,
                diagnostics,
                metrics: None,
                print_output: None,
            })
        }
        Err(join_err) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some(format!("Execution panicked: {}", join_err)),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![],
            metrics: None,
            print_output: None,
        }),
    }
}

async fn handle_execute_project(req: ExecuteProjectRequest, config: &ServeConfig) -> WireMessage {
    let request_id = req.request_id;
    let project_dir = req.project_dir.clone();
    let extensions = config.extensions.clone();
    let provider_opts = config.provider_opts.clone();
    let security = config.security.clone();

    let result = tokio::task::spawn_blocking(move || {
        execute_project_in_process(&project_dir, &extensions, &provider_opts, &security)
    })
    .await;

    match result {
        Ok(Ok(r)) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id,
            success: true,
            value: r.value,
            stdout: r.stdout,
            error: None,
            content_terminal: r.content_terminal,
            content_html: r.content_html,
            diagnostics: vec![],
            metrics: Some(ExecutionMetrics {
                instructions_executed: 0,
                wall_time_ms: r.wall_time_ms,
                memory_bytes_peak: 0,
            }),
            print_output: None,
        }),
        Ok(Err(err)) => {
            let (message, diagnostics) = format_error(&err);
            WireMessage::ExecuteResponse(ExecuteResponse {
                request_id,
                success: false,
                value: WireValue::Null,
                stdout: None,
                error: Some(message),
                content_terminal: None,
                content_html: None,
                diagnostics,
                metrics: None,
                print_output: None,
            })
        }
        Err(join_err) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some(format!("Execution panicked: {}", join_err)),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![],
            metrics: None,
            print_output: None,
        }),
    }
}

fn handle_validate_path(req: ValidatePathRequest) -> WireMessage {
    let path = std::path::Path::new(&req.path);

    // Determine the source file to validate
    let (source, context_path) = if path.is_dir() {
        // Project directory — find entry point from shape.toml
        match shape_runtime::project::find_project_root(path) {
            Some(project) => match &project.config.project.entry {
                Some(entry) => {
                    let entry_path = project.root_path.join(entry);
                    match std::fs::read_to_string(&entry_path) {
                        Ok(src) => (src, entry_path),
                        Err(e) => {
                            return WireMessage::ValidateResponse(ValidateResponse {
                                request_id: req.request_id,
                                success: false,
                                diagnostics: vec![WireDiagnostic {
                                    severity: "error".to_string(),
                                    message: format!(
                                        "Failed to read entry file '{}': {}",
                                        entry_path.display(),
                                        e
                                    ),
                                    line: None,
                                    column: None,
                                }],
                            });
                        }
                    }
                }
                None => {
                    return WireMessage::ValidateResponse(ValidateResponse {
                        request_id: req.request_id,
                        success: false,
                        diagnostics: vec![WireDiagnostic {
                            severity: "error".to_string(),
                            message: "shape.toml has no [project].entry field".to_string(),
                            line: None,
                            column: None,
                        }],
                    });
                }
            },
            None => {
                return WireMessage::ValidateResponse(ValidateResponse {
                    request_id: req.request_id,
                    success: false,
                    diagnostics: vec![WireDiagnostic {
                        severity: "error".to_string(),
                        message: format!("No shape.toml found in '{}'", path.display()),
                        line: None,
                        column: None,
                    }],
                });
            }
        }
    } else {
        // Single .shape file
        match std::fs::read_to_string(path) {
            Ok(src) => (src, path.to_path_buf()),
            Err(e) => {
                return WireMessage::ValidateResponse(ValidateResponse {
                    request_id: req.request_id,
                    success: false,
                    diagnostics: vec![WireDiagnostic {
                        severity: "error".to_string(),
                        message: format!("Failed to read '{}': {}", path.display(), e),
                        line: None,
                        column: None,
                    }],
                });
            }
        }
    };

    // Parse + compile (type-check) without executing
    let mut diagnostics = Vec::new();

    match shape_ast::parse_program(&source) {
        Ok(ast) => {
            // Try bytecode compilation for type checking
            let compiler = shape_vm::compiler::BytecodeCompiler::new();
            if let Err(e) = compiler.compile(&ast) {
                let (line, column) = extract_location(&e);
                diagnostics.push(WireDiagnostic {
                    severity: "error".to_string(),
                    message: e.to_string(),
                    line,
                    column,
                });
            }
        }
        Err(e) => {
            diagnostics.push(WireDiagnostic {
                severity: "error".to_string(),
                message: e.to_string(),
                line: None,
                column: None,
            });
        }
    }

    let _ = context_path; // used for future module resolution

    let success = diagnostics.iter().all(|d| d.severity != "error");
    WireMessage::ValidateResponse(ValidateResponse {
        request_id: req.request_id,
        success,
        diagnostics,
    })
}

fn handle_call(
    req: shape_vm::remote::RemoteCallRequest,
    _state: &mut ConnectionState,
    language_runtimes: &LanguageRuntimes,
    granted: &shape_abi_v1::PermissionSet,
) -> WireMessage {
    let tmp_dir = std::env::temp_dir().join("shape-serve-snapshots");
    match shape_runtime::snapshot::SnapshotStore::new(&tmp_dir) {
        Ok(store) => {
            // WF-1D: gate the remote Call path with the server's derived grant.
            let response = if language_runtimes.is_empty() {
                shape_vm::remote::execute_remote_call(req, &store, granted)
            } else {
                shape_vm::remote::execute_remote_call_with_runtimes(
                    req,
                    &store,
                    language_runtimes,
                    granted,
                )
            };
            WireMessage::CallResponse(response)
        }
        Err(e) => WireMessage::CallResponse(shape_vm::remote::RemoteCallResponse {
            result: Err(shape_vm::remote::RemoteCallError::new(
                shape_vm::remote::RemoteErrorKind::RuntimeError,
                format!("Failed to create snapshot store: {}", e),
            )),
        }),
    }
}

fn handle_negotiation(
    req: BlobNegotiationRequest,
    cache: &shape_vm::remote::RemoteBlobCache,
) -> WireMessage {
    let response = shape_vm::remote::handle_negotiation(&req, cache);
    WireMessage::BlobNegotiationReply(response)
}

/// Result from in-process execution, carrying structured data.
struct InProcessResult {
    value: WireValue,
    stdout: Option<String>,
    content_terminal: Option<String>,
    content_html: Option<String>,
    wall_time_ms: u64,
}

/// Execute Shape code in-process using the full engine pipeline.
fn execute_code_in_process(
    code: &str,
    _extensions: &[std::path::PathBuf],
    _provider_opts: &ProviderOptions,
    security: &SecurityPosture,
) -> Result<InProcessResult> {
    use shape_runtime::output_adapter::SharedCaptureAdapter;
    use std::time::Instant;

    let start = Instant::now();

    let mut engine =
        ShapeEngine::new().map_err(|e| anyhow::anyhow!("failed to create Shape engine: {}", e))?;

    let mut executor = BytecodeExecutor::new();
    apply_security_posture(&mut executor, security);

    extension_loading::register_extension_capability_modules(&mut engine, &mut executor);
    let module_info = executor.module_schemas();
    engine.register_extension_modules(&module_info);
    engine.register_language_runtime_artifacts();

    let interrupt = Arc::new(AtomicU8::new(0));
    executor.set_interrupt(interrupt);

    crate::module_loading::wire_vm_executor_module_loading(
        &mut engine,
        &mut executor,
        None,
        Some(code),
    )?;

    // Capture print() output so wire responses include stdout.
    let capture = SharedCaptureAdapter::new();
    if let Some(ctx) = engine.runtime.persistent_context_mut() {
        ctx.set_output_adapter(Box::new(capture.clone()));
    }

    let result = engine.execute(&mut executor, code)?;

    let wall_time_ms = start.elapsed().as_millis() as u64;

    // Collect print output from adapter
    let captured_lines = capture.output();
    let stdout: String = captured_lines.iter().map(|l| format!("{}\n", l)).collect();
    let printed_content_html = capture.content_html();

    Ok(InProcessResult {
        value: result.value,
        stdout: if stdout.is_empty() {
            None
        } else {
            Some(stdout)
        },
        content_terminal: result.content_terminal,
        content_html: if printed_content_html.is_empty() {
            result.content_html
        } else {
            Some(printed_content_html.join("\n"))
        },
        wall_time_ms,
    })
}

/// Execute a Shape file in-process using the full engine pipeline.
fn execute_file_in_process(
    path: &str,
    cwd: Option<&str>,
    _extensions: &[std::path::PathBuf],
    _provider_opts: &ProviderOptions,
    security: &SecurityPosture,
) -> Result<InProcessResult> {
    use shape_runtime::output_adapter::SharedCaptureAdapter;
    use std::time::Instant;

    let file_path = std::path::Path::new(path);
    let source = std::fs::read_to_string(file_path)
        .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", path, e))?;

    // Set cwd if specified
    if let Some(cwd) = cwd {
        std::env::set_current_dir(cwd)
            .map_err(|e| anyhow::anyhow!("Failed to set working directory '{}': {}", cwd, e))?;
    } else if let Some(parent) = file_path.parent() {
        let _ = std::env::set_current_dir(parent);
    }

    let start = Instant::now();

    let mut engine =
        ShapeEngine::new().map_err(|e| anyhow::anyhow!("failed to create Shape engine: {}", e))?;

    let mut executor = BytecodeExecutor::new();
    apply_security_posture(&mut executor, security);

    extension_loading::register_extension_capability_modules(&mut engine, &mut executor);
    let module_info = executor.module_schemas();
    engine.register_extension_modules(&module_info);
    engine.register_language_runtime_artifacts();

    let interrupt = Arc::new(AtomicU8::new(0));
    executor.set_interrupt(interrupt);

    crate::module_loading::wire_vm_executor_module_loading(
        &mut engine,
        &mut executor,
        Some(file_path),
        Some(&source),
    )?;

    // Capture print() output so wire responses include stdout.
    let capture = SharedCaptureAdapter::new();
    if let Some(ctx) = engine.runtime.persistent_context_mut() {
        ctx.set_output_adapter(Box::new(capture.clone()));
    }

    let result = engine.execute(&mut executor, &source)?;

    let wall_time_ms = start.elapsed().as_millis() as u64;

    // Collect print output from adapter
    let captured_lines = capture.output();
    let stdout: String = captured_lines.iter().map(|l| format!("{}\n", l)).collect();
    let printed_content_html = capture.content_html();

    Ok(InProcessResult {
        value: result.value,
        stdout: if stdout.is_empty() {
            None
        } else {
            Some(stdout)
        },
        content_terminal: result.content_terminal,
        content_html: if printed_content_html.is_empty() {
            result.content_html
        } else {
            Some(printed_content_html.join("\n"))
        },
        wall_time_ms,
    })
}

/// Execute a Shape project in-process by finding its entry point.
fn execute_project_in_process(
    project_dir: &str,
    extensions: &[std::path::PathBuf],
    provider_opts: &ProviderOptions,
    security: &SecurityPosture,
) -> Result<InProcessResult> {
    let dir = std::path::Path::new(project_dir);

    let project = shape_runtime::project::find_project_root(dir)
        .ok_or_else(|| anyhow::anyhow!("No shape.toml found in '{}'", project_dir))?;

    let entry = project
        .config
        .project
        .entry
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("shape.toml has no [project].entry field"))?;

    let entry_path = project.root_path.join(entry);
    if !entry_path.is_file() {
        bail!(
            "Entry file '{}' not found (resolved to {})",
            entry,
            entry_path.display()
        );
    }

    // WF-1D: intersect the server's grant with the project's declared
    // [permissions] so a project can only ever narrow, never widen, what the
    // server sandbox allows. Fail closed.
    let project_security = project_narrowed_security(&project.config, security);

    execute_file_in_process(
        &entry_path.to_string_lossy(),
        Some(project_dir),
        extensions,
        provider_opts,
        &project_security,
    )
}

/// Apply a `SecurityPosture` to a `BytecodeExecutor` (WF-1D): install the
/// runtime permission envelope + resource limits so gated stdlib dispatch and
/// the load-time capability gate both fail closed.
fn apply_security_posture(executor: &mut BytecodeExecutor, security: &SecurityPosture) {
    executor.set_granted_permissions(Some(security.granted.clone()), Some(security.scope.clone()));
    executor.set_resource_limits(Some(security.limits.clone()));
}

/// Narrow a base `SecurityPosture` by a project's `shape.toml [permissions]`
/// section: the effective grant is the intersection of the server's grant and
/// the project's declared set (a project may only reduce capability). If the
/// project declares no `[permissions]`, the base posture is used unchanged.
fn project_narrowed_security(
    config: &shape_runtime::project::ShapeProject,
    base: &SecurityPosture,
) -> SecurityPosture {
    let project_set = config.effective_permission_set();
    let granted = base.granted.intersection(&project_set);
    SecurityPosture {
        granted,
        scope: base.scope.clone(),
        limits: base.limits.clone(),
    }
}

/// Extract error message and diagnostics from an anyhow error.
fn format_error(err: &anyhow::Error) -> (String, Vec<WireDiagnostic>) {
    use shape_runtime::error::ShapeError;

    if let Some(shape_err) = err.downcast_ref::<ShapeError>() {
        let message = shape_err.to_string();
        let (line, column) = extract_location(shape_err);
        let diag = WireDiagnostic {
            severity: "error".to_string(),
            message: message.clone(),
            line,
            column,
        };
        (message, vec![diag])
    } else {
        (err.to_string(), vec![])
    }
}

/// Extract line/column from a ShapeError if available.
fn extract_location(err: &shape_runtime::error::ShapeError) -> (Option<u32>, Option<u32>) {
    use shape_runtime::error::ShapeError;

    let loc = match err {
        ShapeError::ParseError { location, .. } => location.as_ref(),
        ShapeError::LexError { location, .. } => location.as_ref(),
        ShapeError::SemanticError { location, .. } => location.as_ref(),
        ShapeError::RuntimeError { location, .. } => location.as_ref(),
        _ => None,
    };

    match loc {
        Some(l) => (Some(l.line as u32), Some(l.column as u32)),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::snapshot::SerializableVMValue;
    use shape_vm::remote::{ExecuteRequest, WireMessage, build_call_request};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// Start a real server on a random port, return the bound address.
    async fn start_test_server() -> SocketAddr {
        start_test_server_with_sandbox(SandboxLevel::None).await
    }

    /// Start a real loopback server at the given sandbox level (WF-1D). The
    /// derived permission envelope is the real one — a `Strict` server grants
    /// nothing, so gated I/O fails closed exactly as in production.
    async fn start_test_server_with_sandbox(level: SandboxLevel) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let config = Arc::new(ServeConfig {
            auth_token: None,
            max_concurrent: 4,
            sandbox: level,
            security: derive_serve_security(level, true),
            _mode: ExecutionModeArg::Vm,
            extensions: vec![],
            provider_opts: ProviderOptions::default(),
        });
        let semaphore = Arc::new(Semaphore::new(4));
        let language_runtimes: Arc<LanguageRuntimes> = Arc::new(HashMap::new());

        tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let config = config.clone();
                let semaphore = semaphore.clone();
                let language_runtimes = language_runtimes.clone();
                tokio::spawn(async move {
                    let _ =
                        handle_connection(socket, &config, &semaphore, &language_runtimes).await;
                });
            }
        });

        addr
    }

    /// Send a WireMessage and read the response back.
    async fn roundtrip(stream: &mut TcpStream, msg: &WireMessage) -> WireMessage {
        let mp = shape_wire::encode_message(msg).unwrap();
        let framed = encode_framed(&mp);
        let len = framed.len() as u32;
        stream.write_all(&len.to_be_bytes()).await.unwrap();
        stream.write_all(&framed).await.unwrap();
        stream.flush().await.unwrap();

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf).await.unwrap();
        let decompressed = decode_framed(&resp_buf).unwrap();
        shape_wire::decode_message(&decompressed).unwrap()
    }

    #[test]
    fn derive_serve_security_maps_levels_and_bind_class() {
        use shape_abi_v1::Permission;

        // Strict on loopback → grant nothing, sandboxed caps.
        let strict = derive_serve_security(SandboxLevel::Strict, true);
        assert!(strict.granted.is_empty(), "strict must grant nothing");
        assert!(!strict.granted.contains(&Permission::FsWrite));
        assert!(strict.limits.max_instructions.is_some());

        // Permissive on loopback → read/env/time/random/connect, but no write.
        let perm = derive_serve_security(SandboxLevel::Permissive, true);
        assert!(perm.granted.contains(&Permission::FsRead));
        assert!(perm.granted.contains(&Permission::NetConnect));
        assert!(
            !perm.granted.contains(&Permission::FsWrite),
            "moderate must not grant fs.write — the escape stays refused"
        );
        assert!(!perm.granted.contains(&Permission::Process));

        // None on loopback → full grant, unlimited.
        let none = derive_serve_security(SandboxLevel::None, true);
        assert!(none.granted.contains(&Permission::FsWrite));
        assert!(none.limits.max_instructions.is_none());

        // Non-loopback → Pure-only regardless of level (fail closed).
        let remote_none = derive_serve_security(SandboxLevel::None, false);
        assert!(
            remote_none.granted.is_empty(),
            "non-loopback must clamp to pure until configured"
        );
    }

    #[tokio::test]
    async fn test_ping_over_tcp() {
        let addr = start_test_server().await;
        let mut stream = TcpStream::connect(addr).await.unwrap();

        let resp = roundtrip(
            &mut stream,
            &WireMessage::Ping(shape_vm::remote::PingRequest {}),
        )
        .await;
        match resp {
            WireMessage::Pong(info) => {
                assert_eq!(info.wire_protocol, shape_wire::WIRE_PROTOCOL_V2);
                assert!(info.capabilities.contains(&"execute".to_string()));
            }
            other => panic!("Expected Pong, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_shape_code_over_tcp() {
        let addr = start_test_server().await;
        let mut stream = TcpStream::connect(addr).await.unwrap();

        // Send real Shape code to execute
        let msg = WireMessage::Execute(ExecuteRequest {
            code: "fn add(a, b) { a + b }\nadd(10, 32)".to_string(),
            request_id: 1,
        });

        let resp = roundtrip(&mut stream, &msg).await;
        match resp {
            WireMessage::ExecuteResponse(r) => {
                assert_eq!(r.request_id, 1);
                assert!(r.success, "execute failed: {:?}", r.error);
                // Shape infers integer addition for integer literals
                let is_42 = matches!(r.value, WireValue::Integer(42))
                    || matches!(r.value, WireValue::Number(n) if n == 42.0);
                assert!(is_42, "expected 42 in value, got: {:?}", r.value);
                assert!(r.stdout.is_none(), "no print output expected");
            }
            other => panic!("Expected ExecuteResponse, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_error_over_tcp() {
        let addr = start_test_server().await;
        let mut stream = TcpStream::connect(addr).await.unwrap();

        let msg = WireMessage::Execute(ExecuteRequest {
            code: "this is not valid shape code !!!".to_string(),
            request_id: 2,
        });

        let resp = roundtrip(&mut stream, &msg).await;
        match resp {
            WireMessage::ExecuteResponse(r) => {
                assert_eq!(r.request_id, 2);
                assert!(!r.success, "should have failed");
                assert!(r.error.is_some(), "should have error message");
            }
            other => panic!("Expected ExecuteResponse, got {:?}", other),
        }
    }

    /// WF-1D regression (red→green): a `serve --sandbox strict` server MUST
    /// refuse `file::write_text`. Before the security wiring this write
    /// succeeded (`SUCCESS=true`, file on disk); after it, the response is
    /// `success:false` with a permission error naming `fs.write`, and no file
    /// is created. Anchors audit §4.2 site 1 (the reproduced live escape).
    #[tokio::test]
    async fn strict_sandbox_refuses_file_write() {
        let addr = start_test_server_with_sandbox(SandboxLevel::Strict).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();

        // Absolute target in the temp dir so the "no file on disk" assertion is
        // independent of the server process cwd.
        let target = std::env::temp_dir().join(format!(
            "wf1d_strict_escape_{}_{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&target);

        let code = format!(
            "use std::core::file\nfile::write_text(\"{}\", \"escaped under strict\")",
            target.display()
        );
        let msg = WireMessage::Execute(ExecuteRequest {
            code,
            request_id: 7,
        });

        let resp = roundtrip(&mut stream, &msg).await;
        match resp {
            WireMessage::ExecuteResponse(r) => {
                assert_eq!(r.request_id, 7);
                assert!(
                    !r.success,
                    "strict sandbox must refuse file::write_text, got success=true value={:?}",
                    r.value
                );
                let err = r.error.unwrap_or_default();
                assert!(
                    err.to_lowercase().contains("permission")
                        && (err.contains("fs.write") || err.to_lowercase().contains("write")),
                    "error should name the denied fs.write permission, got: {err}"
                );
            }
            other => panic!("Expected ExecuteResponse, got {:?}", other),
        }

        assert!(
            !target.exists(),
            "strict sandbox must NOT have written {} to disk",
            target.display()
        );
        let _ = std::fs::remove_file(&target);
    }

    /// WF-1D: `--sandbox none` is the trusted escape hatch — the same write
    /// that strict refuses succeeds under `none`, proving the gate is the
    /// sandbox level (not a blanket block).
    #[tokio::test]
    async fn none_sandbox_allows_file_write() {
        let addr = start_test_server_with_sandbox(SandboxLevel::None).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();

        let target = std::env::temp_dir().join(format!(
            "wf1d_none_write_{}_{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&target);

        let code = format!(
            "use std::core::file\nfile::write_text(\"{}\", \"allowed under none\")",
            target.display()
        );
        let msg = WireMessage::Execute(ExecuteRequest {
            code,
            request_id: 8,
        });

        let resp = roundtrip(&mut stream, &msg).await;
        match resp {
            WireMessage::ExecuteResponse(r) => {
                assert!(r.success, "none sandbox should allow write: {:?}", r.error);
            }
            other => panic!("Expected ExecuteResponse, got {:?}", other),
        }
        assert!(target.exists(), "none sandbox should have written the file");
        let _ = std::fs::remove_file(&target);
    }

    #[tokio::test]
    async fn test_remote_function_call_over_tcp() {
        let addr = start_test_server().await;
        let mut stream = TcpStream::connect(addr).await.unwrap();

        // Compile a Shape program with a function, then call it remotely
        let bytecode = {
            let program = shape_ast::parser::parse_program("function multiply(a, b) { a * b }")
                .expect("parse");
            let compiler = shape_vm::compiler::BytecodeCompiler::new();
            compiler.compile(&program).expect("compile")
        };

        // Build a remote call request for multiply(6, 7)
        let request = build_call_request(
            &bytecode,
            "multiply",
            vec![
                SerializableVMValue::Number(6.0),
                SerializableVMValue::Number(7.0),
            ],
        );

        let msg = WireMessage::Call(request);
        let resp = roundtrip(&mut stream, &msg).await;

        match resp {
            WireMessage::CallResponse(r) => match r.result {
                Ok(SerializableVMValue::Number(n)) => {
                    assert_eq!(n, 42.0, "6 * 7 should be 42");
                }
                Ok(other) => panic!("Expected Number(42.0), got {:?}", other),
                Err(e) => panic!("Remote call failed: {:?}", e),
            },
            other => panic!("Expected CallResponse, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_auth_required_rejects_unauthenticated() {
        // Start server WITH auth token
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let config = Arc::new(ServeConfig {
            auth_token: Some("secret".to_string()),
            max_concurrent: 4,
            sandbox: SandboxLevel::None,
            security: derive_serve_security(SandboxLevel::None, true),
            _mode: ExecutionModeArg::Vm,
            extensions: vec![],
            provider_opts: ProviderOptions::default(),
        });
        let semaphore = Arc::new(Semaphore::new(4));
        let language_runtimes: Arc<LanguageRuntimes> = Arc::new(HashMap::new());

        tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let config = config.clone();
                let semaphore = semaphore.clone();
                let language_runtimes = language_runtimes.clone();
                tokio::spawn(async move {
                    let _ =
                        handle_connection(socket, &config, &semaphore, &language_runtimes).await;
                });
            }
        });

        let mut stream = TcpStream::connect(addr).await.unwrap();

        // Try to execute without auth → should fail
        let msg = WireMessage::Execute(ExecuteRequest {
            code: "42".to_string(),
            request_id: 1,
        });
        let resp = roundtrip(&mut stream, &msg).await;
        match resp {
            WireMessage::ExecuteResponse(r) => {
                assert!(!r.success);
                assert!(r.error.unwrap().contains("Authentication required"));
            }
            other => panic!("Expected ExecuteResponse, got {:?}", other),
        }

        // Now authenticate
        let auth_msg = WireMessage::Auth(AuthRequest {
            token: "secret".to_string(),
        });
        let resp = roundtrip(&mut stream, &auth_msg).await;
        match resp {
            WireMessage::AuthResponse(r) => assert!(r.authenticated),
            other => panic!("Expected AuthResponse, got {:?}", other),
        }

        // Now execute should work
        let msg = WireMessage::Execute(ExecuteRequest {
            code: "42".to_string(),
            request_id: 2,
        });
        let resp = roundtrip(&mut stream, &msg).await;
        match resp {
            WireMessage::ExecuteResponse(r) => {
                assert!(r.success, "should succeed after auth: {:?}", r.error);
            }
            other => panic!("Expected ExecuteResponse, got {:?}", other),
        }
    }
}
