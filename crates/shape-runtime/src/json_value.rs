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
//! rationale, and (2026-05-07 — N7 unified workstream — ε disposition)
//! for the universal-intermediate role.
//!
//! ADR-005: `JsonValue` is the parser-intermediate / wire-form translation
//! layer, NOT a runtime storage type for user objects. Runtime objects live
//! in `HeapValue::TypedObject` with a flat schema-driven slot array. The
//! typed-parse path (`__parse_typed`) projects `JsonValue` to `TypedObject`
//! before reaching user code; only the untyped `json.parse` path surfaces
//! `JsonValue` to user code (as the `Json` enum in
//! `stdlib-src/core/json_value.shape`). See
//! `docs/adr/005-typed-slot-construction.md`.

use shape_value::heap_value::{HeapValue, TypedArrayData};

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

/// Walk a `HeapValue` tree and produce a `JsonValue`.
///
/// Universal intermediate per the N7 ε disposition (`docs/defections.md`,
/// 2026-05-07). Format-specific encoders take `&JsonValue` (NOT
/// `&HeapValue`) and produce per-format bytes/string. Mirrors json.rs's
/// parse-side `serde_json_to_json_value` (`stdlib/json.rs:172-196`) in
/// reverse.
///
/// Recursion lives at the JsonValue layer (Array/Object children); the
/// `ConcreteReturn` leaf-only invariant is preserved.
///
/// # Variant classification (REFINEMENT-1A + REFINEMENT-1B-ITEM-A)
///
/// **Mechanical-yes (5)**: String, BigInt, Char, TypedArray, HashMap
/// + TypedObject schema-aware (1) — produce a JsonValue directly or
/// recurse.
///
/// **Categorically-non-data Reject (5)**: Future, IoHandle, NativeView,
/// ClosureRaw, TaskGroup — `Err("cannot serialize: <variant>")`
/// permanently. These hold runtime resources; no serialization policy
/// can convert them to wire format.
///
/// **Architectural-choice deferred (7)**: Decimal, DataTable, Content,
/// Temporal, TableView, Instant, NativeScalar — first-landing
/// `Err(<policy not yet decided>)`. Each represents a user-visible
/// behavioral commitment requiring explicit decision per consumer
/// demand.
///
/// **TypedArrayData inner-dispatch**: 13 mechanical-yes sub-variants
/// (8 integer-widths → JsonValue::Int; F32/F64 → JsonValue::Number;
/// Bool → JsonValue::Bool with `u8 0/1` → bool semantic mapping;
/// String → JsonValue::String; HeapValue → recurse) + 2
/// architectural-choice deferred sub-variants (Matrix, FloatSlice;
/// 2D-layout encoding policy not yet decided).
pub fn heap_to_json_value(hv: &HeapValue) -> Result<JsonValue, String> {
    match hv {
        // Mechanical-yes top-level (5)
        HeapValue::String(s) => Ok(JsonValue::String((**s).clone())),
        HeapValue::BigInt(n) => Ok(JsonValue::Int(**n)),
        HeapValue::Char(c) => Ok(JsonValue::String(c.to_string())),
        HeapValue::TypedArray(ta) => typed_array_to_json_value(&**ta),
        HeapValue::HashMap(d) => {
            let mut pairs: Vec<(String, JsonValue)> = Vec::with_capacity(d.keys.data.len());
            for (k, v) in d.keys.data.iter().zip(d.values.data.iter()) {
                pairs.push(((**k).clone(), heap_to_json_value(v)?));
            }
            Ok(JsonValue::Object(pairs))
        }

        // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
        // 2026-05-10): Set serializes as a JSON array of strings (the
        // §2.7.15 amendment's documented wire shape — string-only
        // keyspace at landing). One mechanical-yes mapping; no
        // architectural-choice deferral.
        HeapValue::HashSet(d) => Ok(JsonValue::Array(
            d.keys
                .data
                .iter()
                .map(|k| JsonValue::String((**k).clone()))
                .collect(),
        )),

        // TypedObject schema-aware (1)
        HeapValue::TypedObject(storage) => typed_object_to_json_value(
            storage.schema_id,
            &storage.slots,
            storage.heap_mask,
        ),

        // Categorically-non-data Reject (5)
        HeapValue::Future(_) => Err("cannot serialize: Future".into()),
        HeapValue::IoHandle(_) => Err("cannot serialize: IoHandle".into()),
        HeapValue::NativeView(_) => Err("cannot serialize: NativeView (C view)".into()),
        HeapValue::ClosureRaw(_) => Err("cannot serialize: closure".into()),
        HeapValue::TaskGroup(_) => Err("cannot serialize: TaskGroup".into()),

        // Architectural-choice deferred (7) — first-landing Err per supervisor
        // PB 1/4 + REFINEMENT-1A. Each policy = separate sub-decision when first
        // consumer needs it.
        HeapValue::Decimal(_) => {
            Err("Decimal serialization policy not yet decided (N7 architectural-choice deferral)".into())
        }
        HeapValue::DataTable(_) => Err(
            "DataTable serialization policy not yet decided (N7 architectural-choice deferral)"
                .into(),
        ),
        HeapValue::Content(_) => {
            Err("Content serialization policy not yet decided (N7 architectural-choice deferral)".into())
        }
        HeapValue::Temporal(_) => {
            Err("Temporal serialization policy not yet decided (N7 architectural-choice deferral)".into())
        }
        HeapValue::TableView(_) => {
            Err("TableView serialization policy not yet decided (N7 architectural-choice deferral)".into())
        }
        HeapValue::Instant(_) => Err(
            "Instant serialization policy not yet decided (N7 architectural-choice deferral; Instant is monotonic, not absolute — ISO-8601 inapplicable without epoch convention)"
                .into(),
        ),
        HeapValue::NativeScalar(_) => Err(
            "NativeScalar serialization policy not yet decided (N7 architectural-choice deferral; Ptr inner kind is hostile to JSON)"
                .into(),
        ),
        // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / Q8 amendment): a
        // FilterExpr tree is a transient query-DSL value; it has no JSON
        // representation. Reject in the same shape as the other non-data
        // variants.
        HeapValue::FilterExpr(_) => Err("cannot serialize: FilterExpr".into()),
        // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26, 2026-05-10): Reference
        // values are within-program data and never cross the JSON
        // serialization boundary. Reject in the same shape as
        // FilterExpr.
        HeapValue::Reference(_) => Err("cannot serialize: Reference".into()),
        // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10):
        // Iterator pipelines are lazy within-program values and never
        // cross the JSON serialization boundary. Reject in the same
        // shape as FilterExpr / Reference (callers materialise via
        // collect / forEach / etc. before serialisation).
        HeapValue::Iterator(_) => Err("cannot serialize: Iterator".into()),

        // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10): Range
        // serializes as a JSON array of materialised i64 values —
        // mirror of HashSet's "array of strings" serialization shape
        // (one mechanical-yes mapping; no architectural-choice
        // deferral). Empty ranges produce an empty array. Step is
        // baked into the materialisation, not exposed as a separate
        // field.
        HeapValue::Range(r) => Ok(JsonValue::Array(
            r.to_vec_i64()
                .into_iter()
                .map(JsonValue::Int)
                .collect(),
        )),
    }
}

