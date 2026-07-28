//! Emitting the native-entry callback into compiled function bodies (#117 / R15).
//!
//! R15's native claim needs "a subsequent native dispatch on the covered path".
//! Installation is observable at the `get_finalized_function` site; *execution*
//! of the installed code is not observable from outside it, because a
//! JIT-to-JIT call is a direct Cranelift `call` with no runtime hook. So the
//! evidence has to come from inside the emitted body: this module inserts a
//! call to `jit_witness_native_entry(unit_index)` as the first instruction of a
//! compiled function's entry block.
//!
//! **Observer effect, stated plainly.** The instrumented artifact is not the
//! artifact a normal run executes — it carries one extra call per function
//! entry. The instrumentation does not change which functions are classified
//! JIT-compatible, which are installed, or which fall back: those decisions
//! happen in `compile_program_selective` before any body is built, and they read
//! nothing from this module. What a witness proves is therefore "this program's
//! function N compiles, installs, and its native body runs", not "the byte
//! sequence a non-witness run executes is identical". The witness record names
//! its instrumentation (`native-entry-callback`) so a consumer can see this
//! rather than having to know it.

use cranelift::prelude::*;
use cranelift_module::Module;

use super::setup::JITCompiler;

/// The FFI symbol emitted into instrumented function entries.
pub(super) const WITNESS_ENTRY_SYMBOL: &str = "jit_witness_native_entry";

impl JITCompiler {
    /// Emit the native-entry announcement for compilation unit `unit_index` at
    /// the builder's current position.
    ///
    /// A no-op unless a witness session is collecting on this thread. Callers
    /// invoke this immediately after switching to the entry block, so the
    /// announcement precedes any body instruction that could early-return.
    pub(super) fn emit_native_witness_entry(
        &mut self,
        builder: &mut FunctionBuilder,
        unit_index: usize,
    ) -> Result<(), String> {
        if !shape_vm::native_witness::is_active() {
            return Ok(());
        }
        let func_id = *self.ffi_funcs.get(WITNESS_ENTRY_SYMBOL).ok_or_else(|| {
            format!(
                "#117 native witness: FFI symbol `{WITNESS_ENTRY_SYMBOL}` is not \
                 registered; the witness cannot observe native dispatch"
            )
        })?;
        let func_ref = self.module.declare_func_in_func(func_id, builder.func);
        let index = builder.ins().iconst(types::I64, unit_index as i64);
        builder.ins().call(func_ref, &[index]);
        Ok(())
    }
}
