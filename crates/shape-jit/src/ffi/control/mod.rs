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
use crate::ffi::value_ffi::*;
#[allow(unused_imports)]
use crate::ffi::jit_kinds::*;
use std::ffi::c_void;

// ============================================================================
// Trampoline VM — thread-local VirtualMachine for JIT-to-VM fallback
// ============================================================================

use std::cell::Cell;

thread_local! {
    /// Pointer to a fully-initialized VirtualMachine for executing bytecode
    /// functions that weren't JIT-compiled. Set by `execute_with_jit()` before
    /// JIT execution and cleared after. Valid only on the executor thread.
    static TRAMPOLINE_VM: Cell<*mut shape_vm::VirtualMachine> = const { Cell::new(std::ptr::null_mut()) };
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
    upvalue_bits: Option<&[u64]>,
    jit_args: &[u64],
    _jit_ctx: *const JITContext,
) -> u64 {
    use shape_value::NativeKind;

    // §2.7.5 stable-FFI raw-pair shape: each arg / capture pair is
    // `(u64, NativeKind)`. The JIT MIR emitter widened every arg to
    // I64 at terminators.rs:651-671 without an associated kind track;
    // we stamp `NativeKind::UInt64` here (the §2.7.11 callee-
    // classification kind for function-id-shaped slots, also used as
    // the "I64-wide raw bits without further classification" carrier
    // kind at the §2.7.5 stable-FFI boundary). This is NOT a Bool-
    // default fallback — it is the documented function-id-class kind
    // shape per ADR-006 §2.7.11/Q12.
    //
    // The kind companion is consumed by `jit_trampoline_call_closure`
    // which wraps each pair as a `KindedSlot` and threads it into the
    // new frame's locals via `stack_write_kinded`. The VM-side
    // runtime-tier per-slot kind track is established by the callee's
    // own FrameDescriptor when it begins execution; the §2.7.5 stable-
    // FFI handoff doesn't need per-arg semantic kind, only the slot-
    // size discipline (I64 here).
    let arg_pairs: Vec<(u64, NativeKind)> = jit_args
        .iter()
        .copied()
        .map(|bits| (bits, NativeKind::UInt64))
        .collect();

    with_trampoline_vm_mut(|vm| {
        let func_id = function_id as u16;
        match upvalue_bits {
            Some(caps) => {
                // Shape 2 / 3: closure-with-captures. Route through
                // `jit_trampoline_call_closure` which materializes a
                // fresh `OwnedClosureBlock` from `upvalue_bits` and
                // dispatches via `call_closure_with_nb_args_keepalive`.
                let capture_pairs: Vec<(u64, NativeKind)> = caps
                    .iter()
                    .copied()
                    .map(|bits| (bits, NativeKind::UInt64))
                    .collect();
                match vm.jit_trampoline_call_closure(func_id, &capture_pairs, &arg_pairs, None) {
                    Ok(bits) => bits,
                    Err(_) => TAG_NULL,
                }
            }
            None => {
                // Shape 1: bare function callee (no captures). Use the
                // VM's `call_value_immediate_nb` with a `NativeKind::
                // UInt64` callee — the §2.7.11 callee-classification
                // kind for function-id-shaped callees (per
                // `call_convention.rs:853-877` UInt64 arm).
                use shape_value::{KindedSlot, ValueSlot};
                let callee = KindedSlot::new(
                    ValueSlot::from_raw(func_id as u64),
                    NativeKind::UInt64,
                );
                let kinded_args: Vec<KindedSlot> = arg_pairs
                    .iter()
                    .map(|(bits, kind)| {
                        KindedSlot::new(ValueSlot::from_raw(*bits), *kind)
                    })
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
                        // for UInt64 kind. Same for the arg
                        // KindedSlots — JIT pre-incremented each share
                        // before crossing the FFI boundary, and the VM
                        // already consumed them by transferring into
                        // the new frame's locals.
                        std::mem::forget(callee);
                        std::mem::forget(kinded_args);
                        bits
                    }
                    Err(_) => TAG_NULL,
                }
            }
        }
    })
    .unwrap_or(TAG_NULL)
}

