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
    AuthRequest, AuthResponse, BlobNegotiationRequest, BlobSidecar, ExecuteRequest,
    ExecuteResponse, ExecutionMetrics, ServerInfo, ValidateRequest, ValidateResponse,
    WireDiagnostic, WireMessage,
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
            "strict" => Ok(SandboxLevel::Strict),
            "permissive" => Ok(SandboxLevel::Permissive),
            "none" => Ok(SandboxLevel::None),
            _ => Err(format!(
                "unknown sandbox level: '{}' (expected strict|permissive|none)",
                s
            )),
        }
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

    // Safety: refuse non-localhost without TLS
    if !addr.ip().is_loopback() {
        if tls_cert.is_none() || tls_key.is_none() {
            bail!(
                "Refusing to start on non-localhost address {} without TLS.\n\
                 Provide --tls-cert and --tls-key, or bind to 127.0.0.1.",
                addr
            );
        }
    }

    // Warn if non-localhost without auth token
    if !addr.ip().is_loopback() && auth_token.is_none() {
        eprintln!(
            "Warning: serving on {} without --auth-token. Any client can execute code.",
            addr
        );
    }

    let sandbox_level: SandboxLevel = sandbox.parse().map_err(|e: String| anyhow::anyhow!(e))?;

    let _ = (tls_cert, tls_key); // TLS support is a future enhancement

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

    let config = Arc::new(ServeConfig {
        auth_token,
        max_concurrent,
        sandbox: sandbox_level,
        _mode: mode,
        extensions,
        provider_opts: provider_opts.clone(),
    });

    let semaphore = Arc::new(Semaphore::new(config.max_concurrent));

    let listener = TcpListener::bind(addr).await?;
    eprintln!("Shape serve listening on {}", addr);
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
                            result: Err(shape_vm::remote::RemoteCallError {
                                message: "Authentication required.".to_string(),
                                kind: shape_vm::remote::RemoteErrorKind::RuntimeError,
                            }),
                        },
                    ))
                } else {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|_| anyhow::anyhow!("semaphore closed"))?;
                    Some(handle_call(req, &mut state, language_runtimes))
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
            "validate".to_string(),
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
    let _sandbox = config.sandbox;

    let result = tokio::task::spawn_blocking(move || {
        execute_code_in_process(&code, &extensions, &provider_opts)
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

fn handle_call(
    req: shape_vm::remote::RemoteCallRequest,
    _state: &mut ConnectionState,
    language_runtimes: &LanguageRuntimes,
) -> WireMessage {
    let tmp_dir = std::env::temp_dir().join("shape-serve-snapshots");
    match shape_runtime::snapshot::SnapshotStore::new(&tmp_dir) {
        Ok(store) => {
            let response = if language_runtimes.is_empty() {
                shape_vm::remote::execute_remote_call(req, &store)
            } else {
                shape_vm::remote::execute_remote_call_with_runtimes(req, &store, language_runtimes)
            };
            WireMessage::CallResponse(response)
        }
        Err(e) => WireMessage::CallResponse(shape_vm::remote::RemoteCallResponse {
            result: Err(shape_vm::remote::RemoteCallError {
                message: format!("Failed to create snapshot store: {}", e),
                kind: shape_vm::remote::RemoteErrorKind::RuntimeError,
            }),
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
) -> Result<InProcessResult> {
    use std::time::Instant;

    let start = Instant::now();

    let mut engine =
        ShapeEngine::new().map_err(|e| anyhow::anyhow!("failed to create Shape engine: {}", e))?;

    let mut executor = BytecodeExecutor::new();

    extension_loading::register_extension_capability_modules(&mut engine, &mut executor);
    let module_info = executor.module_schemas();
    engine.register_extension_modules(&module_info);

    let interrupt = Arc::new(AtomicU8::new(0));
    executor.set_interrupt(interrupt);

    crate::module_loading::wire_vm_executor_module_loading(
        &mut engine,
        &mut executor,
        None,
        Some(code),
    )?;

    let result = engine.execute(&mut executor, code)?;

    let wall_time_ms = start.elapsed().as_millis() as u64;

    // Collect print output — NOT the return value
    let stdout: String = result
        .messages
        .iter()
        .map(|m| format!("{}\n", m.text))
        .collect();

    Ok(InProcessResult {
        value: result.value,
        stdout: if stdout.is_empty() {
            None
        } else {
            Some(stdout)
        },
        content_terminal: result.content_terminal,
        content_html: result.content_html,
        wall_time_ms,
    })
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let config = Arc::new(ServeConfig {
            auth_token: None,
            max_concurrent: 4,
            sandbox: SandboxLevel::None,
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
