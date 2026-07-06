//! Native `toml` module for TOML parsing and serialization.
//!
//! Exports: toml.parse(text), toml.stringify(value), toml.is_valid(text)
//!
//! WF-2E (2026-07-05): the pre-marshal-core "pending N4/N6" stubs are
//! REMOVED. `toml.parse` / `toml.stringify` are now real serializers built
//! on the shared object-graph marshal (`crate::json_value`):
//!
//! - `toml.parse(text) -> Result<Json>` decodes the TOML text into a
//!   `serde_json::Value` (via `toml::from_str`), funnels it through the
//!   shared `serde_json_to_json_value` wire→intermediate step, and returns
//!   the strict-typed `Json` enum (`ConcreteReturn::JsonValue`). Same shape
//!   and navigation surface as `json.parse` (`.get(key)` / `.at(index)` /
//!   pattern matching).
//! - `toml.stringify(value) -> Result<string>` receives the argument fully
//!   walked into a `JsonValue` tree at the marshal boundary
//!   (`FromSlot<JsonValue>` → `slot_to_json_value`, dispatching on the
//!   STAMPED `NativeKind` — no pointer reinterpretation), converts it to a
//!   `toml::Value` via `json_value_to_toml_value`, and renders with
//!   `toml::to_string`. TOML requires a Table at the document root, so a
//!   non-object top-level value is surfaced as a clean `Err`.
//! - `toml.is_valid(text)` uses the typed `Arc<String>` marshal directly.

use crate::json_value::JsonValue;
use crate::marshal::{register_typed_fn_1, register_typed_fn_1_full};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;

/// Create the `toml` module with TOML parsing and serialization functions.
pub fn create_toml_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::toml");
    module.description = "TOML parsing and serialization".to_string();

    // toml.parse(text: string) -> Result<Json>
    //
    // WF-2E: decode TOML → `serde_json::Value` → shared `JsonValue`
    // intermediate → `Json` enum. Mirrors `json.parse` exactly; TOML
    // tables become `Json::Object`, arrays `Json::Array`, and scalars map
    // to `Json::Str` / `Json::Int` / `Json::Number` / `Json::Bool`.
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse",
        "Parse a TOML string into Shape values",
        "text",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::JsonValue("Json".to_string()))),
        |text: Arc<String>, _ctx| {
            let parsed: serde_json::Value = toml::from_str(text.as_str())
                .map_err(|e| format!("toml.parse() failed: {}", e))?;
            let result = crate::json_value::serde_json_to_json_value(parsed);
            Ok(TypedReturn::Ok(ConcreteReturn::JsonValue(result)))
        },
    );

    // toml.stringify(value) -> Result<string>
    //
    // WF-2E: the `value` argument arrives fully walked into a `JsonValue`
    // tree at the marshal boundary (`FromSlot<JsonValue>` →
    // `slot_to_json_value`, dispatching on the stamped `NativeKind`). This
    // replaces the SIGSEGV-prone `Vec<(Arc<String>, Arc<HeapValue>)>`
    // polymorphic-arg shape. TOML's document root must be a Table, so a
    // non-object top-level value is surfaced as a clean `Err`.
    register_typed_fn_1_full::<_, crate::marshal::PolymorphicArg>(
        &mut module,
        "stringify",
        "Serialize Shape values to a TOML string",
        [ModuleParam {
            name: "value".to_string(),
            type_name: "any".to_string(),
            required: true,
            description: "Value to serialize (must be a table/object at the root)".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |value: crate::marshal::PolymorphicArg, ctx| {
            // WF-2E: walk via the EXECUTION registry (`ctx.schemas`) — see the
            // xml.stringify note; the ambient `None` path diverges VM↔JIT.
            let value: JsonValue = value.to_json_value(ctx.schemas)?;
            let toml_value = crate::json_value::json_value_to_toml_value(&value);
            match &toml_value {
                toml::Value::Table(_) => {}
                other => {
                    return Err(format!(
                        "toml.stringify() requires a table/object at the root, got a {} value",
                        toml_type_name(other)
                    ));
                }
            }
            let out = toml::to_string(&toml_value)
                .map_err(|e| format!("toml.stringify() serialization failed: {}", e))?;
            Ok(TypedReturn::Ok(ConcreteReturn::String(out)))
        },
    );

    // toml.is_valid(text: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "is_valid",
        "Check if a string is valid TOML",
        "text",
        "string",
        ConcreteType::Bool,
        |text: Arc<String>, _ctx| {
            let valid = toml::from_str::<toml::Value>(text.as_str()).is_ok();
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(valid)))
        },
    );

    module
}

/// Human-readable TOML value type name for error messages.
fn toml_type_name(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toml_module_creation() {
        let module = create_toml_module();
        assert_eq!(module.name, "std::core::toml");
        assert!(module.has_export("parse"));
        assert!(module.has_export("stringify"));
        assert!(module.has_export("is_valid"));
    }

    #[test]
    fn test_toml_typed_registry_populated() {
        let module = create_toml_module();
        let typed = module.typed_exports();
        assert!(typed.get("parse").is_some());
        assert!(typed.get("stringify").is_some());
        assert!(typed.get("is_valid").is_some());
    }

    /// WF-2E: `toml.stringify` round-trips a `JsonValue::Object` through the
    /// shared `json_value_to_toml_value` encoder to a TOML string.
    #[test]
    fn test_toml_stringify_object_roundtrip() {
        let value = JsonValue::Object(vec![
            ("name".to_string(), JsonValue::String("my-project".to_string())),
            ("version".to_string(), JsonValue::Int(1)),
        ]);
        let toml_value = crate::json_value::json_value_to_toml_value(&value);
        let out = toml::to_string(&toml_value).expect("toml serialize");
        assert!(out.contains("name = \"my-project\""));
        assert!(out.contains("version = 1"));
    }

    /// WF-2E: `toml.parse` decodes a TOML table into a `JsonValue::Object`
    /// via the shared wire→intermediate path.
    #[test]
    fn test_toml_parse_table() {
        let parsed: serde_json::Value =
            toml::from_str("[server]\nhost = \"localhost\"\nport = 8080\n").expect("toml parse");
        let jv = crate::json_value::serde_json_to_json_value(parsed);
        match &jv {
            JsonValue::Object(pairs) => {
                assert_eq!(pairs.len(), 1);
                assert_eq!(pairs[0].0, "server");
                match &pairs[0].1 {
                    JsonValue::Object(inner) => {
                        assert!(inner.iter().any(|(k, v)| k == "host"
                            && *v == JsonValue::String("localhost".to_string())));
                        assert!(inner
                            .iter()
                            .any(|(k, v)| k == "port" && *v == JsonValue::Int(8080)));
                    }
                    other => panic!("expected nested object, got {:?}", other),
                }
            }
            other => panic!("expected object, got {:?}", other),
        }
    }
}
