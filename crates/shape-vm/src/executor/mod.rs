//! Virtual machine executor for Shape bytecode

// Opcode category implementations (split into submodules)
mod additional;
mod arithmetic;
mod async_ops;
mod builtins;
mod call_convention;
mod comparison;
mod control_flow;
pub(crate) mod dispatch;
mod exceptions;
pub(crate) mod ic_fast_paths;
mod jit_ops;
mod logical;
mod loops;
pub(crate) mod objects;
mod osr;
mod resume;
mod snapshot;
mod stack_ops;
pub mod state_builtins;
pub mod time_travel;
mod trait_object_ops;
// W11-fup-C (Phase 3d, 2026-05-18): exposed `pub` so the JIT-side
// `crates/shape-jit/src/ffi/v2/mod.rs` allocators can call
// `v2_handlers::v2_array_detect::stamp_elem_type` + the
// `ELEM_TYPE_*` constants at allocation time — mirror of the VM-side
// `op_new_typed_array_*` stamp pattern (`v2_handlers/array.rs:40-81`).
// Without the stamp the canonical `as_v2_typed_array` carrier-
// recognition path (`v2_array_detect.rs:181-216`) reads
// `_pad = 0 = ELEM_TYPE_UNKNOWN` and the print / method-dispatch
// arms silently fall back to the non-typed scalar render (the
// pre-fix empirical surface — `print(arr)` JIT output was the raw
// pointer printed as a u64 scalar).
pub mod v2_handlers;
mod variables;
pub(crate) mod vm_state_snapshot;
mod window_join;

// VM infrastructure modules
pub mod debugger_integration;
pub mod gc_integration;
pub mod module_registry;
pub mod printing;
pub mod task_scheduler;
pub mod typed_object_ops;
pub mod utils;

// Test module
#[cfg(test)]
mod tests;

// Re-export async types for external use
pub use async_ops::{AsyncExecutionResult, SuspensionInfo, WaitType};
pub use control_flow::foreign_marshal;
pub use control_flow::native_abi;
pub use task_scheduler::{TaskScheduler, TaskStatus};

/// Reserved future ID used to signal a snapshot suspension
pub const SNAPSHOT_FUTURE_ID: u64 = u64::MAX;

/// Error returned when a program requires permissions not granted by the host.
#[derive(Debug, Clone)]
pub enum PermissionError {
    /// The program requires permissions not in the granted set.
    InsufficientPermissions {
        /// All permissions the program requires.
        required: shape_abi_v1::PermissionSet,
        /// Permissions the host granted.
        granted: shape_abi_v1::PermissionSet,
        /// Permissions required but not granted.
        missing: shape_abi_v1::PermissionSet,
    },
    /// Linking failed before permission checking could occur.
    LinkError(String),
}

impl std::fmt::Display for PermissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionError::InsufficientPermissions { missing, .. } => {
                let names: Vec<&str> = missing.iter().map(|p| p.name()).collect();
                write!(
                    f,
                    "program requires permissions not granted: {}",
                    names.join(", ")
                )
            }
            PermissionError::LinkError(msg) => write!(f, "link error: {msg}"),
        }
    }
}

impl std::error::Error for PermissionError {}

/// Result of VM execution.
///
/// Wave-β R-misc migration (per ADR-006 §2.7 / Q7 + playbook §10
/// E-execution row): the `Completed` variant carries a `KindedSlot` —
/// the canonical post-`ValueWord` runtime-value carrier (raw bits +
/// parallel `NativeKind`). Hosts that previously called methods on the
/// returned `ValueWord` (e.g. `as_number_coerce`, `as_i64`, `as_str`)
/// must dispatch on `result.kind()` and use the per-variant
/// `KindedSlot::as_*` accessors per §2.7.6.
#[derive(Debug, Clone)]
pub enum ExecutionResult {
    /// Execution completed normally with a typed value carrier.
    Completed(shape_value::KindedSlot),
    /// Execution suspended waiting for a future to resolve
    Suspended {
        /// The future ID that needs to be resolved
        future_id: u64,
        /// The instruction pointer to resume at
        resume_ip: usize,
    },
}

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;

use crate::{
    bytecode::{
        BuiltinFunction, BytecodeProgram, FunctionBlob, FunctionHash, Instruction, Operand,
    },
    debugger::VMDebugger,
    memory::{GCConfig, GarbageCollector},
    tier::TierManager,
};
use shape_ast::data::Timeframe;

