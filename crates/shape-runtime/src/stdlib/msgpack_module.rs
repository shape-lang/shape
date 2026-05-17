//! Native `msgpack` module for MessagePack encoding and decoding.
//!
//! Exports: msgpack.encode(value), msgpack.decode(data),
//!          msgpack.encode_bytes(value), msgpack.decode_bytes(data)
//!
//! Phase 1.B (ADR-006 §2.7.4) status: ALL FOUR FUNCTIONS REMAIN
//! DEFERRED pending the **N4** (any-input typed marshal) and **N6**
//! (any-output typed marshal) architectural decisions per
//! `docs/defections.md` HashMap-marshal cluster's sub-decision queue.
//!
//! - `msgpack.encode(value: any)` and `msgpack.encode_bytes(value: any)`
//!   take a polymorphic `value: any` input parameter that maps to the
//!   N4 architectural surface. There is no `FromSlot` impl for an
//!   `any`-typed input in the post-bulldozer typed marshal layer
//!   (`ConcreteType::Any` exists as a RETURN type only).
//! - `msgpack.decode(data: string)` and
//!   `msgpack.decode_bytes(data: Array<int>)` return `Result<any>` —
//!   the decoded payload is a recursive `serde_json::Value`-equivalent
//!   tree with no current `ConcreteReturn::Any` representation, mapping
//!   to the N6 architectural surface.
//!
//! Until N4 + N6 land, the bodies use the variadic
//! [`register_typed_function`] shape (per ADR-006 §2.7.4 ruling) but
//! return `Err(...)` rather than emit a partial / unsound serializer.
//! The schemas + `ModuleParam` declarations remain so the LSP surface
//! is unaffected. New typed-marshal test fixtures arrive with the
//! shape-vm cleanup workstream.

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_function,
};

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
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "msgpack.encode() pending N4 (any-input marshal) — see ADR-006 §2.7.4".to_string(),
            )))
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
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "msgpack.decode() pending N6 (any-output marshal) — see ADR-006 §2.7.4".to_string(),
            )))
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
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "msgpack.encode_bytes() pending N4 (any-input marshal) — see ADR-006 §2.7.4"
                    .to_string(),
            )))
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
        |_args, _ctx| {
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "msgpack.decode_bytes() pending N6 (any-output marshal) — see ADR-006 §2.7.4"
                    .to_string(),
            )))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_msgpack_typed_registry_populated() {
        let module = create_msgpack_module();
        let typed = module.typed_exports();
        assert!(typed.get("encode").is_some());
        assert!(typed.get("decode").is_some());
        assert!(typed.get("encode_bytes").is_some());
        assert!(typed.get("decode_bytes").is_some());
    }

    // Behavioural roundtrip tests deleted alongside `module.invoke_export()`
    // and the deleted `ValueWord` constructors. They return when N4 + N6
    // land and the bodies become real serializers.
}