/// Walk a `TypedArrayData` and produce `JsonValue::Array`.
///
/// 13 mechanical-yes sub-variants + 2 architectural-choice deferred
/// (Matrix, FloatSlice) per REFINEMENT-1B-ITEM-A. The
/// `TypedArrayData::Bool` variant maps `u8 0/1` storage to semantic
/// `bool` (NOT integer coercion).
fn typed_array_to_json_value(ta: &TypedArrayData) -> Result<JsonValue, String> {
    match ta {
        TypedArrayData::I8(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Int(v as i64)).collect(),
        )),
        TypedArrayData::I16(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Int(v as i64)).collect(),
        )),
        TypedArrayData::I32(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Int(v as i64)).collect(),
        )),
        TypedArrayData::I64(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Int(v)).collect(),
        )),
        TypedArrayData::U8(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Int(v as i64)).collect(),
        )),
        TypedArrayData::U16(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Int(v as i64)).collect(),
        )),
        TypedArrayData::U32(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Int(v as i64)).collect(),
        )),
        TypedArrayData::U64(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Int(v as i64)).collect(),
        )),
        TypedArrayData::F32(buf) => Ok(JsonValue::Array(
            buf.data
                .iter()
                .map(|&v| JsonValue::Number(v as f64))
                .collect(),
        )),
        TypedArrayData::F64(buf) => Ok(JsonValue::Array(
            buf.data.iter().map(|&v| JsonValue::Number(v)).collect(),
        )),
        // Bool storage is `Arc<TypedBuffer<u8>>` (heap_value.rs:619); semantic
        // type is bool; map u8 0/1 → JsonValue::Bool (NOT JsonValue::Int).
        TypedArrayData::Bool(buf) => Ok(JsonValue::Array(
            buf.data
                .iter()
                .map(|&b| JsonValue::Bool(b != 0))
                .collect(),
        )),
        TypedArrayData::String(buf) => Ok(JsonValue::Array(
            buf.data
                .iter()
                .map(|s| JsonValue::String((**s).clone()))
                .collect(),
        )),
        TypedArrayData::HeapValue(buf) => {
            let mut elems: Vec<JsonValue> = Vec::with_capacity(buf.data.len());
            for child in buf.data.iter() {
                elems.push(heap_to_json_value(child)?);
            }
            Ok(JsonValue::Array(elems))
        }

        // Architectural-choice deferred (REFINEMENT-1B-ITEM-A) — first-landing
        // Err. 2D-layout encoding policy not yet decided.
        TypedArrayData::Matrix(_) => Err(
            "Matrix serialization policy not yet decided (N7 architectural-choice deferral; multiple natural encodings: nested array-of-arrays vs flat row-major vs {rows, cols, data})"
                .into(),
        ),
        TypedArrayData::FloatSlice { .. } => Err(
            "FloatSlice serialization policy not yet decided (N7 architectural-choice deferral; structurally inherits Matrix's encoding question)"
                .into(),
        ),
    }
}

