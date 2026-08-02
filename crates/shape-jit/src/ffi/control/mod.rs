// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 2 sites
//     jit_box(HK_ARRAY, ...) — jit_control_map, jit_control_filter
//   Category B (intermediate/consumed): 3 sites
//     Vec::with_capacity for args in jit_call_value, jit_call_foreign_impl,
//       jit_call_foreign_native_args_fixed (consumed within call, not escaped)
//     Arc::new in error path of jit_call_foreign_impl (returned as ValueWord)
//   Category C (heap islands): 0 sites (jit_control_map results — fixed via write barrier)
//!
//! Control Flow FFI Functions for JIT
//!
//! Higher-order functions (fold, reduce, map, filter, forEach) and function call helpers
//! for JIT-compiled code.

use crate::context::JITContext;
// crate::jit_array::JitArray removed — see jit_array.rs SURFACE comment.
// Higher-order array-walk FFI functions below now route to surface-and-stop
// per ADR-006 §2.7.4 / W10 jit-playbook §5; the kinded rebuild reads the
// receiver as `Arc<TypedArrayData>` per-element-kind arm (§2.7.6/Q8).
#[allow(unused_imports)]
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;
use std::ffi::c_void;

// ============================================================================
// Trampoline VM — thread-local VirtualMachine for JIT-to-VM fallback
// ============================================================================

use shape_value::encoding::ERROR_PLACEHOLDER_BITS;
use std::cell::Cell;

thread_local! {
    /// Pointer to a fully-initialized VirtualMachine for executing bytecode
    /// functions that weren't JIT-compiled. Set by `execute_with_jit()` before
    /// JIT execution and cleared after. Valid only on the executor thread.
    static TRAMPOLINE_VM: Cell<*mut shape_vm::VirtualMachine> = const { Cell::new(std::ptr::null_mut()) };

    /// `r5c-2-bz-b-jit-err-surface`: error message from the most recent
    /// VM-trampoline FFI call (`jit_call_method`) whose VM-side handler
    /// returned `Err`.
    ///
    /// When a trampoline FFI call hits a VM `Err`, it stores the error message
    /// here and sets `JITContext.pending_call_error = 1`. The MIR emitter
    /// loads that flag right after the FFI call and deopts (returns
    /// `SIGNAL_TRAMPOLINE_ERROR`) — the JIT frame is abandoned before the
    /// FFI's placeholder return value reaches a heap-kinded refcount-retain
    /// site. `JITExecutor` then `take`s this message and surfaces it as the
    /// program's runtime error — identical to the error the VM produces, so
    /// VM and JIT modes agree.
    static JIT_RUNTIME_ERROR: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Record a VM-trampoline error message for the surrounding JIT execution to
/// surface on deopt. Called from `jit_call_method`'s `Err` arm. Overwrites any
/// prior message (the most recent error is the one that triggers the deopt).
pub fn set_jit_runtime_error(message: String) {
    JIT_RUNTIME_ERROR.with(|cell| *cell.borrow_mut() = Some(message));
}

/// Take (and clear) the recorded VM-trampoline error message. Called by
/// `JITExecutor` when a JIT-compiled function returns a negative signal, so
/// the clean VM error can be surfaced in place of a generic JIT error code.
pub fn take_jit_runtime_error() -> Option<String> {
    JIT_RUNTIME_ERROR.with(|cell| cell.borrow_mut().take())
}

fn raise_trampoline_error(jit_ctx: *mut JITContext, message: String) {
    set_jit_runtime_error(message);
    if !jit_ctx.is_null() {
        unsafe { (*jit_ctx).pending_call_error = 1 };
    }
}

/// Register the trampoline VM for use during JIT execution.
///
/// # Safety
/// The pointer must remain valid for the entire duration of JIT execution.
/// Caller must clear it with `unset_trampoline_vm()` after execution.
pub unsafe fn set_trampoline_vm(vm: *mut shape_vm::VirtualMachine) {
    TRAMPOLINE_VM.with(|cell| cell.set(vm));
}

/// Clear the trampoline VM pointer after JIT execution.
pub fn unset_trampoline_vm() {
    TRAMPOLINE_VM.with(|cell| cell.set(std::ptr::null_mut()));
}

/// Access the trampoline VM for read-only queries (schema lookups, etc.)
pub fn with_trampoline_vm<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&shape_vm::VirtualMachine) -> R,
{
    TRAMPOLINE_VM.with(|cell| {
        let vm_ptr = cell.get();
        if vm_ptr.is_null() {
            None
        } else {
            Some(f(unsafe { &*vm_ptr }))
        }
    })
}

/// Execute a closure with mutable access to the trampoline VM.
pub fn with_trampoline_vm_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut shape_vm::VirtualMachine) -> R,
{
    TRAMPOLINE_VM.with(|cell| {
        let vm_ptr = cell.get();
        if vm_ptr.is_null() {
            None
        } else {
            Some(f(unsafe { &mut *vm_ptr }))
        }
    })
}

/// Dispatch a function call through the trampoline VM for functions that
/// aren't JIT-compiled (null entries in the function table).
///
/// `upvalue_bits` carries the closure's captures when the callee is a
/// closure (either VM-format heap or unified-heap `JITClosure`). When the
/// callee is a bare function (TAG_FUNCTION inline), pass `None` to dispatch
/// through `call_value_immediate_nb` with a plain function ValueWord.
///
/// When captures are present we route through `jit_trampoline_call_closure`
/// on the interpreter side, which binds them to the callee frame's
/// upvalues exactly as the `op_call_closure` path does. Without this
/// path, a closure that fails JIT compilation (null entry in the function
/// table) would be reconstructed as a bare function, losing its captures
/// and producing `Null` on return.
fn dispatch_call_via_trampoline_vm(
    function_id: u32,
    upvalue_pairs: Option<&[(u64, shape_value::NativeKind)]>,
    arg_pairs: &[(u64, shape_value::NativeKind)],
    jit_ctx: *mut JITContext,
) -> u64 {
    use shape_value::NativeKind;

    // §2.7.5 stable-FFI raw-pair shape: each arg / capture pair is
    // `(u64, NativeKind)`, and the kind is the one the PRODUCING site
    // stamped into `JITContext.stack_kinds` — read back out by
    // `jit_call_value` in lockstep with the bits (§2.7.7 / Q9).
    //
    // #188: this function used to discard the caller's kinds and
    // re-stamp every argument `NativeKind::UInt64`. That is the
    // "I64-wide raw bits without further classification" carrier kind,
    // which is correct for a function-id-shaped CALLEE and wrong for an
    // ordinary argument: the callee's frame receives the arg's kind
    // verbatim through `stack_write_kinded`, so an `f64` argument
    // arrived labelled as an integer and the callee's first typed read
    // converted the raw bit pattern instead of the value. Measured at
    // e12c82d2 with a native `apply(demoted, 0.5)` — VM `Ok(0.5)`,
    // JIT `Ok(4602678819172647000)` (= the raw f64 bits of 0.5 read as
    // an int); a `string` argument produced the same class with its
    // `Arc<String>` pointer. Both are the c4-4B silent-wrong-output
    // class. The kinds exist at every push site, so the fix is to carry
    // them, not to re-derive or default them.
    //
    // The kind companion is consumed by `jit_trampoline_call_closure`
    // which wraps each pair as a `KindedSlot` and threads it into the
    // new frame's locals via `stack_write_kinded`, and by
    // `call_value_immediate_nb` for the bare-function shape.
    //
    // #117 / R15: the covered-fallback dispatch event. Reaching this function
    // means a native frame handed `function_id` to the bytecode interpreter,
    // which is exactly the observation that must never be relabelled native.
    shape_vm::native_witness::record_interpreter_dispatch(function_id as usize);

    with_trampoline_vm_mut(|vm| {
        let func_id = function_id as u16;
        match upvalue_pairs {
            Some(capture_pairs) => {
                // Shape 2 / 3: closure-with-captures. Route through
                // `jit_trampoline_call_closure` which materializes a
                // fresh `OwnedClosureBlock` from the capture pairs and
                // dispatches via `call_closure_with_nb_args_keepalive`.
                match vm.jit_trampoline_call_closure(func_id, capture_pairs, arg_pairs, None) {
                    Ok(bits) => bits,
                    Err(e) => {
                        raise_trampoline_error(jit_ctx, e.to_string());
                        TAG_NULL
                    }
                }
            }
            None => {
                // Shape 1: bare function callee (no captures). Use the
                // VM's `call_value_immediate_nb` with a `NativeKind::
                // UInt64` callee — the §2.7.11 callee-classification
                // kind for function-id-shaped callees (per
                // `call_convention.rs:853-877` UInt64 arm).
                use shape_value::{KindedSlot, ValueSlot};
                let callee =
                    KindedSlot::new(ValueSlot::from_raw(func_id as u64), NativeKind::UInt64);
                let kinded_args: Vec<KindedSlot> = arg_pairs
                    .iter()
                    .map(|(bits, kind)| KindedSlot::new(ValueSlot::from_raw(*bits), *kind))
                    .collect();
                match vm.call_value_immediate_nb(&callee, &kinded_args, None) {
                    Ok(result) => {
                        let bits = result.slot.raw();
                        // The result's strong-count share transfers to
                        // the JIT-side stack slot via the return path.
                        // `mem::forget` prevents the KindedSlot's Drop
                        // from retiring the share — the caller's stack
                        // slot now owns it (same pattern as the runtime
                        // tier's `dispatch_call_value_immediate` per
                        // §2.7.11/Q12).
                        std::mem::forget(result);
                        // The callee KindedSlot was constructed with
                        // raw bits (no Arc share); its Drop is a no-op
                        // for UInt64 kind. Same for the arg KindedSlots
                        // in this legacy function-id path: the stable
                        // FFI boundary carries raw I64-wide bits, and the
                        // VM call has already copied them into the new
                        // frame.
                        std::mem::forget(callee);
                        std::mem::forget(kinded_args);
                        bits
                    }
                    Err(e) => {
                        raise_trampoline_error(jit_ctx, e.to_string());
                        TAG_NULL
                    }
                }
            }
        }
    })
    .unwrap_or_else(|| {
        // `TRAMPOLINE_VM` is null — the JIT-compiled callee could not be
        // dispatched. Raise `pending_call_error` so the MIR-emitted check
        // deopts rather than continuing with a value-shaped placeholder.
        raise_trampoline_error(
            jit_ctx,
            format!(
                "JIT value-call for function {} could not reach the interpreter \
             trampoline",
                function_id,
            ),
        );
        TAG_NULL
    })
}

