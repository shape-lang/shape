//! Collection operations: arrays, objects, slices, ranges

use cranelift::prelude::*;

use crate::nan_boxing::*;
use shape_vm::bytecode::{Instruction, Operand};
use shape_vm::type_tracking::StorageHint;

use crate::translator::storage::CraneliftRepr;
use crate::translator::types::BytecodeToIR;

// Temporarily disabled by default until dense-bool fast paths are retuned.
const ENABLE_BOOL_DENSE_ARRAY_PATH: bool = false;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    // Array/Object operations
    pub(crate) fn compile_new_array(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Count(count)) = &instr.operand {
            let count_usize = *count as usize;

            // Escape analysis: check if this NewArray is eligible for scalar replacement.
            if let Some(entry) = self
                .optimization_plan
                .escape_analysis
                .scalar_arrays
                .get(&self.current_instr_idx)
                .cloned()
            {
                // Scalar replacement: allocate Cranelift variables for each element.
                let mut element_vars = Vec::with_capacity(entry.element_count);
                for _ in 0..entry.element_count {
                    let var = Variable::new(self.next_var);
                    self.next_var += 1;
                    self.builder.declare_var(var, types::I64);
                    element_vars.push(var);
                }

                if count_usize > 0 && count_usize <= entry.element_count {
                    // Pop initial elements directly from the JIT operand stack.
                    // Stack order: elem_0 is deepest, elem_{n-1} is TOS.
                    // Pop in reverse: pop -> elem[n-1], ..., pop -> elem[0].
                    let mut popped = Vec::with_capacity(count_usize);
                    for _ in 0..count_usize {
                        if let Some(val) = self.stack_pop_boxed() {
                            popped.push(val);
                        }
                    }
                    // popped[0] = elem[n-1], popped[n-1] = elem[0]
                    for (i, val) in popped.into_iter().rev().enumerate() {
                        self.builder.def_var(element_vars[i], val);
                    }
                    // Initialize any remaining slots to TAG_NULL.
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    for var in element_vars.iter().skip(count_usize) {
                        self.builder.def_var(*var, null_val);
                    }
                } else {
                    // Zero-element array: initialize all slots to TAG_NULL.
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    for var in &element_vars {
                        self.builder.def_var(*var, null_val);
                    }
                }

                // Register this array for scalar replacement.
                self.scalar_replaced_arrays
                    .insert(entry.local_slot, element_vars);

                // Push a sentinel value (TAG_NULL) onto the stack.
                // The StoreLocal that follows will consume it, but the actual
                // array operations will use the scalar variables instead.
                let sentinel = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                self.stack_push(sentinel);

                return Ok(());
            }

            self.materialize_to_stack(count_usize);

            let count_val = self.builder.ins().iconst(types::I64, *count as i64);
            let inst = self
                .builder
                .ins()
                .call(self.ffi.new_array, &[self.ctx_ptr, count_val]);
            let result = self.builder.inst_results(inst)[0];

            self.update_sp_after_ffi(count_usize, 0);
            self.stack_push(result);
        }
        Ok(())
    }

    pub(crate) fn compile_new_object(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Count(field_count)) = &instr.operand {
            let pair_count = (*field_count as usize) * 2;
            self.materialize_to_stack(pair_count);

            let count_val = self.builder.ins().iconst(types::I64, *field_count as i64);
            let inst = self
                .builder
                .ins()
                .call(self.ffi.new_object, &[self.ctx_ptr, count_val]);
            let result = self.builder.inst_results(inst)[0];

            self.update_sp_after_ffi(pair_count, 0);
            self.stack_push(result);
        }
        Ok(())
    }

    pub(crate) fn compile_get_prop(&mut self, instr: &Instruction) -> Result<(), String> {
        // Generic property access - field names are resolved via schema at compile time
        // Clear any pending data offset since we're using standard property access
        self.pending_data_offset = None;

        // Escape analysis: scalar-replaced array read.
        // Check if this GetProp is a planned scalar read site.
        if instr.operand.is_none() {
            let scalar_var = self
                .optimization_plan
                .escape_analysis
                .scalar_arrays
                .values()
                .find_map(|entry| {
                    entry
                        .get_sites
                        .get(&self.current_instr_idx)
                        .map(|&elem_idx| (entry.local_slot, elem_idx))
                })
                .and_then(|(local_slot, elem_idx)| {
                    self.scalar_replaced_arrays
                        .get(&local_slot)
                        .and_then(|vars| vars.get(elem_idx).copied())
                });
            if let Some(var) = scalar_var {
                // Pop the index and array sentinel from the stack.
                let _key = self.stack_pop();
                let _arr = self.stack_pop();
                // Read from the scalar variable.
                let val = self.builder.use_var(var);
                self.stack_push(val);
                return Ok(());
            }
        }

        {
            // Check if we're in a try block - need to handle "not found" as exception
            let in_try_block = !self.exception_handlers.is_empty();
            let catch_idx = self.exception_handlers.last().copied();

            // Fast path for dynamic index reads (no Property operand).
            let (result, obj_val, numeric_hint) =
                if self.stack_len() >= 2 && instr.operand.is_none() {
                    let hoisted = self.typed_stack.second_hoisted_array_info();
                    let planned_int_numeric = self
                        .optimization_plan
                        .numeric_arrays
                        .int_get_sites
                        .contains(&self.current_instr_idx);
                    let planned_float_numeric = self
                        .optimization_plan
                        .numeric_arrays
                        .float_get_sites
                        .contains(&self.current_instr_idx);
                    let planned_bool = self
                        .optimization_plan
                        .numeric_arrays
                        .bool_get_sites
                        .contains(&self.current_instr_idx)
                        && ENABLE_BOOL_DENSE_ARRAY_PATH;
                    let speculate_numeric_array = !planned_int_numeric
                        && !planned_float_numeric
                        && !planned_bool
                        && self.should_speculate_numeric_array_read();
                    let numeric_hint = if planned_int_numeric {
                        Some(StorageHint::Int64)
                    } else if planned_bool {
                        Some(StorageHint::Bool)
                    } else if planned_float_numeric || speculate_numeric_array {
                        Some(StorageHint::Float64)
                    } else {
                        None
                    };
                    let key_hint = self.peek_stack_type();
                    let key_is_raw_i64 = self
                        .typed_stack
                        .peek()
                        .map(|tv| tv.repr == CraneliftRepr::I64)
                        .unwrap_or(false);
                    let key_is_typed_int = !key_is_raw_i64 && key_hint.is_integer_family();
                    let trusted_get = self
                        .optimization_plan
                        .trusted_array_get_indices
                        .contains(&self.current_instr_idx);
                    let non_negative_get = self
                        .optimization_plan
                        .non_negative_array_get_indices
                        .contains(&self.current_instr_idx);
                    let key = self.stack_pop().unwrap();
                    let obj = self.stack_pop().unwrap();
                    let key_i64 = if key_is_raw_i64 {
                        Some(key)
                    } else if key_is_typed_int {
                        let key_f64 = self.i64_to_f64(key);
                        Some(self.builder.ins().fcvt_to_sint_sat(types::I64, key_f64))
                    } else {
                        None
                    };
                    if let Some((data_ptr, length)) = hoisted {
                        if let Some(key_i64) = key_i64 {
                            let result = if planned_bool {
                                if trusted_get {
                                    self.inline_array_get_hoisted_i64_trusted_bool(
                                        obj, key_i64, data_ptr, length,
                                    )
                                } else if non_negative_get {
                                    self.inline_array_get_hoisted_i64_non_negative_bool(
                                        obj, key_i64, data_ptr, length,
                                    )
                                } else {
                                    self.inline_array_get_hoisted_i64_bool(
                                        obj, key_i64, data_ptr, length,
                                    )
                                }
                            } else if trusted_get {
                                // Hoisted fast path for typed kernels.
                                self.inline_array_get_hoisted_i64_trusted(key_i64, data_ptr, length)
                            } else if non_negative_get {
                                self.inline_array_get_hoisted_i64_non_negative(
                                    key_i64, data_ptr, length,
                                )
                            } else {
                                self.inline_array_get_hoisted_i64(key_i64, data_ptr, length)
                            };
                            (Some(result), Some(obj), numeric_hint)
                        } else {
                            let result = self.inline_array_get_hoisted(key, data_ptr, length);
                            (Some(result), Some(obj), numeric_hint)
                        }
                    } else {
                        let result = if let Some(key_i64) = key_i64 {
                            if planned_bool {
                                if trusted_get {
                                    self.inline_array_get_i64_trusted_bool(obj, key_i64)
                                } else if non_negative_get {
                                    self.inline_array_get_i64_non_negative_bool(obj, key_i64)
                                } else {
                                    self.inline_array_get_i64_bool(obj, key_i64)
                                }
                            } else if trusted_get {
                                self.inline_array_get_i64_trusted(obj, key_i64)
                            } else if non_negative_get {
                                self.inline_array_get_i64_non_negative(obj, key_i64)
                            } else {
                                self.inline_array_get_i64(obj, key_i64)
                            }
                        } else {
                            // Non-integer key (string property name) — must use FFI
                            // because the object might be a TypedObject or HashMap,
                            // not an array.
                            let inst = self.builder.ins().call(self.ffi.get_prop, &[obj, key]);
                            self.builder.inst_results(inst)[0]
                        };
                        (Some(result), Some(obj), numeric_hint)
                    }
                } else if self.stack_len() >= 2 {
                    let key = self.stack_pop().unwrap();
                    let obj = self.stack_pop().unwrap();

                    let key_val = if let Some(Operand::Property(prop_idx)) = &instr.operand {
                        let prop_name = &self.program.strings[*prop_idx as usize];
                        let boxed_key = jit_box(HK_STRING, prop_name.clone());
                        self.builder.ins().iconst(types::I64, boxed_key as i64)
                    } else {
                        key
                    };

                    let inst = self.builder.ins().call(self.ffi.get_prop, &[obj, key_val]);
                    (Some(self.builder.inst_results(inst)[0]), Some(obj), None)
                } else if self.stack_len() >= 1 {
                    if let Some(Operand::Property(prop_idx)) = &instr.operand {
                        let obj = self.stack_pop().unwrap();

                        // Tier 2: try feedback-guided speculative property access.
                        // TypedObject path: schema_id guard → direct indexed field load.
                        // HashMap path: shape_id guard → O(1) indexed slot access.
                        let speculative_result = if self.has_feedback() {
                            let bc_offset = self.current_instr_idx;
                            self.speculative_property_info(bc_offset).and_then(
                                |(schema_id, field_idx, field_type_tag, receiver_kind)| {
                                    if receiver_kind == 1 {
                                        // HashMap: shape-guarded O(1) access with FFI fallback
                                        let prop_name = &self.program.strings[*prop_idx as usize];
                                        let boxed_key = jit_box(HK_STRING, prop_name.clone());
                                        let key_val =
                                            self.builder.ins().iconst(types::I64, boxed_key as i64);
                                        Some(self.emit_shape_guarded_get_with_fallback(
                                            obj,
                                            shape_value::shape_graph::ShapeId(schema_id as u32),
                                            field_idx as usize,
                                            key_val,
                                        ))
                                    } else {
                                        // TypedObject: schema guard → direct indexed field load
                                        self.emit_speculative_property_load(
                                            obj,
                                            schema_id,
                                            field_idx,
                                            field_type_tag,
                                            bc_offset,
                                        )
                                    }
                                },
                            )
                        } else {
                            None
                        };

                        if let Some(result) = speculative_result {
                            (Some(result), Some(obj), None)
                        } else {
                            // Generic property access via FFI
                            let prop_name = &self.program.strings[*prop_idx as usize];
                            let boxed_key = jit_box(HK_STRING, prop_name.clone());
                            let key_val = self.builder.ins().iconst(types::I64, boxed_key as i64);
                            let inst = self.builder.ins().call(self.ffi.get_prop, &[obj, key_val]);
                            (Some(self.builder.inst_results(inst)[0]), Some(obj), None)
                        }
                    } else {
                        (None, None, None)
                    }
                } else {
                    (None, None, None)
                };

            if let Some(result) = result {
                // If in try block and result is null AND object was an object type, jump to catch
                if in_try_block {
                    if let (Some(catch_idx), Some(obj_val)) = (catch_idx, obj_val) {
                        // Check if object was a typed object and result is TAG_NULL
                        let is_object = self.emit_is_heap_kind(obj_val, HK_TYPED_OBJECT);

                        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                        let is_null = self.builder.ins().icmp(IntCC::Equal, result, null_val);

                        let should_throw = self.builder.ins().band(is_object, is_null);

                        // Create blocks for the branch
                        let throw_block = self.builder.create_block();
                        let continue_block = self.builder.create_block();
                        self.builder.append_block_param(continue_block, types::I64);

                        self.builder.ins().brif(
                            should_throw,
                            throw_block,
                            &[],
                            continue_block,
                            &[result],
                        );

                        // Throw block: jump to catch handler
                        self.builder.switch_to_block(throw_block);
                        self.builder.seal_block(throw_block);
                        if let Some(&catch_block) = self.blocks.get(&catch_idx) {
                            // Pop exception handler since we're jumping to it
                            self.exception_handlers.pop();
                            self.block_stack_depth.insert(catch_idx, 0);
                            let error_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                            self.builder.ins().jump(catch_block, &[error_val]);
                        } else {
                            // No block found, just return null
                            let error_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                            self.builder.ins().jump(continue_block, &[error_val]);
                        }

                        // Continue block: use the result
                        self.builder.switch_to_block(continue_block);
                        self.builder.seal_block(continue_block);
                        let continued_result = self.builder.block_params(continue_block)[0];
                        self.stack_push(continued_result);
                    } else {
                        self.stack_push(result);
                    }
                } else {
                    match numeric_hint {
                        Some(StorageHint::Int64) => {
                            let result_f64 = self.i64_to_f64(result);
                            let result_i64 =
                                self.builder.ins().fcvt_to_sint_sat(types::I64, result_f64);
                            self.stack_push_typed(result_i64, StorageHint::Int64);
                            self.typed_stack.replace_top(
                                crate::translator::storage::TypedValue::i64(result_i64),
                            );
                        }
                        Some(StorageHint::Float64) => {
                            self.stack_push_typed(result, StorageHint::Float64);
                            let result_f64 = self.i64_to_f64(result);
                            self.typed_stack.replace_top(
                                crate::translator::storage::TypedValue::f64(result_f64),
                            );
                        }
                        Some(StorageHint::Bool) => {
                            self.stack_push_typed(result, StorageHint::Bool);
                        }
                        _ => self.stack_push(result),
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn compile_set_prop(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Property(prop_idx)) = &instr.operand {
            // Static property key from operand
            if self.stack_len() >= 2 {
                let value = self.stack_pop().unwrap();
                let obj = self.stack_pop().unwrap();

                let prop_name = &self.program.strings[*prop_idx as usize];
                let boxed_key = jit_box(HK_STRING, prop_name.clone());
                let key_val = self.builder.ins().iconst(types::I64, boxed_key as i64);

                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.set_prop, &[obj, key_val, value]);
                let result = self.builder.inst_results(inst)[0];
                self.stack_push(result);
            }
        } else {
            // Dynamic key from stack (for array index or range assignment)
            if self.stack_len() >= 3 {
                let value = self.stack_pop_boxed().unwrap();
                let key = self.stack_pop_boxed().unwrap();
                let obj = self.stack_pop().unwrap();

                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.set_prop, &[obj, key, value]);
                let result = self.builder.inst_results(inst)[0];
                self.stack_push(result);
            }
        }
        Ok(())
    }

    pub(crate) fn compile_set_local_index(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Local(idx)) = &instr.operand {
            // Escape analysis: scalar-replaced array write.
            // Look up the element index from the plan and the scalar variable.
            let scalar_var = self
                .optimization_plan
                .escape_analysis
                .scalar_arrays
                .values()
                .find_map(|entry| {
                    if entry.local_slot == *idx {
                        entry
                            .set_sites
                            .get(&self.current_instr_idx)
                            .copied()
                    } else {
                        None
                    }
                })
                .and_then(|elem_idx| {
                    self.scalar_replaced_arrays
                        .get(idx)
                        .and_then(|vars| vars.get(elem_idx).copied())
                });
            if let Some(var) = scalar_var {
                if self.stack_len() >= 2 {
                    // Pop the value and index from the stack.
                    let value = self.stack_pop_boxed().unwrap();
                    let _key = self.stack_pop();
                    // Write to the scalar variable.
                    self.builder.def_var(var, value);
                    return Ok(());
                }
            }

            if self.stack_len() >= 2 {
                let value = self.stack_pop_boxed().unwrap();
                let key_hint = self.peek_stack_type();
                let key_is_raw_i64 = self
                    .typed_stack
                    .peek()
                    .map(|tv| tv.repr == CraneliftRepr::I64)
                    .unwrap_or(false);
                let key_is_typed_int = !key_is_raw_i64 && key_hint.is_integer_family();
                let key = self.stack_pop().unwrap();

                // Load current array from the local variable
                let var = self.get_or_create_local(*idx);
                let array = self.builder.use_var(var);
                let trusted_set = self
                    .optimization_plan
                    .trusted_array_set_indices
                    .contains(&self.current_instr_idx);
                let non_negative_set = self
                    .optimization_plan
                    .non_negative_array_set_indices
                    .contains(&self.current_instr_idx);
                let planned_bool_set = self
                    .optimization_plan
                    .numeric_arrays
                    .bool_set_sites
                    .contains(&self.current_instr_idx)
                    && ENABLE_BOOL_DENSE_ARRAY_PATH;
                let key_i64 = if key_is_raw_i64 {
                    Some(key)
                } else if key_is_typed_int {
                    let key_f64 = self.i64_to_f64(key);
                    Some(self.builder.ins().fcvt_to_sint_sat(types::I64, key_f64))
                } else {
                    None
                };

                // If Array LICM hoisted this local's data_ptr/length, reuse it to
                // avoid redundant per-store array metadata loads.
                if let Some(&(data_ptr, length)) = self.hoisted_array_info.get(idx) {
                    if let Some(key_i64) = key_i64 {
                        if planned_bool_set {
                            if trusted_set {
                                self.inline_array_set_hoisted_i64_trusted_bool(
                                    array, key_i64, data_ptr, length, value,
                                );
                            } else if non_negative_set {
                                self.inline_array_set_hoisted_i64_non_negative_bool(
                                    array, key_i64, data_ptr, length, value,
                                );
                            } else {
                                self.inline_array_set_hoisted_i64_bool(
                                    array, key_i64, data_ptr, length, value,
                                );
                            }
                        } else if trusted_set {
                            self.inline_array_set_hoisted_i64_trusted(
                                array, key_i64, data_ptr, length, value,
                            );
                        } else if non_negative_set {
                            self.inline_array_set_hoisted_i64_non_negative(
                                array, key_i64, data_ptr, length, value,
                            );
                        } else {
                            self.inline_array_set_hoisted_i64(
                                array, key_i64, data_ptr, length, value,
                            );
                        }
                        return Ok(());
                    } else if planned_bool_set {
                        let key_f64 = self.i64_to_f64(key);
                        let idx_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, key_f64);
                        if trusted_set {
                            self.inline_array_set_hoisted_i64_trusted_bool(
                                array, idx_i64, data_ptr, length, value,
                            );
                        } else if non_negative_set {
                            self.inline_array_set_hoisted_i64_non_negative_bool(
                                array, idx_i64, data_ptr, length, value,
                            );
                        } else {
                            self.inline_array_set_hoisted_i64_bool(
                                array, idx_i64, data_ptr, length, value,
                            );
                        }
                        return Ok(());
                    }
                }

                if let Some(key_i64) = key_i64 {
                    if planned_bool_set {
                        if trusted_set {
                            self.inline_array_set_i64_trusted_bool(array, key_i64, value);
                        } else if non_negative_set {
                            self.inline_array_set_i64_non_negative_bool(array, key_i64, value);
                        } else {
                            self.inline_array_set_i64_bool(array, key_i64, value);
                        }
                    } else if trusted_set {
                        self.inline_array_set_i64_trusted(array, key_i64, value);
                    } else if non_negative_set {
                        self.inline_array_set_i64_non_negative(array, key_i64, value);
                    } else {
                        self.inline_array_set_i64(array, key_i64, value);
                    }
                } else if planned_bool_set {
                    let key_f64 = self.i64_to_f64(key);
                    let idx_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, key_f64);
                    if trusted_set {
                        self.inline_array_set_i64_trusted_bool(array, idx_i64, value);
                    } else if non_negative_set {
                        self.inline_array_set_i64_non_negative_bool(array, idx_i64, value);
                    } else {
                        self.inline_array_set_i64_bool(array, idx_i64, value);
                    }
                } else {
                    self.inline_array_set(array, key, value);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn compile_set_module_binding_index(
        &mut self,
        instr: &Instruction,
    ) -> Result<(), String> {
        if let Some(Operand::ModuleBinding(idx)) = &instr.operand {
            if self.stack_len() >= 2 {
                let value = self.stack_pop_boxed().unwrap();
                let key_hint = self.peek_stack_type();
                let key_is_raw_i64 = self
                    .typed_stack
                    .peek()
                    .map(|tv| tv.repr == CraneliftRepr::I64)
                    .unwrap_or(false);
                let key_is_typed_int = !key_is_raw_i64 && key_hint.is_integer_family();
                let key = self.stack_pop().unwrap();

                // Load current array from module_binding slot in JITContext
                let locals_offset = crate::context::LOCALS_OFFSET;
                let byte_offset = locals_offset + (*idx as i32 * 8);
                let array =
                    self.builder
                        .ins()
                        .load(types::I64, MemFlags::new(), self.ctx_ptr, byte_offset);
                let trusted_set = self
                    .optimization_plan
                    .trusted_array_set_indices
                    .contains(&self.current_instr_idx);
                let non_negative_set = self
                    .optimization_plan
                    .non_negative_array_set_indices
                    .contains(&self.current_instr_idx);
                let planned_bool_set = self
                    .optimization_plan
                    .numeric_arrays
                    .bool_set_sites
                    .contains(&self.current_instr_idx)
                    && ENABLE_BOOL_DENSE_ARRAY_PATH;
                let key_i64 = if key_is_raw_i64 {
                    Some(key)
                } else if key_is_typed_int {
                    let key_f64 = self.i64_to_f64(key);
                    Some(self.builder.ins().fcvt_to_sint_sat(types::I64, key_f64))
                } else {
                    None
                };

                if let Some(key_i64) = key_i64 {
                    if planned_bool_set {
                        if trusted_set {
                            self.inline_array_set_i64_trusted_bool(array, key_i64, value);
                        } else if non_negative_set {
                            self.inline_array_set_i64_non_negative_bool(array, key_i64, value);
                        } else {
                            self.inline_array_set_i64_bool(array, key_i64, value);
                        }
                    } else if trusted_set {
                        self.inline_array_set_i64_trusted(array, key_i64, value);
                    } else if non_negative_set {
                        self.inline_array_set_i64_non_negative(array, key_i64, value);
                    } else {
                        self.inline_array_set_i64(array, key_i64, value);
                    }
                } else if planned_bool_set {
                    let key_f64 = self.i64_to_f64(key);
                    let idx_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, key_f64);
                    if trusted_set {
                        self.inline_array_set_i64_trusted_bool(array, idx_i64, value);
                    } else if non_negative_set {
                        self.inline_array_set_i64_non_negative_bool(array, idx_i64, value);
                    } else {
                        self.inline_array_set_i64_bool(array, idx_i64, value);
                    }
                } else {
                    self.inline_array_set(array, key, value);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn compile_length(&mut self) -> Result<(), String> {
        if self.stack_depth == 0 {
            return Ok(());
        }
        // Peek type before popping to decide path
        let hint = self.peek_stack_type();

        if matches!(hint, StorageHint::String | StorageHint::Unknown) {
            // String or unknown type — use jit_length FFI which handles
            // arrays, strings, objects, etc.
            let value = self.stack_pop_boxed().unwrap();
            let inst = self.builder.ins().call(self.ffi.length, &[value]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push_typed(result, StorageHint::Float64);
        } else {
            // Known array — inline the fast path
            let value = self.stack_pop().unwrap();
            let result = self.inline_array_length(value);
            self.stack_push_typed(result, StorageHint::Int64);
        }
        Ok(())
    }

    pub(crate) fn compile_slice_access(&mut self) -> Result<(), String> {
        if self.stack_len() >= 3 {
            let end = self.stack_pop().unwrap();
            let start = self.stack_pop().unwrap();
            let arr = self.stack_pop().unwrap();
            let inst = self.builder.ins().call(self.ffi.slice, &[arr, start, end]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }

    pub(crate) fn compile_array_push(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            // Value stored to array must be NaN-boxed
            let value = self.stack_pop_boxed().unwrap();
            let array = self.stack_pop().unwrap();
            let inst = self
                .builder
                .ins()
                .call(self.ffi.array_push_elem, &[array, value]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }

    pub(crate) fn compile_array_push_local(&mut self, instr: &Instruction) -> Result<(), String> {
        match &instr.operand {
            Some(Operand::Local(idx)) => {
                // Value stored to array must be NaN-boxed (not raw i64 from unboxing)
                if let Some(value) = self.stack_pop_boxed() {
                    let var = self.get_or_create_local(*idx);
                    let arr = self.builder.use_var(var);
                    let result = if self
                        .trusted_array_push_local_sites
                        .contains(&self.current_instr_idx)
                    {
                        if let Some(&iv_slot) = self
                            .trusted_array_push_local_iv_by_site
                            .get(&self.current_instr_idx)
                        {
                            let iv_var = self.get_or_create_local(iv_slot);
                            let iv_val = self.builder.use_var(iv_var);
                            let iv_i64 = if self.unboxed_int_locals.contains(&iv_slot) {
                                iv_val
                            } else if self.unboxed_f64_locals.contains(&iv_slot)
                                && let Some(&f64_var) = self.f64_local_vars.get(&iv_slot)
                            {
                                let f64_val = self.builder.use_var(f64_var);
                                self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val)
                            } else {
                                let iv_f64 = self.i64_to_f64(iv_val);
                                self.builder.ins().fcvt_to_sint_sat(types::I64, iv_f64)
                            };
                            self.inline_array_push_at_index_trusted_capacity(arr, iv_i64, value)
                        } else {
                            self.inline_array_push_trusted_capacity(arr, value)
                        }
                    } else {
                        self.inline_array_push(arr, value)
                    };
                    self.builder.def_var(var, result);
                }
            }
            Some(Operand::ModuleBinding(idx)) => {
                if let Some(value) = self.stack_pop_boxed() {
                    let byte_offset = crate::context::LOCALS_OFFSET + (*idx as i32 * 8);
                    let arr = self.builder.ins().load(
                        types::I64,
                        MemFlags::new(),
                        self.ctx_ptr,
                        byte_offset,
                    );
                    let result = self.inline_array_push(arr, value);
                    self.builder
                        .ins()
                        .store(MemFlags::new(), result, self.ctx_ptr, byte_offset);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Inline fast path for array push: tag check + capacity check, then direct
    /// memory store. Falls back to FFI (`jit_array_push_local`) when the value
    /// is not a JitArray or the backing buffer needs to grow.
    fn inline_array_push(
        &mut self,
        arr: cranelift::codegen::ir::Value,
        value: cranelift::codegen::ir::Value,
    ) -> cranelift::codegen::ir::Value {
        // Heap kind check: is this a JitArray?
        let is_array = self.emit_is_heap_kind(arr, HK_ARRAY);

        // Extract JitArray pointer: (arr & PAYLOAD_MASK) + JIT_ALLOC_DATA_OFFSET
        let arr_ptr = self.emit_jit_alloc_data_ptr(arr);

        // Load len (offset 8) and cap (offset 16) from JitArray repr(C)
        let len = self.emit_trusted_load(types::I64, arr_ptr, 8);
        let cap = self.emit_trusted_load(types::I64, arr_ptr, 16);
        let has_capacity = self.builder.ins().icmp(IntCC::UnsignedLessThan, len, cap);

        // Both conditions must pass for inline path
        let can_inline = self.builder.ins().band(is_array, has_capacity);

        let inline_block = self.builder.create_block();
        let ffi_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);

        self.builder
            .ins()
            .brif(can_inline, inline_block, &[], ffi_block, &[]);

        // Inline path: store element at data[len], increment len
        self.builder.switch_to_block(inline_block);
        self.builder.seal_block(inline_block);
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        let eight = self.builder.ins().iconst(types::I64, 8);
        let offset = self.builder.ins().imul(len, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, offset);
        self.builder
            .ins()
            .store(MemFlags::new(), value, elem_addr, 0);
        let new_len = self.builder.ins().iadd_imm(len, 1);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_len, arr_ptr, 8);
        // Sync the typed bool bitset overlay (if present) so that
        // subsequent bool-dense reads see the correct value.
        self.emit_sync_bool_typed_slot_if_present(arr_ptr, len, value);
        // Return the original tagged array (unchanged)
        self.builder.ins().jump(merge_block, &[arr]);

        // FFI fallback path: handles grow + re-tag
        self.builder.switch_to_block(ffi_block);
        self.builder.seal_block(ffi_block);
        let inst = self
            .builder
            .ins()
            .call(self.ffi.array_push_local, &[arr, value]);
        let ffi_result = self.builder.inst_results(inst)[0];
        self.builder.ins().jump(merge_block, &[ffi_result]);

        // Merge block: result is the (possibly updated) tagged array
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        self.builder.block_params(merge_block)[0]
    }

    /// Inline push for sites where loop entry already reserved enough capacity.
    ///
    /// Preconditions guaranteed by loop planning:
    /// - `arr` is an array value
    /// - capacity is sufficient for all loop pushes
    fn inline_array_push_trusted_capacity(
        &mut self,
        arr: cranelift::codegen::ir::Value,
        value: cranelift::codegen::ir::Value,
    ) -> cranelift::codegen::ir::Value {
        let arr_ptr = self.emit_jit_alloc_data_ptr(arr);

        let data_ptr = self.emit_trusted_load(types::I64, arr_ptr, 0);
        let len = self.emit_trusted_load(types::I64, arr_ptr, 8);

        let offset = self.builder.ins().ishl_imm(len, 3);
        let elem_addr = self.builder.ins().iadd(data_ptr, offset);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, elem_addr, 0);

        let new_len = self.builder.ins().iadd_imm(len, 1);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_len, arr_ptr, 8);
        // Sync the typed bool bitset overlay (if present).
        self.emit_sync_bool_typed_slot_if_present(arr_ptr, len, value);
        arr
    }

    /// Inline push for trusted-capacity loops where the loop IV is known to be
    /// the append index (typically `i` starting at 0 with one push per trip).
    ///
    /// This skips the per-iteration len load: store to data[index] and set
    /// len = index + 1.
    fn inline_array_push_at_index_trusted_capacity(
        &mut self,
        arr: cranelift::codegen::ir::Value,
        index_i64: cranelift::codegen::ir::Value,
        value: cranelift::codegen::ir::Value,
    ) -> cranelift::codegen::ir::Value {
        let arr_ptr = self.emit_jit_alloc_data_ptr(arr);

        let data_ptr = self.emit_trusted_load(types::I64, arr_ptr, 0);
        let offset = self.builder.ins().ishl_imm(index_i64, 3);
        let elem_addr = self.builder.ins().iadd(data_ptr, offset);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, elem_addr, 0);

        let new_len = self.builder.ins().iadd_imm(index_i64, 1);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_len, arr_ptr, 8);
        // Sync the typed bool bitset overlay (if present).
        self.emit_sync_bool_typed_slot_if_present(arr_ptr, index_i64, value);
        arr
    }

    pub(crate) fn compile_array_pop(&mut self) -> Result<(), String> {
        if let Some(array) = self.stack_pop() {
            let inst = self.builder.ins().call(self.ffi.array_pop, &[array]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }

    pub(crate) fn compile_null_coalesce(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let b = self.stack_pop().unwrap();
            let a = self.stack_pop().unwrap();

            let null_tag = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            let a_is_null = self.builder.ins().icmp(IntCC::Equal, a, null_tag);
            let result = self.builder.ins().select(a_is_null, b, a);
            self.stack_push(result);
        }
        Ok(())
    }

    /// MergeObject: pop two objects, push merged result via FFI.
    /// Uses generic_add FFI which handles object merging for non-numeric types.
    pub(crate) fn compile_merge_object(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let b = self.stack_pop().unwrap();
            let a = self.stack_pop().unwrap();
            let inst = self.builder.ins().call(self.ffi.generic_add, &[a, b]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }

    /// Convert: generic type conversion.
    /// In NaN-boxing, most conversions are identity operations since int and float
    /// are both stored as f64. The compiler has already ensured type compatibility.
    pub(crate) fn compile_convert(&mut self) -> Result<(), String> {
        // Identity pass-through: value stays on stack unchanged.
        // The compiler guarantees type compatibility at this point.
        Ok(())
    }

    pub(crate) fn compile_make_range(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let end_val = self.stack_pop().unwrap();
            let start_val = self.stack_pop().unwrap();

            // Call jit_make_range(start, end) -> Range object
            let inst = self
                .builder
                .ins()
                .call(self.ffi.make_range, &[start_val, end_val]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }
}