/// Walk a `HeapValue::TypedObject` and produce `JsonValue::Object`.
///
/// Schema lookup via `lookup_schema_by_id_public`; per-FieldDef
/// `field_type` dispatch using `wire_name()` for JSON field name.
/// Heap-typed fields are read via `slot.as_heap_value()` and recursed
/// into `heap_to_json_value`; inline-typed fields are read via
/// `slot.as_i64()` / `as_f64()` / `as_bool()` per the FieldType arm.
///
/// Mirrors json.rs's parse-side `build_typed_object_from_json` in
/// reverse direction.
fn typed_object_to_json_value(
    schema_id: u64,
    slots: &[shape_value::ValueSlot],
    heap_mask: u64,
) -> Result<JsonValue, String> {
    use crate::type_schema::{lookup_schema_by_id_public, FieldType};

    let schema = lookup_schema_by_id_public(schema_id as u32).ok_or_else(|| {
        format!(
            "heap_to_json_value: unknown TypedObject schema id {}",
            schema_id
        )
    })?;

    let mut pairs: Vec<(String, JsonValue)> = Vec::with_capacity(schema.fields.len());
    for field in &schema.fields {
        let idx = field.index as usize;
        if idx >= slots.len() {
            return Err(format!(
                "heap_to_json_value: TypedObject field '{}' index {} out of bounds (slots.len()={})",
                field.name,
                idx,
                slots.len()
            ));
        }
        let slot = &slots[idx];
        let is_heap = (heap_mask & (1u64 << field.index)) != 0;
        let child = match (&field.field_type, is_heap) {
            (FieldType::I64, false)
            | (FieldType::I8, false)
            | (FieldType::U8, false)
            | (FieldType::I16, false)
            | (FieldType::U16, false)
            | (FieldType::I32, false)
            | (FieldType::U32, false)
            | (FieldType::U64, false) => JsonValue::Int(slot.as_i64()),
            (FieldType::F64, false) => JsonValue::Number(slot.as_f64()),
            (FieldType::Bool, false) => JsonValue::Bool(slot.as_bool()),
            (FieldType::Timestamp, false) => {
                // Timestamp is i64 ms-since-epoch — distinct from Instant
                // (which is monotonic). Same architectural-choice as Temporal/
                // Instant (user-visible behavioral commitment); first-landing
                // Err per N7 deferral.
                return Err(format!(
                    "Timestamp serialization policy not yet decided (N7 architectural-choice deferral; field '{}')",
                    field.name
                ));
            }
            (FieldType::Decimal, _) => {
                return Err(format!(
                    "Decimal serialization policy not yet decided (N7 architectural-choice deferral; field '{}')",
                    field.name
                ));
            }
            (_, true) => heap_to_json_value(slot.as_heap_value())?,
            // Inline scalar types where storage doesn't match field_type
            // (Array/Object/Any when not heap-tagged; impossible if heap_mask
            // is correct).
            (other, false) => {
                return Err(format!(
                    "heap_to_json_value: TypedObject field '{}' has field_type {} but heap_mask bit clear (corrupt mask?)",
                    field.name, other
                ));
            }
        };
        pairs.push((field.wire_name().to_string(), child));
    }
    Ok(JsonValue::Object(pairs))
}