/// Maximum `[captures..., args...]` count the native closure-call path can
/// enter directly. Bounded by `call_jit_fn_with_args`'s transmute table, which
/// covers `fn(ctx)` through `fn(ctx, a0..a7)`; anything wider takes the
/// interpreter.
const MAX_NATIVE_CLOSURE_CALL_ARGS: usize = 8;

/// Dispatch a raw-Arc closure callee through the trampoline VM by borrowing
/// its existing `OwnedClosureBlock`.
///
/// `jit_trampoline_call_closure` constructs a fresh owning block from raw
/// capture bits. That is not valid for cell-storage captures:
/// `OwnedMutable` / `Shared` slots hold transfer-only cell pointers owned by
/// the original closure block.
fn dispatch_borrowed_closure_via_trampoline_vm(
    closure_block: &shape_value::v2::closure_raw::OwnedClosureBlock,
    arg_pairs: &[(u64, shape_value::NativeKind)],
    jit_ctx: *mut JITContext,
) -> u64 {
    use shape_value::{KindedSlot, ValueSlot};

    // #117 / R15 (#188 close): the raw-Arc closure trampoline is a
    // covered-fallback dispatch event exactly like
    // `dispatch_call_via_trampoline_vm`, and it was the only trampoline
    // entry that did not announce one. Without this, a program whose
    // closure body runs entirely on the interpreter reported
    // `disposition: installed-not-dispatched` with 0 interpreter
    // dispatches — a witness that cannot distinguish "never called" from
    // "called, but never native", which is precisely the relabelling R15
    // forbids. Measured at e12c82d2 with a closure passed as a parameter
    // and called 200 times: `__closure_0` showed 0/0.
    //
    // SAFETY: `closure_block` is a live `OwnedClosureBlock` borrowed from
    // the caller's `Arc<HeapValue::ClosureRaw>`; `as_ptr()` addresses its
    // `TypedClosureHeader` per the construction invariant.
    let closure_function_id =
        unsafe { shape_value::v2::closure_raw::typed_closure_function_id(closure_block.as_ptr()) };

    // ── #188 slice 2: native closure dispatch from the trampoline ────────
    //
    // The MIR emitter's guarded fast path (`terminators.rs`) can only pin a
    // closure whose `MakeClosureHeap` it can see in the SAME function. A
    // closure that arrives as a PARAMETER — `fn apply(g: (int) => int, n: int)
    // { g(n) }`, the shape every higher-order function has — has no such
    // definition to speculate from, and the emitter cannot even know the
    // capture count, so it cannot build a call signature.
    //
    // Here that information is all available dynamically: the block carries
    // its `function_id`, and its `ClosureLayout` carries the capture count,
    // per-capture offsets and widths. So this is where the parameter shape
    // gets its native call. If the closure body was JIT-compiled, marshal
    // `[captures..., args...]` and enter it through the same native ABI the
    // emitter's direct path uses (`fn(ctx, param_0..param_{arity-1})` with
    // params [0..captures_count) being the captures, per
    // `compiler/function_abi.rs`).
    //
    // Declined, each falling through to the interpreter below:
    //   * cell-storage captures (`OwnedMutable` / `Shared`) — those slots hold
    //     transfer-only cell pointers the callee reads through its
    //     closure-self opcodes, not as leading params, so they do not fit
    //     this ABI;
    //   * arity above `call_jit_fn_with_args`'s supported range;
    //   * a null function-table entry (body not compiled — #187 demotion).
    //
    // Capture bits come from `read_capture_kinded`, which reads at the slot's
    // own `FieldKind` width with the per-kind sign/zero extension that
    // `write_capture_typed` round-trips — the same widening the emitter's
    // `widen_to_i64` performs, and never a blanket 8-byte read that would
    // overrun a narrow trailing capture.
    //
    // Reads are borrows: the shares stay owned by the block, which the caller
    // holds live across this call.
    {
        let layout = closure_block.layout();
        let no_cell_captures =
            layout.owned_mutable_capture_mask == 0 && layout.shared_capture_mask == 0;
        let total_args = layout.capture_count() + arg_pairs.len();
        if no_cell_captures && total_args <= MAX_NATIVE_CLOSURE_CALL_ARGS && !jit_ctx.is_null() {
            let ctx_ref = unsafe { &mut *jit_ctx };
            if !ctx_ref.function_table.is_null()
                && (closure_function_id as usize) < ctx_ref.function_table_len
            {
                let raw_fn_ptr = unsafe {
                    *(ctx_ref.function_table as *const *const u8).add(closure_function_id as usize)
                };
                if !raw_fn_ptr.is_null() {
                    let mut native_args: Vec<u64> = Vec::with_capacity(total_args);
                    for i in 0..layout.capture_count() {
                        let (bits, _kind) = unsafe { closure_block.read_capture_kinded(i) };
                        native_args.push(bits);
                    }
                    native_args.extend(arg_pairs.iter().map(|(bits, _)| *bits));

                    ctx_ref.stack_ptr = 0;
                    let signal =
                        unsafe { call_jit_fn_with_args(raw_fn_ptr, jit_ctx, &native_args) };
                    if signal >= 0 && ctx_ref.stack_ptr > 0 {
                        ctx_ref.stack_ptr -= 1;
                        let ret_bits = ctx_ref.stack[ctx_ref.stack_ptr];
                        ctx_ref.stack_kinds[ctx_ref.stack_ptr] =
                            crate::ffi::stack_kind_code::SENTINEL;
                        return ret_bits;
                    }
                    // A negative signal is the callee's own deopt/error
                    // signal; it has already been surfaced through
                    // `pending_call_error` or is a hard JIT error. Report it
                    // rather than silently re-running the body on the
                    // interpreter, which would double any side effect the
                    // native attempt already performed.
                    raise_trampoline_error(
                        jit_ctx,
                        format!(
                            "JIT closure native dispatch for function {} returned signal {}",
                            closure_function_id, signal
                        ),
                    );
                    return TAG_NULL;
                }
            }
        }
    }

    // #117 / R15 (#188 close): the raw-Arc closure trampoline is a
    // covered-fallback dispatch event. Recorded only once the native path
    // above has declined — announcing it earlier would report an interpreter
    // dispatch for a call that ran natively.
    shape_vm::native_witness::record_interpreter_dispatch(closure_function_id as usize);

    let kinded_args: Vec<KindedSlot> = arg_pairs
        .iter()
        .map(|(bits, kind)| KindedSlot::new(ValueSlot::from_raw(*bits), *kind))
        .collect();

    match with_trampoline_vm_mut(|vm| vm.execute_closure(closure_block, kinded_args, None)) {
        Some(Ok(result)) => {
            let bits = result.slot.raw();
            // The returned share transfers to the JIT-side destination slot.
            std::mem::forget(result);
            bits
        }
        Some(Err(e)) => {
            raise_trampoline_error(jit_ctx, e.to_string());
            TAG_NULL
        }
        None => {
            raise_trampoline_error(
                jit_ctx,
                "JIT closure value-call could not reach the interpreter trampoline".to_string(),
            );
            TAG_NULL
        }
    }
}

/// Dispatch a native module function call through the trampoline VM.
// Pending phase-2c kinded-handler ABI rebuild (body is `todo!`); not yet
// dispatched. Kept as the named landing point for that work.
#[allow(dead_code)]
fn dispatch_module_fn_call(_module_fn_id: u32, _jit_args: &[u64], _ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.10/Q11: JIT-side kinded handler ABI rebuild — \
         dispatch_module_fn_call. ModuleFunction callee construction and \
         the call_value_immediate_nb dispatch shell now take &KindedSlot \
         per ADR-006 §2.7.10/Q11; the deleted ValueWord::from_module_function \
         constructor needs a kinded replacement at the producing call \
         signature per §2.7.5. See \
         docs/cluster-audits/wave-10-jit-playbook.md §5."
    )
}

