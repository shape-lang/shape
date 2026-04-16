//! Json enum navigation builtins.
//!
//! Called from `std::core::json_value` extend block methods.
//! These operate on the payload values extracted by enum pattern matching:
//! - Object payload: ValueWord hashmap (keys=strings, values=Json TypedObjects)
//! - Array payload: ValueWord array of Json TypedObjects

use crate::executor::VirtualMachine;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueSlot, ValueWord, ValueWordExt};
use std::sync::Arc;

impl VirtualMachine {
    /// `__json_object_get(obj, key)` — look up a key in a Json Object payload hashmap.
    /// Returns the Json value, or constructs Json::Null for missing keys.
    pub(in crate::executor) fn builtin_json_object_get(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::ArityMismatch {
                function: "__json_object_get".to_string(),
                expected: 2,
                got: args.len(),
            });
        }
        let key = args[1].as_str().ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: args[1].type_name(),
        })?;

        if let Some((keys, values, index)) = args[0].as_hashmap() {
            let key_nb = ValueWord::from_string(Arc::new(key.to_string()));
            let hash = key_nb.vw_hash();
            if let Some(bucket) = index.get(&hash) {
                for &idx in bucket {
                    if idx < keys.len() && keys[idx].vw_equals(&key_nb) {
                        return Ok(values[idx].clone());
                    }
                }
            }
            Ok(self.construct_json_null())
        } else {
            Ok(self.construct_json_null())
        }
    }

    /// `__json_array_at(arr, index)` — access an element of a Json Array payload.
    /// Returns the Json value, or Json::Null for out-of-range indices.
    pub(in crate::executor) fn builtin_json_array_at(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::ArityMismatch {
                function: "__json_array_at".to_string(),
                expected: 2,
                got: args.len(),
            });
        }
        let index = args[1]
            .as_f64()
            .or_else(|| args[1].as_i64().map(|i| i as f64))
            .ok_or_else(|| VMError::TypeError {
                expected: "number",
                got: args[1].type_name(),
            })? as usize;

        if let Some(view) = args[0].as_any_array() {
            if index < view.len() {
                return Ok(view.get_nb(index).unwrap_or_else(ValueWord::none));
            }
        }
        Ok(self.construct_json_null())
    }

    /// `__json_object_keys(obj)` — return the string keys of a Json Object payload.
    pub(in crate::executor) fn builtin_json_object_keys(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::ArityMismatch {
                function: "__json_object_keys".to_string(),
                expected: 1,
                got: args.len(),
            });
        }
        if let Some((keys, _values, _index)) = args[0].as_hashmap() {
            Ok(ValueWord::from_array(Arc::new(keys.clone())))
        } else {
            Ok(ValueWord::from_array(Arc::new(Vec::new())))
        }
    }

    /// `__json_array_len(arr)` — return the length of a Json Array payload.
    pub(in crate::executor) fn builtin_json_array_len(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::ArityMismatch {
                function: "__json_array_len".to_string(),
                expected: 1,
                got: args.len(),
            });
        }
        if let Some(view) = args[0].as_any_array() {
            Ok(ValueWord::from_f64(view.len() as f64))
        } else {
            Ok(ValueWord::from_f64(0.0))
        }
    }

    /// `__json_object_len(obj)` — return the number of keys in a Json Object payload.
    pub(in crate::executor) fn builtin_json_object_len(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::ArityMismatch {
                function: "__json_object_len".to_string(),
                expected: 1,
                got: args.len(),
            });
        }
        if let Some((keys, _values, _index)) = args[0].as_hashmap() {
            Ok(ValueWord::from_f64(keys.len() as f64))
        } else {
            Ok(ValueWord::from_f64(0.0))
        }
    }

    /// Construct a `Json::Null` TypedObject (variant_id=0, no payload).
    fn construct_json_null(&self) -> ValueWord {
        if let Some(schema) = self.program.type_schema_registry.get("Json") {
            let num_slots = schema.fields.len();
            let mut slots = vec![ValueSlot::none(); num_slots];
            // __variant = 0 (Null)
            slots[0] = ValueSlot::from_int(0);
            ValueWord::from_heap_value(HeapValue::TypedObject {
                schema_id: schema.id as u64,
                slots: slots.into_boxed_slice(),
                heap_mask: 0, // no heap pointers
            })
        } else {
            ValueWord::none()
        }
    }
}
