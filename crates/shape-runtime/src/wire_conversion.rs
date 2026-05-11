//! Conversion between runtime values and wire format.
//!
//! Phase 2b kind-threaded rewrite. Public functions take `(bits: u64,
//! kind: NativeKind)` pairs threaded from the FunctionBlob's compile-
//! time slot-kind metadata; internal dispatch is a `match kind { ... }`
//! with no tag-bit probing. Heap slots use `NativeKind::Ptr(HeapKind)` —
//! the kind tells the dispatcher which `HeapValue` arm decodes the
//! bits without probing the heap object's self-reported discriminant
//! in production (debug-only consistency check).
//!
//! See `docs/defections.md` 2026-05-06 (Phase 2b unified marshal +
//! wire/snapshot kind threading) for the architectural rationale.
//!
//! ## API
//!
//! - [`slot_to_wire`] — project (bits, kind) into a `WireValue`.
//! - [`wire_to_slot`] — project a `WireValue` into typed slot bits,
//!   given the `expected_kind` the caller wants. Returns
//!   `Result<u64, MarshalError>`.
//! - [`slot_to_envelope`] — wrap a typed slot in a `ValueEnvelope` with
//!   metadata.
//! - [`slot_extract_content`] — extract Content node renderings from a
//!   slot whose kind says it carries Content / DataTable / TableView.
//! - [`datatable_to_wire`] / [`datatable_to_ipc_bytes`] /
//!   [`datatable_from_ipc_bytes`] — typed `DataTable` ↔ wire/IPC.

use crate::Context;
use crate::marshal::MarshalError;
use arrow_ipc::{reader::FileReader, writer::FileWriter};
use shape_value::heap_value::HeapValue;
use shape_value::{DataTable, HeapKind, NativeKind};
use shape_wire::{
    DurationUnit as WireDurationUnit, ValueEnvelope, WireTable, WireValue,
};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Project a typed slot's `(bits, kind)` to a `WireValue`.
///
/// The `kind` fully determines the projection — no tag-bit probing.
/// For `NativeKind::Ptr(hk)`, the function casts `bits` to
/// `*const HeapValue`, debug-asserts the kind matches, and dispatches
/// per `HeapValue` arm.
pub fn slot_to_wire(bits: u64, kind: NativeKind, ctx: &Context) -> WireValue {
    match kind {
        NativeKind::Float64 => WireValue::Number(f64::from_bits(bits)),
        NativeKind::NullableFloat64 => {
            let v = f64::from_bits(bits);
            if v.is_nan() {
                WireValue::Null
            } else {
                WireValue::Number(v)
            }
        }
        NativeKind::Int64 => WireValue::Integer(bits as i64),
        NativeKind::NullableInt64 => WireValue::Integer(bits as i64),
        NativeKind::Int8 => WireValue::I8(bits as i8),
        NativeKind::Int16 => WireValue::I16(bits as i16),
        NativeKind::Int32 => WireValue::I32(bits as i32),
        NativeKind::UInt8 => WireValue::U8(bits as u8),
        NativeKind::UInt16 => WireValue::U16(bits as u16),
        NativeKind::UInt32 => WireValue::U32(bits as u32),
        NativeKind::UInt64 => WireValue::U64(bits),
        NativeKind::IntSize => WireValue::Isize(bits as i64),
        NativeKind::UIntSize => WireValue::Usize(bits),
        NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUIntSize => WireValue::Integer(bits as i64),
        NativeKind::Bool => WireValue::Bool(bits != 0),
        NativeKind::String => {
            // bits is an Arc<String> raw pointer
            let ptr = bits as *const String;
            // SAFETY: kind contract pins this slot to an Arc<String> raw ptr.
            let s = unsafe { &*ptr };
            WireValue::String(s.clone())
        }
        NativeKind::Ptr(hk) => heap_to_wire(bits, hk, ctx),
    }
}

