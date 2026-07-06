//! Native `msgpack` module for MessagePack encoding and decoding.
//!
//! Exports: msgpack.encode(value), msgpack.decode(data),
//!          msgpack.encode_bytes(value), msgpack.decode_bytes(data)
//!
//! WF-2E (2026-07-05): all four functions are landed on the shared
//! object-graph marshal core. The deferred "N4 (any-input) / N6
//! (any-output)" architectural surfaces are resolved by the canonical
//! `JsonValue` intermediate (`crate::json_value`):
//!
//! - **encode direction** (`encode` / `encode_bytes`): the polymorphic
//!   `value` argument arrives fully walked into a `JsonValue` tree at the
//!   marshal boundary via `FromSlot<JsonValue>` → `slot_to_json_value`,
//!   dispatching on the stamped `NativeKind` (§2.7.7). It is then rendered
//!   to MessagePack bytes via `json_value_to_msgpack_bytes` (rmp-serde).
//!   `encode` hex-encodes the bytes to a text-safe string; `encode_bytes`
//!   returns them as an `Array<int>` (`ConcreteReturn::Bytes`).
//! - **decode direction** (`decode` / `decode_bytes`): the MessagePack
//!   bytes are decoded to a `serde_json::Value` via `rmp_serde::from_slice`,
//!   funneled through the shared `serde_json_to_json_value` wire→intermediate
//!   converter, and returned as a typed `Json` enum
//!   (`ConcreteReturn::JsonValue`) — the same strict-typed return shape as
//!   `json.parse`.
//!
//! No pointer reinterpretation, no `ValueWord`, no tag-decode shim: every
//! hop is either the stamped-kind `slot_to_json_value` walk or the
//! `serde_json::Value` ↔ `JsonValue` structural mapping.

use crate::json_value::JsonValue;
use crate::marshal::{register_typed_fn_1, register_typed_fn_1_full};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;

/// Create the `msgpack` module with MessagePack encoding and decoding functions.
pub fn create_msgpack_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::msgpack");
    module.description = "MessagePack binary serialization".to_string();

    // msgpack.encode(value: any) -> Result<string>
    //
    // The `value` slot is walked into a `JsonValue` tree at the marshal
    // boundary (stamped-kind directed), rendered to MessagePack bytes via
    // rmp-serde, then hex-encoded to a text-safe string.
    register_typed_fn_1_full::<_, crate::marshal::PolymorphicArg>(
        &mut module,
        "encode",
        "Encode a value to MessagePack (hex-encoded string)",
        [ModuleParam {
            name: "value".to_string(),
            type_name: "any".to_string(),
            required: true,
            description: "Value to encode".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |value: crate::marshal::PolymorphicArg, ctx| {
            // WF-2E: walk via the EXECUTION registry (`ctx.schemas`) — see the
            // xml.stringify note; the ambient `None` path diverges VM↔JIT.
            let value: JsonValue = value.to_json_value(ctx.schemas)?;
            let bytes = crate::json_value::json_value_to_msgpack_bytes(&value)?;
            Ok(TypedReturn::Ok(ConcreteReturn::String(hex::encode(bytes))))
        },
    );

    // msgpack.decode(data: string) -> Result<Json>
    //
    // Hex-decode to raw MessagePack bytes, decode to a `serde_json::Value`
    // via rmp-serde, then project to a typed `Json` enum through the shared
    // `serde_json_to_json_value` converter (same return shape as
    // `json.parse`).
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "decode",
        "Decode a hex-encoded MessagePack string to a value",
        "data",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::JsonValue("Json".to_string()))),
        |data: Arc<String>, _ctx| {
            let bytes = hex::decode(data.as_str())
                .map_err(|e| format!("msgpack.decode() invalid hex: {}", e))?;
            let serde_v: serde_json::Value = rmp_serde::from_slice(&bytes)
                .map_err(|e| format!("msgpack.decode() failed: {}", e))?;
            let jv = crate::json_value::serde_json_to_json_value(serde_v);
            Ok(TypedReturn::Ok(ConcreteReturn::JsonValue(jv)))
        },
    );

    // msgpack.encode_bytes(value: any) -> Result<Array<int>>
    //
    // Same walk as `encode`, but returns the raw MessagePack bytes as an
    // `Array<int>` (`ConcreteReturn::Bytes` — each byte widened to i64 in
    // 0..=255) rather than a hex string.
    register_typed_fn_1_full::<_, crate::marshal::PolymorphicArg>(
        &mut module,
        "encode_bytes",
        "Encode a value to MessagePack as a byte array",
        [ModuleParam {
            name: "value".to_string(),
            type_name: "any".to_string(),
            required: true,
            description: "Value to encode".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::Bytes)),
        |value: crate::marshal::PolymorphicArg, ctx| {
            // WF-2E: walk via the EXECUTION registry (`ctx.schemas`) — see the
            // xml.stringify note; the ambient `None` path diverges VM↔JIT.
            let value: JsonValue = value.to_json_value(ctx.schemas)?;
            let bytes = crate::json_value::json_value_to_msgpack_bytes(&value)?;
            Ok(TypedReturn::Ok(ConcreteReturn::Bytes(bytes)))
        },
    );

    // msgpack.decode_bytes(data: Array<int>) -> Result<Json>
    //
    // Same as `decode`, but the raw MessagePack bytes arrive as an
    // `Array<int>` of byte values (0..=255) instead of a hex string.
    register_typed_fn_1::<_, Vec<i64>>(
        &mut module,
        "decode_bytes",
        "Decode MessagePack from a byte array to a value",
        "data",
        "Array<int>",
        ConcreteType::Result(Box::new(ConcreteType::JsonValue("Json".to_string()))),
        |data: Vec<i64>, _ctx| {
            let mut bytes = Vec::with_capacity(data.len());
            for (i, &b) in data.iter().enumerate() {
                if !(0..=255).contains(&b) {
                    return Err(format!(
                        "msgpack.decode_bytes(): byte at index {} out of range 0..=255: {}",
                        i, b
                    ));
                }
                bytes.push(b as u8);
            }
            let serde_v: serde_json::Value = rmp_serde::from_slice(&bytes)
                .map_err(|e| format!("msgpack.decode_bytes() failed: {}", e))?;
            let jv = crate::json_value::serde_json_to_json_value(serde_v);
            Ok(TypedReturn::Ok(ConcreteReturn::JsonValue(jv)))
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

    /// The rmp-serde round-trip that the encode/decode bodies rely on:
    /// a `serde_json::Value` → MessagePack bytes → `serde_json::Value`
    /// preserves scalars, arrays, and string-keyed objects. Guards the
    /// `rmp_serde::to_vec` / `from_slice` pairing used by the bodies
    /// (the object-graph walk itself is covered by `json_value` tests).
    #[test]
    fn test_msgpack_serde_roundtrip() {
        let original = serde_json::json!({
            "name": "Alice",
            "age": 30,
            "tags": ["a", "b"],
            "active": true
        });
        let bytes = rmp_serde::to_vec(&original).expect("encode");
        let decoded: serde_json::Value = rmp_serde::from_slice(&bytes).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_msgpack_hex_roundtrip_scalar() {
        let original = serde_json::json!(42);
        let bytes = rmp_serde::to_vec(&original).expect("encode");
        let hexed = hex::encode(&bytes);
        let back = hex::decode(&hexed).expect("hex decode");
        let decoded: serde_json::Value = rmp_serde::from_slice(&back).expect("decode");
        assert_eq!(original, decoded);
    }
}