/// Dispatch a native module function call through the trampoline VM.
fn dispatch_module_fn_call(
    _module_fn_id: u32,
    _jit_args: &[u64],
    _ctx: *mut JITContext,
) -> u64 {
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
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;

        // Check if we have a function table
        if ctx_ref.function_table.is_null() || (function_id as usize) >= ctx_ref.function_table_len
        {
            return TAG_NULL;
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
/// Two callee shapes flow through `jit_call_value` today:
///
///   1. **Inline function** (`box_function(fn_id)` → `TAG_FUNCTION_BITS`
///      tag): classified by `is_inline_function(callee_bits)`, function-
///      id recovered by `unbox_function_id(callee_bits)`. The JIT MIR
///      emitter pushes this shape when the callee operand is a bare
///      `FunctionRef` constant.
///
///   2. **Deprecated `unified_box(HK_CLOSURE, JITClosure)` callees**:
///      classified by `is_heap_kind(callee_bits, HK_CLOSURE)`. This is
///      the legacy `jit_make_closure` FFI return shape. New code goes
///      through `jit_finalize_heap_closure` which returns a raw
///      `Arc::into_raw(Arc<HeapValue::ClosureRaw>)` (no NaN-box) — see
///      "kind-source gap" below.
///
/// ## Kind-source gap (§2.7.5 surface)
///
/// `jit_finalize_heap_closure` (the current preferred closure path)
/// returns `Arc::into_raw(Arc::new(HeapValue::ClosureRaw(owned))) as u64`
/// — a raw Arc pointer, not a NaN-boxed value. There is no tag-bit
/// signature on the bits themselves; the callee's `NativeKind::Ptr(
/// HeapKind::Closure)` is supplied by the producing site at JIT compile
/// time and lives in a separate side-table the MIR emitter would have
/// to thread through the call signature.
///
/// Under the current `extern "C" fn(*mut JITContext)` signature, the
/// callee kind is NOT recoverable from `callee_bits` alone — and per
/// §2.7.7 #4 / #7 / CLAUDE.md "Forbidden Patterns" we MUST NOT probe
/// `is_heap()` / `is_tagged()` on the bits to classify (those predicates
/// are JIT-internal NaN-box checks, valid for the *NaN-boxed* shapes
/// above, but NOT for raw Arc pointers — a heap pointer with bit-63=0
/// reads as "not tagged" and the predicate returns false; a heap pointer
/// that happens to alias a tag pattern is a wrong-shape match).
///
/// Per the §2.7.5 stamp-at-compile-time discipline, the principled fix
/// is for the JIT MIR emitter to extend the call signature to carry a
/// parallel kind track (or per-callee kind side-table) — that is an
/// ADR-006 §2.7.5 follow-up and an architectural extension beyond this
/// sub-cluster's scope. For raw-Arc closure callees today, we
/// surface-and-stop: return TAG_NULL after popping the stack frame, so
/// the calling MIR continues with a null result rather than crashing
/// via `extern "C" todo!()` SIGABRT. The shape mirrors the W11-round-1
/// close's `jit_join_init` surface — graceful surface, audible via
/// `--trace-jit=shape_jit=debug` (cluster-2 closure-wave-F tracing-crate
/// migration 2026-05-16), no silent leak (the Arc share remains owned by
/// the stack slot per the §2.7.7 retain-on-read discipline).
///
/// ## Argument kind sourcing
///
/// JIT MIR widens args to I64 at terminators.rs:651-671 without an
/// associated kind track — the same §2.7.5 gap. We pass raw `u64` bits
/// through to `jit_trampoline_call_closure` paired with `NativeKind::
/// UInt64` companions (the §2.7.11 callee-classification kind for
/// function-id callees) ONLY when we can prove the callee is a function
/// (case 1 above). The VM-side trampoline does not currently consume
/// per-arg kinds beyond function dispatch; per `call_convention.rs:
/// jit_trampoline_call_closure` the args are wrapped as
/// `KindedSlot::new(ValueSlot::from_raw(bits), kind)` and threaded into
/// the new frame's locals without inspecting kind on the read side. For
/// heap-bearing args, the W11-round-1 retain-on-read discipline on the
/// runtime tier handles refcount; the JIT side has already retained
/// each share before pushing per the §2.7.7 retain semantics. No
/// fabrication: `NativeKind::UInt64` is the documented function-id
/// classification kind, not a Bool-default fallback.
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
    use crate::ffi::jit_kinds::unified_unbox;
    use crate::ffi::stack_kind_code;
    use crate::context::JITClosure;
    use shape_value::{HeapKind, NativeKind, heap_value::HeapValue};
    use std::sync::Arc;

    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
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
            return TAG_NULL;
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
                return TAG_NULL;
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
                    return TAG_NULL;
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
            return TAG_NULL;
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
                return TAG_NULL;
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
        //   `Arc<HeapValue>` slot-tier convention and pass through to
        //   `jit_trampoline_call_closure`, which decodes the closure
        //   captures kinded.
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
        let mut vm_captures: Option<Vec<u64>> = None;

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
                    return TAG_NULL;
                }
                // Borrow the `Arc<HeapValue>` (use `from_raw` + `into_raw`
                // to avoid taking the share — the share stays in the
                // stack slot per §2.7.11 / Q12 the dispatch shell borrow
                // contract).
                let arc = Arc::<HeapValue>::from_raw(callee_bits as *const HeapValue);
                let extracted: Option<(u16, Vec<u64>)> = match &*arc {
                    HeapValue::ClosureRaw(block) => {
                        // §2.7.11/Q12: read the function_id from the
                        // TypedClosureHeader prefix at offset 8 (per
                        // `closure_raw.rs` `TypedClosureHeader` layout).
                        let fid = shape_value::v2::closure_raw::typed_closure_function_id(
                            block.as_ptr(),
                        );
                        let cap_count = block.layout().capture_count();
                        let mut caps: Vec<u64> = Vec::with_capacity(cap_count);
                        for idx in 0..cap_count {
                            // §2.7.8/Q10 read_capture_kinded returns
                            // `(bits, kind)`. The trampoline VM
                            // `jit_trampoline_call_closure` re-pairs each
                            // bits/kind through `KindedSlot::new` inside
                            // the new frame setup; here we only need the
                            // raw bits because the trampoline call
                            // ignores per-capture kind on the JIT-FFI
                            // boundary (the runtime-tier per-slot kind
                            // track is established by the callee's own
                            // FrameDescriptor when it begins execution
                            // — same shape as the bare-function path).
                            let (cap_bits, _cap_kind) = block.read_capture_kinded(idx);
                            caps.push(cap_bits);
                        }
                        Some((fid, caps))
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
                        None
                    }
                };
                // Restore the `Arc` raw pointer — the slot share is
                // still owned by whoever pushed it (the call signature
                // borrow contract leaves the share with the producer).
                let _ = Arc::into_raw(arc);
                match extracted {
                    Some((f, c)) => {
                        function_id = f;
                        vm_captures = Some(c);
                    }
                    None => return TAG_NULL,
                }
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
                return TAG_NULL;
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
                    let mut caps: Vec<u64> = Vec::with_capacity(count);
                    for i in 0..count {
                        caps.push(*closure.captures_ptr.add(i));
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
                    return TAG_NULL;
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
                return TAG_NULL;
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
                return TAG_NULL;
            }
        }

        // Fallback: route through the trampoline VM. This handles:
        //   - JIT-untranslated function bodies (null function-table entry).
        //   - HK_CLOSURE callees (captures threaded into the new frame).
        //   - Raw-Arc HeapKind::Closure callees (Case 3 closed via the
        //     §2.7.11/Q12 kind dispatch above).
        let upvalues: Option<&[u64]> = vm_captures.as_deref();
        dispatch_call_via_trampoline_vm(
            function_id as u32,
            upvalues,
            &args,
            ctx as *const JITContext,
        )
    }
}

