//! Native `json` module for JSON parsing and serialization.
//!
//! Exports: json.parse(text), json.stringify(value, pretty?), json.is_valid(text)

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use crate::type_schema::{SchemaId, TypeSchemaRegistry, nb_to_slot};
use shape_value::heap_value::HeapValue;
use shape_value::{ValueSlot, ValueWord};
use std::sync::Arc;

/// Convert a `serde_json::Value` into an untyped `ValueWord` (legacy fallback).
fn json_value_to_nanboxed(value: serde_json::Value) -> ValueWord {
    match value {
        serde_json::Value::Null => ValueWord::none(),
        serde_json::Value::Bool(b) => ValueWord::from_bool(b),
        serde_json::Value::Number(n) => ValueWord::from_f64(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => ValueWord::from_string(Arc::new(s)),
        serde_json::Value::Array(arr) => {
            let items: Vec<ValueWord> = arr.into_iter().map(json_value_to_nanboxed).collect();
            ValueWord::from_array(Arc::new(items))
        }
        serde_json::Value::Object(map) => {
            let mut keys = Vec::with_capacity(map.len());
            let mut values = Vec::with_capacity(map.len());
            for (k, v) in map.into_iter() {
                keys.push(ValueWord::from_string(Arc::new(k)));
                values.push(json_value_to_nanboxed(v));
            }
            ValueWord::from_hashmap_pairs(keys, values)
        }
    }
}

// Json enum variant IDs (must match order in json_value.shape)
const JSON_VARIANT_NULL: i64 = 0;
const JSON_VARIANT_BOOL: i64 = 1;
const JSON_VARIANT_NUMBER: i64 = 2;
const JSON_VARIANT_STR: i64 = 3;
const JSON_VARIANT_ARRAY: i64 = 4;
const JSON_VARIANT_OBJECT: i64 = 5;

/// Build a Json enum TypedObject with the given variant and payload.
fn make_json_enum(schema_id: u64, variant_id: i64, payload: Option<ValueWord>) -> ValueWord {
    // Json layout: slot 0 = __variant (I64), slot 1 = __payload_0 (Any)
    let variant_slot = ValueSlot::from_int(variant_id);
    let (payload_slot, heap_mask) = if let Some(ref p) = payload {
        let (slot, is_heap) = nb_to_slot(p);
        (slot, if is_heap { 1u64 << 1 } else { 0u64 })
    } else {
        (ValueSlot::none(), 0u64)
    };
    let slots = vec![variant_slot, payload_slot].into_boxed_slice();
    ValueWord::from_heap_value(HeapValue::TypedObject {
        schema_id,
        slots,
        heap_mask,
    })
}

/// Convert a `serde_json::Value` into a typed `Json` enum TypedObject.
fn json_value_to_enum(value: serde_json::Value, schema_id: u64) -> ValueWord {
    match value {
        serde_json::Value::Null => make_json_enum(schema_id, JSON_VARIANT_NULL, None),
        serde_json::Value::Bool(b) => {
            make_json_enum(schema_id, JSON_VARIANT_BOOL, Some(ValueWord::from_bool(b)))
        }
        serde_json::Value::Number(n) => make_json_enum(
            schema_id,
            JSON_VARIANT_NUMBER,
            Some(ValueWord::from_f64(n.as_f64().unwrap_or(0.0))),
        ),
        serde_json::Value::String(s) => make_json_enum(
            schema_id,
            JSON_VARIANT_STR,
            Some(ValueWord::from_string(Arc::new(s))),
        ),
        serde_json::Value::Array(arr) => {
            let items: Vec<ValueWord> = arr
                .into_iter()
                .map(|v| json_value_to_enum(v, schema_id))
                .collect();
            make_json_enum(
                schema_id,
                JSON_VARIANT_ARRAY,
                Some(ValueWord::from_array(Arc::new(items))),
            )
        }
        serde_json::Value::Object(map) => {
            let mut keys = Vec::with_capacity(map.len());
            let mut values = Vec::with_capacity(map.len());
            for (k, v) in map.into_iter() {
                keys.push(ValueWord::from_string(Arc::new(k)));
                values.push(json_value_to_enum(v, schema_id));
            }
            make_json_enum(
                schema_id,
                JSON_VARIANT_OBJECT,
                Some(ValueWord::from_hashmap_pairs(keys, values)),
            )
        }
    }
}

/// Convert a `serde_json::Value` into a typed struct `ValueWord` using a schema.
///
/// Matches JSON keys to schema fields using `wire_name()` (respects `@alias`).
fn json_object_to_typed(
    schema_id: SchemaId,
    schema: &crate::type_schema::TypeSchema,
    map: &serde_json::Map<String, serde_json::Value>,
    registry: &TypeSchemaRegistry,
) -> Result<ValueWord, String> {
    use crate::type_schema::FieldType;

    let num_fields = schema.fields.len();
    let mut slots = vec![ValueSlot::none(); num_fields];
    let mut heap_mask = 0u64;

    for field in &schema.fields {
        let wire = field.wire_name();
        let json_val = map.get(wire);
        let nb = if let Some(jv) = json_val {
            json_value_to_typed_nb(jv, &field.field_type, registry)?
        } else {
            ValueWord::none()
        };

        // Convert ValueWord to ValueSlot based on field type
        let (slot, is_heap) = match &field.field_type {
            FieldType::I64 => (
                ValueSlot::from_int(
                    nb.as_i64()
                        .or_else(|| nb.as_f64().map(|n| n as i64))
                        .unwrap_or(0),
                ),
                false,
            ),
            FieldType::Bool => (ValueSlot::from_bool(nb.as_bool().unwrap_or(false)), false),
            FieldType::F64 | FieldType::Decimal => (
                ValueSlot::from_number(nb.as_number_coerce().unwrap_or(0.0)),
                false,
            ),
            _ => nb_to_slot(&nb),
        };

        slots[field.index as usize] = slot;
        if is_heap {
            heap_mask |= 1u64 << field.index;
        }
    }

    Ok(ValueWord::from_heap_value(HeapValue::TypedObject {
        schema_id: schema_id as u64,
        slots: slots.into_boxed_slice(),
        heap_mask,
    }))
}

/// Convert a single JSON value to a ValueWord according to the field type.
fn json_value_to_typed_nb(
    value: &serde_json::Value,
    field_type: &crate::type_schema::FieldType,
    registry: &TypeSchemaRegistry,
) -> Result<ValueWord, String> {
    use crate::type_schema::FieldType;
    match (value, field_type) {
        (serde_json::Value::Null, _) => Ok(ValueWord::none()),
        (serde_json::Value::Bool(b), _) => Ok(ValueWord::from_bool(*b)),
        (serde_json::Value::Number(n), FieldType::I64) => {
            Ok(ValueWord::from_i64(n.as_i64().unwrap_or(0)))
        }
        (serde_json::Value::Number(n), _) => Ok(ValueWord::from_f64(n.as_f64().unwrap_or(0.0))),
        (serde_json::Value::String(s), _) => Ok(ValueWord::from_string(Arc::new(s.clone()))),
        (serde_json::Value::Array(arr), _) => {
            let items: Vec<ValueWord> = arr
                .iter()
                .map(|v| json_value_to_typed_nb(v, &FieldType::Any, registry))
                .collect::<Result<_, _>>()?;
            Ok(ValueWord::from_array(Arc::new(items)))
        }
        (serde_json::Value::Object(obj), FieldType::Object(type_name)) => {
            if let Some(nested_schema) = registry.get(type_name) {
                json_object_to_typed(nested_schema.id, nested_schema, obj, registry)
            } else {
                // Schema not found — fall back to untyped hashmap
                Ok(json_value_to_nanboxed(serde_json::Value::Object(
                    obj.clone(),
                )))
            }
        }
        (serde_json::Value::Object(obj), _) => Ok(json_value_to_nanboxed(
            serde_json::Value::Object(obj.clone()),
        )),
    }
}

/// Create the `json` module with JSON parsing and serialization functions.
pub fn create_json_module() -> ModuleExports {
    let mut module = ModuleExports::new("json");
    module.description = "JSON parsing and serialization".to_string();

    // json.parse(text: string) -> Result<Json>
    // Returns typed Json enum when the schema is registered, otherwise untyped.
    module.add_function_with_schema(
        "parse",
        |args: &[ValueWord], ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "json.parse() requires a string argument".to_string())?;

            let parsed: serde_json::Value =
                serde_json::from_str(text).map_err(|e| format!("json.parse() failed: {}", e))?;

            let result = if let Some(json_schema) = ctx.schemas.get("Json") {
                json_value_to_enum(parsed, json_schema.id as u64)
            } else {
                json_value_to_nanboxed(parsed)
            };

            Ok(ValueWord::from_ok(result))
        },
        ModuleFunction {
            description: "Parse a JSON string into Shape values".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "JSON string to parse".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<Json>".to_string()),
        },
    );

    // json.__parse_typed(text: string, schema_id: number) -> Result<T>
    // Internal: deserializes JSON directly into a typed struct using the schema.
    module.add_function_with_schema(
        "__parse_typed",
        |args: &[ValueWord], ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "json.__parse_typed() requires a string argument".to_string())?;
            let schema_id = args
                .get(1)
                .and_then(|a| {
                    a.as_f64()
                        .map(|n| n as u32)
                        .or_else(|| a.as_i64().map(|n| n as u32))
                })
                .ok_or_else(|| "json.__parse_typed() requires a schema_id argument".to_string())?;

            let parsed: serde_json::Value = serde_json::from_str(text)
                .map_err(|e| format!("json.__parse_typed() failed: {}", e))?;

            let map = match parsed {
                serde_json::Value::Object(m) => m,
                _ => {
                    return Err("json.__parse_typed() requires a JSON object".to_string());
                }
            };

            let schema = ctx
                .schemas
                .get_by_id(schema_id)
                .ok_or_else(|| format!("json.__parse_typed(): unknown schema id {}", schema_id))?;

            let result = json_object_to_typed(schema_id, schema, &map, ctx.schemas)?;
            Ok(ValueWord::from_ok(result))
        },
        ModuleFunction {
            description: "Parse a JSON string into a typed struct".to_string(),
            params: vec![
                ModuleParam {
                    name: "text".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "JSON string to parse".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "schema_id".to_string(),
                    type_name: "number".to_string(),
                    required: true,
                    description: "Schema ID of the target type".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Result<any>".to_string()),
        },
    );

    // json.stringify(value: any, pretty?: bool) -> Result<string>
    module.add_function_with_schema(
        "stringify",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let value = args
                .first()
                .ok_or_else(|| "json.stringify() requires a value argument".to_string())?;

            let pretty = args.get(1).and_then(|a| a.as_bool()).unwrap_or(false);

            let json_value = value.to_json_value();

            let output = if pretty {
                serde_json::to_string_pretty(&json_value)
            } else {
                serde_json::to_string(&json_value)
            }
            .map_err(|e| format!("json.stringify() failed: {}", e))?;

            Ok(ValueWord::from_ok(ValueWord::from_string(Arc::new(output))))
        },
        ModuleFunction {
            description: "Serialize a Shape value to a JSON string".to_string(),
            params: vec![
                ModuleParam {
                    name: "value".to_string(),
                    type_name: "any".to_string(),
                    required: true,
                    description: "Value to serialize".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "pretty".to_string(),
                    type_name: "bool".to_string(),
                    required: false,
                    description: "Pretty-print with indentation (default: false)".to_string(),
                    default_snippet: Some("false".to_string()),
                    ..Default::default()
                },
            ],
            return_type: Some("Result<string>".to_string()),
        },
    );

    // json.is_valid(text: string) -> bool
    module.add_function_with_schema(
        "is_valid",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "json.is_valid() requires a string argument".to_string())?;

            let valid = serde_json::from_str::<serde_json::Value>(text).is_ok();
            Ok(ValueWord::from_bool(valid))
        },
        ModuleFunction {
            description: "Check if a string is valid JSON".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "String to validate as JSON".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
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
    fn test_json_module_creation() {
        let module = create_json_module();
        assert_eq!(module.name, "json");
        assert!(module.has_export("parse"));
        assert!(module.has_export("stringify"));
        assert!(module.has_export("is_valid"));
    }

    #[test]
    fn test_json_parse_string() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(r#""hello""#.to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        // Result is Ok(value)
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("hello"));
    }

    #[test]
    fn test_json_parse_number() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("42.5".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_f64(), Some(42.5));
    }

    #[test]
    fn test_json_parse_bool() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("true".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_bool(), Some(true));
    }

    #[test]
    fn test_json_parse_null() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("null".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert!(inner.is_none());
    }

    #[test]
    fn test_json_parse_array() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("[1, 2, 3]".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let arr = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_f64(), Some(1.0));
        assert_eq!(arr[1].as_f64(), Some(2.0));
        assert_eq!(arr[2].as_f64(), Some(3.0));
    }

    #[test]
    fn test_json_parse_object() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(r#"{"a": 1, "b": "two"}"#.to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, _values, _index) = inner.as_hashmap().expect("should be hashmap");
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_json_parse_invalid() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("{invalid}".to_string()));
        let result = parse_fn(&[input], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_json_parse_requires_string() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let result = parse_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_json_stringify_number() {
        let module = create_json_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let result = stringify_fn(&[ValueWord::from_f64(42.0)], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("42.0"));
    }

    #[test]
    fn test_json_stringify_string() {
        let module = create_json_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let result = stringify_fn(
            &[ValueWord::from_string(Arc::new("hello".to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("\"hello\""));
    }

    #[test]
    fn test_json_stringify_bool() {
        let module = create_json_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let result = stringify_fn(&[ValueWord::from_bool(true)], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("true"));
    }

    #[test]
    fn test_json_stringify_none() {
        let module = create_json_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let result = stringify_fn(&[ValueWord::none()], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("null"));
    }

    #[test]
    fn test_json_stringify_array() {
        let module = create_json_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let arr = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
        ]));
        let result = stringify_fn(&[arr], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("[1.0,2.0]"));
    }

    #[test]
    fn test_json_stringify_pretty() {
        let module = create_json_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let result = stringify_fn(
            &[ValueWord::from_f64(42.0), ValueWord::from_bool(true)],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        // Pretty mode with a single number is the same as compact
        assert_eq!(inner.as_str(), Some("42.0"));
    }

    #[test]
    fn test_json_is_valid_true() {
        let module = create_json_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(
            &[ValueWord::from_string(Arc::new(
                r#"{"key": "value"}"#.to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_json_is_valid_false() {
        let module = create_json_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(
            &[ValueWord::from_string(Arc::new(
                "{not valid json".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_json_is_valid_requires_string() {
        let module = create_json_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_json_schemas() {
        let module = create_json_module();

        let parse_schema = module.get_schema("parse").unwrap();
        assert_eq!(parse_schema.params.len(), 1);
        assert_eq!(parse_schema.params[0].name, "text");
        assert!(parse_schema.params[0].required);
        assert_eq!(parse_schema.return_type.as_deref(), Some("Result<Json>"));

        let stringify_schema = module.get_schema("stringify").unwrap();
        assert_eq!(stringify_schema.params.len(), 2);
        assert!(stringify_schema.params[0].required);
        assert!(!stringify_schema.params[1].required);

        let is_valid_schema = module.get_schema("is_valid").unwrap();
        assert_eq!(is_valid_schema.params.len(), 1);
        assert_eq!(is_valid_schema.return_type.as_deref(), Some("bool"));
    }

    #[test]
    fn test_json_roundtrip_nested() {
        let module = create_json_module();
        let parse_fn = module.get_export("parse").unwrap();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();

        let json_str = r#"{"name":"test","values":[1,2,3],"active":true,"meta":null}"#;
        let parsed = parse_fn(
            &[ValueWord::from_string(Arc::new(json_str.to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = parsed.as_ok_inner().expect("should be Ok");

        let re_stringified = stringify_fn(&[inner.clone()], &ctx).unwrap();
        let re_str = re_stringified.as_ok_inner().expect("should be Ok");

        // Re-parse to verify round-trip validity
        let re_parsed = parse_fn(&[re_str.clone()], &ctx).unwrap();
        assert!(re_parsed.as_ok_inner().is_some());
    }

    /// Test json_value_to_enum produces TypedObjects with correct variant IDs.
    #[test]
    fn test_json_value_to_enum_variants() {
        use crate::type_schema::{EnumVariantInfo, TypeSchema};
        // Register Json enum schema
        let schema = TypeSchema::new_enum(
            "Json",
            vec![
                EnumVariantInfo::new("Null", 0, 0),
                EnumVariantInfo::new("Bool", 1, 1),
                EnumVariantInfo::new("Number", 2, 1),
                EnumVariantInfo::new("Str", 3, 1),
                EnumVariantInfo::new("Array", 4, 1),
                EnumVariantInfo::new("Object", 5, 1),
            ],
        );
        let sid = schema.id as u64;

        // Null
        let null_nb = json_value_to_enum(serde_json::Value::Null, sid);
        let (variant, _payload) = extract_enum_variant(&null_nb);
        assert_eq!(variant, 0, "Null should be variant 0");

        // Bool
        let bool_nb = json_value_to_enum(serde_json::Value::Bool(true), sid);
        let (variant, _payload) = extract_enum_variant(&bool_nb);
        assert_eq!(variant, 1, "Bool should be variant 1");

        // Number
        let num_nb = json_value_to_enum(serde_json::json!(42.5), sid);
        let (variant, _payload) = extract_enum_variant(&num_nb);
        assert_eq!(variant, 2, "Number should be variant 2");

        // String
        let str_nb = json_value_to_enum(serde_json::json!("hello"), sid);
        let (variant, _payload) = extract_enum_variant(&str_nb);
        assert_eq!(variant, 3, "Str should be variant 3");

        // Array
        let arr_nb = json_value_to_enum(serde_json::json!([1, 2, 3]), sid);
        let (variant, _payload) = extract_enum_variant(&arr_nb);
        assert_eq!(variant, 4, "Array should be variant 4");

        // Object
        let obj_nb = json_value_to_enum(serde_json::json!({"a": 1}), sid);
        let (variant, _payload) = extract_enum_variant(&obj_nb);
        assert_eq!(variant, 5, "Object should be variant 5");
    }

    /// Test that __parse_typed uses @alias annotations to map JSON keys to fields.
    #[test]
    fn test_parse_typed_with_alias() {
        use crate::type_schema::{FieldAnnotation, TypeSchemaBuilder};
        use shape_value::heap_value::HeapValue;

        let mut registry = crate::type_schema::TypeSchemaRegistry::new();
        let mut schema = TypeSchemaBuilder::new("Trade")
            .f64_field("close")
            .f64_field("volume")
            .build();

        // Add @alias annotations manually
        schema.fields[0].annotations.push(FieldAnnotation {
            name: "alias".to_string(),
            args: vec!["Close Price".to_string()],
        });
        schema.fields[1].annotations.push(FieldAnnotation {
            name: "alias".to_string(),
            args: vec!["vol.".to_string()],
        });
        let trade_id = schema.id;
        registry.register(schema);

        let module = create_json_module();
        let parse_typed_fn = module.get_export("__parse_typed").unwrap();
        let ctx = crate::module_exports::ModuleContext {
            schemas: &registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        };

        let text = ValueWord::from_string(Arc::new(
            r#"{"Close Price": 100.5, "vol.": 1000}"#.to_string(),
        ));
        let sid = ValueWord::from_f64(trade_id as f64);
        let result = parse_typed_fn(&[text, sid], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");

        // Verify it's a TypedObject with correct field values
        if let Some(HeapValue::TypedObject { slots, .. }) = inner.as_heap_ref() {
            // Field 0 ("close", aliased from "Close Price") should be 100.5
            let close_val = f64::from_bits(slots[0].raw());
            assert!(
                (close_val - 100.5).abs() < f64::EPSILON,
                "close field should be 100.5, got {}",
                close_val
            );
            // Field 1 ("volume", aliased from "vol.") should be 1000.0
            let volume_val = f64::from_bits(slots[1].raw());
            assert!(
                (volume_val - 1000.0).abs() < f64::EPSILON,
                "volume field should be 1000.0, got {}",
                volume_val
            );
        } else {
            panic!("expected TypedObject, got: {:?}", inner.type_name());
        }
    }

    /// Test that register_type_with_annotations propagates @alias to schema.
    #[test]
    fn test_register_type_with_annotations_alias() {
        use crate::type_schema::{FieldAnnotation, FieldType};

        let mut registry = crate::type_schema::TypeSchemaRegistry::new();
        let annotations = vec![
            vec![FieldAnnotation {
                name: "alias".to_string(),
                args: vec!["user_name".to_string()],
            }],
            vec![], // age has no annotations
        ];
        registry.register_type_with_annotations(
            "User",
            vec![
                ("name".to_string(), FieldType::String),
                ("age".to_string(), FieldType::I64),
            ],
            annotations,
        );

        let schema = registry.get("User").expect("schema should exist");
        assert_eq!(schema.fields[0].wire_name(), "user_name");
        assert_eq!(schema.fields[1].wire_name(), "age");
    }

    /// Test that @alias annotations enable JSON deserialization with wire names.
    #[test]
    fn test_parse_typed_alias_string_field() {
        use crate::type_schema::{FieldAnnotation, FieldType};
        use shape_value::heap_value::HeapValue;

        let mut registry = crate::type_schema::TypeSchemaRegistry::new();
        let annotations = vec![
            vec![FieldAnnotation {
                name: "alias".to_string(),
                args: vec!["user_name".to_string()],
            }],
            vec![],
        ];
        let schema_id = registry.register_type_with_annotations(
            "User",
            vec![
                ("name".to_string(), FieldType::String),
                ("age".to_string(), FieldType::I64),
            ],
            annotations,
        );

        let module = create_json_module();
        let parse_typed_fn = module.get_export("__parse_typed").unwrap();
        let ctx = crate::module_exports::ModuleContext {
            schemas: &registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        };

        // JSON uses the wire name "user_name" instead of the field name "name"
        let text = ValueWord::from_string(Arc::new(
            r#"{"user_name": "Bob", "age": 30}"#.to_string(),
        ));
        let sid = ValueWord::from_f64(schema_id as f64);
        let result = parse_typed_fn(&[text, sid], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");

        // Verify it's a TypedObject and the name field was populated from the aliased key
        if let Some(HeapValue::TypedObject { slots, .. }) = inner.as_heap_ref() {
            // Field 0 ("name") should be a heap string "Bob"
            let name_nb = slots[0].as_heap_nb();
            assert_eq!(name_nb.as_str(), Some("Bob"), "name field should be 'Bob'");
            // Field 1 ("age") should be 30
            let age_val = slots[1].as_i64();
            assert_eq!(age_val, 30, "age field should be 30");
        } else {
            panic!("expected TypedObject, got: {:?}", inner.type_name());
        }
    }

    /// Test that without @alias, field name is used as wire name.
    #[test]
    fn test_parse_typed_no_alias_uses_field_name() {
        use crate::type_schema::FieldType;
        use shape_value::heap_value::HeapValue;

        let mut registry = crate::type_schema::TypeSchemaRegistry::new();
        let schema_id = registry.register_type(
            "Simple",
            vec![
                ("name".to_string(), FieldType::String),
                ("value".to_string(), FieldType::F64),
            ],
        );

        let module = create_json_module();
        let parse_typed_fn = module.get_export("__parse_typed").unwrap();
        let ctx = crate::module_exports::ModuleContext {
            schemas: &registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        };

        let text = ValueWord::from_string(Arc::new(
            r#"{"name": "test", "value": 42.5}"#.to_string(),
        ));
        let sid = ValueWord::from_f64(schema_id as f64);
        let result = parse_typed_fn(&[text, sid], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");

        if let Some(HeapValue::TypedObject { slots, .. }) = inner.as_heap_ref() {
            let name_nb = slots[0].as_heap_nb();
            assert_eq!(name_nb.as_str(), Some("test"));
            let value_val = f64::from_bits(slots[1].raw());
            assert!((value_val - 42.5).abs() < f64::EPSILON);
        } else {
            panic!("expected TypedObject");
        }
    }

    /// Extract variant_id from a Json enum TypedObject.
    fn extract_enum_variant(nb: &ValueWord) -> (i64, Option<ValueWord>) {
        use shape_value::heap_value::HeapValue;
        if let Some(HeapValue::TypedObject {
            slots, heap_mask, ..
        }) = nb.as_heap_ref()
        {
            let variant_id = slots[0].as_i64();
            let payload = if slots.len() > 1 {
                // Only dereference as heap pointer if the heap_mask says slot 1 is a pointer
                if heap_mask & (1u64 << 1) != 0 {
                    Some(slots[1].as_heap_nb())
                } else if slots[1].raw() == 0 && variant_id == 0 {
                    // Null variant has no payload
                    None
                } else {
                    // Non-heap payload (inline ValueWord) — reconstruct from raw bits
                    // Safety: bits were stored by nb_to_slot from a valid inline ValueWord.
                    Some(unsafe { ValueWord::clone_from_bits(slots[1].raw()) })
                }
            } else {
                None
            };
            (variant_id, payload)
        } else {
            panic!("expected TypedObject, got: {:?}", nb.type_name())
        }
    }
}
