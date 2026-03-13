//! Data operations: DataFrame access, column reads, kernel mode data access

use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;

use crate::nan_boxing::*;
use shape_vm::bytecode::{Instruction, OpCode, Operand};
use shape_vm::type_tracking::StorageHint;

use crate::translator::types::{BytecodeToIR, CompilationMode};

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    // ========================================================================
    // Typed Column Access (LoadCol* opcodes → FFI calls)
    // ========================================================================

    /// Compile LoadCol* opcodes — pops a RowView/TAG_INT row ref from the stack,
    /// calls the appropriate FFI function with (ctx, col_id, row_ref).
    ///
    /// Stack: [row_ref] -> [typed_value]
    /// Operand: ColumnAccess { col_id }
    pub(crate) fn compile_load_col(
        &mut self,
        instr: &Instruction,
        ffi_func: FuncRef,
    ) -> Result<(), String> {
        let col_id = match &instr.operand {
            Some(Operand::ColumnAccess { col_id }) => *col_id,
            _ => return Err("LoadCol* requires ColumnAccess operand".to_string()),
        };

        // Pop row reference from stack
        let row_ref = self
            .stack_pop()
            .ok_or("LoadCol: missing row reference on stack")?;

        let result = match instr.opcode {
            OpCode::LoadColF64 | OpCode::LoadColI64 | OpCode::LoadColBool => {
                self.inline_load_typed_column(col_id, row_ref, instr.opcode)
            }
            _ => {
                let col_id_val = self.builder.ins().iconst(types::I32, col_id as i64);
                let inst = self
                    .builder
                    .ins()
                    .call(ffi_func, &[self.ctx_ptr, col_id_val, row_ref]);
                self.builder.inst_results(inst)[0]
            }
        };
        match instr.opcode {
            OpCode::LoadColF64 => {
                self.stack_push_typed(result, StorageHint::Float64);
                let result_f64 = self.i64_to_f64(result);
                self.typed_stack
                    .replace_top(crate::translator::storage::TypedValue::f64(result_f64));
            }
            OpCode::LoadColI64 => {
                self.stack_push_typed(result, StorageHint::Int64);
            }
            OpCode::LoadColBool => {
                self.stack_push_typed(result, StorageHint::Bool);
            }
            _ => self.stack_push(result),
        }
        Ok(())
    }

    /// Inline typed column load from `ctx.column_ptrs[col_id][row_idx]`.
    ///
    /// Mirrors the semantics of `jit_load_col_{f64,i64,bool}` without FFI:
    /// - if `row_ref` is TAG_INT (data row), use its payload as row index
    /// - otherwise use `ctx.current_row`
    /// - out-of-bounds/null pointers return `TAG_NULL`
    fn inline_load_typed_column(&mut self, col_id: u32, row_ref: Value, op: OpCode) -> Value {
        use crate::context::{
            COLUMN_COUNT_OFFSET, COLUMN_PTRS_OFFSET, CURRENT_ROW_OFFSET, ROW_COUNT_OFFSET,
        };

        let tag_mask = self.builder.ins().iconst(types::I64, TAG_MASK as i64);
        let data_row_tag = self.builder.ins().iconst(types::I64, TAG_DATA_ROW as i64);
        let row_tag = self.builder.ins().band(row_ref, tag_mask);
        let is_data_row = self.builder.ins().icmp(IntCC::Equal, row_tag, data_row_tag);

        let payload_mask = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let row_from_ref = self.builder.ins().band(row_ref, payload_mask);
        let current_row = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            self.ctx_ptr,
            CURRENT_ROW_OFFSET,
        );
        let row_idx = self
            .builder
            .ins()
            .select(is_data_row, row_from_ref, current_row);

        let col_id_i64 = self.builder.ins().iconst(types::I64, col_id as i64);
        let row_count = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            self.ctx_ptr,
            ROW_COUNT_OFFSET,
        );
        let col_count = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            self.ctx_ptr,
            COLUMN_COUNT_OFFSET,
        );
        let cols_base = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            self.ctx_ptr,
            COLUMN_PTRS_OFFSET,
        );
        let zero_i64 = self.builder.ins().iconst(types::I64, 0);
        let row_in_bounds = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, row_idx, row_count);
        let col_in_bounds = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, col_id_i64, col_count);
        let base_non_null = self
            .builder
            .ins()
            .icmp(IntCC::NotEqual, cols_base, zero_i64);
        let valid_bounds = self.builder.ins().band(row_in_bounds, col_in_bounds);
        let can_load = self.builder.ins().band(valid_bounds, base_non_null);

        let fast_block = self.builder.create_block();
        let null_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);
        self.builder
            .ins()
            .brif(can_load, fast_block, &[], null_block, &[]);

        self.builder.switch_to_block(fast_block);
        self.builder.seal_block(fast_block);
        let col_offset = self.builder.ins().ishl_imm(col_id_i64, 3);
        let col_ptr_addr = self.builder.ins().iadd(cols_base, col_offset);
        let col_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), col_ptr_addr, 0);
        let col_non_null = self.builder.ins().icmp(IntCC::NotEqual, col_ptr, zero_i64);
        let load_value_block = self.builder.create_block();
        let fast_null_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(col_non_null, load_value_block, &[], fast_null_block, &[]);

        self.builder.switch_to_block(load_value_block);
        self.builder.seal_block(load_value_block);
        let row_offset = self.builder.ins().ishl_imm(row_idx, 3);
        let value_addr = self.builder.ins().iadd(col_ptr, row_offset);
        let value_f64 = self
            .builder
            .ins()
            .load(types::F64, MemFlags::trusted(), value_addr, 0);
        let loaded = match op {
            OpCode::LoadColF64 => self.f64_to_i64(value_f64),
            OpCode::LoadColI64 => {
                let value_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, value_f64);
                let boxed_f64 = self.builder.ins().fcvt_from_sint(types::F64, value_i64);
                self.f64_to_i64(boxed_f64)
            }
            OpCode::LoadColBool => {
                let zero_f64 = self.builder.ins().f64const(0.0);
                let is_true = self
                    .builder
                    .ins()
                    .fcmp(FloatCC::NotEqual, value_f64, zero_f64);
                self.emit_boxed_bool_from_i1(is_true)
            }
            _ => self.builder.ins().iconst(types::I64, TAG_NULL as i64),
        };
        self.builder.ins().jump(merge_block, &[loaded]);

        self.builder.switch_to_block(fast_null_block);
        self.builder.seal_block(fast_null_block);
        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        self.builder.ins().jump(merge_block, &[null_val]);

        self.builder.switch_to_block(null_block);
        self.builder.seal_block(null_block);
        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        self.builder.ins().jump(merge_block, &[null_val]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        self.builder.block_params(merge_block)[0]
    }

    // ========================================================================
    // Generic DataFrame Access (Industry-Agnostic)
    // ========================================================================

    /// Compile GetDataField opcode - get field value by column index
    ///
    /// Stack: [row_offset] -> [field_value]
    /// Operand: ColumnIndex(u32) - compile-time resolved column index
    ///
    /// In kernel mode, uses series_ptrs[col_idx][cursor_index + row_offset] directly.
    pub(crate) fn compile_get_data_field(&mut self, instr: &Instruction) -> Result<(), String> {
        // Get column index from operand
        let column_index = match &instr.operand {
            Some(Operand::ColumnIndex(idx)) => *idx,
            _ => return Err("GetDataField requires ColumnIndex operand".to_string()),
        };

        // Kernel mode: direct series access via series_ptrs
        if self.mode == CompilationMode::Kernel {
            return self.compile_get_data_field_kernel(column_index);
        }

        // Pop row offset from stack
        let row_offset_i32 = if self.stack_len() >= 1 {
            let offset_val = self.stack_pop().unwrap();
            let offset_f64 = self.i64_to_f64(offset_val);
            self.builder.ins().fcvt_to_sint_sat(types::I32, offset_f64)
        } else {
            self.builder.ins().iconst(types::I32, 0)
        };

        // Call jit_get_field(ctx, row_offset, column_index)
        let col_idx_val = self.builder.ins().iconst(types::I32, column_index as i64);
        let inst = self.builder.ins().call(
            self.ffi.get_field,
            &[self.ctx_ptr, row_offset_i32, col_idx_val],
        );
        let result = self.builder.inst_results(inst)[0];
        self.stack_push(result);
        Ok(())
    }

    /// Compile GetDataRow opcode - get a lightweight row reference
    ///
    /// Stack: [row_offset] -> [row_ref]
    /// Returns TAG_INT with row index in payload
    pub(crate) fn compile_get_data_row(&mut self) -> Result<(), String> {
        // Pop row offset from stack
        let row_offset_i32 = if self.stack_len() >= 1 {
            let offset_val = self.stack_pop().unwrap();
            let offset_f64 = self.i64_to_f64(offset_val);
            self.builder.ins().fcvt_to_sint_sat(types::I32, offset_f64)
        } else {
            self.builder.ins().iconst(types::I32, 0)
        };

        // Call jit_get_row_ref(ctx, row_offset)
        let inst = self
            .builder
            .ins()
            .call(self.ffi.get_row_ref, &[self.ctx_ptr, row_offset_i32]);
        let result = self.builder.inst_results(inst)[0];
        self.stack_push(result);
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn compile_unknown(&mut self) -> Result<(), String> {
        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        self.stack_push(null_val);
        Ok(())
    }

    // ========================================================================
    // Kernel Mode Helpers (direct memory access, no FFI)
    // ========================================================================

    /// Kernel mode: GetDataField via series_ptrs[col_idx][cursor_index]
    ///
    /// In kernel mode, data access is direct pointer arithmetic:
    /// value = series_ptrs[column_index][cursor_index + row_offset]
    ///
    /// For V1, we only support row_offset=0 (current row).
    fn compile_get_data_field_kernel(&mut self, column_index: u32) -> Result<(), String> {
        let series_ptrs = self
            .kernel_series_ptrs
            .ok_or("Kernel mode requires kernel_series_ptrs")?;
        let cursor_index = self
            .kernel_cursor_index
            .ok_or("Kernel mode requires kernel_cursor_index")?;

        // Pop row offset from stack (for V1, we ignore non-zero offsets)
        // TODO: Support negative offsets for lookback
        let _ = self.stack_pop();

        // col_ptr = series_ptrs[column_index]
        let col_offset = (column_index as i64) * 8;
        let col_ptr = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            series_ptrs,
            col_offset as i32,
        );

        // value = col_ptr[cursor_index] (scaled by 8 for f64)
        // OPTIMIZATION NOTE: On x86-64, this could be folded into addressing mode
        // [base + index*8] via Cranelift's load_complex. Micro-optimization for later.
        let scaled_idx = self.builder.ins().imul_imm(cursor_index, 8);
        let value_addr = self.builder.ins().iadd(col_ptr, scaled_idx);
        let value_f64 = self
            .builder
            .ins()
            .load(types::F64, MemFlags::trusted(), value_addr, 0);

        // Box as NaN-boxed f64 (already in f64 format, just reinterpret bits)
        let value_bits = self
            .builder
            .ins()
            .bitcast(types::I64, MemFlags::new(), value_f64);
        self.stack_push(value_bits);
        Ok(())
    }
}
