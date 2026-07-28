//! Per-function remote execution support
//!
//! This module provides the types and executor for transferring function
//! execution to another machine. The design sends the full compiled
//! `BytecodeProgram` + a "call this function with these args" message,
//! running it on a full Shape VM on the remote side.
//!
//! # Architecture
//!
//! ```text
//! Layer 4: @remote / @distributed annotations    (Shape stdlib — user-defined policy)
//! Layer 3: RemoteCallRequest/Response            (this module)
//! Layer 2: shape-wire codec (MessagePack)        (encode_message / decode_message)
//! Layer 1: Transport (TCP/QUIC/Unix socket)      (user-provided, pluggable)
//! ```
//!
//! Layer 0 (the foundation): Full Shape VM on both sides, same `BytecodeProgram`,
//! same `Executor`.
//!
//! # Closure semantics
//!
//! Upvalues (SharedCell-backed shared captures or Phase D typed-pointer
//! captures alike) become **value copies** on serialization. If the remote
//! side mutates a captured variable, the sender doesn't see it. This is the
//! correct semantic for distributed computing — a **send-copy** model.

use serde::{Deserialize, Serialize};
use shape_runtime::snapshot::{SerializableVMValue, SnapshotStore};
use shape_runtime::type_schema::TypeSchemaRegistry;

use shape_wire::WireValue;

use crate::bytecode::{BytecodeProgram, FunctionBlob, FunctionHash, Program};

// `execute_inner` / `execute_inner_with_runtimes` previously called
// `VirtualMachine::new` / `load_program` / `populate_module_objects` /
// `execute_*` / and round-tripped each argument and return value through
// `serializable_to_nanboxed_with_layouts` / `nanboxed_to_serializable`.
// Both round-trip helpers are deleted (see `crates/shape-runtime/src/snapshot.rs:649`
// "The slot-(de)serialization functions ... were deleted in Phase 2b") and
// their replacement is a kind-threaded `slot_to_serializable(bits, kind, store)`
// pair scheduled for the Phase-2c snapshot rebuild (ADR-006 §2.7.4). The
// execute paths are stubbed at the entry below; the rebuild lands the
// kind-threaded serializer + a `Vec<KindedSlot>` arg pipeline together.
//
// Imports `VMConfig` / `VirtualMachine` are pulled in lazily inside
// `execute_remote_call*` so the file still compiles when those are the
// only consumers and the `execute_inner*` bodies are stubbed.

/// Request to execute a function on a remote VM.
///
/// Contains everything needed to call a function: the full compiled program
/// (cacheable by `program_hash`), function identity, serialized arguments,
/// and optional closure captures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCallRequest {
    /// Sender-assigned identity for best-effort cancellation of this call.
    ///
    /// This is not a durable remote future handle and is not exposed to Shape
    /// code. It only lets a caller-side cancelled `remote::call_async` ask the
    /// receiver to drop work that has not entered an executing VM frame yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<RemoteCallId>,

    /// The full compiled program. After the first transfer, the remote
    /// side caches by `program_hash` and subsequent calls only need args.
    pub program: BytecodeProgram,

    /// Function to call by name (for named functions).
    pub function_name: String,

    /// Function to call by ID (for closures that have no user-facing name).
    /// Takes precedence over `function_name` when `Some`.
    pub function_id: Option<u16>,

    /// Function to call by content hash (canonical identity).
    ///
    /// Preferred over name-based lookup when present. This avoids ambiguity
    /// when multiple modules define functions with the same name.
    #[serde(default)]
    pub function_hash: Option<FunctionHash>,

    /// Serialized arguments to the function.
    pub arguments: Vec<SerializableVMValue>,

    /// Closure upvalues, if calling a closure. These are value-copied from
    /// the sender's upvalue slots regardless of the local storage class
    /// (SharedCell, typed frame-pointer capture, or inline scalar).
    pub upvalues: Option<Vec<SerializableVMValue>>,

    /// Per-capture `NativeKind` track, lockstep (index-aligned, equal length)
    /// with `upvalues` — the ADR-006 §2.7.7/§2.7.8 parallel-vec shape carried
    /// across the wire (distributed §4.4). REQUIRED when `upvalues` is `Some`:
    /// the receiver materializes each capture at its proven kind and cross-
    /// checks against the callee blob's hash-covered `capture_kinds`. A closure
    /// request with `upvalues: Some` but `upvalue_kinds: None` is a structured
    /// `ArgumentError` — never a Bool-default, never a kind fabricated from raw
    /// bits (CLAUDE.md §Forbidden Patterns).
    #[serde(default)]
    pub upvalue_kinds: Option<Vec<shape_value::NativeKind>>,

    /// Type schema registry — sent separately because `BytecodeProgram`
    /// has `#[serde(skip)]` on its registry (it's populated at compile time).
    pub type_schemas: TypeSchemaRegistry,

    /// Content hash of the program for caching. If the remote side has
    /// already seen this hash, it can skip deserializing the program.
    pub program_hash: [u8; 32],

    /// Minimal content-addressed blobs for the called function and its
    /// transitive dependencies. When present, the callee can reconstruct
    /// a `Program` from these blobs instead of deserializing the full
    /// `BytecodeProgram`, dramatically reducing payload size.
    #[serde(default)]
    pub function_blobs: Option<Vec<(FunctionHash, FunctionBlob)>>,
}

/// Response from a remote function execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCallResponse {
    /// The function's return value, or an error message.
    pub result: Result<SerializableVMValue, RemoteCallError>,
}

/// Internal identity for one `remote::call_async` wire call.
///
/// It is intentionally just a transport correlation token. It does not grant a
/// polling API, does not survive snapshots, and does not identify a remotely
/// awaitable future.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RemoteCallId {
    pub high: u64,
    pub low: u64,
}

/// Best-effort request to cancel a previously sent remote call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCancelRequest {
    pub call_id: RemoteCallId,
}

/// Receiver's honest cancellation outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteCancelOutcome {
    /// The call was queued or had not arrived yet; the receiver will not run it.
    AcceptedQueued,
    /// The call is already executing inside a receiver VM frame.
    AlreadyRunning,
    /// The receiver has already completed or retired the call.
    AlreadyFinished,
    /// The receiver has no state for this call id.
    UnknownCall,
    /// The cancel request was refused by the same auth gate as calls.
    AuthRequired,
}

/// Response to a best-effort remote cancellation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCancelResponse {
    pub call_id: RemoteCallId,
    pub outcome: RemoteCancelOutcome,
    pub message: String,
}

/// Error from remote execution (the wire-level error carried in
/// `RemoteCallResponse`). This is the RECEIVER→SENDER structured signal; the
/// sender's `remote::call` maps it onto the user-facing Shape `RemoteError`
/// enum (see `stdlib-src/core/remote.shape`) per the normative §4.9 table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCallError {
    /// Human-readable error message.
    pub message: String,
    /// Optional error kind for programmatic handling.
    pub kind: RemoteErrorKind,
    /// Content hashes the receiver still needs to link the entry function.
    /// Populated when `kind == MissingModuleFunction` (distributed §4.2 /
    /// §4.3-4) so the sender can resupply all missing blobs in one round-trip.
    #[serde(default)]
    pub missing_blobs: Option<Vec<FunctionHash>>,
}

/// Classification of remote execution errors.
///
/// The first four variants are the pre-existing wire kinds (kept in place so
/// `call_format == 0` senders keep decoding). The remainder are additive
/// (distributed §4.2): under named msgpack encoding, variant order is
/// non-semantic, so appending is wire-compatible for existing kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteErrorKind {
    /// Function not found in the program.
    FunctionNotFound,
    /// Call-ABI mismatch class: arity, argument-kind, AND return-kind
    /// cross-check failures — not only "bad argument" (distributed §4.2).
    ArgumentError,
    /// Runtime error during execution.
    RuntimeError,
    /// Module function (dependency blob) required on the remote side is
    /// missing. NOW CONSTRUCTED (distributed §4.3-4). The `missing_blobs`
    /// field carries the hashes the sender must resupply.
    MissingModuleFunction,
    /// Receiver refused the linked program's `required_permissions` union
    /// against its own granted set (distributed §4.6). Message carries the
    /// missing permission names.
    PermissionDenied,
    /// A received blob's recomputed content hash did not match its claimed
    /// key — tampered or corrupt (distributed §4.3-2). Blob NOT cached.
    HashMismatch,
    /// Wire / call_format / protocol mismatch (distributed §4.2).
    VersionSkew,
    /// A closure capture was refused (distributed §4.4).
    UnsupportedCapture,
    /// Auth token missing or rejected (distributed §4.7).
    AuthRequired,
    /// Remote execution hit `ResourceLimits` (incl. wall-time overrun).
    ResourceLimitExceeded,
    /// Reserved — not produced in v1. Receiver execution-deadline overrun is
    /// `ResourceLimitExceeded`; the sender-side read timeout is sender-LOCAL
    /// and never crosses the wire (distributed §4.9).
    Timeout,
    /// Sender-LOCAL transport failure (connect refused / DNS / send / receive)
    /// observed before a structured `CallResponse` was decoded. Never crosses
    /// the wire — synthesized sender-side so the recoverable `remote::call`
    /// surface can map it onto `RemoteError::Transport` (pre-send, retry-safe)
    /// distinctly from a callee's own `RuntimeError` (which maps to
    /// `RemoteError::Remote`). Appended last: named-msgpack variant order is
    /// non-semantic, so this is wire-compatible for existing kinds.
    Transport,
}

impl RemoteCallError {
    /// Construct an error with no `missing_blobs` payload (the common case).
    pub fn new(kind: RemoteErrorKind, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind,
            missing_blobs: None,
        }
    }

    /// Construct a `MissingModuleFunction` error carrying the hashes the
    /// sender must resupply (distributed §4.3-4).
    pub fn missing_module_function(
        message: impl Into<String>,
        missing: Vec<FunctionHash>,
    ) -> Self {
        Self {
            message: message.into(),
            kind: RemoteErrorKind::MissingModuleFunction,
            missing_blobs: Some(missing),
        }
    }
}

impl std::fmt::Display for RemoteCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for RemoteCallError {}

/// The pre-send / post-send split for a sender-local transport failure
/// (distributed §4.9). This is the load-bearing distinction that userland
/// retry / idempotency annotations branch on: a call that failed *before* its
/// request frame was fully written provably did not execute and is retry-safe;
/// a failure *after* the frame went out may have executed and must never be
/// auto-retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPhase {
    /// Pre-send: connect / DNS / send-frame / payload-cap failure. The request
    /// was never fully sent, so the call did NOT execute → Shape `Transport`.
    DidNotExecute,
    /// Post-send: the reply was lost or timed out after the full request frame
    /// was written. The call MAY have executed → Shape `ConnectionLost` /
    /// `Timeout`. Never auto-retried.
    MayHaveExecuted,
}

/// Classify a sender-local `TransportError` into the pre-send / post-send
/// phase (distributed §4.9). Framing makes the boundary crisp: by the time the
/// transport is reading a reply, the request frame was fully flushed, so any
/// read-side failure is post-send.
pub fn transport_send_phase(err: &shape_wire::transport::TransportError) -> SendPhase {
    use shape_wire::transport::TransportError as TE;
    match err {
        // Connect / write-side / cap failures: the frame never fully went out.
        TE::ConnectionFailed(_) | TE::SendFailed(_) | TE::PayloadTooLarge { .. } => {
            SendPhase::DidNotExecute
        }
        // Read-side / timeout / reset: the frame was already sent.
        TE::Timeout | TE::ReceiveFailed(_) | TE::ConnectionClosed => SendPhase::MayHaveExecuted,
        // Ambiguous low-level io: assume the unsafe side (may have executed).
        TE::Io(_) => SendPhase::MayHaveExecuted,
    }
}

/// Map a sender-local `TransportError` to the user-facing Shape `RemoteError`
/// variant name it becomes when surfaced by `remote::call` (distributed §4.9).
/// Sender-local transport failures never cross the wire; this is the sender's
/// own classification. Kept as a pure, testable function so the sender path
/// (and its future `remote::call` wiring) share one vocabulary.
pub fn transport_error_shape_variant(err: &shape_wire::transport::TransportError) -> &'static str {
    use shape_wire::transport::TransportError as TE;
    match err {
        TE::ConnectionFailed(_) | TE::SendFailed(_) | TE::PayloadTooLarge { .. } => "Transport",
        TE::Timeout => "Timeout",
        TE::ReceiveFailed(_) | TE::ConnectionClosed | TE::Io(_) => "ConnectionLost",
    }
}

/// Short hex rendering of a content hash for legible error messages
/// (e.g. `3fa2b1c0…`), mirroring the design's §4.9 sample messages.
fn short_hash(hash: &FunctionHash) -> String {
    let b = &hash.0;
    format!(
        "{:02x}{:02x}{:02x}{:02x}…",
        b[0], b[1], b[2], b[3]
    )
}

// ---------------------------------------------------------------------------
// Wire message envelope (Phase 2: blob negotiation)
// ---------------------------------------------------------------------------

/// Envelope for all wire protocol messages.
///
/// Wraps the existing `RemoteCallRequest`/`RemoteCallResponse` with negotiation
/// and sidecar message types for bandwidth optimization on persistent connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireMessage {
    /// Offer function blob hashes to check what the remote already has.
    BlobNegotiation(BlobNegotiationRequest),
    /// Reply with the subset of offered hashes that are already cached.
    BlobNegotiationReply(BlobNegotiationResponse),
    /// A remote function call (may have blobs stripped if negotiation occurred).
    Call(RemoteCallRequest),
    /// Response to a remote function call.
    CallResponse(RemoteCallResponse),
    /// Best-effort cancellation for a `Call` that carried `call_id`.
    CancelCall(RemoteCancelRequest),
    /// Receiver's cancellation outcome.
    CancelCallResponse(RemoteCancelResponse),
    /// A large blob sent as a separate message before the call (Phase 3).
    Sidecar(BlobSidecar),

    // --- Execution server messages (V2) ---
    /// Execute Shape source code on the server.
    Execute(ExecuteRequest),
    /// Response to an Execute request.
    ExecuteResponse(ExecuteResponse),
    /// Validate Shape source code (parse + type-check) without executing.
    Validate(ValidateRequest),
    /// Response to a Validate request.
    ValidateResponse(ValidateResponse),
    /// Authenticate with the server (required for non-localhost).
    Auth(AuthRequest),
    /// Response to an Auth request.
    AuthResponse(AuthResponse),
    /// Execute a Shape file on the server.
    ExecuteFile(ExecuteFileRequest),
    /// Execute a Shape project (shape.toml) on the server.
    ExecuteProject(ExecuteProjectRequest),
    /// Validate a Shape file or project (parse + type-check) without executing.
    ValidatePath(ValidatePathRequest),
    /// Ping the server for liveness / capability discovery.
    Ping(PingRequest),
    /// Pong reply with server info.
    Pong(ServerInfo),
}

/// Ping request (empty payload for wire format consistency).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingRequest {}

/// Request to check which function blobs the remote side already has cached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobNegotiationRequest {
    /// Content hashes of function blobs the caller wants to send.
    pub offered_hashes: Vec<FunctionHash>,
}

/// Response indicating which offered blobs are already cached on the remote side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobNegotiationResponse {
    /// Subset of offered hashes that the remote already has in its blob cache.
    pub known_hashes: Vec<FunctionHash>,
}

/// A large binary payload sent as a separate message before the call request.
///
/// Used for splitting large BlobRef-backed values (DataTables, TypedArrays, etc.)
/// out of the main serialized payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobSidecar {
    pub sidecar_id: u32,
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Execution server message types (V2)
// ---------------------------------------------------------------------------

/// Request to execute Shape source code on the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRequest {
    /// Shape source code to execute.
    pub code: String,
    /// Client-assigned request ID for correlation.
    pub request_id: u64,
}

/// Response from executing Shape source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResponse {
    /// The request ID this response corresponds to.
    pub request_id: u64,
    /// Whether execution completed successfully.
    pub success: bool,
    /// Structured return value from execution.
    pub value: WireValue,
    /// Print/log output captured during execution (NOT the return value).
    pub stdout: Option<String>,
    /// Error message (if execution failed).
    pub error: Option<String>,
    /// Pre-rendered Content terminal representation (if value is Content).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_terminal: Option<String>,
    /// Pre-rendered Content HTML representation (if value is Content).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_html: Option<String>,
    /// Diagnostics (parse errors, type errors, warnings).
    pub diagnostics: Vec<WireDiagnostic>,
    /// Execution metrics (if available).
    pub metrics: Option<ExecutionMetrics>,
    /// Structured print output with rendered strings (MsgPack-serialized).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub print_output: Option<Vec<shape_wire::print_result::WirePrintResult>>,
}

/// Request to validate Shape source code without executing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateRequest {
    /// Shape source code to validate.
    pub code: String,
    /// Client-assigned request ID for correlation.
    pub request_id: u64,
}

/// Response from validating Shape source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateResponse {
    /// The request ID this response corresponds to.
    pub request_id: u64,
    /// Whether the code is valid (no errors).
    pub success: bool,
    /// Diagnostics (parse errors, type errors, warnings).
    pub diagnostics: Vec<WireDiagnostic>,
}

/// Request to execute a Shape file on the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteFileRequest {
    /// Absolute path to the .shape file.
    pub path: String,
    /// Optional working directory (defaults to file's parent).
    pub cwd: Option<String>,
    /// Client-assigned request ID for correlation.
    pub request_id: u64,
}

/// Request to execute a Shape project (shape.toml) on the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteProjectRequest {
    /// Absolute path to the project directory (must contain shape.toml).
    pub project_dir: String,
    /// Client-assigned request ID for correlation.
    pub request_id: u64,
}

/// Request to validate a Shape file or project without executing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatePathRequest {
    /// Path to a .shape file or a project directory (containing shape.toml).
    pub path: String,
    /// Client-assigned request ID for correlation.
    pub request_id: u64,
}

/// Authentication request for non-localhost connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    /// Bearer token for authentication.
    pub token: String,
}

/// Authentication response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    /// Whether authentication succeeded.
    pub authenticated: bool,
    /// Error message if authentication failed.
    pub error: Option<String>,
}

/// Server information returned in Pong responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Shape language version.
    pub shape_version: String,
    /// Wire protocol version.
    pub wire_protocol: u32,
    /// Server capabilities (e.g., "execute", "validate", "call", "blob-negotiation").
    pub capabilities: Vec<String>,
}

/// A diagnostic message (error, warning, info).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireDiagnostic {
    /// Severity: "error", "warning", "info".
    pub severity: String,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Source line number (1-indexed), if available.
    pub line: Option<u32>,
    /// Source column number (1-indexed), if available.
    pub column: Option<u32>,
}

/// Execution performance metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetrics {
    /// Number of VM instructions executed.
    pub instructions_executed: u64,
    /// Wall-clock time in milliseconds.
    pub wall_time_ms: u64,
    /// Peak memory usage in bytes.
    pub memory_bytes_peak: u64,
}

// ---------------------------------------------------------------------------
// Per-connection blob cache (Phase 2)
// ---------------------------------------------------------------------------