use crate::constants::{DEFAULT_GC_TRIGGER_THRESHOLD, MAX_CALL_STACK_DEPTH, MAX_STACK_SIZE};
// `KindedSlot` and `NativeKind` are the post-`ValueWord` runtime-value
// carriers (ADR-006 §2.7); both used in the `ExecutionResult` /
// `CallFrame` / `VirtualMachine` field types below. `HeapValue` /
// `ValueSlot` / `VMError` are intentionally left unimported here —
// fields that need them name the type via their fully-qualified path
// to keep the executor `mod.rs` lean.
use shape_value::{KindedSlot, NativeKind};
/// VM configuration
#[derive(Debug, Clone)]
pub struct VMConfig {
    /// Maximum stack size
    pub max_stack_size: usize,
    /// Maximum call depth
    pub max_call_depth: usize,
    /// Enable debug mode
    pub debug_mode: bool,
    /// Enable instruction tracing
    pub trace_execution: bool,
    /// Garbage collection configuration
    pub gc_config: GCConfig,
    /// Enable automatic garbage collection
    pub auto_gc: bool,
    /// GC trigger threshold (instructions between collections)
    pub gc_trigger_threshold: usize,
    /// Enable VM metrics collection (counters, tier/GC event ring buffers, histograms).
    /// When false (default), `VirtualMachine.metrics` is `None` for zero overhead.
    pub metrics_enabled: bool,
    /// When true, automatically initialise the tracing GC heap (`shape-gc`) on
    /// VM creation instead of relying on Arc reference counting.
    ///
    /// Requires the `gc` crate feature to be compiled in; otherwise this flag
    /// is silently ignored.
    pub use_tracing_gc: bool,
}

impl Default for VMConfig {
    fn default() -> Self {
        Self {
            max_stack_size: MAX_STACK_SIZE,
            max_call_depth: MAX_CALL_STACK_DEPTH,
            debug_mode: false,
            trace_execution: false,
            gc_config: GCConfig::default(),
            auto_gc: true,
            gc_trigger_threshold: DEFAULT_GC_TRIGGER_THRESHOLD,
            metrics_enabled: false,
            use_tracing_gc: false,
        }
    }
}

/// Call frame for function calls
#[derive(Debug)]
pub struct CallFrame {
    /// Return address
    pub return_ip: usize,
    /// Base pointer into the unified value stack where this frame's locals start
    pub base_pointer: usize,
    /// Number of locals
    pub locals_count: usize,
    /// Function index
    pub function_id: Option<u16>,
    /// Upvalues captured by this closure (None for regular functions).
    ///
    /// SURFACE (phase-2c): the legacy `shape_value::Upvalue` carrier was
    /// deleted during the strict-typing bulldozer (the v1 ValueWord-tagged
    /// closure-capture word). The replacement is the v2 typed closure
    /// surface (`shape_value::v2::closure_raw::OwnedClosureBlock` /
    /// `ClosureLayout`), but the wiring through `CallFrame` is part of
    /// the §2.7.8 / Q10 cell-storage extension scheduled for
    /// `B6-variables-loadptr` / `B7-closure-cells`. Until then this field
    /// is a `Vec<u64>` of raw-bit captures, matching the existing layout
    /// in `closure_raw::ClosureCell`. Consumers in
    /// `executor/variables/mod.rs`, `executor/gc_integration.rs`,
    /// `executor/state_builtins/introspection.rs`, and `crates/shape-vm/
    /// src/remote.rs` are pre-existing Wave-α-broken and migrate together
    /// with this surface — see ADR-006 §2.7.4 deferral pattern.
    pub upvalues: Option<Vec<u64>>,
    /// Content hash of the function blob being executed (for content-addressed state capture).
    /// `None` for programs compiled without content-addressed metadata.
    pub blob_hash: Option<FunctionHash>,
    /// WB2.3 retain-on-read: optional owning `ValueWord` bits for the
    /// closure HeapValue that backs this frame's `upvalues`. When the
    /// frame is a closure call, `CaptureKind::OwnedMutable` / `Shared`
    /// captures in `upvalues` are raw `*mut ValueWord` / `*const
    /// SharedCell` pointer bits into this block's allocation — the
    /// frame must hold this share so the block outlives the callee's
    /// pointer dereferences.
    ///
    /// `None` for regular function calls and host-closure calls.
    /// Released via `drop_with_kind(bits, kind)` on frame-pop (see
    /// `op_return` / `op_return_value` cleanup) using the lockstep
    /// `closure_heap_kind` companion below.
    pub closure_heap_bits: Option<u64>,
    /// ADR-006 §2.7.8 / Q10 — lockstep `NativeKind` companion for
    /// `closure_heap_bits`. Both fields are `Some` together or `None`
    /// together at every observable boundary; mixed states are a bug.
    /// When `closure_heap_bits = Some(bits)`, this carries the
    /// `NativeKind` that the teardown path (`op_return` /
    /// `op_return_value`) feeds into `drop_with_kind(bits, kind)` —
    /// replacing the forbidden `vw_drop(bits)` (§2.7.7 #8) and the
    /// forbidden Bool-default fallback (§2.7.7 #9). For closure calls
    /// the kind is `NativeKind::Ptr(HeapKind::Closure)` (the
    /// `closure_heap_bits` always come from a TAG_HEAP `ValueWord`
    /// pointing to a `HeapValue::ClosureRaw`).
    pub closure_heap_kind: Option<shape_value::NativeKind>,
}

/// Function pointer type for JIT-compiled functions.
/// `ctx` is a mutable pointer to VM execution context (e.g., stack base).
/// `args` is a pointer to the argument buffer.
/// Returns a NaN-boxed result as raw u64 bits.
#[cfg(feature = "jit")]
pub type JitFnPtr = unsafe extern "C" fn(*mut u8, *const u8) -> u64;

