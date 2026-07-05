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

use shape_value::heap_value::{
    HashMapKindedRef, HashSetElementKind, HeapValue, TypedObjectPtr, TypedObjectStorage,
};
use shape_value::{HeapKind, NativeKind};

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
/// V3-S5 ckpt-5-prime (2026-05-15): the **TypedArrayData inner-dispatch**
/// description below previously named the 13-arm `typed_array_to_json_value`
/// helper. That helper + the `HeapValue::TypedArray(ta)` outer arm here are
/// RETIRED in lockstep with the deleted `HeapValue::TypedArray` variant
/// (ckpt-4) + deleted `TypedArrayData` inner enum (ckpt-1). The v2-raw
/// `*mut TypedArray<T>` JSON-serialisation path lands at the ckpt-5-prime²
/// + ckpt-6 producer/consumer storage-shape migration (per W12 audit §3.6
/// — no `*mut TypedArray<T>` value ever reaches `heap_to_json_value`
/// post-V3-S5 ckpt-5: the JSON projection happens at the marshal layer
/// before the value becomes a `HeapValue`). Refusal #1 binding.
pub fn heap_to_json_value(hv: &HeapValue) -> Result<JsonValue, String> {
    match hv {
        // Mechanical-yes top-level (4 after V3-S5 ckpt-5-prime TypedArray retirement)
        HeapValue::String(s) => Ok(JsonValue::String((**s).clone())),
        HeapValue::BigInt(n) => Ok(JsonValue::Int(**n)),
        HeapValue::Char(c) => Ok(JsonValue::String(c.to_string())),
        HeapValue::HashMap(kref) => hashmap_kref_to_json_value(kref),

        // Wave 13 W13-hashset-rebuild plus W74B int-key redrive: Set
        // serializes as a JSON array matching its explicit element arm.
        // No fallback to the string buffer: an int set with zero string keys
        // must not silently serialize as an empty string set.
        HeapValue::HashSet(d) => match d.element_kind() {
            HashSetElementKind::String => Ok(JsonValue::Array(
                d.string_keys()
                    .iter()
                    .map(|k| JsonValue::String((**k).clone()))
                    .collect(),
            )),
            HashSetElementKind::I64 => Ok(JsonValue::Array(
                d.i64_keys().iter().map(|k| JsonValue::Int(*k)).collect(),
            )),
        },

        // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10):
        // Deque serializes as a JSON array of front-to-back elements.
        // Each element dispatches through the canonical ADR-005 §1
        // single-discriminator `HeapValue` recursion. Same mechanical-
        // yes mapping shape as HashSet (string-array specialisation
        // generalised to heterogeneous-element).
        HeapValue::Deque(d) => {
            let mut elems: Vec<JsonValue> = Vec::with_capacity(d.items.len());
            for v in d.items.iter() {
                elems.push(heap_to_json_value(v)?);
            }
            Ok(JsonValue::Array(elems))
        }

        // TypedObject schema-aware (1)
        HeapValue::TypedObject(storage) => typed_object_ptr_to_json_value(storage),

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
        // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
        // 2026-05-10): channels are concurrency primitives with
        // interior `Mutex<ChannelInner>` state; the queue contents
        // are runtime-mutable and don't have a stable serialized
        // form. Reject in the same shape as FilterExpr / Iterator.
        HeapValue::Channel(_) => Err("cannot serialize: Channel".into()),

        // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
        // 2026-05-10): PriorityQueue serialises as a JSON array of
        // i64 priorities in heap-array order (the §2.7.18 amendment's
        // documented wire shape — i64-priority-only at landing). The
        // sorted shape is exposed only via `pq.toSortedArray()`; raw
        // serialisation preserves heap order to match Display.
        HeapValue::PriorityQueue(d) => Ok(JsonValue::Array(
            d.heap
                .iter()
                .map(|v| JsonValue::Int(*v))
                .collect(),
        )),

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
        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10):
        // Result/Option carriers are within-program control-flow values;
        // serialisation policy is deferred to the AnyError marshal /
        // unwrapped-inner-value path. Reject in the same shape as
        // Iterator until the policy is decided.
        HeapValue::Result(_) => Err("cannot serialize: Result".into()),
        HeapValue::Option(_) => Err("cannot serialize: Option".into()),
        // W17-concurrency (ADR-006 §2.7.25, 2026-05-11): concurrency
        // primitives carry runtime-mutable interior state (Mutex inner
        // value, atomic counter, lazy initializer) and don't have a
        // stable serialized form. Reject in the same shape as
        // Channel / Iterator.
        HeapValue::Mutex(_) => Err("cannot serialize: Mutex".into()),
        HeapValue::Atomic(_) => Err("cannot serialize: Atomic".into()),
        HeapValue::Lazy(_) => Err("cannot serialize: Lazy".into()),
        // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C, 2026-05-11):
        // a `dyn Trait` carrier has no stable JSON form — the boxed
        // value's schema is dynamic, and serializing through the
        // vtable would require a `to_json()` trait method that
        // doesn't exist at the language level. Reject in the same
        // shape as the concurrency primitives. The compiler-emission
        // tier may later add a `Serializable` trait whose impls
        // self-serialize through the vtable — that's a follow-up.
        HeapValue::TraitObject(_) => Err("cannot serialize: TraitObject".into()),
        // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
        // ModuleFn references are VM-internal callable handles with
        // no stable serialised form — they index `module_fn_table`
        // which is rebuilt per-VM-instance, not part of the
        // serialisable program state.
        HeapValue::ModuleFn(_) => Err("cannot serialize: ModuleFn".into()),
        // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): Matrix /
        // MatrixSlice JSON serialization-policy is N7-architectural-choice
        // deferred (mirror of the pre-amendment
        // `TypedArrayData::Matrix` / `FloatSlice` rejection at this layer;
        // 2D-layout encoding is undecided — nested array-of-arrays vs
        // flat row-major vs `{rows, cols, data}` forms have different
        // round-trip properties). MatrixSlice inherits the same deferral.
        HeapValue::Matrix(_) => Err(
            "Matrix serialization policy not yet decided (N7 architectural-choice deferral; multiple natural encodings: nested array-of-arrays vs flat row-major vs {rows, cols, data})"
                .into(),
        ),
        HeapValue::MatrixSlice(_) => Err(
            "MatrixSlice serialization policy not yet decided (N7 architectural-choice deferral; structurally inherits Matrix's encoding question)"
                .into(),
        ),
    }
}

