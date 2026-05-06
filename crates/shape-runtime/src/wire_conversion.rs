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
    metadata::{TypeInfo, TypeRegistry},
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
        HeapValue::BigInt(i) => WireValue::Integer(*i),
        HeapValue::Char(c) => WireValue::String(c.to_string()),
        HeapValue::Future(id) => WireValue::String(format!("<future:{}>", id)),
        HeapValue::DataTable(dt) => datatable_to_wire(dt.as_ref()),
        HeapValue::Content(node) => {
            // Render to JSON for wire transport (the canonical lossless form).
            use crate::renderers::json::JsonRenderer;
            let j = JsonRenderer.render(node);
            serde_json::from_str(&j).map(WireValue::Json).unwrap_or(WireValue::Null)
        }
        HeapValue::Instant(t) => WireValue::String(format!("{:?}", **t)),
        HeapValue::IoHandle(h) => WireValue::String(format!("<io_handle:{}>", h.id())),
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
        HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask: _,
        } => {
            // Schema-driven kind threading (Phase 2b): for each field, ask the
            // schema for the field's NativeKind and project that slot directly.
            let schema = ctx
                .type_schema_registry()
                .get_by_id(*schema_id as u32)
                .cloned()
                .or_else(|| crate::type_schema::lookup_schema_by_id_public(*schema_id as u32));
            if let Some(schema) = schema {
                let mut map = BTreeMap::new();
                for field_def in &schema.fields {
                    let idx = field_def.index as usize;
                    if idx >= slots.len() {
                        continue;
                    }
                    let Some(field_kind) = schema.field_kind(idx) else {
                        // FieldType::Any has no strict-typed projection;
                        // skip the field rather than fall back to dynamic
                        // dispatch. Strict-typed schemas should not have
                        // Any fields — see docs/defections.md watchlist.
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
        HeapValue::ClosureRaw(handle) => {
            WireValue::String(format!("<closure:fn#{}>", handle.function_id()))
        }
        HeapValue::TaskGroup { kind, task_ids } => WireValue::String(format!(
            "<task_group:kind={} count={}>",
            kind,
            task_ids.len()
        )),
        HeapValue::TypedArray(ta) => typed_array_to_wire(ta),
        HeapValue::Temporal(td) => temporal_to_wire(td),
        HeapValue::TableView(tv) => match tv {
            shape_value::heap_value::TableViewData::TypedTable { table, schema_id } => {
                datatable_to_wire_with_schema(table.as_ref(), Some(*schema_id as u32))
            }
            shape_value::heap_value::TableViewData::IndexedTable { table, .. } => {
                datatable_to_wire(table.as_ref())
            }
        },
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
            WireValue::Array(buf.iter().map(|v| WireValue::Bool(*v)).collect())
        }
        TypedArrayData::String(buf) => WireValue::Array(
            buf.iter()
                .map(|s| WireValue::String((**s).clone()))
                .collect(),
        ),
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
            value: d.num_milliseconds() as f64,
            unit: WireDurationUnit::Milliseconds,
        },
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
            "wire_to_slot: no projection for wire {:?} into kind {:?}",
            wire.discriminant_name(),
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
    let type_info = infer_type_info_for_name(type_name, ctx);
    ValueEnvelope { value, type_info }
}

fn infer_type_info_for_name(name: &str, _ctx: &Context) -> Option<TypeInfo> {
    if name.is_empty() {
        return None;
    }
    let registry = TypeRegistry::default();
    registry.lookup(name).cloned()
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
        (HeapKind::TableView, HeapValue::TableView(
            shape_value::heap_value::TableViewData::TypedTable { table, .. }
            | shape_value::heap_value::TableViewData::IndexedTable { table, .. },
        )) => Some(crate::content_dispatch::datatable_to_content_node(
            table.as_ref(),
            None,
        )),
        _ => None,
    };
    let Some(node) = node else {
        return (None, None, None);
    };

    use crate::content_renderer::ContentRenderer;
    use crate::renderers::{html::HtmlRenderer, json::JsonRenderer, terminal::TerminalRenderer};

    let json_str = JsonRenderer.render(&node);
    let content_json = serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
    let content_html = HtmlRenderer::new().render(&node);
    let content_terminal = TerminalRenderer::new().render(&node);
    (Some(content_json), Some(content_html), Some(content_terminal))
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
            schema_id,
            row_count: dt.row_count(),
            column_count: dt.column_count(),
        }),
        Err(e) => WireValue::String(format!("<datatable_serialize_error: {}>", e)),
    }
}

pub fn datatable_to_ipc_bytes(dt: &DataTable) -> std::result::Result<Vec<u8>, String> {
    let arrow_batch = dt
        .to_arrow_batch()
        .map_err(|e| format!("DataTable -> Arrow batch failed: {}", e))?;
    let schema = arrow_batch.schema();
    let mut buf = Vec::new();
    {
        let mut writer = FileWriter::try_new(&mut buf, &schema)
            .map_err(|e| format!("Arrow IPC writer init failed: {}", e))?;
        writer
            .write(&arrow_batch)
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
        return Ok(DataTable::empty());
    }
    let mut dt = DataTable::from_arrow_batches(&batches)
        .map_err(|e| format!("DataTable::from_arrow_batches failed: {}", e))?;
    if let Some(cols) = column_overrides {
        dt.replace_column_ptrs(cols);
    }
    if let Some(sid) = schema_id_override {
        dt.set_schema_id(Some(sid));
    }
    Ok(dt)
}