/// Linked foreign-function handles.
///
/// Dynamic language runtimes are compiled/invoked through extension plugins.
/// Native ABI entries (`extern "C"`) are linked directly through the VM's
/// internal C ABI path.
#[derive(Clone)]
pub(crate) enum ForeignFunctionHandle {
    Runtime {
        runtime: std::sync::Arc<shape_runtime::plugins::language_runtime::PluginLanguageRuntime>,
        compiled: shape_runtime::plugins::language_runtime::CompiledForeignFunction,
    },
    Native(std::sync::Arc<control_flow::native_abi::NativeLinkedFunction>),
}

/// The Shape virtual machine
pub struct VirtualMachine {
    /// Configuration
    config: VMConfig,

    /// The program being executed
    pub(crate) program: BytecodeProgram,

    /// Instruction pointer
    ip: usize,

    /// Unified value stack (pre-allocated, raw u64: 8 bytes per slot).
    /// Locals live in register windows on this stack.
    /// Each slot stores the raw bit pattern of a typed value, interpreted
    /// according to the parallel `kinds` track. Ownership of any embedded
    /// Arc refcounts is managed manually via `push_kinded`/`pop_kinded`/
    /// `read_owned_kinded`/`stack_write_kinded` helpers (ADR-006 §2.7.7).
    pub(crate) stack: Vec<u64>,

    /// Parallel kind track (ADR-006 §2.7.7 / Q9).
    ///
    /// `kinds[i]` is the `NativeKind` interpretation of `stack[i]`. Index
    /// invariant: `stack.len() == kinds.len()` at every API boundary.
    /// WB2.4 retain-on-read uses this track for kind-aware clone/drop
    /// dispatch (`clone_with_kind` / `drop_with_kind`); the deleted
    /// tag_bits dispatch and the deleted `is_heap()` call do not run here.
    pub(crate) kinds: Vec<shape_value::NativeKind>,

    /// Stack pointer — logical top of the value stack.
    /// `stack[0..sp]` are live values; `stack[sp..]` is pre-allocated dead space.
    pub(crate) sp: usize,

    /// ModuleBinding variables (raw u64 bit patterns; see `stack` comment).
    ///
    /// Lockstep with `module_binding_kinds` per ADR-006 §2.7.8 / Q10:
    /// `module_bindings.len() == module_binding_kinds.len()` at every
    /// observable boundary. Pad/resize/clear sites must update both vecs
    /// together — see `module_binding_pad_to_kinded` for the canonical
    /// resize helper.
    pub(crate) module_bindings: Vec<u64>,

    /// Parallel `NativeKind` track for `module_bindings` (ADR-006 §2.7.8 /
    /// Q10). `module_binding_kinds[i]` is the `NativeKind` interpretation
    /// of `module_bindings[i]`. Index invariant: lengths agree at every
    /// API boundary.
    ///
    /// VM teardown dispatches `drop_with_kind(bits[i], kind[i])` per slot
    /// — the kind-aware counterpart of the deleted `vw_drop`/
    /// `vw_drop_slice` (forbidden #8 per §2.7.7). Slots written by typed-
    /// scalar Store opcodes (`StoreModuleBindingI64` / `…F64` / `…Bool`
    /// / etc.) carry their statically-known kind; PTR slots get the
    /// matching `NativeKind::Ptr(HeapKind::*)` from the producer. Slots
    /// pre-initialised by the resize-pad path use `NativeKind::Bool` as
    /// the no-op-on-drop sentinel — the same convention as the stack's
    /// dead-space pre-init (`init.rs:31`). This is **not** a Bool-default
    /// fallback in the §2.7.7 #9 sense: every actual write threads the
    /// caller's known kind, and the sentinel only persists for slots
    /// that never received a store.
    pub(crate) module_binding_kinds: Vec<NativeKind>,

    /// Track A.1C.3: indices of module-binding slots that were
    /// promoted to `Arc<parking_lot::Mutex<ValueWord>>` via
    /// `AllocSharedModuleBinding`. Each tracked slot holds raw
    /// `Arc::into_raw(...)` pointer bits — NOT a NaN-tagged ValueWord —
    /// and must be reclaimed via `Arc::from_raw` once at VM drop. The
    /// top-level `Drop` impl on VM consults this set before iterating
    /// `module_bindings`.
    pub(crate) shared_module_bindings: std::collections::HashSet<usize>,

    /// Call stack
    call_stack: Vec<CallFrame>,

    /// Loop stack for break/continue
    loop_stack: Vec<LoopContext>,
    /// Timeframe stack for timeframe context
    timeframe_stack: Vec<Option<Timeframe>>,

    /// Integrated debugger
    debugger: Option<VMDebugger>,

    /// Garbage collector
    gc: GarbageCollector,

    /// Instruction counter (used for interrupt checking)
    instruction_count: usize,

