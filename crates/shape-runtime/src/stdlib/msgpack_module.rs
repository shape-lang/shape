//! Native `msgpack` module for MessagePack encoding and decoding.
//!
//! Exports: msgpack.encode(value), msgpack.decode(data),
//!          msgpack.encode_bytes(value), msgpack.decode_bytes(data)
//!
//! NOTE: ALL FOUR FUNCTIONS REMAIN DEFERRED pending the **N4** (any-input
//! typed marshal) and **N6** (any-output typed marshal) architectural
//! decisions per `docs/defections.md` HashMap-marshal cluster's
//! sub-decision queue extension subsection (commit `d3411a7`).
//!
//! - `msgpack.encode(value: any)` and `msgpack.encode_bytes(value: any)`
//!   take a polymorphic `value: any` input parameter that maps to the
//!   N4 architectural surface. There is no `FromSlot` impl for an
//!   `any`-typed input in the post-bulldozer typed marshal layer
//!   (`ConcreteType::Any` exists as a RETURN type only).
//! - `msgpack.decode(data: string)` and
//!   `msgpack.decode_bytes(data: Array<int>)` return `Result<any>` —
//!   the decoded payload is a recursive `serde_json::Value`-equivalent
//!   tree projected via the deleted `TypedReturn::ValueWord` escape-
//!   hatch wrapper. `ConcreteReturn::Any` doesn't exist; mapping to
//!   the N6 architectural surface.
//!
//! Both N4 + N6 are supervisor-level decisions queued in the
//! HashMap-marshal cluster's sub-decision queue. After both land, the
//! msgpack module migrates as a batch (Stage D or equivalent).
//! Mirrors the deferral pattern from `csv_module.rs:7-8/183-189`'s
//! `parse_records`/`stringify_records` breadcrumb (now activated at
//! commit `fbe6155` once HashMap-marshal P1(b) landed).
//!
//! Tests use the deleted ValueWord API; deletable per `csv_module.rs`
//! migration precedent (commit `9f6b1d3`) once the bodies migrate. New
//! typed-marshal test fixtures arrive with the shape-vm cleanup
//! workstream.
//!
//! Current state: legacy bodies retained as cascade-broken pending
//! N4+N6 sign-off; the import errors at lines 7-8 + body errors
//! remain on-record in the cascade.

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteType, TypedReturn, register_typed_function};
use shape_value::{ArgVec, ValueWord, ValueWordExt};
use std::sync::Arc;

