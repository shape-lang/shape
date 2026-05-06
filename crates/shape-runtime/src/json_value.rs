//! Typed sum-type for parsed-data trees.
//!
//! Replaces the `ValueWord`-tree return that pre-bulldozer parsers
//! (`json` / `yaml` / `toml` / `msgpack` / `xml`) used. The strict-typed
//! answer is a single concrete enum with the union of variants needed
//! across all five formats; consumers pattern-match exhaustively.
//!
//! Insertion order of `Object` fields is preserved by storing key-value
//! pairs in a `Vec` rather than a `HashMap`. This matches the on-the-wire
//! ordering of JSON / TOML / YAML / MsgPack and lets round-trip
//! serialization stay byte-identical.
//!
//! See `docs/defections.md` (2026-05-06 — typed JsonValue) for the
//! rationale.

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Int(i64),
    Number(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// Return the type-name of this value as a static string. Useful for
    /// error messages without allocating.
    pub fn type_name(&self) -> &'static str {
        match self {
            JsonValue::Null => "null",
            JsonValue::Bool(_) => "bool",
            JsonValue::Int(_) => "int",
            JsonValue::Number(_) => "number",
            JsonValue::String(_) => "string",
            JsonValue::Bytes(_) => "bytes",
            JsonValue::Array(_) => "array",
            JsonValue::Object(_) => "object",
        }
    }
}