    /// Exception handler stack for try/catch blocks
    exception_handlers: Vec<ExceptionHandler>,

    /// Builtin schema IDs for fixed-layout runtime objects (AnyError, TraceFrame, etc.)
    pub(crate) builtin_schemas: shape_runtime::type_schema::BuiltinSchemaIds,

    /// Last error location (line number) for LSP integration
    /// Set by enrich_error_with_location when an error occurs
    last_error_line: Option<u32>,

    /// Last error file path for LSP integration
    /// Set by enrich_error_with_location when an error occurs
    last_error_file: Option<String>,

    /// Uncaught exception payload captured at VM boundary.
    ///
    /// Set when an exception escapes with no handler so hosts can render
    /// structured AnyError output without reparsing plain strings.
    last_uncaught_exception: Option<KindedSlot>,

    /// Whether module-level initialization code has been executed.
    /// Used by `execute_function_by_name` to ensure module bindings
    /// are initialized before calling the target function.
    module_init_done: bool,

    /// Output capture buffer for testing
    /// When Some, print output is captured here instead of going to stdout
    output_buffer: Option<Vec<String>>,

    /// Extension module registry — single source of truth for all extension modules.
    /// Used by extension dispatch, auto-available module_bindings, and LSP completions.
    module_registry: shape_runtime::module_exports::ModuleExportRegistry,

    /// Table of module-function entries indexed by usize ID.
    /// `ValueWord::ModuleFunction(id)` references this table for dispatch.
    ///
    /// Phase 4c.3: entries are now sum-typed
    /// (`Typed` / `TypedAsync` / `Legacy`) so the dispatch path can
    /// route typed-return functions through a path that skips the
    /// body-side `TypedReturn → ValueWord` round-trip.
    module_fn_table: Vec<shape_runtime::module_exports::ModuleFnEntry>,

    /// Runtime function name → index lookup for UFCS dispatch.
    /// Populated after program load. Used by handle_object_method to find
    /// type-scoped impl methods (e.g., "DuckDbQuery::filter") at runtime.
    pub(crate) function_name_index: HashMap<String, u16>,

    /// Method intrinsics for fast dispatch on typed Objects.
    /// Populated from ModuleExports.method_intrinsics during module registration.
    /// Checked in handle_object_method() after built-in methods, before UFCS.
    extension_methods: HashMap<String, HashMap<String, shape_runtime::module_exports::ModuleFn>>,

    /// Cache of resolved merged schemas: (left_id, right_id) → merged_id
    merged_schema_cache: HashMap<(u32, u32), u32>,

    /// Interrupt flag set by Ctrl+C handler (0 = none, >0 = interrupted)
    interrupt: Arc<AtomicU8>,

    /// Counter for generating unique future IDs (for SpawnTask).
    ///
    /// # Safety (single-threaded access)
    ///
    /// This is a plain `u64` rather than an `AtomicU64` because the VM executor
    /// is inherently single-threaded: `VirtualMachine` is `!Sync` and all
    /// execution happens on the thread that owns the VM instance. The counter
    /// is only mutated by `next_future_id()` which requires `&mut self`,
    /// guaranteeing exclusive access at compile time.
    future_id_counter: u64,

    /// Stack of async scopes for structured concurrency.
    /// Each entry is a list of Future IDs spawned within that scope.
    /// AsyncScopeEnter pushes a new Vec; AsyncScopeExit pops and cancels.
    async_scope_stack: Vec<Vec<u64>>,

    /// Task scheduler for async host runtime.
    /// Stores spawned callables and tracks their completion status.
    pub(crate) task_scheduler: task_scheduler::TaskScheduler,

    /// Compiled foreign function handles (linked at pre-execution time).
    /// Index corresponds to program.foreign_functions index.
    pub(crate) foreign_fn_handles: Vec<Option<ForeignFunctionHandle>>,

    /// Content hashes for each function, indexed by function_id.
    /// Populated from `BytecodeProgram.content_addressed` or `LinkedProgram`.
    /// `None` entries mean the function has no content-addressed metadata.
    function_hashes: Vec<Option<FunctionHash>>,

    /// Raw byte representation of `function_hashes` for passing to `ModuleContext`.
    /// Kept in sync with `function_hashes`; avoids per-call allocation when
    /// constructing `ModuleContext` (which uses `[u8; 32]` to avoid a dependency
    /// on `FunctionHash`).
    function_hash_raw: Vec<Option<[u8; 32]>>,

    /// Reverse lookup for hash-first execution identity.
    /// Maps function blob hash -> runtime function ID.
    function_id_by_hash: HashMap<FunctionHash, u16>,

    /// Entry points for each function, indexed by function_id.
    /// Used to compute `local_ip = ip - function_entry_points[function_id]`
    /// for content-addressed snapshot frames.
    function_entry_points: Vec<usize>,

    /// Effective execution entry IP for the currently loaded program.
    /// Normal bytecode starts at 0; linked content-addressed programs start
    /// at the entry function's `entry_point`.
    program_entry_ip: usize,

