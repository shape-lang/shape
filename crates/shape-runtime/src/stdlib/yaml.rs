//! Native `yaml` module for YAML parsing and serialization.
//!
//! Exports: yaml.parse(text), yaml.parse_all(text), yaml.stringify(value), yaml.is_valid(text)
//!
//! WF-2E (2026-07-05): the pre-marshal-core "pending N4/N6" stubs are
//! REMOVED. `yaml.parse` / `yaml.parse_all` / `yaml.stringify` are now real
//! serializers built on the shared object-graph marshal
//! (`crate::json_value`):
//!
//! - `yaml.parse(text) -> Result<Json>` decodes the YAML text into a
//!   `serde_json::Value` (via `serde_yaml::from_str`), funnels it through
//!   the shared `serde_json_to_json_value` wire→intermediate step, and
//!   returns the strict-typed `Json` enum (`ConcreteReturn::JsonValue`).
//!   Same shape/navigation surface as `json.parse` (`.get(key)` /
//!   `.at(index)` / pattern matching). YAML `null` maps to `Json::Null`.
//! - `yaml.parse_all(text) -> Result<Json>` walks the multi-document YAML
//!   stream (documents separated by `---`) via
//!   `serde_yaml::Deserializer::from_str`, decoding each document to a
//!   `JsonValue` and collecting them into a `Json::Array`. `result.at(i)`
//!   yields document `i`; `result.len()` yields the document count.
//! - `yaml.stringify(value) -> Result<string>` receives the argument fully
//!   walked into a `JsonValue` tree at the marshal boundary
//!   (`FromSlot<JsonValue>` → `slot_to_json_value`, dispatching on the
//!   STAMPED `NativeKind` — no pointer reinterpretation), converts it to a
//!   `serde_yaml::Value` via `json_value_to_serde_yaml`, and renders with
//!   `serde_yaml::to_string`.
//! - `yaml.is_valid(text)` uses the typed `Arc<String>` marshal directly.

use crate::json_value::JsonValue;
use crate::marshal::{register_typed_fn_1, register_typed_fn_1_full};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use serde::Deserialize;
use std::sync::Arc;

