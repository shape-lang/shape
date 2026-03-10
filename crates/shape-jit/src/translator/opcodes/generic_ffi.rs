//! Generic opcode-to-FFI dispatch
//!
//! Provides a catch-all mechanism for opcodes that don't have dedicated JIT
//! lowering: pop args from the JIT typed stack, flush them to ctx.stack,
//! call the generic builtin FFI trampoline with an opcode-specific ID,
//! and optionally push the result.

use cranelift::prelude::{types as cl_types, *};
use shape_vm::bytecode::OpCode;

use crate::context::STACK_PTR_OFFSET;
use crate::translator::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Compile an opcode via the generic FFI trampoline.
    ///
    /// This is a catch-all for opcodes that don't have dedicated JIT paths.
    /// It flushes operands to ctx.stack and calls `jit_generic_builtin` with
    /// an opcode-derived ID (offset by 0x8000 to distinguish from builtin IDs).
    ///
    /// # Arguments
    /// * `opcode` - The opcode being compiled
    /// * `pop_count` - Number of values to pop from the JIT typed stack
    /// * `pushes_result` - Whether the opcode pushes a result value
    pub(crate) fn compile_opcode_via_generic_ffi(
        &mut self,
        opcode: OpCode,
        pop_count: usize,
        pushes_result: bool,
    ) -> Result<(), String> {
        // Pop values from the JIT typed stack
        let mut vals = Vec::with_capacity(pop_count);
        for _ in 0..pop_count {
            if let Some(val) = self.stack_pop() {
                vals.push(val);
            }
        }

        // Flush to ctx.stack in original order (reverse of pop order)
        vals.reverse();
        for &val in &vals {
            self.emit_opcode_ctx_stack_push(val);
        }

        // Encode the opcode as an ID in the 0x8000+ range to distinguish
        // from BuiltinFunction discriminants (which are < 0x8000).
        let opcode_id = 0x8000u16 | (opcode as u16);
        let opcode_id_val = self.builder.ins().iconst(cl_types::I16, opcode_id as i64);
        let arg_count_val = self.builder.ins().iconst(cl_types::I16, pop_count as i64);

        // Call jit_generic_builtin(ctx, opcode_id, arg_count)
        let call = self.builder.ins().call(
            self.ffi.generic_builtin,
            &[self.ctx_ptr, opcode_id_val, arg_count_val],
        );
        let result = self.builder.inst_results(call)[0];

        if pushes_result {
            self.stack_push(result);
        }
        Ok(())
    }

    /// Push a value onto the JIT context stack for opcode FFI dispatch.
    fn emit_opcode_ctx_stack_push(&mut self, val: Value) {
        let sp_addr = self
            .builder
            .ins()
            .iadd_imm(self.ctx_ptr, STACK_PTR_OFFSET as i64);
        let sp_val = self
            .builder
            .ins()
            .load(cl_types::I64, MemFlags::trusted(), sp_addr, 0);

        let stack_base = self
            .builder
            .ins()
            .iadd_imm(self.ctx_ptr, crate::context::STACK_OFFSET as i64);
        let byte_offset = self.builder.ins().imul_imm(sp_val, 8);
        let slot_addr = self.builder.ins().iadd(stack_base, byte_offset);

        self.builder
            .ins()
            .store(MemFlags::trusted(), val, slot_addr, 0);

        let new_sp = self.builder.ins().iadd_imm(sp_val, 1);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_sp, sp_addr, 0);
    }
}
