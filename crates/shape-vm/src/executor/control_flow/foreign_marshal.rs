//! ValueWord <-> MessagePack marshaling for foreign function calls.

use crate::executor::objects::object_creation::read_slot_nb;
use rust_decimal::prelude::ToPrimitive;
use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueSlot, ValueWord, ValueWordExt};
use std::sync::Arc;

/// Serialize a slice of ValueWord args to msgpack bytes (as an array).
pub fn marshal_args(args: &[ValueWord], schemas: &TypeSchemaRegistry) -> Result<Vec<u8>, VMError> {
    let values: Vec<rmpv::Value> = args
        .iter()
        .map(|nb| nanboxed_to_msgpack_value(nb, schemas))
        .collect();
    let array = rmpv::Value::Array(values);
    rmp_serde::to_vec(&array).map_err(|e| {
        VMError::RuntimeError(format!("Failed to marshal foreign function args: {}", e))
    })
}

/// Deserialize msgpack bytes to a ValueWord result using declared type information.
///
/// All callers must provide type info. The `return_type` string is the full
/// declared return type (e.g. "Result<int>", "Result<{id: int, name: string}>").
/// `schema_id` is `Some` when the return type contains an inline object type
/// (registered at compile time).
pub fn unmarshal_result(
    bytes: &[u8],
    return_type: &str,
    schema_id: Option<u32>,
    schemas: &TypeSchemaRegistry,
) -> Result<ValueWord, VMError> {
    if bytes.is_empty() {
        return Ok(ValueWord::none());
    }
    let value: rmpv::Value = rmp_serde::from_slice(bytes).map_err(|e| {
        VMError::RuntimeError(format!(
            "Failed to unmarshal foreign function result: {}",
            e
        ))
    })?;

    let inner_type = strip_result_wrapper(return_type);
    typed_msgpack_to_nanboxed(&value, inner_type, schema_id, schemas)
}

// ============================================================================
// Typed msgpack -> ValueWord conversion
// ============================================================================

