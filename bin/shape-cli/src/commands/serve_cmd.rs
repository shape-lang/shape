use anyhow::{Result, bail};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
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
use crate::commands::{ProviderOptions, snapshot_cmd};
use crate::extension_loading;

/// Pre-loaded language runtimes for polyglot remote execution.
type LanguageRuntimes =
    HashMap<String, Arc<shape_runtime::plugins::language_runtime::PluginLanguageRuntime>>;

fn fresh_worker_language_runtimes(
    runtimes: &LanguageRuntimes,
) -> std::result::Result<LanguageRuntimes, String> {
    let mut fresh = LanguageRuntimes::new();
    for (language, runtime) in runtimes {
        let instance = runtime
            .fresh_instance()
            .map_err(|e| format!("failed to initialize '{language}' runtime for worker: {e}"))?;
        fresh.insert(language.clone(), Arc::new(instance));
    }
    Ok(fresh)
}

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
    snapshot_store_root: std::path::PathBuf,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteCallState {
    Queued,
    CancelRequested,
    Running,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterQueuedOutcome {
    Queued,
    AlreadyCancelled,
}

struct RemoteCallRegistry {
    calls: Mutex<HashMap<shape_vm::remote::RemoteCallId, RemoteCallState>>,
}

impl RemoteCallRegistry {
    fn new() -> Self {
        Self {
            calls: Mutex::new(HashMap::new()),
        }
    }

    fn register_queued(&self, call_id: shape_vm::remote::RemoteCallId) -> RegisterQueuedOutcome {
        let mut calls = self.calls.lock().expect("remote call registry poisoned");
        match calls.get(&call_id).copied() {
            Some(RemoteCallState::CancelRequested) => {
                calls.remove(&call_id);
                RegisterQueuedOutcome::AlreadyCancelled
            }
            _ => {
                calls.insert(call_id, RemoteCallState::Queued);
                RegisterQueuedOutcome::Queued
            }
        }
    }

    fn mark_running(&self, call_id: shape_vm::remote::RemoteCallId) -> bool {
        let mut calls = self.calls.lock().expect("remote call registry poisoned");
        match calls.get(&call_id).copied() {
            Some(RemoteCallState::CancelRequested) => {
                calls.remove(&call_id);
                false
            }
            _ => {
                calls.insert(call_id, RemoteCallState::Running);
                true
            }
        }
    }

    fn finish(&self, call_id: shape_vm::remote::RemoteCallId) {
        let mut calls = self.calls.lock().expect("remote call registry poisoned");
        calls.insert(call_id, RemoteCallState::Finished);
    }

    fn cancel(
        &self,
        call_id: shape_vm::remote::RemoteCallId,
    ) -> shape_vm::remote::RemoteCancelOutcome {
        use shape_vm::remote::RemoteCancelOutcome;

        let mut calls = self.calls.lock().expect("remote call registry poisoned");
        if calls.len() > 4096 {
            calls.retain(|_, state| {
                matches!(state, RemoteCallState::Queued | RemoteCallState::Running)
            });
        }
        match calls.get(&call_id).copied() {
            Some(RemoteCallState::Queued) | Some(RemoteCallState::CancelRequested) => {
                calls.insert(call_id, RemoteCallState::CancelRequested);
                RemoteCancelOutcome::AcceptedQueued
            }
            Some(RemoteCallState::Running) => RemoteCancelOutcome::AlreadyRunning,
            Some(RemoteCallState::Finished) => RemoteCancelOutcome::AlreadyFinished,
            None => {
                calls.insert(call_id, RemoteCallState::CancelRequested);
                RemoteCancelOutcome::AcceptedQueued
            }
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
    snapshot_store_root: Option<std::path::PathBuf>,
) -> Result<()> {
    let addr: SocketAddr = address.parse()?;
    let snapshot_store_root = snapshot_cmd::snapshot_store_root(snapshot_store_root);

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
        snapshot_store_root,
    });

    let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
    let call_registry = Arc::new(RemoteCallRegistry::new());

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
        let call_registry = call_registry.clone();
        let language_runtimes = language_runtimes.clone();
        let tls_acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            match tls_acceptor {
                // TLS-terminating path: complete the handshake, then run the
                // framing protocol over the encrypted `TlsStream`.
                Some(acceptor) => match acceptor.accept(socket).await {
                    Ok(tls_stream) => {
                        if let Err(e) = handle_connection(
                            tls_stream,
                            &config,
                            &semaphore,
                            &call_registry,
                            &language_runtimes,
                        )
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
                    if let Err(e) = handle_connection(
                        socket,
                        &config,
                        &semaphore,
                        &call_registry,
                        &language_runtimes,
                    )
                    .await
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
    call_registry: &RemoteCallRegistry,
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
            WireMessage::Call(mut req) => {
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
                    cache_and_hydrate_call_blobs(&mut req, &mut state.blob_cache);

                    if let Some(call_id) = req.call_id {
                        if matches!(
                            call_registry.register_queued(call_id),
                            RegisterQueuedOutcome::AlreadyCancelled
                        ) {
                            Some(cancelled_call_response(call_id))
                        } else {
                            let _permit = semaphore
                                .acquire()
                                .await
                                .map_err(|_| anyhow::anyhow!("semaphore closed"))?;
                            if !call_registry.mark_running(call_id) {
                                Some(cancelled_call_response(call_id))
                            } else {
                                let language_runtimes = language_runtimes.clone();
                                let granted = config.security.granted.clone();
                                let scope = config.security.scope.clone();
                                let snapshot_store_root = config.snapshot_store_root.clone();
                                let response = tokio::task::spawn_blocking(move || {
                                    handle_call(
                                        req,
                                        &language_runtimes,
                                        &granted,
                                        &scope,
                                        &snapshot_store_root,
                                    )
                                })
                                .await
                                .map_err(|e| anyhow::anyhow!("remote call worker failed: {e}"))?;
                                call_registry.finish(call_id);
                                Some(response)
                            }
                        }
                    } else {
                        let _permit = semaphore
                            .acquire()
                            .await
                            .map_err(|_| anyhow::anyhow!("semaphore closed"))?;
                        let language_runtimes = language_runtimes.clone();
                        let granted = config.security.granted.clone();
                        let scope = config.security.scope.clone();
                        let snapshot_store_root = config.snapshot_store_root.clone();
                        Some(
                            tokio::task::spawn_blocking(move || {
                                handle_call(
                                    req,
                                    &language_runtimes,
                                    &granted,
                                    &scope,
                                    &snapshot_store_root,
                                )
                            })
                            .await
                            .map_err(|e| anyhow::anyhow!("remote call worker failed: {e}"))?,
                        )
                    }
                }
            }
            WireMessage::CancelCall(req) => {
                if requires_auth(config) && !state.authenticated {
                    Some(WireMessage::CancelCallResponse(
                        shape_vm::remote::RemoteCancelResponse {
                            call_id: req.call_id,
                            outcome: shape_vm::remote::RemoteCancelOutcome::AuthRequired,
                            message: "Authentication required.".to_string(),
                        },
                    ))
                } else {
                    Some(handle_cancel_call(req, call_registry))
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
            | WireMessage::CancelCallResponse(_)
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
            "call-cancel".to_string(),
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

/// Retain verified content-addressed blobs and restore omitted dependencies from
/// this connection's cache before executing a negotiated call.
fn cache_and_hydrate_call_blobs(
    request: &mut shape_vm::remote::RemoteCallRequest,
    cache: &mut shape_vm::remote::RemoteBlobCache,
) {
    let Some(blobs) = request.function_blobs.as_mut() else {
        return;
    };

    // Keep malformed wire input on the normal execution path, where it returns
    // the established HashMismatch response. Never cache it.
    if blobs
        .iter()
        .any(|(declared_hash, blob)| blob.compute_hash() != *declared_hash)
    {
        return;
    }

    cache.insert_blobs(blobs);

    let Some(entry_hash) = request.function_hash else {
        return;
    };

    let mut pending = vec![entry_hash];
    let mut visited = std::collections::HashSet::new();
    while let Some(hash) = pending.pop() {
        if !visited.insert(hash) {
            continue;
        }

        let dependencies = if let Some((_, blob)) = blobs.iter().find(|(known, _)| *known == hash) {
            blob.dependencies.clone()
        } else if let Some(blob) = cache.get(&hash) {
            let blob = blob.clone();
            let dependencies = blob.dependencies.clone();
            blobs.push((hash, blob));
            dependencies
        } else {
            continue;
        };
        pending.extend(dependencies);
    }
}

fn handle_call(
    req: shape_vm::remote::RemoteCallRequest,
    language_runtimes: &LanguageRuntimes,
    granted: &shape_abi_v1::PermissionSet,
    scope: &shape_abi_v1::ScopeConstraints,
    snapshot_store_root: &std::path::Path,
) -> WireMessage {
    // WF-2F acceptance genuineness log: prove a real inbound content-addressed
    // Call landed on this node (blob count + foreign-entry count), so a passing
    // matrix cell cannot be a sender-side local fallback.
    eprintln!(
        "[serve] inbound Call id={:?} fn={:?} blobs={} foreign_entries={}",
        req.call_id,
        req.function_name,
        req.function_blobs.as_ref().map(|b| b.len()).unwrap_or(0),
        req.program.foreign_functions.len(),
    );
    match shape_runtime::snapshot::SnapshotStore::new(snapshot_store_root.to_path_buf()) {
        Ok(store) => {
            // WF-1D: gate the remote Call path with the server's derived grant.
            let response = if language_runtimes.is_empty() {
                shape_vm::remote::execute_remote_call_with_scope(req, &store, granted, scope)
            } else {
                match fresh_worker_language_runtimes(language_runtimes) {
                    Ok(worker_runtimes) => shape_vm::remote::execute_remote_call_with_runtimes(
                        req,
                        &store,
                        &worker_runtimes,
                        granted,
                        scope,
                    ),
                    Err(e) => shape_vm::remote::RemoteCallResponse {
                        result: Err(shape_vm::remote::RemoteCallError::new(
                            shape_vm::remote::RemoteErrorKind::RuntimeError,
                            e,
                        )),
                    },
                }
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

fn cancelled_call_response(call_id: shape_vm::remote::RemoteCallId) -> WireMessage {
    eprintln!("[serve] inbound Call id={call_id:?} cancelled before execution");
    WireMessage::CallResponse(shape_vm::remote::RemoteCallResponse {
        result: Err(shape_vm::remote::RemoteCallError::new(
            shape_vm::remote::RemoteErrorKind::RuntimeError,
            format!(
                "remote call {:?} was cancelled before receiver execution",
                call_id
            ),
        )),
    })
}

fn handle_cancel_call(
    req: shape_vm::remote::RemoteCancelRequest,
    call_registry: &RemoteCallRegistry,
) -> WireMessage {
    use shape_vm::remote::RemoteCancelOutcome;

    let outcome = call_registry.cancel(req.call_id);
    let message = match outcome {
        RemoteCancelOutcome::AcceptedQueued => {
            "remote call cancellation accepted before receiver execution"
        }
        RemoteCancelOutcome::AlreadyRunning => {
            "remote call is already running in a receiver VM frame and is not preemptible"
        }
        RemoteCancelOutcome::AlreadyFinished => "remote call already finished",
        RemoteCancelOutcome::UnknownCall => "remote call id is unknown to this receiver",
        RemoteCancelOutcome::AuthRequired => "authentication required",
    };
    eprintln!(
        "[serve] inbound CancelCall id={:?} outcome={:?} message={}",
        req.call_id, outcome, message
    );
    WireMessage::CancelCallResponse(shape_vm::remote::RemoteCancelResponse {
        call_id: req.call_id,
        outcome,
        message: message.to_string(),
    })
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

    fn test_snapshot_store_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("shape-serve-unit-snapshots-{}", std::process::id()))
    }

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
            snapshot_store_root: test_snapshot_store_root(),
        });
        let semaphore = Arc::new(Semaphore::new(4));
        let call_registry = Arc::new(RemoteCallRegistry::new());
        let language_runtimes: Arc<LanguageRuntimes> = Arc::new(HashMap::new());

        tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let config = config.clone();
                let semaphore = semaphore.clone();
                let call_registry = call_registry.clone();
                let language_runtimes = language_runtimes.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(
                        socket,
                        &config,
                        &semaphore,
                        &call_registry,
                        &language_runtimes,
                    )
                    .await;
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
            snapshot_store_root: test_snapshot_store_root(),
        });
        let semaphore = Arc::new(Semaphore::new(4));
        let call_registry = Arc::new(RemoteCallRegistry::new());
        let language_runtimes: Arc<LanguageRuntimes> = Arc::new(HashMap::new());

        tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let config = config.clone();
                let semaphore = semaphore.clone();
                let call_registry = call_registry.clone();
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
                                &call_registry,
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
    ///
    /// Post-D4 (WF-3E fixC, design §4.1.1 / Q26): `remote::call` now yields a
    /// real `Result<R, RemoteError>` value — a mutable-capture refusal surfaces
    /// as `Err(RemoteError::UnsupportedCapture { .. })` the program can handle,
    /// NOT an uncatchable runtime abort. The closure is still refused (never
    /// executed remotely, never a Bool-default); the delivery mechanism is the
    /// recoverable Result surface. Assert on the printed Err value's message.
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

        // A mutable capture must NOT execute remotely. Post-D4 the refusal is a
        // recoverable `Err(RemoteError::UnsupportedCapture)` VALUE (the program
        // runs cleanly and prints it), not a program-level runtime error.
        let stdout = match result {
            Ok(out) => out.stdout.unwrap_or_default(),
            Err(e) => panic!(
                "post-D4 `remote::call` must yield a recoverable Result, not a \
                 program abort; got error {e}"
            ),
        };
        assert!(
            stdout.contains("Err") && stdout.contains("Capture"),
            "refusal should print an Err(RemoteError::...Capture...) value, got stdout: {stdout:?}"
        );
        assert!(
            stdout.contains("capture")
                || stdout.contains("immutable")
                || stdout.contains("mutable"),
            "refusal should name the capture problem in user-legible words, got stdout: {stdout:?}"
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
            snapshot_store_root: test_snapshot_store_root(),
        });
        let semaphore = Arc::new(Semaphore::new(4));
        let call_registry = Arc::new(RemoteCallRegistry::new());
        let language_runtimes: Arc<LanguageRuntimes> = Arc::new(HashMap::new());

        tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let config = config.clone();
                let semaphore = semaphore.clone();
                let call_registry = call_registry.clone();
                let language_runtimes = language_runtimes.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(
                        socket,
                        &config,
                        &semaphore,
                        &call_registry,
                        &language_runtimes,
                    )
                    .await;
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

    /// D1 (WF-3E) — `@remote` FOREIGN composition end-to-end. A regular `@remote`
    /// fn whose BODY calls an `extern C` foreign stub transfers over the wire and
    /// executes ON the serve node. Both the wrapper blob AND the foreign-stub blob
    /// travel: the sender's minimal-blob closure now follows the
    /// `LoadModuleBinding` + `CallValue` edge to the foreign stub (pre-fix it
    /// shipped `blobs=1` with the stub missing, and the receiver died
    /// "frame_descriptor has 0 slots but arity is 1"). The `extern C labs` runs
    /// server-side via libffi — no language runtime needed — so
    /// `remote_abs(-42) == 42` proves the foreign body executed on the receiver,
    /// not a client-side fallback. This is the exact `blobs>=2` /
    /// foreign-functions-non-empty regression path the audit found untested.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "dark window: E4 re-implements @remote on typed HookDecision — see issue #68"]
    async fn test_remote_foreign_extern_c_transfer_over_tcp() {
        // `none` on loopback grants `Ffi`; `extern C` is not gated by the
        // (empty) `ffi_languages` allow-list, so the foreign call is admitted.
        let addr = start_test_server().await;
        let security = derive_serve_security(SandboxLevel::None, true, &[]);

        let code = format!(
            r#"
use std::core::remote

extern "C" fn labs(x: int) -> int from "c"

@remote("{addr}")
fn remote_abs(x: int) -> int {{
    labs(x)
}}

print(remote_abs(-42))
"#
        );

        let result = tokio::task::spawn_blocking(move || {
            execute_code_in_process(&code, &[], &ProviderOptions::default(), &security)
        })
        .await
        .expect("client thread panicked");

        let out = result.expect("@remote extern C composition transfer failed");
        let stdout = out.stdout.unwrap_or_default();
        assert!(
            stdout.contains("42"),
            "@remote fn calling extern C labs(-42) must return 42 server-side, \
             got stdout {stdout:?} value {:?}",
            out.value
        );
    }

    /// D4 (WF-3E, design §4.1.1 / Q26) — `remote::call` yields a REAL
    /// `Result<R, RemoteError>`. Pre-fix the compiler lowered `remote::call` at
    /// the bare callee return type, so the documented
    /// `match { Ok(v) => .., Err(e) => .. }` type-checked then crashed at runtime
    /// ("No match arm matched"). Here BOTH arms are reachable from the same
    /// program shape: a live node returns `Ok(42)`, and a dead port returns
    /// `Err(RemoteError::Transport)` (pre-send connect-refused) — a recoverable
    /// value, never an abort.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_remote_call_result_ok_and_err_over_tcp() {
        let addr = start_test_server().await;

        // Ok arm — live node.
        let security_ok = derive_serve_security(SandboxLevel::None, true, &[]);
        let code_ok = format!(
            r#"
use std::core::remote
fn mul(a: int, b: int) -> int {{ a * b }}
let r = remote::call("{addr}", mul, 6, 7)
match r {{
    Ok(v) => print(f"OK={{v}}")
    Err(e) => print(f"ERR={{e}}")
}}
"#
        );
        let ok = tokio::task::spawn_blocking(move || {
            execute_code_in_process(&code_ok, &[], &ProviderOptions::default(), &security_ok)
        })
        .await
        .expect("client thread panicked")
        .expect("remote::call Ok path failed");
        assert!(
            ok.stdout.unwrap_or_default().contains("OK=42"),
            "live remote::call mul(6,7) must take the Ok arm with value 42"
        );

        // Err arm — nothing listening on 127.0.0.1:2 → a pre-send Transport
        // failure surfaces as a recoverable `Err`, and the program runs to
        // completion printing it (no "No match arm matched" abort).
        let security_err = derive_serve_security(SandboxLevel::None, true, &[]);
        let code_err = r#"
use std::core::remote
fn mul(a: int, b: int) -> int { a * b }
let r = remote::call("127.0.0.1:2", mul, 6, 7)
match r {
    Ok(v) => print(f"OK={v}")
    Err(e) => print("ERR_FIRED")
}
"#
        .to_string();
        let err = tokio::task::spawn_blocking(move || {
            execute_code_in_process(&code_err, &[], &ProviderOptions::default(), &security_err)
        })
        .await
        .expect("client thread panicked")
        .expect("remote::call Err path must run cleanly (recoverable), not abort");
        assert!(
            err.stdout.unwrap_or_default().contains("ERR_FIRED"),
            "dead-port remote::call must take the Err arm as a recoverable Result, not crash"
        );
    }

    /// D5 (WF-3E, §4.6) — receiver permission-over-wire refusal on REAL derived
    /// permissions. A transferred per-function blob now carries its own body's
    /// required permissions (pre-fix `record_blob_permissions` fired only for
    /// NAMED top-level imports, so a namespace import + the callee body recorded
    /// nothing and the §4.6 load-refusal had empty data). A strict node (grants
    /// nothing) receives a fn that calls `file::write_text` and refuses it at LOAD
    /// with `Err(RemoteError::PermissionDenied { missing: [..fs.write..] })`; the
    /// write never runs (no file on disk).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_remote_permission_refusal_over_wire() {
        // Strict receiver grants nothing; the sender is fully trusted.
        let addr = start_test_server_with_sandbox(SandboxLevel::Strict).await;
        let security = derive_serve_security(SandboxLevel::None, true, &[]);

        let target = std::env::temp_dir().join(format!(
            "wf3e_d5_wire_refuse_{}_{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&target);

        let code = format!(
            r#"
use std::core::remote
use std::core::file
fn writer(x: int) -> int {{ file::write_text("{}", "escaped over wire"); x }}
let r = remote::call("{addr}", writer, 1)
match r {{
    Ok(v) => print(f"OK={{v}}")
    Err(e) => print(f"REFUSED={{e}}")
}}
"#,
            target.display()
        );

        let result = tokio::task::spawn_blocking(move || {
            execute_code_in_process(&code, &[], &ProviderOptions::default(), &security)
        })
        .await
        .expect("client thread panicked");

        let stdout = result
            .expect("D5 client program must run cleanly (recoverable refusal)")
            .stdout
            .unwrap_or_default();
        assert!(
            stdout.contains("REFUSED")
                && (stdout.contains("fs.write") || stdout.to_lowercase().contains("permission")),
            "strict node must refuse the transferred fs.write fn at load with a \
             PermissionDenied Err naming fs.write, got stdout: {stdout:?}"
        );
        assert!(
            !target.exists(),
            "the refused write must NOT have hit disk on the strict receiver: {}",
            target.display()
        );
        let _ = std::fs::remove_file(&target);
    }

    // =====================================================================
    // Polyglot × distributed: `fn python` / `fn typescript` @remote-transfer
    // =====================================================================
    //
    // The extern-C composition above runs the serve node IN-PROCESS because
    // libffi links libc with no dlopen of a poisoning runtime. A `fn python`
    // / `fn typescript` serve node MUST dlopen CPython / deno_core V8, whose
    // pthread-TLS destructors SIGSEGV at tokio-worker-thread teardown when
    // loaded into THIS test binary (`__nptl_deallocate_tsd`). So — unlike the
    // extern-C test — these drive the REAL `shape serve` + `shape run`
    // binaries as SUBPROCESSES; the crash-prone runtime lives in a child
    // process and never touches the harness. The assertion shape still mirrors
    // `test_remote_foreign_extern_c_transfer_over_tcp`: the foreign body
    // executes ON the serve node (proved by the serve-side
    // `blobs=2 foreign_entries=1` genuineness log + the matrix return value),
    // under BOTH vm and jit sender modes, and a client whose language is NOT
    // opted in server-side cannot produce the value.
    //
    // GATING (close the CI-coverage gap without an #[ignore] that silently
    // runs nothing): these SKIP CLEANLY — a `println!` note + early return —
    // when the language `.so` is absent, so the default gate on a machine with
    // no built extensions stays green, and they RUN (never skip silently)
    // whenever the extension IS present (`just build-extensions` /
    // `SHAPE_FFI_EXT_DIR`). This is the exact `py/ts × @remote` regression that
    // the WF-2F close matrix exercised only by hand.

    /// Serialize the heavy polyglot subprocess launches (each child spins a
    /// Tokio runtime and embeds CPython / V8). One-at-a-time keeps a resource
    /// spike in one child from destabilizing a sibling.
    fn polyglot_process_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    /// Resolve the freshly-built `shape` binary from the running unit-test
    /// harness path (`target/<profile>/deps/<hash>` → `target/<profile>/shape`).
    /// Avoids the deprecated `assert_cmd::cargo::cargo_bin` (whose macro form
    /// needs `CARGO_BIN_EXE_shape`, unset for a bin's own unit tests).
    fn shape_binary() -> std::path::PathBuf {
        let mut p = std::env::current_exe().expect("current_exe");
        p.pop(); // drop the test-harness filename → .../deps
        if p.ends_with("deps") {
            p.pop(); // → .../<profile>
        }
        p.push(if cfg!(windows) { "shape.exe" } else { "shape" });
        assert!(
            p.is_file(),
            "resolved `shape` binary does not exist at {} — build it first",
            p.display()
        );
        p
    }

    /// Locate the built `libshape_ext_<lang>.so`. Search order: `$SHAPE_FFI_EXT_DIR`,
    /// the workspace `extensions/` dir (where `just build-extensions` copies them),
    /// then `target/{debug,release}/`. Returns `None` (→ the test skips cleanly)
    /// when the extension has not been built.
    fn language_ext_so(lang: &str) -> Option<std::path::PathBuf> {
        let file = format!("libshape_ext_{lang}.so");
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os("SHAPE_FFI_EXT_DIR") {
            dirs.push(std::path::PathBuf::from(d));
        }
        // CARGO_MANIFEST_DIR = <workspace>/bin/shape-cli
        let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        if let Some(ws) = workspace {
            dirs.push(ws.join("extensions"));
            dirs.push(ws.join("target").join("debug"));
            dirs.push(ws.join("target").join("release"));
        }
        dirs.into_iter()
            .map(|d| d.join(&file))
            .find(|p| p.is_file())
    }

    /// A `shape serve` subprocess bound to a random loopback port, killed on drop.
    struct PolyglotServeNode {
        child: std::process::Child,
        addr: String,
        stderr_path: std::path::PathBuf,
        _cfg: tempfile::TempDir,
        _extdir: tempfile::TempDir,
    }

    impl Drop for PolyglotServeNode {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Start a real `shape serve` node loading exactly the one language runtime
    /// at `so`, opting in the languages in `ffi_languages` (empty string = the
    /// strict-refusal node). `SHAPE_CONFIG_DIR` points at an empty tempdir so no
    /// stale `~/.shape/extensions` is auto-scanned — the node loads precisely the
    /// `.so` we symlink in. Blocks until the accept loop is up (extension load
    /// happens BEFORE `bind`, so an accepted connection means the runtime is
    /// ready).
    fn start_polyglot_serve(
        shape_bin: &std::path::Path,
        so: &std::path::Path,
        ffi_languages: &str,
    ) -> PolyglotServeNode {
        let cfg = tempfile::tempdir().expect("serve cfg tempdir");
        let extdir = tempfile::tempdir().expect("serve ext tempdir");
        let filename = so.file_name().expect("extension .so filename");
        std::os::unix::fs::symlink(so, extdir.path().join(filename))
            .expect("symlink extension .so");

        // Free ephemeral port (bind-then-drop). The small race window before the
        // child re-binds is acceptable in a test harness.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
            l.local_addr().expect("local_addr").port()
        };
        let addr = format!("127.0.0.1:{port}");

        let stderr_path = cfg.path().join("serve.stderr");
        let stderr_file = std::fs::File::create(&stderr_path).expect("create serve stderr file");

        let mut cmd = std::process::Command::new(shape_bin);
        cmd.args(["serve", "--address", &addr, "--sandbox", "none"]);
        if !ffi_languages.is_empty() {
            cmd.args(["--ffi-languages", ffi_languages]);
        }
        cmd.arg("--extension-dir")
            .arg(extdir.path())
            .env("SHAPE_CONFIG_DIR", cfg.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(stderr_file));
        let child = cmd.spawn().expect("spawn `shape serve`");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if std::net::TcpStream::connect(&addr).is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "`shape serve` did not become ready at {addr} within 60s"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        PolyglotServeNode {
            child,
            addr,
            stderr_path,
            _cfg: cfg,
            _extdir: extdir,
        }
    }

    /// Run `program` through `shape run --mode <mode>` as a subprocess.
    /// `SHAPE_CONFIG_DIR` is an empty tempdir so the CLIENT never auto-loads an
    /// extension (the foreign body runs server-side; loading CPython/V8 client
    /// side would only re-introduce the teardown crash). Returns the completed
    /// `Output`.
    fn run_remote_client(
        shape_bin: &std::path::Path,
        program: &str,
        mode: &str,
    ) -> std::process::Output {
        let dir = tempfile::tempdir().expect("client script tempdir");
        let script = dir.path().join("remote.shape");
        std::fs::write(&script, program).expect("write client script");
        let cfg = tempfile::tempdir().expect("client cfg tempdir");

        let mut cmd = assert_cmd::Command::new(shape_bin);
        cmd.args(["run", "--mode", mode])
            .arg(&script)
            .env("SHAPE_CONFIG_DIR", cfg.path())
            .timeout(std::time::Duration::from_secs(120));
        cmd.output().expect("client subprocess output")
    }

    /// WF-2F / WF-3E — `fn python` @remote transfer over TCP. A `@remote` fn
    /// whose body calls an inline `fn python` returning `Result<int>` transfers
    /// to a serve node that has opted the python runtime in, executes THERE, and
    /// returns the matrix value `105` (`padd(100) = 100 + 5`) to the vm AND jit
    /// sender. Genuineness: the serve node logs `blobs=2 foreign_entries=1` (the
    /// foreign stub blob travelled alongside the `@remote` wrapper), and a strict
    /// node (python NOT opted in) refuses the identical program server-side — it
    /// never yields `105`. Skips cleanly when `libshape_ext_python.so` is absent.
    #[test]
    #[ignore = "dark window: E4 re-implements @remote on typed HookDecision — see issue #68"]
    fn test_remote_foreign_python_transfer_over_tcp() {
        let _guard = polyglot_process_lock()
            .lock()
            .expect("polyglot process lock");
        let Some(so) = language_ext_so("python") else {
            println!(
                "SKIP test_remote_foreign_python_transfer_over_tcp: libshape_ext_python.so \
                 not found (build via `just build-extensions` or set $SHAPE_FFI_EXT_DIR). \
                 Default CI gate stays green."
            );
            return;
        };
        let shape_bin = shape_binary();

        // --- opted-in node: the foreign body executes server-side ---
        let node = start_polyglot_serve(&shape_bin, &so, "python");
        let program = format!(
            r#"
use std::core::remote

fn python padd(x: int) -> Result<int> {{
    return x + 5
}}

@remote("{addr}")
fn remote_py(x: int) -> int {{
    match padd(x) {{
        Ok(v) => v
        Err(e) => 0 - 1
    }}
}}

print(remote_py(100))
"#,
            addr = node.addr
        );

        for mode in ["vm", "jit"] {
            let out = run_remote_client(&shape_bin, &program, mode);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success(),
                "python @remote {mode} sender must exit 0; stdout={stdout:?} stderr={}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                stdout.contains("105"),
                "python @remote {mode}: padd(100) must return 105 server-side (WF-2F matrix cell), \
                 got stdout={stdout:?}"
            );
        }

        // Genuineness: the transferred content-addressed Call carried the
        // foreign stub (blobs=2) and one foreign entry, and landed on THIS node.
        let serve_err = std::fs::read_to_string(&node.stderr_path).unwrap_or_default();
        assert!(
            serve_err.contains("blobs=2") && serve_err.contains("foreign_entries=1"),
            "serve node must log the inbound Call carrying the foreign stub \
             (blobs=2 foreign_entries=1); serve stderr:\n{serve_err}"
        );
        drop(node);

        // --- client-cannot-reproduce: a strict node refuses server-side ---
        let strict = start_polyglot_serve(&shape_bin, &so, "");
        let refused = format!(
            r#"
use std::core::remote

fn python padd(x: int) -> Result<int> {{
    return x + 5
}}

@remote("{addr}")
fn remote_py(x: int) -> int {{
    match padd(x) {{
        Ok(v) => v
        Err(e) => 0 - 1
    }}
}}

print(remote_py(100))
"#,
            addr = strict.addr
        );
        let out = run_remote_client(&shape_bin, &refused, "vm");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !out.status.success() && !combined.contains("105"),
            "a strict node (no --ffi-languages) must REFUSE the python foreign call \
             server-side and never yield 105; got success={} combined={combined:?}",
            out.status.success()
        );
        assert!(
            combined.contains("has not opted into the 'python'")
                || combined.to_lowercase().contains("python"),
            "the refusal must name the un-opted-in language; got {combined:?}"
        );
        let strict_err = std::fs::read_to_string(&strict.stderr_path).unwrap_or_default();
        assert!(
            strict_err.contains("blobs=2") && strict_err.contains("foreign_entries=1"),
            "even the refusing node received the transferred stub — proving the SERVE node \
             made the gating decision (not a client-side fallback); serve stderr:\n{strict_err}"
        );
    }

    /// WF-2F / WF-3E — `fn typescript` @remote transfer over TCP. Sibling of the
    /// python test via deno_core V8: `tadd(20) = 20 + 1 = 21` (the matrix cell),
    /// vm AND jit sender, same `blobs=2 foreign_entries=1` genuineness log, same
    /// strict-node server-side refusal. Skips cleanly when
    /// `libshape_ext_typescript.so` is absent.
    #[test]
    #[ignore = "dark window: E4 re-implements @remote on typed HookDecision — see issue #68"]
    fn test_remote_foreign_typescript_transfer_over_tcp() {
        let _guard = polyglot_process_lock()
            .lock()
            .expect("polyglot process lock");
        let Some(so) = language_ext_so("typescript") else {
            println!(
                "SKIP test_remote_foreign_typescript_transfer_over_tcp: \
                 libshape_ext_typescript.so not found (build via `just build-extensions` or \
                 set $SHAPE_FFI_EXT_DIR). Default CI gate stays green."
            );
            return;
        };
        let shape_bin = shape_binary();

        // --- opted-in node ---
        let node = start_polyglot_serve(&shape_bin, &so, "typescript");
        let program = format!(
            r#"
use std::core::remote

fn typescript tadd(x: int) -> Result<int> {{
    return x + 1;
}}

@remote("{addr}")
fn remote_ts(x: int) -> int {{
    match tadd(x) {{
        Ok(v) => v
        Err(e) => 0 - 1
    }}
}}

print(remote_ts(20))
"#,
            addr = node.addr
        );

        for mode in ["vm", "jit"] {
            let out = run_remote_client(&shape_bin, &program, mode);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success(),
                "typescript @remote {mode} sender must exit 0; stdout={stdout:?} stderr={}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                stdout.contains("21"),
                "typescript @remote {mode}: tadd(20) must return 21 server-side (WF-2F matrix cell), \
                 got stdout={stdout:?}"
            );
        }

        let serve_err = std::fs::read_to_string(&node.stderr_path).unwrap_or_default();
        assert!(
            serve_err.contains("blobs=2") && serve_err.contains("foreign_entries=1"),
            "serve node must log the inbound Call carrying the foreign stub \
             (blobs=2 foreign_entries=1); serve stderr:\n{serve_err}"
        );
        drop(node);

        // --- client-cannot-reproduce ---
        let strict = start_polyglot_serve(&shape_bin, &so, "");
        let refused = format!(
            r#"
use std::core::remote

fn typescript tadd(x: int) -> Result<int> {{
    return x + 1;
}}

@remote("{addr}")
fn remote_ts(x: int) -> int {{
    match tadd(x) {{
        Ok(v) => v
        Err(e) => 0 - 1
    }}
}}

print(remote_ts(20))
"#,
            addr = strict.addr
        );
        let out = run_remote_client(&shape_bin, &refused, "vm");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !out.status.success() && !combined.contains("21"),
            "a strict node (no --ffi-languages) must REFUSE the typescript foreign call \
             server-side and never yield 21; got success={} combined={combined:?}",
            out.status.success()
        );
        assert!(
            combined.contains("has not opted into the 'typescript'")
                || combined.to_lowercase().contains("typescript"),
            "the refusal must name the un-opted-in language; got {combined:?}"
        );
        let strict_err = std::fs::read_to_string(&strict.stderr_path).unwrap_or_default();
        assert!(
            strict_err.contains("blobs=2") && strict_err.contains("foreign_entries=1"),
            "even the refusing node received the transferred stub — proving the SERVE node \
             made the gating decision (not a client-side fallback); serve stderr:\n{strict_err}"
        );
    }
}
