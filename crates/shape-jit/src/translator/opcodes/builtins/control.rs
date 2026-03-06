//! Control flow and exception handling builtin functions for JIT compilation

use cranelift::prelude::*;

use crate::nan_boxing::TAG_NULL;
use crate::translator::types::BytecodeToIR;
use shape_vm::bytecode::BuiltinFunction;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Compile control flow builtin functions
    #[inline(always)]
    pub(super) fn compile_control_builtin(
        &mut self,
        builtin: &BuiltinFunction,
        idx: usize,
    ) -> bool {
        match builtin {
            BuiltinFunction::Format => {
                let arg_count = self.get_arg_count_from_prev_instruction(idx);
                let needed = arg_count + 1; // args + arg_count on stack
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let count_val = self.builder.ins().iconst(types::I64, arg_count as i64);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.format, &[self.ctx_ptr, count_val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }

            _ => false,
        }
    }
}
