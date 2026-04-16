//! Object operation builtin implementations
//!
//! Handles: objectRest

use crate::executor::VirtualMachine;
use shape_value::{HeapValue, VMError, ValueWord, ValueWordExt};

impl VirtualMachine {
    /// ObjectRest: Create new object excluding specified keys
    pub(in crate::executor) fn builtin_object_rest(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError(
                "object_rest() requires exactly 2 arguments".to_string(),
            ));
        }

        // Extract exclude keys from the second arg (array of strings)
        let keys_arr = args[1]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("object_rest() second argument must be an array".to_string())
            })?
            .to_generic();

        let mut exclude = std::collections::HashSet::new();
        for key in keys_arr.iter() {
            if let Some(s) = key.as_str() {
                exclude.insert(s.to_string());
            } else {
                return Err(VMError::RuntimeError(
                    "object_rest() keys must be strings".to_string(),
                ));
            }
        }

        // TypedObject: schema-driven subset (HeapValue fast path)
        if let Some(HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask,
        // cold-path: as_heap_ref retained — TypedObject schema-driven subset
        }) = args[0].as_heap_ref() // cold-path
        {
            let sid = *schema_id as u32;
            let orig_slots = slots.clone();
            let orig_mask = *heap_mask;

            // Collect kept field indices before mutable borrow
            let kept_indices: Vec<usize> = {
                let schema = self
                    .lookup_schema(sid)
                    .ok_or_else(|| VMError::RuntimeError(format!("Schema {} not found", sid)))?;
                schema
                    .fields
                    .iter()
                    .filter(|f| !exclude.contains(&f.name))
                    .map(|f| f.index as usize)
                    .collect()
            };

            let subset_id = self.derive_subset_schema(sid, &exclude)?;

            // Build subset slots
            let mut new_slots = Vec::with_capacity(kept_indices.len());
            let mut new_mask: u64 = 0;
            for &orig_idx in &kept_indices {
                let new_idx = new_slots.len();
                if orig_mask & (1u64 << orig_idx) != 0 {
                    new_slots.push(unsafe { orig_slots[orig_idx].clone_heap() });
                    new_mask |= 1u64 << new_idx;
                } else {
                    new_slots.push(orig_slots[orig_idx]);
                }
            }

            return Ok(ValueWord::from_heap_value(
                shape_value::heap_value::HeapValue::TypedObject {
                    schema_id: subset_id as u64,
                    slots: new_slots.into_boxed_slice(),
                    heap_mask: new_mask,
                },
            ));
        }

        Err(VMError::RuntimeError(
            "object_rest() first argument must be an object".to_string(),
        ))
    }
}
