//! Object and array operations for the VM executor
//!
//! Handles: NewArray, NewObject, GetProp, SetProp, Length, ArrayPush, ArrayPop, MakeClosure, MergeObject, NewTypedObject, TypedMergeObject, CallMethod

// PHF method registry
pub mod method_registry;

// Property access operations (GetProp, SetProp, Length)
pub mod property_access;

// Object creation operations (NewArray, NewObject, NewTypedObject)
pub mod object_creation;

// Object merge operations (MergeObject, TypedMergeObject)
pub mod object_operations;

// Array operations (ArrayPush, ArrayPop, SliceAccess)
pub mod array_operations;

// Array method modules
pub mod array_aggregation;
pub mod array_basic;
pub mod array_joins;
pub mod array_query;
pub mod array_sets;
pub mod array_sort;
pub mod array_transform;

// DataTable method handlers
pub mod datatable_methods;

// Column method handlers
pub mod column_methods;

// IndexedTable method handlers
pub mod indexed_table_methods;

// HashMap method handlers
pub mod hashmap_methods;

// Set method handlers
pub mod deque_methods;
pub mod priority_queue_methods;
pub mod set_methods;

// String method handlers
pub mod string_methods;

// Content method handlers
pub mod content_methods;

// DateTime method handlers
pub mod datetime_methods;

// Instant method handlers
pub mod instant_methods;

// Matrix method handlers
pub mod matrix_methods;

// Iterator method handlers
pub mod iterator_methods;

// Typed array (Vec<int>, Vec<number>, Vec<bool>) method handlers
pub mod typed_array_methods;

// Concurrency primitive (Mutex<T>, Atomic<T>, Lazy<T>) method handlers
pub mod concurrency_methods;

