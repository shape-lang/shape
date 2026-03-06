//! Type definitions for the bytecode-to-IR translator

use cranelift::codegen;
use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use std::collections::HashMap;

use super::storage::TypedStack;
use crate::context::SimulationKernelConfig;
use crate::optimizer::FunctionOptimizationPlan;
use shape_vm::bytecode::{BytecodeProgram, DeoptInfo};
use shape_vm::feedback::FeedbackVector;
use shape_vm::type_tracking::{SlotKind, StorageHint};

/// Compilation mode for BytecodeToIR
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CompilationMode {
    /// Standard mode: uses ctx_ptr for all access (JITContext-based)
    #[default]
    Standard,
    /// Kernel mode: uses (cursor_index, series_ptrs, state_ptr) directly
    /// Bypasses JITContext for maximum simulation throughput
    Kernel,
}

/// FFI function references for heap operations
#[allow(dead_code)]
pub struct FFIFuncRefs {
    pub(crate) new_array: FuncRef,
    pub(crate) new_object: FuncRef,
    pub(crate) get_prop: FuncRef,
    pub(crate) set_prop: FuncRef,
    pub(crate) length: FuncRef,
    pub(crate) array_get: FuncRef,
    pub(crate) call_function: FuncRef,
    pub(crate) call_value: FuncRef,
    pub(crate) call_foreign: FuncRef,
    pub(crate) call_foreign_native: FuncRef,
    pub(crate) call_foreign_dynamic: FuncRef,
    pub(crate) call_foreign_native_0: FuncRef,
    pub(crate) call_foreign_native_1: FuncRef,
    pub(crate) call_foreign_native_2: FuncRef,
    pub(crate) call_foreign_native_3: FuncRef,
    pub(crate) call_foreign_native_4: FuncRef,
    pub(crate) call_foreign_native_5: FuncRef,
    pub(crate) call_foreign_native_6: FuncRef,
    pub(crate) call_foreign_native_7: FuncRef,
    pub(crate) call_foreign_native_8: FuncRef,
    pub(crate) iter_next: FuncRef,
    pub(crate) iter_done: FuncRef,
    pub(crate) call_method: FuncRef,
    pub(crate) type_of: FuncRef,
    pub(crate) type_check: FuncRef,
    // Result type operations (Ok/Err)
    pub(crate) make_ok: FuncRef,
    pub(crate) make_err: FuncRef,
    pub(crate) is_ok: FuncRef,
    pub(crate) is_err: FuncRef,
    pub(crate) is_result: FuncRef,
    pub(crate) unwrap_ok: FuncRef,
    pub(crate) unwrap_err: FuncRef,
    pub(crate) unwrap_or: FuncRef,
    pub(crate) result_inner: FuncRef,
    // Option type operations (Some/None)
    pub(crate) make_some: FuncRef,
    pub(crate) is_some: FuncRef,
    pub(crate) is_none: FuncRef,
    pub(crate) unwrap_some: FuncRef,
    pub(crate) array_first: FuncRef,
    pub(crate) array_last: FuncRef,
    pub(crate) array_min: FuncRef,
    pub(crate) array_max: FuncRef,
    pub(crate) slice: FuncRef,
    pub(crate) range: FuncRef,
    pub(crate) make_range: FuncRef,
    pub(crate) to_string: FuncRef,
    pub(crate) to_number: FuncRef,
    pub(crate) print: FuncRef,
    pub(crate) sin: FuncRef,
    pub(crate) cos: FuncRef,
    pub(crate) tan: FuncRef,
    pub(crate) asin: FuncRef,
    pub(crate) acos: FuncRef,
    pub(crate) atan: FuncRef,
    pub(crate) exp: FuncRef,
    pub(crate) ln: FuncRef,
    pub(crate) log: FuncRef,
    pub(crate) pow: FuncRef,
    pub(crate) control_fold: FuncRef,
    pub(crate) control_reduce: FuncRef,
    pub(crate) control_map: FuncRef,
    pub(crate) control_filter: FuncRef,
    pub(crate) control_foreach: FuncRef,
    pub(crate) control_find: FuncRef,
    pub(crate) control_find_index: FuncRef,
    pub(crate) control_some: FuncRef,
    pub(crate) control_every: FuncRef,
    pub(crate) array_push: FuncRef,
    pub(crate) array_pop: FuncRef,
    pub(crate) array_push_elem: FuncRef,
    pub(crate) array_push_local: FuncRef,
    pub(crate) array_reserve_local: FuncRef,
    pub(crate) array_zip: FuncRef,
    pub(crate) array_filled: FuncRef,
    pub(crate) array_reverse: FuncRef,
    pub(crate) array_push_element: FuncRef,
    pub(crate) make_closure: FuncRef,
    pub(crate) eval_datetime_expr: FuncRef,
    pub(crate) eval_time_reference: FuncRef,
    pub(crate) eval_data_datetime_ref: FuncRef,
    pub(crate) eval_data_relative: FuncRef,
    pub(crate) intrinsic_series: FuncRef,
    pub(crate) series_method: FuncRef,
    pub(crate) format_error: FuncRef,
    pub(crate) object_rest: FuncRef,
    pub(crate) format: FuncRef,
    pub(crate) generic_add: FuncRef,
    pub(crate) generic_sub: FuncRef,
    pub(crate) generic_mul: FuncRef,
    pub(crate) generic_div: FuncRef,
    pub(crate) series_shift: FuncRef,
    pub(crate) series_fillna: FuncRef,
    pub(crate) series_rolling_mean: FuncRef,
    pub(crate) series_rolling_sum: FuncRef,
    pub(crate) series_rolling_std: FuncRef,
    pub(crate) intrinsic_rolling_std: FuncRef,
    pub(crate) series_cumsum: FuncRef,
    pub(crate) series_gt: FuncRef,
    pub(crate) series_lt: FuncRef,
    pub(crate) series_gte: FuncRef,
    pub(crate) series_lte: FuncRef,
    pub(crate) intrinsic_sum: FuncRef,
    pub(crate) intrinsic_mean: FuncRef,
    pub(crate) intrinsic_min: FuncRef,
    pub(crate) intrinsic_max: FuncRef,
    pub(crate) intrinsic_std: FuncRef,
    pub(crate) intrinsic_variance: FuncRef,
    pub(crate) intrinsic_median: FuncRef,
    pub(crate) intrinsic_percentile: FuncRef,
    pub(crate) intrinsic_correlation: FuncRef,
    pub(crate) intrinsic_covariance: FuncRef,
    // Vector intrinsics (boxed interface - legacy)
    pub(crate) intrinsic_vec_abs: FuncRef,
    pub(crate) intrinsic_vec_sqrt: FuncRef,
    pub(crate) intrinsic_vec_ln: FuncRef,
    pub(crate) intrinsic_vec_exp: FuncRef,
    pub(crate) intrinsic_vec_add: FuncRef,
    pub(crate) intrinsic_vec_sub: FuncRef,
    pub(crate) intrinsic_vec_mul: FuncRef,
    pub(crate) intrinsic_vec_div: FuncRef,
    pub(crate) intrinsic_vec_max: FuncRef,
    pub(crate) intrinsic_vec_min: FuncRef,
    pub(crate) intrinsic_matmul_vec: FuncRef,
    pub(crate) intrinsic_matmul_mat: FuncRef,