/// Create the `yaml` module with YAML parsing and serialization functions.
pub fn create_yaml_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::yaml");
    module.description = "YAML parsing and serialization".to_string();

    // yaml.parse(text: string) -> Result<Json>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse",
        "Parse a YAML string into Shape values",
        "text",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::JsonValue("Json".to_string()))),
        |text: Arc<String>, _ctx| {
            let parsed: serde_json::Value = serde_yaml::from_str(text.as_str())
                .map_err(|e| format!("yaml.parse() failed: {}", e))?;
            let result = crate::json_value::serde_json_to_json_value(parsed);
            Ok(TypedReturn::Ok(ConcreteReturn::JsonValue(result)))
        },
    );

    // yaml.parse_all(text: string) -> Result<Json>
    //
    // Multi-document YAML: each `---`-separated document is decoded to a
    // `JsonValue` and collected into a single `Json::Array`. Consumers use
    // `result.at(i)` / `result.len()` (the `Json` enum navigation surface).
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse_all",
        "Parse a multi-document YAML string into an array of Shape values",
        "text",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::JsonValue("Json".to_string()))),
        |text: Arc<String>, _ctx| {
            let mut docs: Vec<JsonValue> = Vec::new();
            for de in serde_yaml::Deserializer::from_str(text.as_str()) {
                let parsed = serde_json::Value::deserialize(de)
                    .map_err(|e| format!("yaml.parse_all() failed: {}", e))?;
                docs.push(crate::json_value::serde_json_to_json_value(parsed));
            }
            Ok(TypedReturn::Ok(ConcreteReturn::JsonValue(
                JsonValue::Array(docs),
            )))
        },
    );

    // yaml.stringify(value) -> Result<string>
    //
    // WF-2E: the `value` argument arrives fully walked into a `JsonValue`
    // tree at the marshal boundary (`FromSlot<JsonValue>` →
    // `slot_to_json_value`, dispatching on the stamped `NativeKind`),
    // converted to a `serde_yaml::Value` and rendered via
    // `serde_yaml::to_string`.
    register_typed_fn_1_full::<_, crate::marshal::PolymorphicArg>(
        &mut module,
        "stringify",
        "Serialize Shape values to a YAML string",
        [ModuleParam {
            name: "value".to_string(),
            type_name: "any".to_string(),
            required: true,
            description: "Value to serialize".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |value: crate::marshal::PolymorphicArg, ctx| {
            // WF-2E: walk via the EXECUTION registry (`ctx.schemas`) — see the
            // xml.stringify note; the ambient `None` path diverges VM↔JIT.
            let value: JsonValue = value.to_json_value(ctx.schemas)?;
            let yaml_value = crate::json_value::json_value_to_serde_yaml(&value);
            let out = serde_yaml::to_string(&yaml_value)
                .map_err(|e| format!("yaml.stringify() serialization failed: {}", e))?;
            Ok(TypedReturn::Ok(ConcreteReturn::String(out)))
        },
    );

    // yaml.is_valid(text: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "is_valid",
        "Check if a string is valid YAML",
        "text",
        "string",
        ConcreteType::Bool,
        |text: Arc<String>, _ctx| {
            let valid = serde_yaml::from_str::<serde_yaml::Value>(text.as_str()).is_ok();
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(valid)))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_yaml_typed_registry_populated() {
        let module = create_yaml_module();
        let typed = module.typed_exports();
        assert!(typed.get("parse").is_some());
        assert!(typed.get("parse_all").is_some());
        assert!(typed.get("stringify").is_some());
        assert!(typed.get("is_valid").is_some());
    }

    /// WF-2E: `yaml.parse` decodes a YAML mapping into a `JsonValue::Object`
    /// via the shared wire→intermediate path.
    #[test]
    fn test_yaml_parse_mapping() {
        let parsed: serde_json::Value =
            serde_yaml::from_str("server:\n  host: localhost\n  port: 8080\n").expect("yaml parse");
        let jv = crate::json_value::serde_json_to_json_value(parsed);
        match &jv {
            JsonValue::Object(pairs) => {
                assert_eq!(pairs[0].0, "server");
                match &pairs[0].1 {
                    JsonValue::Object(inner) => {
                        assert!(inner.iter().any(|(k, v)| k == "host"
                            && *v == JsonValue::String("localhost".to_string())));
                    }
                    other => panic!("expected nested object, got {:?}", other),
                }
            }
            other => panic!("expected object, got {:?}", other),
        }
    }

    /// WF-2E: `yaml.parse_all` collects each `---`-separated document into a
    /// `Json::Array`.
    #[test]
    fn test_yaml_parse_all_multidoc() {
        let text = "---\nname: doc1\n---\nname: doc2\n---\nname: doc3\n";
        let mut docs: Vec<JsonValue> = Vec::new();
        for de in serde_yaml::Deserializer::from_str(text) {
            let parsed = serde_json::Value::deserialize(de).expect("yaml doc parse");
            docs.push(crate::json_value::serde_json_to_json_value(parsed));
        }
        assert_eq!(docs.len(), 3);
    }

    /// WF-2E: `yaml.stringify` round-trips a `JsonValue::Object` to a YAML
    /// string via the shared `json_value_to_serde_yaml` encoder.
    #[test]
    fn test_yaml_stringify_object() {
        let value = JsonValue::Object(vec![
            (
                "name".to_string(),
                JsonValue::String("my-project".to_string()),
            ),
            ("version".to_string(), JsonValue::Int(42)),
            ("active".to_string(), JsonValue::Bool(true)),
        ]);
        let yaml_value = crate::json_value::json_value_to_serde_yaml(&value);
        let out = serde_yaml::to_string(&yaml_value).expect("yaml serialize");
        assert!(out.contains("name: my-project"));
        assert!(out.contains("version: 42"));
        assert!(out.contains("active: true"));
    }
}
