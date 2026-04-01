//! Virtual machine executor for Shape bytecode

// Opcode category implementations (split into submodules)
mod additional;
mod arithmetic;
mod async_ops;
mod builtins;
mod call_convention;
mod comparison;
mod control_flow;
mod dispatch;
mod exceptions;
pub(crate) mod ic_fast_paths;
mod jit_ops;
mod logical;
mod loops;
mod objects;
mod osr;
mod resume;
mod snapshot;
mod stack_ops;
pub mod state_builtins;
pub mod time_travel;
mod trait_object_ops;
mod v2_handlers;
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

/// Result of VM execution
#[derive(Debug, Clone)]
pub enum ExecutionResult {
    /// Execution completed normally with a ValueWord value
    Completed(ValueWord),
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
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueSlot, ValueWord};
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
    /// Upvalues captured by this closure (None for regular functions)
    pub upvalues: Option<Vec<shape_value::Upvalue>>,
    /// Content hash of the function blob being executed (for content-addressed state capture).
    /// `None` for programs compiled without content-addressed metadata.
    pub blob_hash: Option<FunctionHash>,
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

    /// Unified value stack (pre-allocated, NaN-boxed: 8 bytes per slot).
    /// Locals live in register windows on this stack.
    stack: Vec<ValueWord>,

    /// Stack pointer — logical top of the value stack.
    /// `stack[0..sp]` are live values; `stack[sp..]` is pre-allocated dead space.
    pub(crate) sp: usize,

    /// ModuleBinding variables (NaN-boxed for compact storage)
    pub(crate) module_bindings: Vec<ValueWord>,

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
    last_uncaught_exception: Option<ValueWord>,

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

    /// Table of ModuleFn closures indexed by usize ID.
    /// ValueWord::ModuleFunction(id) references this table for dispatch.
    module_fn_table: Vec<shape_runtime::module_exports::ModuleFn>,

    /// Runtime function name → index lookup for UFCS dispatch.
    /// Populated after program load. Used by handle_object_method to find
    /// type-scoped impl methods (e.g., "DuckDbQuery::filter") at runtime.
    function_name_index: HashMap<String, u16>,

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
    pub(crate) pending_resume: Option<ValueWord>,

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
}

/// Data for resuming a single call frame mid-function.
pub(crate) struct FrameResumeData {
    /// IP offset within the function to resume at.
    pub ip_offset: usize,
    /// Locals to restore in the resumed frame.
    pub locals: Vec<ValueWord>,
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