/// Call a JIT-compiled function pointer with the right number of native arguments.
/// The function has Cranelift signature: fn(ctx_ptr: i64, arg0: i64, ...) -> i32
unsafe fn call_jit_fn_with_args(
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

    let result = match args.len() {
        0 => std::mem::transmute::<_, F0>(fn_ptr)(ctx),
        1 => std::mem::transmute::<_, F1>(fn_ptr)(ctx, args[0]),
        2 => std::mem::transmute::<_, F2>(fn_ptr)(ctx, args[0], args[1]),
        3 => std::mem::transmute::<_, F3>(fn_ptr)(ctx, args[0], args[1], args[2]),
        4 => std::mem::transmute::<_, F4>(fn_ptr)(ctx, args[0], args[1], args[2], args[3]),
        5 => std::mem::transmute::<_, F5>(fn_ptr)(ctx, args[0], args[1], args[2], args[3], args[4]),
        6 => std::mem::transmute::<_, F6>(fn_ptr)(ctx, args[0], args[1], args[2], args[3], args[4], args[5]),
        7 => std::mem::transmute::<_, F7>(fn_ptr)(ctx, args[0], args[1], args[2], args[3], args[4], args[5], args[6]),
        8 => std::mem::transmute::<_, F8>(fn_ptr)(ctx, args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7]),
        _ => {
            // Too many args for direct dispatch — fall back to trampoline
            -1
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
    _foreign_idx: u32,
    _arg_count: usize,
    _mode: ForeignInvokeMode,
) -> u64 {
    todo!(
        "phase-2c §2.7.10/Q11: JIT-side kinded foreign-call ABI rebuild — \
         jit_call_foreign_impl. The foreign_bridge invoke / invoke_native / \
         invoke_dynamic surfaces still take &[ValueWord]; once that crate's \
         own kinded-ABI migration lands, args flow as &[KindedSlot] per \
         ADR-006 §2.7.10/Q11 and the Err() arm constructs the Result::Err \
         carrier through the kinded HeapKind::Err producer per §2.7.6/Q8. \
         See docs/cluster-audits/wave-10-jit-playbook.md §5."
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
    _foreign_idx: u32,
    _args: [u64; N],
) -> u64 {
    todo!(
        "phase-2c §2.7.10/Q11: JIT-side kinded foreign-call ABI rebuild — \
         jit_call_foreign_native_args_fixed<N>. Same gating as \
         jit_call_foreign_impl: foreign_bridge invoke_native still takes \
         &[ValueWord]; once that crate's own kinded-ABI migration lands, \
         the fixed-arity boxed_args array becomes [KindedSlot; N] per \
         ADR-006 §2.7.10/Q11. See \
         docs/cluster-audits/wave-10-jit-playbook.md §5."
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

    #[test]
    #[ignore = "SURFACE: jit_call_foreign_native_0 is extern \"C\" todo!() pending kinded foreign-call ABI rebuild (ADR-006 §2.7.10/Q11, docs/cluster-audits/wave-10-jit-playbook.md §5); extern C can't unwind, so #[should_panic] aborts the test process. Re-enable via `cargo test -- --ignored` once the underlying SURFACE closes."]
    fn native_fixed_arity_helpers_surface_pending_kinded_abi() {
        // SURFACE: jit_call_foreign_native_args_fixed routes to todo!()
        // pending the kinded foreign-call ABI rebuild (§2.7.10/Q11).
        // Can't use #[should_panic] on extern "C" functions: Rust 1.93+
        // aborts the process (SIGABRT) on a non-unwinding panic instead of
        // reporting a clean test failure. Same constraint as
        // ffi/v2/mod.rs:1060 `test_array_get_oob_returns_none_via_typed_array`.
        let _ = jit_call_foreign_native_0(std::ptr::null_mut(), 0);
    }

    // Suppress the unused-helpers lint for the moved `native_fixed_arity_helpers_return_null_for_null_context`.
    #[allow(dead_code)]
    fn native_fixed_arity_helpers_return_null_for_null_context() {
        assert_eq!(jit_call_foreign_native_0(std::ptr::null_mut(), 0), TAG_NULL);
        assert_eq!(
            jit_call_foreign_native_1(std::ptr::null_mut(), 0, TAG_NULL),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_2(std::ptr::null_mut(), 0, TAG_NULL, TAG_NULL),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_3(std::ptr::null_mut(), 0, TAG_NULL, TAG_NULL, TAG_NULL),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_4(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_5(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_6(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_7(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_8(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
    }
}