/// Call a function by function_id
/// Stack reads args from ctx.stack before the call
pub extern "C" fn jit_call_function(
    ctx: *mut JITContext,
    function_id: u16,
    _args: *const u64, // deprecated, pass null
    _arg_count: usize,
) -> u64 {
    unsafe {
        if ctx.is_null() {
            // #234 B1: unreachable absent a JIT codegen bug, and there is no
            // context to record `pending_call_error` in — the context IS what
            // is null. Returns the placeholder, memory-safe by #234.
            return ERROR_PLACEHOLDER_BITS;
        }
        let ctx_ref = &mut *ctx;

        // Check if we have a function table
        if ctx_ref.function_table.is_null() || (function_id as usize) >= ctx_ref.function_table_len
        {
            // #234 c1: corrupted JIT state — record the error so the caller
            // deopts to the interpreter instead of computing on garbage.
            crate::ffi::control::set_jit_runtime_error(
                "jit_call_function: function table missing or function id out of range — deopting to interpreter".to_string(),
            );
            ctx_ref.pending_call_error = 1;
            return ERROR_PLACEHOLDER_BITS;
        }

        // Get the function pointer
        let fn_ptr = *ctx_ref.function_table.add(function_id as usize);

        // The function reads its args from the stack (already pushed by caller)
        // and returns result on the stack
        let _result_code = fn_ptr(ctx);

        // Pop result from stack
        if ctx_ref.stack_ptr > 0 {
            ctx_ref.stack_ptr -= 1;
            ctx_ref.stack[ctx_ref.stack_ptr]
        } else {
            TAG_NULL
        }
    }
}

