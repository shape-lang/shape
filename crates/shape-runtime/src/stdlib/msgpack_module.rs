//! Native `msgpack` module for MessagePack encoding and decoding.
//!
//! Exports: msgpack.encode(value), msgpack.decode(data),
//!          msgpack.encode_bytes(value), msgpack.decode_bytes(data)

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::ValueWord;
use std::sync::Arc;

/// Convert a `serde_json::Value` into an untyped `ValueWord`.
///
/// This mirrors `json_value_to_nanboxed` from the json module.
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
            let items: Vec<ValueWord> = arr.into_iter().map(json_value_to_valueword).collect();
            ValueWord::from_array(Arc::new(items))
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
    let mut module = ModuleExports::new("msgpack");
    module.description = "MessagePack binary serialization".to_string();

    // msgpack.encode(value: any) -> Result<string>
    // Encodes a value to MessagePack and returns a hex-encoded string.
    module.add_function_with_schema(
        "encode",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let value = args
                .first()
                .ok_or_else(|| "msgpack.encode() requires a value argument".to_string())?;

            let json_value = value.to_json_value();
            let bytes = rmp_serde::to_vec(&json_value)
                .map_err(|e| format!("msgpack.encode() failed: {}", e))?;
            let hex_str = hex::encode(&bytes);

            Ok(ValueWord::from_ok(ValueWord::from_string(Arc::new(
                hex_str,
            ))))
        },
        ModuleFunction {
            description: "Encode a value to MessagePack (hex-encoded string)".to_string(),
            params: vec![ModuleParam {
                name: "value".to_string(),
                type_name: "any".to_string(),
                required: true,
                description: "Value to encode".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<string>".to_string()),
        },
    );

    // msgpack.decode(data: string) -> Result<any>
    // Decodes a hex-encoded MessagePack string to a value.
    module.add_function_with_schema(
        "decode",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let hex_str = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "msgpack.decode() requires a string argument".to_string())?;

            let bytes =
                hex::decode(hex_str).map_err(|e| format!("msgpack.decode() invalid hex: {}", e))?;
            let json_value: serde_json::Value = rmp_serde::from_slice(&bytes)
                .map_err(|e| format!("msgpack.decode() failed: {}", e))?;

            Ok(ValueWord::from_ok(json_value_to_valueword(json_value)))
        },
        ModuleFunction {
            description: "Decode a hex-encoded MessagePack string to a value".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Hex-encoded MessagePack data".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<any>".to_string()),
        },
    );

    // msgpack.encode_bytes(value: any) -> Result<Array<int>>
    // Encodes a value to MessagePack and returns raw bytes as an array of ints.
    module.add_function_with_schema(
        "encode_bytes",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let value = args
                .first()
                .ok_or_else(|| "msgpack.encode_bytes() requires a value argument".to_string())?;

            let json_value = value.to_json_value();
            let bytes = rmp_serde::to_vec(&json_value)
                .map_err(|e| format!("msgpack.encode_bytes() failed: {}", e))?;

            let items: Vec<ValueWord> = bytes
                .iter()
                .map(|&b| ValueWord::from_i64(b as i64))
                .collect();

            Ok(ValueWord::from_ok(ValueWord::from_array(Arc::new(items))))
        },
        ModuleFunction {
            description: "Encode a value to MessagePack as a byte array".to_string(),
            params: vec![ModuleParam {
                name: "value".to_string(),
                type_name: "any".to_string(),
                required: true,
                description: "Value to encode".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<Array<int>>".to_string()),
        },
    );

    // msgpack.decode_bytes(data: Array<int>) -> Result<any>
    // Decodes MessagePack from a byte array to a value.
    module.add_function_with_schema(
        "decode_bytes",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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

            Ok(ValueWord::from_ok(json_value_to_valueword(json_value)))
        },
        ModuleFunction {
            description: "Decode MessagePack from a byte array to a value".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "Array<int>".to_string(),
                required: true,
                description: "Array of byte values (0-255)".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<any>".to_string()),
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
        assert_eq!(module.name, "msgpack");
        assert!(module.has_export("encode"));
        assert!(module.has_export("decode"));
        assert!(module.has_export("encode_bytes"));
        assert!(module.has_export("decode_bytes"));
    }

    #[test]
    fn test_encode_decode_roundtrip_string() {
        let module = create_msgpack_module();
        let encode_fn = module.get_export("encode").unwrap();
        let decode_fn = module.get_export("decode").unwrap();
        let ctx = test_ctx();

        let input = ValueWord::from_string(Arc::new("hello".to_string()));
        let encoded = encode_fn(&[input], &ctx).unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = decode_fn(&[hex_str.clone()], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("hello"));
    }

    #[test]
    fn test_encode_decode_roundtrip_number() {
        let module = create_msgpack_module();
        let encode_fn = module.get_export("encode").unwrap();
        let decode_fn = module.get_export("decode").unwrap();
        let ctx = test_ctx();

        let input = ValueWord::from_f64(42.5);
        let encoded = encode_fn(&[input], &ctx).unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = decode_fn(&[hex_str.clone()], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_f64(), Some(42.5));
    }

    #[test]
    fn test_encode_decode_roundtrip_bool() {
        let module = create_msgpack_module();
        let encode_fn = module.get_export("encode").unwrap();
        let decode_fn = module.get_export("decode").unwrap();
        let ctx = test_ctx();

        let input = ValueWord::from_bool(true);
        let encoded = encode_fn(&[input], &ctx).unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = decode_fn(&[hex_str.clone()], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_bool(), Some(true));
    }

    #[test]
    fn test_encode_decode_roundtrip_null() {
        let module = create_msgpack_module();
        let encode_fn = module.get_export("encode").unwrap();
        let decode_fn = module.get_export("decode").unwrap();
        let ctx = test_ctx();

        let input = ValueWord::none();
        let encoded = encode_fn(&[input], &ctx).unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = decode_fn(&[hex_str.clone()], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert!(inner.is_none());
    }

    #[test]
    fn test_encode_decode_roundtrip_array() {
        let module = create_msgpack_module();
        let encode_fn = module.get_export("encode").unwrap();
        let decode_fn = module.get_export("decode").unwrap();
        let ctx = test_ctx();

        let input = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ]));
        let encoded = encode_fn(&[input], &ctx).unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = decode_fn(&[hex_str.clone()], &ctx).unwrap();
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
        let encode_fn = module.get_export("encode").unwrap();
        let decode_fn = module.get_export("decode").unwrap();
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
        let encoded = encode_fn(&[input], &ctx).unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = decode_fn(&[hex_str.clone()], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        let (keys, _values, _index) = inner.as_hashmap().expect("should be hashmap");
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_encode_bytes_decode_bytes_roundtrip() {
        let module = create_msgpack_module();
        let encode_bytes_fn = module.get_export("encode_bytes").unwrap();
        let decode_bytes_fn = module.get_export("decode_bytes").unwrap();
        let ctx = test_ctx();

        let input = ValueWord::from_string(Arc::new("test".to_string()));
        let encoded = encode_bytes_fn(&[input], &ctx).unwrap();
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

        let decoded = decode_bytes_fn(&[byte_arr.clone()], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("test"));
    }

    #[test]
    fn test_decode_invalid_hex() {
        let module = create_msgpack_module();
        let decode_fn = module.get_export("decode").unwrap();
        let ctx = test_ctx();

        let input = ValueWord::from_string(Arc::new("not_valid_hex!@#".to_string()));
        let result = decode_fn(&[input], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_invalid_msgpack() {
        let module = create_msgpack_module();
        let decode_fn = module.get_export("decode").unwrap();
        let ctx = test_ctx();

        // Valid hex but not valid msgpack
        let input = ValueWord::from_string(Arc::new("deadbeef".to_string()));
        let result = decode_fn(&[input], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_requires_string() {
        let module = create_msgpack_module();
        let decode_fn = module.get_export("decode").unwrap();
        let ctx = test_ctx();

        let result = decode_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_bytes_requires_array() {
        let module = create_msgpack_module();
        let decode_bytes_fn = module.get_export("decode_bytes").unwrap();
        let ctx = test_ctx();

        let result = decode_bytes_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_bytes_invalid_byte_values() {
        let module = create_msgpack_module();
        let decode_bytes_fn = module.get_export("decode_bytes").unwrap();
        let ctx = test_ctx();

        // Array with value > 255
        let input = ValueWord::from_array(Arc::new(vec![ValueWord::from_i64(300)]));
        let result = decode_bytes_fn(&[input], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_and_bytes_encode_same_data() {
        let module = create_msgpack_module();
        let encode_fn = module.get_export("encode").unwrap();
        let encode_bytes_fn = module.get_export("encode_bytes").unwrap();
        let ctx = test_ctx();

        let input = ValueWord::from_i64(42);
        let hex_result = encode_fn(&[input.clone()], &ctx).unwrap();
        let hex_str = hex_result.as_ok_inner().expect("should be Ok");
        let hex_bytes = hex::decode(hex_str.as_str().unwrap()).unwrap();

        let bytes_result = encode_bytes_fn(&[input], &ctx).unwrap();
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
        let encode_fn = module.get_export("encode").unwrap();
        let decode_fn = module.get_export("decode").unwrap();
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
            vec![ValueWord::from_array(Arc::new(vec![user1, user2]))],
        );

        let encoded = encode_fn(&[input], &ctx).unwrap();
        let hex_str = encoded.as_ok_inner().expect("should be Ok");

        let decoded = decode_fn(&[hex_str.clone()], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        let (keys, _values, _index) = inner.as_hashmap().expect("should be hashmap");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].as_str(), Some("users"));
    }
}
