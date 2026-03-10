//! Variable operations: LoadLocal, StoreLocal, LoadModuleBinding, StoreModuleBinding, closures

use cranelift::prelude::*;

use crate::context::LOCALS_OFFSET;
use crate::nan_boxing::*;
use shape_vm::bytecode::{Instruction, Operand};
use shape_vm::type_tracking::StorageHint;

use crate::translator::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    pub(crate) fn compile_load_local(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Local(idx)) = &instr.operand {
            // LICM optimization: if this local was hoisted as loop-invariant,
            // reuse the pre-loaded value instead of emitting a use_var.
            // This avoids redundant PHI nodes and register pressure in tight loops.
            let val = if let Some(&hoisted_val) = self.hoisted_locals.get(idx) {
                hoisted_val
            } else {
                let var = self.get_or_create_local(*idx);
                self.builder.use_var(var)
            };

            if self.unboxed_f64_locals.contains(idx) {
                // Float-unboxed local: read from f64-typed Variable
                if let Some(&f64_var) = self.f64_local_vars.get(idx) {
                    let f64_val = self.builder.use_var(f64_var);
                    // Push the f64 to legacy stack as NaN-boxed (stack_push expects i64)
                    let boxed = self.f64_to_i64(f64_val);
                    self.stack_push(boxed);
                    self.typed_stack
                        .replace_top(crate::translator::storage::TypedValue::f64(f64_val));
                } else {
                    // Fallback: use regular path
                    self.stack_push(val);
                }
            } else if self.unboxed_int_locals.contains(idx) {
                // Unboxed local: value is raw i64. Preserve tracked width hint so
                // typed int ops still pick width-aware lowering.
                let hint = self
                    .local_types
                    .get(idx)
                    .copied()
                    .unwrap_or(StorageHint::Int64);
                let raw = if hint.is_integer_family() {
                    self.normalize_i64_to_hint(val, hint)
                } else {
                    val
                };
                self.stack_push_typed(raw, hint);
                self.typed_stack
                    .replace_top(crate::translator::storage::TypedValue::i64(raw));
            } else {
                // Standard path: retrieve type hint if we tracked it when storing
                let hint = self
                    .local_types
                    .get(idx)
                    .copied()
                    .unwrap_or(StorageHint::Unknown);
                self.stack_push_typed(val, hint);
                if hint.is_integer_family() && !hint.is_default_int_family() {
                    let raw_i64 = self.boxed_to_i64_for_hint(val, hint);
                    self.replace_stack_top_value(raw_i64);
                    self.typed_stack
                        .replace_top(crate::translator::storage::TypedValue::i64(raw_i64));
                }
                // Promote known numeric locals to typed f64 on the typed stack.
                // This keeps numeric params/bounds on the fast path without changing
                // the boxed storage representation in locals.
                match hint {
                    StorageHint::Float64
                    | StorageHint::NullableFloat64
                    | StorageHint::Int64
                    | StorageHint::NullableInt64 => {
                        let f64_val = if let Some(&cached) = self.local_f64_cache.get(idx) {
                            cached
                        } else {
                            let converted = self.i64_to_f64(val);
                            self.local_f64_cache.insert(*idx, converted);
                            converted
                        };
                        let typed = if matches!(
                            hint,
                            StorageHint::NullableFloat64 | StorageHint::NullableInt64
                        ) {
                            crate::translator::storage::TypedValue::nullable_f64(f64_val)
                        } else {
                            crate::translator::storage::TypedValue::f64(f64_val)
                        };
                        self.typed_stack.replace_top(typed);
                    }
                    _ => {}
                }
                if let Some((data_ptr, length)) = self.hoisted_array_info.get(idx).copied() {
                    let tv = crate::translator::storage::TypedValue::boxed(val)
                        .with_hoisted_array_info(data_ptr, length);
                    self.typed_stack.replace_top(tv);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn compile_store_local(&mut self, instr: &Instruction) -> Result<(), String> {
        let local_idx = match &instr.operand {
            Some(Operand::Local(idx)) => Some(*idx),
            Some(Operand::TypedLocal(idx, _)) => Some(*idx),
            _ => None,
        };
        if let Some(idx) = &local_idx {
            // Get type hint before popping from stack
            let hinted = self.peek_stack_type();
            let existing_hint = self
                .local_types
                .get(idx)
                .copied()
                .unwrap_or(StorageHint::Unknown);
            let hint = if hinted == StorageHint::Unknown {
                existing_hint
            } else if existing_hint.is_integer_family() && hinted.is_numeric_family() {
                existing_hint
            } else {
                hinted
            };
            // Check representation of value on stack
            let top_repr = self
                .typed_stack
                .peek()
                .map(|tv| tv.repr)
                .unwrap_or(crate::translator::storage::CraneliftRepr::NanBoxed);
            let is_raw_i64 = top_repr == crate::translator::storage::CraneliftRepr::I64;
            let is_raw_f64 = top_repr == crate::translator::storage::CraneliftRepr::F64;

            // Float-unboxed local: store raw f64 to the f64-typed Variable
            if self.unboxed_f64_locals.contains(idx) {
                if let Some(&f64_var) = self.f64_local_vars.get(idx) {
                    // Get f64 value from stack, converting if needed
                    let f64_val = self.stack_pop_f64().unwrap();
                    self.builder.def_var(f64_var, f64_val);
                    self.local_f64_cache.remove(idx);
                    self.local_types.insert(*idx, hint);
                    return Ok(());
                }
            }

            if let Some(val) = self.stack_pop() {
                // Convert between representations at the boundary:
                // - raw i64 → non-unboxed local: convert to NaN-boxed
                // - NaN-boxed → unboxed local: convert to raw i64
                // - raw f64 → non-f64-unboxed local: convert to NaN-boxed
                let stored_val = if is_raw_i64 && !self.unboxed_int_locals.contains(idx) {
                    if hint.is_integer_family() {
                        self.raw_i64_to_boxed_for_hint(val, hint)
                    } else {
                        let f64_val = self.builder.ins().fcvt_from_sint(types::F64, val);
                        self.f64_to_i64(f64_val)
                    }
                } else if !is_raw_i64 && !is_raw_f64 && self.unboxed_int_locals.contains(idx) {
                    let target_hint = self
                        .local_types
                        .get(idx)
                        .copied()
                        .unwrap_or(StorageHint::Int64);
                    if target_hint.is_integer_family() {
                        self.boxed_to_i64_for_hint(val, target_hint)
                    } else {
                        let f64_val = self.i64_to_f64(val);
                        self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val)
                    }
                } else if is_raw_f64 && self.unboxed_int_locals.contains(idx) {
                    // f64 result → unboxed int local: convert f64 to raw i64.
                    // val is the NaN-boxed i64 (from f64_to_i64 in the arith path),
                    // bitcast back to f64, then convert to integer.
                    let f64_val = self.i64_to_f64(val);
                    self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val)
                } else {
                    val
                };
                // Width truncation for StoreLocalTyped
                let stored_val = if let Some(Operand::TypedLocal(_, width)) = &instr.operand {
                    if width.bits() < 64 {
                        let mask_val = width.mask() as i64;
                        let mask = self.builder.ins().iconst(types::I64, mask_val);
                        let masked = self.builder.ins().band(stored_val, mask);
                        // Sign-extend for signed widths
                        if width.is_signed() {
                            let shift = (64 - width.bits()) as i64;
                            let shift_val = self.builder.ins().iconst(types::I64, shift);
                            let shl = self.builder.ins().ishl(masked, shift_val);
                            self.builder.ins().sshr(shl, shift_val)
                        } else {
                            masked
                        }
                    } else {
                        stored_val
                    }
                } else {
                    stored_val
                };
                let var = self.get_or_create_local(*idx);
                self.builder.def_var(var, stored_val);
                self.local_f64_cache.remove(idx);
                // Track type for this local variable
                self.local_types.insert(*idx, hint);
            }
        }
        Ok(())
    }

    pub(crate) fn compile_load_global(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::ModuleBinding(idx)) = &instr.operand {
            // Check if this module binding is promoted to a register (integer unboxing)
            if let Some(&var) = self.promoted_module_bindings.get(idx) {
                let val = self.builder.use_var(var);
                if self.unboxed_int_module_bindings.contains(idx) {
                    // Unboxed: value is raw i64. Keep width hint on stack so
                    // arithmetic/comparisons use the right integer width.
                    let hint = self
                        .module_binding_types
                        .get(idx)
                        .copied()
                        .unwrap_or(StorageHint::Int64);
                    let raw = if hint.is_integer_family() {
                        self.normalize_i64_to_hint(val, hint)
                    } else {
                        val
                    };
                    self.stack_push_typed(raw, hint);
                    self.typed_stack
                        .replace_top(crate::translator::storage::TypedValue::i64(raw));
                } else {
                    let hint = self
                        .module_binding_types
                        .get(idx)
                        .copied()
                        .unwrap_or(StorageHint::Unknown);
                    self.stack_push_typed(val, hint);
                    if hint.is_integer_family() && !hint.is_default_int_family() {
                        let raw_i64 = self.boxed_to_i64_for_hint(val, hint);
                        self.replace_stack_top_value(raw_i64);
                        self.typed_stack
                            .replace_top(crate::translator::storage::TypedValue::i64(raw_i64));
                    } else if hint.is_float_family() || hint.is_default_int_family() {
                        let f64_val = self.i64_to_f64(val);
                        self.typed_stack
                            .replace_top(crate::translator::storage::TypedValue::f64(f64_val));
                    }
                }
            } else {
                let locals_offset = LOCALS_OFFSET;
                let byte_offset = locals_offset + (*idx as i32 * 8);
                let val =
                    self.builder
                        .ins()
                        .load(types::I64, MemFlags::new(), self.ctx_ptr, byte_offset);
                let hint = self
                    .module_binding_types
                    .get(idx)
                    .copied()
                    .unwrap_or(StorageHint::Unknown);
                self.stack_push_typed(val, hint);
                if hint.is_integer_family() && !hint.is_default_int_family() {
                    let raw_i64 = self.boxed_to_i64_for_hint(val, hint);
                    self.replace_stack_top_value(raw_i64);
                    self.typed_stack
                        .replace_top(crate::translator::storage::TypedValue::i64(raw_i64));
                } else if hint.is_float_family() || hint.is_default_int_family() {
                    let f64_val = self.i64_to_f64(val);
                    self.typed_stack
                        .replace_top(crate::translator::storage::TypedValue::f64(f64_val));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn compile_store_global(&mut self, instr: &Instruction) -> Result<(), String> {
        let idx_ref = match &instr.operand {
            Some(Operand::ModuleBinding(idx)) => Some(idx),
            Some(Operand::TypedModuleBinding(idx, _)) => Some(idx),
            _ => None,
        };
        if let Some(idx) = idx_ref {
            // Check if the value on stack is raw i64 (from unboxed context)
            let top_repr = self
                .typed_stack
                .peek()
                .map(|tv| tv.repr)
                .unwrap_or(crate::translator::storage::CraneliftRepr::NanBoxed);
            let is_raw_i64 = top_repr == crate::translator::storage::CraneliftRepr::I64;
            let is_raw_f64 = top_repr == crate::translator::storage::CraneliftRepr::F64;
            let hinted = self.peek_stack_type();
            let existing_hint = self
                .module_binding_types
                .get(idx)
                .copied()
                .unwrap_or(StorageHint::Unknown);
            let hint = if hinted == StorageHint::Unknown {
                existing_hint
            } else if existing_hint.is_integer_family() && hinted.is_numeric_family() {
                existing_hint
            } else {
                hinted
            };
            if let Some(val) = self.stack_pop() {
                // Check if this module binding is promoted to a register (integer unboxing)
                if let Some(&var) = self.promoted_module_bindings.get(idx) {
                    // For non-unboxed promoted bindings, keep boxed representation in the var.
                    let store_val = if is_raw_i64 && !self.unboxed_int_module_bindings.contains(idx)
                    {
                        if hint.is_integer_family() {
                            self.raw_i64_to_boxed_for_hint(val, hint)
                        } else {
                            let f64_val = self.builder.ins().fcvt_from_sint(types::F64, val);
                            self.f64_to_i64(f64_val)
                        }
                    } else if is_raw_f64 {
                        self.f64_to_i64(val)
                    } else {
                        val
                    };
                    self.builder.def_var(var, store_val);
                } else if is_raw_i64 {
                    // Storing raw i64 to a non-promoted module binding: convert to NaN-boxed
                    let boxed = if hint.is_integer_family() {
                        self.raw_i64_to_boxed_for_hint(val, hint)
                    } else {
                        let f64_val = self.builder.ins().fcvt_from_sint(types::F64, val);
                        self.f64_to_i64(f64_val)
                    };
                    let locals_offset = LOCALS_OFFSET;
                    let byte_offset = locals_offset + (*idx as i32 * 8);
                    self.builder
                        .ins()
                        .store(MemFlags::new(), boxed, self.ctx_ptr, byte_offset);
                } else {
                    let locals_offset = LOCALS_OFFSET;
                    let byte_offset = locals_offset + (*idx as i32 * 8);
                    self.builder
                        .ins()
                        .store(MemFlags::new(), val, self.ctx_ptr, byte_offset);
                }
                self.module_binding_types.insert(*idx, hint);
            }
        }
        Ok(())
    }

    pub(crate) fn compile_load_closure(&mut self, instr: &Instruction) -> Result<(), String> {
        // LoadClosure reads captured variables from ctx.locals[] at runtime.
        // This is necessary because closures receive their captures via ctx.locals[]
        // when called through jit_call_value.
        if let Some(Operand::Local(idx)) = &instr.operand {
            let locals_offset = LOCALS_OFFSET;
            let byte_offset = locals_offset + (*idx as i32 * 8);
            let val =
                self.builder
                    .ins()
                    .load(types::I64, MemFlags::new(), self.ctx_ptr, byte_offset);
            self.stack_push(val);
        }
        Ok(())
    }

    pub(crate) fn compile_store_closure(&mut self, instr: &Instruction) -> Result<(), String> {
        // StoreClosure writes to ctx.locals[] at runtime.
        // This is needed for closures that modify captured variables.
        if let Some(Operand::Local(idx)) = &instr.operand {
            if let Some(val) = self.stack_pop() {
                let locals_offset = LOCALS_OFFSET;
                let byte_offset = locals_offset + (*idx as i32 * 8);
                self.builder
                    .ins()
                    .store(MemFlags::new(), val, self.ctx_ptr, byte_offset);
            }
        }
        Ok(())
    }

    pub(crate) fn compile_make_closure(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Function(fn_id)) = &instr.operand {
            let fn_entry = self.program.functions.get(fn_id.index());
            let captures_count = fn_entry.map(|f| f.captures_count).unwrap_or(0);

            // If any captures are mutable, deopt to the interpreter.
            // Mutable captures require Arc<RwLock<>> shared state that the JIT
            // cannot efficiently manage with raw NaN-boxed u64 values.
            let has_mutable_captures = fn_entry
                .map(|f| f.mutable_captures.iter().any(|&m| m))
                .unwrap_or(false);
            if has_mutable_captures {
                let deopt = self.get_or_create_deopt_block();
                let generic_id = self.builder.ins().iconst(types::I32, u32::MAX as i64);
                self.builder.ins().jump(deopt, &[generic_id]);
                // Create an unreachable continuation block so subsequent
                // instructions have a valid insertion point.
                let unreachable_block = self.builder.create_block();
                self.builder.switch_to_block(unreachable_block);
                self.builder.seal_block(unreachable_block);
                return Ok(());
            }

            if captures_count == 0 {
                let closure_val = self
                    .builder
                    .ins()
                    .iconst(types::I64, box_function(fn_id.0) as i64);
                self.stack_push(closure_val);
            } else {
                let count = captures_count as usize;
                if self.stack_len() >= count {
                    self.materialize_to_stack(count);

                    let fn_id_val = self.builder.ins().iconst(types::I64, fn_id.0 as i64);
                    let captures_val = self.builder.ins().iconst(types::I64, captures_count as i64);
                    let inst = self.builder.ins().call(
                        self.ffi.make_closure,
                        &[self.ctx_ptr, fn_id_val, captures_val],
                    );
                    let result = self.builder.inst_results(inst)[0];

                    self.update_sp_after_ffi(count, 0);
                    self.stack_push(result);
                }
            }
        }
        Ok(())
    }
}