// V3-S5 ckpt-5-prime (2026-05-15): `typed_array_to_json_value` helper RETIRED
// per W12 audit §3.6. The helper pattern-matched on the deleted `TypedArrayData`
// enum (retired at ckpt-1) and was called by the deleted `HeapValue::TypedArray`
// outer arm (retired at ckpt-4) above. The 13 mechanical-yes inner-arm
// dispatches (I8/I16/I32/I64/U8/U16/U32/U64/F32/F64/Bool/String + later
// Decimal/BigInt/Char/TypedObject from W17-typed-carrier-bundle-A) lose their
// landing point with the carrier enum gone. The v2-raw `*mut TypedArray<T>`
// JSON-serialisation path lands at the ckpt-5-prime² + ckpt-6 producer/
// consumer storage-shape migration (per-element-type marshal-layer projection
// before the value becomes a `HeapValue`). Refusal #1 binding: do not
// reintroduce under any rename/shim/bridge.

/// Walk a direct `HashMapKindedRef` carrier and produce `JsonValue::Object`.
///
/// This is the post-W70 direct-carrier counterpart to the
/// `HeapValue::HashMap` arm. It never assumes a `Box<HeapValue>` or
/// `Arc<HeapValue>` wrapper around the map.
pub fn hashmap_kref_to_json_value(kref: &HashMapKindedRef) -> Result<JsonValue, String> {
    hashmap_kref_to_json_value_with_registry(kref, None)
}