    // Raw pointer SIMD operations (zero-copy, high performance)
    // Binary: simd_op(ptr_a, ptr_b, len) -> result_ptr
    pub(crate) simd_add: FuncRef,
    pub(crate) simd_sub: FuncRef,
    pub(crate) simd_mul: FuncRef,
    pub(crate) simd_div: FuncRef,
    pub(crate) simd_max: FuncRef,
    pub(crate) simd_min: FuncRef,
    // Scalar broadcast: simd_op_scalar(ptr, scalar, len) -> result_ptr
    pub(crate) simd_add_scalar: FuncRef,
    pub(crate) simd_sub_scalar: FuncRef,
    pub(crate) simd_mul_scalar: FuncRef,
    pub(crate) simd_div_scalar: FuncRef,
    // Comparison: simd_cmp(ptr_a, ptr_b, len) -> mask_ptr (1.0/0.0)
    pub(crate) simd_gt: FuncRef,
    pub(crate) simd_lt: FuncRef,
    pub(crate) simd_gte: FuncRef,
    pub(crate) simd_lte: FuncRef,
    pub(crate) simd_eq: FuncRef,
    pub(crate) simd_neq: FuncRef,
    // Memory management
    pub(crate) simd_free: FuncRef,

    pub(crate) series_rolling_min: FuncRef,
    pub(crate) series_rolling_max: FuncRef,
    pub(crate) series_ema: FuncRef,
    pub(crate) series_diff: FuncRef,
    pub(crate) series_pct_change: FuncRef,
    pub(crate) series_cumprod: FuncRef,
    pub(crate) series_clip: FuncRef,
    pub(crate) series_broadcast: FuncRef,
    pub(crate) series_highest_index: FuncRef,
    pub(crate) series_lowest_index: FuncRef,
    pub(crate) time_current_time: FuncRef,
    pub(crate) time_symbol: FuncRef,
    pub(crate) time_last_row: FuncRef,
    pub(crate) time_range: FuncRef,
    pub(crate) get_all_rows: FuncRef,
    pub(crate) align_series: FuncRef,
    pub(crate) run_simulation: FuncRef,
    // Generic DataFrame access (industry-agnostic)
    pub(crate) get_field: FuncRef,
    pub(crate) get_row_ref: FuncRef,
    pub(crate) row_get_field: FuncRef,
    pub(crate) get_row_timestamp: FuncRef,
    // Type-specialized field access (JIT optimization)
    pub(crate) get_field_typed: FuncRef,
    pub(crate) set_field_typed: FuncRef,
    // TypedObject allocation
    pub(crate) typed_object_alloc: FuncRef,
    // TypedObject merge (O(1) memcpy-based)
    pub(crate) typed_merge_object: FuncRef,
    // Typed column access (Arrow-backed LoadCol* opcodes)
    pub(crate) load_col_f64: FuncRef,
    pub(crate) load_col_i64: FuncRef,
    pub(crate) load_col_bool: FuncRef,
    pub(crate) load_col_str: FuncRef,
    // GC safepoint poll (called at loop headers)
    pub(crate) gc_safepoint: FuncRef,
    // Reference operations
    pub(crate) set_index_ref: FuncRef,
    // Array info (data_ptr, length) extraction via stable Rust API
    pub(crate) array_info: FuncRef,
    // HOF inlining array operations
    pub(crate) hof_array_alloc: FuncRef,
    pub(crate) hof_array_push: FuncRef,
    // Async task operations
    pub(crate) spawn_task: FuncRef,
    pub(crate) join_init: FuncRef,
    pub(crate) join_await: FuncRef,
    pub(crate) cancel_task: FuncRef,
    pub(crate) async_scope_enter: FuncRef,
    pub(crate) async_scope_exit: FuncRef,
    // Shape guard operations (HashMap hidden class)
    pub(crate) hashmap_shape_id: FuncRef,
    pub(crate) hashmap_value_at: FuncRef,
    // Generic builtin trampoline (handles any builtin not lowered by dedicated JIT paths)
    pub(crate) generic_builtin: FuncRef,
}

