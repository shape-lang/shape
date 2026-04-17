//! Object creation operations (NewArray, NewObject, NewTypedObject)
//!
//! Handles allocation and initialization of arrays, objects, and typed objects.

use crate::{
    bytecode::{Instruction, Operand},
    executor::VirtualMachine,
};
use rust_decimal::prelude::ToPrimitive;
use shape_runtime::type_schema::FieldType;
use shape_value::{VMError, ValueSlot, ValueWord, ValueWordExt};
use std::collections::HashMap;
use std::sync::Arc;

fn field_type_to_int_width(ft: &FieldType) -> Option<shape_ast::IntWidth> {
    match ft {
        FieldType::I8 => Some(shape_ast::IntWidth::I8),
        FieldType::U8 => Some(shape_ast::IntWidth::U8),
        FieldType::I16 => Some(shape_ast::IntWidth::I16),
        FieldType::U16 => Some(shape_ast::IntWidth::U16),
        FieldType::I32 => Some(shape_ast::IntWidth::I32),
        FieldType::U32 => Some(shape_ast::IntWidth::U32),
        FieldType::U64 => Some(shape_ast::IntWidth::U64),
        _ => None,
    }
}

impl VirtualMachine {
    /// Create a new TypedObject with fields from stack
    ///
    /// Stack: [...field_values] -> [typed_object]
    /// Operand: TypedObjectAlloc { schema_id, field_count }
    pub(in crate::executor) fn op_new_typed_object(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (schema_id, field_count) = match instruction.operand {
            Some(Operand::TypedObjectAlloc {
                schema_id,
                field_count,
            }) => (schema_id, field_count),
            _ => return Err(VMError::InvalidOperand),
        };

        // Pop field values from stack (in reverse order since stack is LIFO)
        let mut nb_fields: Vec<ValueWord> = Vec::with_capacity(field_count as usize);
        for _ in 0..field_count {
            nb_fields.push(ValueWord::from_raw_bits(self.pop_raw_u64()?));
        }
        nb_fields.reverse();

        let field_types: Option<Vec<FieldType>> = self
            .lookup_schema(schema_id as u32)
            .map(|schema| schema.fields.iter().map(|f| f.field_type.clone()).collect());

        // Allocate slots: one ValueSlot per field (ValueWord-native path)
        let mut slots = Vec::with_capacity(field_count as usize);
        let mut heap_mask: u64 = 0;

        for (i, nb) in nb_fields.iter().enumerate() {
            let field_type = field_types.as_ref().and_then(|types| types.get(i));
            let (slot, is_heap) = nb_to_slot_with_field_type(nb, field_type);
            if is_heap {
                heap_mask |= 1u64 << i;
            }
            slots.push(slot);
        }

        // Create TypedObject and push to stack via HeapValue (no ValueWord round-trip)
        use shape_value::heap_value::HeapValue;
        let typed_obj = HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        };
        self.push_raw_u64(ValueWord::from_heap_value(typed_obj))?;
        Ok(())
    }

    pub(in crate::executor) fn op_new_object(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Count(count)) = instruction.operand {
            let mut object: HashMap<String, ValueWord> = HashMap::new();

            // Pop key-value pairs
            for _ in 0..count {
                let value_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
                let key_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
                let key_str = key_nb
                    .as_str()
                    .ok_or_else(|| VMError::TypeError {
                        expected: "string",
                        got: key_nb.type_name(),
                    })?
                    .to_string();

                object.insert(key_str, value_nb);
            }

            let pairs: Vec<(&str, ValueWord)> = object
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect();
            let typed = self.create_typed_object_from_pairs(&pairs)?;
            self.push_raw_u64(typed)?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Create a new Matrix from values on the stack.
    ///
    /// Stack: [...f64_values (rows*cols)] -> [matrix]
    /// Operand: MatrixDims { rows, cols }
    pub(in crate::executor) fn op_new_matrix(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (rows, cols) = match instruction.operand {
            Some(Operand::MatrixDims { rows, cols }) => (rows as u32, cols as u32),
            _ => return Err(VMError::InvalidOperand),
        };

        let total = (rows as usize) * (cols as usize);
        let mut data = shape_value::aligned_vec::AlignedVec::with_capacity(total);

        // Pop values from stack in reverse order (LIFO), then reverse
        let mut values = Vec::with_capacity(total);
        for _ in 0..total {
            let nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
            let val = nb.as_number_coerce().ok_or_else(|| VMError::TypeError {
                expected: "number",
                got: nb.type_name(),
            })?;
            values.push(val);
        }
        values.reverse();

        for v in values {
            data.push(v);
        }

        let mat = shape_value::heap_value::MatrixData::from_flat(data, rows, cols);
        self.push_raw_u64(ValueWord::from_matrix(std::sync::Arc::new(mat)))
    }

    pub(in crate::executor) fn op_new_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Count(count)) = instruction.operand {
            let mut elements: Vec<ValueWord> = Vec::with_capacity(count as usize);

            // Pop elements in reverse order
            for _ in 0..count {
                elements.push(ValueWord::from_raw_bits(self.pop_raw_u64()?));
            }
            elements.reverse();

            self.push_raw_u64(ValueWord::from_array(shape_value::vmarray_from_vec(elements)))?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Create a typed array (IntArray/FloatArray/BoolArray) from N elements on the stack.
    ///
    /// Inspects element types at runtime and packs into the most specific typed representation.
    /// Falls back to a generic Array if elements are mixed or unsupported.
    pub(in crate::executor) fn op_new_typed_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let count = match instruction.operand {
            Some(Operand::Count(c)) => c as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        // Pop elements in reverse order
        let mut elements: Vec<ValueWord> = Vec::with_capacity(count);
        for _ in 0..count {
            elements.push(ValueWord::from_raw_bits(self.pop_raw_u64()?));
        }
        elements.reverse();

        if count == 0 {
            // Empty array — default to generic array
            return self.push_raw_u64(ValueWord::from_array(shape_value::vmarray_from_vec(elements)));
        }

        // Detect element type from first element, then verify all match
        if elements[0].is_i64() {
            // Try to pack as IntArray
            let mut ints = Vec::with_capacity(count);
            for elem in &elements {
                if let Some(i) = elem.as_i64() {
                    ints.push(i);
                } else if let Some(f) = elem.as_f64() {
                    // f64 whole number coercion
                    if f.is_finite() && f == f.trunc() && f.abs() < (i64::MAX as f64) {
                        ints.push(f as i64);
                    } else {
                        // Fallback to generic
                        return self.push_raw_u64(ValueWord::from_array(shape_value::vmarray_from_vec(elements)));
                    }
                } else {
                    return self.push_raw_u64(ValueWord::from_array(shape_value::vmarray_from_vec(elements)));
                }
            }
            self.push_raw_u64(ValueWord::from_int_array(Arc::new(ints.into())))
        } else if elements[0].is_f64() {
            // Try to pack as FloatArray
            let mut floats = shape_value::aligned_vec::AlignedVec::with_capacity(count);
            for elem in &elements {
                if let Some(f) = elem.as_f64() {
                    floats.push(f);
                } else if let Some(i) = elem.as_i64() {
                    floats.push(i as f64);
                } else {
                    return self.push_raw_u64(ValueWord::from_array(shape_value::vmarray_from_vec(elements)));
                }
            }
            self.push_raw_u64(ValueWord::from_float_array(Arc::new(floats.into())))
        } else if elements[0].is_bool() {
            // Try to pack as BoolArray
            let mut bools = Vec::with_capacity(count);
            for elem in &elements {
                if let Some(b) = elem.as_bool() {
                    bools.push(b as u8);
                } else {
                    return self.push_raw_u64(ValueWord::from_array(shape_value::vmarray_from_vec(elements)));
                }
            }
            self.push_raw_u64(ValueWord::from_bool_array(Arc::new(bools.into())))
        } else {
            // Not a typed-array-eligible type, fall back to generic
            self.push_raw_u64(ValueWord::from_array(shape_value::vmarray_from_vec(elements)))
        }
    }
}