/// Convert a `JsonValue` into a `serde_json::Value`.
///
/// Inverse of `serde_json_to_json_value` (`stdlib/json.rs:172-196`).
/// Used by N7 consumers that produce JSON strings: `json.stringify`
/// (C7), `http.post_json` (C8), `http.put_json` (C9). Pair with
/// `heap_to_json_value` to round-trip a `HeapValue` tree to a JSON
/// string via `serde_json::to_string(&v)?` / `to_string_pretty(&v)?`.
///
/// `JsonValue::Bytes` maps to `serde_json::Value::Array` of `u8`-as-
/// `Number` per JSON's no-byte-array convention. `JsonValue::Bytes` is
/// not currently produced by `heap_to_json_value` (the C2 walker has
/// no path that emits Bytes); included here for completeness +
/// bidirectional symmetry with future 3.C msgpack-binary parse paths
/// per supervisor PB 3/4.
pub fn json_value_to_serde_json(jv: &JsonValue) -> serde_json::Value {
    match jv {
        JsonValue::Null => serde_json::Value::Null,
        JsonValue::Bool(b) => serde_json::Value::Bool(*b),
        JsonValue::Int(i) => serde_json::Value::Number((*i).into()),
        JsonValue::Number(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        JsonValue::String(s) => serde_json::Value::String(s.clone()),
        JsonValue::Bytes(bytes) => serde_json::Value::Array(
            bytes
                .iter()
                .map(|&b| serde_json::Value::Number(b.into()))
                .collect(),
        ),
        JsonValue::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(json_value_to_serde_json).collect())
        }
        JsonValue::Object(pairs) => {
            let mut map = serde_json::Map::with_capacity(pairs.len());
            for (k, v) in pairs.iter() {
                map.insert(k.clone(), json_value_to_serde_json(v));
            }
            serde_json::Value::Object(map)
        }
    }
}

/// Convert a `JsonValue` into a `serde_yaml::Value`.
///
/// Used by N7 consumer C10 (`yaml.stringify`). Pair with
/// `heap_to_json_value` to round-trip a `HeapValue` tree to a YAML
/// string via `serde_yaml::to_string(&v)?`.
///
/// Lossy mapping shape parallels parse-side yaml.rs precedent
/// (yaml.rs:75-78 unwraps `serde_yaml::Value::Tagged`); on the encode
/// side, we never produce Tagged, so no lossy path. `JsonValue::Bytes`
/// maps to `Value::Sequence` of `u8` numbers (YAML has no native byte
/// type); reserved for future msgpack-binary roundtrip via 3.C.
pub fn json_value_to_serde_yaml(jv: &JsonValue) -> serde_yaml::Value {
    match jv {
        JsonValue::Null => serde_yaml::Value::Null,
        JsonValue::Bool(b) => serde_yaml::Value::Bool(*b),
        JsonValue::Int(i) => serde_yaml::Value::Number((*i).into()),
        JsonValue::Number(f) => serde_yaml::Value::Number((*f).into()),
        JsonValue::String(s) => serde_yaml::Value::String(s.clone()),
        JsonValue::Bytes(bytes) => serde_yaml::Value::Sequence(
            bytes
                .iter()
                .map(|&b| serde_yaml::Value::Number((b as u64).into()))
                .collect(),
        ),
        JsonValue::Array(arr) => {
            serde_yaml::Value::Sequence(arr.iter().map(json_value_to_serde_yaml).collect())
        }
        JsonValue::Object(pairs) => {
            let mut map = serde_yaml::Mapping::with_capacity(pairs.len());
            for (k, v) in pairs.iter() {
                map.insert(
                    serde_yaml::Value::String(k.clone()),
                    json_value_to_serde_yaml(v),
                );
            }
            serde_yaml::Value::Mapping(map)
        }
    }
}