/// Information about a function eligible for inlining at call sites.
///
/// Straight-line functions (no branches, no loops) with < 80 instructions
/// are considered inline candidates. Non-leaf functions (with calls) are
/// allowed — nested calls are handled recursively up to depth 4.
#[derive(Debug, Clone)]
pub(crate) struct InlineCandidate {
    /// Entry point in program.instructions
    pub entry_point: usize,
    /// Number of instructions in the function body
    pub instruction_count: usize,
    /// Number of parameters
    pub arity: u16,
    /// Number of local variables
    pub locals_count: u16,
}

/// Describes a per-guard spill block to be emitted at function epilogue.
///
/// When a speculative guard fails, the JIT needs to store all live locals
/// and operand stack values to ctx_buf so the VM can reconstruct the
/// interpreter frame at the exact guard failure point. Each `DeferredSpill`
/// describes one such spill block.
#[derive(Clone)]
pub(crate) struct DeferredSpill {
    /// The Cranelift block for this spill point.
    pub block: Block,
    /// Deopt ID to pass to the shared deopt block.
    pub deopt_id: u32,
    /// Live locals at the guard point: (bytecode_idx, Cranelift Variable).
    pub live_locals: Vec<(u16, Variable)>,
    /// SlotKind for each live local (parallel to live_locals).
    pub local_kinds: Vec<SlotKind>,
    /// Number of operand stack entries already on the JIT stack (via stack_vars).
    pub on_stack_count: usize,
    /// Number of extra values passed as block params (pre-popped operands).
    pub extra_param_count: usize,
    /// Locals that hold raw f64 values (from float-unboxed loops).
    /// These need `bitcast(I64, f64_val)` before storing to ctx_buf.
    pub f64_locals: std::collections::HashSet<u16>,
    /// Locals that hold raw i64 values (from integer-unboxed loops).
    /// These are stored directly to ctx_buf (raw i64 fits in u64).
    pub int_locals: std::collections::HashSet<u16>,
    /// Inline frames for multi-frame deopt (innermost-first).
    /// Each entry contains the caller frame's live locals to spill.
    pub inline_frames: Vec<DeferredInlineFrame>,
}