/// Call a closure or function value through the trampoline VM.
///
/// Stack layout (set by MIR `TerminatorKind::Call` lowering in
/// `mir_compiler/terminators.rs`):
/// ```text
///   [..., callee_bits, arg0_bits, arg1_bits, ..., argN-1_bits, arg_count]
///                                                                       ^ ctx.stack_ptr
/// ```
/// `arg_count` is a raw `i64` (not NaN-boxed) per the MIR-side
/// `iconst(types::I64, args.len() as i64)` push at terminators.rs:681.
///
/// ## Callee classification (JIT-internal NaN-box, NOT deleted ValueWord)
///
/// Per ADR-006 §2.7.5 the JIT-internal NaN-box scheme in
/// `crates/shape-jit/src/ffi/value_ffi.rs` is the JIT's own value
/// representation — it is NOT the deleted runtime-tier `tag_bits`
/// dispatch (CLAUDE.md "Forbidden Patterns" #4 enumerates the deleted
/// ValueWord synthesizer / `is_tagged()` runtime handlers / runtime
/// return-kind stamp family). The JIT-internal predicates
/// (`is_inline_function`, `is_heap_kind`) operate on the JIT's own
/// slot encoding and are intentionally preserved.
///
/// Three callee shapes flow through `jit_call_value` today:
///
///   1. **Inline function** (`box_function(fn_id)` → `TAG_FUNCTION_BITS`
///      tag): classified by `is_inline_function(callee_bits)`, function-
///      id recovered by `unbox_function_id(callee_bits)`. The JIT MIR
///      emitter pushes this shape when the callee operand is a bare
///      `FunctionRef` constant.
///
///   2. **Raw-Arc closure** (`NativeKind::Ptr(HeapKind::Closure)`):
///      the producing site stamps the callee kind in the JIT stack kind
///      track. The bits are `Arc::into_raw(Arc<HeapValue::ClosureRaw>)`;
///      this path borrows the existing `OwnedClosureBlock` into the
///      trampoline VM.
///
///   3. **Deprecated `unified_box(HK_CLOSURE, JITClosure)` callees**:
///      classified by `is_heap_kind(callee_bits, HK_CLOSURE)`. This is
///      the legacy `jit_make_closure` FFI return shape.
///
/// ## Raw-Arc closure kind sourcing (§2.7.5)
///
/// `jit_finalize_heap_closure` (the current preferred closure path)
/// returns `Arc::into_raw(Arc::new(HeapValue::ClosureRaw(owned))) as u64`
/// — a raw Arc pointer, not a NaN-boxed value. There is no tag-bit
/// signature on the bits themselves; the callee's `NativeKind::Ptr(
/// HeapKind::Closure)` is supplied by the producing site at JIT compile
/// time and lives in the `JITContext.stack_kinds` lockstep side track.
///
/// The callee kind is intentionally NOT recovered from `callee_bits` via
/// `is_heap()` / `is_tagged()` probes. Those predicates are valid for the
/// JIT-internal NaN-boxed shapes above, but not for raw Arc pointers.
/// Dispatch follows the compile-time kind stamp, then only uses the
/// inline-function predicate as the documented zero-capture dual-carrier
/// check before dereferencing raw-Arc bits.
///
/// ## Argument kind sourcing
///
/// Indirect-call lowering writes each argument's producing-site
/// `NativeKind` into `JITContext.stack_kinds` beside the raw `u64` bits.
/// Raw-Arc closure dispatch preserves those pairs into
/// `VirtualMachine::execute_closure`; legacy bare-function / `JITClosure`
/// fallback still uses `NativeKind::UInt64` at the stable FFI boundary
/// because that path has only raw function-id-class bits.
///
/// ## Forbidden alternatives (refuse on sight)
///
/// - **Decoding callee kind from `callee_bits` via tag-bit probe** —
///   §2.7.7 #4 / #7 / CLAUDE.md "Forbidden Patterns" #4.
/// - **Bool-default kind for args/callee** — §2.7.7 #9 / CLAUDE.md
///   "Forbidden rationalizations" ("Soft-fail counter for now, harden
///   later" — the W11 round-1 walk-back precedent).
/// - **Silent no-op of the function-id call path** — the supervisor
///   explicitly refused the W11 round-1 walk-back of `jit_arc_retain` /
///   `jit_arc_release` to silent no-ops; the same discipline applies
///   here (ADR-006 §2.7.14 "Reopen amendment").
/// - **Resurrecting `ValueWord::clone_from_bits` /
///   `value_word_drop::vw_drop`** — CLAUDE.md "Forbidden Patterns" #1.
pub extern "C" fn jit_call_value(ctx: *mut JITContext) -> u64 {
    use crate::context::JITClosure;
    use crate::ffi::jit_kinds::unified_unbox;
    use crate::ffi::stack_kind_code;
    use shape_value::{HeapKind, NativeKind, heap_value::HeapValue};
    use std::sync::Arc;

    unsafe {
        if ctx.is_null() {
            // #234 B1: unreachable absent a JIT codegen bug, and there is no
            // context to record `pending_call_error` in — the context IS what
            // is null. Returns the placeholder, memory-safe by #234.
            return ERROR_PLACEHOLDER_BITS;
        }
        let ctx_ref = &mut *ctx;

        // Pop arg_count (raw i64 per the MIR-side `iconst(I64,
        // args.len() as i64)` push at terminators.rs). The parallel-kind
        // track byte at this slot is `NativeKind::UInt64` (the documented
        // §2.7.11 / §2.7.5 I64-wide raw bits carrier kind for FFI
        // scalar sentinels) per the producing emit_kind_track_write call.
        if ctx_ref.stack_ptr == 0 {
            tracing::debug!(
                target: "shape_jit",
                "jit-call-value BAIL: stack_ptr=0 at arg_count pop",
            );
            // #234 c1: corrupted JIT state — record the error so the caller
            // deopts to the interpreter instead of computing on garbage.
            crate::ffi::control::set_jit_runtime_error(
                "jit_call_value: operand stack empty while popping arg_count — deopting to interpreter".to_string(),
            );
            ctx_ref.pending_call_error = 1;
            return ERROR_PLACEHOLDER_BITS;
        }
        ctx_ref.stack_ptr -= 1;
        let arg_count = ctx_ref.stack[ctx_ref.stack_ptr] as usize;
        // Reset the kind byte sentinel for hygiene (matches the VM
        // `pop_kinded` "write Bool sentinel on dead slot" discipline at
        // `vm_impl/stack.rs:706`).
        ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;

        // Pop args together with their parallel-track kinds (reverse
        // stack order, then reverse to source order). The §2.7.7 / Q9
        // lockstep invariant: each `(bits, kind)` pair is read from the
        // same slot index.
        let mut arg_pairs: Vec<(u64, NativeKind)> = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if ctx_ref.stack_ptr == 0 {
                // #234 c1: corrupted JIT state — record the error so the caller
                // deopts to the interpreter instead of computing on garbage.
                crate::ffi::control::set_jit_runtime_error(
                    "jit_call_value: operand stack exhausted mid-argument-pop — deopting to interpreter".to_string(),
                );
                ctx_ref.pending_call_error = 1;
                return ERROR_PLACEHOLDER_BITS;
            }
            ctx_ref.stack_ptr -= 1;
            let bits = ctx_ref.stack[ctx_ref.stack_ptr];
            let code = ctx_ref.stack_kinds[ctx_ref.stack_ptr];
            ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
            // Decode the kind from the parallel track. `None` is a
            // kind-source gap (§2.7.7 #9) — surface, do not Bool-default.
            let kind = match stack_kind_code::decode(code) {
                Some(k) => k,
                None => {
                    tracing::debug!(
                        target: "shape_jit",
                        code,
                        stack_ptr = ctx_ref.stack_ptr,
                        "jit-call-value SURFACE \u{a7}2.7.7 / Q9: arg \
                         kind-byte is SENTINEL / reserved. The producing \
                         call site at `mir_compiler/terminators.rs` must \
                         stamp a concrete NativeKind for every push (no \
                         Bool-default fallback per \u{a7}2.7.7 #9).",
                    );
                    // #259: a stop that returns a legal value does not stop.
                    set_jit_runtime_error(
                        "jit_call_value: arg kind-byte is SENTINEL/reserved — a producing-site kind \
                         stamp is missing; native execution aborted"
                            .to_string(),
                    );
                    ctx_ref.pending_call_error = 1;
                    return ERROR_PLACEHOLDER_BITS;
                }
            };
            arg_pairs.push((bits, kind));
        }
        arg_pairs.reverse();

        // Pop callee together with its parallel-track kind. The kind IS
        // the §2.7.11/Q12 callee-classification discriminator — no tag-
        // bit decode on `callee_bits`, no `is_heap()` probe (§2.7.7 #4 /
        // #7 forbidden).
        if ctx_ref.stack_ptr == 0 {
            // #234 c1: corrupted JIT state — record the error so the caller
            // deopts to the interpreter instead of computing on garbage.
            crate::ffi::control::set_jit_runtime_error(
                "jit_call_value: operand stack empty while popping the callee — deopting to interpreter".to_string(),
            );
            ctx_ref.pending_call_error = 1;
            return ERROR_PLACEHOLDER_BITS;
        }
        ctx_ref.stack_ptr -= 1;
        let callee_bits = ctx_ref.stack[ctx_ref.stack_ptr];
        let callee_code = ctx_ref.stack_kinds[ctx_ref.stack_ptr];
        ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
        let callee_kind = match stack_kind_code::decode(callee_code) {
            Some(k) => k,
            None => {
                tracing::debug!(
                    target: "shape_jit",
                    callee_code,
                    stack_ptr = ctx_ref.stack_ptr,
                    "jit-call-value SURFACE \u{a7}2.7.7 / Q9: callee kind-byte \
                     is SENTINEL / reserved. The producing call site must \
                     stamp the callee's NativeKind from `operand_slot_kind` \
                     per ADR-006 \u{a7}2.7.11 / Q12. No Bool-default fallback \
                     (\u{a7}2.7.7 #9).",
                );
                // #259: a stop that returns a legal value does not stop.
                set_jit_runtime_error(
                    "jit_call_value: callee kind-byte is SENTINEL/reserved — a producing-site \
                     kind stamp is missing; native execution aborted"
                        .to_string(),
                );
                ctx_ref.pending_call_error = 1;
                return ERROR_PLACEHOLDER_BITS;
            }
        };

        // ── Dispatch on callee kind (§2.7.11 / Q12) ──────────────────────
        //
        // Mirror of the VM-side `dispatch_call_value_immediate` in
        // `crates/shape-vm/src/executor/control_flow/mod.rs:389`. The
        // callee kind classifies the dispatch shape:
        //
        // - `Ptr(HeapKind::Closure)`: raw `Arc::into_raw(Arc<HeapValue::
        //   ClosureRaw>)` slot bits (the `jit_finalize_heap_closure`
        //   return shape). Recover the `OwnedClosureBlock` via the
        //   `Arc<HeapValue>` slot-tier convention and borrow that block
        //   directly into the trampoline VM. Re-materializing an owning
        //   block from raw captures is unsound for cell-storage captures
        //   (`OwnedMutable` / `Shared`).
        //
        // - `UInt64` / `Int64` / `IntSize` / `UIntSize`: function-id
        //   class kind (the §2.7.5 I64-wide raw bits carrier kind also
        //   used for inline function refs whose bits hold a NaN-boxed
        //   `TAG_FUNCTION` value). Pass through to the trampoline VM's
        //   `call_value_immediate_nb` function-id path.
        //
        // - Anything else: surface — the language doesn't have other
        //   callable kinds at the indirect-call entry yet.
        //
        // Cases 1 and 2 below are the legacy bit-shape predicates we
        // preserved through W11-jit-carrier-conversion. They fire only
        // when the stamped kind is the generic `UInt64` / `Int64`
        // carrier kind (so the producing site didn't stamp a specific
        // closure or function-ref kind), and the bits themselves are a
        // JIT-internal NaN-box pattern (per `value_ffi.rs`). They're
        // shrunk to a narrow legacy compatibility surface; the principled
        // dispatch is by kind.
        let function_id: u16;
        let mut vm_captures: Option<Vec<(u64, NativeKind)>> = None;

        match callee_kind {
            NativeKind::Ptr(HeapKind::Closure) => {
                // Case 3 (closed): raw `Arc::into_raw(Arc<HeapValue::
                // ClosureRaw(OwnedClosureBlock)>)` callee bits. Per the
                // §2.7.11/Q12 slot-tier convention (W7 Round-2.5 close
                // `5fa4b19`), `clone_with_kind` / `drop_with_kind` for
                // `HeapKind::Closure` retain/release at the
                // `Arc<HeapValue>` shape; recover the `OwnedClosureBlock`
                // by going through `HeapValue::ClosureRaw`.
                if callee_bits == 0 {
                    tracing::debug!(
                        target: "shape_jit",
                        "jit-call-value BAIL \u{a7}2.7.11/Q12: callee \
                         stamped Ptr(HeapKind::Closure) but bits=0 \u{2014} \
                         producing site emitted a null callee.",
                    );
                    // #234 c1: corrupted JIT state — record the error so the caller
                    // deopts to the interpreter instead of computing on garbage.
                    crate::ffi::control::set_jit_runtime_error(
                        "jit_call_value: callee stamped Ptr(HeapKind::Closure) but bits are null — deopting to interpreter".to_string(),
                    );
                    ctx_ref.pending_call_error = 1;
                    return ERROR_PLACEHOLDER_BITS;
                }
                // W15.2-LANG-4 jit-filter-predicate close (2026-05-18).
                // Function-typed parameter slots (e.g. `apply(p: (int)=>bool,
                // ...)`'s `p`) are stamped `Ptr(HeapKind::Closure)` per
                // the declared `ConcreteType::Function` mapping in
                // `native_kind_from_concrete_type`. The producing call
                // signature `apply(pred, ...)` may deliver either runtime
                // carrier shape per the closure-zero-captures
                // optimization in the bytecode compiler:
                //
                //   (a) `Arc::into_raw(Arc<HeapValue::ClosureRaw(block)>)`
                //       — the §2.7.11/Q12 canonical heap-closure carrier
                //       (escaping closure with captures OR escaping
                //       closure without captures routed through
                //       `emit_heap_closure` + `jit_finalize_heap_closure`).
                //
                //   (b) `box_function(fn_id)` — the §2.7.11 NaN-box
                //       function-ref carrier (the bytecode compiler can
                //       emit `Operand::Function(fid)` for a `let pred =
                //       |x| x > 24` shape where `x > 24` has no captures
                //       AND the binding storage class permits the
                //       fn-ref-as-callable optimization).
                //
                // The carrier shape is determined at producing-site
                // codegen time but the declared-type-based kind
                // classification (`ConcreteType::Function` →
                // `Ptr(HeapKind::Closure)`) can't statically project the
                // carrier — the kind is the slot's *semantic* type, not
                // the runtime carrier discriminator. Dispatch on the
                // bit-shape predicate before falling through to the
                // `Arc::from_raw` deref to avoid UB on the NaN-box
                // carrier (which would deref random memory).
                if is_inline_function(callee_bits) {
                    function_id = unbox_function_id(callee_bits);
                    // No captures — bare function ref path.
                    let args: Vec<u64> = arg_pairs.iter().map(|(b, _)| *b).collect();
                    if !ctx_ref.function_table.is_null()
                        && (function_id as usize) < ctx_ref.function_table_len
                    {
                        let raw_fn_ptr =
                            *(ctx_ref.function_table as *const *const u8).add(function_id as usize);
                        if !raw_fn_ptr.is_null() {
                            ctx_ref.stack_ptr = 0;
                            let _signal = call_jit_fn_with_args(raw_fn_ptr, ctx, &args);
                            if ctx_ref.stack_ptr > 0 {
                                ctx_ref.stack_ptr -= 1;
                                let ret_bits = ctx_ref.stack[ctx_ref.stack_ptr];
                                ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
                                return ret_bits;
                            }
                            // #259 DELIBERATELY NOT CONVERTED — not a
                            // surface-and-stop path. Reaching here means the
                            // callee left no value on the stack, which for a
                            // unit-returning callee is CORRECT (ADR-020 §3.3:
                            // unit calls are void and the emit site discards
                            // the result). Setting `pending_call_error` here
                            // would deopt every unit-returning value call.
                            //
                            // `jit_call_value` cannot distinguish "unit callee
                            // returned nothing" from "non-unit callee failed to
                            // produce a value", because its `-> u64` signature
                            // carries no return-kind information — that is the
                            // channel defect #239 converts. Resolve there, when
                            // the void monomorph makes the distinction
                            // expressible in the signature.
                            //
                            // Separately suspicious and also #239's: `_signal`
                            // above discards the callee's JIT signal, so a
                            // callee that signalled an error is indistinguishable
                            // from one that returned nothing.
                            return TAG_NULL;
                        }
                    }
                    // Fall through to trampoline VM for the bare-fn case.
                    // #188: `arg_pairs` (not the kind-stripped `args`) —
                    // the trampoline threads each kind into the callee frame.
                    return dispatch_call_via_trampoline_vm(
                        function_id as u32,
                        None,
                        &arg_pairs,
                        ctx,
                    );
                }
                // Take ownership of the callee share that was pushed onto
                // the JIT stack. `compile_operand` retained for Copy
                // operands and transferred for Move operands; after the
                // call this dispatch frame retires that share. Holding the
                // Arc local across `execute_closure` keeps the borrowed
                // `OwnedClosureBlock` live for the VM call.
                let arc = Arc::<HeapValue>::from_raw(callee_bits as *const HeapValue);
                let result = match &*arc {
                    HeapValue::ClosureRaw(block) => {
                        dispatch_borrowed_closure_via_trampoline_vm(block, &arg_pairs, ctx)
                    }
                    other => {
                        // Wrong HeapValue arm under the stamped kind —
                        // a producing-site bug, not a tag-decode gap.
                        // Surface with diagnostic.
                        tracing::debug!(
                            target: "shape_jit",
                            heap_kind = ?other.kind(),
                            "jit-call-value SURFACE \u{a7}2.7.6/Q8: callee \
                             stamped Ptr(HeapKind::Closure) but HeapValue \
                             arm is not ClosureRaw. Producing site \
                             mislabeled the slot kind.",
                        );
                        // #259: a stop that returns a legal value does not
                        // stop. This arm is the measured mechanism of #219 —
                        // it fired 6,554 times in a single corpus program
                        // (`SYN__closure-infn-tagnull.shape`), each time
                        // handing `TAG_NULL` back as a usable number that the
                        // caller accumulated into an integer overflow.
                        set_jit_runtime_error(
                            "jit_call_value: callee stamped Ptr(HeapKind::Closure) but the \
                             HeapValue arm is not ClosureRaw; native execution aborted"
                                .to_string(),
                        );
                        ctx_ref.pending_call_error = 1;
                        ERROR_PLACEHOLDER_BITS
                    }
                };
                drop(arc);
                return result;
            }
            NativeKind::Ptr(HeapKind::ModuleFn) => {
                // ModuleFn callees flow through the comptime dispatch —
                // the §2.7.26 path. Not yet supported in the JIT-side
                // value-call surface; the bytecode compiler shouldn't
                // emit a top-level module-fn callee through this opcode
                // at present. Surface.
                tracing::debug!(
                    target: "shape_jit",
                    "jit-call-value SURFACE \u{a7}2.7.26: ModuleFn callee \
                     not implemented in jit_call_value.",
                );
                // #259: a stop that returns a legal value does not stop.
                set_jit_runtime_error(
                    "jit_call_value: ModuleFn callee is not implemented in the JIT value-call \
                     surface; native execution aborted"
                        .to_string(),
                );
                ctx_ref.pending_call_error = 1;
                return ERROR_PLACEHOLDER_BITS;
            }
            NativeKind::UInt64
            | NativeKind::Int64
            | NativeKind::IntSize
            | NativeKind::UIntSize
            | NativeKind::NullableUInt64
            | NativeKind::NullableInt64
            | NativeKind::NullableIntSize
            | NativeKind::NullableUIntSize => {
                // Generic I64-wide raw bits carrier kind (§2.7.5 / §2.7.11).
                // The bits hold either (a) a NaN-boxed inline function
                // ref (the JIT MIR emitter pushes `box_function(fn_id)`
                // when the callee is a `FunctionRef` constant), or (b)
                // a NaN-boxed `HK_CLOSURE` legacy unified-heap
                // `JITClosure` allocation. The JIT-internal NaN-box
                // predicates `is_inline_function` and
                // `is_heap_kind(_, HK_CLOSURE)` are intentionally
                // preserved here per ADR-006 §2.7.5 — they operate on
                // the JIT's own value representation, NOT on the
                // deleted runtime-tier `tag_bits` dispatch (CLAUDE.md
                // "Forbidden Patterns" #4 enumerates the deleted runtime
                // synthesizer / `is_tagged()` handlers; the JIT-internal
                // NaN-box checks in `value_ffi.rs` are a different
                // surface and remain valid).
                if is_inline_function(callee_bits) {
                    function_id = unbox_function_id(callee_bits);
                } else if is_heap_kind(callee_bits, HK_CLOSURE) {
                    let closure = unified_unbox::<JITClosure>(callee_bits);
                    function_id = closure.function_id;
                    let count = closure.captures_count as usize;
                    // Legacy `unified_box(HK_CLOSURE, JITClosure)` carrier
                    // (`jit_make_closure`, only reachable when the program
                    // carries no `ClosureLayout` for this function — see
                    // `mir_compiler/statements.rs` "LEGACY HEAP PATH"). That
                    // carrier has no parallel-kind track of its own: the
                    // `JITClosure.captures_ptr` array is bare `u64`. `UInt64`
                    // here is the §2.7.5 I64-wide-raw-bits carrier kind for
                    // that source, and it is the ONLY remaining kind-source
                    // gap on this path (#188 removed the argument one). It is
                    // not extended to arguments, which do have a kind track.
                    let mut caps: Vec<(u64, NativeKind)> = Vec::with_capacity(count);
                    for i in 0..count {
                        caps.push((*closure.captures_ptr.add(i), NativeKind::UInt64));
                    }
                    vm_captures = Some(caps);
                } else {
                    tracing::debug!(
                        target: "shape_jit",
                        callee_bits,
                        "jit-call-value SURFACE \u{a7}2.7.5: callee_bits \
                         stamped UInt64 but is neither inline function \
                         (TAG_FUNCTION) nor unified-heap HK_CLOSURE. \
                         Producing site stamped the carrier kind but \
                         emitted bits that don't match either UInt64-class \
                         shape.",
                    );
                    // #259: a stop that returns a legal value does not stop.
                    // This is the arm the investigator identified under gdb as
                    // #259's reproducer and #254's silent variant: it detects
                    // the carrier violation, describes it accurately, and then
                    // returned a usable number.
                    set_jit_runtime_error(
                        "jit_call_value: callee stamped UInt64 is neither an inline function \
                         nor a unified-heap closure; native execution aborted"
                            .to_string(),
                    );
                    ctx_ref.pending_call_error = 1;
                    return ERROR_PLACEHOLDER_BITS;
                }
            }
            other => {
                tracing::debug!(
                    target: "shape_jit",
                    kind = ?other,
                    "jit-call-value SURFACE \u{a7}2.7.11/Q12: callee kind \
                     is not a recognized callable kind. The \u{a7}2.7.11/Q12 \
                     callee-classification kinds at the indirect-call entry \
                     are Ptr(HeapKind::Closure) (raw-Arc closure shape), \
                     Ptr(HeapKind::ModuleFn) (deferred), and UInt64/Int64-\
                     family (function-id and JIT-internal NaN-box shapes).",
                );
                // #259: a stop that returns a legal value does not stop.
                set_jit_runtime_error(
                    "jit_call_value: callee kind is not a recognized callable kind; native \
                     execution aborted"
                        .to_string(),
                );
                ctx_ref.pending_call_error = 1;
                return ERROR_PLACEHOLDER_BITS;
            }
        }

        // Extract the raw arg bits for dispatch. Per-arg kinds are
        // already paired into `arg_pairs` and consumed inside the
        // trampoline VM as `KindedSlot` carriers (see
        // `dispatch_call_via_trampoline_vm`); we keep raw bits here for
        // the JIT function-table fast path which uses native Cranelift
        // call signatures (uniformly I64) and has no kind dependency.
        let args: Vec<u64> = arg_pairs.iter().map(|(b, _)| *b).collect();

        // ── Dispatch ─────────────────────────────────────────────────────

        // Try the JIT function table fast path first (no trampoline
        // hop). Only the bare-function shape can use this path —
        // closures need the trampoline VM for the captures-binding
        // semantics.
        if vm_captures.is_none()
            && !ctx_ref.function_table.is_null()
            && (function_id as usize) < ctx_ref.function_table_len
        {
            let raw_fn_ptr =
                *(ctx_ref.function_table as *const *const u8).add(function_id as usize);
            if !raw_fn_ptr.is_null() {
                // Reset ctx.stack_ptr so the callee starts with a clean
                // stack frame. The kind track is naturally re-initialized
                // by the callee's own push sequence — the §2.7.7 / Q9
                // lockstep invariant only constrains the live region of
                // the stack (`stack[..stack_ptr]`), not the dead region
                // beyond.
                ctx_ref.stack_ptr = 0;
                let _signal = call_jit_fn_with_args(raw_fn_ptr, ctx, &args);
                // Result is on ctx.stack[0..sp]; pop the top slot.
                if ctx_ref.stack_ptr > 0 {
                    ctx_ref.stack_ptr -= 1;
                    let ret_bits = ctx_ref.stack[ctx_ref.stack_ptr];
                    // Return-slot kind is consumed implicitly by the
                    // executor's RETURN_TAG_* dispatch (see
                    // `executor.rs::execute_with_jit`); we don't need
                    // to thread it back through `stack_kinds` because
                    // the calling MIR slot's kind is set by the
                    // destination write via `write_place`.
                    ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
                    return ret_bits;
                }
                // #259 DELIBERATELY NOT CONVERTED — sibling of the
                // function-table path above; same reasoning. "No value on the
                // stack" is a legitimate unit return, not a detected
                // violation, and the `-> u64` signature cannot express the
                // difference. Resolve in #239 with the void monomorph.
                return TAG_NULL;
            }
        }

        // Fallback: route through the trampoline VM. This handles:
        //   - JIT-untranslated function bodies (null function-table entry).
        //   - HK_CLOSURE callees (captures threaded into the new frame).
        //   - Raw-Arc HeapKind::Closure callees (Case 3 closed via the
        //     §2.7.11/Q12 kind dispatch above).
        let upvalues: Option<&[(u64, NativeKind)]> = vm_captures.as_deref();
        dispatch_call_via_trampoline_vm(function_id as u32, upvalues, &arg_pairs, ctx)
    }
}