/// Convert an rmpv::Value to a ValueWord using the declared type for validation.
fn typed_msgpack_to_nanboxed(
    val: &rmpv::Value,
    target: &str,
    schema_id: Option<u32>,
    schemas: &TypeSchemaRegistry,
) -> Result<ValueWord, VMError> {
    // Handle nil
    if matches!(val, rmpv::Value::Nil) {
        if target == "none" {
            return Ok(ValueWord::none());
        }
        return Err(marshal_error(format!("expected {}, got None", target)));
    }

    match target {
        "int" => match val {
            rmpv::Value::Integer(i) => {
                if let Some(n) = i.as_i64() {
                    Ok(ValueWord::from_i64(n))
                } else if let Some(n) = i.as_u64() {
                    Ok(ValueWord::from_i64(n as i64))
                } else {
                    Err(marshal_error("integer out of range"))
                }
            }
            _ => Err(marshal_error(format!(
                "expected int, got {}",
                msgpack_type_name(val)
            ))),
        },

        "float" | "number" => match val {
            rmpv::Value::F64(f) => Ok(ValueWord::from_f64(*f)),
            rmpv::Value::F32(f) => Ok(ValueWord::from_f64(*f as f64)),
            rmpv::Value::Integer(i) => {
                // Coerce int -> float
                if let Some(n) = i.as_i64() {
                    Ok(ValueWord::from_f64(n as f64))
                } else if let Some(n) = i.as_u64() {
                    Ok(ValueWord::from_f64(n as f64))
                } else {
                    Err(marshal_error("integer out of range for float coercion"))
                }
            }
            _ => Err(marshal_error(format!(
                "expected {}, got {}",
                target,
                msgpack_type_name(val)
            ))),
        },

        "string" => match val {
            rmpv::Value::String(s) => {
                if let Some(s) = s.as_str() {
                    Ok(ValueWord::from_string(Arc::new(s.to_string())))
                } else {
                    Err(marshal_error("string contains invalid UTF-8"))
                }
            }
            _ => Err(marshal_error(format!(
                "expected string, got {}",
                msgpack_type_name(val)
            ))),
        },

        "bool" => match val {
            rmpv::Value::Boolean(b) => Ok(ValueWord::from_bool(*b)),
            _ => Err(marshal_error(format!(
                "expected bool, got {}",
                msgpack_type_name(val)
            ))),
        },

        "none" => Err(marshal_error(format!(
            "expected none, got {}",
            msgpack_type_name(val)
        ))),

        // Vec<T>
        s if s.starts_with("Vec<") && s.ends_with('>') => {
            let elem_type = &s[4..s.len() - 1];
            match val {
                rmpv::Value::Array(arr) => {
                    let items: Result<Vec<ValueWord>, VMError> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, item)| {
                            // For arrays of objects, pass the schema_id through
                            typed_msgpack_to_nanboxed(item, elem_type, schema_id, schemas).map_err(
                                |e| VMError::RuntimeError(format!("Vec element [{}]: {}", i, e)),
                            )
                        })
                        .collect();
                    Ok(ValueWord::from_array(Arc::new(items?)))
                }
                _ => Err(marshal_error(format!(
                    "expected Vec, got {}",
                    msgpack_type_name(val)
                ))),
            }
        }

        // Object type: {f1: T1, f2: T2, ...}
        s if s.starts_with('{') && s.ends_with('}') => {
            match val {
                rmpv::Value::Map(entries) => {
                    if let Some(sid) = schema_id {
                        marshal_typed_object(entries, sid, schemas)
                    } else {
                        // No schema registered — fall back to HashMap
                        Ok(untyped_msgpack_to_nanboxed(val))
                    }
                }
                _ => Err(marshal_error(format!(
                    "expected object, got {}",
                    msgpack_type_name(val)
                ))),
            }
        }

        // Named type with schema_id — marshal as typed object
        _ if schema_id.is_some() => match val {
            rmpv::Value::Map(entries) => marshal_typed_object(entries, schema_id.unwrap(), schemas),
            _ => Err(marshal_error(format!(
                "expected object for type '{}', got {}",
                target,
                msgpack_type_name(val)
            ))),
        },

        // "any" or unknown — untyped path
        _ => Ok(untyped_msgpack_to_nanboxed(val)),
    }
}

