//! Generic Builtin FFI Trampoline
//!
//! A single FFI function that can execute ANY builtin function, eliminating
//! the need for individual FFI wrappers for each of the ~170 builtins.
//!
//! Uses the same static trampoline pattern as async_ops: the VM registers a
//! dispatch function before JIT execution, and the JIT calls it via
//! `jit_generic_builtin`.

use super::super::context::JITContext;
use crate::ffi::object::conversion::{jit_bits_to_nanboxed, nanboxed_to_jit_bits};
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
pub extern "C" fn builtin_dispatch_trampoline(
    ctx: *mut JITContext,
    builtin_id: u16,
    arg_count: u16,
) -> u64 {
    use shape_vm::bytecode::{BuiltinFunction, Instruction, OpCode, Operand};
    use shape_vm::{VMConfig, VirtualMachine};
    use std::cell::RefCell;

    if ctx.is_null() {
        return TAG_NULL;
    }

    let ctx_ref = unsafe { &mut *ctx };

    // ── Opcode-based dispatch (0x8000+ range) ──────────────────────────
    if builtin_id & 0x8000 != 0 {
        return dispatch_opcode(ctx_ref, builtin_id, arg_count);
    }

    // ── BuiltinFunction dispatch (0x0000..0x7FFF) ──────────────────────
    let builtin = match BuiltinFunction::from_discriminant(builtin_id) {
        Some(b) => b,
        None => {
            let pop_count = (arg_count as usize).min(ctx_ref.stack_ptr);
            ctx_ref.stack_ptr -= pop_count;
            return TAG_NULL;
        }
    };

    // Read all items from ctx.stack (args + arg_count number).
    // The JIT translator pushes: [arg0, arg1, ..., argN-1, box_number(N)]
    // and passes arg_count = N+1 (total items on stack).
    let total = (arg_count as usize).min(ctx_ref.stack_ptr);
    let start = ctx_ref.stack_ptr - total;
    let mut jit_stack_items: Vec<u64> = Vec::with_capacity(total);
    for i in start..ctx_ref.stack_ptr {
        jit_stack_items.push(ctx_ref.stack[i]);
    }
    ctx_ref.stack_ptr = start;

    // Use a thread-local VM for dispatch to avoid re-creating one per call
    thread_local! {
        static TRAMPOLINE_VM: RefCell<VirtualMachine> =
            RefCell::new(VirtualMachine::new(VMConfig::default()));
    }

    TRAMPOLINE_VM.with(|vm_cell| {
        let mut vm = vm_cell.borrow_mut();

        // Push all items as ValueWord onto the VM stack (args + count)
        for &bits in &jit_stack_items {
            let vw = jit_bits_to_nanboxed(bits);
            if vm.push_vw(vw).is_err() {
                return TAG_NULL;
            }
        }

        // Create synthetic instruction
        let instr = Instruction {
            opcode: OpCode::BuiltinCall,
            operand: Some(Operand::Builtin(builtin)),
        };

        // Dispatch (no ExecutionContext available in JIT)
        if vm.op_builtin_call(&instr, None).is_err() {
            return TAG_NULL;
        }

        // Pop result from VM stack
        match vm.pop_vw() {
            Ok(result) => nanboxed_to_jit_bits(&result),
            Err(_) => TAG_NULL,
        }
    })
}

/// Handle opcode-based generic FFI calls (ConvertToNumber, ConvertToInt, etc.)
///
/// These opcodes pop `arg_count` values from ctx.stack, perform a conversion,
/// and return a single NaN-boxed result. No arg_count suffix is on the stack.
fn dispatch_opcode(ctx_ref: &mut JITContext, builtin_id: u16, arg_count: u16) -> u64 {
    use shape_value::ValueWord;

    let pop_count = (arg_count as usize).min(ctx_ref.stack_ptr);
    if pop_count == 0 {
        return TAG_NULL;
    }

    // Pop the single operand (Convert opcodes take exactly 1 arg)
    ctx_ref.stack_ptr -= pop_count;
    let arg_bits = ctx_ref.stack[ctx_ref.stack_ptr];
    let arg_vw = jit_bits_to_nanboxed(arg_bits);

    let opcode_byte = (builtin_id & 0x7FFF) as u8;

    // Match on raw opcode bytes — these must stay in sync with OpCode repr(u8)
    // in crates/shape-vm/src/bytecode/opcode_defs.rs.
    const CONVERT_TO_INT: u8 = 0x76;
    const CONVERT_TO_NUMBER: u8 = 0x77;
    const CONVERT_TO_STRING: u8 = 0x78;
    const CONVERT_TO_BOOL: u8 = 0x79;
    const TRY_CONVERT_TO_INT: u8 = 0x7C;
    const TRY_CONVERT_TO_NUMBER: u8 = 0x7D;
    const TRY_CONVERT_TO_STRING: u8 = 0x7E;
    const TRY_CONVERT_TO_BOOL: u8 = 0x7F;

    match opcode_byte {
        CONVERT_TO_NUMBER | TRY_CONVERT_TO_NUMBER => {
            if let Some(n) = arg_vw.as_number_coerce() {
                return nanboxed_to_jit_bits(&ValueWord::from_f64(n));
            }
            if let Some(s) = arg_vw.as_str() {
                if let Ok(n) = s.parse::<f64>() {
                    return nanboxed_to_jit_bits(&ValueWord::from_f64(n));
                }
            }
            if let Some(b) = arg_vw.as_bool() {
                return nanboxed_to_jit_bits(&ValueWord::from_f64(if b { 1.0 } else { 0.0 }));
            }
            if opcode_byte == TRY_CONVERT_TO_NUMBER {
                // TryConvert returns Result::Err on failure
                return nanboxed_to_jit_bits(&ValueWord::from_err(
                    ValueWord::from_string(std::sync::Arc::new("cannot convert to number".into())),
                ));
            }
            TAG_NULL
        }
        CONVERT_TO_INT | TRY_CONVERT_TO_INT => {
            if let Some(i) = arg_vw.as_i64() {
                return nanboxed_to_jit_bits(&ValueWord::from_i64(i));
            }
            if let Some(n) = arg_vw.as_number_coerce() {
                return nanboxed_to_jit_bits(&ValueWord::from_i64(n as i64));
            }
            if let Some(s) = arg_vw.as_str() {
                if let Ok(i) = s.parse::<i64>() {
                    return nanboxed_to_jit_bits(&ValueWord::from_i64(i));
                }
            }
            TAG_NULL
        }
        CONVERT_TO_STRING | TRY_CONVERT_TO_STRING => {
            let s = format!("{}", arg_vw);
            nanboxed_to_jit_bits(&ValueWord::from_string(std::sync::Arc::new(s)))
        }
        CONVERT_TO_BOOL | TRY_CONVERT_TO_BOOL => {
            let b = arg_vw.is_truthy();
            nanboxed_to_jit_bits(&ValueWord::from_bool(b))
        }
        _ => {
            // Unhandled opcode — dispatch through the trampoline VM.
            // Re-push the args we popped back onto ctx.stack, then let the
            // builtin_dispatch_trampoline's thread-local VM handle it.
            // For now, return the input unchanged as a pass-through.
            arg_bits
        }
    }
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
