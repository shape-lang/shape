//! The native-entry callback (#117 / R15).
//!
//! This is the only place in the codebase that can honestly say "the emitted
//! native body for function N is executing": it is called *from inside* that
//! body, from an instruction the JIT emitted into the function's entry block
//! (`crate::compiler::witness_emit`). Everything else the witness records —
//! preflight classification, installation, fallback reasons — is a compile-time
//! fact, and R15's whole point is that compile-time facts do not add up to a
//! native execution claim.
//!
//! The call is emitted only while a witness session is active
//! (`shape_vm::native_witness::is_active()` at compile time), so a normal
//! `--mode jit` run gets byte-identical codegen to before this landed.

/// Announce entry into JIT-emitted native code for compilation unit
/// `func_index`.
///
/// `func_index` is the unit's position in `BytecodeProgram::functions`, with
/// `functions.len()` denoting the top-level (`__main__`) unit — the same
/// numbering `shape_vm::native_witness::begin_program` registers.
///
/// # Safety
///
/// Called from Cranelift-generated code with the C ABI. It touches no pointer
/// arguments and cannot unwind into the native frame: the recording path is a
/// thread-local counter bump, and an inactive session makes it a no-op.
#[unsafe(no_mangle)]
pub extern "C" fn jit_witness_native_entry(func_index: i64) {
    if func_index < 0 {
        return;
    }
    shape_vm::native_witness::record_native_dispatch(func_index as usize);
}