/// Convert a ValueWord to a ValueSlot using schema field type when available.
/// This avoids ambiguous non-heap encodings for `FieldType::Any`.
pub(in crate::executor) fn nb_to_slot_with_field_type(
    nb: &ValueWord,
    field_type: Option<&FieldType>,
) -> (ValueSlot, bool) {
    match field_type {
        Some(FieldType::I64) => (
            ValueSlot::from_int(
                nb.as_i64()
                    .or_else(|| nb.as_f64().map(|n| n as i64))
                    .unwrap_or(0),
            ),
            false,
        ),
        Some(ft) if ft.is_width_integer() => {
            if matches!(ft, FieldType::U64) {
                // U64 may exceed i64::MAX — extract via as_u64() for lossless storage
                let val = nb
                    .as_u64_value()
                    .or_else(|| nb.as_i64().map(|i| i as u64))
                    .or_else(|| nb.as_f64().map(|n| n as u64))
                    .unwrap_or(0);
                (ValueSlot::from_int(val as i64), false)
            } else {
                let raw = nb
                    .as_i64()
                    .or_else(|| nb.as_f64().map(|n| n as i64))
                    .unwrap_or(0);
                let truncated = if let Some(w) = field_type_to_int_width(ft) {
                    w.truncate(raw)
                } else {
                    raw
                };
                (ValueSlot::from_int(truncated), false)
            }
        }
        Some(FieldType::Bool) => (
            ValueSlot::from_bool(nb.as_bool().unwrap_or(nb.is_truthy())),
            false,
        ),
        Some(FieldType::F64) | Some(FieldType::Decimal) => (
            ValueSlot::from_number(
                nb.as_number_coerce()
                    .or_else(|| nb.as_decimal().and_then(|d| d.to_f64()))
                    .unwrap_or(0.0),
            ),
            false,
        ),
        // `Any` must preserve dynamic type losslessly — including inline inline tag
        // variants like Function, ModuleFunction, I48, etc.  `from_value_word`
        // stores raw NaN-boxed bits for inline tags and clones HeapValues for
        // heap tags, so the exact tag round-trips through `as_value_word`.
        Some(FieldType::Any) | None => ValueSlot::from_value_word(nb),
        // For non-primitive schema field types, preserve full value via from_value_word.
        Some(_) => {
            if nb.is_none() {
                (ValueSlot::none(), false)
            } else {
                ValueSlot::from_value_word(nb)
            }
        }
    }
}