/// A caller frame's spill data for multi-frame inline deopt.
#[derive(Clone)]
pub(crate) struct DeferredInlineFrame {
    /// Live locals for this caller frame: (ctx_buf_position, Cranelift Variable).
    pub live_locals: Vec<(u16, Variable)>,
    /// SlotKind for each live local (parallel to live_locals).
    pub local_kinds: Vec<SlotKind>,
    /// Locals that hold raw f64 values.
    pub f64_locals: std::collections::HashSet<u16>,
    /// Locals that hold raw i64 values.
    pub int_locals: std::collections::HashSet<u16>,
}

/// Context for an inline frame, pushed onto a stack during inlining.
///
/// Captures the caller's state at the point of the inline call so that
/// multi-frame deopt can reconstruct the full call stack.
#[derive(Clone)]
pub(crate) struct InlineFrameContext {
    /// Function ID of the caller (from bytecode).
    pub function_id: u16,
    /// Function ID of the callee being inlined.
    /// Used by deeper inline levels to identify their caller.
    pub callee_fn_id: u16,
    /// Bytecode IP of the CallValue/Call in the caller.
    pub call_site_ip: usize,
    /// Caller's live locals at call site: (bytecode_idx, Cranelift Variable).
    pub locals_snapshot: Vec<(u16, Variable)>,
    /// SlotKind for each local in the snapshot.
    pub local_kinds: Vec<SlotKind>,
    /// Caller's operand stack depth at call site.
    pub stack_depth: usize,
    /// Caller's f64-unboxed locals at call site.
    pub f64_locals: std::collections::HashSet<u16>,
    /// Caller's int-unboxed locals at call site.
    pub int_locals: std::collections::HashSet<u16>,
}

/// Loop context for Break/Continue handling
pub(super) struct LoopContext {
    pub(super) start_block: Block,
    pub(super) end_block: Block,
}

/// Tracks which locals were newly unboxed at a specific loop nesting level.
/// Used for scope-stacked unboxing: each nested loop pushes a scope with its
/// delta (newly unboxed locals), and pops it on exit to rebox only those locals.
pub(crate) struct UnboxedScope {
    /// Locals newly unboxed as raw i64 at this loop level (delta from outer scope).
    pub int_locals: std::collections::HashSet<u16>,
    /// Locals newly unboxed as raw f64 at this loop level (delta from outer scope).
    pub f64_locals: std::collections::HashSet<u16>,
    /// Module bindings newly unboxed as raw i64 at this loop level.
    pub int_module_bindings: std::collections::HashSet<u16>,
    /// The loop_stack.len() when this scope was opened.
    pub depth: usize,
}

/// Information for 4x loop unrolling at the back-edge.
///
/// Set at LoopStart for eligible loops, consumed at the back-edge Jump.
/// The back-edge emits 3 additional body copies with intermediate bounds
/// checks, turning each header iteration into 4 body executions.
pub(crate) struct UnrollInfo {
    /// First body instruction to re-compile (after JumpIfFalse)
    pub body_start: usize,
    /// Last body instruction (exclusive, the Jump back-edge itself)
    pub body_end: usize,
    /// Induction variable local slot
    pub iv_slot: u16,
    /// Loop bound local slot
    pub bound_slot: u16,
    /// Comparison condition (e.g., SignedLessThan for `i < n`)
    pub bound_cmp: IntCC,
    /// Unroll factor (2, 4, ...). A value of 1 means no unrolling.
    pub factor: u8,
}

