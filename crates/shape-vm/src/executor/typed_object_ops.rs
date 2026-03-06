//! TypedObject operations for the VM
//!
//! This module handles type-specialized operations for TypedObject values,
//! including fast field access using precomputed offsets and timeframe context management.

use crate::bytecode::{Instruction, Operand};
use crate::executor::objects::object_creation::clone_slots_with_update;
use shape_runtime::type_schema::FieldType;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueSlot, ValueWord};

/// Compile-time field type tags for zero-cost field access.
/// Stored in `Operand::TypedField::field_type_tag` so the executor
/// can interpret slot bits without a runtime schema lookup.
pub const FIELD_TAG_F64: u16 = 0;
pub const FIELD_TAG_I64: u16 = 1;
pub const FIELD_TAG_BOOL: u16 = 2;
pub const FIELD_TAG_STRING: u16 = 3;
pub const FIELD_TAG_TIMESTAMP: u16 = 4;
pub const FIELD_TAG_ARRAY: u16 = 5;
pub const FIELD_TAG_OBJECT: u16 = 6;
pub const FIELD_TAG_DECIMAL: u16 = 7;
pub const FIELD_TAG_ANY: u16 = 8;
pub const FIELD_TAG_UNKNOWN: u16 = 255;

/// Encode a FieldType as a compact u16 tag for the operand.
pub fn field_type_to_tag(ft: &FieldType) -> u16 {
    match ft {
        FieldType::F64 => FIELD_TAG_F64,
        FieldType::I64 => FIELD_TAG_I64,
        FieldType::Bool => FIELD_TAG_BOOL,
        FieldType::String => FIELD_TAG_STRING,
        FieldType::Timestamp => FIELD_TAG_TIMESTAMP,
        FieldType::Array(_) => FIELD_TAG_ARRAY,
        FieldType::Object(_) => FIELD_TAG_OBJECT,
        FieldType::Decimal => FIELD_TAG_DECIMAL,
        FieldType::Any => FIELD_TAG_ANY,
        // Width integer types stored as I64 in NaN-boxed slots
        FieldType::I8
        | FieldType::U8
        | FieldType::I16
        | FieldType::U16
        | FieldType::I32
        | FieldType::U32
        | FieldType::U64 => FIELD_TAG_I64,
    }
}

/// Read a ValueWord from a TypedObject slot using the precomputed field type tag.
/// No schema lookup required — the tag was embedded at compile time.
#[inline(always)]
fn read_slot_fast(slot: &ValueSlot, is_heap: bool, field_type_tag: u16) -> ValueWord {
    if is_heap {
        return slot.as_heap_nb();
    }

    match field_type_tag {
        FIELD_TAG_I64 | FIELD_TAG_TIMESTAMP => ValueWord::from_i64(slot.as_i64()),
        FIELD_TAG_BOOL => ValueWord::from_bool(slot.as_bool()),
        FIELD_TAG_DECIMAL => ValueWord::from_decimal(
            rust_decimal::Decimal::from_f64_retain(slot.as_f64()).unwrap_or_default(),
        ),
        FIELD_TAG_F64 => ValueWord::from_f64(slot.as_f64()),
        // Any and non-primitive types: use as_value_word(false) to preserve
        // all inline NanTag variants (Function, ModuleFunction, I48, etc.)
        _ => slot.as_value_word(false),
    }
}

/// Convert a field_type_tag back to a FieldType for set operations.
fn tag_to_field_type(tag: u16) -> Option<FieldType> {
    match tag {
        FIELD_TAG_F64 => Some(FieldType::F64),
        FIELD_TAG_I64 => Some(FieldType::I64),
        FIELD_TAG_BOOL => Some(FieldType::Bool),
        FIELD_TAG_STRING => Some(FieldType::String),
        FIELD_TAG_TIMESTAMP => Some(FieldType::Timestamp),
        FIELD_TAG_ARRAY => Some(FieldType::Array(Box::new(FieldType::Any))),
        FIELD_TAG_OBJECT => Some(FieldType::Object(String::new())),
        FIELD_TAG_DECIMAL => Some(FieldType::Decimal),
        FIELD_TAG_ANY => Some(FieldType::Any),
        _ => None,
    }
}

/// TypedObject operations for VirtualMachine
pub trait TypedObjectOps {
    /// Get field from typed object using precomputed offset (JIT optimization)
    fn op_get_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError>;

    /// Set field on typed object using precomputed offset (JIT optimization)
    fn op_set_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError>;
}

impl TypedObjectOps for super::VirtualMachine {
    /// Get field from typed object using precomputed field type tag.
    ///
    /// Zero-cost field access: the compiler embeds type_id, field_idx, and
    /// field_type_tag into the operand. At runtime we just read
    /// `slots[field_idx]` and use heap_mask + field_type_tag to interpret it.
    /// No schema lookup required.
    #[inline(always)]
    fn op_get_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let operand = instruction
            .operand
            .as_ref()
            .ok_or(VMError::InvalidOperand)?;