/// Convert a `JsonValue` into a `toml::Value`.
///
/// Used by N7 consumer C11 (`toml.stringify`). Pair with
/// `heap_to_json_value` to round-trip a `HeapValue` tree to a TOML
/// string via `toml::to_string(&v)?`. **Replaces** the legacy
/// `nanboxed_to_toml_value` walker (`stdlib/toml_module.rs:67-107`)
/// entirely; that walker used deleted ValueWord accessors and is
/// removed by C11.
///
/// **TOML constraint**: TOML has no native null. `JsonValue::Null` maps
/// to `toml::Value::String("null")` — the same lossy sentinel used by
/// the legacy `nanboxed_to_toml_value` walker (`toml_module.rs:68-70`),
/// preserved here for round-trip behavior continuity. Reconsidering
/// this sentinel is a future architectural-choice sub-decision (the
/// alternative — refusing serialization with Err — would be a behavioral
/// regression vs the legacy walker; held as future N7 sub-disposition).
///
/// **TOML constraint**: TOML's top-level must be a Table. This helper
/// returns a `toml::Value` of any shape; the consumer (`toml.stringify`
/// body in C11) is responsible for verifying root-level Table when
/// passing to `toml::to_string`. Surfacing root-level non-Table as Err
/// is C11's responsibility, not this helper's.
///
/// `JsonValue::Bytes` maps to `Array` of `u8`-as-Integer (TOML has no
/// native byte type); reserved for future msgpack-binary roundtrip via
/// 3.C.
pub fn json_value_to_toml_value(jv: &JsonValue) -> toml::Value {
    match jv {
        JsonValue::Null => toml::Value::String("null".to_string()),
        JsonValue::Bool(b) => toml::Value::Boolean(*b),
        JsonValue::Int(i) => toml::Value::Integer(*i),
        JsonValue::Number(f) => toml::Value::Float(*f),
        JsonValue::String(s) => toml::Value::String(s.clone()),
        JsonValue::Bytes(bytes) => toml::Value::Array(
            bytes
                .iter()
                .map(|&b| toml::Value::Integer(b as i64))
                .collect(),
        ),
        JsonValue::Array(arr) => {
            toml::Value::Array(arr.iter().map(json_value_to_toml_value).collect())
        }
        JsonValue::Object(pairs) => {
            let mut map = toml::map::Map::new();
            for (k, v) in pairs.iter() {
                map.insert(k.clone(), json_value_to_toml_value(v));
            }
            toml::Value::Table(map)
        }
    }
}

/// Encode a `JsonValue` to MessagePack bytes.
///
/// Used by N7 consumers C12 (`msgpack.encode`) and C13
/// (`msgpack.encode_bytes`). Pair with `heap_to_json_value` to
/// round-trip a `HeapValue` tree to MessagePack-encoded bytes.
///
/// **Routing**: this helper internally converts the `JsonValue` to a
/// `serde_json::Value` via `json_value_to_serde_json` (C3) and then
/// calls `rmp_serde::to_vec` on the result. The external surface is a
/// single named `&JsonValue → Result<Vec<u8>, String>` contract;
/// consumers do NOT see the internal serde_json::Value intermediate.
///
/// **Why this shape (Option C per team-lead authorization)**: the
/// `rmpv::Value` library is NOT in workspace deps, only `rmp-serde` and
/// `rmp` are. The legacy msgpack path
/// (`stdlib/msgpack_module.rs:104-107` pre-bulldozer) routed
/// `value.to_json_value()` (deleted) through
/// `rmp_serde::to_vec(&json_value)` — the routing-through-serde_json
/// pattern is precedent. Option C preserves this structural pattern
/// while exposing a single named JsonValue→bytes helper, decoupling
/// consumer-body from internal routing (forbidden state: "consumer-
/// body couples with internal routing" is unrepresentable; future
/// rmpv-adoption for performance won't change this helper's external
/// contract).
///
/// **Naming correction**: the original REFINEMENT-1A scope brief
/// paraphrased C6 as `json_value_to_rmpv_value`. Team-lead self-flagged
/// this as paraphrase error: supervisor PB 1/4 said "C3-C6 per-format
/// encoders (json/yaml/toml/msgpack)" with implicit naming, NOT a
/// literal `rmpv` requirement. The signature here matches the actual
/// supervisor framing; rmpv is not used.
pub fn json_value_to_msgpack_bytes(jv: &JsonValue) -> Result<Vec<u8>, String> {
    let serde_json_v = json_value_to_serde_json(jv);
    rmp_serde::to_vec(&serde_json_v).map_err(|e| format!("msgpack encode failed: {}", e))
}