/// Per-connection cache of function blobs received from a remote peer.
///
/// Content hashes make stale entries harmless (same hash = same content),
/// so no invalidation protocol is needed. LRU eviction bounds memory usage.
pub struct RemoteBlobCache {
    blobs: std::collections::HashMap<FunctionHash, FunctionBlob>,
    /// Access order for LRU eviction (most recently used at the end).
    order: Vec<FunctionHash>,
    /// Maximum number of entries before LRU eviction kicks in.
    max_entries: usize,
}

impl RemoteBlobCache {
    /// Create a new blob cache with the given capacity.
    pub fn new(max_entries: usize) -> Self {
        Self {
            blobs: std::collections::HashMap::new(),
            order: Vec::new(),
            max_entries,
        }
    }

    /// Default cache with 4096 entry capacity.
    pub fn default_cache() -> Self {
        Self::new(4096)
    }

    /// Insert a blob, evicting the least recently used entry if at capacity.
    pub fn insert(&mut self, hash: FunctionHash, blob: FunctionBlob) {
        if self.blobs.contains_key(&hash) {
            // Move to end (most recently used)
            self.order.retain(|h| h != &hash);
            self.order.push(hash);
            return;
        }

        // Evict LRU if at capacity
        while self.blobs.len() >= self.max_entries && !self.order.is_empty() {
            let evicted = self.order.remove(0);
            self.blobs.remove(&evicted);
        }

        self.blobs.insert(hash, blob);
        self.order.push(hash);
    }

    /// Look up a cached blob by hash, updating access order.
    pub fn get(&mut self, hash: &FunctionHash) -> Option<&FunctionBlob> {
        if self.blobs.contains_key(hash) {
            self.order.retain(|h| h != hash);
            self.order.push(*hash);
            self.blobs.get(hash)
        } else {
            None
        }
    }

    /// Check if a hash is cached without updating access order.
    pub fn contains(&self, hash: &FunctionHash) -> bool {
        self.blobs.contains_key(hash)
    }

    /// Return all cached hashes.
    pub fn known_hashes(&self) -> Vec<FunctionHash> {
        self.blobs.keys().copied().collect()
    }

    /// Return the subset of `offered` hashes that are in the cache.
    pub fn filter_known(&self, offered: &[FunctionHash]) -> Vec<FunctionHash> {
        offered
            .iter()
            .filter(|h| self.blobs.contains_key(h))
            .copied()
            .collect()
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.blobs.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }

    /// Insert all blobs from a set, typically received from a remote call.
    pub fn insert_blobs(&mut self, blobs: &[(FunctionHash, FunctionBlob)]) {
        for (hash, blob) in blobs {
            self.insert(*hash, blob.clone());
        }
    }
}

/// Build a minimal set of function blobs for a function hash and its
/// transitive dependencies from a content-addressed `Program`.
///
/// Returns `None` if the program has no content-addressed representation
/// or the entry hash is not present in the function store.
pub fn build_minimal_blobs_by_hash(
    program: &BytecodeProgram,
    entry_hash: FunctionHash,
) -> Option<Vec<(FunctionHash, FunctionBlob)>> {
    let ca = program.content_addressed.as_ref()?;
    if !ca.function_store.contains_key(&entry_hash) {
        return None;
    }

    // Compute transitive closure of dependencies.
    //
    // WF-3E fixAB (sender side): the static `blob.dependencies` graph only
    // records `Call`/`CallForeign` edges. A foreign-bearing `@remote` function
    // reaches its `extern C` / `fn python` / `fn typescript` stub via
    // `LoadModuleBinding(idx) + CallValue` — a value-call through a module
    // binding, NOT a static `Call` edge. So the stub's own blob (which holds
    // the `CallForeign` body) never enters the closure and the receiver
    // reconstructs a program without the stub body ("got Bool" at dispatch).
    //
    // To pack the FULL reachable closure we additionally scan each blob's
    // instruction stream for `Operand::ModuleBinding(idx)` operands, resolve
    // `module_binding_names[idx]` → function name → the function's content
    // hash (`function_blob_hashes[fn_id]`), and enqueue that hash. This pulls
    // in foreign-stub blobs and any module-scope function-value target the
    // transferred function references. Native stdlib module bindings
    // (`env`/`file`/`http`) resolve to module OBJECTS with no matching
    // top-level function → no hash → skipped here (handled receiver-side by
    // registering the native capability modules).
    let mut needed: std::collections::HashSet<FunctionHash> = std::collections::HashSet::new();
    let mut queue = vec![entry_hash];
    while let Some(hash) = queue.pop() {
        if needed.insert(hash) {
            if let Some(blob) = ca.function_store.get(&hash) {
                for dep in &blob.dependencies {
                    if !needed.contains(dep) {
                        queue.push(*dep);
                    }
                }
                // Scan for module-binding value targets (foreign stubs +
                // module-scope function values reached via CallValue).
                for instr in &blob.instructions {
                    let Some(crate::bytecode::Operand::ModuleBinding(idx)) = instr.operand else {
                        continue;
                    };
                    let Some(binding_name) = program.module_binding_names.get(idx as usize) else {
                        continue;
                    };
                    let Some(fn_id) = program
                        .functions
                        .iter()
                        .position(|f| f.name == *binding_name)
                    else {
                        // No top-level function with this name — a native
                        // stdlib module binding (env/file/http) or a non-
                        // function value. Nothing to transfer from the sender.
                        continue;
                    };
                    if let Some(Some(target_hash)) =
                        program.function_blob_hashes.get(fn_id).copied()
                    {
                        if !needed.contains(&target_hash) {
                            queue.push(target_hash);
                        }
                    }
                }
            }
        }
    }

    // Collect the minimal blob set
    let blobs: Vec<(FunctionHash, FunctionBlob)> = needed
        .into_iter()
        .filter_map(|hash| {
            ca.function_store
                .get(&hash)
                .map(|blob| (hash, blob.clone()))
        })
        .collect();

    Some(blobs)
}

/// Backwards-compatible name-based wrapper around `build_minimal_blobs_by_hash`.
///
/// If multiple blobs share the same name, this returns `None` to avoid
/// ambiguous, potentially incorrect dependency selection.
pub fn build_minimal_blobs(
    program: &BytecodeProgram,
    fn_name: &str,
) -> Option<Vec<(FunctionHash, FunctionBlob)>> {
    let ca = program.content_addressed.as_ref()?;
    let mut matches = ca.function_store.iter().filter_map(|(hash, blob)| {
        if blob.name == fn_name {
            Some(*hash)
        } else {
            None
        }
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    build_minimal_blobs_by_hash(program, first)
}

/// Build a minimal `Program` from function blobs and an explicit entry hash.
///
/// Used on the callee side to reconstruct a `Program` from blobs received in
/// a `RemoteCallRequest`.
pub fn program_from_blobs_by_hash(
    blobs: Vec<(FunctionHash, FunctionBlob)>,
    entry_hash: FunctionHash,
    source: &BytecodeProgram,
) -> Option<Program> {
    let function_store: std::collections::HashMap<FunctionHash, FunctionBlob> =
        blobs.into_iter().collect();
    if !function_store.contains_key(&entry_hash) {
        return None;
    }

    Some(Program {
        entry: entry_hash,
        function_store,
        top_level_locals_count: source.top_level_locals_count,
        top_level_local_storage_hints: source.top_level_local_storage_hints.clone(),
        module_binding_names: source.module_binding_names.clone(),
        module_binding_storage_hints: source.module_binding_storage_hints.clone(),
        function_local_storage_hints: source.function_local_storage_hints.clone(),
        top_level_frame: source.top_level_frame.clone(),
        top_level_local_concrete_types: source.top_level_local_concrete_types.clone(),
        function_local_concrete_types: source.function_local_concrete_types.clone(),
        function_return_concrete_types: source.function_return_concrete_types.clone(),
        monomorphized_method_call_sites: source.monomorphized_method_call_sites.clone(),
        value_call_return_concrete_types: source.value_call_return_concrete_types.clone(),
        operator_trait_dispatch_sites: source.operator_trait_dispatch_sites.clone(),
        data_schema: source.data_schema.clone(),
        type_schema_registry: source.type_schema_registry.clone(),
        trait_method_symbols: source.trait_method_symbols.clone(),
        foreign_functions: source.foreign_functions.clone(),
        native_struct_layouts: source.native_struct_layouts.clone(),
        debug_info: source.debug_info.clone(),
        // Closure spec §14.6 (H6.5): propagate the per-name layout side-
        // table. Remote-stream origins that lack the side-table fail
        // hard at the VM producer; there is no legacy fallback.
        closure_function_layouts_by_name: source
            .content_addressed
            .as_ref()
            .map(|ca| ca.closure_function_layouts_by_name.clone())
            .unwrap_or_default(),
        // ADR-006 §2.7.24 Q25.C: propagate trait-object vtables from
        // the source BytecodeProgram so remote-streamed programs can
        // dispatch dyn method calls.
        trait_vtables: source.trait_vtables.clone(),
        // R8 W8 Cluster A surface-and-stop flag propagation.
        has_imported_const_inline: source.has_imported_const_inline,
        // R8 W9 B1 W17-marshal-return surface-and-stop flag propagation.
        has_w17_marshal_residual: source.has_w17_marshal_residual,
        // c4-4B TryUnwrap (`?` operator) surface-and-stop flag propagation.
        has_try_unwrap_residual: source.has_try_unwrap_residual,
        has_reference_escape_promotion: source.has_reference_escape_promotion,
        has_null_coalesce_residual: source.has_null_coalesce_residual,
    })
}

/// Backwards-compatible name-based wrapper around `program_from_blobs_by_hash`.
pub fn program_from_blobs(
    blobs: Vec<(FunctionHash, FunctionBlob)>,
    fn_name: &str,
    source: &BytecodeProgram,
) -> Option<Program> {
    let mut matches = blobs.iter().filter_map(|(hash, blob)| {
        if blob.name == fn_name {
            Some(*hash)
        } else {
            None
        }
    });
    let entry = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    program_from_blobs_by_hash(blobs, entry, source)
}

/// Execute a remote call request on this machine.
///
/// This is the entry point for the receiving side. It:
/// 1. Reconstructs the `BytecodeProgram` and populates its `TypeSchemaRegistry`
/// 2. Creates a full `VirtualMachine` with the program
/// 3. Materializes serialized arguments as kinded VM slots
/// 4. Calls the function by name or ID
/// 5. Converts the result back to `SerializableVMValue`
///
/// The `store` is used for `SerializableVMValue` ↔ slot conversion
/// (needed for `BlobRef`-backed values like DataTable).
///
/// Phase-2c deferral: the slot/serializable round-trip is currently
/// stubbed (see `execute_inner` body). The dispatch entry continues to
/// exist so callers compile through the deferral; invocation surfaces
/// the gap at runtime.
pub fn execute_remote_call(
    request: RemoteCallRequest,
    store: &SnapshotStore,
    granted: &shape_abi_v1::PermissionSet,
) -> RemoteCallResponse {
    // The no-runtimes path serves servers with no dynamic language extensions
    // loaded (extern C only, or plain Shape). No `ffi_languages` opt-in list
    // is meaningful here — `extern C` is `is_native` and bypasses the language
    // gate; a dynamic foreign call on such a server has no runtime to link and
    // fails cleanly regardless. Pass an empty scope; the receiver-strict flag
    // (set inside `run_remote_call`) still refuses any dynamic foreign call.
    match execute_inner(request, store, granted, &shape_abi_v1::ScopeConstraints::none()) {
        Ok(value) => RemoteCallResponse { result: Ok(value) },
        Err(err) => RemoteCallResponse { result: Err(err) },
    }
}

/// Execute a remote call without dynamic language runtimes, while preserving
/// the receiver's scope constraints.
///
/// This is the serve path for receivers that may still need strict
/// `ffi_languages` enforcement even when no runtime registry is installed.
/// Dynamic calls then either fail the receiver opt-in gate or continue to the
/// normal "no extension provides language" error; `extern C` remains governed
/// by `Ffi` plus native library/symbol scope, not by `ffi_languages`.
pub fn execute_remote_call_with_scope(
    request: RemoteCallRequest,
    store: &SnapshotStore,
    granted: &shape_abi_v1::PermissionSet,
    scope: &shape_abi_v1::ScopeConstraints,
) -> RemoteCallResponse {
    match execute_inner(request, store, granted, scope) {
        Ok(value) => RemoteCallResponse { result: Ok(value) },
        Err(err) => RemoteCallResponse { result: Err(err) },
    }
}

/// Execute a remote call with pre-loaded language runtime extensions.
///
/// `language_runtimes` maps language IDs (e.g. "python") to pre-loaded
/// runtime handles. The server loads these once at startup from installed
/// extensions. The bytecode carries foreign function source text; the
/// runtime on the server compiles and executes it.
pub fn execute_remote_call_with_runtimes(
    request: RemoteCallRequest,
    store: &SnapshotStore,
    language_runtimes: &std::collections::HashMap<
        String,
        std::sync::Arc<shape_runtime::plugins::language_runtime::PluginLanguageRuntime>,
    >,
    granted: &shape_abi_v1::PermissionSet,
    scope: &shape_abi_v1::ScopeConstraints,
) -> RemoteCallResponse {
    match execute_inner_with_runtimes(request, store, language_runtimes, granted, scope) {
        Ok(value) => RemoteCallResponse { result: Ok(value) },
        Err(err) => RemoteCallResponse { result: Err(err) },
    }
}

fn execute_inner(
    request: RemoteCallRequest,
    store: &SnapshotStore,
    granted: &shape_abi_v1::PermissionSet,
    scope: &shape_abi_v1::ScopeConstraints,
) -> Result<SerializableVMValue, RemoteCallError> {
    // T1-host-tier-marshal-rebuild (ADR-006 §2.7.4, R8 2026-05-23):
    // kind-threaded marshal protocol via
    // `shape_runtime::snapshot::serializable_to_slot` (in) +
    // `shape_runtime::snapshot::slot_to_serializable` (out). Each arg's
    // expected kind is read from the callee's `frame_descriptor.slots`
    // (i.e. the per-slot proven `NativeKind` per ADR-006 §2.7.5.1 — no
    // `Unknown` placeholder). The return-kind is read from the callee's
    // `frame_descriptor.abi_return_kind()`.
    //
    // Per the §0.A.iv supervisor ruling, frame-descriptor absence
    // produces a structured `RemoteCallError` (no silent-degrade): a
    // remote call cannot proceed if the callee has no proven param
    // kinds, because the marshal protocol cannot pick an in-arm.
    run_remote_call(request, store, None, granted, scope)
}

fn execute_inner_with_runtimes(
    request: RemoteCallRequest,
    store: &SnapshotStore,
    language_runtimes: &std::collections::HashMap<
        String,
        std::sync::Arc<shape_runtime::plugins::language_runtime::PluginLanguageRuntime>,
    >,
    granted: &shape_abi_v1::PermissionSet,
    scope: &shape_abi_v1::ScopeConstraints,
) -> Result<SerializableVMValue, RemoteCallError> {
    // Same path as `execute_inner` plus the foreign-function language-
    // runtime hookup. T1-host-tier-marshal-rebuild covers the marshal
    // protocol; the language-runtime registration is forwarded through
    // `run_remote_call` so the VM picks up the runtimes before invoking
    // the callee.
    run_remote_call(request, store, Some(language_runtimes), granted, scope)
}

/// Collect every dependency content hash referenced by a blob in the store
/// but not itself present in the store (distributed §4.3-4). The linker
/// reports only the FIRST missing blob; this accumulates ALL of them so the
/// sender can resupply in a single round-trip. Returned in first-seen order,
/// deduplicated.
fn missing_dependency_blobs(ca: &Program) -> Vec<FunctionHash> {
    let mut missing: Vec<FunctionHash> = Vec::new();
    let mut seen: std::collections::HashSet<FunctionHash> = std::collections::HashSet::new();
    for blob in ca.function_store.values() {
        for dep in &blob.dependencies {
            if !ca.function_store.contains_key(dep) && seen.insert(*dep) {
                missing.push(*dep);
            }
        }
    }
    missing
}

/// Shared marshal+dispatch core for `execute_inner` /
/// `execute_inner_with_runtimes`. Per ADR-006 §2.7.4 the protocol is:
///
/// 1. Reconstruct the `BytecodeProgram` (full payload or from blobs).
/// 2. Build a `VirtualMachine`, load the program, populate module objects.
/// 3. Resolve the callee (hash → id → name precedence).
/// 4. Read the callee's per-slot `NativeKind` from `frame_descriptor.slots`
///    and the return ABI kind from `frame_descriptor.abi_return_kind()`.
/// 5. Materialize each `SerializableVMValue` arg into a `KindedSlot` via
///    `serializable_to_slot(arg, expected_kind, store)`.
/// 6. Invoke the callee through the kinded ABI at a host boundary.
/// 7. If the callee declared and returned a local `Future<T>`, resolve that
///    receiver-local future first; then project the payload `KindedSlot` to
///    `SerializableVMValue` via `slot_to_serializable(bits, kind, store)`.
///
/// Closure upvalue marshal (`request.upvalues` and `execute_closure`) is
/// NOT covered by T1: the per-capture kind track lives on the closure
/// header (ADR-006 §2.7.8 / Q10 cell-storage parallel-kind), which is
/// rebuilt in a downstream sub-cluster. The body surfaces this with a
/// structured error rather than silently dispatching.
fn run_remote_call(
    request: RemoteCallRequest,
    store: &SnapshotStore,
    language_runtimes: Option<
        &std::collections::HashMap<
            String,
            std::sync::Arc<shape_runtime::plugins::language_runtime::PluginLanguageRuntime>,
        >,
    >,
    granted: &shape_abi_v1::PermissionSet,
    scope: &shape_abi_v1::ScopeConstraints,
) -> Result<SerializableVMValue, RemoteCallError> {
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_runtime::context::ExecutionContext;
    use shape_runtime::snapshot::serializable_to_slot;
    use shape_value::{KindedSlot, ValueSlot};

    // Step 1: reconstruct the program. If function_blobs are supplied,
    // build a content-addressed Program; otherwise use the full payload.
    let mut program: BytecodeProgram = request.program;
    program.type_schema_registry = request.type_schemas;

    if let (Some(blobs), Some(entry_hash)) = (request.function_blobs.clone(), request.function_hash)
    {
        if let Some(ca) = program_from_blobs_by_hash(blobs, entry_hash, &program) {
            program.content_addressed = Some(ca);
        }
    }

    // Closures cross the wire with an explicit per-capture NativeKind track
    // (ADR-006 §2.7.8 / Q10; distributed §4.4). Validate the capture identity
    // and the refusal matrix against the callee blob BEFORE load/execute, so
    // statically-refusable captures (mutable / reference / resource / nested
    // closure) and a missing kind track fail fast and never touch the VM.
    // Materialization + dispatch happen after callee resolution (Step 3b).
    if let Some(upvalues) = request.upvalues.as_ref() {
        validate_remote_closure_captures(
            &program,
            upvalues,
            request.upvalue_kinds.as_deref(),
            request.function_hash,
        )?;
    }

    // Step 2: build VM and load program under RECEIVER-OWNED permission
    // enforcement (distributed §4.6 — "never trust the sender").
    //
    // WF-1D + WF-2C security wiring: remote/wire code is untrusted. The
    // fail-closed runtime gate below (`set_permissions(Some(...), _)`) is the
    // security boundary against dishonest senders — `None` is forbidden here
    // because `check_permission` is fail-OPEN when `None`. For content-
    // addressed payloads we additionally (a) recompute every blob's content
    // hash from the received bytes and reject mismatches (§4.3-2), (b)
    // accumulate any missing dependency blobs into a structured
    // `MissingModuleFunction` (§4.3-4), and (c) recompute the linker
    // permission union from the VERIFIED blobs and gate the load against the
    // receiver's granted set (§4.6) — never the sender's self-declared claim.
    let mut vm = VirtualMachine::new(VMConfig::default());
    // WF-2F axis C (§4.6 / OQ-6): install the receiver's granted permission
    // set AND its scope constraints, then flip the wire-serve receiver posture
    // on. `ffi_languages` is now enforced as a strict OPT-IN allow-list for
    // dynamic foreign languages (`fn python` / `fn typescript`): an empty list
    // refuses every dynamic foreign call unless the operator explicitly opted
    // the language in (`shape serve --ffi-languages python`). The scope is the
    // RECEIVER's own — never the sender's self-declared claim (zero sender
    // trust). `extern C` stays gated by `Ffi` + `ffi_libraries`, not language.
    vm.set_permissions(Some(granted.clone()), Some(scope.clone()));
    vm.ffi_receiver_strict = true;

    // WF-2F axis A (design F3): register the receiver's language runtimes into
    // the VM BEFORE load so the dynamic foreign-call link-now path
    // (`fn python` / `fn typescript`) can resolve its runtime. `extern C`
    // links via native_abi dlopen and needs no registry. Never-trust-the-
    // sender is unaffected: these are the RECEIVER's own installed runtimes,
    // and the `Ffi` permission gate below still governs whether any foreign
    // call is allowed at all.
    if let Some(runtimes) = language_runtimes {
        vm.set_language_runtimes(runtimes.clone());
    }
    match program.content_addressed.clone() {
        Some(ca) => {
            // (a) §4.3-2: recompute each blob's content hash and reject any
            // blob whose bytes do not match its claimed key. Permissions are
            // baked into the hash, so a sender cannot claim `Pure` for a blob
            // whose bytes demand `FsWrite` and still verify.
            for (claimed, blob) in ca.function_store.iter() {
                let recomputed = blob.compute_hash();
                if recomputed != *claimed {
                    return Err(RemoteCallError::new(
                        RemoteErrorKind::HashMismatch,
                        format!(
                            "blob {} failed content verification — rejected \
                             (recomputed {})",
                            short_hash(claimed),
                            short_hash(&recomputed),
                        ),
                    ));
                }
            }

            // (b) §4.3-4: accumulate ALL dependency blobs referenced but not
            // present, so the sender can resupply in a single round-trip. The
            // fallible linker path replaces `load_program`'s panic on missing
            // blobs — network input can never reach the panic.
            let missing = missing_dependency_blobs(&ca);
            if !missing.is_empty() {
                return Err(RemoteCallError::missing_module_function(
                    format!(
                        "cannot link '{}': missing {} dependency blob(s)",
                        request.function_name,
                        missing.len(),
                    ),
                    missing,
                ));
            }

            // (c) §4.6: recompute the linker permission union from the verified
            // blobs, then gate the load against the receiver's granted set.
            let linked = crate::linker::link(&ca).map_err(|e| {
                RemoteCallError::new(
                    RemoteErrorKind::RuntimeError,
                    format!("link failed for '{}': {e}", request.function_name),
                )
            })?;
            vm.load_linked_program_with_permissions(linked, granted)
                .map_err(|e| match e {
                    crate::executor::PermissionError::InsufficientPermissions {
                        missing,
                        ..
                    } => {
                        let names: Vec<&str> = missing.iter().map(|p| p.name()).collect();
                        RemoteCallError::new(
                            RemoteErrorKind::PermissionDenied,
                            format!(
                                "remote call '{}' refused — the server does not grant [{}]",
                                request.function_name,
                                names.join(", "),
                            ),
                        )
                    }
                    crate::executor::PermissionError::LinkError(s) => RemoteCallError::new(
                        RemoteErrorKind::RuntimeError,
                        format!("link failed for '{}': {s}", request.function_name),
                    ),
                    crate::executor::PermissionError::DeterministicForeignRefused => {
                        RemoteCallError::new(
                            RemoteErrorKind::PermissionDenied,
                            format!(
                                "remote call '{}' refused — this program requires the Ffi \
                                 permission (extern C / embedded Python/TypeScript), and a \
                                 deterministic execution context cannot attest foreign bodies \
                                 through the extension boundary",
                                request.function_name,
                            ),
                        )
                    }
                })?;
        }
        // Full-payload fallback: no content-addressed metadata to verify or
        // link. `load_program` does not invoke the linker when
        // `content_addressed` is `None`, so it cannot hit the linker panic
        // path. The fail-closed runtime gate above is the security boundary
        // for this path.
        None => vm.load_program(program),
    }
    vm.populate_module_objects();

    // WF-2F axis A: initialize the module bindings holding foreign-stub
    // function values. The transferred function reaches its `extern C` /
    // `fn python` / `fn typescript` stub via `LoadModuleBinding` + `CallValue`,
    // but per-function dispatch never runs top-level module-init, so those
    // bindings would otherwise read the `(0, Bool)` uninitialised sentinel and
    // `call_value_immediate_nb` would refuse the Bool-kinded callee. See
    // `VirtualMachine::initialize_foreign_stub_bindings`.
    vm.initialize_foreign_stub_bindings().map_err(|e| {
        RemoteCallError::new(
            RemoteErrorKind::RuntimeError,
            format!(
                "remote call '{}': foreign stub binding init failed: {:?}",
                request.function_name, e,
            ),
        )
    })?;

    // WF-2F axis C (combined compose A+B): install the snapshot persistence
    // context on the receiver VM so a `snapshot()` reached mid-transfer
    // CAPTURES and PERSISTS a complete, resumable content-addressed envelope
    // into the receiver's store — the same store a subsequent `--resume` on
    // this node reads. Without this the in-loop consumer (dispatch.rs) still
    // continues, but only via the `NoStore` barrier marker; with it, a
    // foreign-bearing function transferred `@remote` that snapshots mid-flight
    // produces a genuine `Ok(Snapshot::Hash(id))` — this is what makes the
    // transfer + snapshot + resume composition real rather than a no-op refusal.
    // The `SemanticSnapshot` is empty (a transferred function exports nothing;
    // resume re-verifies through the `CodeManifest` blob graph regardless).
    {
        use shape_runtime::snapshot::{SemanticSnapshot, SnapshotEnvelopeSeed};
        let semantic = SemanticSnapshot {
            exported_symbols: std::collections::HashSet::new(),
        };
        if let Ok(semantic_hash) = store.put_struct(&semantic) {
            let seed = SnapshotEnvelopeSeed {
                semantic_hash,
                script_path: None,
            };
            vm.set_snapshot_context(std::sync::Arc::new(store.clone()), seed);
        }
        // A store write failure is non-fatal: the in-loop consumer falls back
        // to the clean `NoStore`/`PersistFailed` barrier marker and the run
        // still continues — never a trap.
    }

    // Step 3: resolve callee. function_hash (canonical) > function_id > name.
    let func_id: u16 = if let Some(hash) = request.function_hash {
        vm.program
            .function_blob_hashes
            .iter()
            .position(|h| *h == Some(hash))
            .map(|p| p as u16)
            .or_else(|| request.function_id)
            .or_else(|| {
                vm.program
                    .functions
                    .iter()
                    .position(|f| f.name == request.function_name)
                    .map(|p| p as u16)
            })
            .ok_or_else(|| {
                RemoteCallError::new(
                    RemoteErrorKind::FunctionNotFound,
                    format!(
                        "function not found by hash; name='{}', id={:?}",
                        request.function_name, request.function_id,
                    ),
                )
            })?
    } else if let Some(id) = request.function_id {
        id
    } else {
        vm.program
            .functions
            .iter()
            .position(|f| f.name == request.function_name)
            .map(|p| p as u16)
            .ok_or_else(|| {
                RemoteCallError::new(
                    RemoteErrorKind::FunctionNotFound,
                    format!("function '{}' not found", request.function_name),
                )
            })?
    };

    // Step 3b: closure dispatch. A closure's frame ABI differs from a plain
    // function — captures are the leading frame slots, so `Function.arity`
    // counts them and the actual arguments start after the captures. Closures
    // therefore take a dedicated marshal + `OwnedClosureBlock` materialization
    // path (distributed §4.4). The capture track was already validated above.
    // The receiver has a snapshot store seed but still needs a live runtime
    // context for snapshot envelope persistence, matching the non-REPL run path.
    let mut ctx = ExecutionContext::new_empty();
    if let Some(upvalues) = request.upvalues.as_ref() {
        return finish_remote_closure_call(
            &mut vm,
            func_id,
            upvalues,
            &request.arguments,
            store,
            &mut ctx,
        );
    }

    // Step 4: pick per-arg expected kinds from the callee's frame
    // descriptor. ADR-006 §2.7.5.1: a present FunctionBlob has every
    // slot's NativeKind proven — no Unknown placeholder.
    let function = vm
        .program
        .functions
        .get(func_id as usize)
        .ok_or_else(|| {
            RemoteCallError::new(
                RemoteErrorKind::FunctionNotFound,
                format!("function_id {} out of range", func_id),
            )
        })?;

    let arity = function.arity as usize;
    if request.arguments.len() != arity {
        return Err(RemoteCallError::new(
            RemoteErrorKind::ArgumentError,
            format!(
                "argument count mismatch for function '{}': expected {}, got {}",
                function.name,
                arity,
                request.arguments.len(),
            ),
        ));
    }

    let frame_desc = function.frame_descriptor.clone();
    let arg_kinds: Vec<shape_value::NativeKind> = if let Some(ref fd) = frame_desc {
        if fd.slots.len() < arity {
            return Err(RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                format!(
                    "function '{}' frame_descriptor has {} slots but arity is {}",
                    function.name,
                    fd.slots.len(),
                    arity,
                ),
            ));
        }
        fd.slots.iter().take(arity).copied().collect()
    } else if arity == 0 {
        Vec::new()
    } else {
        return Err(RemoteCallError::new(
            RemoteErrorKind::ArgumentError,
            format!(
                "function '{}' has no frame_descriptor — cannot derive \
                 per-arg NativeKind for marshal protocol (ADR-006 §2.7.5.1)",
                function.name,
            ),
        ));
    };

    let return_kind = frame_desc.as_ref().and_then(|fd| fd.abi_return_kind());
    let function_name_owned = function.name.clone();
    let _ = function; // release the borrow before moving vm into call

    // Step 5: marshal each SerializableVMValue → KindedSlot per
    // expected_kind. `serializable_to_slot` allocates strong-count
    // shares for heap-kinded args; each share transfers into the
    // callee's frame via `execute_function_by_id`'s share-neutral
    // call-helper (per cluster-1.5 fix).
    let mut args: Vec<KindedSlot> = Vec::with_capacity(arity);
    for (idx, sv) in request.arguments.iter().enumerate() {
        let expected = arg_kinds[idx];
        let (bits, kind) = serializable_to_slot(sv, expected, store).map_err(|e| {
            RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                format!(
                    "arg {} marshal failure (expected kind {:?}): {}",
                    idx, expected, e,
                ),
            )
        })?;
        args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
    }

    // Step 6: dispatch.
    let result = vm
        .execute_function_by_id_at_host_boundary(func_id, args, Some(&mut ctx))
        .map_err(|e| {
            RemoteCallError::new(
                RemoteErrorKind::RuntimeError,
                format!(
                    "remote execution of '{}' failed: {:?}",
                    function_name_owned, e,
                ),
            )
        })?;

    // Step 7: project the returned KindedSlot → SerializableVMValue.
    serialize_remote_return_slot(
        &mut vm,
        result,
        return_kind,
        store,
        &format!("function '{}'", function_name_owned),
    )
}