/// Call a JIT-compiled function pointer with the right number of native arguments.
/// The function has Cranelift signature: fn(ctx_ptr: i64, arg0: i64, ...) -> i32
///
/// `pub(crate)` visibility: shared with `ffi/call_method/mod.rs::try_call_user_method`
/// per W14.2-E-followup soundness fix (2026-05-19) — trait-method UFCS user-callee
/// dispatch must invoke the JIT-compiled callee through the same native-ABI path
/// as `jit_call_value`'s bare-function fast path (line ~717). The prior shape that
/// called `fn_ptr(ctx)` directly under the `JittedStrategyFn` typedef silently
/// dropped every receiver/arg slot since the callee's extended Cranelift signature
/// `fn(ctx_ptr, arg0, ..., argN) -> i32` reads its params via System V register/
/// stack convention, NOT from `ctx.stack`. Per ADR-006 §2.7.10/Q11 the dispatch
/// shell sources every kind from the §2.7.7/Q9 parallel-kind track; the data half
/// flows through this helper's typed-fn transmute selector.
pub(crate) unsafe fn call_jit_fn_with_args(
    fn_ptr: *const u8,
    ctx: *mut JITContext,
    args: &[u64],
) -> i32 {
    type F0 = unsafe extern "C" fn(*mut JITContext) -> i32;
    type F1 = unsafe extern "C" fn(*mut JITContext, u64) -> i32;
    type F2 = unsafe extern "C" fn(*mut JITContext, u64, u64) -> i32;
    type F3 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64) -> i32;
    type F4 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64) -> i32;
    type F5 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64, u64) -> i32;
    type F6 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64, u64, u64) -> i32;
    type F7 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64, u64, u64, u64) -> i32;
    type F8 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64, u64, u64, u64, u64) -> i32;

    // SAFETY: callers pass a non-null JIT function-table entry compiled with
    // the Cranelift ABI shape selected by `args.len()`: `ctx` plus exactly
    // that many `u64` native arguments. Unsupported arities do not call
    // through the pointer.
    let result = unsafe {
        match args.len() {
            0 => std::mem::transmute::<_, F0>(fn_ptr)(ctx),
            1 => std::mem::transmute::<_, F1>(fn_ptr)(ctx, args[0]),
            2 => std::mem::transmute::<_, F2>(fn_ptr)(ctx, args[0], args[1]),
            3 => std::mem::transmute::<_, F3>(fn_ptr)(ctx, args[0], args[1], args[2]),
            4 => std::mem::transmute::<_, F4>(fn_ptr)(ctx, args[0], args[1], args[2], args[3]),
            5 => std::mem::transmute::<_, F5>(fn_ptr)(
                ctx, args[0], args[1], args[2], args[3], args[4],
            ),
            6 => std::mem::transmute::<_, F6>(fn_ptr)(
                ctx, args[0], args[1], args[2], args[3], args[4], args[5],
            ),
            7 => std::mem::transmute::<_, F7>(fn_ptr)(
                ctx, args[0], args[1], args[2], args[3], args[4], args[5], args[6],
            ),
            8 => std::mem::transmute::<_, F8>(fn_ptr)(
                ctx, args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
            ),
            _ => {
                // Too many args for direct dispatch — fall back to trampoline
                -1
            }
        }
    };
    result
}