/// Bytecode to IR compiler helper
pub struct BytecodeToIR<'a, 'b> {
    pub(crate) builder: &'a mut FunctionBuilder<'b>,
    pub(crate) program: &'a BytecodeProgram,
    pub(crate) ctx_ptr: Value,
    pub(crate) stack_depth: usize,
    pub(crate) stack_vars: HashMap<usize, Variable>,
    pub(crate) locals: HashMap<u16, Variable>,
    pub(crate) next_var: usize,
    pub(crate) blocks: HashMap<usize, Block>,
    pub(crate) current_block_idx: usize,
    pub(crate) ffi: FFIFuncRefs,
    pub(super) loop_stack: Vec<LoopContext>,
    pub(crate) loop_ends: HashMap<usize, usize>,
    pub(crate) exit_block: Option<Block>,
    pub(crate) compile_time_sp: usize,
    pub(crate) merge_blocks: std::collections::HashSet<usize>,
    pub(crate) block_stack_depth: HashMap<usize, usize>,
    pub(crate) pending_data_offset: Option<Value>,
    pub(crate) exception_handlers: Vec<usize>,
    pub(crate) current_instr_idx: usize,
    /// User-defined function references for direct JIT calls (bypasses FFI overhead)
    pub(crate) user_funcs: HashMap<u16, FuncRef>,
    /// User-defined function arity by function id.
    /// Used by direct-call lowering to validate call-site argc against the
    /// callee's fixed signature.
    pub(crate) user_func_arities: HashMap<u16, u16>,
    /// Stack type tracking for typed compilation
    /// Maps stack position to known StorageHint (if type is known at compile time)
    /// Enables optimizations like NaN-sentinel operations for Option<f64>
    pub(crate) stack_types: HashMap<usize, StorageHint>,
    /// Local variable type tracking
    /// Maps local variable index to StorageHint (tracked when storing)
    pub(crate) local_types: HashMap<u16, StorageHint>,
    /// Module-binding storage hints (index -> StorageHint).
    pub(crate) module_binding_types: HashMap<u16, StorageHint>,
    /// Typed stack for StorageType-aware compilation
    /// Tracks both values and their CraneliftRepr for unboxed operations
    pub(crate) typed_stack: TypedStack,

    // ========================================================================
    // Kernel Mode Fields (only used when mode == Kernel)
    // ========================================================================
    /// Compilation mode: Standard (JITContext) or Kernel (direct pointers)
    pub(crate) mode: CompilationMode,
    /// Kernel mode: cursor index (current row in simulation)
    pub(crate) kernel_cursor_index: Option<Value>,
    /// Kernel mode: pointer to series data pointers (Vec<*const f64>)
    pub(crate) kernel_series_ptrs: Option<Value>,
    /// Kernel mode: pointer to state buffer (TypedObject)
    pub(crate) kernel_state_ptr: Option<Value>,
    /// Kernel mode: configuration for column/field mappings
    #[allow(dead_code)]
    pub(crate) kernel_config: Option<SimulationKernelConfig>,
    /// Loop analysis results for optimization (LICM, bounds hoisting, etc.)
    pub(crate) loop_info: HashMap<usize, super::loop_analysis::LoopInfo>,
    /// Phase-based optimization plan built from strict static analysis.
    pub(crate) optimization_plan: FunctionOptimizationPlan,
    /// Hoisted loop-invariant locals: maps local index to pre-loaded Cranelift Value.
    /// Populated at LoopStart, consulted by LoadLocal to avoid redundant memory loads.
    pub(crate) hoisted_locals: HashMap<u16, Value>,
    /// Cache of f64 Cranelift Values for local variables.
    /// Populated by StoreLocal when the value has repr F64, used by LoadLocal
    /// to skip redundant i64->f64 bitcasts. Cleared at block boundaries.
    pub(crate) local_f64_cache: HashMap<u16, Value>,

    // ========================================================================
    // Function Inlining Fields
    // ========================================================================
    /// Inline candidate functions (fn_id → candidate info)
    pub(crate) inline_candidates: HashMap<u16, InlineCandidate>,
    /// Base offset for local variable remapping during inlining.
    /// When > 0, all local accesses are offset to avoid caller/callee collisions.
    pub(crate) inline_local_base: u16,
    /// Current inlining depth (to prevent deep nesting)
    pub(crate) inline_depth: u8,