fn serialize_remote_return_slot(
    vm: &mut crate::executor::VirtualMachine,
    result: shape_value::KindedSlot,
    return_kind: Option<shape_value::NativeKind>,
    store: &SnapshotStore,
    callee_subject: &str,
) -> Result<SerializableVMValue, RemoteCallError> {
    use shape_runtime::snapshot::slot_to_serializable;
    use shape_value::{HeapKind, NativeKind};

    let bits = result.raw();
    let kind = result.kind();
    let future_kind = NativeKind::Ptr(HeapKind::Future);

    let result = if kind == future_kind {
        match return_kind {
            Some(declared) if declared == future_kind => {}
            Some(declared) => {
                return Err(RemoteCallError::new(
                    RemoteErrorKind::ArgumentError,
                    format!(
                        "{callee_subject} returned kind {:?} but frame_descriptor \
                         declared return_kind {:?}",
                        kind, declared,
                    ),
                ));
            }
            None => {
                return Err(RemoteCallError::new(
                    RemoteErrorKind::ArgumentError,
                    format!(
                        "{callee_subject} returned a Future handle but has no declared \
                         Future return kind in its frame_descriptor"
                    ),
                ));
            }
        }
        // The `Future` carrier is an inline scheduler id, not a heap share.
        drop(result);
        vm.resolve_future_handle_blocking(bits).map_err(|e| {
            RemoteCallError::new(
                RemoteErrorKind::RuntimeError,
                format!("{callee_subject} future materialization failed: {:?}", e),
            )
        })?
    } else {
        if let Some(declared) = return_kind {
            if kind != declared {
                return Err(RemoteCallError::new(
                    RemoteErrorKind::ArgumentError,
                    format!(
                        "{callee_subject} returned kind {:?} but frame_descriptor \
                         declared return_kind {:?}",
                        kind, declared,
                    ),
                ));
            }
        }
        result
    };

    // The returned slot owns one strong-count share that we must release after
    // serialization. `slot_to_serializable` borrows; `KindedSlot::Drop` retires
    // the share at scope exit. For `Future`, the original inline handle was
    // dropped before resolving, and this slot is the materialized payload.
    let serialized = slot_to_serializable(result.raw(), result.kind(), store).map_err(|e| {
        RemoteCallError::new(
            RemoteErrorKind::RuntimeError,
            format!("return-value marshal failure: {}", e),
        )
    })?;
    drop(result);
    Ok(serialized)
}

/// Render a capture's `NativeKind` in Shape-surface words for user-facing
/// refusal messages (distributed §4.4: no `NativeKind` / `HeapKind` jargon and
/// no slot indices in messages the user reads).
fn capture_type_surface(kind: shape_value::NativeKind) -> &'static str {
    use shape_value::{HeapKind, NativeKind};
    match kind {
        NativeKind::Int64 | NativeKind::Int32 | NativeKind::Int8 => "an int",
        NativeKind::Float64 => "a number",
        NativeKind::Bool => "a bool",
        NativeKind::String | NativeKind::StringV2 => "a string",
        NativeKind::DecimalV2 => "a decimal",
        NativeKind::Null => "null",
        NativeKind::Ptr(HeapKind::TypedArray) => "an array",
        NativeKind::Ptr(HeapKind::TypedObject) => "an object",
        NativeKind::Ptr(HeapKind::HashMap) => "a map",
        NativeKind::Ptr(HeapKind::Closure) => "a closure",
        NativeKind::Ptr(HeapKind::Reference) => "a reference",
        NativeKind::Ptr(HeapKind::SharedCell) => "a shared cell",
        NativeKind::Ptr(HeapKind::IoHandle) => "an open resource handle",
        NativeKind::Ptr(HeapKind::Future) => "a pending async value",
        NativeKind::Ptr(HeapKind::TaskGroup) => "a task group",
        NativeKind::Ptr(HeapKind::String) => "a string",
        _ => "an unsupported value",
    }
}

/// Validate a remote closure call's capture track against the callee blob
/// BEFORE any load/execute (distributed §4.4). Runs, in order:
///
/// 1. the kind-track presence + length check (`upvalues` without a lockstep
///    `upvalue_kinds` track is a structured `ArgumentError` — never a
///    Bool-default, never a kind fabricated from raw bits; ADR-006 §2.7.7/§2.7.8);
/// 2. the `upvalue_kinds` ↔ hash-covered `capture_kinds` cross-check (§4.8);
/// 3. the refusal matrix (mutable / reference / shared-cell / resource /
///    nested-closure captures) — all refusals are `UnsupportedCapture` naming
///    the captured *variable* (from the blob's non-hash `capture_names`) and
///    rendering its type in Shape-surface words.
///
/// Returns the first fault encountered; `Ok(())` means the captures are all
/// immutable, kind-consistent, and safe to materialize.
fn validate_remote_closure_captures(
    program: &BytecodeProgram,
    upvalues: &[SerializableVMValue],
    upvalue_kinds: Option<&[shape_value::NativeKind]>,
    function_hash: Option<FunctionHash>,
) -> Result<(), RemoteCallError> {
    use shape_value::{HeapKind, NativeKind};

    // (1) The kind track is REQUIRED alongside upvalues.
    let upvalue_kinds = upvalue_kinds.ok_or_else(|| {
        RemoteCallError::new(
            RemoteErrorKind::ArgumentError,
            "closure request carries upvalues but no upvalue_kinds track \
             (ADR-006 §2.7.8) — the receiver refuses to materialize captures \
             from a kind-blind payload",
        )
    })?;
    if upvalues.len() != upvalue_kinds.len() {
        return Err(RemoteCallError::new(
            RemoteErrorKind::ArgumentError,
            format!(
                "closure capture track length mismatch: {} upvalue(s) vs {} kind(s)",
                upvalues.len(),
                upvalue_kinds.len(),
            ),
        ));
    }

    // Resolve the callee blob for its hash-covered capture identity + names.
    let blob = function_hash
        .and_then(|h| {
            program
                .content_addressed
                .as_ref()
                .and_then(|ca| ca.function_store.get(&h))
        })
        .ok_or_else(|| {
            RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                "closure call carries no resolvable content hash — cannot verify \
                 capture identity (a closure must ship its function blob)",
            )
        })?;

    // (2) capture_kinds is hash-covered call-ABI identity (§4.8). A shipped
    // upvalue_kinds track that disagrees with the blob it arrived with is a
    // protocol-integrity fault.
    if blob.capture_kinds.len() != upvalue_kinds.len() {
        return Err(RemoteCallError::new(
            RemoteErrorKind::ArgumentError,
            format!(
                "closure '{}' declares {} capture(s) but the request carries {} \
                 upvalue kind(s)",
                blob.name,
                blob.capture_kinds.len(),
                upvalue_kinds.len(),
            ),
        ));
    }
    for (i, (declared, shipped)) in blob
        .capture_kinds
        .iter()
        .zip(upvalue_kinds.iter())
        .enumerate()
    {
        if declared != shipped {
            return Err(RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                format!(
                    "closure '{}' capture {} kind disagrees with the callee blob \
                     (request {:?}, blob {:?})",
                    blob.name, i, shipped, declared,
                ),
            ));
        }
    }

    // (3) Refusal matrix. Name the captured VARIABLE; render its type in
    // Shape-surface words. Every refusal is `UnsupportedCapture`.
    let name_of = |i: usize| -> String {
        blob.capture_names
            .get(i)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("capture #{i}"))
    };
    for (i, kind) in blob.capture_kinds.iter().enumerate() {
        if blob.mutable_captures.get(i).copied().unwrap_or(false) {
            return Err(RemoteCallError::new(
                RemoteErrorKind::UnsupportedCapture,
                format!(
                    "closure captures '{}' mutably — a remote call copies captures \
                     by value, so writes on the remote node would be lost; pass it \
                     as an argument and return the new value",
                    name_of(i),
                ),
            ));
        }
        let reason = match kind {
            NativeKind::Ptr(HeapKind::Closure) => Some(
                "is itself a closure — hoist it to a named top-level function (a \
                 static dependency) or pass its result as a value"
                    .to_string(),
            ),
            NativeKind::Ptr(HeapKind::Reference) => Some(
                "is a reference (& / &mut) — references do not cross nodes; pass the \
                 referenced value and return the result"
                    .to_string(),
            ),
            NativeKind::Ptr(HeapKind::SharedCell) => Some(
                "is a shared mutable cell — cross-node coherence is not supported; \
                 pass the value as an argument and return the new value"
                    .to_string(),
            ),
            NativeKind::Ptr(HeapKind::IoHandle)
            | NativeKind::Ptr(HeapKind::Future)
            | NativeKind::Ptr(HeapKind::TaskGroup) => Some(format!(
                "is {} — open resources have no meaningful remote identity",
                capture_type_surface(*kind),
            )),
            _ => None,
        };
        if let Some(reason) = reason {
            return Err(RemoteCallError::new(
                RemoteErrorKind::UnsupportedCapture,
                format!("closure capture '{}' {}", name_of(i), reason),
            ));
        }
    }

    Ok(())
}

