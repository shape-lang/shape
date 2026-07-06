use anyhow::{Result, bail};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;

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
fn derive_serve_security(
    level: SandboxLevel,
    is_loopback: bool,
    ffi_languages: &[String],
) -> SecurityPosture {
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
        //
        // WF-2F axis A: `full()` also includes the `Deterministic` marker, but
        // per ffi-rebuild §4.8.3 (Q6) that marker is a mode SELECTOR whose sole
        // effect is refusing foreign code (extern C / fn python / fn typescript)
        // — the local `shape run` path only inserts it on explicit
        // `[sandbox] deterministic = true` (script_cmd.rs:73-79), never as an
        // implication of "total trust". A serve node that grants `Ffi` and then
        // refuses every foreign call in `check_ffi_permission` would be
        // self-contradictory, so `off` drops Deterministic to genuinely RUN
        // foreign-bearing transfers (the ratified `off -> grants ffi.call`
        // posture). Deterministic serve execution remains reachable through a
        // dedicated future knob, not by piling it onto the FFI-granting level.
        SandboxLevel::None => {
            let mut set = PermissionSet::full();
            set.remove(&Permission::Deterministic);
            (set, ResourceLimits::unlimited())
        }
    };

    // Non-loopback binds fail closed to Pure-only until explicitly configured.
    let granted = if is_loopback {
        granted
    } else {
        granted.intersection(&PermissionSet::pure())
    };

    // WF-2F axis C (§4.6 / OQ-6, ratified 2026-07-05): `ffi_languages` is the
    // wire-serve OPT-IN allow-list for dynamic foreign languages. It defaults
    // EMPTY (strict) — even `--sandbox off`, which grants `Ffi` broadly, will
    // refuse a transferred `fn python` / `fn typescript` call unless the
    // operator explicitly opts the language in with `--ffi-languages`. This is
    // the deliberate asymmetry against local `shape run` (unscoped `Ffi`): the
    // caller here is the network. `extern C` is NOT gated by this list (it is
    // governed by `Ffi` + `ffi_libraries`), so a bare `off` still runs
    // transferred `extern C` code. The list is only consulted when `Ffi` is
    // granted at all (strict/permissive omit `Ffi`, so it never applies there).
    let mut scope = ScopeConstraints::none();
    scope.ffi_languages = ffi_languages.to_vec();

    // D6c (WF-3E): opting a language in with `--ffi-languages python` IS the
    // operator declaring the FFI vertical the gate governs, so it must GRANT
    // the `Ffi` permission that the §4.8.3 load-refusal + the phase-1 dynamic
    // dispatch check key on. Without this, a strict node started
    // `--ffi-languages python` still refuses at LOAD ("requires permissions not
    // granted: ffi.call") — the opt-in flag never granting the permission it
    // gates on. `ffi_languages` then correctly SCOPES which languages that
    // `Ffi` grant may execute (`control_flow/mod.rs` phase-1 opt-in check).
    // Gated on loopback to preserve the non-loopback fail-closed posture above.
    let granted = if !ffi_languages.is_empty() && is_loopback {
        let mut g = granted;
        g.insert(Permission::Ffi);
        g
    } else {
        granted
    };

    SecurityPosture {
        granted,
        scope,
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
    ffi_languages: Vec<String>,
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

    // Transport security (distributed §4.7 / Q29 / OQ-4): TLS-on-TCP
    // termination (tokio-rustls) is now ACTIVE in the accept loop. When the
    // operator supplies `--tls-cert`/`--tls-key`, we build a `TlsAcceptor` from
    // a rustls `ServerConfig` and wrap every accepted socket in a TLS session
    // before framing — the server presents its cert and the client verifies it.
    // Presence-driven: the non-loopback gate above guarantees cert+key for
    // non-loopback binds; a loopback bind with an explicit cert also gets TLS
    // (encryption is only ever additive). No cert → plain framing (dev
    // loopback), the honest no-encryption path.
    let tls_acceptor: Option<TlsAcceptor> = match (&tls_cert, &tls_key) {
        (Some(cert), Some(key)) => {
            let acceptor = build_tls_acceptor(cert, key)
                .map_err(|e| anyhow::anyhow!("failed to configure TLS termination: {}", e))?;
            eprintln!(
                "  TLS: termination ACTIVE — presenting cert {} (traffic on {} is encrypted)",
                cert.display(),
                addr
            );
            Some(acceptor)
        }
        (None, None) => None,
        // Half-configured TLS: the non-loopback gate already rejects this for
        // remote binds; on loopback we fail closed rather than silently serve
        // plaintext when the operator clearly intended TLS.
        _ => bail!("TLS is half-configured: pass BOTH --tls-cert and --tls-key, or neither."),
    };

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
    let security = derive_serve_security(sandbox_level, addr.ip().is_loopback(), &ffi_languages);

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
    // WF-2F axis C: surface the foreign-language opt-in posture so an operator
    // can see at a glance why a transferred `fn python` might be refused.
    if config
        .security
        .granted
        .contains(&shape_abi_v1::Permission::Ffi)
    {
        if config.security.scope.ffi_languages.is_empty() {
            eprintln!(
                "  ffi.call granted; ffi_languages: [] (strict — dynamic foreign \
                 refused; extern C allowed). Opt in with --ffi-languages python,typescript"
            );
        } else {
            eprintln!(
                "  ffi.call granted; ffi_languages: [{}] (dynamic foreign opted in)",
                config.security.scope.ffi_languages.join(", ")
            );
        }
    }

    loop {
        let (socket, peer) = listener.accept().await?;
        eprintln!("Connection from {}", peer);

        let config = config.clone();
        let semaphore = semaphore.clone();
        let language_runtimes = language_runtimes.clone();
        let tls_acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            match tls_acceptor {
                // TLS-terminating path: complete the handshake, then run the
                // framing protocol over the encrypted `TlsStream`.
                Some(acceptor) => match acceptor.accept(socket).await {
                    Ok(tls_stream) => {
                        if let Err(e) =
                            handle_connection(tls_stream, &config, &semaphore, &language_runtimes)
                                .await
                        {
                            eprintln!("Connection error from {}: {}", peer, e);
                        }
                    }
                    Err(e) => {
                        // A plaintext client (or a bad cert) fails the handshake
                        // here — the connection is dropped without ever running
                        // the framing protocol. This is the plaintext-rejection
                        // path.
                        eprintln!("TLS handshake failed from {}: {}", peer, e);
                    }
                },
                // Plain path (loopback dev, no cert): unencrypted framing.
                None => {
                    if let Err(e) =
                        handle_connection(socket, &config, &semaphore, &language_runtimes).await
                    {
                        eprintln!("Connection error from {}: {}", peer, e);
                    }
                }
            }
        });
    }
}

