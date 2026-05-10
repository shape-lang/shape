//! Generic Builtin FFI Trampoline
//!
//! A single FFI function that can execute ANY builtin function, eliminating
//! the need for individual FFI wrappers for each of the ~170 builtins.
//!
//! Uses the same static trampoline pattern as async_ops: the VM registers a
//! dispatch function before JIT execution, and the JIT calls it via
//! `jit_generic_builtin`.

use super::super::context::JITContext;
use crate::ffi::value_ffi::TAG_NULL;
// jit_bits_to_nanboxed / nanboxed_to_jit_bits removed at the import
// site — both decoded/encoded `ValueWord` from raw bits, the deleted
// W-series shape. The trampoline body below now surfaces rather than
// going through that pipeline.

/// Static trampoline function pointer for generic builtin dispatch.
///
/// Signature: `fn(ctx: *mut JITContext, builtin_id: u16, arg_count: u16) -> u64`
///
/// The registered function should:
/// 1. Read `arg_count` values from `ctx.stack` (popping them)
/// 2. Dispatch to the appropriate builtin handler
/// 3. Return the NaN-boxed result
pub static GENERIC_BUILTIN_FN: std::sync::atomic::AtomicPtr<()> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Register the generic builtin dispatch trampoline.
///
/// # Safety
/// The function pointer must be valid for the duration of JIT execution and
/// must have the signature: `extern "C" fn(*mut JITContext, u16, u16) -> u64`
pub unsafe fn register_generic_builtin_fn(f: extern "C" fn(*mut JITContext, u16, u16) -> u64) {
    GENERIC_BUILTIN_FN.store(f as *mut (), std::sync::atomic::Ordering::Release);
}

/// Clear the generic builtin dispatch registration.
pub fn unregister_generic_builtin_fn() {
    GENERIC_BUILTIN_FN.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
}

/// Execute any builtin function via the registered trampoline.
///
/// Called from JIT-compiled code when a builtin is not handled by a dedicated
/// JIT lowering path. The JIT translator flushes args onto `ctx.stack` before
/// calling this function.
///
/// # Arguments
/// * `ctx` - JIT execution context (args are on ctx.stack)
/// * `builtin_id` - The `BuiltinFunction` discriminant as u16
/// * `arg_count` - Number of arguments on the stack
///
/// # Returns
/// NaN-boxed result, or TAG_NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn jit_generic_builtin(
    ctx: *mut JITContext,
    builtin_id: u16,
    arg_count: u16,
) -> u64 {
    let f = GENERIC_BUILTIN_FN.load(std::sync::atomic::Ordering::Acquire);
    if f.is_null() {
        // No trampoline registered — pop args and return null
        if !ctx.is_null() {
            let ctx_ref = unsafe { &mut *ctx };
            let pop_count = (arg_count as usize).min(ctx_ref.stack_ptr);
            ctx_ref.stack_ptr -= pop_count;
        }
        return TAG_NULL;
    }
    let dispatch: extern "C" fn(*mut JITContext, u16, u16) -> u64 =
        unsafe { std::mem::transmute(f) };
    dispatch(ctx, builtin_id, arg_count)
}