/// fold(array, initial, fn) - left fold over array
///
/// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): walked the deleted
/// `JitArray` heap layout (`from_heap_bits`). Kinded rebuild reads
/// `Arc<TypedArrayData>` per-element-kind arm (§2.7.6/Q8) and threads
/// the per-element kind into the callback dispatch per §2.7.5.
pub extern "C" fn jit_control_fold(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_control_fold. The deleted UnifiedArray-walk decoded element \
         bits without per-element NativeKind tracking; the kinded rebuild \
         reads Arc<TypedArrayData> per ADR-006 §2.7.6/Q8 and dispatches \
         the callback through the §2.7.10/Q11 kinded handler ABI."
    )
}

/// reduce(array, fn, initial) - reduce array to single value
pub extern "C" fn jit_control_reduce(ctx: *mut JITContext) -> u64 {
    // reduce is the same as fold
    jit_control_fold(ctx)
}

/// map(array, fn) - transform each element
///
/// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): same JitArray
/// deletion as `jit_control_fold` plus the result allocation goes
/// through the deleted `JitArray::from_vec(...).heap_box()`. Kinded
/// rebuild allocates a `TypedArray<T>` for the inferred element kind.
pub extern "C" fn jit_control_map(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_control_map. Receiver decode + result allocation both \
         block on the kinded TypedArray<T> rebuild per ADR-006 §2.7.6/Q8."
    )
}

/// filter(array, predicate) - keep elements where predicate returns true
pub extern "C" fn jit_control_filter(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_control_filter. Same kinded-TypedArray<T> rebuild as \
         jit_control_map."
    )
}

/// forEach(array, fn, count) - execute fn for each element (side effects)
pub extern "C" fn jit_control_foreach(_ctx: *mut JITContext, _count: usize) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_control_foreach. Same kinded-TypedArray<T> rebuild as \
         jit_control_map."
    )
}

/// find(array, predicate) - find first element matching predicate
pub extern "C" fn jit_control_find(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_control_find. Same kinded-TypedArray<T> rebuild as \
         jit_control_map."
    )
}

// Closure-invoker trampoline staged ahead of its JIT call site.
#[allow(dead_code)]
unsafe fn jit_callable_invoker(
    _ctx: *mut c_void,
    _callable: &u64,
    _args: &[u64],
) -> Result<u64, String> {
    // Phase-2c §2.7.10/Q11 + §2.7.11/Q12: the kinded value-call ABI
    // rebuild applies here too — the native-callback re-entry path
    // pushes the callable + args back onto the JIT stack and dispatches
    // through `jit_call_value`. Both ends are now kinded surfaces; the
    // RawCallableInvoker signature must thread `KindedSlot` through
    // once the kinded JIT-FFI consumer waves land. See
    // docs/cluster-audits/wave-10-jit-playbook.md §5.
    Err(
        "phase-2c §2.7.10/Q11: jit_callable_invoker is a kinded-ABI \
         surface awaiting the value-call kind-companion lowering"
            .to_string(),
    )
}

/// Invoke a linked foreign function from JIT code.
///
/// Args are read from `ctx.stack` (already materialized by lowering):
/// `[... arg0, arg1, ..., argN-1]` with `arg_count` provided out-of-band.
enum ForeignInvokeMode {
    Any,
    NativeOnly,
    DynamicOnly,
}

unsafe fn jit_call_foreign_impl(
    _ctx: *mut JITContext,
    foreign_idx: u32,
    _arg_count: usize,
    _mode: ForeignInvokeMode,
) -> u64 {
    // ffi-rebuild §4.9 J1 (WF-2A stage 4, Q12): UNREACHABLE by construction.
    //
    // Foreign calls execute on the ONE shared interpreter core
    // `VirtualMachine::invoke_foreign_kinded` (control_flow/mod.rs). The JIT
    // preflight gate `vm_only_opcode_reason(OpCode::CallForeign)` (see
    // compiler/accessors.rs) refuses to compile any function or top-level
    // whose bytecode contains `CallForeign`, so Cranelift codegen never emits
    // a call to this trampoline — the enclosing function runs in the bytecode
    // interpreter via the `[jit-fallback]` path. Because tier-2 never runs a
    // foreign-bearing function, `--mode jit` and `--mode vm` cannot diverge
    // on foreign-call semantics; the "deopt" is a compile-time refusal with
    // zero runtime deopt state to get wrong.
    //
    // Reaching this body means the preflight gate regressed (a compiler bug):
    // fail LOUDLY rather than silently return a divergent value. This is NOT
    // the J2 out-of-line lowering (a deferred pure-perf follow-up, design
    // OQ9); J2 would lower `CallForeign` to an out-of-line Cranelift call into
    // this SAME `invoke_foreign_kinded`, preserving the invariant. extern "C"
    // cannot unwind, so this aborts the process — never confusable with a
    // real foreign result.
    unreachable!(
        "ffi-rebuild §4.9 J1 invariant violated: the JIT reached \
         jit_call_foreign_impl for foreign fn #{foreign_idx}, but \
         `vm_only_opcode_reason(OpCode::CallForeign)` (shape-jit \
         compiler/accessors.rs) should have routed the enclosing function to \
         the bytecode interpreter. The preflight gate regressed."
    )
}