/// Read a ValueWord value from a TypedObject slot.
///
/// `field_type` is optional and lets callers preserve i64/bool semantics for non-heap slots.
pub(in crate::executor) fn read_slot_nb(
    slots: &[ValueSlot],
    index: usize,
    heap_mask: u64,
    field_type: Option<&shape_runtime::type_schema::FieldType>,
) -> ValueWord {
    if index >= slots.len() {
        return ValueWord::none();
    }

    if heap_mask & (1u64 << index) != 0 {
        return slots[index].as_heap_nb();
    }

    match field_type {
        Some(shape_runtime::type_schema::FieldType::I64) => {
            ValueWord::from_i64(slots[index].as_i64())
        }
        Some(shape_runtime::type_schema::FieldType::Bool) => {
            ValueWord::from_bool(slots[index].as_bool())
        }
        Some(shape_runtime::type_schema::FieldType::F64) => {
            ValueWord::from_f64(slots[index].as_f64())
        }
        Some(shape_runtime::type_schema::FieldType::Decimal) => ValueWord::from_decimal(
            rust_decimal::Decimal::from_f64_retain(slots[index].as_f64()).unwrap_or_default(),
        ),
        // Width integer types: stored via from_int(), read back via as_i64()
        Some(ft) if ft.is_width_integer() => {
            let raw_bits = slots[index].as_i64() as u64;
            if matches!(ft, shape_runtime::type_schema::FieldType::U64)
                && raw_bits > i64::MAX as u64
            {
                ValueWord::from_native_u64(raw_bits)
            } else {
                ValueWord::from_i64(slots[index].as_i64())
            }
        }
        // Any and non-primitive types: reconstruct via as_value_word to preserve
        // all inline inline tag variants (Function, ModuleFunction, I48, etc.)
        Some(_) | None => slots[index].as_value_word(false),
    }
}

/// Read a ValueWord from a TypedObject slot with optional schema field type.
#[cfg(test)]
pub(in crate::executor) fn read_slot_value_typed(
    slots: &[ValueSlot],
    index: usize,
    heap_mask: u64,
    field_type: Option<&FieldType>,
) -> ValueWord {
    if index >= slots.len() {
        return ValueWord::none();
    }
    if heap_mask & (1u64 << index) != 0 {
        return slots[index].as_heap_nb();
    }

    match field_type {
        Some(FieldType::I64) => ValueWord::from_i64(slots[index].as_i64()),
        Some(FieldType::Bool) => ValueWord::from_bool(slots[index].as_bool()),
        Some(FieldType::F64) => ValueWord::from_f64(slots[index].as_f64()),
        Some(FieldType::Decimal) => ValueWord::from_decimal(
            rust_decimal::Decimal::from_f64_retain(slots[index].as_f64()).unwrap_or_default(),
        ),
        // Width integer types: stored via from_int(), read back via as_i64()
        Some(ft) if ft.is_width_integer() => {
            let raw_bits = slots[index].as_i64() as u64;
            if matches!(ft, FieldType::U64) && raw_bits > i64::MAX as u64 {
                ValueWord::from_native_u64(raw_bits)
            } else {
                ValueWord::from_i64(slots[index].as_i64())
            }
        }
        // Any and non-primitive types: reconstruct via as_value_word to preserve
        // all inline inline tag variants (Function, ModuleFunction, I48, etc.)
        Some(_) | None => slots[index].as_value_word(false),
    }
}

/// Clone slots and overwrite one index with a new ValueWord value.
pub(in crate::executor) fn clone_slots_with_update(
    slots: &[ValueSlot],
    heap_mask: u64,
    update_index: usize,
    update_value: &ValueWord,
    field_type: Option<&FieldType>,
) -> (Vec<ValueSlot>, u64) {
    let mut new_slots = Vec::with_capacity(slots.len());
    let mut new_mask: u64 = 0;

    for (index, slot) in slots.iter().enumerate() {
        if index == update_index {
            let (updated_slot, is_heap) = nb_to_slot_with_field_type(update_value, field_type);
            if is_heap {
                new_mask |= 1u64 << index;
            }
            new_slots.push(updated_slot);
            continue;
        }

        if heap_mask & (1u64 << index) != 0 {
            new_slots.push(unsafe { slot.clone_heap() });
            new_mask |= 1u64 << index;
        } else {
            new_slots.push(*slot);
        }
    }

    (new_slots, new_mask)
}
