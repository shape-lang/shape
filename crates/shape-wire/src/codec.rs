//! Encoding and decoding functions
//!
//! This module provides the core serialization functions for the wire format.
//! Primary format is MessagePack (via rmp-serde) for performance, with JSON
//! wrappers for debugging and external tool interoperability.

use crate::envelope::ValueEnvelope;
use crate::error::{Result, WireError};

/// Encode an envelope to binary format (MessagePack)
///
/// This is the primary serialization format for performance-critical paths
/// like REPL communication and fchart interop.
///
/// Uses named encoding (MessagePack map format) for better compatibility
/// and forward/backward compatibility with schema changes.
///
/// # Example
/// ```
/// use shape_wire::{encode, ValueEnvelope};
///
/// let env = ValueEnvelope::number(42.0);
/// let bytes = encode(&env);
/// assert!(!bytes.is_empty());
/// ```
pub fn encode(envelope: &ValueEnvelope) -> Vec<u8> {
    rmp_serde::to_vec_named(envelope).expect("Failed to serialize envelope - this is a bug")
}

/// Decode an envelope from binary format (MessagePack)
///
/// # Example
/// ```
/// use shape_wire::{encode, decode, ValueEnvelope};
///
/// let env = ValueEnvelope::number(42.0);
/// let bytes = encode(&env);
/// let decoded = decode(&bytes).unwrap();
/// assert_eq!(env.value, decoded.value);
/// ```
pub fn decode(bytes: &[u8]) -> Result<ValueEnvelope> {
    rmp_serde::from_slice(bytes).map_err(|e| WireError::DeserializationError(e.to_string()))
}

/// Convert an envelope to JSON (for debugging/external tools)
///
/// JSON is less efficient than MessagePack but more portable and human-readable.
///
/// # Example
/// ```
/// use shape_wire::{to_json, ValueEnvelope};
///
/// let env = ValueEnvelope::string("hello");
/// let json = to_json(&env);
/// assert!(json["value"]["String"].is_string());
/// ```
pub fn to_json(envelope: &ValueEnvelope) -> serde_json::Value {
    serde_json::to_value(envelope).expect("Failed to convert to JSON - this is a bug")
}

/// Parse an envelope from JSON
///
/// # Example
/// ```
/// use shape_wire::{to_json, from_json, ValueEnvelope};
///
/// let env = ValueEnvelope::bool(true);
/// let json = to_json(&env);
/// let decoded = from_json(&json).unwrap();
/// assert_eq!(env.value, decoded.value);
/// ```
pub fn from_json(json: &serde_json::Value) -> Result<ValueEnvelope> {
    serde_json::from_value(json.clone()).map_err(|e| WireError::DeserializationError(e.to_string()))
}

/// Encode to JSON string
pub fn to_json_string(envelope: &ValueEnvelope) -> String {
    serde_json::to_string(envelope).expect("Failed to serialize to JSON string - this is a bug")
}

/// Encode to pretty JSON string (for debugging)
pub fn to_json_string_pretty(envelope: &ValueEnvelope) -> String {
    serde_json::to_string_pretty(envelope)
        .expect("Failed to serialize to JSON string - this is a bug")
}

/// Decode from JSON string
pub fn from_json_string(s: &str) -> Result<ValueEnvelope> {
    serde_json::from_str(s).map_err(|e| WireError::DeserializationError(e.to_string()))
}

/// Get the size of an encoded envelope in bytes
///
/// Useful for debugging and monitoring serialization overhead.
pub fn encoded_size(envelope: &ValueEnvelope) -> usize {
    encode(envelope).len()
}

// =========================================================================
// Generic encode/decode — for any Serialize/Deserialize type
// =========================================================================

/// Encode any serializable value to MessagePack bytes.
///
/// This is the generic version of [`encode`] — it works with any type that
/// implements `serde::Serialize`, not just `ValueEnvelope`. Use this for
/// remote execution messages (`RemoteCallRequest`, `RemoteCallResponse`, etc.).
pub fn encode_message<T: serde::Serialize>(message: &T) -> Result<Vec<u8>> {
    rmp_serde::to_vec_named(message)
        .map_err(|e| WireError::DeserializationError(format!("encode failed: {}", e)))
}

/// Decode any deserializable value from MessagePack bytes.
///
/// This is the generic version of [`decode`] — it works with any type that
/// implements `serde::Deserialize`, not just `ValueEnvelope`.
pub fn decode_message<'a, T: serde::Deserialize<'a>>(bytes: &'a [u8]) -> Result<T> {
    rmp_serde::from_slice(bytes).map_err(|e| WireError::DeserializationError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::WireValue;

    #[test]
    fn test_msgpack_roundtrip() {
        let env = ValueEnvelope::from_value(WireValue::Number(42.0));
        let bytes = encode(&env);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(env, decoded);
    }

    #[test]
    fn test_json_roundtrip() {
        let env = ValueEnvelope::from_value(WireValue::String("hello world".to_string()));
        let json = to_json(&env);
        let decoded = from_json(&json).unwrap();
        assert_eq!(env, decoded);
    }

    #[test]
    fn test_complex_value_roundtrip() {
        use crate::value::WireTable;

        let table = WireTable {
            ipc_bytes: vec![1, 2, 3, 4],
            type_name: Some("TestTable".to_string()),
            schema_id: Some(1),
            row_count: 3,
            column_count: 1,
        };

        let env = ValueEnvelope::from_value(WireValue::Table(table));

        // MessagePack roundtrip
        let bytes = encode(&env);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(env, decoded);

        // JSON roundtrip
        let json = to_json(&env);
        let decoded = from_json(&json).unwrap();
        assert_eq!(env, decoded);
    }

    #[test]
    fn test_json_string_roundtrip() {
        let env = ValueEnvelope::timestamp(1704067200000);
        let json_str = to_json_string(&env);
        let decoded = from_json_string(&json_str).unwrap();
        assert_eq!(env, decoded);
    }

    #[test]
    fn test_encoded_size() {
        let small = ValueEnvelope::number(1.0);
        let large = ValueEnvelope::from_value(WireValue::Array(
            (0..1000).map(|i| WireValue::Number(i as f64)).collect(),
        ));

        let small_size = encoded_size(&small);
        let large_size = encoded_size(&large);

        // Small values should be smaller than large arrays
        assert!(small_size < large_size);
        // Named encoding includes field names, so it's larger but still reasonable
        // The envelope includes type metadata so expect ~1KB for a simple value
        assert!(small_size < 2000);
    }

    #[test]
    fn test_invalid_msgpack() {
        let result = decode(&[0xFF, 0xFF, 0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_json() {
        let json = serde_json::json!({"invalid": "structure"});
        let result = from_json(&json);
        assert!(result.is_err());
    }
}
