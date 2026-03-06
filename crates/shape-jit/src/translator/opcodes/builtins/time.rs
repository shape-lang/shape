//! Time operations builtin functions for JIT compilation
//!
//! Time builtins (TimeCurrentTime, TimeSymbol, TimeLastRow, TimeRange) were removed
//! from BuiltinFunction. Time operations are now handled through the stdlib.

use crate::translator::types::BytecodeToIR;
use shape_vm::bytecode::BuiltinFunction;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Compile time builtin functions (currently none — all removed)
    #[inline(always)]
    pub(super) fn compile_time_builtin(&mut self, _builtin: &BuiltinFunction) -> bool {
        false
    }
}