/// Convert a `serde_json::Value` into an untyped `ValueWord`.
///
/// Local helper for the msgpack module — produces HashMap/Array/scalar
/// values (NOT a typed `Json` enum). The json module's typed `Json` enum
/// path replaced the equivalent untyped helper there in sweep phase 4a;
/// this msgpack helper is scheduled for the same migration in a later
/// phase. TODO: phase-4b/4c — return a typed value so callers can pattern
/// match instead of reaching for `as_hashmap` / `as_any_array`.
fn json_value_to_valueword(value: serde_json::Value) -> ValueWord {
    match value {
        serde_json::Value::Null => ValueWord::none(),
        serde_json::Value::Bool(b) => ValueWord::from_bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ValueWord::from_i64(i)
            } else {
                ValueWord::from_f64(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => ValueWord::from_string(Arc::new(s)),
        serde_json::Value::Array(arr) => {
            let items: ArgVec =
                ArgVec::from_vec(arr.into_iter().map(json_value_to_valueword).collect());
            ValueWord::from_array(shape_value::vmarray_from_vec(items.into_inner()))
        }
        serde_json::Value::Object(map) => {
            let mut keys = Vec::with_capacity(map.len());
            let mut values = Vec::with_capacity(map.len());
            for (k, v) in map.into_iter() {
                keys.push(ValueWord::from_string(Arc::new(k)));
                values.push(json_value_to_valueword(v));
            }
            ValueWord::from_hashmap_pairs(keys, values)
        }
    }
}

/// Create the `msgpack` module with MessagePack encoding and decoding functions.
pub fn create_msgpack_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::msgpack");
    module.description = "MessagePack binary serialization".to_string();

    // msgpack.encode(value: any) -> Result<string>
    register_typed_function(
        &mut module,
        "encode",
        "Encode a value to MessagePack (hex-encoded string)",
        vec![ModuleParam {
            name: "value".to_string(),
            type_name: "any".to_string(),
            required: true,
            description: "Value to encode".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |args, _ctx| {
            let value = args
                .first()
                .ok_or_else(|| "msgpack.encode() requires a value argument".to_string())?;

            let json_value = value.to_json_value();
            let bytes = rmp_serde::to_vec(&json_value)
                .map_err(|e| format!("msgpack.encode() failed: {}", e))?;
            let hex_str = hex::encode(&bytes);

            Ok(TypedReturn::Ok(Box::new(TypedReturn::String(hex_str))))
        },
    );

    // msgpack.decode(data: string) -> Result<any>
    register_typed_function(
        &mut module,
        "decode",
        "Decode a hex-encoded MessagePack string to a value",
        vec![ModuleParam {
            name: "data".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Hex-encoded MessagePack data".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::Any)),
        |args, _ctx| {
            let hex_str = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "msgpack.decode() requires a string argument".to_string())?;

            let bytes = hex::decode(hex_str)
                .map_err(|e| format!("msgpack.decode() invalid hex: {}", e))?;
            let json_value: serde_json::Value = rmp_serde::from_slice(&bytes)
                .map_err(|e| format!("msgpack.decode() failed: {}", e))?;

            // The decoded payload is a recursive serde_json::Value that
            // lowers to nested HashMap/Array ValueWords. Phase 4d may
            // promote this to a typed `Any` enum modelled like the json
            // module's typed `Json`; for now we keep the existing
            // hand-rolled lowering and wrap the resulting ValueWord.
            Ok(TypedReturn::Ok(Box::new(TypedReturn::ValueWord(
                json_value_to_valueword(json_value),
            ))))
        },
    );

    // msgpack.encode_bytes(value: any) -> Result<Array<int>>
    register_typed_function(
        &mut module,
        "encode_bytes",
        "Encode a value to MessagePack as a byte array",
        vec![ModuleParam {
            name: "value".to_string(),
            type_name: "any".to_string(),
            required: true,
            description: "Value to encode".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::ArrayInt)),
        |args, _ctx| {
            let value = args
                .first()
                .ok_or_else(|| "msgpack.encode_bytes() requires a value argument".to_string())?;

            let json_value = value.to_json_value();
            let bytes = rmp_serde::to_vec(&json_value)
                .map_err(|e| format!("msgpack.encode_bytes() failed: {}", e))?;

            let items: Vec<i64> = bytes.iter().map(|&b| b as i64).collect();
            Ok(TypedReturn::Ok(Box::new(TypedReturn::ArrayI64(items))))
        },
    );

    // msgpack.decode_bytes(data: Array<int>) -> Result<any>
    register_typed_function(
        &mut module,
        "decode_bytes",
        "Decode MessagePack from a byte array to a value",
        vec![ModuleParam {
            name: "data".to_string(),
            type_name: "Array<int>".to_string(),
            required: true,
            description: "Array of byte values (0-255)".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::Any)),
        |args, _ctx| {
            let arr = args.first().and_then(|a| a.as_any_array()).ok_or_else(|| {
                "msgpack.decode_bytes() requires an Array<int> argument".to_string()
            })?;

            let generic = arr.to_generic();
            let bytes: Result<Vec<u8>, String> = generic
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    v.as_i64()
                        .or_else(|| v.as_f64().map(|f| f as i64))
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or_else(|| {
                            format!(
                                "msgpack.decode_bytes() element at index {} is not a valid byte",
                                i
                            )
                        })
                })
                .collect();
            let bytes = bytes?;

            let json_value: serde_json::Value = rmp_serde::from_slice(&bytes)
                .map_err(|e| format!("msgpack.decode_bytes() failed: {}", e))?;

            Ok(TypedReturn::Ok(Box::new(TypedReturn::ValueWord(
                json_value_to_valueword(json_value),
            ))))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> crate::module_exports::ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        crate::module_exports::ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        }
    }

    #[test]
    fn test_msgpack_module_creation() {
        let module = create_msgpack_module();
        assert_eq!(module.name, "std::core::msgpack");
        assert!(module.has_export("encode"));
        assert!(module.has_export("decode"));
        assert!(module.has_export("encode_bytes"));
        assert!(module.has_export("decode_bytes"));
    }

    #[test]
    fn test_encode_decode_roundtrip_string() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::from_string(Arc::new("hello".to_string()));
        let encoded = module.invoke_export("encode", &[input], &ctx).unwrap().unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = module.invoke_export("decode", &[hex_str.clone()], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("hello"));
    }

    #[test]
    fn test_encode_decode_roundtrip_number() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::from_f64(42.5);
        let encoded = module.invoke_export("encode", &[input], &ctx).unwrap().unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = module.invoke_export("decode", &[hex_str.clone()], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_f64(), Some(42.5));
    }

    #[test]
    fn test_encode_decode_roundtrip_bool() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::from_bool(true);
        let encoded = module.invoke_export("encode", &[input], &ctx).unwrap().unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = module.invoke_export("decode", &[hex_str.clone()], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_bool(), Some(true));
    }

    #[test]
    fn test_encode_decode_roundtrip_null() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::none();
        let encoded = module.invoke_export("encode", &[input], &ctx).unwrap().unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = module.invoke_export("decode", &[hex_str.clone()], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert!(inner.is_none());
    }

    #[test]
    fn test_encode_decode_roundtrip_array() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ]));
        let encoded = module.invoke_export("encode", &[input], &ctx).unwrap().unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = module.invoke_export("decode", &[hex_str.clone()], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        let arr = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64(), Some(1));
        assert_eq!(arr[1].as_i64(), Some(2));
        assert_eq!(arr[2].as_i64(), Some(3));
    }

    #[test]
    fn test_encode_decode_roundtrip_object() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("name".to_string())),
                ValueWord::from_string(Arc::new("age".to_string())),
            ],
            vec![
                ValueWord::from_string(Arc::new("Alice".to_string())),
                ValueWord::from_i64(30),
            ],
        );
        let encoded = module.invoke_export("encode", &[input], &ctx).unwrap().unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = module.invoke_export("decode", &[hex_str.clone()], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        let (keys, _values, _index) = inner.as_hashmap().expect("should be hashmap");
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_encode_bytes_decode_bytes_roundtrip() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::from_string(Arc::new("test".to_string()));
        let encoded = module.invoke_export("encode_bytes", &[input], &ctx).unwrap().unwrap();
        let byte_arr = encoded.as_ok_inner().expect("should be Ok");

        // Verify it's an array of ints
        let arr = byte_arr
            .as_any_array()
            .expect("should be array")
            .to_generic();
        assert!(!arr.is_empty());
        for v in arr.iter() {
            let byte_val = v.as_i64().expect("should be int");
            assert!((0..=255).contains(&byte_val));
        }

        let decoded = module.invoke_export("decode_bytes", &[byte_arr.clone()], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("test"));
    }

    #[test]
    fn test_decode_invalid_hex() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::from_string(Arc::new("not_valid_hex!@#".to_string()));
        let result = module.invoke_export("decode", &[input], &ctx).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_invalid_msgpack() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        // Valid hex but not valid msgpack
        let input = ValueWord::from_string(Arc::new("deadbeef".to_string()));
        let result = module.invoke_export("decode", &[input], &ctx).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_requires_string() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let result = module.invoke_export("decode", &[ValueWord::from_f64(42.0)], &ctx).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_bytes_requires_array() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let result = module.invoke_export("decode_bytes", &[ValueWord::from_f64(42.0)], &ctx).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_bytes_invalid_byte_values() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        // Array with value > 255
        let input = ValueWord::from_array(shape_value::vmarray_from_vec(vec![ValueWord::from_i64(300)]));
        let result = module.invoke_export("decode_bytes", &[input], &ctx).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_and_bytes_encode_same_data() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        let input = ValueWord::from_i64(42);
        let hex_result = module.invoke_export("encode", &[input.clone()], &ctx).unwrap().unwrap();
        let hex_str = hex_result.as_ok_inner().expect("should be Ok");
        let hex_bytes = hex::decode(hex_str.as_str().unwrap()).unwrap();

        let bytes_result = module.invoke_export("encode_bytes", &[input], &ctx).unwrap().unwrap();
        let byte_arr = bytes_result.as_ok_inner().expect("should be Ok");
        let arr = byte_arr
            .as_any_array()
            .expect("should be array")
            .to_generic();
        let arr_bytes: Vec<u8> = arr.iter().map(|v| v.as_i64().unwrap() as u8).collect();

        assert_eq!(hex_bytes, arr_bytes);
    }

    #[test]
    fn test_schemas() {
        let module = create_msgpack_module();

        let encode_schema = module.get_schema("encode").unwrap();
        assert_eq!(encode_schema.params.len(), 1);
        assert_eq!(encode_schema.params[0].name, "value");
        assert!(encode_schema.params[0].required);
        assert_eq!(encode_schema.return_type.as_deref(), Some("Result<string>"));

        let decode_schema = module.get_schema("decode").unwrap();
        assert_eq!(decode_schema.params.len(), 1);
        assert_eq!(decode_schema.params[0].name, "data");
        assert!(decode_schema.params[0].required);
        assert_eq!(decode_schema.return_type.as_deref(), Some("Result<any>"));

        let encode_bytes_schema = module.get_schema("encode_bytes").unwrap();
        assert_eq!(encode_bytes_schema.params.len(), 1);
        assert_eq!(
            encode_bytes_schema.return_type.as_deref(),
            Some("Result<Array<int>>")
        );

        let decode_bytes_schema = module.get_schema("decode_bytes").unwrap();
        assert_eq!(decode_bytes_schema.params.len(), 1);
        assert_eq!(decode_bytes_schema.params[0].type_name, "Array<int>");
        assert_eq!(
            decode_bytes_schema.return_type.as_deref(),
            Some("Result<any>")
        );
    }

    #[test]
    fn test_encode_decode_nested_structure() {
        let module = create_msgpack_module();
        let ctx = test_ctx();

        // Build nested: { "users": [{"name": "Alice"}, {"name": "Bob"}] }
        let user1 = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_string(Arc::new("name".to_string()))],
            vec![ValueWord::from_string(Arc::new("Alice".to_string()))],
        );
        let user2 = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_string(Arc::new("name".to_string()))],
            vec![ValueWord::from_string(Arc::new("Bob".to_string()))],
        );
        let input = ValueWord::from_hashmap_pairs(
            vec![ValueWord::from_string(Arc::new("users".to_string()))],
            vec![ValueWord::from_array(shape_value::vmarray_from_vec(vec![user1, user2]))],
        );

        let encoded = module.invoke_export("encode", &[input], &ctx).unwrap().unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = module.invoke_export("decode", &[hex_str.clone()], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        let (keys, _values, _index) = inner.as_hashmap().expect("should be hashmap");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].as_str(), Some("users"));
    }
}
