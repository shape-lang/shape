//! Native `toml` module for TOML parsing and serialization.
//!
//! Exports: toml.parse(text), toml.stringify(value), toml.is_valid(text)
//!
//! Phase 1.B (ADR-006 §2.7.4) status: `toml.parse` / `toml.stringify`
//! REMAIN DEFERRED pending the **N4** (any-input typed marshal) and
//! **N6** (any-output typed marshal) architectural decisions per
//! `docs/defections.md` HashMap-marshal cluster's sub-decision queue.
//!
//! - `toml.parse(text) -> Result<any>` returns a polymorphic recursive
//!   `toml::Value`-equivalent tree. Mapping to the N6 architectural
//!   surface.
//! - `toml.stringify(value: any)` takes a polymorphic `value: any`
//!   input. Mapping to the N4 architectural surface.
//!
//! `toml.is_valid(text)` is migratable standalone — it only needs the
//! incoming `string` slot.
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

/// Read a [`KindedSlot`]'s bits as an `Arc<String>` payload. Mirrors
/// the helper in `stdlib/json.rs` — Phase 1.B variadic shim until per-
/// position kind threading lands in Phase 2c.
fn slot_as_string(slot: &KindedSlot) -> Option<Arc<String>> {
    let bits = slot.slot().raw();
    if bits == 0 {
        return None;
    }
    // SAFETY: variadic-arg slots whose registered param type is
    // `string` store `Arc::into_raw::<String>` bits. Reconstitute via
    // `from_raw` + `clone` + `forget` to bump the refcount without
    // consuming the slot's strong-count share.
    unsafe {
        let arc = Arc::<String>::from_raw(bits as *const String);
        let cloned = arc.clone();
        std::mem::forget(arc);
        Some(cloned)
    }
}

/// Create the `toml` module with TOML parsing and serialization functions.
pub fn create_toml_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::toml");
    module.description = "TOML parsing and serialization".to_string();

    // toml.parse(text: string) -> Result<HashMap>
    register_typed_function(
        &mut module,
        "parse",
        "Parse a TOML string into Shape values",
        vec![ModuleParam {
            name: "text".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "TOML string to parse".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::HashMap)),
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "toml.parse() pending N6 (any-output marshal) — see ADR-006 §2.7.4".to_string(),
            )))
        },
    );

    // toml.stringify(value: any) -> Result<string>
    register_typed_function(
        &mut module,
        "stringify",
        "Serialize Shape values to a TOML string",
        vec![ModuleParam {
            name: "value".to_string(),
            type_name: "any".to_string(),
            required: true,
            description: "Value to serialize (must be a HashMap or TypedObject)".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "toml.stringify() pending N4 (any-input marshal) — see ADR-006 §2.7.4"
                    .to_string(),
            )))
        },
    );

    // toml.is_valid(text: string) -> bool
    register_typed_function(
        &mut module,
        "is_valid",
        "Check if a string is valid TOML",
        vec![ModuleParam {
            name: "text".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "String to validate as TOML".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        |args, _ctx| {
            let slot = args
                .first()
                .ok_or_else(|| "toml.is_valid() requires a string argument".to_string())?;
            let text = slot_as_string(slot)
                .ok_or_else(|| "toml.is_valid() requires a string argument".to_string())?;
            let valid = toml::from_str::<toml::Value>(text.as_str()).is_ok();
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(valid)))
        },
    );

    module
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

    // Behavioural roundtrip tests deleted alongside `module.invoke_export()`
    // and the deleted `ValueWord` constructors. They return when N4 + N6
    // land and the bodies become real serializers.
}