    // ========================================================================
    // Reference Tracking
    // ========================================================================
    /// Maps local variable index to the stack slot allocated by MakeRef.
    /// After any function call, referenced locals must be reloaded from their
    /// stack slots because the callee may have modified the value through the reference.
    pub(crate) ref_stack_slots: HashMap<u16, codegen::ir::StackSlot>,

    // ========================================================================
    // Integer Unboxing (Sprint 5.1 — Native Integer Arithmetic)
    // ========================================================================
    /// Locals currently holding raw i64 values (within an integer-unboxed loop).
    /// NOT cleared at block boundaries — persists for the entire loop scope.
    /// Empty outside of unboxed loops.
    pub(crate) unboxed_int_locals: std::collections::HashSet<u16>,

    /// Module bindings currently promoted to Cranelift Variables holding raw i64.
    /// Maps module binding index → Cranelift Variable for register-promoted access.
    /// Empty outside of unboxed loops.
    pub(crate) unboxed_int_module_bindings: std::collections::HashSet<u16>,

    /// Cranelift Variables created for promoted module bindings.
    /// During an unboxed loop, Load/StoreModuleBinding uses these Variables
    /// instead of memory loads/stores.
    pub(crate) promoted_module_bindings: HashMap<u16, Variable>,

    /// Non-unboxed module bindings promoted to loop-carried registers.
    /// These stay boxed but avoid repeated ctx.locals[] loads/stores in hot loops.
    pub(crate) register_carried_module_bindings: std::collections::HashSet<u16>,

    /// The loop_stack depth at which integer unboxing was activated.
    /// Only the loop at this depth should rebox locals on exit.
    /// Prevents nested inner loops from prematurely clearing the outer loop's unboxed state.
    pub(crate) unboxed_loop_depth: usize,

    /// Scope stack for nested loop unboxing. Each entry records the delta
    /// (newly unboxed locals) at a specific loop nesting level. On loop exit,
    /// only the delta locals are reboxed, preserving outer loop's unboxed state.
    pub(crate) unboxed_scope_stack: Vec<UnboxedScope>,

    /// The loop_stack depth at which loop-carried module-binding promotion was activated.
    pub(crate) register_carried_loop_depth: usize,

    /// Pending rebox at loop exit: set of locals that must be converted from
    /// raw i64 back to NaN-boxed at the start of the loop's end_block.
    /// Set by compile_loop_end, consumed by the main compile loop at the next block switch.
    pub(crate) pending_rebox: Option<std::collections::HashSet<u16>>,

    /// Pending rebox for module bindings at loop exit.
    pub(crate) pending_rebox_module_bindings: Option<std::collections::HashSet<u16>>,

    /// Pending flush of boxed, register-carried module bindings at loop exit.
    pub(crate) pending_flush_module_bindings: Option<std::collections::HashSet<u16>>,

    // ========================================================================
    // Float Unboxing (Sprint 5.2 — Float Loop Variables)
    // ========================================================================
    /// Locals currently holding raw f64 values (within a float-unboxed loop).
    /// NOT cleared at block boundaries — persists for the entire loop scope.
    /// Empty outside of unboxed loops.
    pub(crate) unboxed_f64_locals: std::collections::HashSet<u16>,

    /// Cranelift Variables (typed as F64) created for float-unboxed locals.
    /// Maps local index → f64-typed Cranelift Variable.
    pub(crate) f64_local_vars: HashMap<u16, Variable>,

    /// Pending rebox of f64 locals at loop exit: convert raw f64 → NaN-boxed i64.
    pub(crate) pending_rebox_f64: Option<std::collections::HashSet<u16>>,

    // ========================================================================
    // Invariant IntToNumber Hoisting (LICM for int→f64 conversions)
    // ========================================================================
    /// Pre-converted f64 values for loop-invariant int locals that feed IntToNumber.
    /// Maps local index → f64-typed Cranelift Variable holding the precomputed f64.
    /// When IntToNumber encounters one of these locals, it uses the precomputed
    /// value instead of re-emitting fcvt_from_sint every iteration.
    pub(crate) precomputed_f64_for_invariant_int: HashMap<u16, Variable>,

    /// Scope stack tracking which locals were precomputed at each loop nesting level.
    /// Popped at compile_loop_end to remove entries from precomputed_f64_for_invariant_int.
    pub(crate) precomputed_f64_scope_stack: Vec<Vec<u16>>,