    /// Optional resource usage tracker for sandboxed execution.
    /// When set, the dispatch loop calls `tick_instruction()` each cycle.
    pub resource_usage: Option<crate::resource_limits::ResourceUsage>,

    /// Time-travel debugger for recording and navigating VM state history.
    /// `None` when time-travel debugging is not active.
    pub(crate) time_travel: Option<time_travel::TimeTravel>,

    /// GC heap (only present when `gc` feature is enabled).
    #[cfg(feature = "gc")]
    gc_heap: Option<shape_gc::GcHeap>,

    /// Whether selective JIT compilation has been applied to the loaded program.
    #[cfg(feature = "jit")]
    jit_compiled: bool,

    /// JIT dispatch table: function_id → extern "C" function pointer.
    /// Populated by external JIT compilers (e.g., shape-jit) via `register_jit_function`.
    #[cfg(feature = "jit")]
    jit_dispatch_table: std::collections::HashMap<u16, JitFnPtr>,

    /// Tiered compilation manager. Tracks per-function call counts and
    /// coordinates background JIT compilation via channels.
    /// `None` when tiered compilation is disabled.
    tier_manager: Option<TierManager>,

    /// Pending resume snapshot. Set by `state.resume()` stdlib function via
    /// the `set_pending_resume` callback on `ModuleContext`. Consumed by the
    /// dispatch loop after the current instruction completes.
    ///
    /// W17-state-tier-roundtrip (§2.7.4 + §2.7.5.1, Phase 2d Wave 3,
    /// 2026-05-12): The state.resume body in
    /// `state_builtins/introspection.rs` calls `set_pending_resume` (when
    /// `ModuleContext.set_pending_resume` is wired) to queue the
    /// snapshot KindedSlot here. `apply_pending_resume` consumes the
    /// queue on the next dispatch tick; the actual resume reconstruction
    /// path (decode the typed-object VmState payload → rebuild
    /// stack/locals via `serializable_to_slot`) requires a typed-object
    /// field-decode helper that lands with W17-marshal-return-arms.
    /// Until that lands, `apply_pending_resume` returns a structured
    /// `VMError::NotImplemented` carrying the `PHASE_2C_SNAPSHOT_SURFACE`
    /// string (`executor/resume.rs:55`).
    pub(crate) pending_resume: Option<KindedSlot>,

    /// Pending single-frame resume data. Set by `state.resume_frame()` to
    /// override IP and locals after function invocation sets up the call frame.
    pub(crate) pending_frame_resume: Option<FrameResumeData>,

    /// Optional VM metrics collector. `None` when `VMConfig.metrics_enabled`
    /// is false (the default), giving zero per-instruction overhead.
    pub metrics: Option<crate::metrics::VmMetrics>,

    /// Per-function feedback vectors for inline cache profiling.
    /// Indexed by function_id. None means no feedback collected for that function.
    /// Only populated when tiered compilation is enabled.
    feedback_vectors: Vec<Option<crate::feedback::FeedbackVector>>,

    /// Megamorphic property lookup cache. Used when a property access site has
    /// seen too many different schemas (>4 targets) and IC state is Megamorphic.
    megamorphic_cache: crate::megamorphic_cache::MegamorphicCache,

    /// Shape transition table + transition log owned by this VM.
    ///
    /// Replaces the process-global `GLOBAL_SHAPE_TABLE` / `SHAPE_TRANSITION_LOG`
    /// statics. The VM installs this handle as the ambient
    /// `shape_value::current_shape_table()` around every execution entry
    /// point so HashMapData helpers (both VM-side and JIT-FFI-side) can
    /// reach it without passing `&mut vm` through raw `extern "C"` calls.
    pub(crate) shape_table: std::sync::Arc<shape_value::ShapeTableHandle>,
}

/// Data for resuming a single call frame mid-function.
///
/// SURFACE (phase-2c): consumed by `apply_pending_frame_resume` which is
/// a Phase-2c stub (snapshot subsystem deferral, ADR-006 §2.7.4). Locals
/// are carried as `Vec<KindedSlot>` so the rebuild path threads
/// `NativeKind` for each local through the resumed frame.
pub(crate) struct FrameResumeData {
    /// IP offset within the function to resume at.
    pub ip_offset: usize,
    /// Locals to restore in the resumed frame.
    pub locals: Vec<KindedSlot>,
}

/// Exception handler for try/catch blocks
#[derive(Debug, Clone)]
struct ExceptionHandler {
    /// Instruction pointer to jump to on exception
    catch_ip: usize,
    /// Stack size when handler was set up (for unwinding)
    stack_size: usize,
    /// Call stack depth when handler was set up
    call_depth: usize,
}

/// Loop context for break/continue
#[derive(Debug)]
struct LoopContext {
    /// Start of loop body (for continue)
    start: usize,
    /// End of loop (for break)
    end: usize,
}

/// Debug VM state snapshot for the debugger
#[derive(Debug)]
pub struct DebugVMState {
    /// Current instruction pointer
    pub ip: usize,
    /// Call stack depth
    pub call_stack_depth: usize,
}

mod vm_impl;

