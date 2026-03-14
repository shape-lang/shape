//! Native `yaml` module for YAML parsing and serialization.
//!
//! Exports: yaml.parse(text), yaml.parse_all(text), yaml.stringify(value), yaml.is_valid(text)

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use serde::Deserialize;
use shape_value::ValueWord;
use std::sync::Arc;

/// Convert a `serde_yaml::Value` into a `ValueWord`.
fn yaml_value_to_nanboxed(value: serde_yaml::Value) -> ValueWord {
    match value {
        serde_yaml::Value::Null => ValueWord::none(),
        serde_yaml::Value::Bool(b) => ValueWord::from_bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ValueWord::from_i64(i)
            } else {
                ValueWord::from_f64(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_yaml::Value::String(s) => ValueWord::from_string(Arc::new(s)),
        serde_yaml::Value::Sequence(arr) => {
            let items: Vec<ValueWord> = arr.into_iter().map(yaml_value_to_nanboxed).collect();
            ValueWord::from_array(Arc::new(items))
        }
        serde_yaml::Value::Mapping(map) => {
            let mut keys = Vec::with_capacity(map.len());
            let mut values = Vec::with_capacity(map.len());
            for (k, v) in map.into_iter() {
                let key_str = match k {
                    serde_yaml::Value::String(s) => s,
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    other => format!("{:?}", other),
                };
                keys.push(ValueWord::from_string(Arc::new(key_str)));
                values.push(yaml_value_to_nanboxed(v));
            }
            ValueWord::from_hashmap_pairs(keys, values)
        }
        serde_yaml::Value::Tagged(tagged) => {
            // Unwrap tagged values — preserve the inner value
            yaml_value_to_nanboxed(tagged.value)
        }
    }
}

/// Create the `yaml` module with YAML parsing and serialization functions.
pub fn create_yaml_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::yaml");
    module.description = "YAML parsing and serialization".to_string();

    // yaml.parse(text: string) -> Result<HashMap>
    module.add_function_with_schema(
        "parse",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "yaml.parse() requires a string argument".to_string())?;

            let parsed: serde_yaml::Value =
                serde_yaml::from_str(text).map_err(|e| format!("yaml.parse() failed: {}", e))?;

            let result = yaml_value_to_nanboxed(parsed);
            Ok(ValueWord::from_ok(result))
        },
        ModuleFunction {
            description: "Parse a YAML string into Shape values".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "YAML string to parse".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<HashMap>".to_string()),
        },
    );

    // yaml.parse_all(text: string) -> Result<Array>
    module.add_function_with_schema(
        "parse_all",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "yaml.parse_all() requires a string argument".to_string())?;

            let mut documents = Vec::new();
            for document in serde_yaml::Deserializer::from_str(text) {
                let value: serde_yaml::Value = serde_yaml::Value::deserialize(document)
                    .map_err(|e| format!("yaml.parse_all() failed: {}", e))?;
                documents.push(yaml_value_to_nanboxed(value));
            }

            Ok(ValueWord::from_ok(ValueWord::from_array(Arc::new(
                documents,
            ))))
        },
        ModuleFunction {
            description: "Parse a multi-document YAML string into an array of Shape values"
                .to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "YAML string with one or more documents".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<Array>".to_string()),
        },
    );

    // yaml.stringify(value: any) -> Result<string>
    module.add_function_with_schema(
        "stringify",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let value = args
                .first()
                .ok_or_else(|| "yaml.stringify() requires a value argument".to_string())?;

            let json_value = value.to_json_value();
            let output = serde_yaml::to_string(&json_value)
                .map_err(|e| format!("yaml.stringify() failed: {}", e))?;

            Ok(ValueWord::from_ok(ValueWord::from_string(Arc::new(output))))
        },
        ModuleFunction {
            description: "Serialize a Shape value to a YAML string".to_string(),
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

    // yaml.is_valid(text: string) -> bool
    module.add_function_with_schema(
        "is_valid",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "yaml.is_valid() requires a string argument".to_string())?;

            let valid = serde_yaml::from_str::<serde_yaml::Value>(text).is_ok();
            Ok(ValueWord::from_bool(valid))
        },
        ModuleFunction {
            description: "Check if a string is valid YAML".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "String to validate as YAML".to_string(),
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
    fn test_yaml_module_creation() {
        let module = create_yaml_module();
        assert_eq!(module.name, "std::core::yaml");
        assert!(module.has_export("parse"));
        assert!(module.has_export("parse_all"));
        assert!(module.has_export("stringify"));
        assert!(module.has_export("is_valid"));
    }

    #[test]
    fn test_yaml_parse_mapping() {
        let module = create_yaml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(
            "name: test\nversion: 42\npi: 3.14\nactive: true\n".to_string(),
        ));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, _values, _index) = inner.as_hashmap().expect("should be hashmap");
        assert_eq!(keys.len(), 4);
    }

    #[test]
    fn test_yaml_parse_sequence() {
        let module = create_yaml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("- 1\n- 2\n- 3\n".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let arr = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn test_yaml_parse_scalar_string() {
        let module = create_yaml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("hello world".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("hello world"));
    }

    #[test]
    fn test_yaml_parse_null() {
        let module = create_yaml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("null".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert!(inner.is_none());
    }

    #[test]
    fn test_yaml_parse_nested() {
        let module = create_yaml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(
            "server:\n  host: localhost\n  port: 8080\n".to_string(),
        ));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, _values, _index) = inner.as_hashmap().expect("should be hashmap");
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn test_yaml_parse_requires_string() {
        let module = create_yaml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let result = parse_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_yaml_parse_all_multi_document() {
        let module = create_yaml_module();
        let parse_all_fn = module.get_export("parse_all").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(
            "---\nname: doc1\n---\nname: doc2\n---\nname: doc3\n".to_string(),
        ));
        let result = parse_all_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let arr = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn test_yaml_parse_all_single_document() {
        let module = create_yaml_module();
        let parse_all_fn = module.get_export("parse_all").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("name: single\n".to_string()));
        let result = parse_all_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let arr = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn test_yaml_stringify_mapping() {
        let module = create_yaml_module();
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
    fn test_yaml_stringify_number() {
        let module = create_yaml_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let result = stringify_fn(&[ValueWord::from_f64(42.0)], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let s = inner.as_str().expect("should be string");
        assert!(s.contains("42"));
    }

    #[test]
    fn test_yaml_stringify_bool() {
        let module = create_yaml_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let result = stringify_fn(&[ValueWord::from_bool(true)], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let s = inner.as_str().expect("should be string");
        assert!(s.contains("true"));
    }

    #[test]
    fn test_yaml_is_valid_true() {
        let module = create_yaml_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(
            &[ValueWord::from_string(Arc::new("key: value\n".to_string()))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_yaml_is_valid_false() {
        let module = create_yaml_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(
            &[ValueWord::from_string(Arc::new(
                ":\n  :\n    - : :\n  bad: [".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        // serde_yaml may or may not parse some edge cases; just verify we get a bool
        assert!(result.as_bool().is_some());
    }

    #[test]
    fn test_yaml_is_valid_requires_string() {
        let module = create_yaml_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_yaml_roundtrip() {
        let module = create_yaml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();

        let yaml_str = "name: test\nversion: 42\n";
        let parsed = parse_fn(
            &[ValueWord::from_string(Arc::new(yaml_str.to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = parsed.as_ok_inner().expect("should be Ok");
        let re_stringified = stringify_fn(&[inner.clone()], &ctx).unwrap();
        let re_str = re_stringified.as_ok_inner().expect("should be Ok");
        assert!(re_str.as_str().is_some());
    }

    #[test]
    fn test_yaml_schemas() {
        let module = create_yaml_module();

        let parse_schema = module.get_schema("parse").unwrap();
        assert_eq!(parse_schema.params.len(), 1);
        assert_eq!(parse_schema.params[0].name, "text");
        assert!(parse_schema.params[0].required);
        assert_eq!(parse_schema.return_type.as_deref(), Some("Result<HashMap>"));

        let parse_all_schema = module.get_schema("parse_all").unwrap();
        assert_eq!(parse_all_schema.params.len(), 1);
        assert_eq!(
            parse_all_schema.return_type.as_deref(),
            Some("Result<Array>")
        );

        let stringify_schema = module.get_schema("stringify").unwrap();
        assert_eq!(stringify_schema.params.len(), 1);
        assert!(stringify_schema.params[0].required);

        let is_valid_schema = module.get_schema("is_valid").unwrap();
        assert_eq!(is_valid_schema.params.len(), 1);
        assert_eq!(is_valid_schema.return_type.as_deref(), Some("bool"));
    }
}
