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
                // Create an empty array via jit_new_array(ctx, 0), then push elements.
                // Using count=0 avoids popping anything from ctx.stack — MirToIR
                // compiles operands directly rather than staging through the stack.
                let zero = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    0i64,
                );
                let inst = self.builder.ins().call(
                    self.ffi.new_array,
                    &[self.ctx_ptr, zero],
                );
                let arr = self.builder.inst_results(inst)[0];

                for op in operands {
                    let val = self.compile_operand(op)?;
                    self.builder
                        .ins()
                        .call(self.ffi.array_push_elem, &[arr, val]);
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
                // Create a new empty object via jit_new_object(ctx, 0).
                // Using field_count=0 avoids popping anything from ctx.stack.
                let zero = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    0i64,
                );
                let inst = self.builder.ins().call(
                    self.ffi.new_object,
                    &[self.ctx_ptr, zero],
                );
                let mut obj = self.builder.inst_results(inst)[0];

                // Set each field on the object using jit_set_prop(obj, key, value).
                // field_names carries the string keys from the AST (aligned with operands).
                for (i, op) in operands.iter().enumerate() {
                    let val = self.compile_operand(op)?;

                    if let Some(name) = field_names.get(i) {
                        if !name.is_empty() {
                            // Box the field name as a NaN-boxed string at JIT compile time.
                            let boxed_key = crate::nan_boxing::box_string(name.clone());
                            let key_val = self.builder.ins().iconst(
                                cranelift::prelude::types::I64,
                                boxed_key as i64,
                            );
                            let inst = self.builder.ins().call(
                                self.ffi.set_prop,
                                &[obj, key_val, val],
                            );
                            obj = self.builder.inst_results(inst)[0];
                        }
                        // Empty name = spread entry — skip (spread merging not yet supported in MIR path)
                    }
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
                    let arr = self.builder.inst_results(inst)[0];

                    for op in operands {
                        let val = self.compile_operand(op)?;
                        self.builder
                            .ins()
                            .call(self.ffi.array_push_elem, &[arr, val]);
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
                    let val = self.compile_operand(op)?;
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
                    cranelift::prelude::types::I16,
                    fid as i64,
                );
                let cap_count = self.builder.ins().iconst(
                    cranelift::prelude::types::I16,
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