/// Drop implementation for VirtualMachine.
///
/// Releases the strong-count share that each live stack slot and each
/// module-binding slot owns over its heap-tagged payload. Per ADR-006
/// §2.7.7 / §2.7.8, the stack and the module-binding store each carry a
/// parallel `Vec<NativeKind>` track; teardown dispatches
/// `drop_with_kind(bits, kind)` per slot — the kind-aware counterpart of
/// the deleted `vw_drop_slice` call (forbidden #8 per §2.7.7).
///
/// The `shared_module_bindings` Arc reclamation still runs first because
/// those slots hold raw `Arc::into_raw` pointer bits (not heap-tagged
/// values) and the producer is the unique strong owner — the kind-aware
/// drop loop over `module_bindings` skips them because the slot bits
/// have been zeroed and `drop_with_kind` is a no-op on the zero bit
/// pattern.
impl Drop for VirtualMachine {
    fn drop(&mut self) {
        // Track A.1C.3: release Shared module-binding Arcs before
        // draining the kinded bindings. These slots hold raw
        // `Arc::into_raw(Arc::new(SharedCell))` pointer bits — they
        // must be reclaimed via `Arc::from_raw`, not via the
        // kinded-drop dispatch.
        use shape_value::v2::closure_layout::SharedCell;
        for &idx in &self.shared_module_bindings {
            if idx >= self.module_bindings.len() {
                continue;
            }
            let bits = self.module_bindings[idx];
            self.module_bindings[idx] = 0u64;
            // Zero the parallel kind slot to a no-op-on-drop sentinel
            // so the lockstep loop below treats the cleared bits as a
            // dead slot. Without this, a stale `Ptr(HeapKind::*)` kind
            // would survive the bits-clear and double-release on the
            // generic loop pass.
            if idx < self.module_binding_kinds.len() {
                self.module_binding_kinds[idx] = NativeKind::Bool;
            }
            let cell_ptr = bits as *const SharedCell;
            if cell_ptr.is_null() {
                continue;
            }
            // SAFETY: `cell_ptr` was produced by
            // `Arc::into_raw(Arc::new(...))` in
            // `op_alloc_shared_module_binding` and this is the unique
            // release point for the strong share owned by the module-
            // bindings slot. Capture-side Arc shares owned by still-
            // live closures stay alive independently; the underlying
            // SharedCell persists until every share is dropped.
            unsafe {
                drop(std::sync::Arc::from_raw(cell_ptr));
            }
        }
        self.shared_module_bindings.clear();

        // Release the live stack window. `self.sp` is the high-water
        // mark of owned slots; everything above is already NONE_BITS
        // sentinels (per-opcode cleanup invariant). Kind comes from
        // the parallel `Vec<NativeKind>` track — ADR-006 §2.7.7
        // dispatch surface.
        let live = self.sp.min(self.stack.len()).min(self.kinds.len());
        for i in 0..live {
            let bits = self.stack[i];
            let kind = self.kinds[i];
            vm_impl::stack::drop_with_kind(bits, kind);
            self.stack[i] = Self::NONE_BITS;
            self.kinds[i] = NativeKind::Bool;
        }

        // Release every module binding via the parallel-kind track
        // (ADR-006 §2.7.8 / Q10). `drop_with_kind` is a no-op on
        // inline-scalar kinds (Int*/UInt*/Bool/Float64) so typed-scalar
        // bindings cost a kind-check and return; heap-bearing bindings
        // (`NativeKind::Ptr(HeapKind::*)` / `NativeKind::String`)
        // release exactly one strong-count share via the matching
        // `Arc::decrement_strong_count::<T>`. This closes the R-misc
        // Wave-β "non-shared module bindings hold one strong share each
        // that is not released at VM teardown" leak surface — once the
        // kind track is populated lockstep with the bits track, the
        // generic teardown sees the same kind every other §2.7.7
        // dispatch surface uses.
        //
        // Defensive `min()` walks: if a producer push site grew
        // `module_bindings` without growing `module_binding_kinds`
        // (B6-round-2 territory not yet migrated), the lockstep
        // invariant is violated. The debug-build assertion below
        // surfaces such drift; release builds walk the prefix that
        // both vecs cover. Prefix slots that exceed the kinds-vec
        // length keep the legacy "leak rather than misdispatch"
        // disposition until B6-round-2 lands the kind threading —
        // explicitly NOT a Bool-default fallback (§2.7.7 #9 forbidden,
        // §2.7.8 forbidden-shapes "Transitional Bool-default
        // fallbacks"): the unwalked tail is the kind-source SURFACE,
        // not a silent kind-fabrication.
        debug_assert_eq!(
            self.module_bindings.len(),
            self.module_binding_kinds.len(),
            "ADR-006 §2.7.8 / Q10 lockstep invariant violated at \
             VirtualMachine::Drop: module_bindings.len() ({}) != \
             module_binding_kinds.len() ({}). A push/resize site in \
             cluster-B-round-2 territory (executor/variables/mod.rs) \
             grew the bits vec without growing the kinds vec.",
            self.module_bindings.len(),
            self.module_binding_kinds.len(),
        );
        let bound = self
            .module_bindings
            .len()
            .min(self.module_binding_kinds.len());
        for i in 0..bound {
            let bits = self.module_bindings[i];
            let kind = self.module_binding_kinds[i];
            vm_impl::stack::drop_with_kind(bits, kind);
            self.module_bindings[i] = Self::NONE_BITS;
            self.module_binding_kinds[i] = NativeKind::Bool;
        }
        // Zero any tail slots that exceeded the kinds-vec length so
        // post-Drop reads (debug print, snapshot enumeration) do not
        // see dangling pointer bits. The corresponding strong-count
        // share leaks until the kinds vec catches up — that's the
        // remaining B6-round-2 work, not the §2.7.8 structural
        // extension this Drop is part of.
        for slot in self.module_bindings[bound..].iter_mut() {
            *slot = Self::NONE_BITS;
        }
    }
}