/// Construct a `HeapValue::TypedObject` from a msgpack Map using a registered schema.
fn marshal_typed_object(
    entries: &[(rmpv::Value, rmpv::Value)],
    schema_id: u32,
    schemas: &TypeSchemaRegistry,
) -> Result<ValueWord, VMError> {
    let schema = schemas.get_by_id(schema_id).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "FFI marshal: schema ID {} not found in registry",
            schema_id
        ))
    })?;

    // Build name -> value lookup from msgpack entries
    let mut name_to_value: std::collections::HashMap<&str, &rmpv::Value> =
        std::collections::HashMap::with_capacity(entries.len());
    for (k, v) in entries {
        if let rmpv::Value::String(s) = k {
            if let Some(name) = s.as_str() {
                name_to_value.insert(name, v);
            }
        }
    }

    let field_count = schema.fields.len();
    let mut slots = Vec::with_capacity(field_count);
    let mut heap_mask: u64 = 0;

    for field in &schema.fields {
        let val = name_to_value.get(field.wire_name());
        use shape_runtime::type_schema::FieldType;

        match &field.field_type {
            FieldType::I64 => {
                let n = val
                    .and_then(|v| match v {
                        rmpv::Value::Integer(i) => i.as_i64(),
                        _ => None,
                    })
                    .unwrap_or(0);
                slots.push(ValueSlot::from_int(n));
            }
            FieldType::F64 => {
                let f = val
                    .and_then(|v| match v {
                        rmpv::Value::F64(f) => Some(*f),
                        rmpv::Value::F32(f) => Some(*f as f64),
                        rmpv::Value::Integer(i) => i.as_i64().map(|n| n as f64),
                        _ => None,
                    })
                    .unwrap_or(0.0);
                slots.push(ValueSlot::from_number(f));
            }
            FieldType::Bool => {
                let b = val
                    .and_then(|v| match v {
                        rmpv::Value::Boolean(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(false);
                slots.push(ValueSlot::from_bool(b));
            }
            FieldType::String => {
                let s = val
                    .and_then(|v| match v {
                        rmpv::Value::String(s) => s.as_str().map(|s| s.to_string()),
                        _ => None,
                    })
                    .unwrap_or_default();
                slots.push(ValueSlot::from_heap(HeapValue::String(Arc::new(s))));
                heap_mask |= 1u64 << (slots.len() - 1);
            }
            FieldType::Array(_) => {
                let arr_nb = val
                    .map(|v| untyped_msgpack_to_nanboxed(v))
                    .unwrap_or_else(|| ValueWord::from_array(Arc::new(Vec::new())));
                // Extract the inner array or wrap
                let heap_val = if let Some(view) = arr_nb.as_any_array() {
                    HeapValue::Array(view.to_generic())
                // cold-path: as_heap_ref retained — msgpack marshaling fallback
                } else if let Some(hv) = arr_nb.as_heap_ref() { // cold-path
                    hv.clone()
                } else {
                    HeapValue::Array(Arc::new(Vec::new()))
                };
                slots.push(ValueSlot::from_heap(heap_val));
                heap_mask |= 1u64 << (slots.len() - 1);
            }
            FieldType::Object(_) => {
                let obj_nb = val
                    .map(|v| untyped_msgpack_to_nanboxed(v))
                    .unwrap_or_else(ValueWord::none);
                // cold-path: as_heap_ref retained — msgpack marshaling object field
                if let Some(hv) = obj_nb.as_heap_ref() { // cold-path
                    slots.push(ValueSlot::from_heap(hv.clone()));
                    heap_mask |= 1u64 << (slots.len() - 1);
                } else {
                    slots.push(ValueSlot::none());
                }
            }
            // Any, Timestamp, Decimal — use heap for heap types, inline for primitives
            _ => {
                let nb = val
                    .map(|v| untyped_msgpack_to_nanboxed(v))
                    .unwrap_or_else(ValueWord::none);
                // cold-path: as_heap_ref retained — msgpack marshaling generic fallback
                if let Some(hv) = nb.as_heap_ref() { // cold-path
                    slots.push(ValueSlot::from_heap(hv.clone()));
                    heap_mask |= 1u64 << (slots.len() - 1);
                } else if let Some(f) = nb.as_f64() {
                    slots.push(ValueSlot::from_number(f));
                } else if let Some(i) = nb.as_i64() {
                    slots.push(ValueSlot::from_number(i as f64));
                } else if let Some(b) = nb.as_bool() {
                    slots.push(ValueSlot::from_bool(b));
                } else {
                    slots.push(ValueSlot::none());
                }
            }
        }
    }

    Ok(ValueWord::from_heap_value(HeapValue::TypedObject {
        schema_id: schema_id as u64,
        slots: slots.into_boxed_slice(),
        heap_mask,
    }))
}

// ============================================================================
// ValueWord -> msgpack (outgoing marshalling, unchanged)
// ============================================================================

/// Convert a ValueWord value to an rmpv::Value.
fn nanboxed_to_msgpack_value(nb: &ValueWord, schemas: &TypeSchemaRegistry) -> rmpv::Value {
    use shape_value::tags::{is_tagged, get_tag, TAG_INT, TAG_BOOL, TAG_NONE, TAG_HEAP};
    let bits = nb.raw_bits();
    if !is_tagged(bits) {
        if let Some(f) = nb.as_f64() {
            return rmpv::Value::F64(f);
        } else {
            return rmpv::Value::Nil;
        }
    }
    match get_tag(bits) {
        TAG_INT => {
            if let Some(i) = nb.as_i64() {
                rmpv::Value::Integer(rmpv::Integer::from(i))
            } else {
                rmpv::Value::Nil
            }
        }
        TAG_BOOL => rmpv::Value::Boolean(nb.as_bool().unwrap_or(false)),
        TAG_NONE => rmpv::Value::Nil,
        TAG_HEAP => {
            // Handle unified arrays.
            if let Some(view) = nb.as_any_array() {
                return rmpv::Value::Array(
                    (0..view.len())
                        .map(|i| {
                            let elem = view.get_nb(i).unwrap_or_else(ValueWord::none);
                            nanboxed_to_msgpack_value(&elem, schemas)
                        })
                        .collect(),
                );
            }
            // cold-path: as_heap_ref retained — msgpack serialization
            nb.as_heap_ref() // cold-path
                .map(|hv| heap_to_msgpack_value(hv, schemas))
                .unwrap_or(rmpv::Value::Nil)
        }
        _ => rmpv::Value::Nil,
    }
}

fn heap_to_msgpack_value(hv: &HeapValue, schemas: &TypeSchemaRegistry) -> rmpv::Value {
    match hv {
        HeapValue::String(s) => rmpv::Value::String(rmpv::Utf8String::from(s.as_str())),
        HeapValue::Array(arr) => rmpv::Value::Array(
            arr.iter()
                .map(|item| nanboxed_to_msgpack_value(item, schemas))
                .collect(),
        ),
        HeapValue::HashMap(map) => {
            let entries: Vec<(rmpv::Value, rmpv::Value)> = map
                .keys
                .iter()
                .zip(map.values.iter())
                .map(|(key, value)| {
                    (
                        nanboxed_to_msgpack_value(key, schemas),
                        nanboxed_to_msgpack_value(value, schemas),
                    )
                })
                .collect();
            rmpv::Value::Map(entries)
        }
        HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask,
        } => {
            if let Some(schema) = schemas.get_by_id(*schema_id as u32) {
                let mut entries = Vec::with_capacity(schema.fields.len());
                for field in &schema.fields {
                    let value = read_slot_nb(
                        slots,
                        field.index as usize,
                        *heap_mask,
                        Some(&field.field_type),
                    );
                    entries.push((
                        rmpv::Value::String(rmpv::Utf8String::from(field.wire_name().to_string())),
                        nanboxed_to_msgpack_value(&value, schemas),
                    ));
                }
                return rmpv::Value::Map(entries);
            }

            // Unknown schema: preserve payload as a stable positional map.
            let entries: Vec<(rmpv::Value, rmpv::Value)> = slots
                .iter()
                .enumerate()
                .map(|(index, slot)| {
                    let is_heap = *heap_mask & (1u64 << index) != 0;
                    let value = slot.as_value_word(is_heap);
                    (
                        rmpv::Value::String(rmpv::Utf8String::from(index.to_string())),
                        nanboxed_to_msgpack_value(&value, schemas),
                    )
                })
                .collect();
            rmpv::Value::Map(entries)
        }
        HeapValue::Rare(shape_value::RareHeapData::TypeAnnotatedValue { value, .. }) => nanboxed_to_msgpack_value(value, schemas),
        HeapValue::Some(inner) => nanboxed_to_msgpack_value(inner, schemas),
        HeapValue::Ok(inner) => nanboxed_to_msgpack_value(inner, schemas),
        HeapValue::Err(inner) => nanboxed_to_msgpack_value(inner, schemas),
        HeapValue::BigInt(n) => rmpv::Value::Integer(rmpv::Integer::from(*n)),
        HeapValue::Decimal(d) => d
            .to_f64()
            .map(rmpv::Value::F64)
            .unwrap_or_else(|| rmpv::Value::String(rmpv::Utf8String::from(d.to_string()))),
        _ => rmpv::Value::Nil,
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Strip `Result<...>` wrapper from a type string.
fn strip_result_wrapper(s: &str) -> &str {
    if s.starts_with("Result<") && s.ends_with('>') {
        &s[7..s.len() - 1]
    } else {
        s
    }
}

/// Create a MARSHAL_ERROR VMError.
fn marshal_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Human-readable msgpack type name for error messages.
fn msgpack_type_name(val: &rmpv::Value) -> &'static str {
    match val {
        rmpv::Value::Nil => "nil",
        rmpv::Value::Boolean(_) => "bool",
        rmpv::Value::Integer(_) => "int",
        rmpv::Value::F32(_) | rmpv::Value::F64(_) => "float",
        rmpv::Value::String(_) => "string",
        rmpv::Value::Array(_) => "array",
        rmpv::Value::Map(_) => "map",
        rmpv::Value::Binary(_) => "binary",
        rmpv::Value::Ext(_, _) => "ext",
    }
}

/// Untyped msgpack -> ValueWord conversion (used for "any" type and fallback).
fn untyped_msgpack_to_nanboxed(val: &rmpv::Value) -> ValueWord {
    match val {
        rmpv::Value::Nil => ValueWord::none(),
        rmpv::Value::Boolean(b) => ValueWord::from_bool(*b),
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                ValueWord::from_i64(n)
            } else if let Some(n) = i.as_u64() {
                ValueWord::from_i64(n as i64)
            } else {
                ValueWord::none()
            }
        }
        rmpv::Value::F32(f) => ValueWord::from_f64(*f as f64),
        rmpv::Value::F64(f) => ValueWord::from_f64(*f),
        rmpv::Value::String(s) => {
            if let Some(s) = s.as_str() {
                ValueWord::from_string(Arc::new(s.to_string()))
            } else {
                ValueWord::none()
            }
        }
        rmpv::Value::Array(arr) => {
            let items: Vec<ValueWord> = arr.iter().map(untyped_msgpack_to_nanboxed).collect();
            ValueWord::from_array(Arc::new(items))
        }
        rmpv::Value::Map(entries) => {
            let mut keys = Vec::with_capacity(entries.len());
            let mut values = Vec::with_capacity(entries.len());
            for (k, v) in entries.iter() {
                keys.push(untyped_msgpack_to_nanboxed(k));
                values.push(untyped_msgpack_to_nanboxed(v));
            }
            ValueWord::from_hashmap_pairs(keys, values)
        }
        rmpv::Value::Ext(_, _) | rmpv::Value::Binary(_) => ValueWord::none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::{FieldType, TypeSchemaRegistry};
    use shape_value::{ValueSlot, heap_value::HeapValue};

    fn measurement_value(
        schema_id: u32,
        timestamp: &str,
        value: f64,
        sensor_id: &str,
    ) -> ValueWord {
        ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: vec![
                ValueSlot::from_heap(HeapValue::String(Arc::new(timestamp.to_string()))),
                ValueSlot::from_number(value),
                ValueSlot::from_heap(HeapValue::String(Arc::new(sensor_id.to_string()))),
            ]
            .into_boxed_slice(),
            heap_mask: 0b101,
        })
    }

    #[test]
    fn marshal_args_preserves_typed_object_fields_as_msgpack_map() {
        let mut schemas = TypeSchemaRegistry::new();
        let measurement_schema_id = schemas.register_type(
            "Measurement",
            vec![
                ("timestamp".to_string(), FieldType::String),
                ("value".to_string(), FieldType::F64),
                ("sensor_id".to_string(), FieldType::String),
            ],
        );

        let readings = ValueWord::from_array(Arc::new(vec![
            measurement_value(measurement_schema_id, "2026-02-22T10:00:00Z", 10.0, "A"),
            measurement_value(measurement_schema_id, "2026-02-22T10:01:00Z", 10.5, "A"),
        ]));

        let bytes = marshal_args(&[readings], &schemas).expect("marshal should succeed");
        let decoded: rmpv::Value = rmp_serde::from_slice(&bytes).expect("valid msgpack");

        let outer = decoded.as_array().expect("expected outer arg array");
        let reading_items = outer[0].as_array().expect("expected readings array");
        let first = reading_items[0]
            .as_map()
            .expect("expected typed object map");

        let mut fields = std::collections::HashMap::new();
        for (k, v) in first {
            if let rmpv::Value::String(s) = k
                && let Some(name) = s.as_str()
            {
                fields.insert(name.to_string(), v.clone());
            }
        }

        assert_eq!(
            fields.get("timestamp").and_then(|v| v.as_str()),
            Some("2026-02-22T10:00:00Z")
        );
        assert_eq!(fields.get("value").and_then(|v| v.as_f64()), Some(10.0));
        assert_eq!(fields.get("sensor_id").and_then(|v| v.as_str()), Some("A"));
    }

    #[test]
    fn unmarshal_result_typed_int() {
        let schemas = TypeSchemaRegistry::new();
        let val = rmpv::Value::Integer(rmpv::Integer::from(42));
        let bytes = rmp_serde::to_vec(&val).unwrap();
        let result = unmarshal_result(&bytes, "Result<int>", None, &schemas).unwrap();
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn unmarshal_result_typed_string_rejects_int() {
        let schemas = TypeSchemaRegistry::new();
        let val = rmpv::Value::Integer(rmpv::Integer::from(42));
        let bytes = rmp_serde::to_vec(&val).unwrap();
        let result = unmarshal_result(&bytes, "Result<string>", None, &schemas);
        assert!(result.is_err());
    }

    #[test]
    fn unmarshal_result_typed_bool() {
        let schemas = TypeSchemaRegistry::new();
        let val = rmpv::Value::Boolean(true);
        let bytes = rmp_serde::to_vec(&val).unwrap();
        let result = unmarshal_result(&bytes, "Result<bool>", None, &schemas).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn unmarshal_result_typed_array_of_ints() {
        let schemas = TypeSchemaRegistry::new();
        let val = rmpv::Value::Array(vec![
            rmpv::Value::Integer(rmpv::Integer::from(1)),
            rmpv::Value::Integer(rmpv::Integer::from(2)),
            rmpv::Value::Integer(rmpv::Integer::from(3)),
        ]);
        let bytes = rmp_serde::to_vec(&val).unwrap();
        let result = unmarshal_result(&bytes, "Result<Vec<int>>", None, &schemas).unwrap();
        let view = result.as_any_array().expect("Expected array view");
        let items = view.to_generic();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].as_i64(), Some(1));
    }

    #[test]
    fn unmarshal_result_typed_object() {
        let mut schemas = TypeSchemaRegistry::new();
        let sid = schemas.register_type(
            "__ffi_test_return",
            vec![
                ("id".to_string(), FieldType::I64),
                ("name".to_string(), FieldType::String),
            ],
        );

        let val = rmpv::Value::Map(vec![
            (
                rmpv::Value::String(rmpv::Utf8String::from("id")),
                rmpv::Value::Integer(rmpv::Integer::from(42)),
            ),
            (
                rmpv::Value::String(rmpv::Utf8String::from("name")),
                rmpv::Value::String(rmpv::Utf8String::from("hello")),
            ),
        ]);
        let bytes = rmp_serde::to_vec(&val).unwrap();
        let result = unmarshal_result(
            &bytes,
            "Result<{id: int, name: string}>",
            Some(sid as u32),
            &schemas,
        )
        .unwrap();

        // Verify it's a TypedObject
        // cold-path: as_heap_ref retained — test assertion
        match result.as_heap_ref() { // cold-path
            Some(HeapValue::TypedObject {
                schema_id, slots, ..
            }) => {
                assert_eq!(*schema_id, sid as u64);
                assert_eq!(slots[0].as_i64(), 42);
                match slots[1].as_heap_value() {
                    HeapValue::String(s) => assert_eq!(s.as_str(), "hello"),
                    other => panic!("expected string, got {:?}", other),
                }
            }
            _ => panic!("expected TypedObject"),
        }
    }

    #[test]
    fn unmarshal_result_any_fallback() {
        let schemas = TypeSchemaRegistry::new();
        let val = rmpv::Value::String(rmpv::Utf8String::from("anything"));
        let bytes = rmp_serde::to_vec(&val).unwrap();
        let result = unmarshal_result(&bytes, "Result<any>", None, &schemas).unwrap();
        // cold-path: as_heap_ref retained — test assertion
        match result.as_heap_ref() { // cold-path
            Some(HeapValue::String(s)) => assert_eq!(s.as_str(), "anything"),
            _ => panic!("expected string"),
        }
    }
}