/// Project an `Arc<HeapValue>` raw pointer slot to `WireValue`,
/// dispatching on the pre-known `HeapKind` rather than probing the
/// heap object's self-reported `kind()`.
fn heap_to_wire(bits: u64, hk: HeapKind, ctx: &Context) -> WireValue {
    if bits == 0 {
        return WireValue::Null;
    }
    let ptr = bits as *const HeapValue;
    // SAFETY: NativeKind::Ptr(hk) contract — bits is a valid Arc<HeapValue> ptr.
    let hv = unsafe { &*ptr };
    debug_assert_eq!(
        hv.kind(),
        hk,
        "slot kind {:?} does not match HeapValue::{:?}",
        hk,
        hv.kind()
    );
    heap_value_to_wire(hv, ctx)
}

/// Project a `&HeapValue` to `WireValue` by dispatching on its
/// surviving variants. Reused by the snapshot path (Phase 2b
/// snapshot.rs commit) which has the same heap projection needs.
pub fn heap_value_to_wire(hv: &HeapValue, ctx: &Context) -> WireValue {
    match hv {
        HeapValue::String(s) => WireValue::String((**s).clone()),
        HeapValue::Decimal(d) => WireValue::Number(d.to_string().parse().unwrap_or(0.0)),
        HeapValue::BigInt(i) => WireValue::Integer(**i),
        HeapValue::Char(c) => WireValue::String(c.to_string()),
        HeapValue::Future(id) => WireValue::String(format!("<future:{}>", id)),
        HeapValue::DataTable(dt) => datatable_to_wire(dt.as_ref()),
        HeapValue::Content(_node) => {
            // Phase 1.B: the JSON-renderer integration for Content trees
            // is the deferred Phase 2c content-marshalling rebuild — see
            // ADR-006 §2.7.4. Until then, surface a placeholder
            // WireValue rather than emit a partial / wrong-shape
            // serialization.
            WireValue::String("<content:phase-2c-rebuild>".to_string())
        }
        HeapValue::Instant(t) => WireValue::String(format!("{:?}", **t)),
        HeapValue::IoHandle(_h) => {
            // Phase 1.B: IoHandleData no longer exposes a stable `id()`
            // accessor; the handle's identity is structural (the inner
            // OS resource) rather than a numeric tag. Phase 2c surfaces
            // a kind-threaded handle-printer.
            WireValue::String("<io_handle>".to_string())
        }
        HeapValue::NativeScalar(v) => match v {
            shape_value::heap_value::NativeScalar::I8(n) => WireValue::I8(*n),
            shape_value::heap_value::NativeScalar::U8(n) => WireValue::U8(*n),
            shape_value::heap_value::NativeScalar::I16(n) => WireValue::I16(*n),
            shape_value::heap_value::NativeScalar::U16(n) => WireValue::U16(*n),
            shape_value::heap_value::NativeScalar::I32(n) => WireValue::I32(*n),
            shape_value::heap_value::NativeScalar::I64(n) => WireValue::I64(*n),
            shape_value::heap_value::NativeScalar::U32(n) => WireValue::U32(*n),
            shape_value::heap_value::NativeScalar::U64(n) => WireValue::U64(*n),
            shape_value::heap_value::NativeScalar::Isize(n) => WireValue::Isize(*n as i64),
            shape_value::heap_value::NativeScalar::Usize(n) => WireValue::Usize(*n as u64),
            shape_value::heap_value::NativeScalar::Ptr(n) => WireValue::Ptr(*n as u64),
            shape_value::heap_value::NativeScalar::F32(n) => WireValue::F32(*n),
        },
        HeapValue::NativeView(v) => WireValue::Object(
            [
                (
                    "__type".to_string(),
                    WireValue::String(if v.mutable { "cmut" } else { "cview" }.to_string()),
                ),
                (
                    "layout".to_string(),
                    WireValue::String(v.layout.name.clone()),
                ),
                (
                    "ptr".to_string(),
                    WireValue::String(format!("0x{:x}", v.ptr)),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        HeapValue::TypedObject(storage) => {
            // ADR-005 §Forbidden / Q10 forward pointer: wire serialization
            // must NOT re-introduce Box<HeapValue> slot wrapping. The
            // schema-driven kind threading below is ADR-005-aligned (typed
            // slot bits + schema; no intermediate HeapValue materialization
            // on deserialization).
            let schema_id = storage.schema_id;
            let slots = &storage.slots;
            let schema = ctx
                .type_schema_registry()
                .get_by_id(schema_id as u32)
                .cloned()
                .or_else(|| crate::type_schema::lookup_schema_by_id_public(schema_id as u32));
            if let Some(schema) = schema {
                let mut map = BTreeMap::new();
                for field_def in &schema.fields {
                    let idx = field_def.index as usize;
                    if idx >= slots.len() {
                        continue;
                    }
                    let Some(field_kind) = schema.field_kind(idx) else {
                        continue;
                    };
                    let field_bits = slots[idx].raw();
                    let field_wire = slot_to_wire(field_bits, field_kind, ctx);
                    map.insert(field_def.name.clone(), field_wire);
                }
                WireValue::Object(map)
            } else {
                WireValue::String(format!("<typed_object:schema#{}>", schema_id))
            }
        }
        HeapValue::ClosureRaw(_handle) => {
            // Phase 1.B: OwnedClosureBlock no longer exposes a public
            // `function_id()` accessor on the runtime side (the typed-
            // closure slot ABI carries the function-id via the
            // `TypedClosureHeader` itself). Phase 2c lands a
            // schema-aware closure printer.
            WireValue::String("<closure>".to_string())
        }
        HeapValue::TaskGroup(_data) => {
            WireValue::String("<task_group>".to_string())
        }
        HeapValue::TypedArray(arc) => typed_array_to_wire(&**arc),
        HeapValue::Temporal(td) => temporal_to_wire(&**td),
        HeapValue::TableView(tv) => match &**tv {
            shape_value::heap_value::TableViewData::TypedTable { table, schema_id } => {
                datatable_to_wire_with_schema(table.as_ref(), Some(*schema_id as u32))
            }
            shape_value::heap_value::TableViewData::IndexedTable { table, .. } => {
                datatable_to_wire(table.as_ref())
            }
            shape_value::heap_value::TableViewData::RowView { .. }
            | shape_value::heap_value::TableViewData::ColumnRef { .. } => {
                WireValue::String("<table_view:phase-2c>".to_string())
            }
        },
        HeapValue::HashMap(_) => {
            // Phase 1.B (ADR-006 §2.7.4): kind-threaded HashMap-to-wire
            // serialization is the deferred Phase 2c marshal rebuild.
            WireValue::String("<hashmap:phase-2c>".to_string())
        }
        // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
        // 2026-05-10): Set wire serialization follows the same
        // phase-2c deferral shape as HashMap; surface as an opaque
        // tag until the marshal rebuild lands.
        HeapValue::HashSet(_) => WireValue::String("<hashset:phase-2c>".to_string()),
        // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10):
        // Deque wire serialization follows the same phase-2c deferral
        // shape as HashMap / HashSet — opaque tag until the marshal
        // rebuild lands.
        HeapValue::Deque(_) => WireValue::String("<deque:phase-2c>".to_string()),
        // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / Q8 amendment):
        // FilterExpr trees are transient query-DSL values; they don't
        // cross the wire boundary today. Surface as an opaque tag.
        HeapValue::FilterExpr(_) => WireValue::String("<filter_expr>".to_string()),
        // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26, 2026-05-10): Reference
        // values are within-program data and never cross the wire
        // boundary. Surface as an opaque tag, same as FilterExpr.
        HeapValue::Reference(_) => WireValue::String("<ref>".to_string()),
        // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10):
        // Iterator pipelines are lazy within-program values and never
        // cross the wire boundary (callers materialise via collect /
        // forEach / etc. before serialisation). Surface as an opaque
        // tag, same as FilterExpr / Reference.
        HeapValue::Iterator(_) => WireValue::String("<iterator>".to_string()),
        // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21, 2026-05-10):
        // channels are concurrency primitives with interior
        // `Mutex<ChannelInner>` state; no wire serialization at landing —
        // same phase-2c deferral shape as HashMap / HashSet. Surface as
        // an opaque tag for diagnostics.
        HeapValue::Channel(_) => WireValue::String("<channel:phase-2c>".to_string()),
        // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
        // 2026-05-10): PriorityQueue wire serialisation projects to a
        // `WireValue::Array` of i64 priorities in heap-array order
        // (mirror of the JSON shape — i64-priority-only at landing).
        HeapValue::PriorityQueue(d) => WireValue::Array(
            d.heap
                .data
                .iter()
                .map(|v| WireValue::Integer(*v))
                .collect(),
        ),
        // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10): Range
        // serializes as a JSON-ish `{"start", "end", "step",
        // "inclusive"}` payload via the `as_array_for_wire` shape
        // (range bounds + step are tiny scalars; lossless round-trip).
        // Wire serialization here just stamps the literal-form string
        // — full structured wire is the deferred Phase 2c marshal
        // rebuild same as HashMap / HashSet (which surface as opaque
        // tags above). Matches the playbook's "wire/JSON conversion
        // arms (rejection or proper)" guidance.
        HeapValue::Range(r) => {
            let s = if r.inclusive {
                format!("{}..={}", r.start, r.end)
            } else {
                format!("{}..{}", r.start, r.end)
            };
            WireValue::String(s)
        }
        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10):
        // Result/Option carriers are within-program control-flow values;
        // wire serialisation goes through the AnyError schema for thrown
        // errors and the unwrapped inner value for `Ok(_)` / `Some(_)`.
        // Until those marshal paths land, surface as an opaque tag —
        // same Phase-2c deferral shape as HashMap / HashSet / Iterator.
        HeapValue::Result(_) => WireValue::String("<result:phase-2c>".to_string()),
        HeapValue::Option(_) => WireValue::String("<option:phase-2c>".to_string()),
        // W17-concurrency (ADR-006 §2.7.25, 2026-05-11): concurrency
        // primitives are runtime-tier handles with no wire shape.
        // Surface as opaque tags — same Phase-2c deferral shape as
        // Channel / HashMap / HashSet.
        HeapValue::Mutex(_) => WireValue::String("<mutex:phase-2c>".to_string()),
        HeapValue::Atomic(_) => WireValue::String("<atomic:phase-2c>".to_string()),
        HeapValue::Lazy(_) => WireValue::String("<lazy:phase-2c>".to_string()),
        // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C, 2026-05-11):
        // `dyn Trait` carriers have no wire shape — same Phase-2c
        // deferral as concurrency primitives. A future `Serializable`
        // trait could route through the vtable, but that's emission-tier
        // work outside this sub-cluster.
        HeapValue::TraitObject(_) => WireValue::String("<trait_object:phase-2c>".to_string()),
    }
}

fn typed_array_to_wire(ta: &shape_value::heap_value::TypedArrayData) -> WireValue {
    use shape_value::heap_value::TypedArrayData;
    match ta {
        TypedArrayData::I64(buf) => {
            WireValue::Array(buf.iter().map(|v| WireValue::Integer(*v)).collect())
        }
        TypedArrayData::F64(buf) => {
            WireValue::Array(buf.iter().map(|v| WireValue::Number(*v)).collect())
        }
        TypedArrayData::Bool(buf) => {
            WireValue::Array(buf.iter().map(|v| WireValue::Bool(*v != 0)).collect())
        }
        TypedArrayData::String(buf) => WireValue::Array(
            buf.iter()
                .map(|s| WireValue::String((**s).clone()))
                .collect(),
        ),
        // Other TypedArrayData variants (Matrix / I8/I16/I32/U8/U16/U32/U64/
        // F32/HeapValue) — wire serialization is part of the deferred
        // Phase 2c marshal-layer rebuild. Surface a placeholder rather
        // than emit a wrong-shape array.
        _ => WireValue::String("<typed_array:phase-2c>".to_string()),
    }
}

fn temporal_to_wire(td: &shape_value::heap_value::TemporalData) -> WireValue {
    use shape_value::heap_value::TemporalData;
    match td {
        TemporalData::DateTime(dt) => WireValue::Timestamp(dt.timestamp_millis()),
        TemporalData::TimeSpan(d) => WireValue::Duration {
            value: d.num_milliseconds() as f64,
            unit: WireDurationUnit::Milliseconds,
        },
        TemporalData::Duration(d) => WireValue::Duration {
            value: d.value,
            unit: WireDurationUnit::Milliseconds,
        },
        TemporalData::Timeframe(_)
        | TemporalData::TimeReference(_)
        | TemporalData::DateTimeExpr(_)
        | TemporalData::DataDateTimeRef(_) => WireValue::String(format!("<{}>", td.type_name())),
    }
}

/// Project a `WireValue` to typed slot bits, given the kind the caller
/// wants. Returns [`MarshalError::KindMismatch`] when wire shape doesn't
/// match the expected kind.
///
/// For heap kinds, this allocates a new `Arc<HeapValue>` and returns
/// the raw pointer as bits — caller takes ownership of the heap
/// reference (one strong count).
pub fn wire_to_slot(wire: &WireValue, expected_kind: NativeKind) -> Result<u64, MarshalError> {
    match (wire, expected_kind) {
        (WireValue::Number(n), NativeKind::Float64) => Ok(f64::to_bits(*n)),
        (WireValue::Integer(i), NativeKind::Int64) => Ok(*i as u64),
        (WireValue::Bool(b), NativeKind::Bool) => Ok(*b as u64),
        (WireValue::Null, NativeKind::NullableFloat64) => Ok(f64::to_bits(f64::NAN)),
        (WireValue::String(s), NativeKind::String) => {
            let arc = Arc::new(s.clone());
            Ok(Arc::into_raw(arc) as u64)
        }
        (WireValue::I8(n), NativeKind::Int8) => Ok((*n as i64) as u64),
        (WireValue::I16(n), NativeKind::Int16) => Ok((*n as i64) as u64),
        (WireValue::I32(n), NativeKind::Int32) => Ok((*n as i64) as u64),
        (WireValue::U8(n), NativeKind::UInt8) => Ok(*n as u64),
        (WireValue::U16(n), NativeKind::UInt16) => Ok(*n as u64),
        (WireValue::U32(n), NativeKind::UInt32) => Ok(*n as u64),
        (WireValue::U64(n), NativeKind::UInt64) => Ok(*n),
        // Heap kinds are constructed by allocating Arc<HeapValue> with the
        // matching variant. Each surviving HeapKind variant is handled here
        // as stdlib mass migration (Phase 2c) and the snapshot replay path
        // discover concrete consumers.
        (WireValue::String(s), NativeKind::Ptr(HeapKind::String)) => {
            let arc = Arc::new(HeapValue::String(Arc::new(s.clone())));
            Ok(Arc::into_raw(arc) as u64)
        }
        (WireValue::Table(table), NativeKind::Ptr(HeapKind::DataTable)) => {
            let dt = datatable_from_ipc_bytes(&table.ipc_bytes, None, None)
                .map_err(MarshalError::Body)?;
            let arc = Arc::new(HeapValue::DataTable(Arc::new(dt)));
            Ok(Arc::into_raw(arc) as u64)
        }
        // Calling site passed a wire/kind pair we don't currently handle.
        // The strict-typed answer is to extend this match, not fall back —
        // each new case represents a concrete stdlib/wire shape, and
        // pattern-match exhaustiveness is the discipline.
        _ => Err(MarshalError::Body(format!(
            "wire_to_slot: no projection for wire variant into kind {:?}",
            expected_kind
        ))),
    }
}

/// Wrap a typed slot in a `ValueEnvelope` with optional metadata.
///
/// `type_name` is the user-facing Shape type name (e.g. `"int"`,
/// `"DataTable"`, `"MyType"`). The envelope's `type_info` is populated
/// from the type registry when available.
pub fn slot_to_envelope(
    bits: u64,
    kind: NativeKind,
    type_name: &str,
    ctx: &Context,
) -> ValueEnvelope {
    let value = slot_to_wire(bits, kind, ctx);
    let _ = type_name;
    let _ = ctx;
    // Phase 1.B (ADR-006 §2.7.4): the type-info / type-registry lookup
    // path that resolved a `TypeRegistry` from `TypeRegistry::default()`
    // is gone; the rebuilt path queries `TypeRegistry::for_number` /
    // primitives + the runtime's per-schema cache. Until the kind-
    // threaded envelope lookup lands in Phase 2c, fall back to the
    // wire-side inference helper.
    ValueEnvelope::from_value(value)
}

/// If the slot carries a renderable Content shape (Content node, DataTable,
/// or TableView), return `(content_json, content_html, content_terminal)`.
/// Otherwise all three are `None`.
pub fn slot_extract_content(
    bits: u64,
    kind: NativeKind,
) -> (Option<serde_json::Value>, Option<String>, Option<String>) {
    let NativeKind::Ptr(hk) = kind else {
        return (None, None, None);
    };
    if bits == 0 {
        return (None, None, None);
    }
    let hv = unsafe { &*(bits as *const HeapValue) };
    let node = match (hk, hv) {
        (HeapKind::Content, HeapValue::Content(node)) => Some((**node).clone()),
        (HeapKind::DataTable, HeapValue::DataTable(dt)) => Some(
            crate::content_dispatch::datatable_to_content_node(dt.as_ref(), None),
        ),
        (HeapKind::TableView, HeapValue::TableView(arc)) => match &**arc {
            shape_value::heap_value::TableViewData::TypedTable { table, .. }
            | shape_value::heap_value::TableViewData::IndexedTable { table, .. } => Some(
                crate::content_dispatch::datatable_to_content_node(table.as_ref(), None),
            ),
            // RowView / ColumnRef are deferred Phase 2c content
            // adapters — no current renderer.
            _ => None,
        },
        _ => None,
    };
    let Some(_node) = node else {
        return (None, None, None);
    };

    // Phase 1.B (ADR-006 §2.7.4): the JSON / HTML / terminal renderer
    // adapters for `ContentNode` are part of the deferred Phase 2c
    // content-marshal rebuild. Until then, return `None` for all three
    // payloads rather than emit a partial / wrong-shape rendering.
    (None, None, None)
}

// ───────────────────────── DataTable ↔ wire/IPC ─────────────────────────
//
// Typed-handle conversions. These don't go through `(bits, kind)` —
// the caller passes a `&DataTable` directly, which is the typed-Rust
// equivalent of NativeKind::Ptr(HeapKind::DataTable). The marshal layer
// uses these internally when projecting a DataTable slot.

pub fn datatable_to_wire(dt: &DataTable) -> WireValue {
    datatable_to_wire_with_schema(dt, dt.schema_id())
}

fn datatable_to_wire_with_schema(dt: &DataTable, schema_id: Option<u32>) -> WireValue {
    match datatable_to_ipc_bytes(dt) {
        Ok(ipc_bytes) => WireValue::Table(WireTable {
            ipc_bytes,
            type_name: None,
            schema_id,
            row_count: dt.row_count(),
            column_count: dt.column_count(),
        }),
        Err(e) => WireValue::String(format!("<datatable_serialize_error: {}>", e)),
    }
}

pub fn datatable_to_ipc_bytes(dt: &DataTable) -> std::result::Result<Vec<u8>, String> {
    // The DataTable now wraps a `RecordBatch` directly (`inner()`); the
    // pre-bulldozer `to_arrow_batch` accessor is gone since the wrapper
    // is the batch.
    let arrow_batch = dt.inner();
    let schema = arrow_batch.schema();
    let mut buf = Vec::new();
    {
        let mut writer = FileWriter::try_new(&mut buf, &schema)
            .map_err(|e| format!("Arrow IPC writer init failed: {}", e))?;
        writer
            .write(arrow_batch)
            .map_err(|e| format!("Arrow IPC write failed: {}", e))?;
        writer
            .finish()
            .map_err(|e| format!("Arrow IPC finish failed: {}", e))?;
    }
    Ok(buf)
}

pub fn datatable_from_ipc_bytes(
    bytes: &[u8],
    column_overrides: Option<&[shape_value::datatable::ColumnPtrs]>,
    schema_id_override: Option<u32>,
) -> std::result::Result<DataTable, String> {
    let cursor = std::io::Cursor::new(bytes);
    let reader = FileReader::try_new(cursor, None)
        .map_err(|e| format!("Arrow IPC reader init failed: {}", e))?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(|e| format!("Arrow IPC batch read failed: {}", e))?);
    }
    if batches.is_empty() {
        return Err(
            "datatable_from_ipc_bytes: empty IPC stream — no Arrow RecordBatch to wrap".to_string(),
        );
    }
    // The first batch is the canonical wrapper; concatenation is a
    // Phase 2c rebuild item alongside the broader DataTable IPC layer.
    let first = batches.into_iter().next().unwrap();
    let _ = column_overrides;
    let dt = DataTable::new(first);
    let dt = if let Some(sid) = schema_id_override {
        dt.with_schema_id(sid)
    } else {
        dt
    };
    Ok(dt)
}
