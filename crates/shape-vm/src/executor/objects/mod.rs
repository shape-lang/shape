//! Object and array operations for the VM executor
//!
//! Handles: NewArray, NewObject, GetProp, SetProp, Length, ArrayPush, ArrayPop, MakeClosure, MergeObject, NewTypedObject, TypedMergeObject, CallMethod

// PHF method registry
pub mod method_registry;
// Raw u64 extraction helpers (v2 — no ValueWord)
pub mod raw_helpers;

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

// Number method handlers (V2 native)
pub mod number_methods;

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

// Concatenation opcodes (StringConcat, ArrayConcat) — dedicated v2 replacements
// for the generic Add overload on built-in heap types.
pub mod concat;

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use smallvec::SmallVec;
use std::sync::Arc;

/// Reinterpret a `&u64` as `&ValueWord` for read-only inspection.
///
/// # Safety
/// `ValueWord` is `#[repr(transparent)]` over `u64`, so the layouts are identical.
/// This does NOT create an owning ValueWord — no Drop will run through this reference.
#[inline(always)]
fn as_vw_ref(bits: &u64) -> &ValueWord {
    // SAFETY: ValueWord is #[repr(transparent)] over u64
    unsafe { &*(bits as *const u64 as *const ValueWord) }
}

/// Reinterpret a `&[u64]` as `&[ValueWord]` for read-only inspection of a slice.
#[inline(always)]
fn as_vw_slice(bits: &[u64]) -> &[ValueWord] {
    // SAFETY: ValueWord is #[repr(transparent)] over u64
    unsafe { std::slice::from_raw_parts(bits.as_ptr() as *const ValueWord, bits.len()) }
}
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

    /// Dispatch a method handler on raw u64 args — zero-alloc for <=8 args.
    ///
    /// `raw_args` is a stack-inline `SmallVec<[u64; 8]>` whose elements are
    /// owned raw u64 bits (obtained via `pop_raw_u64` or `into_raw_bits`).
    /// The handler receives `&mut [u64]` as sole owner of the heap values.
    /// After the handler returns, we drop each raw arg to decrement refcounts.
    ///
    /// This enables `as_*_mut()` in handlers: since no duplicate ValueWord
    /// exists during the call, `Arc::get_mut()` succeeds when refcount == 1.
    /// If `Arc::make_mut` reallocates, the handler updates `args[0]` in place,
    /// and we drop the correct (updated) pointer.
    #[inline]
    fn dispatch_method_handler(
        &mut self,
        handler: &method_registry::MethodHandler,
        mut raw_args: SmallVec<[u64; 8]>,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        let result = handler(self, &mut raw_args, ctx)?;
        // Drop args: the handler may have updated pointers (e.g. after Arc::make_mut).
        for bits in raw_args {
            drop(ValueWord::from_raw_bits(bits));
        }
        self.push_raw_u64(result)?;
        Ok(())
    }

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

        // Extract method_id, arg_count, method_name, and receiver_type_tag from instruction or stack
        let (method_id, arg_count, method_name, receiver_type_tag);
        match &instruction.operand {
            Some(Operand::TypedMethodCall {
                method_id: mid,
                arg_count: ac,
                string_id: sid,
                receiver_type_tag: rtt,
            }) => {
                method_id = MethodId(*mid);
                arg_count = *ac as usize;
                receiver_type_tag = *rtt;
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
                receiver_type_tag = 0xFF; // Unknown in legacy path
            }
        }

        // Pop args as raw u64 directly — no ValueWord wrapping on the hot path.
        let mut raw_args: SmallVec<[u64; 8]> = SmallVec::with_capacity(arg_count + 1);
        for _ in 0..arg_count {
            raw_args.push(self.pop_raw_u64()?);
        }
        raw_args.reverse();

        // Pop receiver as raw u64
        let mut receiver_bits = self.pop_raw_u64()?;

        // Resolve refs on receiver (needs temporary ValueWord for ref resolution)
        {
            let receiver_ref = as_vw_ref(&receiver_bits);
            if receiver_ref.is_ref() {
                let receiver_vw = ValueWord::from_raw_bits(receiver_bits);
                let resolved = self.resolve_ref_value(&receiver_vw).unwrap_or(receiver_vw);
                receiver_bits = resolved.into_raw_bits();
            }
        }

        // Universal intrinsic methods available on all values.
        // Check BEFORE moving receiver into args (we need it for push_vw).
        if method_id == MethodId::TYPE {
            if arg_count != 0 {
                return Err(VMError::ArityMismatch {
                    function: "type".to_string(),
                    expected: 0,
                    got: arg_count,
                });
            }
            // Reuse existing type resolution path (typed-object schema lookup included).
            self.push_vw(ValueWord::from_raw_bits(receiver_bits))?;
            let result = self.builtin_type_of(vec![])?;
            self.push_vw(result)?;
            // Drop the popped args to release refcounts
            for bits in raw_args {
                drop(ValueWord::from_raw_bits(bits));
            }
            return Ok(());
        }

        // Save receiver type info before moving it into args.
        use shape_value::NanTag;
        use shape_value::heap_value::HeapKind;
        let receiver_tag = as_vw_ref(&receiver_bits).tag();
        let receiver_heap_kind = as_vw_ref(&receiver_bits).heap_kind();

        // Prepend receiver to args — raw bits, no ValueWord clone.
        // This keeps the Arc refcount at 1, allowing mutating methods
        // (e.g. Set.add, Deque.pushBack) to succeed with Arc::get_mut().
        raw_args.insert(0, receiver_bits);

        // v2 typed array method dispatch.
        // Uses as_vw_ref for read-only inspection of the receiver.
        if let Some(view) =
            crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(as_vw_ref(&raw_args[0]))
        {
            if self.dispatch_v2_typed_array_method(&method_name, &view, &raw_args)? {
                return Ok(());
            }
        }

        // IC fast path: if the method dispatch site is monomorphic, skip PHF lookup.
        {
            let ic_ip = self.ip;
            let mid = method_id.0 as u32;
            if let Some(heap_kind) = receiver_heap_kind {
                if let Some(hit) =
                    crate::executor::ic_fast_paths::method_ic_check(self, ic_ip, heap_kind, mid)
                {
                    self.dispatch_method_handler(&hit.handler, raw_args, ctx)?;
                    return Ok(());
                }
            }
        }

        // Type-tagged fast dispatch: when the compiler resolved the receiver's
        // ConcreteType at compile time, skip the NanTag/HeapKind cascade entirely.
        if receiver_type_tag != 0xFF {
            let phf_result = match receiver_type_tag {
                0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 22 => // F64, I64, int widths, Decimal
                    method_registry::NUMBER_METHODS.get(method_name.as_str()),
                9 => // Bool
                    method_registry::BOOL_METHODS.get(method_name.as_str()),
                10 => // String
                    method_registry::STRING_METHODS.get(method_name.as_str()),
                12 => // Array — still needs HeapKind for FloatArray/IntArray/BoolArray specialization
                    None, // fall through to legacy dispatch
                13 => // HashMap
                    method_registry::HASHMAP_METHODS.get(method_name.as_str()),
                24 => // DateTime
                    method_registry::DATETIME_METHODS.get(method_name.as_str()),
                _ => None, // unknown or complex type — fall through
            };
            if let Some(handler) = phf_result {
                crate::executor::ic_fast_paths::method_ic_record(
                    self, self.ip,
                    receiver_type_tag, method_id.0 as u32, handler,
                );
                self.dispatch_method_handler(handler, raw_args, ctx)?;
                return Ok(());
            }
            // Fall through to NanTag/HeapKind cascade for unresolved types
        }

        // Legacy NanTag/HeapKind dispatch — fallback when receiver_type_tag is 0xFF
        // or for types that need HeapKind specialization (e.g. FloatArray vs IntArray).
        match receiver_tag {
            NanTag::I48 | NanTag::F64 => {
                let handler = method_registry::NUMBER_METHODS
                    .get(method_name.as_str())
                    .ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "Unknown method '{}' on Number type",
                            method_name
                        ))
                    })?;
                self.dispatch_method_handler(handler, raw_args, ctx)?;
            }
            NanTag::Bool => {
                let handler = method_registry::BOOL_METHODS
                    .get(method_name.as_str())
                    .ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "Unknown method '{}' on Boolean type",
                            method_name
                        ))
                    })?;
                self.dispatch_method_handler(handler, raw_args, ctx)?;
            }
            NanTag::Heap => match receiver_heap_kind.unwrap() {
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
                }
                HeapKind::String => {
                    let handler = method_registry::STRING_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on String type",
                                method_name
                            ))
                        })?;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        HeapKind::String as u8,
                        method_id.0 as u32,
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
                }
                HeapKind::Decimal => {
                    let handler = method_registry::NUMBER_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Number type",
                                method_name
                            ))
                        })?;
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                    let rk = receiver_heap_kind.unwrap() as u8;
                    crate::executor::ic_fast_paths::method_ic_record(
                        self,
                        self.ip,
                        rk,
                        method_id.0 as u32,
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
                }
                HeapKind::IndexedTable => {
                    if let Some(handler) =
                        method_registry::INDEXED_TABLE_METHODS.get(method_name.as_str())
                    {
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
                    } else if let Some(handler) =
                        method_registry::DATATABLE_METHODS.get(method_name.as_str())
                    {
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
                }
                HeapKind::FloatArray => {
                    if let Some(handler) =
                        method_registry::FLOAT_ARRAY_METHODS.get(method_name.as_str())
                    {
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
                    } else if let Some(handler) =
                        method_registry::ARRAY_METHODS.get(method_name.as_str())
                    {
                        // Fallback: promote to generic array for standard array methods.
                        // Drop old receiver, replace with promoted generic array.
                        let old = ValueWord::from_raw_bits(raw_args[0]);
                        raw_args[0] =
                            ValueWord::from_array(old.as_any_array().unwrap().to_generic()).into_raw_bits();
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on Vec<number> type",
                            method_name
                        )));
                    }
                }
                HeapKind::FloatArraySlice => {
                    // Materialize the slice as a FloatArray, then dispatch
                    if let Some(HeapValue::FloatArraySlice { parent, offset, len }) = as_vw_ref(&raw_args[0]).as_heap_ref() {
                        let off = *offset as usize;
                        let slice_len = *len as usize;
                        let data = &parent.data[off..off + slice_len];
                        let mut aligned = shape_value::aligned_vec::AlignedVec::with_capacity(slice_len);
                        for &v in data {
                            aligned.push(v);
                        }
                        // Drop old slice, replace with materialized float array
                        drop(ValueWord::from_raw_bits(raw_args[0]));
                        raw_args[0] = ValueWord::from_float_array(Arc::new(aligned.into())).into_raw_bits();
                    }
                    if let Some(handler) =
                        method_registry::FLOAT_ARRAY_METHODS.get(method_name.as_str())
                    {
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
                    } else if let Some(handler) =
                        method_registry::ARRAY_METHODS.get(method_name.as_str())
                    {
                        let old = ValueWord::from_raw_bits(raw_args[0]);
                        raw_args[0] =
                            ValueWord::from_array(old.as_any_array().unwrap().to_generic()).into_raw_bits();
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
                    } else if let Some(handler) =
                        method_registry::ARRAY_METHODS.get(method_name.as_str())
                    {
                        let old = ValueWord::from_raw_bits(raw_args[0]);
                        raw_args[0] =
                            ValueWord::from_array(old.as_any_array().unwrap().to_generic()).into_raw_bits();
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
                    } else if let Some(handler) =
                        method_registry::ARRAY_METHODS.get(method_name.as_str())
                    {
                        let old = ValueWord::from_raw_bits(raw_args[0]);
                        raw_args[0] =
                            ValueWord::from_array(old.as_any_array().unwrap().to_generic()).into_raw_bits();
                        self.dispatch_method_handler(handler, raw_args, ctx)?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on Vec<bool> type",
                            method_name
                        )));
                    }
                }
                HeapKind::TypedObject => {
                    self.handle_typed_object_method_v2(&method_name, &raw_args)?;
                }
                HeapKind::Content => {
                    let handler = method_registry::CONTENT_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Content type. Available: bold, italic, underline, dim, fg, bg, border, max_rows, series, title, x_label, y_label, toString",
                                method_name
                            ))
                        })?;
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
                }
                HeapKind::TimeSpan => {
                    let handler = method_registry::TIMESPAN_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on TimeSpan type",
                                method_name
                            ))
                        })?;
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
                }
                HeapKind::Range => {
                    let handler = method_registry::RANGE_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on Range type",
                                method_name
                            ))
                        })?;
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
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
                        handler,
                    );
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
                }
                HeapKind::Char => {
                    let handler = method_registry::CHAR_METHODS
                        .get(method_name.as_str())
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Unknown method '{}' on char type",
                                method_name
                            ))
                        })?;
                    self.dispatch_method_handler(handler, raw_args, ctx)?;
                }
                _ => {
                    return Err(VMError::RuntimeError(format!(
                        "Method '{}' not available on type '{}'",
                        method_name,
                        as_vw_ref(&raw_args[0]).type_name()
                    )));
                }
            },
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Method '{}' not available on type '{}'",
                    method_name,
                    as_vw_ref(&raw_args[0]).type_name()
                )));
            }
        }

        Ok(())
    }

    /// Dispatch a method call where the receiver is a v2 typed array.
    /// Returns Ok(true) if handled, Ok(false) to fall through to legacy path.
    ///
    /// `raw_args` contains raw u64 bits: `[receiver, arg1, ...]`.
    /// Uses `as_vw_ref` / `as_vw_slice` for read-only ValueWord access.
    fn dispatch_v2_typed_array_method(
        &mut self,
        method: &str,
        view: &crate::executor::v2_handlers::v2_array_detect::V2TypedArrayView,
        raw_args: &SmallVec<[u64; 8]>,
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
            "avg" | "mean" => {
                if let Some(val) = v2::avg_elements(view) {
                    self.push_vw(val)?;
                    Ok(true)
                } else {
                    Err(VMError::RuntimeError(
                        "avg() not supported on bool typed array".to_string(),
                    ))
                }
            }
            "min" => {
                if let Some(val) = v2::min_elements(view) {
                    self.push_vw(val)?;
                    Ok(true)
                } else {
                    Err(VMError::RuntimeError(
                        "min() not supported on bool typed array".to_string(),
                    ))
                }
            }
            "max" => {
                if let Some(val) = v2::max_elements(view) {
                    self.push_vw(val)?;
                    Ok(true)
                } else {
                    Err(VMError::RuntimeError(
                        "max() not supported on bool typed array".to_string(),
                    ))
                }
            }
            "variance" => {
                if let Some(val) = v2::variance_elements(view) {
                    self.push_vw(val)?;
                    Ok(true)
                } else {
                    Err(VMError::RuntimeError(
                        "variance() only supported on float typed arrays".to_string(),
                    ))
                }
            }
            "std" => {
                if let Some(val) = v2::std_elements(view) {
                    self.push_vw(val)?;
                    Ok(true)
                } else {
                    Err(VMError::RuntimeError(
                        "std() only supported on float typed arrays".to_string(),
                    ))
                }
            }
            "dot" => {
                if raw_args.len() < 2 {
                    return Err(VMError::RuntimeError(
                        "dot() requires a Vec<number> argument".to_string(),
                    ));
                }
                let view_b = v2::as_v2_typed_array(as_vw_ref(&raw_args[1]));
                if let Some(vb) = &view_b {
                    if view.len != vb.len {
                        return Err(VMError::RuntimeError(format!(
                            "Vec length mismatch in dot(): {} vs {}",
                            view.len, vb.len
                        )));
                    }
                    if let Some(val) = v2::dot_elements(view, vb) {
                        self.push_vw(val)?;
                        return Ok(true);
                    }
                }
                // Fall through to legacy path
                Ok(false)
            }
            "norm" => {
                if let Some(val) = v2::norm_elements(view) {
                    self.push_vw(val)?;
                    Ok(true)
                } else {
                    Err(VMError::RuntimeError(
                        "norm() only supported on float typed arrays".to_string(),
                    ))
                }
            }
            "count" => {
                if let Some(val) = v2::count_true_elements(view) {
                    self.push_vw(val)?;
                    Ok(true)
                } else {
                    Ok(false) // fall through for non-bool
                }
            }
            "clone" => {
                let new_ptr = v2::clone_array(view);
                self.push_vw(ValueWord::from_native_ptr(new_ptr as usize))?;
                Ok(true)
            }
            "push" => {
                if raw_args.len() != 2 {
                    return Err(VMError::ArityMismatch {
                        function: "push".to_string(),
                        expected: 1,
                        got: raw_args.len().saturating_sub(1),
                    });
                }
                v2::push_element(view, as_vw_ref(&raw_args[1]))
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
                // Higher-order methods: materialize elements and fall back to generic array handler.
                let mut elems: Vec<ValueWord> = Vec::with_capacity(view.len as usize);
                for i in 0..view.len {
                    elems.push(v2::read_element(view, i).unwrap_or_else(ValueWord::none));
                }
                let legacy = ValueWord::from_array(std::sync::Arc::new(elems));
                // Build a new SmallVec with the promoted receiver
                let mut new_args: SmallVec<[u64; 8]> = SmallVec::with_capacity(raw_args.len());
                new_args.push(legacy.into_raw_bits());
                // Clone the remaining args (bump refcounts)
                for bits in &raw_args[1..] {
                    new_args.push(ValueWord::from_raw_bits(*bits).clone().into_raw_bits());
                }
                let handler = method_registry::ARRAY_METHODS.get(method).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Unknown method '{}' on v2 typed array",
                        method
                    ))
                })?;
                self.dispatch_method_handler(handler, new_args, None)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Handle TypedObject methods via direct schema-based access (v2 compatible).
    /// No HashMap conversion — reads/writes slots directly via schema field indices.
    /// Pushes result directly to stack.
    ///
    /// Takes raw u64 args; uses `as_vw_ref` / `as_vw_slice` for read-only access.
    fn handle_typed_object_method_v2(
        &mut self,
        method: &str,
        raw_args: &SmallVec<[u64; 8]>,
    ) -> Result<(), VMError> {
        use crate::executor::objects::object_creation::read_slot_nb;
        use shape_value::heap_value::HeapValue;

        let args_vw = as_vw_slice(raw_args.as_slice());

        // Extract TypedObject fields via HeapValue (no ValueWord materialization)
        let (schema_id, slots, heap_mask) = match args_vw[0].as_heap_ref() {
            Some(HeapValue::TypedObject {
                schema_id,
                slots,
                heap_mask,
            }) => (*schema_id as u32, slots.clone(), *heap_mask),
            _ => {
                return Err(VMError::TypeError {
                    expected: "TypedObject",
                    got: args_vw[0].type_name(),
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
            let call_args_nb: Vec<ValueWord> = args_vw[1..].to_vec();
            let result_nb = self.invoke_module_fn(&intrinsic_fn, &call_args_nb)?;
            self.push_vw(result_nb)?;
            return Ok(());
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
                let result_nb = self.call_value_immediate_nb(&func_nb, args_vw, None)?;
                self.push_vw(result_nb)?;
                return Ok(());
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
                    let result_nb = self.call_value_immediate_nb(&func_nb, args_vw, None)?;
                    self.push_vw(result_nb)?;
                    return Ok(());
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
                    let call_args_nb: Vec<ValueWord> = args_vw[1..].to_vec();
                    let result_nb = self.call_value_immediate_nb(&field_nb, &call_args_nb, None)?;
                    self.push_vw(result_nb)?;
                    return Ok(());
                }
            }
        }

        Err(VMError::RuntimeError(format!(
            "Unknown method '{}' on TypedObject",
            method
        )))
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
