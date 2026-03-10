//! Generic Builtin FFI Trampoline
//!
//! A single FFI function that can execute ANY builtin function, eliminating
//! the need for individual FFI wrappers for each of the ~170 builtins.
//!
//! Uses the same static trampoline pattern as async_ops: the VM registers a
//! dispatch function before JIT execution, and the JIT calls it via
//! `jit_generic_builtin`.

use super::super::context::JITContext;
use crate::nan_boxing::TAG_NULL;

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