/// Build a `tokio_rustls::TlsAcceptor` from the operator's PEM cert + key
/// (WF-2C-fu R4). Uses the already-linked rustls `ring` provider explicitly so
/// the config never depends on a process-global default provider being
/// installed. Not a new TLS stack — rustls 0.23 / tokio-rustls 0.26 are already
/// in the workspace lockfile.
fn build_tls_acceptor(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<TlsAcceptor> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let cert_bytes = std::fs::read(cert_path)
        .map_err(|e| anyhow::anyhow!("read TLS cert '{}': {}", cert_path.display(), e))?;
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &cert_bytes[..])
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| anyhow::anyhow!("parse TLS cert '{}': {}", cert_path.display(), e))?;
    if certs.is_empty() {
        bail!(
            "TLS cert '{}' contained no certificates",
            cert_path.display()
        );
    }

    let key_bytes = std::fs::read(key_path)
        .map_err(|e| anyhow::anyhow!("read TLS key '{}': {}", key_path.display(), e))?;
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut &key_bytes[..])
        .map_err(|e| anyhow::anyhow!("parse TLS key '{}': {}", key_path.display(), e))?
        .ok_or_else(|| {
            anyhow::anyhow!("TLS key '{}' contained no private key", key_path.display())
        })?;

    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| anyhow::anyhow!("TLS protocol versions: {}", e))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| anyhow::anyhow!("TLS server config: {}", e))?;

    Ok(TlsAcceptor::from(std::sync::Arc::new(config)))
}

