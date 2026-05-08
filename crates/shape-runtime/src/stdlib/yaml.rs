//! Native `yaml` module for YAML parsing and serialization.
//!
//! Exports: yaml.parse(text), yaml.parse_all(text), yaml.stringify(value), yaml.is_valid(text)
//!
//! Phase 1.B (ADR-006 §2.7.4) status: yaml.parse / yaml.parse_all /
//! yaml.stringify REMAIN DEFERRED pending the **N4** (any-input typed
//! marshal) and **N6** (any-output typed marshal) architectural
//! decisions per `docs/defections.md` HashMap-marshal cluster's
//! sub-decision queue.
//!
//! - `yaml.parse(text) -> Result<any>` and
//!   `yaml.parse_all(text) -> Result<Array<any>>` return polymorphic
//!   recursive `serde_yaml::Value`-equivalent trees. Mapping to N6.
//! - `yaml.stringify(value: any)` takes a polymorphic `value: any`
//!   input parameter that maps to N4.
//!
//! `yaml.is_valid(text)` is migratable standalone.
//!
//! Until N4 + N6 land, the bodies use the variadic
//! [`register_typed_function`] shape (per ADR-006 §2.7.4 ruling) and
//! return `Err(...)` for the deferred halves; `is_valid` is fully
//! functional.

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_function,
};
use shape_value::KindedSlot;
use std::sync::Arc;

/// Read a [`KindedSlot`]'s bits as an `Arc<String>` payload. See
/// `stdlib/json.rs` for the same Phase 1.B variadic shim.
fn slot_as_string(slot: &KindedSlot) -> Option<Arc<String>> {
    let bits = slot.slot().raw();
    if bits == 0 {
        return None;
    }
    unsafe {
        let arc = Arc::<String>::from_raw(bits as *const String);
        let cloned = arc.clone();
        std::mem::forget(arc);
        Some(cloned)
    }
}

/// Create the `yaml` module with YAML parsing and serialization functions.
pub fn create_yaml_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::yaml");
    module.description = "YAML parsing and serialization".to_string();

    // yaml.parse(text: string) -> Result<HashMap>
    register_typed_function(
        &mut module,
        "parse",
        "Parse a YAML string into Shape values",
        vec![ModuleParam {
            name: "text".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "YAML string to parse".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::HashMap)),
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "yaml.parse() pending N6 (any-output marshal) — see ADR-006 §2.7.4".to_string(),
            )))
        },
    );

    // yaml.parse_all(text: string) -> Result<Array>
    register_typed_function(
        &mut module,
        "parse_all",
        "Parse a multi-document YAML string into an array of Shape values",
        vec![ModuleParam {
            name: "text".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "YAML string with one or more documents".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::Array)),
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "yaml.parse_all() pending N6 (any-output marshal) — see ADR-006 §2.7.4"
                    .to_string(),
            )))
        },
    );

    // yaml.stringify(value: any) -> Result<string>
    register_typed_function(
        &mut module,
        "stringify",
        "Serialize Shape values to a YAML string",
        vec![ModuleParam {
            name: "value".to_string(),
            type_name: "any".to_string(),
            required: true,
            description: "Value to serialize".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "yaml.stringify() pending N4 (any-input marshal) — see ADR-006 §2.7.4"
                    .to_string(),
            )))
        },
    );

    // yaml.is_valid(text: string) -> bool
    register_typed_function(
        &mut module,
        "is_valid",
        "Check if a string is valid YAML",
        vec![ModuleParam {
            name: "text".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "String to validate as YAML".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        |args, _ctx| {
            let slot = args
                .first()
                .ok_or_else(|| "yaml.is_valid() requires a string argument".to_string())?;
            let text = slot_as_string(slot)
                .ok_or_else(|| "yaml.is_valid() requires a string argument".to_string())?;
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

    // Behavioural roundtrip tests deleted alongside `module.invoke_export()`
    // and the deleted `ValueWord` constructors. They return when N4 + N6
    // land and the bodies become real serializers.
}