/// Marshal + materialize + execute a remote CLOSURE call (distributed §4.4).
///
/// Called only after [`validate_remote_closure_captures`] has cleared the
/// capture track (all captures immutable, kind-consistent). Rebuilds the
/// captures into a fresh `OwnedClosureBlock` at their proven `NativeKind`s (the
/// ADR-006 §2.7.8 cell-storage parallel-kind track — the same template as
/// `VirtualMachine::from_snapshot`'s `restore_call_stack`), marshals the actual
/// arguments at the callee's post-capture frame slots, and dispatches through
/// the §2.7.11 value-call ABI (`execute_closure`).
/// Rebuild an all-`Immutable` [`ClosureLayout`] from a proven per-capture
/// `NativeKind` track (distributed §4.4 receiver path). Used when the
/// `#[serde(skip)]` `closure_function_layouts_by_name` conduit did not cross
/// the wire, so the linker could not populate `closure_function_layouts` for a
/// remote-streamed closure blob. The kinds are the callee blob's hash-covered
/// `frame_descriptor` capture slots — never fabricated from bits. Every wire
/// capture is `Immutable` (the v1 refusal matrix rejected mutable / reference /
/// resource / nested captures upstream), so this is the complete v1 shape.
///
/// The layout's per-capture `FieldKind` (slot width / offset / heap-mask) is
/// determined by each `NativeKind`; `native_kind` metadata is stored verbatim
/// so `release_typed_closure`'s refcount dispatch drops each capture at its true
/// kind. A representative `ConcreteType` per `NativeKind` supplies the field
/// kind to the tested `ClosureLayout::from_capture_types_with_native_kinds`
/// constructor — scalar widths map exactly; every heap / string / decimal kind
/// maps to a pointer-sized field (`FieldKind::Ptr`), which is all the layout
/// needs from the type.
fn rebuild_immutable_closure_layout(
    capture_native_kinds: &[shape_value::NativeKind],
) -> shape_value::v2::closure_layout::ClosureLayout {
    use shape_value::NativeKind;
    use shape_value::v2::ConcreteType as CT;
    use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};

    let capture_types: Vec<CT> = capture_native_kinds
        .iter()
        .map(|nk| match nk {
            NativeKind::Float64 => CT::F64,
            NativeKind::Float32 => CT::F32,
            NativeKind::Int64 => CT::I64,
            NativeKind::Int32 => CT::I32,
            NativeKind::Int16 => CT::I16,
            NativeKind::Int8 => CT::I8,
            NativeKind::UInt64 => CT::U64,
            NativeKind::UInt32 => CT::U32,
            NativeKind::UInt16 => CT::U16,
            NativeKind::UInt8 => CT::U8,
            NativeKind::Bool => CT::Bool,
            NativeKind::Char => CT::Char,
            // Every heap / string / decimal / pointer kind is pointer-sized:
            // `ConcreteType::to_field_kind` maps all non-scalar types to
            // `FieldKind::Ptr`, which is all the layout reads from the type.
            // The true `NativeKind` is preserved via `native_kinds` below so
            // teardown refcount dispatch stays kind-exact.
            _ => CT::Pointer(Box::new(CT::Void)),
        })
        .collect();
    let kinds = vec![CaptureKind::Immutable; capture_native_kinds.len()];
    ClosureLayout::from_capture_types_with_native_kinds(&capture_types, &kinds, capture_native_kinds)
}

fn finish_remote_closure_call(
    vm: &mut crate::executor::VirtualMachine,
    func_id: u16,
    upvalues: &[SerializableVMValue],
    arguments: &[SerializableVMValue],
    store: &SnapshotStore,
    ctx: &mut shape_runtime::context::ExecutionContext,
) -> Result<SerializableVMValue, RemoteCallError> {
    use shape_runtime::snapshot::serializable_to_slot;
    use shape_value::v2::closure_layout::CaptureKind;
    use shape_value::v2::closure_raw::{
        OwnedClosureBlock, alloc_typed_closure, write_capture_raw_u64,
    };
    use shape_value::{KindedSlot, ValueSlot};

    // Callee metadata first (arity, frame_descriptor, name, captures_count).
    // The frame descriptor lays out slots as [captures.. , params.. , locals..]
    // (compile_expr_closure builds params = captures ++ params).
    let (arity, frame_desc, callee_name, captures_count) = {
        let f = vm.program.functions.get(func_id as usize).ok_or_else(|| {
            RemoteCallError::new(
                RemoteErrorKind::FunctionNotFound,
                format!("function_id {func_id} out of range"),
            )
        })?;
        (
            f.arity as usize,
            f.frame_descriptor.clone(),
            f.name.clone(),
            f.captures_count as usize,
        )
    };

    // Resolve the receiver-side ClosureLayout. The linker rebuilds
    // `closure_function_layouts` from the `closure_function_layouts_by_name`
    // conduit — but that conduit is `#[serde(skip)]` (content_addressed.rs), so
    // it does NOT cross the wire: a remote-streamed program never carries it.
    // When it is absent, rebuild an all-Immutable layout from the callee's
    // hash-covered per-capture `NativeKind` track — the leading `captures_count`
    // slots of the callee blob's `frame_descriptor` (ADR-006 §2.7.5.1: every
    // slot kind is proven at blob construction, hash-covered per §4.8). v1 only
    // transfers immutable by-value captures (the refusal matrix in
    // `validate_remote_closure_captures` rejected mutable / reference / resource
    // / nested captures before reaching here), so every wire capture is
    // `Immutable` by construction. No Bool-default, no kind-from-bits: the kinds
    // are the proven frame-descriptor kinds.
    let layout = if let Some(l) = vm
        .program
        .closure_function_layouts
        .get(func_id as usize)
        .and_then(|o| o.clone())
    {
        l
    } else {
        let fd = frame_desc.as_ref().ok_or_else(|| {
            RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                format!(
                    "closure function {func_id} ('{callee_name}') has neither a \
                     registered ClosureLayout nor a frame_descriptor on the \
                     receiver — cannot recover the capture kind track (ADR-006 §2.7.8)"
                ),
            )
        })?;
        if fd.slots.len() < captures_count {
            return Err(RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                format!(
                    "closure '{callee_name}' frame_descriptor has {} slots but \
                     declares {} capture(s) — cannot recover the capture kind track",
                    fd.slots.len(),
                    captures_count,
                ),
            ));
        }
        let capture_native_kinds = &fd.slots[..captures_count];
        std::sync::Arc::new(rebuild_immutable_closure_layout(capture_native_kinds))
    };

    let capture_count = layout.capture_count();
    if capture_count != upvalues.len() {
        return Err(RemoteCallError::new(
            RemoteErrorKind::ArgumentError,
            format!(
                "closure capture count mismatch: receiver layout expects {} but the \
                 request carries {}",
                capture_count,
                upvalues.len(),
            ),
        ));
    }

    // Defend the invariant: after `validate_*` every surviving capture is
    // Immutable. A non-Immutable layout capture reaching here is an
    // inconsistency, not something to Bool-default around.
    for i in 0..capture_count {
        if !matches!(layout.capture_storage_kind(i), CaptureKind::Immutable) {
            return Err(RemoteCallError::new(
                RemoteErrorKind::UnsupportedCapture,
                format!(
                    "closure capture #{i} is not an immutable by-value capture — \
                     only immutable captures cross the wire in v1"
                ),
            ));
        }
    }

    let actual_arity = arity.saturating_sub(capture_count);
    if arguments.len() != actual_arity {
        return Err(RemoteCallError::new(
            RemoteErrorKind::ArgumentError,
            format!(
                "closure '{}' takes {} argument(s) but the request carried {}",
                callee_name,
                actual_arity,
                arguments.len(),
            ),
        ));
    }

    // Per-arg expected kinds: the callee's frame descriptor lays out
    // [captures.. , params.. , locals..]; the actual params start at
    // `capture_count`. ADR-006 §2.7.5.1: every slot kind is proven.
    let arg_kinds: Vec<shape_value::NativeKind> = if let Some(ref fd) = frame_desc {
        if fd.slots.len() < capture_count + actual_arity {
            return Err(RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                format!(
                    "closure '{}' frame_descriptor has {} slots but needs {} \
                     (captures {} + args {})",
                    callee_name,
                    fd.slots.len(),
                    capture_count + actual_arity,
                    capture_count,
                    actual_arity,
                ),
            ));
        }
        fd.slots[capture_count..capture_count + actual_arity].to_vec()
    } else if actual_arity == 0 {
        Vec::new()
    } else {
        return Err(RemoteCallError::new(
            RemoteErrorKind::ArgumentError,
            format!(
                "closure '{}' has no frame_descriptor — cannot derive per-arg \
                 NativeKind for the marshal protocol (ADR-006 §2.7.5.1)",
                callee_name,
            ),
        ));
    };
    let return_kind = frame_desc.as_ref().and_then(|fd| fd.abi_return_kind());

    // Materialize captures into a fresh OwnedClosureBlock at each capture's
    // proven kind (mirrors `restore_call_stack`).
    //
    // SAFETY: `alloc_typed_closure` returns a zeroed block sized for the
    // layout; `write_capture_raw_u64` writes in-bounds for `i < capture_count`;
    // `from_raw` adopts the single owning share, retired when `block` drops at
    // scope exit. On a mid-loop marshal error the partially-written allocation
    // leaks (no double-free) — the same error-path behaviour as the snapshot
    // restore template.
    let ptr = unsafe { alloc_typed_closure(func_id, 0, &layout) };
    for (i, sv) in upvalues.iter().enumerate() {
        let expected = layout.capture_native_kind(i);
        let (bits, _kind) = serializable_to_slot(sv, expected, store).map_err(|e| {
            RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                format!("closure capture {i} marshal failure (expected {expected:?}): {e}"),
            )
        })?;
        // SAFETY: i < capture_count (checked above).
        unsafe { write_capture_raw_u64(ptr, &layout, i, bits) };
    }
    // SAFETY: `ptr` is a live block with all captures initialised.
    let block = unsafe { OwnedClosureBlock::from_raw(ptr as *const u8, layout) };

    // Marshal the actual arguments.
    let mut args: Vec<KindedSlot> = Vec::with_capacity(actual_arity);
    for (idx, sv) in arguments.iter().enumerate() {
        let expected = arg_kinds[idx];
        let (bits, kind) = serializable_to_slot(sv, expected, store).map_err(|e| {
            RemoteCallError::new(
                RemoteErrorKind::ArgumentError,
                format!("closure arg {idx} marshal failure (expected {expected:?}): {e}"),
            )
        })?;
        args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
    }

    // Dispatch through the value-call ABI. `execute_closure` borrows the block
    // (share-neutral: it clones each capture into the frame), so `block` keeps
    // owning its shares and drops them at scope exit.
    let result = vm
        .execute_closure_at_host_boundary(&block, args, Some(ctx))
        .map_err(|e| {
            RemoteCallError::new(
                RemoteErrorKind::RuntimeError,
                format!(
                    "remote closure execution of '{}' failed: {:?}",
                    callee_name, e
                ),
            )
        })?;

    // `result` drops first (retiring its return share), then `block` (retiring
    // the capture shares via the layout's capture-mask walk).
    let serialized = serialize_remote_return_slot(
        vm,
        result,
        return_kind,
        store,
        &format!("closure '{}'", callee_name),
    )?;
    drop(block);
    Ok(serialized)
}

/// Compute a SHA-256 hash of a `BytecodeProgram` for caching.
///
/// Remote VMs can cache programs by this hash, avoiding re-transfer
/// of the same program on repeated calls.
pub fn program_hash(program: &BytecodeProgram) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let bytes =
        rmp_serde::to_vec_named(program).expect("BytecodeProgram serialization should not fail");
    let hash = Sha256::digest(&bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hash);
    out
}

/// Create a minimal stub program containing only metadata (no instructions/constants/functions).
///
/// Used by `build_call_request` and `build_closure_call_request` when content-addressed
/// blobs are available, to reduce payload size.
fn create_stub_program(program: &BytecodeProgram) -> BytecodeProgram {
    let mut stub = BytecodeProgram::default();
    stub.type_schema_registry = program.type_schema_registry.clone();
    // Carry enough content-addressed metadata for program_from_blobs()
    if let Some(ref ca) = program.content_addressed {
        stub.content_addressed = Some(Program {
            entry: ca.entry,
            function_store: std::collections::HashMap::new(),
            top_level_locals_count: ca.top_level_locals_count,
            top_level_local_storage_hints: ca.top_level_local_storage_hints.clone(),
            module_binding_names: ca.module_binding_names.clone(),
            module_binding_storage_hints: ca.module_binding_storage_hints.clone(),
            function_local_storage_hints: ca.function_local_storage_hints.clone(),
            top_level_frame: ca.top_level_frame.clone(),
            top_level_local_concrete_types: ca.top_level_local_concrete_types.clone(),
            function_local_concrete_types: ca.function_local_concrete_types.clone(),
            function_return_concrete_types: ca.function_return_concrete_types.clone(),
            monomorphized_method_call_sites: ca.monomorphized_method_call_sites.clone(),
            value_call_return_concrete_types: ca.value_call_return_concrete_types.clone(),
            operator_trait_dispatch_sites: ca.operator_trait_dispatch_sites.clone(),
            data_schema: ca.data_schema.clone(),
            type_schema_registry: ca.type_schema_registry.clone(),
            trait_method_symbols: ca.trait_method_symbols.clone(),
            foreign_functions: ca.foreign_functions.clone(),
            native_struct_layouts: ca.native_struct_layouts.clone(),
            debug_info: ca.debug_info.clone(),
            closure_function_layouts_by_name: ca.closure_function_layouts_by_name.clone(),
            trait_vtables: ca.trait_vtables.clone(),
            // R8 W8 Cluster A surface-and-stop flag propagation.
            has_imported_const_inline: ca.has_imported_const_inline,
            // R8 W9 B1 W17-marshal-return surface-and-stop flag propagation.
            has_w17_marshal_residual: ca.has_w17_marshal_residual,
            // c4-4B TryUnwrap (`?` operator) surface-and-stop flag propagation.
            has_try_unwrap_residual: ca.has_try_unwrap_residual,
            has_reference_escape_promotion: ca.has_reference_escape_promotion,
            has_null_coalesce_residual: ca.has_null_coalesce_residual,
        });
    }
    // Copy top-level metadata needed by program_from_blobs
    stub.top_level_locals_count = program.top_level_locals_count;
    stub.top_level_local_storage_hints = program.top_level_local_storage_hints.clone();
    stub.module_binding_names = program.module_binding_names.clone();
    stub.module_binding_storage_hints = program.module_binding_storage_hints.clone();
    stub.function_local_storage_hints = program.function_local_storage_hints.clone();
    stub.data_schema = program.data_schema.clone();
    stub.trait_method_symbols = program.trait_method_symbols.clone();
    stub.foreign_functions = program.foreign_functions.clone();
    stub.native_struct_layouts = program.native_struct_layouts.clone();
    stub.debug_info = program.debug_info.clone();
    stub.function_blob_hashes = program.function_blob_hashes.clone();
    stub
}

/// Perform blob negotiation before sending a call request.
///
/// Creates a `BlobNegotiationRequest` with the hashes from the blob set,
/// checks which blobs the remote already has (via the provided cache as a
/// local stand-in), and returns the set of known hashes that can be stripped
/// from the outgoing request.
///
/// In a real transport scenario the `BlobNegotiationRequest` would be sent
/// over the wire and the `BlobNegotiationResponse` received from the remote.
/// Currently this performs the negotiation locally against the provided cache.
///
/// # Example flow
/// ```text
/// 1. Caller builds blob set for function
/// 2. negotiate_blobs() → BlobNegotiationRequest with offered hashes
/// 3. Remote replies with BlobNegotiationResponse (known_hashes)
/// 4. Caller strips known blobs from the request
/// ```
pub fn negotiate_blobs(
    blobs: &[(FunctionHash, FunctionBlob)],
    remote_cache: &RemoteBlobCache,
) -> BlobNegotiationResponse {
    let request = BlobNegotiationRequest {
        offered_hashes: blobs.iter().map(|(h, _)| *h).collect(),
    };
    // TODO: Wire this to actual transport — currently performs negotiation
    // locally against the provided cache. In production, `request` would be
    // serialized, sent over the wire, and the response deserialized.
    handle_negotiation(&request, remote_cache)
}

/// Build a `RemoteCallRequest` for a named function, with blob negotiation.
///
/// Performs a negotiation step against the provided `remote_cache` to discover
/// which blobs the remote already has, then strips those from the request.
/// If `remote_cache` is `None`, sends all blobs (no negotiation).
pub fn build_call_request_with_negotiation(
    program: &BytecodeProgram,
    function_name: &str,
    arguments: Vec<SerializableVMValue>,
    remote_cache: Option<&RemoteBlobCache>,
) -> RemoteCallRequest {
    let mut request = build_call_request(program, function_name, arguments);

    if let (Some(cache), Some(blobs)) = (remote_cache, &mut request.function_blobs) {
        let response = negotiate_blobs(blobs, cache);
        let known_set: std::collections::HashSet<FunctionHash> =
            response.known_hashes.into_iter().collect();
        blobs.retain(|(hash, _)| !known_set.contains(hash));
    }

    request
}