/// ADR-006 §2.7.8 / Q10 kinded module-binding accessors.
///
/// The §2.7.8 cell-storage extension grew `VirtualMachine.module_bindings`
/// from a bare `Vec<u64>` to a `Vec<u64>` + parallel `Vec<NativeKind>`
/// pair (`module_binding_kinds`). These methods are the lockstep-safe
/// API every consumer SHOULD use; direct field access remains in place
/// for the Wave-β B6-round-2 migration window but is replaced site-by-
/// site as those handlers are rewritten.
///
/// The dispatch tables match `vm_impl::stack::clone_with_kind` /
/// `drop_with_kind` exactly — same retain-on-read primitives the stack
/// and `KindedSlot` use. No `vw_clone` (forbidden #8 per §2.7.7), no
/// `is_heap` probe (forbidden #7), no Bool-default fallback when a
/// kind-source gap appears (§2.7.7 #9, §2.7.8 forbidden-shapes).
impl VirtualMachine {
    /// Grow the module-binding store so that `index` is in bounds. Pads
    /// the bits vec with `NONE_BITS` and the kinds vec with the no-op-on-
    /// drop sentinel `NativeKind::Bool` — same convention as the stack's
    /// dead-space pre-init in `init.rs:31`. Both vecs grow together so
    /// the §2.7.8 lockstep invariant `module_bindings.len() ==
    /// module_binding_kinds.len()` holds at the call boundary.
    ///
    /// The sentinel kind is **not** a Bool-default fallback in the §2.7.7
    /// #9 sense: it marks "no value has ever been written to this slot",
    /// not "we don't know the kind of an existing heap-bearing payload".
    /// The first real write replaces it via `module_binding_write_kinded`.
    #[inline]
    pub(crate) fn module_binding_pad_to_kinded(&mut self, index: usize) {
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
            self.module_binding_kinds.push(NativeKind::Bool);
        }
        debug_assert_eq!(
            self.module_bindings.len(),
            self.module_binding_kinds.len(),
            "ADR-006 §2.7.8 / Q10 lockstep invariant",
        );
    }

    /// Write a fresh kinded value into `module_bindings[index]`,
    /// releasing the previous occupant via `drop_with_kind` (ADR-006
    /// §2.7.8 / Q10 retain-on-overwrite). Mirrors `stack_write_kinded`
    /// in `vm_impl/stack.rs:374`.
    ///
    /// **Ownership**: the new slot owns the strong-count share
    /// transferred in by the caller. The caller MUST have retained the
    /// share before calling (e.g. via `clone_with_kind` on the source
    /// slot, or a fresh `Arc::into_raw`). The previous occupant's
    /// share is released here and MUST NOT be accessed by the caller
    /// afterwards.
    #[inline]
    pub(crate) fn module_binding_write_kinded(
        &mut self,
        index: usize,
        bits: u64,
        kind: NativeKind,
    ) {
        self.module_binding_pad_to_kinded(index);
        let old_bits = self.module_bindings[index];
        let old_kind = self.module_binding_kinds[index];
        vm_impl::stack::drop_with_kind(old_bits, old_kind);
        self.module_bindings[index] = bits;
        self.module_binding_kinds[index] = kind;
    }

    /// Read the raw bits + kind at `module_bindings[index]` as a borrow
    /// (no refcount change). The slot retains ownership of the share;
    /// the caller MUST NOT drop the returned bits. Mirrors
    /// `stack_read_kinded_raw` in `vm_impl/stack.rs:366`.
    ///
    /// Returns `(0, NativeKind::Bool)` for indices past the vec end —
    /// matches the legacy `Vec<u64>` "uninitialised reads as zero"
    /// behaviour with the no-op-on-drop kind sentinel paired in.
    #[inline]
    pub(crate) fn module_binding_read_kinded_raw(&self, index: usize) -> (u64, NativeKind) {
        if index >= self.module_bindings.len() {
            return (0u64, NativeKind::Bool);
        }
        debug_assert_eq!(
            self.module_bindings.len(),
            self.module_binding_kinds.len(),
            "ADR-006 §2.7.8 / Q10 lockstep invariant violated at \
             module_binding_read_kinded_raw",
        );
        // The lockstep invariant has been observed; if the kinds vec
        // is short the bounded access falls back to the no-op sentinel
        // rather than panicking on a release build.
        let kind = self
            .module_binding_kinds
            .get(index)
            .copied()
            .unwrap_or(NativeKind::Bool);
        (self.module_bindings[index], kind)
    }

    /// Read an **owning share** of `module_bindings[index]` as a
    /// `KindedSlot`. Bumps the underlying `Arc<T>` strong-count via
    /// `clone_with_kind` so the returned `KindedSlot` has an
    /// independent share; the binding slot itself stays live. Mirrors
    /// `read_owned_kinded` in `vm_impl/stack.rs:354`.
    ///
    /// Use this at every site that hands a binding to a runtime-tier
    /// `KindedSlot` carrier (host-API `module_bindings()` enumeration,
    /// snapshot serialisation, etc.).
    #[inline]
    pub(crate) fn module_binding_read_owned_kinded(&self, index: usize) -> KindedSlot {
        let (bits, kind) = self.module_binding_read_kinded_raw(index);
        vm_impl::stack::clone_with_kind(bits, kind);
        KindedSlot::new(shape_value::ValueSlot::from_raw(bits), kind)
    }

    /// Take ownership of `module_bindings[index]`, replacing it with
    /// the zero/Bool sentinel. Does NOT drop — the caller owns the
    /// returned bits. Mirrors `stack_take_kinded` in
    /// `vm_impl/stack.rs:385`.
    #[inline]
    pub(crate) fn module_binding_take_kinded(&mut self, index: usize) -> (u64, NativeKind) {
        if index >= self.module_bindings.len() {
            return (0u64, NativeKind::Bool);
        }
        let bits = self.module_bindings[index];
        let kind = self
            .module_binding_kinds
            .get(index)
            .copied()
            .unwrap_or(NativeKind::Bool);
        self.module_bindings[index] = Self::NONE_BITS;
        if index < self.module_binding_kinds.len() {
            self.module_binding_kinds[index] = NativeKind::Bool;
        }
        (bits, kind)
    }

    /// Length of the module-binding store (lockstep-checked).
    #[inline]
    pub(crate) fn module_bindings_len(&self) -> usize {
        debug_assert_eq!(
            self.module_bindings.len(),
            self.module_binding_kinds.len(),
            "ADR-006 §2.7.8 / Q10 lockstep invariant",
        );
        self.module_bindings.len()
    }
}

