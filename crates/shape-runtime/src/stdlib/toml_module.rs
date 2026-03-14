//! Native `toml` module for TOML parsing and serialization.
//!
//! Exports: toml.parse(text), toml.stringify(value), toml.is_valid(text)

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::ValueWord;
use std::sync::Arc;

/// Convert a `toml::Value` into a `ValueWord`.
fn toml_value_to_nanboxed(value: toml::Value) -> ValueWord {
    match value {
        toml::Value::Boolean(b) => ValueWord::from_bool(b),
        toml::Value::Integer(n) => ValueWord::from_i64(n),
        toml::Value::Float(f) => ValueWord::from_f64(f),
        toml::Value::String(s) => ValueWord::from_string(Arc::new(s)),
        toml::Value::Datetime(dt) => ValueWord::from_string(Arc::new(dt.to_string())),
        toml::Value::Array(arr) => {
            let items: Vec<ValueWord> = arr.into_iter().map(toml_value_to_nanboxed).collect();
            ValueWord::from_array(Arc::new(items))
        }
        toml::Value::Table(map) => {
            let mut keys = Vec::with_capacity(map.len());
            let mut values = Vec::with_capacity(map.len());
            for (k, v) in map.into_iter() {
                keys.push(ValueWord::from_string(Arc::new(k)));
                values.push(toml_value_to_nanboxed(v));
            }
            ValueWord::from_hashmap_pairs(keys, values)
        }
    }
}

/// Convert a `ValueWord` into a `toml::Value` for serialization.
fn nanboxed_to_toml_value(nb: &ValueWord) -> toml::Value {
    use shape_value::heap_value::HeapValue;

    if nb.is_none() {
        return toml::Value::String("null".to_string());
    }
    if let Some(b) = nb.as_bool() {
        return toml::Value::Boolean(b);
    }
    if let Some(n) = nb.as_i64() {
        return toml::Value::Integer(n);
    }
    if let Some(f) = nb.as_f64() {
        return toml::Value::Float(f);
    }
    if let Some(s) = nb.as_str() {
        return toml::Value::String(s.to_string());
    }
    if let Some(arr) = nb.as_any_array() {
        let items: Vec<toml::Value> = arr
            .to_generic()
            .iter()
            .map(nanboxed_to_toml_value)
            .collect();
        return toml::Value::Array(items);
    }
    if let Some((keys, values, _index)) = nb.as_hashmap() {
        let mut map = toml::map::Map::new();
        for (k, v) in keys.iter().zip(values.iter()) {
            if let Some(key) = k.as_str() {
                map.insert(key.to_string(), nanboxed_to_toml_value(v));
            }
        }
        return toml::Value::Table(map);
    }
    // TypedObject — convert via field extraction
    if let Some(heap) = nb.as_heap_ref() {
        if let HeapValue::TypedObject { slots, .. } = heap {
            // Fall back to string representation for complex types
            let _ = slots;
            return toml::Value::String(format!("{}", nb));
        }
    }
    toml::Value::String(format!("{}", nb))
}

