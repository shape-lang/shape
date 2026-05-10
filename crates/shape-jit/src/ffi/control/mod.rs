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
    _function_id: u32,
    _upvalue_bits: Option<&[u64]>,
    _jit_args: &[u64],
    _jit_ctx: *const JITContext,
) -> u64 {
    todo!(
        "phase-2c §2.7.10/Q11: JIT-side kinded handler ABI rebuild — \
         dispatch_call_via_trampoline_vm. The trampoline-VM call sites \
         (jit_trampoline_call_closure, call_value_immediate_nb) now take \
         &KindedSlot callee + &[KindedSlot] args per ADR-006 §2.7.10/Q11; \
         JIT lowering must thread the per-arg NativeKind companion through \
         the call signature per §2.7.5. See \
         docs/cluster-audits/wave-10-jit-playbook.md §5."
    )
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

/// Call a closure or function value
/// Stack layout: [callee, arg1, ..., argN, arg_count]
/// Returns the result of the call
pub extern "C" fn jit_call_value(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.10/Q11 + §2.7.11/Q12: JIT-side kinded value-call ABI \
         rebuild — jit_call_value. The callee classification (TAG_MODULE_FN \
         tag-bits probe, VM-format heap detection via ValueBits::is_heap, \
         VM-format ValueWord::clone_from_bits + as_closure_handle path, \
         value_word_drop::vw_drop release) and the F7 re-NaN-box i48 \
         retag (ValueBits::make_tagged(TAG_INT, ...)) all decoded from \
         deleted tag_bits / ValueWord / ValueBits machinery. Per ADR-006 \
         §2.7.11/Q12 the value-call ABI is now (callee: KindedSlot, \
         args: &[KindedSlot]) → Result<KindedSlot, VMError>; the per-arg \
         and per-callee NativeKind companion must flow through the JIT \
         lowering site (op_call_value MIR terminator) per §2.7.5. See \
         docs/cluster-audits/wave-10-jit-playbook.md §5."
    )
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
    #[should_panic(expected = "phase-2c")]
    fn native_fixed_arity_helpers_surface_pending_kinded_abi() {
        // SURFACE: jit_call_foreign_native_args_fixed routes to todo!()
        // pending the kinded foreign-call ABI rebuild (§2.7.10/Q11).
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