/// Build a `RemoteCallRequest` for a named function.
///
/// Convenience function that handles program hashing and type schema extraction.
/// When the program has content-addressed blobs, automatically computes the
/// minimal transitive closure and attaches it to the request.
pub fn build_call_request(
    program: &BytecodeProgram,
    function_name: &str,
    arguments: Vec<SerializableVMValue>,
) -> RemoteCallRequest {
    let hash = program_hash(program);
    let function_id = program
        .functions
        .iter()
        .position(|f| f.name == function_name)
        .map(|id| id as u16);
    let function_hash = function_id
        .and_then(|fid| {
            program
                .function_blob_hashes
                .get(fid as usize)
                .copied()
                .flatten()
        })
        .or_else(|| {
            program.content_addressed.as_ref().and_then(|ca| {
                let mut matches = ca.function_store.iter().filter_map(|(hash, blob)| {
                    if blob.name == function_name {
                        Some(*hash)
                    } else {
                        None
                    }
                });
                let first = matches.next()?;
                if matches.next().is_some() {
                    None
                } else {
                    Some(first)
                }
            })
        });
    let blobs = function_hash.and_then(|h| build_minimal_blobs_by_hash(program, h));

    // When content-addressed blobs are available, send a minimal stub program
    // instead of the full BytecodeProgram to reduce payload size.
    let request_program = if blobs.is_some() {
        create_stub_program(program)
    } else {
        program.clone()
    };

    RemoteCallRequest {
        call_id: None,
        program: request_program,
        function_name: function_name.to_string(),
        function_id,
        function_hash,
        arguments,
        upvalues: None,
        // Named-function call: no upvalues, hence no capture-kind track.
        upvalue_kinds: None,
        type_schemas: program.type_schema_registry.clone(),
        program_hash: hash,
        function_blobs: blobs,
    }
}

/// Build a `RemoteCallRequest` for a function identified by its **id** — the
/// canonical sender-side entry when the caller already holds the resolved
/// function value (e.g. `@remote`'s `ctx.target`, or a `remote::call` fn-ref).
///
/// Mirrors [`build_call_request`] but keys off the id directly instead of a
/// name lookup, so a name collision can never misroute the call (distributed
/// §4.3-1 canonical identity). Named-function call: no upvalues.
pub fn build_call_request_by_id(
    program: &BytecodeProgram,
    function_id: u16,
    arguments: Vec<SerializableVMValue>,
) -> Result<RemoteCallRequest, String> {
    let function = program
        .functions
        .get(function_id as usize)
        .ok_or_else(|| format!("function id {function_id} out of range"))?;
    let function_name = function.name.clone();
    let function_hash = program
        .function_blob_hashes
        .get(function_id as usize)
        .copied()
        .flatten();
    let blobs = function_hash.and_then(|h| build_minimal_blobs_by_hash(program, h));
    let request_program = if blobs.is_some() {
        create_stub_program(program)
    } else {
        program.clone()
    };
    Ok(RemoteCallRequest {
        call_id: None,
        program: request_program,
        function_name,
        function_id: Some(function_id),
        function_hash,
        arguments,
        upvalues: None,
        upvalue_kinds: None,
        type_schemas: program.type_schema_registry.clone(),
        program_hash: program_hash(program),
        function_blobs: blobs,
    })
}

/// Build a `RemoteCallRequest` for a closure.
///
/// Serializes the closure's captured upvalues **and** their per-capture
/// `NativeKind` track alongside the function call (distributed §4.4). When the
/// closure's function has a matching content-addressed blob, sends the minimal
/// blob set instead of the full program.
///
/// `upvalue_kinds` MUST be lockstep with `upvalues` (equal length, index-
/// aligned) — the sender reads them from the closure's §2.7.8 cell-storage
/// parallel-kind track, never fabricated from raw bits. The receiver cross-
/// checks them against the callee blob's hash-covered `capture_kinds`.
pub fn build_closure_call_request(
    program: &BytecodeProgram,
    function_id: u16,
    arguments: Vec<SerializableVMValue>,
    upvalues: Vec<SerializableVMValue>,
    upvalue_kinds: Vec<shape_value::NativeKind>,
) -> RemoteCallRequest {
    let hash = program_hash(program);

    let function_hash = program
        .function_blob_hashes
        .get(function_id as usize)
        .copied()
        .flatten();
    let blobs = function_hash.and_then(|h| build_minimal_blobs_by_hash(program, h));

    RemoteCallRequest {
        call_id: None,
        program: if blobs.is_some() {
            create_stub_program(program)
        } else {
            program.clone()
        },
        function_name: String::new(),
        function_id: Some(function_id),
        function_hash,
        arguments,
        upvalues: Some(upvalues),
        // Lockstep per-capture kind track (ADR-006 §2.7.7/§2.7.8). Carried
        // explicitly so the receiver never fabricates Bool-default kinds.
        upvalue_kinds: Some(upvalue_kinds),
        type_schemas: program.type_schema_registry.clone(),
        program_hash: hash,
        function_blobs: blobs,
    }
}

/// Build a `RemoteCallRequest` that strips function blobs the remote already has.
///
/// Like `build_call_request`, but takes a set of hashes the remote is known to
/// have cached (from a prior `BlobNegotiationResponse`). Blobs with matching
/// hashes are omitted from `function_blobs`, reducing payload size.
pub fn build_call_request_negotiated(
    program: &BytecodeProgram,
    function_name: &str,
    arguments: Vec<SerializableVMValue>,
    known_hashes: &[FunctionHash],
) -> RemoteCallRequest {
    let mut request = build_call_request(program, function_name, arguments);

    // Strip blobs the remote already has
    if let Some(ref mut blobs) = request.function_blobs {
        let known_set: std::collections::HashSet<FunctionHash> =
            known_hashes.iter().copied().collect();
        blobs.retain(|(hash, _)| !known_set.contains(hash));
    }

    request
}

/// Orchestrate a remote call with a bounded, retry-**once** resupply of missing
/// dependency blobs (distributed §4.3-5). This is the SENDER side of the
/// content-addressed missing-blob protocol: it reacts to the receiver's
/// structured `MissingModuleFunction` event by looking the named hashes up in
/// its own content store, attaching them, and retrying a single time.
///
/// `send` performs one request → response round-trip and is transport-agnostic:
/// the caller supplies the loopback / TCP / in-process plumbing, so this loop is
/// unit-testable without a socket and reusable by the real `remote::call` path.
///
/// Retry policy (distributed §4.3-5 / OQ-8, deliberately narrow):
/// - Retry **only** on `MissingModuleFunction` — it is provably pre-execution,
///   so resupplying and retrying cannot double-execute a side-effecting call.
///   `Timeout` / connection-loss / any other class is returned unchanged (never
///   auto-retried — they may have executed).
/// - Retry **at most once**. A second `MissingModuleFunction` surfaces
///   terminally (the surface layer maps it to `Protocol` — "still missing after
///   resupply"); there is no unbounded resupply chatter.
/// - If the receiver names a hash the sender's own store does not hold (store
///   eviction; impossible for a closure the sender just computed), the call
///   aborts with a defined terminal error — never a hang.
pub fn call_with_resupply<F>(
    sender_program: &BytecodeProgram,
    mut request: RemoteCallRequest,
    mut send: F,
) -> RemoteCallResponse
where
    F: FnMut(&RemoteCallRequest) -> RemoteCallResponse,
{
    let first = send(&request);

    // Only a MissingModuleFunction carrying hashes is retry-eligible.
    let missing = match &first.result {
        Err(e) if matches!(e.kind, RemoteErrorKind::MissingModuleFunction) => {
            match &e.missing_blobs {
                Some(m) if !m.is_empty() => m.clone(),
                // MissingModuleFunction without a hash list: nothing actionable
                // to resupply — surface as-is.
                _ => return first,
            }
        }
        _ => return first,
    };

    // Look each missing hash up in the sender's own content store.
    let store = match sender_program.content_addressed.as_ref() {
        Some(ca) => &ca.function_store,
        None => {
            return RemoteCallResponse {
                result: Err(RemoteCallError::missing_module_function(
                    "receiver reported missing blobs but the sender has no \
                     content-addressed store to resupply from"
                        .to_string(),
                    missing,
                )),
            };
        }
    };
    let mut resupply: Vec<(FunctionHash, FunctionBlob)> = Vec::with_capacity(missing.len());
    for h in &missing {
        match store.get(h) {
            Some(blob) => resupply.push((*h, blob.clone())),
            None => {
                // §4.3-5 terminal: the sender cannot supply a hash it lacks.
                return RemoteCallResponse {
                    result: Err(RemoteCallError::missing_module_function(
                        format!(
                            "receiver is missing blob {} and the sender cannot \
                             resupply it",
                            short_hash(h),
                        ),
                        vec![*h],
                    )),
                };
            }
        }
    }

    // Attach the resupply blobs (dedup against any already present) and retry.
    match request.function_blobs.as_mut() {
        Some(existing) => {
            let have: std::collections::HashSet<FunctionHash> =
                existing.iter().map(|(h, _)| *h).collect();
            for (h, b) in resupply {
                if !have.contains(&h) {
                    existing.push((h, b));
                }
            }
        }
        None => request.function_blobs = Some(resupply),
    }

    let second = send(&request);

    // Bounded loop: a second missing-blob failure is terminal. Reword so the
    // surface layer's message is truthful about the exhausted retry (the kind
    // stays MissingModuleFunction → surface `Protocol`).
    if let Err(e) = &second.result {
        if matches!(e.kind, RemoteErrorKind::MissingModuleFunction) {
            let still = e
                .missing_blobs
                .as_ref()
                .map(|m| m.len())
                .unwrap_or_default();
            return RemoteCallResponse {
                result: Err(RemoteCallError::missing_module_function(
                    format!("receiver still missing {still} blob(s) after resupply"),
                    e.missing_blobs.clone().unwrap_or_default(),
                )),
            };
        }
    }
    second
}

/// Handle a blob negotiation request on the server side.
///
/// Returns the subset of offered hashes that are present in the cache.
pub fn handle_negotiation(
    request: &BlobNegotiationRequest,
    cache: &RemoteBlobCache,
) -> BlobNegotiationResponse {
    BlobNegotiationResponse {
        known_hashes: cache.filter_known(&request.offered_hashes),
    }
}

// ---------------------------------------------------------------------------
// Wire message dispatch (V1 + V2 handlers)
// ---------------------------------------------------------------------------