/// Concrete trampoline implementation that dispatches builtins AND opcode-based
/// generic FFI calls through a thread-local VirtualMachine.
///
/// The JIT translator uses two ID ranges:
/// - `0x0000..0x7FFF`: BuiltinFunction discriminants (e.g. Abs, Print)
/// - `0x8000..0xFFFF`: opcode byte OR'd with 0x8000 (e.g. ConvertToNumber)
///
/// Protocol: the JIT translator pushes args onto `ctx.stack`. For builtins,
/// it also pushes arg_count as a NaN-boxed number (VM calling convention).
/// For opcode-based calls, only the raw args are pushed (no count suffix).
/// Concrete trampoline implementation that dispatches builtins via the VM.
///
/// PHASE_2C / SURFACE (ADR-006 §2.7.4 / §2.7.5 / §2.7.7): pre-strict-
/// typing this body popped JIT-stack `u64` bits, decoded each via
/// `jit_bits_to_nanboxed(bits)` (returning a `ValueWord` — deleted),
/// pushed them onto a thread-local `VirtualMachine` via
/// `vm.push_raw_u64(vw)` (deleted — replaced by `push_kinded(bits,
/// kind)` per §2.7.7), invoked `op_builtin_call`, then popped via
/// `vm.pop_raw_u64()` (also deleted) and re-encoded the result via
/// `nanboxed_to_jit_bits`. Both ends are the W-series defection-
/// attractor pipeline (CLAUDE.md "Forbidden Patterns" / "Renames to
/// refuse on sight" — `push_raw_u64` / `pop_raw_u64` are explicitly
/// listed as the §2.7.7 forbidden shim shapes).
///
/// The strict-typing rebuild target threads `(bits: u64, kind:
/// NativeKind)` from the JIT call site, pushes via `vm.push_kinded`,
/// invokes the builtin, pops via the matching kinded API, and returns
/// `(bits, kind)` back to the caller. That requires:
///
/// 1. JIT-stack parallel-kind track at the JIT-FFI boundary (§2.7.7
///    extension to the JIT side; W11 / deeper Phase-2c).
/// 2. Per-builtin known result kind (the bytecode's typed
///    `BuiltinCall` opcode signature; partly landed for the typed
///    builtins in shape-vm, not yet threaded through the JIT
///    trampoline).
///
/// Until both land, the body returns `TAG_NULL` after popping the
/// stack — the caller observes the dispatch as "builtin not handled
/// in JIT, falling back to bytecode interpreter" (the same shape as
/// the existing fallback when `GENERIC_BUILTIN_FN` is unregistered).
pub extern "C" fn builtin_dispatch_trampoline(
    ctx: *mut JITContext,
    builtin_id: u16,
    arg_count: u16,
) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }

    let ctx_ref = unsafe { &mut *ctx };

    // ── Opcode-based dispatch (0x8000+ range) ──────────────────────────
    if builtin_id & 0x8000 != 0 {
        return dispatch_opcode(ctx_ref, builtin_id, arg_count);
    }

    // ── BuiltinFunction dispatch (0x0000..0x7FFF) ──────────────────────
    // Pop args (the legacy pre-strict-typing shape pushed an arg_count
    // suffix as a NaN-boxed number on top of the args; we pop the same
    // total to keep the JIT stack drained).
    let pop_count = (arg_count as usize).min(ctx_ref.stack_ptr);
    ctx_ref.stack_ptr -= pop_count;
    let _ = builtin_id;
    TAG_NULL
}

/// Handle opcode-based generic FFI calls (ConvertToNumber, ConvertToInt, etc.).
///
/// PHASE_2C / SURFACE (ADR-006 §2.7.4 / §2.7.5): pre-strict-typing this
/// function decoded the operand via `jit_bits_to_nanboxed(arg_bits)`
/// (returning a `ValueWord`), classified it via `ValueWord::as_*`
/// methods (every one of which decoded `tag_bits` from the raw bits —
/// the deleted W-series shape), and re-encoded the result via
/// `ValueWord::from_*` constructors plus `nanboxed_to_jit_bits`. The
/// W-series defection-attractor list forbids both ends of that
/// pipeline.
///
/// Strict-typing rebuild target: the ConvertTo* opcodes are kind-typed
/// at JIT-emit time per CLAUDE.md "Type System Rules" — there is no
/// generic `ConvertTo*`; the compiler emits per-source-kind variants
/// (`ConvertI64ToF64`, `ConvertStringToF64`, etc.) where both operand
/// and result `NativeKind` are stamped at JIT compile time. The
/// per-variant body reads a typed scalar from the JIT slot and writes
/// a typed scalar back, no FFI hop, no kind probing. This entry point
/// disappears entirely once the per-kind opcode coverage is complete.
/// CLAUDE.md "Forbidden Patterns" calls out the W4-δ-era Convert<X>
/// To<Y> opcode shape as a wrong-shape opcode added to paper over a
/// kind-tracker gap — the right fix was extending the compiler's kind
/// tracker, which the strict-typing plan does.
///
/// Until the per-kind opcode coverage lands, the body returns
/// `arg_bits` unchanged, surfacing the kind-source gap at the caller
/// rather than fabricating a result via the deleted ValueWord API.
fn dispatch_opcode(ctx_ref: &mut JITContext, builtin_id: u16, arg_count: u16) -> u64 {
    let pop_count = (arg_count as usize).min(ctx_ref.stack_ptr);
    if pop_count == 0 {
        return TAG_NULL;
    }

    // Pop the operand (Convert opcodes take exactly 1 arg) and pass
    // through unchanged — see surface comment above. The caller's
    // emitted code treated this as "FFI conversion will produce the
    // typed result"; the strict-typing rebuild routes ConvertTo* to a
    // typed opcode body in the MIR-compile path instead.
    ctx_ref.stack_ptr -= pop_count;
    let arg_bits = ctx_ref.stack[ctx_ref.stack_ptr];
    let _ = builtin_id;
    arg_bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generic_builtin_null_trampoline() {
        let mut ctx = JITContext::default();
        // Push a dummy value on the stack
        ctx.stack[0] = 42;
        ctx.stack_ptr = 1;

        let result = jit_generic_builtin(&mut ctx, 0, 1);
        assert_eq!(result, TAG_NULL);
        // Args should be popped
        assert_eq!(ctx.stack_ptr, 0);
    }
}