/// Handle one client connection's framing protocol. Generic over the stream so
/// it runs unchanged over a plain `tokio::net::TcpStream` (loopback dev) OR a
/// `tokio_rustls::server::TlsStream<TcpStream>` (TLS-terminated) — it only uses
/// the `AsyncRead`/`AsyncWrite` Ext-trait methods (read_exact / write_all /
/// flush), which both concrete stream types satisfy (WF-2C-fu R4).
async fn handle_connection<S>(
    mut socket: S,
    config: &ServeConfig,
    semaphore: &Semaphore,
    language_runtimes: &LanguageRuntimes,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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
                        &config.security.scope,
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
    scope: &shape_abi_v1::ScopeConstraints,
) -> WireMessage {
    // WF-2F acceptance genuineness log: prove a real inbound content-addressed
    // Call landed on this node (blob count + foreign-entry count), so a passing
    // matrix cell cannot be a sender-side local fallback.
    eprintln!(
        "[serve] inbound Call fn={:?} blobs={} foreign_entries={}",
        req.function_name,
        req.function_blobs.as_ref().map(|b| b.len()).unwrap_or(0),
        req.program.foreign_functions.len(),
    );
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
                    scope,
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
    extensions: &[std::path::PathBuf],
    provider_opts: &ProviderOptions,
    security: &SecurityPosture,
) -> Result<InProcessResult> {
    use shape_runtime::output_adapter::SharedCaptureAdapter;
    use std::time::Instant;

    let start = Instant::now();

    let mut engine =
        ShapeEngine::new().map_err(|e| anyhow::anyhow!("failed to create Shape engine: {}", e))?;

    // D6b (WF-3E): the Execute path must load the serve node's extensions so a
    // `fn python` / `fn typescript` code string resolves its language runtime.
    // Previously `_extensions` was ignored and a fresh engine's
    // `register_language_runtime_artifacts()` yielded no runtimes, so foreign
    // code failed "no extension provides language 'python'". Mirror the serve
    // startup + local-run load path (script_cmd.rs) so the engine that
    // `engine.execute` drives actually carries the runtimes.
    let startup_specs =
        extension_loading::collect_startup_specs(provider_opts, None, None, None, extensions);
    let _ = extension_loading::load_specs(&mut engine, &startup_specs, |_, _| {}, |_, _| {});

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
    extensions: &[std::path::PathBuf],
    provider_opts: &ProviderOptions,
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

    // D6b (WF-3E): load the serve node's extensions so a foreign fn resolves its
    // language runtime on the Execute-file path (mirror execute_code_in_process).
    let startup_specs = extension_loading::collect_startup_specs(
        provider_opts,
        None,
        None,
        Some(file_path),
        extensions,
    );
    let _ = extension_loading::load_specs(&mut engine, &startup_specs, |_, _| {}, |_, _| {});

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
            security: derive_serve_security(level, true, &[]),
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
        roundtrip_stream(stream, msg).await
    }

    /// Generic roundtrip over any async stream — used for both the plain
    /// `TcpStream` tests and the TLS client stream (WF-2C-fu R4).
    async fn roundtrip_stream<S>(stream: &mut S, msg: &WireMessage) -> WireMessage
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
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

    /// Generate a throwaway self-signed cert for `localhost`, write it to a
    /// tempdir, and start a real TLS-terminating serve node on a random loopback
    /// port. Returns the bound address, the server cert DER (so a client can
    /// trust it), and the `TempDir` guard (kept alive by the caller so the PEM
    /// files survive for `build_tls_acceptor`). No secrets are committed — the
    /// cert + key live only in the process tempdir for the test's lifetime.
    async fn start_tls_test_server() -> (
        SocketAddr,
        rustls::pki_types::CertificateDer<'static>,
        tempfile::TempDir,
    ) {
        // 1. Throwaway self-signed cert + key (SAN=localhost).
        let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der = generated.cert.der().clone();

        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, generated.cert.pem()).unwrap();
        std::fs::write(&key_path, generated.key_pair.serialize_pem()).unwrap();

        // 2. Build the acceptor via the PRODUCTION PEM path.
        let acceptor = build_tls_acceptor(&cert_path, &key_path).expect("build TLS acceptor");

        // 3. Bind + run the TLS-terminating accept loop.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let config = Arc::new(ServeConfig {
            auth_token: None,
            max_concurrent: 4,
            sandbox: SandboxLevel::None,
            security: derive_serve_security(SandboxLevel::None, true, &[]),
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
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    // Terminate TLS, then run the framing protocol over the
                    // encrypted stream. A plaintext client fails the handshake
                    // here and the connection is dropped (rejection path).
                    match acceptor.accept(socket).await {
                        Ok(tls_stream) => {
                            let _ = handle_connection(
                                tls_stream,
                                &config,
                                &semaphore,
                                &language_runtimes,
                            )
                            .await;
                        }
                        Err(_) => { /* handshake failed — drop */ }
                    }
                });
            }
        });

        (addr, cert_der, dir)
    }

    /// Build a tokio-rustls client that trusts exactly the given server cert.
    fn tls_client_connector(
        cert_der: rustls::pki_types::CertificateDer<'static>,
    ) -> tokio_rustls::TlsConnector {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert_der).unwrap();
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
    }

    #[test]
    fn derive_serve_security_maps_levels_and_bind_class() {
        use shape_abi_v1::Permission;

        // Strict on loopback → grant nothing, sandboxed caps.
        let strict = derive_serve_security(SandboxLevel::Strict, true, &[]);
        assert!(strict.granted.is_empty(), "strict must grant nothing");
        assert!(!strict.granted.contains(&Permission::FsWrite));
        assert!(strict.limits.max_instructions.is_some());

        // Permissive on loopback → read/env/time/random/connect, but no write.
        let perm = derive_serve_security(SandboxLevel::Permissive, true, &[]);
        assert!(perm.granted.contains(&Permission::FsRead));
        assert!(perm.granted.contains(&Permission::NetConnect));
        assert!(
            !perm.granted.contains(&Permission::FsWrite),
            "moderate must not grant fs.write — the escape stays refused"
        );
        assert!(!perm.granted.contains(&Permission::Process));

        // None on loopback → full grant, unlimited.
        let none = derive_serve_security(SandboxLevel::None, true, &[]);
        assert!(none.granted.contains(&Permission::FsWrite));
        assert!(none.limits.max_instructions.is_none());
        // WF-2F axis A: `off` grants Ffi and must genuinely RUN foreign code —
        // so it must NOT carry the Deterministic mode-selector, which
        // `check_ffi_permission` treats as a blanket foreign-call refusal.
        assert!(
            none.granted.contains(&Permission::Ffi),
            "off must grant ffi.call"
        );
        assert!(
            !none.granted.contains(&Permission::Deterministic),
            "off must NOT imply deterministic mode — it would refuse every foreign call"
        );

        // Non-loopback → Pure-only regardless of level (fail closed).
        let remote_none = derive_serve_security(SandboxLevel::None, false, &[]);
        assert!(
            remote_none.granted.is_empty(),
            "non-loopback must clamp to pure until configured"
        );
    }

    #[test]
    fn derive_serve_security_ffi_languages_strict_opt_in() {
        // WF-2F axis C (§4.6 / OQ-6): `off` with no --ffi-languages defaults to
        // the STRICT-EMPTY posture — Ffi is granted (for extern C) but the
        // dynamic-language allow-list is empty, so a transferred fn python /
        // fn typescript is refused at the receiver until explicitly opted in.
        let none_default = derive_serve_security(SandboxLevel::None, true, &[]);
        assert!(
            none_default.scope.ffi_languages.is_empty(),
            "off must default to an EMPTY ffi_languages allow-list (strict opt-in)"
        );

        // Explicit opt-in populates the allow-list verbatim.
        let opted = derive_serve_security(
            SandboxLevel::None,
            true,
            &["python".to_string(), "typescript".to_string()],
        );
        assert_eq!(
            opted.scope.ffi_languages,
            vec!["python".to_string(), "typescript".to_string()],
            "--ffi-languages must populate the receiver's opt-in allow-list"
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

    /// Distributed design T8 — closure-over-wire USER end-to-end (WF-2C-fu R2).
    ///
    /// Drives a real user program through `remote::call` on a **capturing**
    /// closure: the whole pipeline — the `remote::call` compiler elaboration
    /// (`function_calls.rs::compile_remote_call_elaboration`, closure-value arm),
    /// the sender closure arm (`remote_builtins.rs::call_remote` →
    /// `extract_closure_captures` → `build_closure_call_request`), the wire
    /// round-trip over loopback TCP, and the receiver closure path
    /// (`remote.rs::validate_remote_closure_captures` + `finish_remote_closure_call`
    /// materializing the capture at its proven `NativeKind`) — executes on the
    /// in-process `shape serve` node. The captured `base = 100` crosses the wire
    /// via the §2.7.8 per-capture kind track and is added to the argument `5` on
    /// the receiver: `add_base(5) == 105`.
    ///
    /// The client program runs on `spawn_blocking` (the remote transport is
    /// synchronous) so the multi-thread runtime keeps servicing the server tasks.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_remote_capturing_closure_over_tcp() {
        let addr = start_test_server().await;
        // Client grants: loopback + None → full permission set (incl. NetConnect)
        // and unlimited limits — the SENDER side of the call.
        let security = derive_serve_security(SandboxLevel::None, true, &[]);

        // `base` and the argument are `number` (Float64): an UNANNOTATED closure
        // param `|x| x + base` is inferred `number` in the blob's
        // `frame_descriptor`, and the strict wire marshal checks the argument
        // against that proven param kind. Keeping the capture kind, the param
        // kind, and the argument kind all `number` exercises the transfer itself
        // rather than the separate unannotated-int-param inference quirk
        // (single-param `|x: int|` does not parse today — an orthogonal parser
        // limitation).
        let code = format!(
            r#"
use std::core::remote

let base = 100.0
let add_base = |x| x + base
let r = remote::call("{addr}", add_base, 5.0)
print(r)
"#
        );

        let result = tokio::task::spawn_blocking(move || {
            execute_code_in_process(&code, &[], &ProviderOptions::default(), &security)
        })
        .await
        .expect("client thread panicked");

        let out = result.expect("capturing-closure remote call failed");
        let stdout = out.stdout.unwrap_or_default();
        assert!(
            stdout.contains("105"),
            "capturing closure add_base(5) with base=100 should return 105, got stdout {stdout:?} value {:?}",
            out.value
        );
    }

    /// Distributed design T8 — a NON-capturing closure transfers + executes over
    /// the wire (the capture-count-0 arm of the same closure path). `inc(41)`
    /// runs on the remote node and returns `42`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_remote_noncapturing_closure_over_tcp() {
        let addr = start_test_server().await;
        let security = derive_serve_security(SandboxLevel::None, true, &[]);

        let code = format!(
            r#"
use std::core::remote

let inc = |x| x + 1
let r = remote::call("{addr}", inc, 41)
print(r)
"#
        );

        let result = tokio::task::spawn_blocking(move || {
            execute_code_in_process(&code, &[], &ProviderOptions::default(), &security)
        })
        .await
        .expect("client thread panicked");

        let out = result.expect("non-capturing-closure remote call failed");
        let stdout = out.stdout.unwrap_or_default();
        assert!(
            stdout.contains("42"),
            "non-capturing closure inc(41) should return 42, got stdout {stdout:?} value {:?}",
            out.value
        );
    }

    /// Distributed design T8b — a MUTABLE capture is refused with a clean,
    /// user-legible message (the §4.4 refusal matrix), not a Bool-default
    /// execution. `counter` is captured `let mut` and reassigned in the body, so
    /// the closure-over-wire path must reject it rather than ship a mutable cell.
    /// The whole request drives through the real `remote::call` elaboration and
    /// wire round-trip; only the outcome is a structured refusal.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_remote_mutable_capture_refused_over_tcp() {
        let addr = start_test_server().await;
        let security = derive_serve_security(SandboxLevel::None, true, &[]);

        let code = format!(
            r#"
use std::core::remote

let mut counter = 0.0
let bump = |x| {{ counter = counter + x; counter }}
let r = remote::call("{addr}", bump, 5.0)
print(r)
"#
        );

        let result = tokio::task::spawn_blocking(move || {
            execute_code_in_process(&code, &[], &ProviderOptions::default(), &security)
        })
        .await
        .expect("client thread panicked");

        // A mutable capture must NOT execute remotely — the call surfaces a
        // clean refusal (the `_raising` sibling maps it to a runtime error).
        // `InProcessResult` is not `Debug`, so match rather than `expect_err`.
        let msg = match result {
            Ok(out) => panic!(
                "mutable-capture closure must be refused, not executed; got stdout {:?}",
                out.stdout
            ),
            Err(e) => format!("{e}"),
        };
        assert!(
            msg.contains("capture") || msg.contains("immutable") || msg.contains("mutable"),
            "refusal should name the capture problem in user-legible words, got: {msg}"
        );
    }

    /// WF-2C-fu R4 — active TLS termination end-to-end. A serve node presents a
    /// throwaway self-signed cert; a tokio-rustls client verifies it, completes
    /// the TLS 1.3 handshake, and a foreign-free remote function call
    /// (`multiply(6, 7) == 42`) succeeds over the ENCRYPTED channel. The
    /// assertions prove (a) the handshake happened (client observed the server's
    /// peer cert + a negotiated TLS protocol version) and (b) the wire transfer
    /// carrying the real `WireMessage::Call` rode that encrypted session.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_tls_termination_encrypts_remote_call() {
        use tokio_rustls::rustls::pki_types::ServerName;

        let (addr, cert_der, _dir) = start_tls_test_server().await;
        let connector = tls_client_connector(cert_der);

        let tcp = TcpStream::connect(addr).await.unwrap();
        let server_name = ServerName::try_from("localhost").unwrap();
        let mut tls = connector
            .connect(server_name, tcp)
            .await
            .expect("TLS handshake");

        // Prove the transport is actually TLS: the client saw the server's cert
        // and negotiated a concrete TLS protocol version.
        {
            let (_io, session) = tls.get_ref();
            let peer_certs = session.peer_certificates();
            assert!(
                peer_certs.map(|c| !c.is_empty()).unwrap_or(false),
                "client must have received the server's TLS certificate"
            );
            assert!(
                session.protocol_version().is_some(),
                "a TLS protocol version must have been negotiated"
            );
        }

        // A real remote function call over the encrypted channel.
        let bytecode = {
            let program = shape_ast::parser::parse_program("function multiply(a, b) { a * b }")
                .expect("parse");
            let compiler = shape_vm::compiler::BytecodeCompiler::new();
            compiler.compile(&program).expect("compile")
        };
        let request = build_call_request(
            &bytecode,
            "multiply",
            vec![
                SerializableVMValue::Number(6.0),
                SerializableVMValue::Number(7.0),
            ],
        );

        let resp = roundtrip_stream(&mut tls, &WireMessage::Call(request)).await;
        match resp {
            WireMessage::CallResponse(r) => match r.result {
                Ok(SerializableVMValue::Number(n)) => {
                    assert_eq!(n, 42.0, "6 * 7 over TLS should be 42");
                }
                Ok(other) => panic!("Expected Number(42.0) over TLS, got {:?}", other),
                Err(e) => panic!("Remote call over TLS failed: {:?}", e),
            },
            other => panic!("Expected CallResponse over TLS, got {:?}", other),
        }
    }

    /// WF-2C-fu R4 — a PLAINTEXT client talking to a TLS-terminating node is
    /// rejected. The server's `acceptor.accept()` fails the handshake on the
    /// non-TLS bytes and drops the connection, so the plaintext client never
    /// receives a valid framed response (its read hits EOF / connection reset).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_plaintext_client_rejected_by_tls_node() {
        let (addr, _cert_der, _dir) = start_tls_test_server().await;

        // Connect in PLAINTEXT and send a normally-valid framed Ping.
        let mut plain = TcpStream::connect(addr).await.unwrap();
        let msg = WireMessage::Ping(shape_vm::remote::PingRequest {});
        let mp = shape_wire::encode_message(&msg).unwrap();
        let framed = encode_framed(&mp);
        let len = framed.len() as u32;
        // The write may succeed (bytes buffered) — the rejection surfaces on read.
        let _ = plain.write_all(&len.to_be_bytes()).await;
        let _ = plain.write_all(&framed).await;
        let _ = plain.flush().await;

        // The TLS server interprets our plaintext as a (malformed) TLS record,
        // sends a fatal TLS alert, and drops the socket. We must NOT get a valid
        // length-prefixed wire response back. Two shapes both count as rejection:
        //   (a) the read errors (connection reset / EOF), or
        //   (b) the bytes we DO read are a TLS alert record (content-type 0x15),
        //       whose big-endian interpretation as a wire-frame length is absurd
        //       (a real framed Pong is only tens of bytes; this is > 64 MiB).
        let mut len_buf = [0u8; 4];
        match plain.read_exact(&mut len_buf).await {
            Err(_) => { /* rejected via connection drop — good */ }
            Ok(_) => {
                let framed_len = u32::from_be_bytes(len_buf);
                assert!(
                    len_buf[0] == 0x15 || framed_len > 64 * 1024 * 1024,
                    "plaintext client must NOT receive a valid framed wire response from a TLS \
                     node (handshake rejection); got bytes {:?} (len={})",
                    len_buf,
                    framed_len
                );
            }
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
            security: derive_serve_security(SandboxLevel::None, true, &[]),
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