/// Handle a `WireMessage` by dispatching to the appropriate handler.
///
/// V1 messages (BlobNegotiation, Call, CallResponse, Sidecar) are fully handled.
/// V2 messages (Execute, Validate, Auth, Ping, file/project operations) return
/// stub error responses until the execution server is implemented.
pub fn handle_wire_message(
    msg: WireMessage,
    store: &SnapshotStore,
    cache: &mut RemoteBlobCache,
) -> WireMessage {
    match msg {
        WireMessage::BlobNegotiation(req) => {
            let response = handle_negotiation(&req, cache);
            WireMessage::BlobNegotiationReply(response)
        }
        WireMessage::BlobNegotiationReply(_) => {
            // Client-side message — server should not receive this.
            // Return an error wrapped in an ExecuteResponse as a generic error channel.
            WireMessage::ExecuteResponse(ExecuteResponse {
                request_id: 0,
                success: false,
                value: WireValue::Null,
                stdout: None,
                error: Some("Unexpected BlobNegotiationReply on server side".to_string()),
                content_terminal: None,
                content_html: None,
                diagnostics: vec![],
                metrics: None,
                print_output: None,
            })
        }
        WireMessage::Call(req) => {
            // Cache any incoming blobs for future negotiation
            if let Some(ref blobs) = req.function_blobs {
                cache.insert_blobs(blobs);
            }
            // WF-1D: fail closed — this legacy V1 dispatch has no server
            // sandbox context, so grant nothing (pure). The production serve
            // path threads the operator's derived grant via `handle_call`.
            let response = execute_remote_call(req, store, &shape_abi_v1::PermissionSet::pure());
            WireMessage::CallResponse(response)
        }
        WireMessage::CallResponse(_) => {
            // Client-side message — server should not receive this.
            WireMessage::ExecuteResponse(ExecuteResponse {
                request_id: 0,
                success: false,
                value: WireValue::Null,
                stdout: None,
                error: Some("Unexpected CallResponse on server side".to_string()),
                content_terminal: None,
                content_html: None,
                diagnostics: vec![],
                metrics: None,
                print_output: None,
            })
        }
        WireMessage::CancelCall(req) => WireMessage::CancelCallResponse(RemoteCancelResponse {
            call_id: req.call_id,
            outcome: RemoteCancelOutcome::UnknownCall,
            message: "This in-process wire handler has no serve-side call queue registry"
                .to_string(),
        }),
        WireMessage::CancelCallResponse(_) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id: 0,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some("Unexpected CancelCallResponse on server side".to_string()),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![],
            metrics: None,
            print_output: None,
        }),
        WireMessage::Sidecar(_sidecar) => {
            // Sidecars are buffered by the transport layer and reassembled
            // before the Call message is dispatched. If we receive one here,
            // it means the transport did not buffer it.
            WireMessage::ExecuteResponse(ExecuteResponse {
                request_id: 0,
                success: false,
                value: WireValue::Null,
                stdout: None,
                error: Some("Unexpected standalone Sidecar message".to_string()),
                content_terminal: None,
                content_html: None,
                diagnostics: vec![],
                metrics: None,
                print_output: None,
            })
        }

        // --- V2 message stubs ---
        WireMessage::Execute(req) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id: req.request_id,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some("V2 Execute not yet implemented".to_string()),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![WireDiagnostic {
                severity: "error".to_string(),
                message: "V2 Execute handler not yet implemented".to_string(),
                line: None,
                column: None,
            }],
            metrics: None,
            print_output: None,
        }),
        WireMessage::ExecuteResponse(_) => {
            // Client-side message — should not arrive at server.
            WireMessage::ExecuteResponse(ExecuteResponse {
                request_id: 0,
                success: false,
                value: WireValue::Null,
                stdout: None,
                error: Some("Unexpected ExecuteResponse on server side".to_string()),
                content_terminal: None,
                content_html: None,
                diagnostics: vec![],
                metrics: None,
                print_output: None,
            })
        }
        WireMessage::Validate(req) => WireMessage::ValidateResponse(ValidateResponse {
            request_id: req.request_id,
            success: false,
            diagnostics: vec![WireDiagnostic {
                severity: "error".to_string(),
                message: "V2 Validate handler not yet implemented".to_string(),
                line: None,
                column: None,
            }],
        }),
        WireMessage::ValidateResponse(_) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id: 0,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some("Unexpected ValidateResponse on server side".to_string()),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![],
            metrics: None,
            print_output: None,
        }),
        WireMessage::Auth(_req) => WireMessage::AuthResponse(AuthResponse {
            authenticated: false,
            error: Some("V2 Auth handler not yet implemented".to_string()),
        }),
        WireMessage::AuthResponse(_) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id: 0,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some("Unexpected AuthResponse on server side".to_string()),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![],
            metrics: None,
            print_output: None,
        }),
        WireMessage::ExecuteFile(req) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id: req.request_id,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some("V2 ExecuteFile handler not yet implemented".to_string()),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![WireDiagnostic {
                severity: "error".to_string(),
                message: "V2 ExecuteFile handler not yet implemented".to_string(),
                line: None,
                column: None,
            }],
            metrics: None,
            print_output: None,
        }),
        WireMessage::ExecuteProject(req) => WireMessage::ExecuteResponse(ExecuteResponse {
            request_id: req.request_id,
            success: false,
            value: WireValue::Null,
            stdout: None,
            error: Some("V2 ExecuteProject handler not yet implemented".to_string()),
            content_terminal: None,
            content_html: None,
            diagnostics: vec![WireDiagnostic {
                severity: "error".to_string(),
                message: "V2 ExecuteProject handler not yet implemented".to_string(),
                line: None,
                column: None,
            }],
            metrics: None,
            print_output: None,
        }),
        WireMessage::ValidatePath(req) => WireMessage::ValidateResponse(ValidateResponse {
            request_id: req.request_id,
            success: false,
            diagnostics: vec![WireDiagnostic {
                severity: "error".to_string(),
                message: "V2 ValidatePath handler not yet implemented".to_string(),
                line: None,
                column: None,
            }],
        }),
        WireMessage::Ping(_) => WireMessage::Pong(ServerInfo {
            shape_version: env!("CARGO_PKG_VERSION").to_string(),
            wire_protocol: shape_wire::WIRE_PROTOCOL_V2,
            capabilities: vec![
                "call".to_string(),
                "call-cancel".to_string(),
                "blob-negotiation".to_string(),
                "sidecar".to_string(),
            ],
        }),
        WireMessage::Pong(_) => {
            // Client-side message — should not arrive at server.
            WireMessage::ExecuteResponse(ExecuteResponse {
                request_id: 0,
                success: false,
                value: WireValue::Null,
                stdout: None,
                error: Some("Unexpected Pong on server side".to_string()),
                content_terminal: None,
                content_html: None,
                diagnostics: vec![],
                metrics: None,
                print_output: None,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3B: Sidecar extraction and reassembly
// ---------------------------------------------------------------------------

/// Minimum blob size (in bytes) to extract as a sidecar.
/// Blobs smaller than this are left inline in the serialized payload.
pub const SIDECAR_THRESHOLD: usize = 1024 * 1024; // 1 MB

/// Extract large blobs from serialized arguments into sidecars.
///
/// Walks the `SerializableVMValue` tree recursively. Any `BlobRef` whose
/// backing `ChunkedBlob` exceeds `SIDECAR_THRESHOLD` bytes is replaced
/// with a `SidecarRef` and the raw data is collected into a `BlobSidecar`.
///
/// Returns the extracted sidecars. The `args` are modified in place.
pub fn extract_sidecars(
    args: &mut Vec<SerializableVMValue>,
    store: &SnapshotStore,
) -> Vec<BlobSidecar> {
    let mut sidecars = Vec::new();
    let mut next_id: u32 = 0;
    for arg in args.iter_mut() {
        extract_sidecars_recursive(arg, store, &mut sidecars, &mut next_id);
    }
    sidecars
}

/// Extract the BlobRef from a SerializableVMValue if it carries one (non-mutating read).
fn get_blob_ref(value: &SerializableVMValue) -> Option<&shape_runtime::snapshot::BlobRef> {
    use shape_runtime::snapshot::SerializableVMValue as SV;
    match value {
        SV::DataTable(blob)
        | SV::TypedTable { table: blob, .. }
        | SV::RowView { table: blob, .. }
        | SV::ColumnRef { table: blob, .. }
        | SV::IndexedTable { table: blob, .. } => Some(blob),
        SV::TypedArray { blob, .. } | SV::Matrix { blob, .. } => Some(blob),
        _ => None,
    }
}

fn extract_sidecars_recursive(
    value: &mut SerializableVMValue,
    store: &SnapshotStore,
    sidecars: &mut Vec<BlobSidecar>,
    next_id: &mut u32,
) {
    use shape_runtime::snapshot::SerializableVMValue as SV;

    // First: check if this value carries a blob large enough to extract.
    // Capture metadata (TypedArray len, Matrix rows/cols) before replacing.
    let meta = match &*value {
        SV::TypedArray { len, .. } => (*len as u32, 0u32),
        SV::Matrix { rows, cols, .. } => (*rows, *cols),
        _ => (0, 0),
    };
    // Clone the blob info to avoid borrow conflicts with the later mutation.
    if let Some(blob) = get_blob_ref(value) {
        let blob_kind = blob.kind.clone();
        let blob_hash = blob.hash.clone();
        if let Some(sidecar) = try_extract_blob(blob, store, next_id) {
            let sidecar_id = sidecar.sidecar_id;
            sidecars.push(sidecar);
            *value = SV::SidecarRef {
                sidecar_id,
                blob_kind,
                original_hash: blob_hash,
                meta_a: meta.0,
                meta_b: meta.1,
            };
            return;
        }
    }

    // Recursive descent into containers
    match value {
        SV::Array(items) => {
            for item in items.iter_mut() {
                extract_sidecars_recursive(item, store, sidecars, next_id);
            }
        }
        SV::HashMap { keys, values } => {
            for k in keys.iter_mut() {
                extract_sidecars_recursive(k, store, sidecars, next_id);
            }
            for v in values.iter_mut() {
                extract_sidecars_recursive(v, store, sidecars, next_id);
            }
        }
        SV::TypedObject { slot_data, .. } => {
            for slot in slot_data.iter_mut() {
                extract_sidecars_recursive(slot, store, sidecars, next_id);
            }
        }
        SV::Some(inner) | SV::Ok(inner) | SV::Err(inner) => {
            extract_sidecars_recursive(inner, store, sidecars, next_id);
        }
        SV::TypeAnnotatedValue { value: inner, .. } => {
            extract_sidecars_recursive(inner, store, sidecars, next_id);
        }
        SV::Closure { upvalues, .. } => {
            for uv in upvalues.iter_mut() {
                extract_sidecars_recursive(uv, store, sidecars, next_id);
            }
        }
        SV::Enum(ev) => match &mut ev.payload {
            shape_runtime::snapshot::EnumPayloadSnapshot::Unit => {}
            shape_runtime::snapshot::EnumPayloadSnapshot::Tuple(items) => {
                for item in items.iter_mut() {
                    extract_sidecars_recursive(item, store, sidecars, next_id);
                }
            }
            shape_runtime::snapshot::EnumPayloadSnapshot::Struct(fields) => {
                for (_, v) in fields.iter_mut() {
                    extract_sidecars_recursive(v, store, sidecars, next_id);
                }
            }
        },
        SV::PrintResult(pr) => {
            for span in pr.spans.iter_mut() {
                if let shape_runtime::snapshot::PrintSpanSnapshot::Value {
                    raw_value,
                    format_params,
                    ..
                } = span
                {
                    extract_sidecars_recursive(raw_value, store, sidecars, next_id);
                    for (_, v) in format_params.iter_mut() {
                        extract_sidecars_recursive(v, store, sidecars, next_id);
                    }
                }
            }
        }
        SV::SimulationCall { params, .. } => {
            for (_, v) in params.iter_mut() {
                extract_sidecars_recursive(v, store, sidecars, next_id);
            }
        }
        SV::FunctionRef { closure, .. } => {
            if let Some(c) = closure {
                extract_sidecars_recursive(c, store, sidecars, next_id);
            }
        }
        SV::Range { start, end, .. } => {
            if let Some(s) = start {
                extract_sidecars_recursive(s, store, sidecars, next_id);
            }
            if let Some(e) = end {
                extract_sidecars_recursive(e, store, sidecars, next_id);
            }
        }

        // Leaf types and blob carriers (handled above) — nothing more to do
        _ => {}
    }
}

/// Try to extract a BlobRef's data as a sidecar if it exceeds the threshold.
fn try_extract_blob(
    blob: &shape_runtime::snapshot::BlobRef,
    store: &SnapshotStore,
    next_id: &mut u32,
) -> Option<BlobSidecar> {
    // Load the ChunkedBlob metadata to check total size
    let chunked: shape_runtime::snapshot::ChunkedBlob = store.get_struct(&blob.hash).ok()?;
    if chunked.total_len < SIDECAR_THRESHOLD {
        return None;
    }

    // Load the raw data
    let data = shape_runtime::snapshot::load_chunked_bytes(&chunked, store).ok()?;
    let sidecar_id = *next_id;
    *next_id += 1;

    Some(BlobSidecar { sidecar_id, data })
}

/// Reassemble sidecars back into the serialized payload.
///
/// Walks the `SerializableVMValue` tree and replaces `SidecarRef` variants
/// with the original `BlobRef`, storing the sidecar data back into the
/// snapshot store.
pub fn reassemble_sidecars(
    args: &mut Vec<SerializableVMValue>,
    sidecars: &std::collections::HashMap<u32, BlobSidecar>,
    store: &SnapshotStore,
) -> anyhow::Result<()> {
    for arg in args.iter_mut() {
        reassemble_recursive(arg, sidecars, store)?;
    }
    Ok(())
}

fn reassemble_recursive(
    value: &mut SerializableVMValue,
    sidecars: &std::collections::HashMap<u32, BlobSidecar>,
    store: &SnapshotStore,
) -> anyhow::Result<()> {
    use shape_runtime::snapshot::{BlobRef, SerializableVMValue as SV};

    match value {
        SV::SidecarRef {
            sidecar_id,
            blob_kind,
            original_hash: _,
            meta_a,
            meta_b,
        } => {
            let sidecar = sidecars
                .get(sidecar_id)
                .ok_or_else(|| anyhow::anyhow!("missing sidecar with id {}", sidecar_id))?;
            let meta_a = *meta_a;
            let meta_b = *meta_b;

            // Store the sidecar data back into the snapshot store as chunked bytes,
            // then wrap in a ChunkedBlob struct and store that.
            let chunked = shape_runtime::snapshot::store_chunked_bytes(&sidecar.data, store)?;
            let hash = store.put_struct(&chunked)?;

            let blob = BlobRef {
                hash,
                kind: blob_kind.clone(),
            };
            *value = match blob_kind {
                shape_runtime::snapshot::BlobKind::DataTable => SV::DataTable(blob),
                shape_runtime::snapshot::BlobKind::TypedArray(ek) => SV::TypedArray {
                    element_kind: *ek,
                    blob,
                    len: meta_a as usize,
                },
                shape_runtime::snapshot::BlobKind::Matrix => SV::Matrix {
                    blob,
                    rows: meta_a,
                    cols: meta_b,
                },
            };
        }

        // Recursive descent (same structure as extract)
        SV::Array(items) => {
            for item in items.iter_mut() {
                reassemble_recursive(item, sidecars, store)?;
            }
        }
        SV::HashMap { keys, values } => {
            for k in keys.iter_mut() {
                reassemble_recursive(k, sidecars, store)?;
            }
            for v in values.iter_mut() {
                reassemble_recursive(v, sidecars, store)?;
            }
        }
        SV::TypedObject { slot_data, .. } => {
            for slot in slot_data.iter_mut() {
                reassemble_recursive(slot, sidecars, store)?;
            }
        }
        SV::Some(inner) | SV::Ok(inner) | SV::Err(inner) => {
            reassemble_recursive(inner, sidecars, store)?;
        }
        SV::TypeAnnotatedValue { value: inner, .. } => {
            reassemble_recursive(inner, sidecars, store)?;
        }
        SV::Closure { upvalues, .. } => {
            for uv in upvalues.iter_mut() {
                reassemble_recursive(uv, sidecars, store)?;
            }
        }
        SV::Enum(ev) => match &mut ev.payload {
            shape_runtime::snapshot::EnumPayloadSnapshot::Unit => {}
            shape_runtime::snapshot::EnumPayloadSnapshot::Tuple(items) => {
                for item in items.iter_mut() {
                    reassemble_recursive(item, sidecars, store)?;
                }
            }
            shape_runtime::snapshot::EnumPayloadSnapshot::Struct(fields) => {
                for (_, v) in fields.iter_mut() {
                    reassemble_recursive(v, sidecars, store)?;
                }
            }
        },
        SV::PrintResult(pr) => {
            for span in pr.spans.iter_mut() {
                if let shape_runtime::snapshot::PrintSpanSnapshot::Value {
                    raw_value,
                    format_params,
                    ..
                } = span
                {
                    reassemble_recursive(raw_value, sidecars, store)?;
                    for (_, v) in format_params.iter_mut() {
                        reassemble_recursive(v, sidecars, store)?;
                    }
                }
            }
        }
        SV::SimulationCall { params, .. } => {
            for (_, v) in params.iter_mut() {
                reassemble_recursive(v, sidecars, store)?;
            }
        }
        SV::FunctionRef { closure, .. } => {
            if let Some(c) = closure {
                reassemble_recursive(c, sidecars, store)?;
            }
        }
        SV::Range { start, end, .. } => {
            if let Some(s) = start {
                reassemble_recursive(s, sidecars, store)?;
            }
            if let Some(e) = end {
                reassemble_recursive(e, sidecars, store)?;
            }
        }

        // Leaf types and blob-carrying variants (non-sidecar) — nothing to reassemble
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{FunctionBlob, FunctionHash, Instruction, OpCode, Program};
    use crate::compiler::BytecodeCompiler;
    use shape_abi_v1::PermissionSet;
    use std::collections::HashMap;

    /// Helper: compile Shape source to BytecodeProgram
    fn compile(source: &str) -> BytecodeProgram {
        let program = shape_ast::parser::parse_program(source).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        compiler.compile(&program).expect("compile failed")
    }

    /// Helper: create a temp SnapshotStore
    fn temp_store() -> SnapshotStore {
        let dir = std::env::temp_dir().join(format!("shape_remote_test_{}", std::process::id()));
        SnapshotStore::new(dir).expect("create snapshot store")
    }

    fn mk_hash(tag: u8) -> FunctionHash {
        let mut bytes = [0u8; 32];
        bytes[0] = tag;
        FunctionHash(bytes)
    }

    fn mk_blob(name: &str, hash: FunctionHash, dependencies: Vec<FunctionHash>) -> FunctionBlob {
        FunctionBlob {
            content_hash: hash,
            name: name.to_string(),
            arity: 0,
            param_names: Vec::new(),
            locals_count: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: Vec::new(),
            frame_descriptor: None,
            capture_kinds: Vec::new(),
            capture_names: Vec::new(),
            instructions: vec![
                Instruction::simple(OpCode::PushNull),
                Instruction::simple(OpCode::ReturnValue),
            ],
            constants: Vec::new(),
            strings: Vec::new(),
            required_permissions: PermissionSet::pure(),
            dependencies,
            callee_names: Vec::new(),
            type_schemas: Vec::new(),
            foreign_dependencies: Vec::new(),
            source_map: Vec::new(),
        }
    }

    // The pre-bulldozer end-to-end execute tests
    // (`test_remote_call_simple_function`, `test_remote_call_function_not_found`)
    // drove `execute_remote_call`, which is currently a phase-2c stub
    // (see `execute_inner` body). Re-author them once the kind-threaded
    // `slot_to_serializable` / `serializable_to_slot` round-trip lands
    // (ADR-006 §2.7.4 + addendum); both depend on the snapshot-side
    // rebuild plus a `Vec<KindedSlot>` arg pipeline through the VM
    // entrypoints.

    #[test]
    fn test_program_hash_deterministic() {
        let bytecode = compile("function f(x) { x * 2 }");
        let hash1 = program_hash(&bytecode);
        let hash2 = program_hash(&bytecode);
        assert_eq!(hash1, hash2, "Same program should produce same hash");
    }

    #[test]
    fn test_request_response_serialization_roundtrip() {
        let bytecode = compile("function double(x) { x * 2 }");
        let request =
            build_call_request(&bytecode, "double", vec![SerializableVMValue::Number(21.0)]);

        // Encode → decode roundtrip via MessagePack
        let bytes = shape_wire::encode_message(&request).expect("encode request");
        let decoded: RemoteCallRequest =
            shape_wire::decode_message(&bytes).expect("decode request");

        assert_eq!(decoded.function_name, "double");
        assert_eq!(decoded.arguments.len(), 1);
        assert_eq!(decoded.program_hash, request.program_hash);
    }

    #[test]
    fn test_response_serialization_roundtrip() {
        let response = RemoteCallResponse {
            result: Ok(SerializableVMValue::String("hello".to_string())),
        };

        let bytes = shape_wire::encode_message(&response).expect("encode response");
        let decoded: RemoteCallResponse =
            shape_wire::decode_message(&bytes).expect("decode response");

        match decoded.result {
            Ok(SerializableVMValue::String(s)) => assert_eq!(s, "hello"),
            other => panic!("Expected Ok(String), got {:?}", other),
        }
    }

    #[test]
    fn test_type_schema_registry_roundtrip() {
        use shape_runtime::type_schema::{FieldType, TypeSchemaRegistry};

        let mut registry = TypeSchemaRegistry::new();
        registry.register_type(
            "Point",
            vec![
                ("x".to_string(), FieldType::F64),
                ("y".to_string(), FieldType::F64),
            ],
        );

        let bytes = shape_wire::encode_message(&registry).expect("encode registry");
        let decoded: TypeSchemaRegistry =
            shape_wire::decode_message(&bytes).expect("decode registry");

        assert!(decoded.has_type("Point"));
        let schema = decoded.get("Point").unwrap();
        assert_eq!(schema.field_count(), 2);
        assert_eq!(schema.field_offset("x"), Some(0));
        assert_eq!(schema.field_offset("y"), Some(8));
    }

    #[test]
    fn test_build_minimal_blobs_rejects_ambiguous_function_name() {
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        let blob1 = mk_blob("dup", h1, vec![]);
        let blob2 = mk_blob("dup", h2, vec![]);

        let mut function_store = HashMap::new();
        function_store.insert(h1, blob1.clone());
        function_store.insert(h2, blob2.clone());

        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(Program {
            entry: h1,
            function_store,
            top_level_locals_count: 0,
            top_level_local_storage_hints: Vec::new(),
            module_binding_names: Vec::new(),
            module_binding_storage_hints: Vec::new(),
            function_local_storage_hints: Vec::new(),
            top_level_frame: None,
            top_level_local_concrete_types: Vec::new(),
            function_local_concrete_types: Vec::new(),
            function_return_concrete_types: Vec::new(),
            monomorphized_method_call_sites: HashMap::new(),
            value_call_return_concrete_types: HashMap::new(),
            operator_trait_dispatch_sites: HashMap::new(),
            data_schema: None,
            type_schema_registry: shape_runtime::type_schema::TypeSchemaRegistry::new(),
            trait_method_symbols: HashMap::new(),
            foreign_functions: Vec::new(),
            native_struct_layouts: Vec::new(),
            debug_info: crate::bytecode::DebugInfo::new("<test>".to_string()),
            closure_function_layouts_by_name: HashMap::new(),
            trait_vtables: HashMap::new(),
            has_imported_const_inline: false,
            has_w17_marshal_residual: false,
            has_try_unwrap_residual: false,
            has_reference_escape_promotion: false,
            has_null_coalesce_residual: false,
        });

        assert!(
            build_minimal_blobs(&program, "dup").is_none(),
            "name-based selection must reject ambiguous function names"
        );

        let by_hash = build_minimal_blobs_by_hash(&program, h2)
            .expect("hash-based selection should work with duplicate names");
        assert_eq!(by_hash.len(), 1);
        assert_eq!(by_hash[0].0, h2);
        assert_eq!(by_hash[0].1.name, "dup");
    }

    #[test]
    fn test_program_from_blobs_by_hash_requires_entry_blob() {
        let h1 = mk_hash(1);
        let h_missing = mk_hash(9);
        let blob = mk_blob("f", h1, vec![]);
        let source = BytecodeProgram::default();

        let reconstructed = program_from_blobs_by_hash(vec![(h1, blob)], h_missing, &source);
        assert!(
            reconstructed.is_none(),
            "reconstruction must fail when the requested entry hash is absent"
        );
    }

    // ---- Phase 2: Blob negotiation tests ----

    #[test]
    fn test_blob_cache_insert_and_get() {
        let mut cache = RemoteBlobCache::new(10);
        let h1 = mk_hash(1);
        let blob1 = mk_blob("f1", h1, vec![]);

        cache.insert(h1, blob1.clone());
        assert_eq!(cache.len(), 1);
        assert!(cache.contains(&h1));
        assert_eq!(cache.get(&h1).unwrap().name, "f1");
    }

    #[test]
    fn test_blob_cache_lru_eviction() {
        let mut cache = RemoteBlobCache::new(2);
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        let h3 = mk_hash(3);

        cache.insert(h1, mk_blob("f1", h1, vec![]));
        cache.insert(h2, mk_blob("f2", h2, vec![]));
        assert_eq!(cache.len(), 2);

        // Insert h3 should evict h1 (least recently used)
        cache.insert(h3, mk_blob("f3", h3, vec![]));
        assert_eq!(cache.len(), 2);
        assert!(!cache.contains(&h1), "h1 should be evicted");
        assert!(cache.contains(&h2));
        assert!(cache.contains(&h3));
    }

    #[test]
    fn test_blob_cache_access_updates_order() {
        let mut cache = RemoteBlobCache::new(2);
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        let h3 = mk_hash(3);

        cache.insert(h1, mk_blob("f1", h1, vec![]));
        cache.insert(h2, mk_blob("f2", h2, vec![]));

        // Access h1 to make it recently used
        cache.get(&h1);

        // Insert h3 should evict h2 (now least recently used)
        cache.insert(h3, mk_blob("f3", h3, vec![]));
        assert!(
            cache.contains(&h1),
            "h1 was accessed, should not be evicted"
        );
        assert!(!cache.contains(&h2), "h2 should be evicted");
        assert!(cache.contains(&h3));
    }

    #[test]
    fn test_blob_cache_filter_known() {
        let mut cache = RemoteBlobCache::new(10);
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        let h3 = mk_hash(3);

        cache.insert(h1, mk_blob("f1", h1, vec![]));
        cache.insert(h3, mk_blob("f3", h3, vec![]));

        let known = cache.filter_known(&[h1, h2, h3]);
        assert_eq!(known.len(), 2);
        assert!(known.contains(&h1));
        assert!(known.contains(&h3));
        assert!(!known.contains(&h2));
    }

    #[test]
    fn test_handle_negotiation() {
        let mut cache = RemoteBlobCache::new(10);
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        cache.insert(h1, mk_blob("f1", h1, vec![]));

        let request = BlobNegotiationRequest {
            offered_hashes: vec![h1, h2],
        };
        let response = handle_negotiation(&request, &cache);
        assert_eq!(response.known_hashes.len(), 1);
        assert!(response.known_hashes.contains(&h1));
    }

    #[test]
    fn test_build_call_request_negotiated_strips_known_blobs() {
        // Create a program with content-addressed blobs
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        let blob1 = mk_blob("entry", h1, vec![h2]);
        let blob2 = mk_blob("helper", h2, vec![]);

        let mut function_store = HashMap::new();
        function_store.insert(h1, blob1.clone());
        function_store.insert(h2, blob2.clone());

        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(Program {
            entry: h1,
            function_store,
            top_level_locals_count: 0,
            top_level_local_storage_hints: Vec::new(),
            module_binding_names: Vec::new(),
            module_binding_storage_hints: Vec::new(),
            function_local_storage_hints: Vec::new(),
            top_level_frame: None,
            top_level_local_concrete_types: Vec::new(),
            function_local_concrete_types: Vec::new(),
            function_return_concrete_types: Vec::new(),
            monomorphized_method_call_sites: HashMap::new(),
            value_call_return_concrete_types: HashMap::new(),
            operator_trait_dispatch_sites: HashMap::new(),
            data_schema: None,
            type_schema_registry: shape_runtime::type_schema::TypeSchemaRegistry::new(),
            trait_method_symbols: HashMap::new(),
            foreign_functions: Vec::new(),
            native_struct_layouts: Vec::new(),
            debug_info: crate::bytecode::DebugInfo::new("<test>".to_string()),
            closure_function_layouts_by_name: HashMap::new(),
            trait_vtables: HashMap::new(),
            has_imported_const_inline: false,
            has_w17_marshal_residual: false,
            has_try_unwrap_residual: false,
            has_reference_escape_promotion: false,
            has_null_coalesce_residual: false,
        });
        program.functions = vec![crate::bytecode::Function {
            name: "entry".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        }];
        program.function_blob_hashes = vec![Some(h1)];

        // First call: no known hashes -> all blobs sent
        let req1 = build_call_request_negotiated(&program, "entry", vec![], &[]);
        let blobs1 = req1.function_blobs.as_ref().unwrap();
        assert_eq!(blobs1.len(), 2, "first call should send all blobs");

        // Second call: h2 is known -> only h1 sent
        let req2 = build_call_request_negotiated(&program, "entry", vec![], &[h2]);
        let blobs2 = req2.function_blobs.as_ref().unwrap();
        assert_eq!(blobs2.len(), 1, "second call should skip known blobs");
        assert_eq!(blobs2[0].0, h1);
    }

    #[test]
    fn test_wire_message_serialization_roundtrip() {
        let msg = WireMessage::BlobNegotiation(BlobNegotiationRequest {
            offered_hashes: vec![mk_hash(1), mk_hash(2)],
        });
        let bytes = shape_wire::encode_message(&msg).expect("encode WireMessage");
        let decoded: WireMessage = shape_wire::decode_message(&bytes).expect("decode WireMessage");
        match decoded {
            WireMessage::BlobNegotiation(req) => {
                assert_eq!(req.offered_hashes.len(), 2);
            }
            _ => panic!("Expected BlobNegotiation"),
        }
    }

    // ---- V2 execution server message tests ----

    #[test]
    fn test_execute_request_roundtrip() {
        let msg = WireMessage::Execute(ExecuteRequest {
            code: "fn main() { 42 }".to_string(),
            request_id: 7,
        });
        let bytes = shape_wire::encode_message(&msg).expect("encode Execute");
        let decoded: WireMessage = shape_wire::decode_message(&bytes).expect("decode Execute");
        match decoded {
            WireMessage::Execute(req) => {
                assert_eq!(req.code, "fn main() { 42 }");
                assert_eq!(req.request_id, 7);
            }
            _ => panic!("Expected Execute"),
        }
    }

    #[test]
    fn test_execute_response_roundtrip() {
        let msg = WireMessage::ExecuteResponse(ExecuteResponse {
            request_id: 7,
            success: true,
            value: WireValue::Number(42.0),
            stdout: Some("hello\n".to_string()),
            error: None,
            content_terminal: None,
            content_html: None,
            diagnostics: vec![WireDiagnostic {
                severity: "warning".to_string(),
                message: "unused variable".to_string(),
                line: Some(1),
                column: Some(5),
            }],
            metrics: Some(ExecutionMetrics {
                instructions_executed: 100,
                wall_time_ms: 3,
                memory_bytes_peak: 4096,
            }),
            print_output: None,
        });
        let bytes = shape_wire::encode_message(&msg).expect("encode ExecuteResponse");
        let decoded: WireMessage =
            shape_wire::decode_message(&bytes).expect("decode ExecuteResponse");
        match decoded {
            WireMessage::ExecuteResponse(resp) => {
                assert_eq!(resp.request_id, 7);
                assert!(resp.success);
                assert!(matches!(resp.value, WireValue::Number(n) if n == 42.0));
                assert_eq!(resp.stdout.as_deref(), Some("hello\n"));
                assert!(resp.error.is_none());
                assert_eq!(resp.diagnostics.len(), 1);
                assert_eq!(resp.diagnostics[0].severity, "warning");
                assert_eq!(resp.diagnostics[0].line, Some(1));
                let m = resp.metrics.unwrap();
                assert_eq!(m.instructions_executed, 100);
                assert_eq!(m.wall_time_ms, 3);
            }
            _ => panic!("Expected ExecuteResponse"),
        }
    }

    #[test]
    fn test_ping_pong_roundtrip() {
        let ping = WireMessage::Ping(PingRequest {});
        let bytes = shape_wire::encode_message(&ping).expect("encode Ping");
        let decoded: WireMessage = shape_wire::decode_message(&bytes).expect("decode Ping");
        assert!(matches!(decoded, WireMessage::Ping(_)));

        let pong = WireMessage::Pong(ServerInfo {
            shape_version: "0.1.3".to_string(),
            wire_protocol: 2,
            capabilities: vec!["execute".to_string(), "validate".to_string()],
        });
        let bytes = shape_wire::encode_message(&pong).expect("encode Pong");
        let decoded: WireMessage = shape_wire::decode_message(&bytes).expect("decode Pong");
        match decoded {
            WireMessage::Pong(info) => {
                assert_eq!(info.shape_version, "0.1.3");
                assert_eq!(info.wire_protocol, 2);
                assert_eq!(info.capabilities.len(), 2);
            }
            _ => panic!("Expected Pong"),
        }
    }

    #[test]
    fn test_auth_roundtrip() {
        let msg = WireMessage::Auth(AuthRequest {
            token: "secret-token".to_string(),
        });
        let bytes = shape_wire::encode_message(&msg).expect("encode Auth");
        let decoded: WireMessage = shape_wire::decode_message(&bytes).expect("decode Auth");
        match decoded {
            WireMessage::Auth(req) => assert_eq!(req.token, "secret-token"),
            _ => panic!("Expected Auth"),
        }

        let resp = WireMessage::AuthResponse(AuthResponse {
            authenticated: true,
            error: None,
        });
        let bytes = shape_wire::encode_message(&resp).expect("encode AuthResponse");
        let decoded: WireMessage = shape_wire::decode_message(&bytes).expect("decode AuthResponse");
        match decoded {
            WireMessage::AuthResponse(r) => {
                assert!(r.authenticated);
                assert!(r.error.is_none());
            }
            _ => panic!("Expected AuthResponse"),
        }
    }

    #[test]
    fn test_validate_roundtrip() {
        let msg = WireMessage::Validate(ValidateRequest {
            code: "let x = 1".to_string(),
            request_id: 99,
        });
        let bytes = shape_wire::encode_message(&msg).expect("encode Validate");
        let decoded: WireMessage = shape_wire::decode_message(&bytes).expect("decode Validate");
        match decoded {
            WireMessage::Validate(req) => {
                assert_eq!(req.code, "let x = 1");
                assert_eq!(req.request_id, 99);
            }
            _ => panic!("Expected Validate"),
        }

        let resp = WireMessage::ValidateResponse(ValidateResponse {
            request_id: 99,
            success: false,
            diagnostics: vec![WireDiagnostic {
                severity: "error".to_string(),
                message: "parse error".to_string(),
                line: None,
                column: None,
            }],
        });
        let bytes = shape_wire::encode_message(&resp).expect("encode ValidateResponse");
        let decoded: WireMessage =
            shape_wire::decode_message(&bytes).expect("decode ValidateResponse");
        match decoded {
            WireMessage::ValidateResponse(r) => {
                assert_eq!(r.request_id, 99);
                assert!(!r.success);
                assert_eq!(r.diagnostics.len(), 1);
            }
            _ => panic!("Expected ValidateResponse"),
        }
    }

    #[test]
    fn test_ping_framing_roundtrip() {
        use shape_wire::transport::framing::{decode_framed, encode_framed};

        let ping = WireMessage::Ping(PingRequest {});
        let mp = shape_wire::encode_message(&ping).expect("encode Ping");
        eprintln!("Ping msgpack bytes ({} bytes): {:02x?}", mp.len(), &mp);

        let framed = encode_framed(&mp);
        eprintln!("Framed bytes ({} bytes): {:02x?}", framed.len(), &framed);

        let decompressed = decode_framed(&framed).expect("decode_framed");
        assert_eq!(mp, decompressed, "framing roundtrip should preserve bytes");

        let decoded: WireMessage =
            shape_wire::decode_message(&decompressed).expect("decode Ping after framing");
        assert!(matches!(decoded, WireMessage::Ping(_)));
    }

    #[test]
    fn test_execute_framing_roundtrip() {
        use shape_wire::transport::framing::{decode_framed, encode_framed};

        let exec = WireMessage::Execute(ExecuteRequest {
            code: "42".to_string(),
            request_id: 1,
        });
        let mp = shape_wire::encode_message(&exec).expect("encode Execute");
        eprintln!("Execute msgpack bytes ({} bytes): {:02x?}", mp.len(), &mp);

        let framed = encode_framed(&mp);
        let decompressed = decode_framed(&framed).expect("decode_framed");
        let decoded: WireMessage =
            shape_wire::decode_message(&decompressed).expect("decode Execute after framing");
        match decoded {
            WireMessage::Execute(req) => {
                assert_eq!(req.code, "42");
                assert_eq!(req.request_id, 1);
            }
            _ => panic!("Expected Execute"),
        }
    }

    // ---- Phase 3B: Sidecar extraction tests ----

    #[test]
    fn test_extract_sidecars_no_large_blobs() {
        let store = temp_store();
        let mut args = vec![
            SerializableVMValue::Int(42),
            SerializableVMValue::String("hello".to_string()),
            SerializableVMValue::Array(vec![
                SerializableVMValue::Number(1.0),
                SerializableVMValue::Number(2.0),
            ]),
        ];
        let sidecars = extract_sidecars(&mut args, &store);
        assert!(sidecars.is_empty(), "no large blobs → no sidecars");
        // Args should be unchanged
        assert!(matches!(args[0], SerializableVMValue::Int(42)));
    }

    // Sidecar extraction/reassembly tests that constructed input via the
    // deleted `ValueWord::from_float_array` + `nanboxed_to_serializable`
    // pair (`test_extract_sidecars_large_typed_array`,
    // `test_reassemble_sidecars_roundtrip`,
    // `test_extract_sidecars_nested_in_array`) belong to the Phase-2c
    // typed-module-exports rebuild. Re-author once the kind-threaded
    // `slot_to_serializable` round-trip lands. Pure
    // `SerializableVMValue`-shaped sidecar coverage is preserved below
    // (`test_extract_sidecars_no_large_blobs`,
    // `test_sidecar_ref_serialization_roundtrip`) — those exercise
    // `extract_sidecars` / `reassemble_sidecars` without crossing the
    // slot boundary.

    #[test]
    fn test_sidecar_ref_serialization_roundtrip() {
        use shape_runtime::hashing::HashDigest;
        use shape_runtime::snapshot::{BlobKind, TypedArrayElementKind};

        let value = SerializableVMValue::SidecarRef {
            sidecar_id: 7,
            blob_kind: BlobKind::TypedArray(TypedArrayElementKind::F64),
            original_hash: HashDigest::from_hex("abc123"),
            meta_a: 1000,
            meta_b: 0,
        };

        let bytes = shape_wire::encode_message(&value).expect("encode SidecarRef");
        let decoded: SerializableVMValue =
            shape_wire::decode_message(&bytes).expect("decode SidecarRef");
        match decoded {
            SerializableVMValue::SidecarRef { sidecar_id, .. } => {
                assert_eq!(sidecar_id, 7);
            }
            _ => panic!("Expected SidecarRef"),
        }
    }

    // ---- Blob negotiation integration tests ----

    #[test]
    fn test_negotiate_blobs_returns_known_hashes() {
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        let h3 = mk_hash(3);

        let mut cache = RemoteBlobCache::new(10);
        cache.insert(h1, mk_blob("f1", h1, vec![]));
        cache.insert(h3, mk_blob("f3", h3, vec![]));

        let blobs = vec![
            (h1, mk_blob("f1", h1, vec![])),
            (h2, mk_blob("f2", h2, vec![])),
            (h3, mk_blob("f3", h3, vec![])),
        ];
        let response = negotiate_blobs(&blobs, &cache);
        assert_eq!(response.known_hashes.len(), 2);
        assert!(response.known_hashes.contains(&h1));
        assert!(response.known_hashes.contains(&h3));
        assert!(!response.known_hashes.contains(&h2));
    }

    #[test]
    fn test_build_call_request_with_negotiation_strips_known() {
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        let blob1 = mk_blob("entry", h1, vec![h2]);
        let blob2 = mk_blob("helper", h2, vec![]);

        let mut function_store = HashMap::new();
        function_store.insert(h1, blob1.clone());
        function_store.insert(h2, blob2.clone());

        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(Program {
            entry: h1,
            function_store,
            top_level_locals_count: 0,
            top_level_local_storage_hints: Vec::new(),
            module_binding_names: Vec::new(),
            module_binding_storage_hints: Vec::new(),
            function_local_storage_hints: Vec::new(),
            top_level_frame: None,
            top_level_local_concrete_types: Vec::new(),
            function_local_concrete_types: Vec::new(),
            function_return_concrete_types: Vec::new(),
            monomorphized_method_call_sites: HashMap::new(),
            value_call_return_concrete_types: HashMap::new(),
            operator_trait_dispatch_sites: HashMap::new(),
            data_schema: None,
            type_schema_registry: shape_runtime::type_schema::TypeSchemaRegistry::new(),
            trait_method_symbols: HashMap::new(),
            foreign_functions: Vec::new(),
            native_struct_layouts: Vec::new(),
            debug_info: crate::bytecode::DebugInfo::new("<test>".to_string()),
            closure_function_layouts_by_name: HashMap::new(),
            trait_vtables: HashMap::new(),
            has_imported_const_inline: false,
            has_w17_marshal_residual: false,
            has_try_unwrap_residual: false,
            has_reference_escape_promotion: false,
            has_null_coalesce_residual: false,
        });
        program.functions = vec![crate::bytecode::Function {
            name: "entry".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        }];
        program.function_blob_hashes = vec![Some(h1)];

        // Cache has h2 -> negotiation should strip it
        let mut cache = RemoteBlobCache::new(10);
        cache.insert(h2, blob2.clone());

        let req = build_call_request_with_negotiation(&program, "entry", vec![], Some(&cache));
        let blobs = req.function_blobs.as_ref().unwrap();
        assert_eq!(blobs.len(), 1, "should strip known blob h2");
        assert_eq!(blobs[0].0, h1, "only h1 should remain");
    }

    #[test]
    fn test_build_call_request_with_negotiation_no_cache() {
        let h1 = mk_hash(1);
        let blob1 = mk_blob("entry", h1, vec![]);

        let mut function_store = HashMap::new();
        function_store.insert(h1, blob1.clone());

        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(Program {
            entry: h1,
            function_store,
            top_level_locals_count: 0,
            top_level_local_storage_hints: Vec::new(),
            module_binding_names: Vec::new(),
            module_binding_storage_hints: Vec::new(),
            function_local_storage_hints: Vec::new(),
            top_level_frame: None,
            top_level_local_concrete_types: Vec::new(),
            function_local_concrete_types: Vec::new(),
            function_return_concrete_types: Vec::new(),
            monomorphized_method_call_sites: HashMap::new(),
            value_call_return_concrete_types: HashMap::new(),
            operator_trait_dispatch_sites: HashMap::new(),
            data_schema: None,
            type_schema_registry: shape_runtime::type_schema::TypeSchemaRegistry::new(),
            trait_method_symbols: HashMap::new(),
            foreign_functions: Vec::new(),
            native_struct_layouts: Vec::new(),
            debug_info: crate::bytecode::DebugInfo::new("<test>".to_string()),
            closure_function_layouts_by_name: HashMap::new(),
            trait_vtables: HashMap::new(),
            has_imported_const_inline: false,
            has_w17_marshal_residual: false,
            has_try_unwrap_residual: false,
            has_reference_escape_promotion: false,
            has_null_coalesce_residual: false,
        });
        program.functions = vec![crate::bytecode::Function {
            name: "entry".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        }];
        program.function_blob_hashes = vec![Some(h1)];

        // No cache -> all blobs sent
        let req = build_call_request_with_negotiation(&program, "entry", vec![], None);
        let blobs = req.function_blobs.as_ref().unwrap();
        assert_eq!(blobs.len(), 1, "all blobs should be sent when no cache");
    }

    // ---- V2 handler stub tests ----

    #[test]
    fn test_handle_wire_message_ping_returns_pong() {
        let store = temp_store();
        let mut cache = RemoteBlobCache::default_cache();
        let msg = WireMessage::Ping(PingRequest {});
        let response = handle_wire_message(msg, &store, &mut cache);
        match response {
            WireMessage::Pong(info) => {
                assert_eq!(info.wire_protocol, shape_wire::WIRE_PROTOCOL_V2);
                assert!(info.capabilities.contains(&"call".to_string()));
                assert!(info.capabilities.contains(&"blob-negotiation".to_string()));
            }
            _ => panic!("Expected Pong response"),
        }
    }

    #[test]
    fn test_handle_wire_message_execute_returns_v2_stub() {
        let store = temp_store();
        let mut cache = RemoteBlobCache::default_cache();
        let msg = WireMessage::Execute(ExecuteRequest {
            code: "42".to_string(),
            request_id: 5,
        });
        let response = handle_wire_message(msg, &store, &mut cache);
        match response {
            WireMessage::ExecuteResponse(resp) => {
                assert_eq!(resp.request_id, 5);
                assert!(!resp.success);
                assert!(resp.error.as_ref().unwrap().contains("not yet implemented"));
            }
            _ => panic!("Expected ExecuteResponse"),
        }
    }

    #[test]
    fn test_handle_wire_message_validate_returns_v2_stub() {
        let store = temp_store();
        let mut cache = RemoteBlobCache::default_cache();
        let msg = WireMessage::Validate(ValidateRequest {
            code: "let x = 1".to_string(),
            request_id: 10,
        });
        let response = handle_wire_message(msg, &store, &mut cache);
        match response {
            WireMessage::ValidateResponse(resp) => {
                assert_eq!(resp.request_id, 10);
                assert!(!resp.success);
                assert!(resp.diagnostics[0].message.contains("not yet implemented"));
            }
            _ => panic!("Expected ValidateResponse"),
        }
    }

    #[test]
    fn test_handle_wire_message_auth_returns_v2_stub() {
        let store = temp_store();
        let mut cache = RemoteBlobCache::default_cache();
        let msg = WireMessage::Auth(AuthRequest {
            token: "test".to_string(),
        });
        let response = handle_wire_message(msg, &store, &mut cache);
        match response {
            WireMessage::AuthResponse(resp) => {
                assert!(!resp.authenticated);
                assert!(resp.error.as_ref().unwrap().contains("not yet implemented"));
            }
            _ => panic!("Expected AuthResponse"),
        }
    }

    #[test]
    fn test_handle_wire_message_blob_negotiation() {
        let store = temp_store();
        let mut cache = RemoteBlobCache::new(10);
        let h1 = mk_hash(1);
        let h2 = mk_hash(2);
        cache.insert(h1, mk_blob("f1", h1, vec![]));

        let msg = WireMessage::BlobNegotiation(BlobNegotiationRequest {
            offered_hashes: vec![h1, h2],
        });
        let response = handle_wire_message(msg, &store, &mut cache);
        match response {
            WireMessage::BlobNegotiationReply(resp) => {
                assert_eq!(resp.known_hashes.len(), 1);
                assert!(resp.known_hashes.contains(&h1));
            }
            _ => panic!("Expected BlobNegotiationReply"),
        }
    }

    // Track A.2B: a closure payload serialised through the slot/serializable
    // round-trip and replayed through the receiver's
    // `closure_function_layouts` slice. The pre-bulldozer test
    // (`test_a2b_closure_arg_roundtrip_with_layouts`) constructed the
    // closure via deleted ValueWord constructors
    // (`from_f64` + `into_raw_bits` + `from_heap_value(HeapValue::ClosureRaw(_))`)
    // and round-tripped it through the deleted
    // `nanboxed_to_serializable` / `serializable_to_nanboxed_with_layouts`
    // pair. Re-author against the kind-threaded slot pipeline once the
    // Phase-2c snapshot rebuild lands (ADR-006 §2.7.4 + addendum). The
    // wire schema (`function_id: u32`, `type_id: u32`, `upvalues: Vec<…>`)
    // is preserved verbatim.

    // -----------------------------------------------------------------------
    // WF-2C: receiver-owned permission enforcement + blob hash verification +
    // structured missing-blob signalling (distributed §4.3 / §4.6) +
    // sender-local transport pre-send/post-send classification (§4.9).
    // -----------------------------------------------------------------------

    /// Build a content-addressed `Program` around a blob store for the
    /// receiver-enforcement tests.
    fn mk_ca_program(entry: FunctionHash, store: HashMap<FunctionHash, FunctionBlob>) -> Program {
        Program {
            entry,
            function_store: store,
            top_level_locals_count: 0,
            top_level_local_storage_hints: Vec::new(),
            module_binding_names: Vec::new(),
            module_binding_storage_hints: Vec::new(),
            function_local_storage_hints: Vec::new(),
            top_level_frame: None,
            top_level_local_concrete_types: Vec::new(),
            function_local_concrete_types: Vec::new(),
            function_return_concrete_types: Vec::new(),
            monomorphized_method_call_sites: HashMap::new(),
            value_call_return_concrete_types: HashMap::new(),
            operator_trait_dispatch_sites: HashMap::new(),
            data_schema: None,
            type_schema_registry: shape_runtime::type_schema::TypeSchemaRegistry::new(),
            trait_method_symbols: HashMap::new(),
            foreign_functions: Vec::new(),
            native_struct_layouts: Vec::new(),
            debug_info: crate::bytecode::DebugInfo::new("<test>".to_string()),
            closure_function_layouts_by_name: HashMap::new(),
            trait_vtables: HashMap::new(),
            has_imported_const_inline: false,
            has_w17_marshal_residual: false,
            has_try_unwrap_residual: false,
            has_reference_escape_promotion: false,
            has_null_coalesce_residual: false,
        }
    }

    /// Build a receiver request whose entry blob is `entry`, carrying `program`
    /// as the content-addressed payload.
    fn mk_ca_request(program: BytecodeProgram, entry: FunctionHash, name: &str) -> RemoteCallRequest {
        RemoteCallRequest {
            call_id: None,
            program,
            function_name: name.to_string(),
            function_id: None,
            function_hash: Some(entry),
            arguments: vec![],
            upvalues: None,
            upvalue_kinds: None,
            type_schemas: shape_runtime::type_schema::TypeSchemaRegistry::new(),
            program_hash: [0u8; 32],
            function_blobs: None,
        }
    }

    #[test]
    fn receiver_refuses_fn_requiring_fswrite_when_receiver_grants_pure() {
        use shape_abi_v1::Permission;
        // Honest blob that DECLARES fs.write, finalized so its content hash
        // verifies. The receiver grants nothing (pure) → PermissionDenied,
        // decided by the RECEIVER's config — never the sender's claim.
        let mut blob = mk_blob("writes_file", mk_hash(1), vec![]);
        let mut required = PermissionSet::pure();
        required.insert(Permission::FsWrite);
        blob.required_permissions = required;
        blob.finalize();
        let hash = blob.content_hash;
        let mut store = HashMap::new();
        store.insert(hash, blob);
        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(mk_ca_program(hash, store));

        let request = mk_ca_request(program, hash, "writes_file");
        let granted = PermissionSet::pure(); // receiver does NOT grant fs.write
        let resp = execute_remote_call(request, &temp_store(), &granted);
        match resp.result {
            Err(e) => {
                assert_eq!(
                    e.kind,
                    RemoteErrorKind::PermissionDenied,
                    "expected PermissionDenied, msg={}",
                    e.message
                );
                assert!(
                    e.message.contains("fs.write"),
                    "deny message names the missing permission: {}",
                    e.message
                );
            }
            Ok(v) => panic!("expected PermissionDenied, got Ok({:?})", v),
        }
    }

    #[test]
    fn receiver_admits_fn_requiring_fswrite_when_receiver_grants_it() {
        use shape_abi_v1::Permission;
        // Same honest blob; the receiver DOES grant fs.write → the load gate
        // opens. Proves enforcement follows the RECEIVER's config: flipping
        // only the granted set flips the outcome.
        let mut blob = mk_blob("writes_file", mk_hash(1), vec![]);
        let mut required = PermissionSet::pure();
        required.insert(Permission::FsWrite);
        blob.required_permissions = required;
        blob.finalize();
        let hash = blob.content_hash;
        let mut store = HashMap::new();
        store.insert(hash, blob);
        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(mk_ca_program(hash, store));

        let request = mk_ca_request(program, hash, "writes_file");
        let mut granted = PermissionSet::pure();
        granted.insert(Permission::FsWrite);
        let resp = execute_remote_call(request, &temp_store(), &granted);
        // Must NOT refuse when the receiver grants the permission.
        if let Err(ref e) = resp.result {
            assert_ne!(
                e.kind,
                RemoteErrorKind::PermissionDenied,
                "receiver granted fs.write but still refused: {}",
                e.message
            );
        }
    }

    #[test]
    fn receiver_rejects_blob_with_mismatched_content_hash() {
        // Store an honest blob under a WRONG key; recompute-verify catches it.
        let mut blob = mk_blob("f", mk_hash(1), vec![]);
        blob.finalize();
        let wrong_key = mk_hash(0xEE);
        assert_ne!(blob.content_hash, wrong_key, "test setup: keys must differ");
        let mut store = HashMap::new();
        store.insert(wrong_key, blob);
        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(mk_ca_program(wrong_key, store));

        let request = mk_ca_request(program, wrong_key, "f");
        let resp = execute_remote_call(request, &temp_store(), &PermissionSet::pure());
        match resp.result {
            Err(e) => assert_eq!(
                e.kind,
                RemoteErrorKind::HashMismatch,
                "expected HashMismatch, msg={}",
                e.message
            ),
            Ok(v) => panic!("expected HashMismatch, got Ok({:?})", v),
        }
    }

    #[test]
    fn receiver_reports_missing_dependency_as_missing_module_function() {
        // Entry blob references a dependency hash that is absent from the
        // store → structured MissingModuleFunction (no panic), with the
        // absent hash reported so the sender can resupply (§4.3-4).
        let dep_hash = mk_hash(2);
        let mut blob = mk_blob("f", mk_hash(1), vec![dep_hash]);
        blob.finalize();
        let entry = blob.content_hash;
        let mut store = HashMap::new();
        store.insert(entry, blob);
        // dep_hash intentionally absent.
        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(mk_ca_program(entry, store));

        let request = mk_ca_request(program, entry, "f");
        let resp = execute_remote_call(request, &temp_store(), &PermissionSet::pure());
        match resp.result {
            Err(e) => {
                assert_eq!(e.kind, RemoteErrorKind::MissingModuleFunction, "msg={}", e.message);
                let missing = e
                    .missing_blobs
                    .expect("MissingModuleFunction populates missing_blobs");
                assert!(
                    missing.contains(&dep_hash),
                    "missing_blobs reports the absent dependency hash"
                );
            }
            Ok(v) => panic!("expected MissingModuleFunction, got Ok({:?})", v),
        }
    }

    #[test]
    fn transport_error_phase_and_variant_distinguish_presend_from_timeout() {
        use shape_wire::transport::TransportError;
        // Pre-send connect failure: the call provably did not execute.
        let connect = TransportError::ConnectionFailed("refused".to_string());
        assert_eq!(transport_send_phase(&connect), SendPhase::DidNotExecute);
        assert_eq!(transport_error_shape_variant(&connect), "Transport");

        // Post-send read timeout: the call MAY have executed — a DISTINCT
        // classification from the pre-send Transport failure.
        let timeout = TransportError::Timeout;
        assert_eq!(transport_send_phase(&timeout), SendPhase::MayHaveExecuted);
        assert_eq!(transport_error_shape_variant(&timeout), "Timeout");

        // A reset after the frame went out is post-send ConnectionLost.
        let lost = TransportError::ConnectionClosed;
        assert_eq!(transport_send_phase(&lost), SendPhase::MayHaveExecuted);
        assert_eq!(transport_error_shape_variant(&lost), "ConnectionLost");

        // The two phases are not equal — the split is observable.
        assert_ne!(
            transport_send_phase(&connect),
            transport_send_phase(&timeout),
            "pre-send Transport must be distinguishable from post-send Timeout"
        );
    }

    // -----------------------------------------------------------------------
    // WF-2C: minimal blob-closure transfer, sender-side retry-once resupply,
    // and closures crossing the wire with an explicit upvalue_kinds parallel
    // track + refusal matrix (distributed §4.3 / §4.4).
    // -----------------------------------------------------------------------

    /// Locate the (function_id, content hash) of the single closure function in
    /// a compiled content-addressed program.
    fn find_closure(program: &BytecodeProgram) -> (u16, FunctionHash) {
        let fid = program
            .functions
            .iter()
            .position(|f| f.is_closure)
            .expect("program defines a closure") as u16;
        let hash = program
            .function_blob_hashes
            .get(fid as usize)
            .copied()
            .flatten()
            .expect("closure function has a content hash");
        (fid, hash)
    }

    #[test]
    fn remote_named_function_executes_end_to_end() {
        // Baseline: a plain named function marshals + executes through the
        // receiver, so the closure/resupply cases below build on a known-good
        // execution path (the marshal template is unchanged by WF-2C).
        let program = compile("fn add(a: int, b: int) -> int { a + b }");
        let req = build_call_request(
            &program,
            "add",
            vec![SerializableVMValue::Int(3), SerializableVMValue::Int(4)],
        );
        let resp = execute_remote_call(req, &temp_store(), &PermissionSet::pure());
        match resp.result {
            Ok(SerializableVMValue::Int(v)) => assert_eq!(v, 7),
            other => panic!("expected Ok(Int(7)), got {other:?}"),
        }
    }

    #[test]
    fn remote_closure_with_immutable_captures_executes() {
        // A closure capturing an immutable int crosses the wire with its
        // upvalue_kinds track and executes on the receiver (distributed §4.4).
        let program = compile(
            "fn make_adder(n: int) -> int {
    let add_n = |x| x + n
    return add_n(0)
}",
        );
        let (fid, hash) = find_closure(&program);

        // The compiler stamps the closure blob's per-capture identity: the
        // proven kind track (hash-covered §4.8) and the non-hash capture NAMES
        // (§4.4, for legible refusal messages). Assert the full compiler→blob
        // wiring, not just synthetic-blob behaviour.
        let blob = program
            .content_addressed
            .as_ref()
            .and_then(|ca| ca.function_store.get(&hash))
            .expect("closure blob present in the content-addressed store");
        assert_eq!(
            blob.capture_kinds,
            vec![shape_value::NativeKind::Int64],
            "closure captures one int (n)"
        );
        assert_eq!(
            blob.capture_names,
            vec!["n".to_string()],
            "compiler recorded the captured variable name"
        );

        let req = build_closure_call_request(
            &program,
            fid,
            vec![SerializableVMValue::Int(5)],    // arg:     x = 5
            vec![SerializableVMValue::Int(10)],   // capture: n = 10
            vec![shape_value::NativeKind::Int64], // per-capture kind track
        );
        let resp = execute_remote_call(req, &temp_store(), &PermissionSet::pure());
        match resp.result {
            Ok(SerializableVMValue::Int(v)) => assert_eq!(v, 15, "x + n = 5 + 10"),
            other => panic!("expected Ok(Int(15)), got {other:?}"),
        }
    }

    #[test]
    fn remote_closure_mutable_capture_refused_with_legible_message() {
        // A mutable capture is refused (Q27) with a message that names the
        // captured VARIABLE and gives Shape-surface remediation — never a slot
        // index (§4.4 legibility rule).
        let mut blob = mk_blob("counter_closure", mk_hash(1), vec![]);
        blob.is_closure = true;
        blob.captures_count = 1;
        blob.capture_kinds = vec![shape_value::NativeKind::Int64];
        blob.mutable_captures = vec![true];
        blob.capture_names = vec!["counter".to_string()];
        blob.finalize();
        let hash = blob.content_hash;
        let mut store = HashMap::new();
        store.insert(hash, blob);
        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(mk_ca_program(hash, store));

        let mut request = mk_ca_request(program, hash, "counter_closure");
        request.upvalues = Some(vec![SerializableVMValue::Int(0)]);
        request.upvalue_kinds = Some(vec![shape_value::NativeKind::Int64]);

        let resp = execute_remote_call(request, &temp_store(), &PermissionSet::pure());
        match resp.result {
            Err(e) => {
                assert_eq!(
                    e.kind,
                    RemoteErrorKind::UnsupportedCapture,
                    "msg={}",
                    e.message
                );
                assert!(
                    e.message.contains("'counter'"),
                    "names the captured variable: {}",
                    e.message
                );
                assert!(
                    e.message.contains("as an argument"),
                    "gives remediation: {}",
                    e.message
                );
                assert!(
                    !e.message.contains("slot") && !e.message.contains("index"),
                    "no slot-index jargon: {}",
                    e.message
                );
            }
            Ok(v) => panic!("expected UnsupportedCapture refusal, got Ok({v:?})"),
        }
    }

    #[test]
    fn remote_closure_missing_kind_track_refused_not_bool_defaulted() {
        // upvalues present but no upvalue_kinds track ⇒ structured ArgumentError
        // (T8c). The receiver refuses to fabricate Bool-default kinds
        // (ADR-006 §2.7.7/§2.7.8 forbidden), never executes.
        let mut blob = mk_blob("c", mk_hash(1), vec![]);
        blob.is_closure = true;
        blob.captures_count = 1;
        blob.capture_kinds = vec![shape_value::NativeKind::Int64];
        blob.capture_names = vec!["v".to_string()];
        blob.finalize();
        let hash = blob.content_hash;
        let mut store = HashMap::new();
        store.insert(hash, blob);
        let mut program = BytecodeProgram::default();
        program.content_addressed = Some(mk_ca_program(hash, store));

        let mut request = mk_ca_request(program, hash, "c");
        request.upvalues = Some(vec![SerializableVMValue::Int(7)]);
        request.upvalue_kinds = None; // MISSING kind track

        let resp = execute_remote_call(request, &temp_store(), &PermissionSet::pure());
        match resp.result {
            Err(e) => {
                assert_eq!(e.kind, RemoteErrorKind::ArgumentError, "msg={}", e.message);
                assert!(
                    e.message.contains("upvalue_kinds"),
                    "explains the missing kind track: {}",
                    e.message
                );
            }
            Ok(v) => panic!("expected ArgumentError (no Bool-default), got Ok({v:?})"),
        }
    }

    #[test]
    fn sender_resupply_retries_once_and_succeeds() {
        // main_fn depends on helper. Ship a STRIPPED request (helper stripped);
        // the receiver reports MissingModuleFunction; `call_with_resupply` looks
        // the hash up in the sender's own store, resupplies, and retries exactly
        // once (distributed §4.3-5), then the call succeeds.
        let program = compile(
            "fn helper(x: int) -> int { x + 1 } \
             fn main_fn(x: int) -> int { helper(x) + helper(x) }",
        );
        let main_fid = program
            .functions
            .iter()
            .position(|f| f.name == "main_fn")
            .expect("main_fn compiled") as u16;
        let entry_hash = program.function_blob_hashes[main_fid as usize].expect("main_fn hash");
        let full = build_minimal_blobs_by_hash(&program, entry_hash).expect("minimal blobs");
        assert!(full.len() >= 2, "closure of main_fn includes helper");

        // Stripped request: keep only the entry blob, drop its dependency.
        let mut stripped =
            build_call_request(&program, "main_fn", vec![SerializableVMValue::Int(10)]);
        let blobs = stripped
            .function_blobs
            .as_mut()
            .expect("content-addressed request carries blobs");
        blobs.retain(|(h, _)| *h == entry_hash);
        assert_eq!(blobs.len(), 1, "only the entry blob remains after stripping");

        let store = temp_store();
        let granted = PermissionSet::pure();
        let mut call_count = 0u32;
        let resp = call_with_resupply(&program, stripped, |req| {
            call_count += 1;
            execute_remote_call(req.clone(), &store, &granted)
        });

        assert_eq!(call_count, 2, "exactly one retry after resupply");
        match resp.result {
            Ok(SerializableVMValue::Int(v)) => {
                assert_eq!(v, 22, "helper(10) + helper(10) = 11 + 11")
            }
            other => panic!("expected Ok(Int(22)) after resupply, got {other:?}"),
        }
    }

    #[test]
    fn sender_resupply_does_not_retry_on_non_missing_error() {
        // A non-MissingModuleFunction failure is returned unchanged — no
        // resupply, no retry (distributed §4.3-5 / OQ-8: only MissingModule-
        // Function is provably pre-execution and retry-safe).
        let program = compile("fn add(a: int, b: int) -> int { a + b }");
        let request = build_call_request(&program, "add", vec![SerializableVMValue::Int(1)]);
        let mut call_count = 0u32;
        let resp = call_with_resupply(&program, request, |_req| {
            call_count += 1;
            RemoteCallResponse {
                result: Err(RemoteCallError::new(
                    RemoteErrorKind::RuntimeError,
                    "boom",
                )),
            }
        });
        assert_eq!(call_count, 1, "no retry for a non-missing-blob failure");
        assert!(matches!(
            resp.result,
            Err(ref e) if matches!(e.kind, RemoteErrorKind::RuntimeError)
        ));
    }

    #[test]
    fn build_call_request_by_id_round_trips_named_function() {
        // The sender dispatcher's named-function path (`fn_ref =
        // NativeKind::UInt64` id, e.g. `@remote`'s `ctx.target`) resolves
        // through `build_call_request_by_id` — canonical id keying, no
        // name-lookup ambiguity (distributed §4.3-1). Round-trip it in-process.
        let program = compile("fn addup(a: int, b: int) -> int { a + b }");
        let fid = program
            .functions
            .iter()
            .position(|f| f.name == "addup")
            .expect("addup compiled") as u16;
        let request = build_call_request_by_id(
            &program,
            fid,
            vec![SerializableVMValue::Int(2), SerializableVMValue::Int(3)],
        )
        .expect("by-id request built");
        assert_eq!(request.function_id, Some(fid));
        assert_eq!(request.function_name, "addup");
        assert!(request.upvalues.is_none(), "named fn carries no upvalues");

        let store = temp_store();
        let granted = PermissionSet::pure();
        match execute_remote_call(request, &store, &granted).result {
            Ok(SerializableVMValue::Int(v)) => assert_eq!(v, 5, "addup(2,3) == 5 remotely"),
            other => panic!("expected Ok(Int(5)), got {other:?}"),
        }
    }

    #[test]
    fn build_call_request_by_id_rejects_out_of_range_id() {
        let program = compile("fn f() -> int { 1 }");
        let err = build_call_request_by_id(&program, 9999, vec![])
            .expect_err("out-of-range id must error, not panic");
        assert!(err.contains("out of range"), "got: {err}");
    }
}