        if let Operand::TypedField {
            type_id,
            field_idx,
            field_type_tag,
        } = operand
        {
            let obj_nb = self.pop_vw()?;
            let obj_nb =
                if let Some(HeapValue::TypeAnnotatedValue { value, .. }) = obj_nb.as_heap_ref() {
                    value.as_ref().clone()
                } else {
                    obj_nb
                };

            // ValueWord fast path: HeapValue::TypedObject dispatch
            if let Some(HeapValue::TypedObject {
                schema_id,
                slots,
                heap_mask,
            }) = obj_nb.as_heap_ref()
            {
                // Schema mismatch — fall back to name-based field lookup.
                // This happens when e.g. `let u: User = { id: 1, name: "Alice" }`
                // creates an anonymous TypedObject (schema X) then the compiler
                // emits GetFieldTyped referencing the User schema (schema Y).
                if *schema_id != *type_id as u64 {
                    let ic_ip = self.ip;
                    let sid = *schema_id;

                    // IC fast path: if this site is monomorphic for this schema,
                    // use cached field_idx to skip double schema lookup.
                    if let Some(hit) =
                        crate::executor::ic_fast_paths::property_ic_check(self, ic_ip, sid)
                    {
                        let src_idx = hit.field_idx as usize;
                        if src_idx < slots.len() {
                            let is_heap = (*heap_mask & (1u64 << src_idx)) != 0;
                            let result =
                                read_slot_fast(&slots[src_idx], is_heap, hit.field_type_tag);
                            return self.push_vw(result);
                        }
                    }

                    // Resolve target field name from schema registry (immutable borrow).
                    // Extract all needed data before any mutable borrows.
                    let resolved = {
                        let target_schema =
                            self.program.type_schema_registry.get_by_id(*type_id as u32);
                        let source_schema = self
                            .program
                            .type_schema_registry
                            .get_by_id(*schema_id as u32);
                        match (target_schema, source_schema) {
                            (Some(target), Some(source)) => {
                                if let Some(target_field) = target.field_by_index(*field_idx) {
                                    let field_name = target_field.name.clone();
                                    if let Some(src_field_idx) = source.field_index(&field_name) {
                                        let tag = source
                                            .field_by_index(src_field_idx)
                                            .map(|f| field_type_to_tag(&f.field_type))
                                            .unwrap_or(0);
                                        Some((field_name, src_field_idx, tag))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    };

                    // Megamorphic cache fast path: when >4 schemas observed, check
                    // the direct-mapped global cache before doing name-based lookup.
                    if let Some((ref fname, _, _)) = resolved {
                        if let Some(hit) =
                            crate::executor::ic_fast_paths::megamorphic_property_check(
                                self, ic_ip, sid, fname,
                            )
                        {
                            let src_idx = hit.field_idx as usize;
                            if src_idx < slots.len() {
                                let is_heap = (*heap_mask & (1u64 << src_idx)) != 0;
                                let result =
                                    read_slot_fast(&slots[src_idx], is_heap, hit.field_type_tag);
                                return self.push_vw(result);
                            }
                        }
                    }

                    // Full name-based fallback: use pre-resolved field mapping.
                    if let Some((field_name, src_field_idx, tag)) = resolved {
                        let src_idx = src_field_idx as usize;
                        if src_idx < slots.len() {
                            let is_heap = (*heap_mask & (1u64 << src_idx)) != 0;
                            // Record IC and megamorphic cache (mutable borrows are safe now).
                            if let Some(fv) = self.current_feedback_vector() {
                                fv.record_property(
                                    ic_ip,
                                    sid,
                                    src_field_idx,
                                    tag,
                                    crate::feedback::RECEIVER_TYPED_OBJECT,
                                );
                            }
                            crate::executor::ic_fast_paths::megamorphic_property_insert(
                                self,
                                sid,
                                &field_name,
                                src_field_idx,
                                tag,
                            );
                            let result = read_slot_fast(&slots[src_idx], is_heap, tag);
                            return self.push_vw(result);
                        }
                    }
                    return self.push_vw(ValueWord::none());
                }

                let field_index = *field_idx as usize;
                debug_assert!(
                    field_index < slots.len(),
                    "GetFieldTyped field_idx {} out of bounds (slots.len() = {})",
                    field_index,
                    slots.len()
                );

                if field_index < slots.len() {
                    let is_heap = (*heap_mask & (1u64 << field_index)) != 0;
                    let result = read_slot_fast(&slots[field_index], is_heap, *field_type_tag);
                    return self.push_vw(result);
                } else {
                    return self.push_vw(ValueWord::none());
                }
            }

            // Non-TypedObject: return None
            self.push_vw(ValueWord::none())?;
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// Set field on typed object using precomputed field type tag.
    fn op_set_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let operand = instruction
            .operand
            .as_ref()
            .ok_or(VMError::InvalidOperand)?;

        if let Operand::TypedField {
            type_id: _,
            field_idx,
            field_type_tag,
        } = operand
        {
            let value_nb = self.pop_vw()?;
            let object_nb = self.pop_vw()?;
            let object_nb = if let Some(HeapValue::TypeAnnotatedValue { value, .. }) =
                object_nb.as_heap_ref()
            {
                value.as_ref().clone()
            } else {
                object_nb
            };
            let field_index = *field_idx as usize;

            if let Some(HeapValue::TypedObject {
                schema_id,
                slots,
                heap_mask,
            }) = object_nb.as_heap_ref()
            {
                if field_index < slots.len() {
                    let field_type = tag_to_field_type(*field_type_tag);
                    let (new_slots, new_mask) = clone_slots_with_update(
                        // BARRIER: TypedObject field update (CoW)
                        slots,
                        *heap_mask,
                        field_index,
                        &value_nb,
                        field_type.as_ref(),
                    );
                    return self.push_vw(ValueWord::from_heap_value(HeapValue::TypedObject {
                        schema_id: *schema_id,
                        slots: new_slots.into_boxed_slice(),
                        heap_mask: new_mask,
                    }));
                }
            }

            // Non-TypedObject or out-of-bounds index: preserve previous behavior.
            self.push_vw(object_nb)?;
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }
}
