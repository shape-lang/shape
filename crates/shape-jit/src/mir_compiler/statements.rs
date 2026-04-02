//! MIR Statement → Cranelift IR compilation.
//!
//! MIR has ~7 statement kinds (vs ~100 bytecode opcodes).
//! Ownership is structural: Assign releases old heap values,
//! Drop releases refcounts, Nop is skipped.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile a single MIR statement.
    pub(crate) fn compile_statement(
        &mut self,
        stmt: &MirStatement,
    ) -> Result<(), String> {
        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                // Release old value if overwriting a heap local.
                self.release_old_value_if_heap(place)?;
                // Compile the rvalue.
                let val = self.compile_rvalue(rvalue)?;
                // Write the new value.
                self.write_place(place, val)?;
                Ok(())
            }

            StatementKind::Drop(place) => {
                self.emit_drop(place)?;
                Ok(())
            }

            StatementKind::ArrayStore {
                container_slot,
                operands,
            } => {
                let zero = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    0i64,
                );
                let inst = self.builder.ins().call(
                    self.ffi.new_array,
                    &[self.ctx_ptr, zero],
                );
                let mut arr = self.builder.inst_results(inst)[0];

                // jit_array_push_elem expects NaN-boxed I64 elements.
                for op in operands {
                    let raw = self.compile_operand(op)?;
                    let val = self.ensure_nanboxed(raw);
                    let inst = self.builder
                        .ins()
                        .call(self.ffi.array_push_elem, &[arr, val]);
                    arr = self.builder.inst_results(inst)[0];
                }

                let place = Place::Local(*container_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, arr)?;
                Ok(())
            }

            StatementKind::ObjectStore {
                container_slot,
                operands,
                field_names,
            } => {
                // Register a schema for cross-boundary compatibility.
                let real_field_names: Vec<String> = field_names
                    .iter()
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .collect();
                let sid = shape_runtime::type_schema::register_predeclared_any_schema(
                    &real_field_names,
                );

                let schema_id = self.builder.ins().iconst(
                    cranelift::prelude::types::I32,
                    sid as i64,
                );
                let data_size = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    (operands.len() as i64) * 8,
                );
                let inst = self.builder.ins().call(
                    self.ffi.typed_object_alloc,
                    &[schema_id, data_size],
                );
                let mut obj = self.builder.inst_results(inst)[0];

                // Record field_name -> positional byte offset mapping.
                for (i, name) in field_names.iter().enumerate() {
                    if !name.is_empty() {
                        self.field_byte_offsets.insert(name.clone(), (i as u16) * 8);
                    }
                }

                // Use compile_operand_raw; box for FFI.
                for (i, op) in operands.iter().enumerate() {
                    let raw = self.compile_operand_raw(op)?;
                    let val = self.ensure_nanboxed(raw);
                    let offset_val = self.builder.ins().iconst(
                        cranelift::prelude::types::I64,
                        (i as i64) * 8,
                    );
                    let inst = self.builder.ins().call(
                        self.ffi.typed_object_set_field,
                        &[obj, offset_val, val],
                    );
                    obj = self.builder.inst_results(inst)[0];
                }

                let place = Place::Local(*container_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, obj)?;
                Ok(())
            }

            StatementKind::EnumStore {
                container_slot,
                operands,
            } => {
                // Enum variant construction.
                //
                // In the bytecode path, enums are compiled as TypedObjects with a
                // schema_id and variant discriminant. The MIR doesn't carry schema
                // information, so we represent enum payloads as arrays — the
                // preceding Assign(Aggregate) already creates an array with the
                // payload values.
                //
                // For non-empty payloads, rebuild the array from operands to ensure
                // correct ownership semantics (Move/Copy). For unit variants (empty
                // operands), the slot already holds the value from the Assign.
                if !operands.is_empty() {
                    // Create empty array, then push each element.
                    let zero = self.builder.ins().iconst(
                        cranelift::prelude::types::I64,
                        0i64,
                    );
                    let inst = self.builder.ins().call(
                        self.ffi.new_array,
                        &[self.ctx_ptr, zero],
                    );
                    let mut arr = self.builder.inst_results(inst)[0];

                    // jit_array_push_elem expects NaN-boxed I64 elements.
                    for op in operands {
                        let raw = self.compile_operand(op)?;
                        let val = self.ensure_nanboxed(raw);
                        let inst = self.builder
                            .ins()
                            .call(self.ffi.array_push_elem, &[arr, val]);
                        arr = self.builder.inst_results(inst)[0];
                    }

                    let place = Place::Local(*container_slot);
                    self.release_old_value_if_heap(&place)?;
                    self.write_place(&place, arr)?;
                }
                Ok(())
            }

            StatementKind::Nop => Ok(()),

            StatementKind::TaskBoundary(_, _) => {
                // TaskBoundary is a borrow-checker annotation consumed by the MIR
                // solver. Actual async mechanics are handled by Call terminators to
                // spawn_task/join_init FFI functions. No-op at codegen time.
                Ok(())
            }

            StatementKind::ClosureCapture {
                closure_slot,
                operands,
                function_id,
            } => {
                // Create a closure by pushing captures to ctx.stack and calling jit_make_closure.
                let fid = function_id.ok_or_else(|| {
                    "MirToIR: ClosureCapture missing function_id (MIR not patched)".to_string()
                })?;

                // Push each capture operand to ctx.stack[stack_ptr + i]
                let stack_base = crate::context::STACK_OFFSET as i32;
                let sp_offset = crate::context::STACK_PTR_OFFSET as i32;
                let old_sp = self.builder.ins().load(
                    cranelift::prelude::types::I64,
                    MemFlags::new(),
                    self.ctx_ptr,
                    sp_offset,
                );

                for (i, op) in operands.iter().enumerate() {
                    let raw = self.compile_operand(op)?;
                    let val = self.ensure_nanboxed(raw);
                    let slot_idx = self.builder.ins().iadd_imm(old_sp, i as i64);
                    let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
                    let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base as i64);
                    let addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
                    self.builder.ins().store(MemFlags::new(), val, addr, 0);
                }

                // Update ctx.stack_ptr += captures_count
                let new_sp = self.builder.ins().iadd_imm(old_sp, operands.len() as i64);
                self.builder.ins().store(MemFlags::new(), new_sp, self.ctx_ptr, sp_offset);

                // Call jit_make_closure(ctx, function_id, captures_count)
                let fid_val = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    fid as i64,
                );
                let cap_count = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    operands.len() as i64,
                );
                let inst = self.builder.ins().call(
                    self.ffi.make_closure,
                    &[self.ctx_ptr, fid_val, cap_count],
                );
                let closure_val = self.builder.inst_results(inst)[0];

                // Store the closure in the closure_slot
                let place = Place::Local(*closure_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, closure_val)?;
                Ok(())
            }
        }
    }
}