pub extern "C" fn jit_call_foreign(
    ctx: *mut JITContext,
    foreign_idx: u32,
    arg_count: usize,
) -> u64 {
    unsafe { jit_call_foreign_impl(ctx, foreign_idx, arg_count, ForeignInvokeMode::Any) }
}

pub extern "C" fn jit_call_foreign_native(
    ctx: *mut JITContext,
    foreign_idx: u32,
    arg_count: usize,
) -> u64 {
    unsafe { jit_call_foreign_impl(ctx, foreign_idx, arg_count, ForeignInvokeMode::NativeOnly) }
}

pub extern "C" fn jit_call_foreign_dynamic(
    ctx: *mut JITContext,
    foreign_idx: u32,
    arg_count: usize,
) -> u64 {
    unsafe { jit_call_foreign_impl(ctx, foreign_idx, arg_count, ForeignInvokeMode::DynamicOnly) }
}

unsafe fn jit_call_foreign_native_args_fixed<const N: usize>(
    _ctx: *mut JITContext,
    foreign_idx: u32,
    _args: [u64; N],
) -> u64 {
    // ffi-rebuild §4.9 J1 (WF-2A stage 4, Q12): UNREACHABLE by construction —
    // same gate as `jit_call_foreign_impl`. `vm_only_opcode_reason(
    // OpCode::CallForeign)` refuses to JIT foreign-bearing functions, so no
    // codegen emits a call to this fixed-arity native trampoline. Reaching it
    // means the preflight gate regressed; abort loudly rather than diverge.
    unreachable!(
        "ffi-rebuild §4.9 J1 invariant violated: the JIT reached \
         jit_call_foreign_native_args_fixed::<{N}> for foreign fn \
         #{foreign_idx}, but `vm_only_opcode_reason(OpCode::CallForeign)` \
         should have routed the enclosing function to the bytecode \
         interpreter. The preflight gate regressed."
    )
}

macro_rules! define_jit_call_foreign_native_fixed {
    ($name:ident, [$($arg:ident),*]) => {
        pub extern "C" fn $name(
            ctx: *mut JITContext,
            foreign_idx: u32,
            $($arg: u64),*
        ) -> u64 {
            unsafe { jit_call_foreign_native_args_fixed(ctx, foreign_idx, [$($arg),*]) }
        }
    };
}

define_jit_call_foreign_native_fixed!(jit_call_foreign_native_0, []);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_1, [arg0]);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_2, [arg0, arg1]);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_3, [arg0, arg1, arg2]);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_4, [arg0, arg1, arg2, arg3]);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_5, [arg0, arg1, arg2, arg3, arg4]);
define_jit_call_foreign_native_fixed!(
    jit_call_foreign_native_6,
    [arg0, arg1, arg2, arg3, arg4, arg5]
);
define_jit_call_foreign_native_fixed!(
    jit_call_foreign_native_7,
    [arg0, arg1, arg2, arg3, arg4, arg5, arg6]
);
define_jit_call_foreign_native_fixed!(
    jit_call_foreign_native_8,
    [arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7]
);

/// Trampoline placeholder for mixed-table VM fallback paths.
///
/// When implemented, this will dispatch to the VM interpreter for functions
/// that weren't JIT-compiled. The return value from the VM is in ValueWord
/// format, so it must be converted to JIT format via `vm_result_to_jit`.
pub unsafe extern "C" fn jit_vm_fallback_trampoline(
    _ctx: *mut std::ffi::c_void,
    _function_id: u32,
    _args_ptr: *const u64,
    _args_len: u32,
) -> u64 {
    // TODO: when implemented, convert result via vm_result_to_jit():
    //   let vm_result = /* dispatch to VM interpreter */;
    //   crate::ffi::object::conversion::vm_result_to_jit(vm_result)
    TAG_NULL
}

/// findIndex(array, predicate) - find index of first element matching predicate
pub extern "C" fn jit_control_find_index(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_control_find_index. Same kinded-TypedArray<T> rebuild as \
         jit_control_map."
    )
}

/// some(array, predicate) - true if any element matches predicate
pub extern "C" fn jit_control_some(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_control_some. Same kinded-TypedArray<T> rebuild as \
         jit_control_map."
    )
}

/// every(array, predicate) - true if all elements match predicate
pub extern "C" fn jit_control_every(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_control_every. Same kinded-TypedArray<T> rebuild as \
         jit_control_map."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // jit_call_value_decodes_arg_count_as_raw_i64 — removed. The
    // function under test is now SURFACE per ADR-006 §2.7.11/Q12 (kinded
    // value-call ABI rebuild); the behavioural decode-arg_count
    // regression test belongs to the kinded ABI rebuild wave (W11 /
    // deeper Phase-2c) where the call signature exposes the kind
    // companion explicitly.

    // ── r5c-2-bz-b-jit-err-surface: VM-trampoline error channel ─────────

    /// `set_jit_runtime_error` then `take_jit_runtime_error` round-trips the
    /// message exactly once; the take clears it so a later, unrelated JIT
    /// execution on the same thread does not inherit a stale error.
    #[test]
    fn jit_runtime_error_channel_round_trips_and_clears() {
        // Clear any residue from a prior test on this thread.
        let _ = take_jit_runtime_error();
        assert_eq!(take_jit_runtime_error(), None);

        set_jit_runtime_error("Set.add(): key must be a string".to_string());
        assert_eq!(
            take_jit_runtime_error().as_deref(),
            Some("Set.add(): key must be a string"),
        );
        // The take cleared it — a second take sees None.
        assert_eq!(take_jit_runtime_error(), None);
    }

    /// The most recent `set_jit_runtime_error` wins — the message that
    /// triggers the deopt is the one surfaced.
    #[test]
    fn jit_runtime_error_channel_keeps_most_recent() {
        let _ = take_jit_runtime_error();
        set_jit_runtime_error("first error".to_string());
        set_jit_runtime_error("second error".to_string());
        assert_eq!(take_jit_runtime_error().as_deref(), Some("second error"));
        let _ = take_jit_runtime_error();
    }

    // ffi-rebuild §4.9 J1 (WF-2A stage 4, Q12): the foreign-call JIT
    // trampolines (`jit_call_foreign` / `jit_call_foreign_native{,_dynamic}` /
    // `jit_call_foreign_native_N` → `jit_call_foreign_impl` /
    // `jit_call_foreign_native_args_fixed`) are now UNREACHABLE by
    // construction — `vm_only_opcode_reason(OpCode::CallForeign)` refuses to
    // JIT any function whose bytecode contains `CallForeign`, so no codegen
    // emits a call to them. The prior `#[ignore]`'d SURFACE test that invoked
    // `jit_call_foreign_native_0` (aborting the test process on the extern-"C"
    // `todo!()`) and its stale `return_null_for_null_context` companion are
    // removed: the J1 invariant is now asserted at the preflight level in
    // `compiler::accessors` (`call_foreign_is_vm_only_j1` /
    // `preflight_gates_call_foreign_j1`), which is the load-bearing gate and
    // is testable without aborting.
}

#[cfg(test)]
mod zero_capture_trampoline_probe {
    //! #239 step-4b precondition — MEASUREMENT, not flip work.
    //!
    //! `docs/program/adr020/rep-kind-abi-design.md` §6.3 item 4 records that
    //! #227 slice 2's revert reason was
    //! "`dispatch_borrowed_closure_via_trampoline_vm` cannot execute a
    //! zero-capture closure record", that a source reading of the consumer
    //! found nothing rejecting zero captures, and that the reading must not be
    //! trusted until someone constructs the record and calls it. These tests
    //! are that construction.
    //!
    //! The record built here is the exact carrier §6.2 makes the JIT adopt:
    //! `alloc_typed_closure(fid, 0, &ClosureLayout::from_capture_types(&[], &[]))`
    //! wrapped in `OwnedClosureBlock`, mirroring the VM's construction at
    //! `shape-vm/src/executor/call_convention.rs:1327-1332` with no
    //! `write_capture_*` calls. Nothing in these tests changes a producer, a
    //! kind stamp, or a consumer.
    //!
    //! Each test states what would have made it fail, because a probe that
    //! cannot observe the rejection it is looking for is not evidence
    //! (§10.4.1).

    use super::*;
    use shape_value::v2::closure_layout::ClosureLayout;
    use shape_value::v2::closure_raw::{OwnedClosureBlock, alloc_typed_closure};
    use shape_value::{HeapValue, NativeKind};
    use std::sync::Arc;