/// Create the `toml` module with TOML parsing and serialization functions.
pub fn create_toml_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::toml");
    module.description = "TOML parsing and serialization".to_string();

    // toml.parse(text: string) -> Result<HashMap>
    module.add_function_with_schema(
        "parse",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "toml.parse() requires a string argument".to_string())?;

            let parsed: toml::Value =
                toml::from_str(text).map_err(|e| format!("toml.parse() failed: {}", e))?;

            let result = toml_value_to_nanboxed(parsed);
            Ok(ValueWord::from_ok(result))
        },
        ModuleFunction {
            description: "Parse a TOML string into Shape values".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "TOML string to parse".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<HashMap>".to_string()),
        },
    );

    // toml.stringify(value: any) -> Result<string>
    module.add_function_with_schema(
        "stringify",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let value = args
                .first()
                .ok_or_else(|| "toml.stringify() requires a value argument".to_string())?;

            let toml_value = nanboxed_to_toml_value(value);
            let output = toml::to_string(&toml_value)
                .map_err(|e| format!("toml.stringify() failed: {}", e))?;

            Ok(ValueWord::from_ok(ValueWord::from_string(Arc::new(output))))
        },
        ModuleFunction {
            description: "Serialize a Shape value to a TOML string".to_string(),
            params: vec![ModuleParam {
                name: "value".to_string(),
                type_name: "any".to_string(),
                required: true,
                description: "Value to serialize".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<string>".to_string()),
        },
    );

    // toml.is_valid(text: string) -> bool
    module.add_function_with_schema(
        "is_valid",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "toml.is_valid() requires a string argument".to_string())?;

            let valid = toml::from_str::<toml::Value>(text).is_ok();
            Ok(ValueWord::from_bool(valid))
        },
        ModuleFunction {
            description: "Check if a string is valid TOML".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "String to validate as TOML".to_string(),
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
    fn test_toml_module_creation() {
        let module = create_toml_module();
        assert_eq!(module.name, "std::core::toml");
        assert!(module.has_export("parse"));
        assert!(module.has_export("stringify"));
        assert!(module.has_export("is_valid"));
    }

    #[test]
    fn test_toml_parse_simple_table() {
        let module = create_toml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(
            r#"
[server]
host = "localhost"
port = 8080
"#
            .to_string(),
        ));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, _values, _index) = inner.as_hashmap().expect("should be hashmap");
        assert_eq!(keys.len(), 1); // "server" key
    }

    #[test]
    fn test_toml_parse_basic_types() {
        let module = create_toml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(
            r#"
name = "test"
version = 42
pi = 3.14
active = true
"#
            .to_string(),
        ));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert!(inner.as_hashmap().is_some());
    }

    #[test]
    fn test_toml_parse_array() {
        let module = create_toml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(r#"values = [1, 2, 3]"#.to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert!(inner.as_hashmap().is_some());
    }

    #[test]
    fn test_toml_parse_invalid() {
        let module = create_toml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("= invalid toml [".to_string()));
        let result = parse_fn(&[input], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_toml_parse_requires_string() {
        let module = create_toml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let result = parse_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_toml_stringify_table() {
        let module = create_toml_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let keys = vec![ValueWord::from_string(Arc::new("name".to_string()))];
        let values = vec![ValueWord::from_string(Arc::new("test".to_string()))];
        let hm = ValueWord::from_hashmap_pairs(keys, values);
        let result = stringify_fn(&[hm], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let s = inner.as_str().expect("should be string");
        assert!(s.contains("name"));
        assert!(s.contains("test"));
    }

    #[test]
    fn test_toml_is_valid_true() {
        let module = create_toml_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(
            &[ValueWord::from_string(Arc::new(
                r#"key = "value""#.to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_toml_is_valid_false() {
        let module = create_toml_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(
            &[ValueWord::from_string(Arc::new(
                "= not valid toml".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_toml_roundtrip() {
        let module = create_toml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();

        let toml_str = r#"name = "test"
version = 42
"#;
        let parsed = parse_fn(
            &[ValueWord::from_string(Arc::new(toml_str.to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = parsed.as_ok_inner().expect("should be Ok");
        let re_stringified = stringify_fn(&[inner.clone()], &ctx).unwrap();
        let re_str = re_stringified.as_ok_inner().expect("should be Ok");
        assert!(re_str.as_str().is_some());
    }

    #[test]
    fn test_toml_schemas() {
        let module = create_toml_module();

        let parse_schema = module.get_schema("parse").unwrap();
        assert_eq!(parse_schema.params.len(), 1);
        assert_eq!(parse_schema.params[0].name, "text");
        assert!(parse_schema.params[0].required);
        assert_eq!(parse_schema.return_type.as_deref(), Some("Result<HashMap>"));

        let stringify_schema = module.get_schema("stringify").unwrap();
        assert_eq!(stringify_schema.params.len(), 1);
        assert!(stringify_schema.params[0].required);

        let is_valid_schema = module.get_schema("is_valid").unwrap();
        assert_eq!(is_valid_schema.params.len(), 1);
        assert_eq!(is_valid_schema.return_type.as_deref(), Some("bool"));
    }
}