fn hashmap_kref_to_json_value_with_registry(
    kref: &HashMapKindedRef,
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Result<JsonValue, String> {
    // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V walk
    // reading keys (`*mut TypedArray<*const StringObj>` → `&str`) and
    // values (`*mut TypedArray<V>` → `JsonValue` per V). ADR-006
    // §2.7.24 Q25.B SUPERSEDED + audit §C.4.
    let n = kref.len();
    let mut out: Vec<(String, JsonValue)> = Vec::with_capacity(n);
    let keys_ptr = match kref {
        HashMapKindedRef::I64(arc) => arc.keys,
        HashMapKindedRef::F64(arc) => arc.keys,
        HashMapKindedRef::Bool(arc) => arc.keys,
        HashMapKindedRef::Char(arc) => arc.keys,
        HashMapKindedRef::String(arc) => arc.keys,
        HashMapKindedRef::Decimal(arc) => arc.keys,
        HashMapKindedRef::TypedObject(arc) => arc.keys,
        HashMapKindedRef::TraitObject(arc) => arc.keys,
        HashMapKindedRef::Callable(arc) => arc.keys,
        HashMapKindedRef::HashMap(arc) => arc.keys,
    };
    for i in 0..n {
        let key: String = unsafe {
            let ptr = shape_value::v2::typed_array::TypedArray::get_unchecked(keys_ptr, i as u32);
            shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned()
        };
        let value: JsonValue = match kref {
            HashMapKindedRef::I64(arc) => {
                let v: i64 = unsafe { *(*arc.values).data.add(i) };
                JsonValue::Int(v)
            }
            HashMapKindedRef::F64(arc) => {
                let v: f64 = unsafe { *(*arc.values).data.add(i) };
                JsonValue::Number(v)
            }
            HashMapKindedRef::Bool(arc) => {
                let v: u8 = unsafe { *(*arc.values).data.add(i) };
                JsonValue::Bool(v != 0)
            }
            HashMapKindedRef::Char(arc) => {
                let v: char = unsafe { *(*arc.values).data.add(i) };
                JsonValue::String(v.to_string())
            }
            HashMapKindedRef::String(arc) => {
                let ptr: *const shape_value::v2::string_obj::StringObj =
                    unsafe { *(*arc.values).data.add(i) };
                JsonValue::String(unsafe {
                    shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned()
                })
            }
            HashMapKindedRef::Decimal(_) => {
                return Err("HeapValue::HashMap<string, decimal> → JsonValue: \
                    decimal serialization policy not yet decided (precision \
                    preservation vs lossy f64 cast). Surface-and-stop per \
                    playbook §6."
                    .to_string());
            }
            HashMapKindedRef::TypedObject(arc) => {
                let elem: &TypedObjectPtr = unsafe { &*(*arc.values).data.add(i) };
                typed_object_ptr_to_json_value_with_registry_opt(elem, schemas)?
            }
            HashMapKindedRef::TraitObject(_) => {
                return Err("HeapValue::HashMap<string, TraitObject> → JsonValue: \
                    no canonical JSON shape for TraitObject. Surface-and-stop."
                    .to_string());
            }
            HashMapKindedRef::Callable(_) => {
                return Err("HeapValue::HashMap<string, Function> → JsonValue: \
                    no canonical JSON shape for callable values. Surface-and-stop."
                    .to_string());
            }
            HashMapKindedRef::HashMap(arc) => {
                let inner_ref: &HashMapKindedRef = unsafe { &*(*arc.values).data.add(i) };
                hashmap_kref_to_json_value_with_registry(inner_ref, schemas)?
            }
        };
        out.push((key, value));
    }
    Ok(JsonValue::Object(out))
}

/// Walk a direct `TypedObjectPtr` carrier and produce `JsonValue::Object`.
pub fn typed_object_ptr_to_json_value(ptr: &TypedObjectPtr) -> Result<JsonValue, String> {
    typed_object_ptr_to_json_value_with_registry_opt(ptr, None)
}

pub fn typed_object_ptr_to_json_value_with_registry(
    ptr: &TypedObjectPtr,
    schemas: &crate::type_schema::TypeSchemaRegistry,
) -> Result<JsonValue, String> {
    typed_object_ptr_to_json_value_with_registry_opt(ptr, Some(schemas))
}

fn typed_object_ptr_to_json_value_with_registry_opt(
    ptr: &TypedObjectPtr,
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Result<JsonValue, String> {
    if ptr.is_null() {
        return Err("heap_to_json_value: null TypedObject pointer".to_string());
    }
    let storage: &TypedObjectStorage = unsafe { &*ptr.as_ptr() };
    typed_object_to_json_value(
        storage.schema_id,
        storage.slots(),
        storage.heap_mask,
        &storage.field_kinds,
        schemas,
    )
}

/// Resolve the canonical `Json` ADT schema id (`std::core::json_value`).
///
/// Prefers the caller-supplied registry (the active program's registry,
/// e.g. `ctx.schemas` in `json.stringify`); falls back to the ambient
/// task/thread/default registry when the caller passes `None`. The by-name
/// `"Json"` lookup is reliable even when the registry's by-id index has a
/// collision, so it is the correct discriminator for Json-enum nodes.
fn resolve_json_schema_id(
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Option<u64> {
    if let Some(registry) = schemas {
        if let Some(schema) = registry.get("Json") {
            return Some(schema.id as u64);
        }
    }
    let ambient = crate::type_schema::current_registry();
    ambient.get("Json").map(|schema| schema.id as u64)
}

/// Resolve the `TypeSchema` for a `TypedObject` node, robust against registry
/// by-id COLLISIONS.
///
/// WF-2E (2026-07-05): a `TypedObject` carries only a numeric `schema_id`, and
/// the schema registries can map one id to MORE THAN ONE schema — observed:
/// a predeclared `XmlNode`-shaped schema and the builtin `Json` enum both
/// answer to id 41, and `get_by_id` / `lookup_schema_by_id_public` return the
/// `Json` schema (2 fields) for an XmlNode node (3 fields), so the walk emits
/// "node missing 'name'". The node's own STRUCTURE disambiguates: its actual
/// slot count is the arity of its true schema. This resolver gathers every
/// candidate schema registered under `schema_id` (execution registry by-id,
/// ambient by-id, and the ambient predeclared table — the predeclared entry is
/// otherwise unreachable because `lookup_schema_by_id` returns the colliding
/// by-id hit first) and prefers the one whose field count equals `slot_count`.
fn resolve_typed_object_schema(
    schema_id: u64,
    slot_count: usize,
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Option<crate::type_schema::TypeSchema> {
    use crate::type_schema::lookup_schema_by_id_public;
    let id = schema_id as u32;
    let mut candidates: Vec<crate::type_schema::TypeSchema> = Vec::new();
    if let Some(registry) = schemas {
        if let Some(schema) = registry.get_by_id(id) {
            candidates.push(schema.clone());
        }
    }
    if let Some(schema) = lookup_schema_by_id_public(id) {
        candidates.push(schema);
    }
    let ambient = crate::type_schema::current_registry();
    if let Some(schema) = ambient.lookup_predeclared_by_id(id) {
        candidates.push(schema);
    }
    // Prefer the candidate whose arity matches the node's actual slot count
    // (the collision-robust choice); fall back to the first candidate.
    candidates
        .iter()
        .find(|s| s.fields.len() == slot_count)
        .cloned()
        .or_else(|| candidates.into_iter().next())
}

/// Attempt to invert a `Json` enum `TypedObject` node back to its logical
/// `JsonValue`. Returns `None` when the node does NOT structurally match the
/// canonical `Json` ADT layout (so the caller falls through to the generic
/// struct walk — this is what rejects a non-Json object that only passed the
/// `schema_id == json_id` gate via a registry by-id collision).
///
/// The Json ADT layout is `slot 0 = __variant (Int64)`, `slot 1 =
/// __payload_0`. A genuine node satisfies ALL of:
///   * exactly 2 slots and 2 `field_kinds`,
///   * `field_kinds[0] == Int64` (the `__variant` discriminant),
///   * `slots[0]` is a valid Json variant id (0..=6), AND
///   * `field_kinds[1]` (the STAMPED payload kind — ADR-006 §2.7.7, the
///     single discriminator the node was constructed with) is exactly the
///     kind that variant's payload must carry.
/// The variant↔payload-kind consistency is the strong discriminator against a
/// colliding plain object.
///
/// On a match the payload is read from its stamped carrier (Str/Array/Object
/// via the same borrow-only heap reader the struct walk uses, so refcounts
/// are untouched). Array elements and Object values are themselves Json
/// nodes, so the recursion re-enters `typed_object_to_json_value` and this
/// fast path fires again — no schema walk, no `__variant` leak.
fn try_json_enum_node(
    slots: &[shape_value::ValueSlot],
    field_kinds: &[NativeKind],
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Option<Result<JsonValue, String>> {
    if slots.len() != 2 || field_kinds.len() != 2 || field_kinds[0] != NativeKind::Int64 {
        return None;
    }
    let variant = slots[0].as_i64();
    let payload_kind = field_kinds[1];
    let bits = slots[1].raw();
    // Variant id → the payload kind it MUST carry (mirrors
    // `stdlib::json::build_json_enum_heap_value` / `project_json_value_to_slot`).
    let value = match variant {
        // Json::Null
        0 if payload_kind == NativeKind::Null => JsonValue::Null,
        // Json::Bool
        1 if payload_kind == NativeKind::Bool => JsonValue::Bool(bits != 0),
        // Json::Int
        2 if payload_kind == NativeKind::Int64 => JsonValue::Int(bits as i64),
        // Json::Number
        3 if payload_kind == NativeKind::Float64 => JsonValue::Number(f64::from_bits(bits)),
        // Json::Str
        4 if matches!(payload_kind, NativeKind::String | NativeKind::StringV2) => {
            return Some(typed_object_heap_field_to_json_value(
                bits,
                payload_kind,
                "__payload_0",
                schemas,
            ));
        }
        // Json::Array
        5 if payload_kind == NativeKind::Ptr(HeapKind::TypedArray) => {
            return Some(typed_object_heap_field_to_json_value(
                bits,
                payload_kind,
                "__payload_0",
                schemas,
            ));
        }
        // Json::Object
        6 if payload_kind == NativeKind::Ptr(HeapKind::HashMap) => {
            return Some(typed_object_heap_field_to_json_value(
                bits,
                payload_kind,
                "__payload_0",
                schemas,
            ));
        }
        // Not a structurally-valid Json node (collision) — let the caller
        // walk it as a plain struct.
        _ => return None,
    };
    Some(Ok(value))
}

/// Walk a `HeapValue::TypedObject` and produce `JsonValue::Object`.
///
/// Schema lookup via `lookup_schema_by_id_public`; per-FieldDef
/// `field_type` dispatch using `wire_name()` for JSON field name. Heap
/// fields are decoded from their stamped `NativeKind` carrier, while inline
/// fields use the schema `FieldType` arm.
///
/// Mirrors json.rs's parse-side `build_typed_object_from_json` in
/// reverse direction.
fn typed_object_to_json_value(
    schema_id: u64,
    slots: &[shape_value::ValueSlot],
    heap_mask: u64,
    field_kinds: &[NativeKind],
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Result<JsonValue, String> {
    use crate::type_schema::FieldType;

    // ── Json-enum node fast path (WF-2E, 2026-07-05) ────────────────────
    // A `TypedObject` whose schema is the canonical `Json` ADT
    // (`std::core::json_value`) is the output of `json.parse` (and the
    // yaml/toml/msgpack parse siblings). Such a node must be INVERTED back
    // to its logical `JsonValue` — NOT walked as a plain struct (which would
    // leak the `{__variant, __payload_0}` enum representation) and NOT
    // resolved via the schema's by-id index (which can collide: observed
    // `get_by_id(31) -> "std::core::state::Frame"` while `get("Json").id ==
    // 31`). The by-name `"Json".id == schema_id` check is the reliable
    // discriminator, and the stamped `field_kinds` track fully describes the
    // payload — the same single-discriminator contract the node was built
    // with (`project_json_value_to_slot` / `build_json_enum_heap_value`).
    if let Some(json_id) = resolve_json_schema_id(schemas) {
        if json_id == schema_id {
            // The by-name `"Json".id == schema_id` gate is the primary
            // discriminator, but under a registry by-id collision (observed
            // in JIT: an object literal's anonymous schema can be assigned the
            // SAME id as the `Json` enum) a non-Json TypedObject can pass that
            // gate. The structural guard below (exact 2-slot layout +
            // Int64 variant discriminant + variant↔payload-kind consistency)
            // rejects such collisions so a plain object is walked as a struct
            // rather than mis-inverted as a Json enum node.
            if let Some(result) = try_json_enum_node(slots, field_kinds, schemas) {
                return result;
            }
        }
    }

    let schema = resolve_typed_object_schema(schema_id, slots.len(), schemas).ok_or_else(|| {
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
            (_, true) => {
                // Prefer the runtime-stamped kind (ADR-006 §2.7.7 parallel
                // `field_kinds` track). Fall back to the schema `FieldType`'s
                // static projection only when the table is short/empty (e.g.
                // json-parse-built TypedObjects that pass an empty
                // `field_kinds` vector — the historical "exceeds field_kinds
                // length 0" failure). Never fabricate a kind from slot bits.
                let field_kind = match field_kinds.get(idx).copied() {
                    Some(k) => k,
                    None => field.field_type.to_native_kind().map_err(|_| {
                        format!(
                            "heap_to_json_value: TypedObject field '{}' is heap-resident \
                             but the field_kinds table is short (len={}) and FieldType {} \
                             has no static NativeKind projection",
                            field.name,
                            field_kinds.len(),
                            field.field_type
                        )
                    })?,
                };
                typed_object_heap_field_to_json_value(slot.raw(), field_kind, &field.name, schemas)?
            }
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

fn typed_object_heap_field_to_json_value(
    bits: u64,
    kind: NativeKind,
    field_name: &str,
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Result<JsonValue, String> {
    match kind {
        NativeKind::Null => Ok(JsonValue::Null),
        NativeKind::String => {
            if bits == 0 {
                return Err(format!(
                    "heap_to_json_value: TypedObject field '{}' has null String carrier",
                    field_name
                ));
            }
            let s = unsafe { &*(bits as *const String) };
            Ok(JsonValue::String(s.clone()))
        }
        NativeKind::StringV2 => {
            if bits == 0 {
                return Err(format!(
                    "heap_to_json_value: TypedObject field '{}' has null StringV2 carrier",
                    field_name
                ));
            }
            let ptr = bits as *const shape_value::v2::string_obj::StringObj;
            Ok(JsonValue::String(unsafe {
                shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned()
            }))
        }
        NativeKind::Ptr(HeapKind::TypedObject) => {
            if bits == 0 {
                return Err(format!(
                    "heap_to_json_value: TypedObject field '{}' has null TypedObject carrier",
                    field_name
                ));
            }
            let ptr = TypedObjectPtr::new(bits as *const TypedObjectStorage);
            let out = typed_object_ptr_to_json_value_with_registry_opt(&ptr, schemas);
            std::mem::forget(ptr);
            out
        }
        NativeKind::Ptr(HeapKind::HashMap) => {
            if bits == 0 {
                return Err(format!(
                    "heap_to_json_value: TypedObject field '{}' has null HashMap carrier",
                    field_name
                ));
            }
            let kref: &HashMapKindedRef = unsafe { &*(bits as *const HashMapKindedRef) };
            hashmap_kref_to_json_value_with_registry(kref, schemas)
        }
        NativeKind::Ptr(HeapKind::BigInt) => {
            if bits == 0 {
                return Err(format!(
                    "heap_to_json_value: TypedObject field '{}' has null BigInt carrier",
                    field_name
                ));
            }
            let value = unsafe { &*(bits as *const i64) };
            Ok(JsonValue::Int(*value))
        }
        NativeKind::Ptr(HeapKind::Char) | NativeKind::Char => {
            let c = char::from_u32(bits as u32).ok_or_else(|| {
                format!(
                    "heap_to_json_value: TypedObject field '{}' has invalid char bits {}",
                    field_name, bits
                )
            })?;
            Ok(JsonValue::String(c.to_string()))
        }
        // WF-2E (2026-07-05): a `Ptr(HeapKind::TypedArray)` field carries a
        // v2-raw `*mut TypedArray<T>`. Route it through the shared
        // element-type-stamped array reader (empty array on null carrier).
        NativeKind::Ptr(HeapKind::TypedArray) => {
            typed_array_to_json_value(bits as usize as *const u8, schemas)
        }
        other => Err(format!(
            "heap_to_json_value: TypedObject field '{}' has heap NativeKind {:?} with no JSON serialization policy",
            field_name, other
        )),
    }
}

// ───────────────────────── slot → JsonValue (canonical) ─────────────────────
//
// WF-2E shared object-graph marshal foundation (2026-07-05). This is the
// canonical direction-1 entry: read the Shape value carried in a
// `KindedSlot` into a `JsonValue` tree, dispatching on the STAMPED
// `NativeKind` (ADR-006 §2.7.7 parallel-kind track — the single
// discriminator). It NEVER blind-casts slot bits to a `HeapValue` /
// `HashMapKindedRef` / `TypedObjectStorage` pointer based on a *declared*
// (as opposed to *actual*) kind — that unsound reinterpretation is exactly
// the SIGSEGV class this replaces (an object literal is a
// `Ptr(HeapKind::TypedObject)` carrier; reading it as `HashMapKindedRef`
// segfaults). Every heap arm below reads the concrete carrier whose kind
// the slot is actually stamped with.

/// Read the Shape value carried in `slot` into a `JsonValue`, dispatching
/// on the slot's stamped `NativeKind`. `schemas` (when `Some`) is the
/// active program's schema registry, used to resolve `TypedObject` field
/// names; when `None`, the process/thread ambient registry
/// (`lookup_schema_by_id_public`) is consulted.
///
/// Scalars are read inline; heap carriers are read through their concrete
/// per-`NativeKind` pointer shape — no `as_heap_value()` blind-cast.
pub fn slot_to_json_value(
    slot: &shape_value::KindedSlot,
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Result<JsonValue, String> {
    let bits = slot.raw();
    match slot.kind() {
        NativeKind::Null => Ok(JsonValue::Null),
        NativeKind::Bool => Ok(JsonValue::Bool(bits != 0)),
        NativeKind::Float64 => Ok(JsonValue::Number(f64::from_bits(bits))),
        NativeKind::Float32 => Ok(JsonValue::Number(f32::from_bits(bits as u32) as f64)),
        NativeKind::NullableFloat64 => {
            let v = f64::from_bits(bits);
            if v.is_nan() {
                Ok(JsonValue::Null)
            } else {
                Ok(JsonValue::Number(v))
            }
        }
        NativeKind::Char => {
            let c = slot
                .as_char()
                .ok_or_else(|| "slot_to_json_value: invalid char bits".to_string())?;
            Ok(JsonValue::String(c.to_string()))
        }
        NativeKind::String | NativeKind::StringV2 => {
            if bits == 0 {
                Ok(JsonValue::Null)
            } else {
                Ok(JsonValue::String(slot.as_str().unwrap_or("").to_string()))
            }
        }
        NativeKind::DecimalV2 => Err(
            "Decimal serialization policy not yet decided (precision preservation vs lossy \
             f64 cast) — surface-and-stop"
                .to_string(),
        ),
        // Integer family (signed/unsigned/sized/nullable-present): a JSON
        // number. Null-valued nullable integers are stamped `NativeKind::Null`
        // (R5b-2) and take the `Null` arm above.
        k if k.is_integer_family() => Ok(JsonValue::Int(bits as i64)),
        NativeKind::Ptr(hk) => slot_ptr_to_json_value(bits, hk, schemas),
        other => Err(format!(
            "slot_to_json_value: NativeKind {:?} has no JSON serialization policy",
            other
        )),
    }
}

/// Heap-carrier arm of [`slot_to_json_value`]. Each `HeapKind` reads the
/// concrete pointer shape stamped for it — no cross-kind reinterpretation.
fn slot_ptr_to_json_value(
    bits: u64,
    hk: HeapKind,
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Result<JsonValue, String> {
    if bits == 0 {
        // A null carrier serializes to JSON null, except a null TypedArray
        // carrier is the empty array (mirrors the non-null TypedArray arm).
        // This is NOT a per-HeapKind serialization dispatch — every other
        // kind's null projection is uniformly `null` — so it stays an `if`,
        // not a `match hk` (which would demand an exhaustive per-variant arm
        // for no semantic gain, and trips the HeapKind-wildcard guard).
        return if hk == HeapKind::TypedArray {
            Ok(JsonValue::Array(Vec::new()))
        } else {
            Ok(JsonValue::Null)
        };
    }
    match hk {
        HeapKind::TypedObject => {
            // Wrap in a borrow-only `TypedObjectPtr` (no retain), read, then
            // forget so the caller-owned slot share is untouched — mirror of
            // `typed_object_heap_field_to_json_value`'s TypedObject arm.
            let ptr = TypedObjectPtr::new(bits as *const TypedObjectStorage);
            let out = typed_object_ptr_to_json_value_with_registry_opt(&ptr, schemas);
            std::mem::forget(ptr);
            out
        }
        HeapKind::HashMap => {
            let kref: &HashMapKindedRef = unsafe { &*(bits as *const HashMapKindedRef) };
            hashmap_kref_to_json_value_with_registry(kref, schemas)
        }
        HeapKind::TypedArray => typed_array_to_json_value(bits as usize as *const u8, schemas),
        HeapKind::BigInt => {
            let v = unsafe { &*(bits as *const i64) };
            Ok(JsonValue::Int(*v))
        }
        HeapKind::String => {
            let s = unsafe { &*(bits as *const String) };
            Ok(JsonValue::String(s.clone()))
        }
        HeapKind::Char => {
            let c = char::from_u32(bits as u32).ok_or_else(|| {
                "slot_to_json_value: invalid char codepoint in Ptr(Char) carrier".to_string()
            })?;
            Ok(JsonValue::String(c.to_string()))
        }
        HeapKind::Decimal => Err(
            "Decimal serialization policy not yet decided (N7 architectural-choice deferral)"
                .to_string(),
        ),
        // Exhaustive no-JSON-policy tail (WF-2E, 2026-07-05): every remaining
        // HeapKind is listed explicitly rather than swept into a `_` wildcard,
        // so introducing a new HeapKind variant is a compile error here and
        // forces a conscious JSON-serialization decision (HeapKind-wildcard
        // guard, `scripts/check-heapkind-wildcards.sh`).
        HeapKind::Closure
        | HeapKind::DataTable
        | HeapKind::Future
        | HeapKind::TaskGroup
        | HeapKind::Temporal
        | HeapKind::TableView
        | HeapKind::Content
        | HeapKind::Instant
        | HeapKind::IoHandle
        | HeapKind::NativeScalar
        | HeapKind::NativeView
        | HeapKind::FilterExpr
        | HeapKind::Reference
        | HeapKind::SharedCell
        | HeapKind::HashSet
        | HeapKind::Iterator
        | HeapKind::Deque
        | HeapKind::Channel
        | HeapKind::PriorityQueue
        | HeapKind::Range
        | HeapKind::Result
        | HeapKind::Option
        | HeapKind::TraitObject
        | HeapKind::Mutex
        | HeapKind::Atomic
        | HeapKind::Lazy
        | HeapKind::ModuleFn
        | HeapKind::Matrix
        | HeapKind::MatrixSlice => Err(format!(
            "slot_to_json_value: heap kind {:?} has no JSON serialization policy",
            hk
        )),
    }
}

/// Read a v2-raw `*mut TypedArray<T>` (base `*const u8`, element-type stamped
/// at header offset 7) into a `JsonValue::Array`, dispatching on the
/// producer-stamped element discriminant (never inferred from payload bits).
/// Recurses for nested `TypedObject` / `TypedArray` elements.
fn typed_array_to_json_value(
    base: *const u8,
    schemas: Option<&crate::type_schema::TypeSchemaRegistry>,
) -> Result<JsonValue, String> {
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::{
        ELEM_TYPE_BOOL, ELEM_TYPE_CHAR, ELEM_TYPE_DECIMAL, ELEM_TYPE_F32, ELEM_TYPE_F64,
        ELEM_TYPE_I8, ELEM_TYPE_I16, ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_STRING,
        ELEM_TYPE_TYPED_ARRAY, ELEM_TYPE_TYPED_OBJECT, ELEM_TYPE_U8, ELEM_TYPE_U16, ELEM_TYPE_U32,
        TypedArray, TypedArrayElem, read_elem_type,
    };

    if base.is_null() {
        return Ok(JsonValue::Array(Vec::new()));
    }
    let elem_type = unsafe { read_elem_type(base) };
    // SAFETY: `base` is a live `*mut TypedArray<T>` per the caller's stamped
    // `Ptr(HeapKind::TypedArray)` kind; the element-type byte selects the
    // monomorphization to read. Each `as_slice` borrows the array's element
    // buffer for the duration of the copy into `JsonValue` leaves.
    let out: Vec<JsonValue> = unsafe {
        match elem_type {
            ELEM_TYPE_F64 => TypedArray::<f64>::as_slice(base as *const TypedArray<f64>)
                .iter()
                .map(|&v| JsonValue::Number(v))
                .collect(),
            ELEM_TYPE_F32 => TypedArray::<f32>::as_slice(base as *const TypedArray<f32>)
                .iter()
                .map(|&v| JsonValue::Number(v as f64))
                .collect(),
            ELEM_TYPE_I64 => TypedArray::<i64>::as_slice(base as *const TypedArray<i64>)
                .iter()
                .map(|&v| JsonValue::Int(v))
                .collect(),
            ELEM_TYPE_I32 => TypedArray::<i32>::as_slice(base as *const TypedArray<i32>)
                .iter()
                .map(|&v| JsonValue::Int(v as i64))
                .collect(),
            ELEM_TYPE_I16 => TypedArray::<i16>::as_slice(base as *const TypedArray<i16>)
                .iter()
                .map(|&v| JsonValue::Int(v as i64))
                .collect(),
            ELEM_TYPE_U16 => TypedArray::<u16>::as_slice(base as *const TypedArray<u16>)
                .iter()
                .map(|&v| JsonValue::Int(v as i64))
                .collect(),
            ELEM_TYPE_U32 => TypedArray::<u32>::as_slice(base as *const TypedArray<u32>)
                .iter()
                .map(|&v| JsonValue::Int(v as i64))
                .collect(),
            ELEM_TYPE_I8 => TypedArray::<i8>::as_slice(base as *const TypedArray<i8>)
                .iter()
                .map(|&v| JsonValue::Int(v as i64))
                .collect(),
            // BOOL and U8 share the 1-byte `TypedArray<u8>` storage; the stamp
            // distinguishes their JSON projection.
            ELEM_TYPE_U8 => TypedArray::<u8>::as_slice(base as *const TypedArray<u8>)
                .iter()
                .map(|&v| JsonValue::Int(v as i64))
                .collect(),
            ELEM_TYPE_BOOL => TypedArray::<u8>::as_slice(base as *const TypedArray<u8>)
                .iter()
                .map(|&v| JsonValue::Bool(v != 0))
                .collect(),
            ELEM_TYPE_CHAR => TypedArray::<char>::as_slice(base as *const TypedArray<char>)
                .iter()
                .map(|&c| JsonValue::String(c.to_string()))
                .collect(),
            ELEM_TYPE_STRING => {
                TypedArray::<*const StringObj>::as_slice(base as *const TypedArray<*const StringObj>)
                    .iter()
                    .map(|&p| JsonValue::String(StringObj::as_str(p).to_owned()))
                    .collect()
            }
            ELEM_TYPE_TYPED_OBJECT => {
                let slice = TypedArray::<*const TypedObjectStorage>::as_slice(
                    base as *const TypedArray<*const TypedObjectStorage>,
                );
                let mut rows = Vec::with_capacity(slice.len());
                for &p in slice.iter() {
                    let ptr = TypedObjectPtr::new(p);
                    let child = typed_object_ptr_to_json_value_with_registry_opt(&ptr, schemas);
                    std::mem::forget(ptr);
                    rows.push(child?);
                }
                rows
            }
            ELEM_TYPE_TYPED_ARRAY => {
                let slice = TypedArray::<*const TypedArrayElem>::as_slice(
                    base as *const TypedArray<*const TypedArrayElem>,
                );
                let mut rows = Vec::with_capacity(slice.len());
                for &row_ptr in slice.iter() {
                    if row_ptr.is_null() {
                        rows.push(JsonValue::Array(Vec::new()));
                    } else {
                        rows.push(typed_array_to_json_value(row_ptr as *const u8, schemas)?);
                    }
                }
                rows
            }
            ELEM_TYPE_DECIMAL => {
                return Err(
                    "Array<decimal> serialization policy not yet decided (precision vs lossy \
                     f64) — surface-and-stop"
                        .to_string(),
                );
            }
            other => {
                return Err(format!(
                    "typed_array_to_json_value: element-type discriminant {} has no JSON \
                     serialization policy (unstamped array or unsupported element kind)",
                    other
                ));
            }
        }
    };
    Ok(JsonValue::Array(out))
}

/// Convert a `serde_json::Value` into the strict-typed `JsonValue` sum.
///
/// The shared "wire → intermediate" direction (the inverse of
/// [`json_value_to_serde_json`]). Parsers that decode to `serde_json::Value`
/// (json / msgpack-via-serde) funnel through here to reach the universal
/// `JsonValue` intermediate; from there
/// [`crate::stdlib::json::build_json_enum_heap_value`] projects to a Shape
/// runtime `HeapValue` (the "intermediate → Shape value" direction for
/// parse). Integral JSON numbers that fit `i64` map to `JsonValue::Int`;
/// all other numbers map to `JsonValue::Number`, preserving the
/// `int` / `number` distinction at the boundary.
pub fn serde_json_to_json_value(value: serde_json::Value) -> JsonValue {
    match value {
        serde_json::Value::Null => JsonValue::Null,
        serde_json::Value::Bool(b) => JsonValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if !n.to_string().contains('.') {
                    return JsonValue::Int(i);
                }
            }
            JsonValue::Number(n.as_f64().unwrap_or(0.0))
        }
        serde_json::Value::String(s) => JsonValue::String(s),
        serde_json::Value::Array(arr) => {
            JsonValue::Array(arr.into_iter().map(serde_json_to_json_value).collect())
        }
        serde_json::Value::Object(map) => JsonValue::Object(
            map.into_iter()
                .map(|(k, v)| (k, serde_json_to_json_value(v)))
                .collect(),
        ),
    }
}

/// Convert a `JsonValue` into a `serde_json::Value`.
///
/// Inverse of [`serde_json_to_json_value`].
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

#[cfg(test)]
mod tests {
    use super::{JsonValue, json_value_to_serde_json, slot_to_json_value, typed_object_ptr_to_json_value};
    use crate::type_schema::{SyncRegistryScope, TypeSchemaBuilder, TypeSchemaRegistry};
    use shape_value::heap_value::{TypedObjectPtr, TypedObjectStorage};
    use shape_value::{KindedSlot, NativeKind, ValueSlot};
    use std::sync::Arc;

    /// WF-2E: the canonical `slot_to_json_value` walks a `TypedObject`
    /// carried in a `KindedSlot` into a `JsonValue` tree — dispatching on
    /// the stamped `NativeKind`, reading the heap `String` field via its
    /// concrete carrier and the inline `Int64` field inline — WITHOUT the
    /// blind `HashMapKindedRef` reinterpretation that segfaulted before.
    #[test]
    fn slot_to_json_value_walks_typed_object_scalar_and_heap_fields() {
        let mut registry = TypeSchemaRegistry::new_with_stdlib();
        let schema_id = TypeSchemaBuilder::new("__WF2ESlotObj")
            .string_field("name")
            .i64_field("age")
            .register(&mut registry);
        let _scope = SyncRegistryScope::enter(Arc::new(registry));

        let name = Arc::new("Alice".to_string());
        // field 0 (name) is heap (String), field 1 (age) is inline (Int64).
        let ptr = TypedObjectStorage::_new(
            schema_id as u64,
            vec![
                ValueSlot::from_string_arc(Arc::clone(&name)),
                ValueSlot::from_int(30),
            ]
            .into_boxed_slice(),
            0b01,
            Arc::from(vec![NativeKind::String, NativeKind::Int64].into_boxed_slice()),
        );
        // KindedSlot owns the `_new` share; its Drop releases it at scope end.
        let slot = KindedSlot::from_typed_object_raw(ptr);

        let jv = slot_to_json_value(&slot, None).expect("slot_to_json_value on TypedObject");
        match &jv {
            JsonValue::Object(pairs) => {
                assert_eq!(pairs.len(), 2);
            }
            other => panic!("expected JsonValue::Object, got {:?}", other),
        }
        let serde_value = json_value_to_serde_json(&jv);
        assert_eq!(
            serde_value["name"],
            serde_json::Value::String("Alice".to_string())
        );
        assert_eq!(serde_value["age"], serde_json::json!(30));
    }

    /// WF-2E: a scalar-carrying `KindedSlot` (int / bool / string / null)
    /// projects to the matching `JsonValue` leaf via the stamped kind.
    #[test]
    fn slot_to_json_value_reads_scalar_leaves() {
        assert_eq!(
            slot_to_json_value(&KindedSlot::from_int(7), None).unwrap(),
            JsonValue::Int(7)
        );
        assert_eq!(
            slot_to_json_value(&KindedSlot::from_bool(true), None).unwrap(),
            JsonValue::Bool(true)
        );
        assert_eq!(
            slot_to_json_value(&KindedSlot::none(), None).unwrap(),
            JsonValue::Null
        );
        assert_eq!(
            slot_to_json_value(&KindedSlot::from_string_arc(Arc::new("hi".to_string())), None)
                .unwrap(),
            JsonValue::String("hi".to_string())
        );
    }

    #[test]
    fn typed_object_string_field_serializes_from_direct_carrier() {
        let mut registry = TypeSchemaRegistry::new_with_stdlib();
        let schema_id = TypeSchemaBuilder::new("__W71HttpJsonBody")
            .string_field("key")
            .register(&mut registry);
        let _scope = SyncRegistryScope::enter(Arc::new(registry));

        let value = Arc::new("value".to_string());
        let ptr = TypedObjectStorage::_new(
            schema_id as u64,
            vec![ValueSlot::from_string_arc(Arc::clone(&value))].into_boxed_slice(),
            1,
            Arc::from(vec![NativeKind::String].into_boxed_slice()),
        );
        let object = TypedObjectPtr::new(ptr);

        assert_eq!(
            Arc::strong_count(&value),
            2,
            "TypedObject field slot must own one String share"
        );

        let json = typed_object_ptr_to_json_value(&object).expect("typed object to json");
        let serde_value = json_value_to_serde_json(&json);

        assert_eq!(
            serde_value["key"],
            serde_json::Value::String("value".to_string())
        );

        drop(object);
        assert_eq!(
            Arc::strong_count(&value),
            1,
            "dropping the TypedObjectPtr must release the field String share"
        );
    }
}
