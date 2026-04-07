//! Object merge operations (MergeObject, TypedMergeObject)
//!
//! Handles high-performance object merging with O(1) memcpy for typed objects.

use crate::{
    bytecode::{Instruction, Operand},
    executor::VirtualMachine,
};
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};

impl VirtualMachine {
    /// Merge two typed objects using pre-registered intersection schema
    ///
    /// Stack: [left_obj, right_obj] -> [merged_obj]
    /// Operand: TypedMerge { target_schema_id, left_size, right_size }
    ///
    /// O(1) memcpy-based merge - no HashMap allocation or lookup.
    pub(in crate::executor) fn op_typed_merge_object(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (target_schema_id, left_size, right_size) = match instruction.operand {
            Some(Operand::TypedMerge {
                target_schema_id,
                left_size,
                right_size,
            }) => (target_schema_id, left_size as usize, right_size as usize),
            _ => return Err(VMError::InvalidOperand),
        };

        let right_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
        let left_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);

        // Extract slots/heap_mask from both TypedObjects via HeapValue (no ValueWord materialization)
        let (left_slots, left_heap_mask, right_slots, right_heap_mask) =
            match (left_nb.as_heap_ref(), right_nb.as_heap_ref()) {
                (
                    Some(HeapValue::TypedObject {
                        slots: l,
                        heap_mask: lm,
                        ..
                    }),
                    Some(HeapValue::TypedObject {
                        slots: r,
                        heap_mask: rm,
                        ..
                    }),
                ) => (l, *lm, r, *rm),
                _ => {
                    return Err(VMError::RuntimeError(
                        "TypedMergeObject requires two TypedObjects".to_string(),
                    ));
                }
            };

        // Merge slots: concatenate left + right
        let left_count = left_slots.len().min(left_size / 8);
        let right_count = right_slots.len().min(right_size / 8);
        let mut merged_slots = Vec::with_capacity(left_count + right_count);
        let mut merged_heap_mask: u64 = 0;

        for i in 0..left_count {
            if left_heap_mask & (1u64 << i) != 0 {
                // Clone heap value for merged object
                merged_slots.push(unsafe { left_slots[i].clone_heap() });
                merged_heap_mask |= 1u64 << merged_slots.len().wrapping_sub(1);
            } else {
                merged_slots.push(left_slots[i]);
            }
        }
        for i in 0..right_count {
            let merged_idx = merged_slots.len();
            if right_heap_mask & (1u64 << i) != 0 {
                merged_slots.push(unsafe { right_slots[i].clone_heap() });
                merged_heap_mask |= 1u64 << merged_idx;
            } else {
                merged_slots.push(right_slots[i]);
            }
        }

        self.push_vw(ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: target_schema_id as u64,
            slots: merged_slots.into_boxed_slice(),
            heap_mask: merged_heap_mask,
        }))?;
        Ok(())
    }

    /// Merge two objects: pops source_obj, then target_obj from stack
    /// Creates a new object with all fields from target, then all fields from source (overwriting)
    pub(in crate::executor) fn op_merge_object(&mut self) -> Result<(), VMError> {
        let source_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
        let target_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);

        // TypedObject + TypedObject: schema-driven merge (HeapValue fast path)
        if let (
            Some(HeapValue::TypedObject {
                schema_id: left_sid,
                slots: left_slots,
                heap_mask: left_mask,
            }),
            Some(HeapValue::TypedObject {
                schema_id: right_sid,
                slots: right_slots,
                heap_mask: right_mask,
            }),
        ) = (target_nb.as_heap_ref(), source_nb.as_heap_ref())
        {
            let left_id = *left_sid as u32;
            let right_id = *right_sid as u32;

            // Collect field names and slot data before mutable borrow
            let (keep_left_indices, right_count) = {
                let left_schema = self.lookup_schema(left_id).ok_or_else(|| {
                    VMError::RuntimeError(format!("Schema {} not found", left_id))
                })?;
                let right_schema = self.lookup_schema(right_id).ok_or_else(|| {
                    VMError::RuntimeError(format!("Schema {} not found", right_id))
                })?;

                let right_names: std::collections::HashSet<&str> = right_schema
                    .fields
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect();

                let keep: Vec<usize> = left_schema
                    .fields
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| !right_names.contains(f.name.as_str()))
                    .map(|(i, _)| i)
                    .collect();

                (keep, right_schema.fields.len())
            };

            let merged_schema_id = self.derive_merged_schema(left_id, right_id)?;

            // Build merged slots: kept left slots + all right slots
            let mut merged_slots = Vec::with_capacity(keep_left_indices.len() + right_count);
            let mut merged_heap_mask: u64 = 0;

            for &idx in &keep_left_indices {
                let merged_idx = merged_slots.len();
                if *left_mask & (1u64 << idx) != 0 {
                    merged_slots.push(unsafe { left_slots[idx].clone_heap() });
                    merged_heap_mask |= 1u64 << merged_idx;
                } else {
                    merged_slots.push(left_slots[idx]);
                }
            }
            for idx in 0..right_count {
                let merged_idx = merged_slots.len();
                if *right_mask & (1u64 << idx) != 0 {
                    merged_slots.push(unsafe { right_slots[idx].clone_heap() });
                    merged_heap_mask |= 1u64 << merged_idx;
                } else {
                    merged_slots.push(right_slots[idx]);
                }
            }

            self.push_vw(ValueWord::from_heap_value(HeapValue::TypedObject {
                schema_id: merged_schema_id as u64,
                slots: merged_slots.into_boxed_slice(),
                heap_mask: merged_heap_mask,
            }))?;
            return Ok(());
        }

        Err(VMError::RuntimeError(
            "MergeObject requires compile-time typed objects; dynamic object merge is disabled"
                .to_string(),
        ))
    }
}