    // ========================================================================
    // Skip Ranges (for main function compilation — skip function bodies)
    // ========================================================================
    /// Instruction index ranges to skip during compilation.
    /// Used when compiling the main strategy to exclude function body instructions
    /// that are compiled separately via `compile_function_with_user_funcs`.
    pub(crate) skip_ranges: Vec<(usize, usize)>,

    // ========================================================================
    // Array LICM (Loop-Invariant Code Motion for Array Access)
    // ========================================================================
    /// Hoisted array data pointers for invariant locals used as arrays.
    /// Maps local index → (data_ptr, length) extracted once at loop entry.
    /// Eliminates redundant tag checks + pointer extraction per iteration.
    pub(crate) hoisted_array_info: HashMap<u16, (Value, Value)>,

    /// Hoisted reference-array data pointers for invariant ref locals used in SetIndexRef.
    /// Maps ref_slot local index → (data_ptr, length) from deref + extraction at loop entry.
    /// Eliminates redundant deref + tag check + pointer extraction per iteration.
    pub(crate) hoisted_ref_array_info: HashMap<u16, (Value, Value)>,

    /// Function parameters inferred as numeric by local bytecode analysis.
    /// These are used as compile-time hints for LoadLocal typed-stack tracking.
    pub(crate) numeric_param_hints: std::collections::HashSet<u16>,

    /// Shared deopt block (when emitted by guarded helper paths).
    /// Branching here marks the function result signal as negative.
    pub(crate) deopt_block: Option<Block>,
    /// Signal variable returned by the function:
    /// 0 = success, negative = early-exit/deopt requested.
    pub(crate) deopt_signal_var: Option<Variable>,

    /// Collected deopt points emitted during compilation.
    /// Each entry describes how to reconstruct interpreter state when a
    /// speculative guard fails at a specific program point.
    pub(crate) deopt_points: Vec<DeoptInfo>,

    /// Function locals count (from bytecode function metadata).
    /// Used by emit_deopt_point_with_spill to compute stack-value bc_idx offsets.
    pub(crate) func_locals_count: u16,

    /// Deferred spill blocks to be emitted at function epilogue.
    /// Each entry describes a per-guard spill block that stores live
    /// locals + operand stack values to ctx_buf before jumping to the
    /// shared deopt block.
    pub(crate) deferred_spills: Vec<DeferredSpill>,

    /// Pending loop unroll info: set at LoopStart, consumed at the back-edge Jump.
    /// When present, the back-edge emits 3 extra body copies for 4x unrolling.
    pub(crate) pending_unroll: Option<UnrollInfo>,

    /// ArrayPushLocal instruction sites proven safe for unchecked inline push.
    pub(crate) trusted_array_push_local_sites: std::collections::HashSet<usize>,

    /// Trusted ArrayPushLocal sites that can index directly with a loop IV.
    /// Maps instruction index -> IV local slot.
    pub(crate) trusted_array_push_local_iv_by_site: HashMap<usize, u16>,

    // ========================================================================
    // Shape Guard Tracking (HashMap hidden classes)
    // ========================================================================
    /// Shape IDs that were guarded during compilation.
    /// Used to register shape dependencies with the DeoptTracker so that
    /// shape transitions can invalidate stale JIT code.
    pub(crate) shape_guards_emitted: Vec<shape_value::shape_graph::ShapeId>,

    // ========================================================================
    // Feedback-Guided Speculation (Tier 2 JIT)
    // ========================================================================
    /// Feedback vector snapshot from the interpreter's type profiling.
    /// When present (Tier 2 compilation), enables speculative IR emission:
    /// - Monomorphic call sites → direct call + callee guard
    /// - Monomorphic property access → guarded field load
    /// - Stable arithmetic → typed fast path with type guard
    pub(crate) feedback: Option<FeedbackVector>,

    // ========================================================================
    // Multi-Frame Inline Deopt
    // ========================================================================
    /// The bytecode function ID of the function currently being compiled.
    /// Set by `compile_optimizing_function` and used to tag inline frame
    /// contexts with the correct caller function_id.
    pub(crate) compiling_function_id: u16,

    /// Stack of inline frame contexts for multi-frame deopt.
    /// Pushed when entering an inline call, popped when exiting.
    /// Used to reconstruct the full call stack on guard failure inside inlined code.
    pub(crate) inline_frame_stack: Vec<InlineFrameContext>,
}