    /// The zero-capture layout §6.3 item 2 says is well-formed on empty
    /// slices. Constructing it is itself part of the measurement — a panic
    /// here would refute that claim before any dispatch happens.
    fn zero_capture_layout() -> Arc<ClosureLayout> {
        let layout = ClosureLayout::from_capture_types(&[], &[]);
        assert_eq!(layout.capture_count(), 0);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.owned_mutable_capture_mask, 0);
        assert_eq!(layout.shared_capture_mask, 0);
        Arc::new(layout)
    }

    /// Build the zero-capture record for `fid`. Caller owns the single
    /// refcount share `alloc_typed_closure` mints; `OwnedClosureBlock::Drop`
    /// retires it.
    fn zero_capture_block(fid: u16) -> OwnedClosureBlock {
        let layout = zero_capture_layout();
        // SAFETY: `alloc_typed_closure` returns a fresh block matching
        // `layout` with refcount 1, and `from_raw` takes that one share. No
        // capture writes are owed — the layout has no capture slots.
        unsafe {
            let ptr = alloc_typed_closure(fid, 0, &layout);
            OwnedClosureBlock::from_raw(ptr, layout)
        }
    }

    /// Compile `source` through the same pipeline `--mode jit` uses to build
    /// its trampoline bytecode (`JITExecutor::execute_program` →
    /// `compile_program_for_inspection`), so function ids agree with
    /// production.
    fn compile(source: &str) -> shape_vm::BytecodeProgram {
        let _ = shape_runtime::initialize_shared_runtime();
        let mut engine = shape_runtime::engine::ShapeEngine::new().expect("engine creation failed");
        let program = shape_ast::parse_program(source).expect("parse failed");
        let mut executor = shape_vm::BytecodeExecutor::new();
        executor
            .compile_program_for_inspection(&mut engine, &program)
            .expect("bytecode compilation failed")
    }

    fn function_id_of(bytecode: &shape_vm::BytecodeProgram, name: &str) -> u16 {
        bytecode
            .functions
            .iter()
            .position(|f| f.name == name)
            .unwrap_or_else(|| panic!("function `{name}` is not in the compiled function table"))
            as u16
    }

    /// Load `bytecode` into a VM and register it as this thread's trampoline,
    /// exactly as `executor.rs:771-779` does (content-addressed payload
    /// cleared so the linker does not renumber functions).
    fn with_trampoline<R>(bytecode: shape_vm::BytecodeProgram, f: impl FnOnce() -> R) -> R {
        let mut bytecode = bytecode;
        bytecode.content_addressed = None;
        let mut vm = shape_vm::VirtualMachine::new(shape_vm::VMConfig::default());
        vm.load_program(bytecode);
        // SAFETY: `vm` outlives `f()` — it is dropped after the unset below.
        unsafe { set_trampoline_vm(&mut vm as *mut shape_vm::VirtualMachine) };
        let result = f();
        unset_trampoline_vm();
        result
    }

    const ADD_ONE: &str = "fn add_one(x: int) -> int { return x + 1 }\n";

    /// **The named experiment.** A zero-capture record for a NAMED function,
    /// handed straight to `dispatch_borrowed_closure_via_trampoline_vm` with a
    /// null `JITContext` so the native arm declines and the interpreter arm
    /// runs.
    ///
    /// Would fail if: the consumer rejected zero captures (returns `TAG_NULL`
    /// and sets an error message), the capture loops mis-handled
    /// `capture_count() == 0`, or `call_closure_with_nb_args_keepalive`
    /// refused a block whose fid names a non-closure function. Any of those
    /// produce `TAG_NULL` / a recorded error rather than `42`.
    #[test]
    fn zero_capture_record_executes_through_the_interpreter_arm() {
        let _ = take_jit_runtime_error();
        let bytecode = compile(ADD_ONE);
        let fid = function_id_of(&bytecode, "add_one");

        // Two dispatches through the SAME record with different arguments, so
        // a constant or stale return cannot satisfy both assertions.
        let (first, second) = with_trampoline(bytecode, || {
            let block = zero_capture_block(fid);
            let first = dispatch_borrowed_closure_via_trampoline_vm(
                &block,
                &[(41u64, NativeKind::Int64)],
                std::ptr::null_mut(),
            );
            let second = dispatch_borrowed_closure_via_trampoline_vm(
                &block,
                &[(1_000_000u64, NativeKind::Int64)],
                std::ptr::null_mut(),
            );
            (first, second)
        });

        assert_eq!(
            take_jit_runtime_error(),
            None,
            "the dispatcher recorded a trampoline error for a zero-capture record",
        );
        assert_ne!(first, TAG_NULL, "zero-capture dispatch returned TAG_NULL");
        assert_eq!(
            first as i64, 42,
            "add_one(41) through a zero-capture closure record",
        );
        assert_eq!(
            second as i64, 1_000_001,
            "the record is re-entrant: a second dispatch tracks its own argument",
        );
    }

    /// The same record reached the way `jit_call_value`'s
    /// `Ptr(HeapKind::Closure)` arm reaches it after the §6.2 flip:
    /// `Arc::into_raw(Arc<HeapValue::ClosureRaw(block)>)` as callee bits,
    /// through the `is_inline_function` bit-shape predicate, then
    /// `Arc::from_raw` + the `HeapValue::ClosureRaw` match.
    ///
    /// Would fail if: a real record's pointer bits satisfied
    /// `is_inline_function` (the NaN-box predicate would steal the callee
    /// into the bare-function arm), the `HeapValue` arm did not project to
    /// `ClosureRaw`, or the dispatch itself rejected the record.
    #[test]
    fn zero_capture_record_survives_the_arc_heapvalue_callee_round_trip() {
        let _ = take_jit_runtime_error();
        let bytecode = compile(ADD_ONE);
        let fid = function_id_of(&bytecode, "add_one");

        let bits = with_trampoline(bytecode, || {
            let arc: Arc<HeapValue> = Arc::new(HeapValue::ClosureRaw(zero_capture_block(fid)));
            let callee_bits = Arc::into_raw(arc) as u64;

            assert_ne!(callee_bits, 0, "record pointer must be non-null");
            assert!(
                !is_inline_function(callee_bits),
                "a real zero-capture record must not satisfy the NaN-box \
                 inline-function predicate, or jit_call_value would route it \
                 into the bare-function arm",
            );

            // SAFETY: reclaims the exact share `into_raw` released above.
            let recovered = unsafe { Arc::<HeapValue>::from_raw(callee_bits as *const HeapValue) };
            let bits = match &*recovered {
                HeapValue::ClosureRaw(block) => dispatch_borrowed_closure_via_trampoline_vm(
                    block,
                    &[(41u64, NativeKind::Int64)],
                    std::ptr::null_mut(),
                ),
                other => panic!("expected HeapValue::ClosureRaw, got {:?}", other.kind()),
            };
            drop(recovered);
            bits
        });

        assert_eq!(take_jit_runtime_error(), None);
        assert_eq!(bits as i64, 42);
    }

    /// Stand-in for a JIT-compiled closure body: pushes `a0 + 1000` onto the
    /// context stack and returns signal 0, which is the contract
    /// `dispatch_borrowed_closure_via_trampoline_vm`'s native arm reads back
    /// (`signal >= 0 && stack_ptr > 0` → pop).
    unsafe extern "C" fn stub_native_callee(ctx: *mut JITContext, a0: u64) -> i32 {
        // SAFETY: the dispatcher passes the non-null context it was given.
        let ctx = unsafe { &mut *ctx };
        let sp = ctx.stack_ptr;
        ctx.stack[sp] = a0 + 1000;
        ctx.stack_kinds[sp] = crate::ffi::stack_kind_code::SENTINEL;
        ctx.stack_ptr = sp + 1;
        0
    }

    /// The native arm with zero captures. `total_args` must reduce to
    /// `arg_pairs.len()`, the capture-read loop must not execute, and the
    /// result must come back off the context stack.
    ///
    /// The trampoline VM is deliberately left UNSET: if the native arm
    /// declined the zero-capture record for any reason, control would reach
    /// the interpreter arm, find no VM, and return `TAG_NULL` with the
    /// "could not reach the interpreter trampoline" message. That is the
    /// observable this test would report on a rejection.
    #[test]
    fn zero_capture_record_enters_the_native_arm() {
        let _ = take_jit_runtime_error();
        unset_trampoline_vm();

        const FID: u16 = 7;
        let mut table: Vec<*const u8> = vec![std::ptr::null(); (FID as usize) + 1];
        table[FID as usize] = stub_native_callee as *const u8;

        let mut ctx = Box::new(JITContext::default());
        ctx.function_table = table.as_ptr() as *const crate::context::JittedStrategyFn;
        ctx.function_table_len = table.len();
        ctx.stack_ptr = 0;

        let block = zero_capture_block(FID);
        let bits = dispatch_borrowed_closure_via_trampoline_vm(
            &block,
            &[(5u64, NativeKind::Int64)],
            &mut *ctx as *mut JITContext,
        );

        assert_eq!(
            take_jit_runtime_error(),
            None,
            "the native arm recorded an error for a zero-capture record",
        );
        assert_eq!(ctx.pending_call_error, 0);
        assert_eq!(
            bits, 1005,
            "the native arm must marshal [captures..., args...] = [5] and read \
             the callee's result back off the context stack",
        );
    }

    /// Negative control for the two tests above: they assert a value that the
    /// failure path cannot produce. This pins that the failure path is
    /// reachable and distinguishable — a zero-capture record whose fid has a
    /// null function-table entry and no trampoline VM returns `TAG_NULL` and
    /// records an error.
    #[test]
    fn zero_capture_rejection_would_be_visible_as_tag_null() {
        let _ = take_jit_runtime_error();
        unset_trampoline_vm();

        let block = zero_capture_block(3);
        let bits = dispatch_borrowed_closure_via_trampoline_vm(
            &block,
            &[(41u64, NativeKind::Int64)],
            std::ptr::null_mut(),
        );

        assert_eq!(bits, TAG_NULL);
        assert!(
            take_jit_runtime_error()
                .is_some_and(|m| m.contains("could not reach the interpreter trampoline")),
            "the rejection path must record its own message",
        );
    }
}