// Channel (MPSC sender/receiver) method handlers
pub mod channel_methods;

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;
impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_objects(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            NewArray => self.op_new_array(instruction)?,
            NewTypedArray => self.op_new_typed_array(instruction)?,
            NewMatrix => self.op_new_matrix(instruction)?,
            NewObject => self.op_new_object(instruction)?,
            GetProp => self.op_get_prop(ctx)?,
            SetProp => self.op_set_prop()?,
            SetLocalIndex => self.op_set_local_index(instruction)?,
            SetModuleBindingIndex => self.op_set_module_binding_index(instruction)?,
            Length => self.op_length()?,
            ArrayPush => self.op_array_push()?,
            ArrayPushLocal => self.op_array_push_local(instruction)?,
            ArrayPop => self.op_array_pop()?,
            MakeClosure => self.op_make_closure(instruction)?,
            MergeObject => self.op_merge_object()?,
            NewTypedObject => self.op_new_typed_object(instruction)?,
            TypedMergeObject => self.op_typed_merge_object(instruction)?,
            WrapTypeAnnotation => self.op_wrap_type_annotation(instruction)?,
            SliceAccess => self.op_slice_access()?,
            MakeRange => self.op_make_range()?,
            _ => unreachable!(
                "exec_objects called with non-object opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Wrap a value with a type annotation
    fn op_wrap_type_annotation(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use crate::bytecode::Operand;

        // Get type name from string pool
        let type_name_idx = match &instruction.operand {
            Some(Operand::Property(idx)) => *idx,
            _ => {
                return Err(VMError::RuntimeError(
                    "WrapTypeAnnotation requires Property operand".to_string(),
                ));
            }
        };

        let type_name = self
            .program
            .strings
            .get(type_name_idx as usize)
            .ok_or_else(|| {
                VMError::RuntimeError(format!("Invalid string index: {}", type_name_idx))
            })?
            .clone();

        let value_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);

        self.push_vw(ValueWord::from_type_annotated_value(type_name, value_nb))?;

        Ok(())
    }

    // op_new_typed_object moved to object_creation.rs

    // op_typed_merge_object moved to object_operations.rs

    /// Call method on a value (series.mean(), etc.)
    ///
    /// Supports two calling conventions:
    /// 1. **Typed dispatch** (new): `CallMethod` with `TypedMethodCall` operand encodes
    ///    `MethodId`, arg count, and string fallback in the instruction itself.
    ///    Stack: `[receiver, arg1, ..., argN]`
    /// 2. **Legacy dispatch**: `CallMethod` with no operand reads method name and
    ///    arg count from the stack (backward compatibility with old bytecode).
    ///    Stack: `[receiver, arg1, ..., argN, method_name, arg_count]`
    #[inline]
    pub fn op_call_method(
        &mut self,
        instruction: &crate::bytecode::Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        use crate::bytecode::Operand;
        use shape_value::MethodId;

        // Extract method_id, arg_count, and method_name from instruction or stack
        let (method_id, arg_count, method_name);
        match &instruction.operand {
            Some(Operand::TypedMethodCall {
                method_id: mid,
                arg_count: ac,
                string_id: sid,
            }) => {
                method_id = MethodId(*mid);
                arg_count = *ac as usize;
                // Resolve string lazily only when needed (dynamic fallback / error messages)
                method_name = self
                    .program
                    .strings
                    .get(*sid as usize)
                    .cloned()
                    .unwrap_or_default();
            }
            _ => {
                // Legacy stack-based calling convention
                let arg_count_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
                arg_count = arg_count_nb.as_number_coerce().ok_or_else(|| {
                    VMError::RuntimeError("Expected number for arg count".to_string())
                })? as usize;
                let method_name_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
                method_name = match method_name_nb.as_str() {
                    Some(s) => s.to_string(),
                    std::option::Option::None => {
                        return Err(VMError::TypeError {
                            expected: "string",
                            got: method_name_nb.type_name(),
                        });
                    }
                };
                method_id = MethodId::from_name(&method_name);
            }
        }

        // Pop arguments in reverse order (they were pushed in order on stack)
        let mut args_nb = Vec::with_capacity(arg_count + 1); // +1 for receiver
        for _ in 0..arg_count {
            args_nb.push(ValueWord::from_raw_bits(self.pop_raw_u64()?));
        }
        args_nb.reverse();

        // Pop receiver (the object/series/array the method is called on)
        let receiver_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
        let receiver_nb = if receiver_nb.is_ref() {
            self.resolve_ref_value(&receiver_nb).unwrap_or(receiver_nb)
        } else {
            receiver_nb
        };

        // Prepend receiver to args (handler functions expect receiver as first arg)
        args_nb.insert(0, receiver_nb.clone());

        // Universal intrinsic methods available on all values.
        if method_id == MethodId::TYPE {
            if arg_count != 0 {
                return Err(VMError::ArityMismatch {
                    function: "type".to_string(),
                    expected: 0,
                    got: arg_count,
                });
            }
            // Reuse existing type resolution path (typed-object schema lookup included).
            self.push_vw(receiver_nb)?;
            let result = self.builtin_type_of(vec![])?;
            self.push_vw(result)?;
            return Ok(());
        }

        // v2 typed array method dispatch.
        if let Some(view) =
            crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(&receiver_nb)
        {
            if self.dispatch_v2_typed_array_method(&method_name, &view, &args_nb)? {
                return Ok(());
            }
        }

        // IC fast path: if the method dispatch site is monomorphic, skip PHF lookup.
        {
            let ic_ip = self.ip;
            let mid = method_id.0 as u32;
            if let Some(heap_kind) = receiver_nb.heap_kind() {
                if let Some(hit) =
                    crate::executor::ic_fast_paths::method_ic_check(self, ic_ip, heap_kind, mid)
                {
                    let result = (hit.handler)(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    return Ok(());
                }
            }
        }

        // ValueWord tag/HeapKind dispatch — no to_vmvalue() needed
        use shape_value::NanTag;
        use shape_value::heap_value::HeapKind;

        match receiver_nb.tag() {
            NanTag::I48 | NanTag::F64 => {
                let result = self.handle_number_method(&method_name, args_nb)?;
                self.push_vw(result)?;
            }
            NanTag::Bool => {
                let result = self.handle_bool_method(&method_name, args_nb)?;
                self.push_vw(result)?;
            }
            NanTag::Heap => match receiver_nb.heap_kind().unwrap() {
                HeapKind::Array => {
                    let handler = method_registry::ARRAY_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Array type",
                                method_name
                            ))
                        })?;
                    // Record IC with resolved handler pointer.
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Array as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::String => {
                    let result = self.handle_string_method(&method_name, args_nb)?;
                    self.push_vw(result)?;
                }
                HeapKind::Decimal => {
                    let result = self.handle_number_method(&method_name, args_nb)?;
                    self.push_vw(result)?;
                }
                HeapKind::DataTable | HeapKind::TypedTable => {
                    let handler = method_registry::DATATABLE_METHODS
                            .get(method_name.as_str())
                            .ok_or_else(|| {
                                if method_registry::INDEXED_TABLE_METHODS.contains_key(method_name.as_str()) {
                                    VMError::RuntimeError(format!(
                                        "{}() requires an indexed table. Use table.index_by(column) first.",
                                        method_name
                                    ))
                                } else {
                                    VMError::RuntimeError(format!(
                                        "Unknown method '{}' on DataTable type", method_name
                                    ))
                                }
                            })?;
                    let rk = receiver_nb.heap_kind().unwrap() as u8;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        rk,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::IndexedTable => {
                    if let Some(handler) =
                        method_registry::INDEXED_TABLE_METHODS.get(method_name.as_str())
                    {
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else if let Some(handler) =
                        method_registry::DATATABLE_METHODS.get(method_name.as_str())
                    {
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on IndexedTable type",
                            method_name
                        )));
                    }
                }
                HeapKind::ColumnRef => {
                    let handler = method_registry::COLUMN_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Column type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::ColumnRef as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::HashMap => {
                    let handler = method_registry::HASHMAP_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on HashMap type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::HashMap as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Set => {
                    let handler = method_registry::SET_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Set type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Set as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Deque => {
                    let handler = method_registry::DEQUE_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Deque type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Deque as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::PriorityQueue => {
                    let handler = method_registry::PRIORITY_QUEUE_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on PriorityQueue type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::PriorityQueue as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::FloatArray => {
                    if let Some(handler) =
                        method_registry::FLOAT_ARRAY_METHODS.get(method_name.as_str())
                    {
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else if let Some(handler) =
                        method_registry::ARRAY_METHODS.get(method_name.as_str())
                    {
                        // Fallback: promote to generic array for standard array methods
                        args_nb[0] =
                            ValueWord::from_array(args_nb[0].as_any_array().unwrap().to_generic());
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on Vec<number> type",
                            method_name
                        )));
                    }
                }
                HeapKind::FloatArraySlice => {
                    // Materialize the slice as a FloatArray, then dispatch
                    if let Some(HeapValue::FloatArraySlice { parent, offset, len }) = args_nb[0].as_heap_ref() {
                        let off = *offset as usize;
                        let slice_len = *len as usize;
                        let data = &parent.data[off..off + slice_len];
                        let mut aligned = shape_value::aligned_vec::AlignedVec::with_capacity(slice_len);
                        for &v in data {
                            aligned.push(v);
                        }
                        args_nb[0] = ValueWord::from_float_array(Arc::new(aligned.into()));
                    }
                    if let Some(handler) =
                        method_registry::FLOAT_ARRAY_METHODS.get(method_name.as_str())
                    {
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else if let Some(handler) =
                        method_registry::ARRAY_METHODS.get(method_name.as_str())
                    {
                        args_nb[0] =
                            ValueWord::from_array(args_nb[0].as_any_array().unwrap().to_generic());
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on Vec<number> type",
                            method_name
                        )));
                    }
                }
                HeapKind::IntArray => {
                    if let Some(handler) =
                        method_registry::INT_ARRAY_METHODS.get(method_name.as_str())
                    {
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else if let Some(handler) =
                        method_registry::ARRAY_METHODS.get(method_name.as_str())
                    {
                        args_nb[0] =
                            ValueWord::from_array(args_nb[0].as_any_array().unwrap().to_generic());
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on Vec<int> type",
                            method_name
                        )));
                    }
                }
                HeapKind::BoolArray => {
                    if let Some(handler) =
                        method_registry::BOOL_ARRAY_METHODS.get(method_name.as_str())
                    {
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else if let Some(handler) =
                        method_registry::ARRAY_METHODS.get(method_name.as_str())
                    {
                        args_nb[0] =
                            ValueWord::from_array(args_nb[0].as_any_array().unwrap().to_generic());
                        let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on Vec<bool> type",
                            method_name
                        )));
                    }
                }
                HeapKind::TypedObject => {
                    let result = self.handle_typed_object_method(&method_name, args_nb)?;
                    self.push_vw(result)?;
                }
                HeapKind::Content => {
                    let result = self.handle_content_method(&method_name, args_nb)?;
                    self.push_vw(result)?;
                }
                HeapKind::Time => {
                    let handler = method_registry::DATETIME_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on DateTime type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Time as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Instant => {
                    let handler = method_registry::INSTANT_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Instant type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Instant as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Iterator => {
                    let handler = method_registry::ITERATOR_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Iterator type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Iterator as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Range => {
                    if method_name == "iter" {
                        iterator_methods::handle_range_iter(self, args_nb, ctx)?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on Range type",
                            method_name
                        )));
                    }
                }
                HeapKind::Matrix => {
                    let handler = method_registry::MATRIX_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Matrix type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Matrix as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Mutex => {
                    let handler = method_registry::MUTEX_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Mutex type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Mutex as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Atomic => {
                    let handler = method_registry::ATOMIC_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Atomic type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Atomic as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Lazy => {
                    let handler = method_registry::LAZY_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Lazy type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Lazy as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Channel => {
                    let handler = method_registry::CHANNEL_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Channel type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::Channel as u8,
                        method_id.0 as u32,
                        *handler,
                    );
                    let result = handler(self, args_nb, ctx)?;
                    self.push_vw(result)?;
                }
                HeapKind::Char => {
                    let result = self.handle_char_method(&method_name, args_nb)?;
                    self.push_vw(result)?;
                }
                _ => {
                    return Err(VMError::RuntimeError(format!(
                        "Method '{}' not available on type '{}'",
                        method_name,
                        receiver_nb.type_name()
                    )));
                }
            },
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Method '{}' not available on type '{}'",
                    method_name,
                    receiver_nb.type_name()
                )));
            }
        }

        Ok(())
    }

    /// Handle char methods (is_alphabetic, to_uppercase, etc.)
    /// Dispatch a method call where the receiver is a v2 typed array.
    /// Returns Ok(true) if handled, Ok(false) to fall through to legacy path.
    fn dispatch_v2_typed_array_method(
        &mut self,
        method: &str,
        view: &crate::executor::v2_handlers::v2_array_detect::V2TypedArrayView,
        args: &[ValueWord],
    ) -> Result<bool, VMError> {
        use crate::executor::v2_handlers::v2_array_detect as v2;
        match method {
            "len" | "length" => {
                self.push_vw(ValueWord::from_i64(view.len as i64))?;
                Ok(true)
            }
            "first" => {
                let val = if view.len == 0 {
                    ValueWord::none()
                } else {
                    v2::read_element(view, 0).unwrap_or_else(ValueWord::none)
                };
                self.push_vw(val)?;
                Ok(true)
            }
            "last" => {
                let val = if view.len == 0 {
                    ValueWord::none()
                } else {
                    v2::read_element(view, view.len - 1).unwrap_or_else(ValueWord::none)
                };
                self.push_vw(val)?;
                Ok(true)
            }
            "is_empty" | "isEmpty" => {
                self.push_vw(ValueWord::from_bool(view.len == 0))?;
                Ok(true)
            }
            "sum" => {
                if let Some(val) = v2::sum_elements(view) {
                    self.push_vw(val)?;
                    Ok(true)
                } else {
                    Err(VMError::RuntimeError(
                        "sum() not supported on bool typed array".to_string(),
                    ))
                }
            }
            "clone" => {
                let new_ptr = v2::clone_array(view);
                self.push_vw(ValueWord::from_native_ptr(new_ptr as usize))?;
                Ok(true)
            }
            "push" => {
                if args.len() != 2 {
                    return Err(VMError::ArityMismatch {
                        function: "push".to_string(),
                        expected: 1,
                        got: args.len().saturating_sub(1),
                    });
                }
                v2::push_element(view, &args[1])
                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                self.push_vw(ValueWord::none())?;
                Ok(true)
            }
            "pop" => {
                let val = v2::pop_element(view).unwrap_or_else(ValueWord::none);
                self.push_vw(val)?;
                Ok(true)
            }
            "map" | "filter" | "reduce" | "fold" | "forEach" | "for_each" | "find"
            | "findIndex" | "find_index" | "some" | "every" | "any" | "all" => {
                let mut elems: Vec<ValueWord> = Vec::with_capacity(view.len as usize);
                for i in 0..view.len {
                    elems.push(v2::read_element(view, i).unwrap_or_else(ValueWord::none));
                }
                let legacy = ValueWord::from_array(std::sync::Arc::new(elems));
                let mut new_args: Vec<ValueWord> = args.to_vec();
                new_args[0] = legacy;
                let handler = method_registry::ARRAY_METHODS.get(method).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Unknown method '{}' on v2 typed array",
                        method
                    ))
                })?;
                let result = handler(self, new_args, None)?;
                self.push_vw(result)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn handle_char_method(
        &mut self,
        method: &str,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        let c = args[0].as_char().ok_or_else(|| VMError::TypeError {
            expected: "char",
            got: args[0].type_name(),
        })?;
        let result = match method {
            "is_alphabetic" | "isAlphabetic" => ValueWord::from_bool(c.is_alphabetic()),
            "is_numeric" | "isNumeric" => ValueWord::from_bool(c.is_numeric()),
            "is_alphanumeric" | "isAlphanumeric" => ValueWord::from_bool(c.is_alphanumeric()),
            "is_whitespace" | "isWhitespace" => ValueWord::from_bool(c.is_whitespace()),
            "is_uppercase" | "isUppercase" => ValueWord::from_bool(c.is_uppercase()),
            "is_lowercase" | "isLowercase" => ValueWord::from_bool(c.is_lowercase()),
            "is_ascii" | "isAscii" => ValueWord::from_bool(c.is_ascii()),
            "to_uppercase" | "toUppercase" => {
                let upper: String = c.to_uppercase().collect();
                if upper.len() == 1 {
                    ValueWord::from_char(upper.chars().next().unwrap())
                } else {
                    ValueWord::from_string(std::sync::Arc::new(upper))
                }
            }
            "to_lowercase" | "toLowercase" => {
                let lower: String = c.to_lowercase().collect();
                if lower.len() == 1 {
                    ValueWord::from_char(lower.chars().next().unwrap())
                } else {
                    ValueWord::from_string(std::sync::Arc::new(lower))
                }
            }
            "to_string" | "toString" => ValueWord::from_string(std::sync::Arc::new(c.to_string())),
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Unknown method '{}' on char type",
                    method
                )));
            }
        };
        Ok(result)
    }

    /// Handle TypedObject methods via direct schema-based access.
    /// No HashMap conversion — reads/writes slots directly via schema field indices.
    fn handle_typed_object_method(
        &mut self,
        method: &str,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use crate::executor::objects::object_creation::read_slot_nb;
        use shape_value::heap_value::HeapValue;

        // Extract TypedObject fields via HeapValue (no ValueWord materialization)
        let (schema_id, slots, heap_mask) = match args[0].as_heap_ref() {
            Some(HeapValue::TypedObject {
                schema_id,
                slots,
                heap_mask,
            }) => (*schema_id as u32, slots.clone(), *heap_mask),
            _ => {
                return Err(VMError::TypeError {
                    expected: "TypedObject",
                    got: args[0].type_name(),
                });
            }
        };

        // Extension intrinsic dispatch: check __type field
        let schema = self.lookup_schema(schema_id);
        let mut type_name_val = schema.map(|s| s.name.clone());
        if let Some(s) = schema
            && let Some(f) = s.get_field("__type")
            && let Some(explicit_type) =
                read_slot_nb(&slots, f.index as usize, heap_mask, Some(&f.field_type))
                    .as_str()
                    .map(|s| s.to_string())
        {
            type_name_val = Some(explicit_type);
        }

        if let Some(type_name) = &type_name_val
            && let Some(type_methods) = self.extension_methods.get(type_name.as_str())
            && let Some(intrinsic_fn) = type_methods.get(method)
        {
            let intrinsic_fn = intrinsic_fn.clone();
            let call_args_nb: Vec<ValueWord> = args[1..].to_vec();
            let result_nb = self.invoke_module_fn(&intrinsic_fn, &call_args_nb)?;
            return Ok(result_nb);
        }

        // UFCS method dispatch for impl methods (Type::method) and extend methods (Type.method)
        if let Some(type_name) = &type_name_val {
            let ufcs_name = format!("{}::{}", type_name, method);
            let extend_name = format!("{}.{}", type_name, method);
            if let Some(&func_id) = self
                .function_name_index
                .get(&ufcs_name)
                .or_else(|| self.function_name_index.get(&extend_name))
            {
                let func_nb = ValueWord::from_function(func_id);
                let result_nb = self.call_value_immediate_nb(&func_nb, &args, None)?;
                return Ok(result_nb);
            }

            // BUG-TR2 fix: Also check trait_method_symbols for named impls.
            // Named impl methods have function names like "Trait::Type::ImplName::method"
            // which aren't found by the simple "Type::method" lookup above.
            if let Some(impl_func_name) = self
                .program
                .find_default_trait_impl_for_type_method(type_name, method)
            {
                if let Some(&func_id) = self.function_name_index.get(impl_func_name) {
                    let func_nb = ValueWord::from_function(func_id);
                    let result_nb = self.call_value_immediate_nb(&func_nb, &args, None)?;
                    return Ok(result_nb);
                }
            }
        }

        // Module namespace fallback: if the TypedObject is a module and has a field
        // matching the method name, extract and call it as a function value.
        if let Some(schema) = self.lookup_schema(schema_id) {
            if let Some(field) = schema.get_field(method) {
                let field_nb = read_slot_nb(
                    &slots,
                    field.index as usize,
                    heap_mask,
                    Some(&field.field_type),
                );
                // If field is callable (function or closure) or another module (TypedObject),
                // handle accordingly.
                if field_nb.is_function()
                    || matches!(field_nb.as_heap_ref(), Some(HeapValue::Closure { .. }))
                {
                    let call_args_nb: Vec<ValueWord> = args[1..].to_vec();
                    let result_nb = self.call_value_immediate_nb(&field_nb, &call_args_nb, None)?;
                    return Ok(result_nb);
                }
            }
        }

        Err(VMError::RuntimeError(format!(
            "Unknown method '{}' on TypedObject",
            method
        )))
    }

    /// Handle Number/Int methods (toFixed, toString, etc.)
    fn handle_number_method(
        &mut self,
        method: &str,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use std::sync::Arc;

        let first = args.first().ok_or_else(|| VMError::TypeError {
            expected: "number or int",
            got: "other",
        })?;

        // Determine is_int and extract the f64 value directly from ValueWord
        let is_int = first.is_i64();
        let number = if let Some(i) = first.as_i64() {
            i as f64
        } else if let Some(n) = first.as_f64() {
            n
        } else {
            // Decimal fallback — check heap for Decimal variant
            match first.as_heap_ref() {
                Some(shape_value::HeapValue::Decimal(d)) => {
                    use rust_decimal::prelude::ToPrimitive;
                    d.to_f64().unwrap_or(f64::NAN)
                }
                _ => {
                    return Err(VMError::TypeError {
                        expected: "number or int",
                        got: first.type_name(),
                    });
                }
            }
        };

        let result: ValueWord = match method {
            "toFixed" | "to_fixed" => {
                let decimals = if args.len() > 1 {
                    args[1].as_number_coerce().ok_or_else(|| {
                        VMError::RuntimeError("Expected number for decimals".to_string())
                    })? as i32
                } else {
                    2
                };
                ValueWord::from_string(Arc::new(format!(
                    "{:.prec$}",
                    number,
                    prec = decimals as usize
                )))
            }
            "toString" | "to_string" => {
                if is_int {
                    ValueWord::from_string(Arc::new((number as i64).to_string()))
                } else {
                    ValueWord::from_string(Arc::new(number.to_string()))
                }
            }
            "toInt" | "to_int" => ValueWord::from_i64(number as i64),
            "toNumber" | "to_number" => ValueWord::from_f64(number),
            "floor" => {
                if is_int {
                    ValueWord::from_i64(number as i64)
                } else {
                    ValueWord::from_f64(number.floor())
                }
            }
            "ceil" => {
                if is_int {
                    ValueWord::from_i64(number as i64)
                } else {
                    ValueWord::from_f64(number.ceil())
                }
            }
            "round" => {
                if is_int {
                    ValueWord::from_i64(number as i64)
                } else {
                    ValueWord::from_f64(number.round())
                }
            }
            "abs" => {
                if is_int {
                    ValueWord::from_i64((number as i64).abs())
                } else {
                    ValueWord::from_f64(number.abs())
                }
            }
            "sign" => {
                if is_int {
                    let i = number as i64;
                    ValueWord::from_i64(if i > 0 {
                        1
                    } else if i < 0 {
                        -1
                    } else {
                        0
                    })
                } else {
                    ValueWord::from_f64(if number > 0.0 {
                        1.0
                    } else if number < 0.0 {
                        -1.0
                    } else {
                        0.0
                    })
                }
            }
            "clamp" => {
                let min_val = args
                    .get(1)
                    .and_then(|nb| nb.as_number_coerce())
                    .ok_or_else(|| VMError::InvalidArgument {
                        function: "clamp".to_string(),
                        message: "requires a min argument".to_string(),
                    })?;
                let max_val = args
                    .get(2)
                    .and_then(|nb| nb.as_number_coerce())
                    .ok_or_else(|| VMError::InvalidArgument {
                        function: "clamp".to_string(),
                        message: "requires a max argument".to_string(),
                    })?;
                if is_int {
                    let i = number as i64;
                    let lo = min_val as i64;
                    let hi = max_val as i64;
                    ValueWord::from_i64(i.max(lo).min(hi))
                } else {
                    ValueWord::from_f64(number.max(min_val).min(max_val))
                }
            }
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Unknown method '{}' on Number type",
                    method
                )));
            }
        };

        Ok(result)
    }

    /// Handle String methods (toUpperCase, toLowerCase, split, contains, replace, substring, etc.)
    fn handle_string_method(
        &mut self,
        method: &str,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use std::sync::Arc;

        // Delegate to string_methods module for extended methods
        match method {
            "iter" => return iterator_methods::handle_string_iter(self, args, None),
            "split" => return string_methods::handle_split(self, args, None),
            "contains" => return string_methods::handle_contains(self, args, None),
            "replace" => return string_methods::handle_replace(self, args, None),
            "substring" => return string_methods::handle_substring(self, args, None),
            "join" => return string_methods::handle_join(self, args, None),
            _ => {}
        }

        let string = args
            .first()
            .and_then(|nb| nb.as_str())
            .ok_or_else(|| VMError::TypeError {
                expected: "string",
                got: "other",
            })?
            .to_string();

        let result: ValueWord = match method {
            "toUpperCase" | "to_upper_case" => {
                ValueWord::from_string(Arc::new(string.to_uppercase()))
            }
            "toLowerCase" | "to_lower_case" => {
                ValueWord::from_string(Arc::new(string.to_lowercase()))
            }
            "trim" => ValueWord::from_string(Arc::new(string.trim().to_string())),
            "trimStart" | "trim_start" => {
                ValueWord::from_string(Arc::new(string.trim_start().to_string()))
            }
            "trimEnd" | "trim_end" => {
                ValueWord::from_string(Arc::new(string.trim_end().to_string()))
            }
            "length" | "len" => ValueWord::from_i64(string.len() as i64),
            "toString" | "to_string" => ValueWord::from_string(Arc::new(string)),
            "startsWith" | "starts_with" => {
                let prefix = args.get(1).and_then(|nb| nb.as_str()).ok_or_else(|| {
                    VMError::InvalidArgument {
                        function: "startsWith".to_string(),
                        message: "requires a string argument".to_string(),
                    }
                })?;
                ValueWord::from_bool(string.starts_with(prefix))
            }
            "endsWith" | "ends_with" => {
                let suffix = args.get(1).and_then(|nb| nb.as_str()).ok_or_else(|| {
                    VMError::InvalidArgument {
                        function: "endsWith".to_string(),
                        message: "requires a string argument".to_string(),
                    }
                })?;
                ValueWord::from_bool(string.ends_with(suffix))
            }
            "padStart" | "pad_start" => {
                let target_len = args
                    .get(1)
                    .and_then(|nb| nb.as_number_coerce())
                    .ok_or_else(|| VMError::InvalidArgument {
                        function: "padStart".to_string(),
                        message: "requires a length argument".to_string(),
                    })? as usize;
                let fill = args
                    .get(2)
                    .and_then(|nb| nb.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| " ".to_string());
                let char_count = string.chars().count();
                if char_count >= target_len {
                    ValueWord::from_string(Arc::new(string))
                } else {
                    let pad_needed = target_len - char_count;
                    let fill_chars: Vec<char> = fill.chars().collect();
                    let mut padding = String::with_capacity(pad_needed);
                    for i in 0..pad_needed {
                        padding.push(fill_chars[i % fill_chars.len()]);
                    }
                    padding.push_str(&string);
                    ValueWord::from_string(Arc::new(padding))
                }
            }
            "padEnd" | "pad_end" => {
                let target_len = args
                    .get(1)
                    .and_then(|nb| nb.as_number_coerce())
                    .ok_or_else(|| VMError::InvalidArgument {
                        function: "padEnd".to_string(),
                        message: "requires a length argument".to_string(),
                    })? as usize;
                let fill = args
                    .get(2)
                    .and_then(|nb| nb.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| " ".to_string());
                let char_count = string.chars().count();
                if char_count >= target_len {
                    ValueWord::from_string(Arc::new(string))
                } else {
                    let pad_needed = target_len - char_count;
                    let fill_chars: Vec<char> = fill.chars().collect();
                    let mut result = string;
                    for i in 0..pad_needed {
                        result.push(fill_chars[i % fill_chars.len()]);
                    }
                    ValueWord::from_string(Arc::new(result))
                }
            }
            "repeat" => {
                let count = args
                    .get(1)
                    .and_then(|nb| nb.as_number_coerce())
                    .ok_or_else(|| VMError::InvalidArgument {
                        function: "repeat".to_string(),
                        message: "requires a count argument".to_string(),
                    })? as usize;
                ValueWord::from_string(Arc::new(string.repeat(count)))
            }
            "charAt" | "char_at" => {
                let index = args
                    .get(1)
                    .and_then(|nb| nb.as_number_coerce())
                    .ok_or_else(|| VMError::InvalidArgument {
                        function: "charAt".to_string(),
                        message: "requires an index argument".to_string(),
                    })? as usize;
                let ch = string.chars().nth(index);
                match ch {
                    Some(c) => ValueWord::from_char(c),
                    None => ValueWord::none(),
                }
            }
            "reverse" => {
                let reversed: String = string.chars().rev().collect();
                ValueWord::from_string(Arc::new(reversed))
            }
            "indexOf" | "index_of" => {
                let needle = args.get(1).and_then(|nb| nb.as_str()).ok_or_else(|| {
                    VMError::InvalidArgument {
                        function: "indexOf".to_string(),
                        message: "requires a string argument".to_string(),
                    }
                })?;
                match string.find(needle) {
                    Some(pos) => {
                        // Return char index, not byte index
                        let char_idx = string[..pos].chars().count() as i64;
                        ValueWord::from_i64(char_idx)
                    }
                    None => ValueWord::from_i64(-1),
                }
            }
            "isDigit" | "is_digit" => ValueWord::from_bool(
                !string.is_empty() && string.chars().all(|c| c.is_ascii_digit()),
            ),
            "isAlpha" | "is_alpha" => ValueWord::from_bool(
                !string.is_empty() && string.chars().all(|c| c.is_ascii_alphabetic()),
            ),
            "isAscii" | "is_ascii" => ValueWord::from_bool(string.is_ascii()),
            "normalize" => {
                use unicode_normalization::UnicodeNormalization;
                let form = args.get(1).and_then(|nb| nb.as_str()).ok_or_else(|| {
                    VMError::InvalidArgument {
                        function: "normalize".to_string(),
                        message:
                            "requires a form argument (\"NFC\", \"NFD\", \"NFKC\", or \"NFKD\")"
                                .to_string(),
                    }
                })?;
                let normalized: String = match form {
                    "NFC" => string.nfc().collect(),
                    "NFD" => string.nfd().collect(),
                    "NFKC" => string.nfkc().collect(),
                    "NFKD" => string.nfkd().collect(),
                    _ => {
                        return Err(VMError::InvalidArgument {
                            function: "normalize".to_string(),
                            message: format!(
                                "unknown normalization form '{}', expected NFC/NFD/NFKC/NFKD",
                                form
                            ),
                        });
                    }
                };
                ValueWord::from_string(Arc::new(normalized))
            }
            "graphemes" => {
                use unicode_segmentation::UnicodeSegmentation;
                let clusters: Vec<ValueWord> = string
                    .graphemes(true)
                    .map(|g| ValueWord::from_string(Arc::new(g.to_string())))
                    .collect();
                ValueWord::from_array(Arc::new(clusters))
            }
            "graphemeLen" | "grapheme_len" => {
                use unicode_segmentation::UnicodeSegmentation;
                let count = string.graphemes(true).count();
                ValueWord::from_i64(count as i64)
            }
            "toInt" | "to_int" => {
                let trimmed = string.trim();
                let parsed: i64 = trimmed.parse().map_err(|_| {
                    VMError::RuntimeError(format!("Cannot convert '{}' to int", string))
                })?;
                ValueWord::from_i64(parsed)
            }
            "toNumber" | "to_number" | "toFloat" | "to_float" => {
                let trimmed = string.trim();
                let parsed: f64 = trimmed.parse().map_err(|_| {
                    VMError::RuntimeError(format!("Cannot convert '{}' to number", string))
                })?;
                ValueWord::from_f64(parsed)
            }
            "codePointAt" | "code_point_at" => {
                let index = args
                    .get(1)
                    .and_then(|nb| nb.as_number_coerce())
                    .ok_or_else(|| VMError::InvalidArgument {
                        function: "codePointAt".to_string(),
                        message: "requires an index argument".to_string(),
                    })? as usize;
                match string.chars().nth(index) {
                    Some(c) => ValueWord::from_i64(c as u32 as i64),
                    None => ValueWord::from_i64(-1),
                }
            }
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Unknown method '{}' on String type",
                    method
                )));
            }
        };

        Ok(result)
    }

    /// Handle Boolean methods
    fn handle_bool_method(
        &mut self,
        method: &str,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use std::sync::Arc;

        let bool_val =
            args.first()
                .and_then(|nb| nb.as_bool())
                .ok_or_else(|| VMError::TypeError {
                    expected: "bool",
                    got: "other",
                })?;

        let result: ValueWord = match method {
            "toString" | "to_string" => ValueWord::from_string(Arc::new(bool_val.to_string())),
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Unknown method '{}' on Boolean type",
                    method
                )));
            }
        };

        Ok(result)
    }

    // op_new_array and op_new_object moved to object_creation.rs

    // op_merge_object moved to object_operations.rs
    // op_set_prop and op_length moved to property_access.rs
    // op_array_push and op_array_pop moved to array_operations.rs

    pub(in crate::executor) fn op_make_range(&mut self) -> Result<(), VMError> {
        let inclusive_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
        let end_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
        let start_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);

        let inclusive = inclusive_nb.as_bool().unwrap_or(false);
        let start_opt = if start_nb.is_none() {
            None
        } else {
            Some(Box::new(start_nb))
        };
        let end_opt = if end_nb.is_none() {
            None
        } else {
            Some(Box::new(end_nb))
        };

        self.push_vw(ValueWord::from_heap_value(
            shape_value::heap_value::HeapValue::Range {
                start: start_opt,
                end: end_opt,
                inclusive,
            },
        ))
    }

    // op_slice_access moved to array_operations.rs

    // op_get_prop moved to property_access.rs
    // value_to_bytes moved to object_creation.rs
}