/// Replace the active wire transport provider used by VM transport builtins.
pub fn set_transport_provider(
    provider: std::sync::Arc<dyn builtins::transport_provider::WireTransportProvider>,
) {
    builtins::transport_provider::set_transport_provider(provider);
}

/// Restore the default shape-wire transport provider.
pub fn reset_transport_provider() {
    builtins::transport_provider::reset_transport_provider();
}

/// Configure global QUIC settings used by `transport.quic()`.
#[cfg(feature = "quic")]
pub fn configure_quic_transport(
    server_name: String,
    root_certs_der: Vec<Vec<u8>>,
    connect_timeout: Option<std::time::Duration>,
) {
    builtins::transport_provider::configure_quic_transport(
        server_name,
        root_certs_der,
        connect_timeout,
    );
}

/// Clear global QUIC settings used by `transport.quic()`.
#[cfg(feature = "quic")]
pub fn clear_quic_transport_config() {
    builtins::transport_provider::clear_quic_transport_config();
}

/// Create the VM-backed `transport` module exports.
pub(crate) fn create_transport_module_exports() -> shape_runtime::module_exports::ModuleExports {
    builtins::transport_builtins::create_transport_module()
}

/// Create the VM-backed `remote` module exports.
pub(crate) fn create_remote_module_exports() -> shape_runtime::module_exports::ModuleExports {
    builtins::remote_builtins::create_remote_module()
}

/// Remap constant and string pool indices in a single instruction operand after
/// a hot-patch splice. `const_offset` and `string_offset` are the starting
/// indices in the global pools where the blob's local pools were appended.
fn remap_operand(operand: &mut Option<Operand>, const_offset: usize, string_offset: usize) {
    let Some(op) = operand.as_mut() else {
        return;
    };
    match op {
        Operand::Const(idx) => {
            *idx = (*idx as usize + const_offset) as u16;
        }
        Operand::Property(idx) => {
            *idx = (*idx as usize + string_offset) as u16;
        }
        Operand::Name(sid) => {
            sid.0 = (sid.0 as usize + string_offset) as u32;
        }
        Operand::TypedMethodCall { string_id, .. } => {
            *string_id = (*string_id as usize + string_offset) as u16;
        }
        // Other operands (Local, ModuleBinding, Offset, Function, Builtin,
        // Count, ColumnIndex, TypedField, TypedObjectAlloc, TypedMerge,
        // ColumnAccess, ForeignFunction) don't reference the constant or
        // string pools.
        _ => {}
    }
}

#[cfg(test)]
mod v2_stack_tests;
