//! Conversion between runtime values and wire format.
//!
//! This module provides conversions between the runtime's value types
//! and the serializable `WireValue` type used for REPL communication
//! and external tool integration.
//!
//! ## API (ValueWord-native)
//!
//! - [`nb_to_wire`] / [`wire_to_nb`] — convert directly without ValueWord intermediate
//! - [`nb_to_envelope`] — wrap a ValueWord in a ValueEnvelope with metadata
//! - [`nb_extract_typed_value`] / [`nb_typed_value_to_envelope`] — typed extraction

use crate::Context;
use arrow_ipc::{reader::FileReader, writer::FileWriter};
use shape_value::heap_value::HeapValue;
use shape_value::{DataTable, ValueSlot, ValueWord, ValueWordExt};
use shape_wire::{
    DurationUnit as WireDurationUnit, ValueEnvelope, WireTable, WireValue,
    metadata::{TypeInfo, TypeRegistry},
};
use std::collections::BTreeMap;
use std::sync::Arc;

#[inline]
fn slot_to_nb(
    slots: &[ValueSlot],
    heap_mask: u64,
    idx: usize,
    _field_type: &crate::type_schema::FieldType,
) -> ValueWord {
    if idx >= slots.len() {
        return ValueWord::none();
    }
    if heap_mask & (1u64 << idx) != 0 {
        return slots[idx].as_heap_nb();
    }

    // Non-heap slot: raw bits are a ValueWord representation (inline f64, i48,
    // bool, none, unit, function, module_fn). Reconstruct from raw bits.
    // Safety: bits were stored by nb_to_slot from a valid inline ValueWord.
    unsafe { ValueWord::clone_from_bits(slots[idx].raw()) }
}

#[inline]
fn slot_to_nb_raw(slots: &[ValueSlot], heap_mask: u64, idx: usize) -> ValueWord {
    if idx >= slots.len() {
        return ValueWord::none();
    }
    if heap_mask & (1u64 << idx) != 0 {
        slots[idx].as_heap_nb()
    } else {
        // Non-heap slot: reconstruct ValueWord from raw bits.
        // Safety: bits were stored from a valid inline ValueWord.
        unsafe { ValueWord::clone_from_bits(slots[idx].raw()) }
    }
}

fn nb_to_wire_with_builtin_fallback(nb: &ValueWord, ctx: &Context) -> WireValue {
    if let Some((_sid, slots, heap_mask)) = nb.as_typed_object()
        && let Some(value) = fallback_builtin_typedobject_to_wire(slots, heap_mask, ctx)
    {
        return value;
    }
    nb_to_wire(nb, ctx)
}

fn fallback_builtin_typedobject_to_wire(
    slots: &[ValueSlot],
    heap_mask: u64,
    ctx: &Context,
) -> Option<WireValue> {
    use crate::type_schema::builtin_schemas::*;

    // AnyError: [category, payload, cause, trace_info, message, code]
    if slots.len() >= 6 {
        let category = slot_to_nb_raw(slots, heap_mask, ANYERROR_CATEGORY);
        if category.as_str() == Some("AnyError") {
            let mut obj = BTreeMap::new();
            obj.insert(
                "category".to_string(),
                WireValue::String("AnyError".to_string()),
            );
            obj.insert(
                "payload".to_string(),
                nb_to_wire_with_builtin_fallback(
                    &slot_to_nb_raw(slots, heap_mask, ANYERROR_PAYLOAD),
                    ctx,
                ),
            );

            let cause_nb = slot_to_nb_raw(slots, heap_mask, ANYERROR_CAUSE);
            obj.insert(
                "cause".to_string(),
                if cause_nb.is_none() {
                    WireValue::Null
                } else {
                    nb_to_wire_with_builtin_fallback(&cause_nb, ctx)
                },
            );
            obj.insert(
                "trace_info".to_string(),
                nb_to_wire_with_builtin_fallback(
                    &slot_to_nb_raw(slots, heap_mask, ANYERROR_TRACE_INFO),
                    ctx,
                ),
            );
            obj.insert(
                "message".to_string(),
                nb_to_wire_with_builtin_fallback(
                    &slot_to_nb_raw(slots, heap_mask, ANYERROR_MESSAGE),
                    ctx,
                ),
            );
            obj.insert(
                "code".to_string(),
                nb_to_wire_with_builtin_fallback(
                    &slot_to_nb_raw(slots, heap_mask, ANYERROR_CODE),
                    ctx,
                ),
            );
            return Some(WireValue::Object(obj));
        }
    }

    // TraceInfo (full/single): [kind, frames|frame]
    if slots.len() >= 2 {
        let kind_nb = slot_to_nb_raw(slots, heap_mask, TRACEINFO_FULL_KIND);
        if let Some(kind) = kind_nb.as_str()
            && (kind == "full" || kind == "single")
        {
            let mut obj = BTreeMap::new();
            obj.insert("kind".to_string(), WireValue::String(kind.to_string()));
            if kind == "single" {
                obj.insert(
                    "frame".to_string(),
                    nb_to_wire_with_builtin_fallback(
                        &slot_to_nb_raw(slots, heap_mask, TRACEINFO_SINGLE_FRAME),
                        ctx,
                    ),
                );
            } else {
                obj.insert(
                    "frames".to_string(),
                    nb_to_wire_with_builtin_fallback(
                        &slot_to_nb_raw(slots, heap_mask, TRACEINFO_FULL_FRAMES),
                        ctx,
                    ),
                );
            }
            return Some(WireValue::Object(obj));
        }
    }

    // TraceFrame: [ip, line, file, function]
    if slots.len() >= 4 {
        let mut obj = BTreeMap::new();
        obj.insert(
            "ip".to_string(),
            nb_to_wire_with_builtin_fallback(&slot_to_nb_raw(slots, heap_mask, TRACEFRAME_IP), ctx),
        );
        obj.insert(
            "line".to_string(),
            nb_to_wire_with_builtin_fallback(
                &slot_to_nb_raw(slots, heap_mask, TRACEFRAME_LINE),
                ctx,
            ),
        );
        obj.insert(
            "file".to_string(),
            nb_to_wire_with_builtin_fallback(
                &slot_to_nb_raw(slots, heap_mask, TRACEFRAME_FILE),
                ctx,
            ),
        );
        obj.insert(
            "function".to_string(),
            nb_to_wire_with_builtin_fallback(
                &slot_to_nb_raw(slots, heap_mask, TRACEFRAME_FUNCTION),
                ctx,
            ),
        );
        return Some(WireValue::Object(obj));
    }

    None
}

/// Convert a ValueWord to WireValue (internal helper for structural ValueWord fields).
///
/// Used by `nb_heap_to_wire` for types that still embed ValueWord
/// (SimulationCallData.params, PrintSpan.raw_value).
fn value_to_wire(value: &ValueWord, ctx: &Context) -> WireValue {
    nb_to_wire(&value.clone(), ctx)
}

/// Convert a ValueWord value to WireValue without materializing ValueWord.
///
/// This is the ValueWord-native equivalent of `value_to_wire`. For inline types
/// (f64, i48, bool, None, Unit) it avoids heap allocation entirely. For heap types
/// it dispatches on HeapValue directly.
pub fn nb_to_wire(nb: &ValueWord, ctx: &Context) -> WireValue {
    use shape_value::tags::{is_tagged, get_tag, TAG_INT, TAG_BOOL, TAG_NONE, TAG_UNIT, TAG_FUNCTION, TAG_MODULE_FN, TAG_HEAP};

    let bits = nb.raw_bits();
    if !is_tagged(bits) {
        let n = nb.as_f64().unwrap_or(0.0);
        return WireValue::Number(n);
    }
    match get_tag(bits) {
        TAG_NONE | TAG_UNIT => WireValue::Null,

        TAG_BOOL => WireValue::Bool(nb.as_bool().unwrap_or(false)),

        TAG_INT => WireValue::Integer(nb.as_i64().unwrap_or(0)),

        TAG_FUNCTION => {
            let id = nb.as_function_id().unwrap_or(0);
            WireValue::String(format!("<function#{}>", id))
        }

        TAG_MODULE_FN => {
            let idx = nb.as_module_function().unwrap_or(0);
            WireValue::String(format!("<native:{}>", idx))
        }

        TAG_HEAP => nb_heap_to_wire(nb, ctx),

        _ => WireValue::Null, // References and other tags should not appear in wire output
    }
}

/// Convert a heap-tagged ValueWord to WireValue by dispatching on HeapValue.
fn nb_heap_to_wire(nb: &ValueWord, ctx: &Context) -> WireValue {
    // Handle unified-format Ok/Err/Some (bit 47 set)
    if shape_value::tags::is_unified_heap(nb.raw_bits()) {
        let kind = unsafe { shape_value::tags::unified_heap_kind(nb.raw_bits()) };
        if kind == shape_value::tags::HEAP_KIND_OK as u16 {
            let w = unsafe { shape_value::unified_wrapper::UnifiedWrapper::from_heap_bits(nb.raw_bits()) };
            let iv = unsafe { &*(&w.inner as *const u64 as *const ValueWord) };
            return WireValue::Result { ok: true, value: Box::new(nb_to_wire(iv, ctx)) };
        }
        if kind == shape_value::tags::HEAP_KIND_ERR as u16 {
            let w = unsafe { shape_value::unified_wrapper::UnifiedWrapper::from_heap_bits(nb.raw_bits()) };
            let iv = unsafe { &*(&w.inner as *const u64 as *const ValueWord) };
            return WireValue::Result { ok: false, value: Box::new(nb_to_wire(iv, ctx)) };
        }
        if kind == shape_value::tags::HEAP_KIND_SOME as u16 {
            let w = unsafe { shape_value::unified_wrapper::UnifiedWrapper::from_heap_bits(nb.raw_bits()) };
            let iv = unsafe { &*(&w.inner as *const u64 as *const ValueWord) };
            return nb_to_wire(iv, ctx);
        }
        if kind == shape_value::tags::HEAP_KIND_STRING as u16 {
            let us = unsafe { shape_value::unified_string::UnifiedString::from_heap_bits(nb.raw_bits()) };
            return WireValue::String(us.as_str().to_string());
        }
        if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
            let arr = unsafe {
                shape_value::unified_array::UnifiedArray::from_heap_bits(nb.raw_bits())
            };
            return WireValue::Array(
                (0..arr.len())
                    .map(|i| {
                        let elem = unsafe { ValueWord::clone_from_bits(*arr.get(i).unwrap()) };
                        nb_to_wire(&elem, ctx)
                    })
                    .collect(),
            );
        }
        return WireValue::Null;
    }
    // cold-path: as_heap_ref retained — multi-variant serialization dispatch
    let hv = match nb.as_heap_ref() { // cold-path
        Some(hv) => hv,
        None => return WireValue::Null,
    };
    match hv {
        HeapValue::String(s) => WireValue::String((**s).clone()),

        HeapValue::Array(arr) => {
            WireValue::Array(arr.iter().map(|elem| nb_to_wire(elem, ctx)).collect())
        }

        HeapValue::Decimal(d) => WireValue::Number(d.to_string().parse().unwrap_or(0.0)),

        HeapValue::BigInt(i) => WireValue::Integer(*i),
        HeapValue::ProjectedRef(_) => WireValue::Null,

        HeapValue::Temporal(shape_value::heap_value::TemporalData::DateTime(dt)) => WireValue::Timestamp(dt.timestamp_millis()),

        HeapValue::Temporal(shape_value::heap_value::TemporalData::TimeSpan(duration)) => {
            let millis = duration.num_milliseconds();
            WireValue::Duration {
                value: millis as f64,
                unit: WireDurationUnit::Milliseconds,
            }
        }

        HeapValue::Temporal(shape_value::heap_value::TemporalData::Duration(duration)) => {
            let value = duration.value;
            let wire_unit = match duration.unit {
                shape_ast::ast::DurationUnit::Seconds => WireDurationUnit::Seconds,
                shape_ast::ast::DurationUnit::Minutes => WireDurationUnit::Minutes,
                shape_ast::ast::DurationUnit::Hours => WireDurationUnit::Hours,
                shape_ast::ast::DurationUnit::Days => WireDurationUnit::Days,
                shape_ast::ast::DurationUnit::Weeks => WireDurationUnit::Weeks,
                shape_ast::ast::DurationUnit::Months => {
                    return WireValue::Duration {
                        value: value * 30.0,
                        unit: WireDurationUnit::Days,
                    };
                }
                shape_ast::ast::DurationUnit::Years => {
                    return WireValue::Duration {
                        value: value * 365.0,
                        unit: WireDurationUnit::Days,
                    };
                }
                shape_ast::ast::DurationUnit::Samples => {
                    return WireValue::Duration {
                        value,
                        unit: WireDurationUnit::Days,
                    };
                }
            };
            WireValue::Duration {
                value,
                unit: wire_unit,
            }
        }

        HeapValue::Enum(enum_value) => enum_to_wire(enum_value, ctx),

        HeapValue::Some(inner) => nb_to_wire(inner, ctx),

        HeapValue::Ok(inner) => WireValue::Result {
            ok: true,
            value: Box::new(nb_to_wire(inner, ctx)),
        },

        HeapValue::Err(inner) => WireValue::Result {
            ok: false,
            value: Box::new(nb_to_wire(inner, ctx)),
        },

        HeapValue::Range {
            start,
            end,
            inclusive,
        } => WireValue::Range {
            start: start.as_ref().map(|v| Box::new(nb_to_wire(v, ctx))),
            end: end.as_ref().map(|v| Box::new(nb_to_wire(v, ctx))),
            inclusive: *inclusive,
        },

        HeapValue::FunctionRef { name, .. } => WireValue::FunctionRef { name: name.clone() },

        HeapValue::Closure { .. } => WireValue::FunctionRef {
            name: "<closure>".to_string(),
        },

        HeapValue::Temporal(shape_value::heap_value::TemporalData::Timeframe(tf)) => WireValue::String(format!("{}", tf)),

        HeapValue::Rare(shape_value::heap_value::RareHeapData::DataReference(data)) => {
            let mut obj = BTreeMap::new();
            obj.insert(
                "datetime".to_string(),
                WireValue::Timestamp(data.datetime.timestamp_millis()),
            );
            obj.insert("id".to_string(), WireValue::String(data.id.clone()));
            obj.insert(
                "timeframe".to_string(),
                WireValue::String(format!("{}", data.timeframe)),
            );
            WireValue::Object(obj)
        }

        HeapValue::Rare(shape_value::heap_value::RareHeapData::SimulationCall(data)) => {
            let mut obj = BTreeMap::new();
            obj.insert(
                "__type".to_string(),
                WireValue::String("SimulationCall".to_string()),
            );
            obj.insert("name".to_string(), WireValue::String(data.name.clone()));
            // SimulationCallData.params stores ValueWord (structural boundary in shape-value)
            let params_wire: BTreeMap<String, WireValue> = data
                .params
                .iter()
                .map(|(k, v)| (k.clone(), value_to_wire(v, ctx)))
                .collect();
            obj.insert("params".to_string(), WireValue::Object(params_wire));
            WireValue::Object(obj)
        }

        HeapValue::Rare(shape_value::heap_value::RareHeapData::PrintResult(result)) => {
            use shape_wire::print_result::{WirePrintResult, WirePrintSpan};

            let wire_spans: Vec<WirePrintSpan> = result
                .spans
                .iter()
                .map(|span| match span {
                    shape_value::PrintSpan::Literal {
                        text,
                        start,
                        end,
                        span_id,
                    } => WirePrintSpan::Literal {
                        text: text.clone(),
                        start: *start,
                        end: *end,
                        span_id: span_id.clone(),
                    },
                    shape_value::PrintSpan::Value {
                        text,
                        start,
                        end,
                        span_id,
                        variable_name,
                        raw_value,
                        type_name,
                        current_format,
                        format_params,
                    } => {
                        // raw_value is Box<ValueWord> (structural boundary in shape-value)
                        let raw_wire = value_to_wire(raw_value, ctx);
                        let (type_info, type_registry) =
                            infer_metadata_with_ctx(raw_value, type_name, ctx);

                        let params_wire: std::collections::HashMap<String, WireValue> =
                            format_params
                                .iter()
                                .map(|(k, v)| (k.clone(), value_to_wire(v, ctx)))
                                .collect();

                        WirePrintSpan::Value {
                            text: text.clone(),
                            start: *start,
                            end: *end,
                            span_id: span_id.clone(),
                            variable_name: variable_name.clone(),
                            raw_value: Box::new(raw_wire),
                            type_info: Box::new(type_info),
                            current_format: current_format.clone(),
                            type_registry,
                            format_params: params_wire,
                        }
                    }
                })
                .collect();

            WireValue::PrintResult(WirePrintResult {
                rendered: result.rendered.clone(),
                spans: wire_spans,
            })
        }

        HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask,
        } => {
            let schema = ctx
                .type_schema_registry()
                .get_by_id(*schema_id as u32)
                .cloned()
                .or_else(|| crate::type_schema::lookup_schema_by_id_public(*schema_id as u32));

            if let Some(schema) = schema {
                let mut map = BTreeMap::new();
                for field_def in &schema.fields {
                    let idx = field_def.index as usize;
                    if idx < slots.len() {
                        let field_nb = slot_to_nb(slots, *heap_mask, idx, &field_def.field_type);
                        map.insert(
                            field_def.name.clone(),
                            nb_to_wire_with_builtin_fallback(&field_nb, ctx),
                        );
                    }
                }
                WireValue::Object(map)
            } else if let Some(fallback) =
                fallback_builtin_typedobject_to_wire(slots, *heap_mask, ctx)
            {
                fallback
            } else {
                WireValue::String(format!("<typed_object:schema#{}>", schema_id))
            }
        }

        HeapValue::Rare(shape_value::heap_value::RareHeapData::TypeAnnotatedValue { value, .. }) => nb_to_wire(value, ctx),

        HeapValue::Future(id) => WireValue::String(format!("<future:{}>", id)),

        HeapValue::DataTable(dt) => datatable_to_wire(dt.as_ref()),

        HeapValue::TableView(shape_value::heap_value::TableViewData::TypedTable { table, schema_id }) => {
            datatable_to_wire_with_schema(table.as_ref(), Some(*schema_id as u32))
        }

        HeapValue::TableView(shape_value::heap_value::TableViewData::RowView { row_idx, .. }) => WireValue::String(format!("<Row:{}>", row_idx)),
        HeapValue::TableView(shape_value::heap_value::TableViewData::ColumnRef { col_id, .. }) => WireValue::String(format!("<ColumnRef:{}>", col_id)),
        HeapValue::TableView(shape_value::heap_value::TableViewData::IndexedTable { table, .. }) => datatable_to_wire(table.as_ref()),
        HeapValue::HostClosure(_) => WireValue::String("<HostClosure>".to_string()),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::ExprProxy(col)) => WireValue::String(format!("<ExprProxy:{}>", col)),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::FilterExpr(_)) => WireValue::String("<FilterExpr>".to_string()),
        HeapValue::TaskGroup { .. } => WireValue::String("<TaskGroup>".to_string()),
        HeapValue::TraitObject { value, .. } => nb_to_wire(value, ctx),

        // Rare AST types — format as debug strings
        HeapValue::Temporal(shape_value::heap_value::TemporalData::TimeReference(tr)) => WireValue::String(format!("{:?}", tr)),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::DateTimeExpr(expr)) => WireValue::String(format!("{:?}", expr)),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::DataDateTimeRef(dref)) => WireValue::String(format!("{:?}", dref)),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::TypeAnnotation(ann)) => WireValue::String(format!("{:?}", ann)),

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
        HeapValue::HashMap(d) => {
            let pairs: Vec<(String, WireValue)> = d
                .keys
                .iter()
                .zip(d.values.iter())
                .map(|(k, v)| (format!("{}", shape_value::ValueWordDisplay(*k)), nb_to_wire(v, ctx)))
                .collect();
            WireValue::Object(pairs.into_iter().collect())
        }
        HeapValue::Set(d) => WireValue::Array(d.items.iter().map(|v| nb_to_wire(v, ctx)).collect()),
        HeapValue::Deque(d) => {
            WireValue::Array(d.items.iter().map(|v| nb_to_wire(v, ctx)).collect())
        }
        HeapValue::PriorityQueue(d) => {
            WireValue::Array(d.items.iter().map(|v| nb_to_wire(v, ctx)).collect())
        }
        HeapValue::Content(node) => WireValue::Content((**node).clone()),
        HeapValue::Instant(t) => WireValue::String(format!("<instant:{:?}>", t.elapsed())),
        HeapValue::IoHandle(data) => {
            let status = if data.is_open() { "open" } else { "closed" };
            WireValue::String(format!("<io_handle:{}:{}>", data.path, status))
        }
        HeapValue::SharedCell(arc) => nb_to_wire(&arc.read().unwrap(), ctx),
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I64(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Integer(v)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::F64(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Number(v)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        }) => {
            let start = *offset as usize;
            let end = start + *len as usize;
            let slice = &parent.data[start..end];
            WireValue::Array(slice.iter().map(|&v| WireValue::Number(v)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::Bool(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Bool(v != 0)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I8(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Integer(v as i64)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I16(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Integer(v as i64)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I32(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Integer(v as i64)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U8(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Integer(v as i64)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U16(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Integer(v as i64)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U32(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Integer(v as i64)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U64(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Integer(v as i64)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::F32(a)) => {
            WireValue::Array(a.iter().map(|&v| WireValue::Number(v as f64)).collect())
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::Matrix(m)) => WireValue::String(format!("<Mat<number>:{}x{}>", m.rows, m.cols)),
        HeapValue::Iterator(_) => WireValue::String("<iterator>".to_string()),
        HeapValue::Generator(_) => WireValue::String("<generator>".to_string()),
        HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Mutex(_)) => WireValue::String("<mutex>".to_string()),
        HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Atomic(a)) => {
            WireValue::Integer(a.inner.load(std::sync::atomic::Ordering::Relaxed))
        }
        HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Lazy(_)) => WireValue::String("<lazy>".to_string()),
        HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Channel(c)) => {
            if c.is_sender() {
                WireValue::String("<channel:sender>".to_string())
            } else {
                WireValue::String("<channel:receiver>".to_string())
            }
        }
        HeapValue::Char(c) => WireValue::String(c.to_string()),
    }
}

/// Extracted content with multiple render targets.
pub struct ExtractedContent {
    /// The raw ContentNode (for re-rendering on other targets or wire serialization)
    pub content_node: shape_value::content::ContentNode,
    /// JSON renderer output
    pub content_json: serde_json::Value,
    /// HTML renderer output
    pub content_html: String,
    /// Terminal renderer output (ANSI)
    pub content_terminal: String,
}

/// If the ValueWord value is a Content node, render it as JSON, HTML, and terminal strings.
///
/// Returns `(content_json, content_html, content_terminal)` — all `None` if the value is not Content.
pub fn nb_extract_content(
    nb: &ValueWord,
) -> (Option<serde_json::Value>, Option<String>, Option<String>) {
    let extracted = nb_extract_content_full(nb);
    match extracted {
        Some(e) => (
            Some(e.content_json),
            Some(e.content_html),
            Some(e.content_terminal),
        ),
        None => (None, None, None),
    }
}

/// Extract content with full detail including the raw ContentNode.
pub fn nb_extract_content_full(nb: &ValueWord) -> Option<ExtractedContent> {
    use crate::content_renderer::ContentRenderer;

    // Check if value is already a Content node
    let node: Option<shape_value::content::ContentNode> =
        if let Some(content) = nb.as_content() {
            Some(content.clone())
        } else if let Some(dt) = nb.as_datatable() {
            // Auto-wrap DataTable as ContentNode::Table
            Some(crate::content_dispatch::datatable_to_content_node(dt, None))
        } else if let Some((_sid, dt)) = nb.as_typed_table() {
            Some(crate::content_dispatch::datatable_to_content_node(
                dt, None,
            ))
        } else {
            None
        };

    let node = node?;

    let json_renderer = crate::renderers::json::JsonRenderer;
    let html_renderer = crate::renderers::html::HtmlRenderer::new();
    let terminal_renderer = crate::renderers::terminal::TerminalRenderer::new();

    let json_str = json_renderer.render(&node);
    let content_json = serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
    let content_html = html_renderer.render(&node);
    let content_terminal = terminal_renderer.render(&node);

    Some(ExtractedContent {
        content_node: node,
        content_json,
        content_html,
        content_terminal,
    })
}

/// Convert a WireValue to a ValueWord value without ValueWord intermediate.
///
/// This is the ValueWord-native equivalent of `wire_to_value`. For simple types
/// (null, bool, number, integer) it constructs ValueWord inline values directly.
pub fn wire_to_nb(wire: &WireValue) -> ValueWord {
    match wire {
        WireValue::Null => ValueWord::none(),
        WireValue::Bool(b) => ValueWord::from_bool(*b),
        WireValue::Number(n) => ValueWord::from_f64(*n),
        WireValue::Integer(i) => ValueWord::from_i64(*i),
        WireValue::I8(n) => ValueWord::from_native_i8(*n),
        WireValue::U8(n) => ValueWord::from_native_u8(*n),
        WireValue::I16(n) => ValueWord::from_native_i16(*n),
        WireValue::U16(n) => ValueWord::from_native_u16(*n),
        WireValue::I32(n) => ValueWord::from_native_i32(*n),
        WireValue::U32(n) => ValueWord::from_native_u32(*n),
        WireValue::I64(n) => {
            ValueWord::from_native_scalar(shape_value::heap_value::NativeScalar::I64(*n))
        }
        WireValue::U64(n) => ValueWord::from_native_u64(*n),
        WireValue::Isize(n) => match isize::try_from(*n) {
            Ok(v) => ValueWord::from_native_isize(v),
            Err(_) => ValueWord::none(),
        },
        WireValue::Usize(n) => match usize::try_from(*n) {
            Ok(v) => ValueWord::from_native_usize(v),
            Err(_) => ValueWord::none(),
        },
        WireValue::Ptr(n) => match usize::try_from(*n) {
            Ok(v) => ValueWord::from_native_scalar(shape_value::heap_value::NativeScalar::Ptr(v)),
            Err(_) => ValueWord::none(),
        },
        WireValue::F32(n) => ValueWord::from_native_f32(*n),
        WireValue::String(s) => ValueWord::from_string(Arc::new(s.clone())),

        WireValue::Timestamp(ts) => match chrono::DateTime::from_timestamp_millis(*ts) {
            Some(dt) => ValueWord::from_time_utc(dt),
            None => ValueWord::none(),
        },

        WireValue::Duration { value, unit } => {
            let (ast_value, ast_unit) = match unit {
                WireDurationUnit::Nanoseconds => (
                    *value / 1_000_000_000.0,
                    shape_ast::ast::DurationUnit::Seconds,
                ),
                WireDurationUnit::Microseconds => {
                    (*value / 1_000_000.0, shape_ast::ast::DurationUnit::Seconds)
                }
                WireDurationUnit::Milliseconds => {
                    (*value / 1_000.0, shape_ast::ast::DurationUnit::Seconds)
                }
                WireDurationUnit::Seconds => (*value, shape_ast::ast::DurationUnit::Seconds),
                WireDurationUnit::Minutes => (*value, shape_ast::ast::DurationUnit::Minutes),
                WireDurationUnit::Hours => (*value, shape_ast::ast::DurationUnit::Hours),
                WireDurationUnit::Days => (*value, shape_ast::ast::DurationUnit::Days),
                WireDurationUnit::Weeks => (*value, shape_ast::ast::DurationUnit::Weeks),
            };
            ValueWord::from_duration(shape_ast::ast::Duration {
                value: ast_value,
                unit: ast_unit,
            })
        }

        WireValue::Array(arr) => {
            let elements: Vec<ValueWord> = arr.iter().map(wire_to_nb).collect();
            ValueWord::from_array(shape_value::vmarray_from_vec(elements))
        }

        WireValue::Object(obj) => {
            // Check for enum encoding
            let enum_name = obj.get("__enum").and_then(|v| match v {
                WireValue::String(s) => Some(s.clone()),
                _ => None,
            });
            let variant = obj.get("__variant").and_then(|v| match v {
                WireValue::String(s) => Some(s.clone()),
                _ => None,
            });

            if let (Some(enum_name), Some(variant)) = (enum_name, variant) {
                let payload = match obj.get("__fields") {
                    None => shape_value::EnumPayload::Unit,
                    Some(WireValue::Array(values)) => {
                        shape_value::EnumPayload::Tuple(values.iter().map(wire_to_nb).collect())
                    }
                    Some(WireValue::Object(fields)) => {
                        let map: std::collections::HashMap<String, ValueWord> = fields
                            .iter()
                            .map(|(k, v)| (k.clone(), wire_to_nb(v)))
                            .collect();
                        shape_value::EnumPayload::Struct(map)
                    }
                    _ => shape_value::EnumPayload::Unit,
                };
                ValueWord::from_enum(shape_value::EnumValue {
                    enum_name,
                    variant,
                    payload,
                })
            } else {
                // Regular object -> TypedObject
                let pairs: Vec<(String, ValueWord)> = obj
                    .iter()
                    .map(|(k, v)| (k.clone(), wire_to_nb(v)))
                    .collect();
                let pair_refs: Vec<(&str, ValueWord)> =
                    pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                crate::type_schema::typed_object_from_nb_pairs(&pair_refs)
            }
        }

        WireValue::Table(table) => {
            match datatable_from_ipc_bytes(
                &table.ipc_bytes,
                table.type_name.as_deref(),
                table.schema_id,
            ) {
                Ok(dt) => ValueWord::from_datatable(Arc::new(dt)),
                Err(_) => ValueWord::none(),
            }
        }

        WireValue::Result { ok, value } => {
            let inner = wire_to_nb(value);
            if *ok {
                ValueWord::from_ok(inner)
            } else {
                ValueWord::from_err(inner)
            }
        }

        WireValue::Range {
            start,
            end,
            inclusive,
        } => ValueWord::from_range(
            start.as_ref().map(|v| wire_to_nb(v)),
            end.as_ref().map(|v| wire_to_nb(v)),
            *inclusive,
        ),

        WireValue::FunctionRef { name } => ValueWord::from_function_ref(name.clone(), None),

        WireValue::PrintResult(result) => {
            // Convert back as rendered string (same as wire_to_value)
            ValueWord::from_string(Arc::new(result.rendered.clone()))
        }

        WireValue::Content(node) => {
            ValueWord::from_heap_value(HeapValue::Content(Box::new(node.clone())))
        }
    }
}

/// Convert a ValueWord value to a ValueEnvelope with full metadata.
pub fn nb_to_envelope(nb: &ValueWord, type_name: &str, ctx: &Context) -> ValueEnvelope {
    let wire_value = nb_to_wire(nb, ctx);
    let type_info = TypeInfo::primitive(type_name);
    let registry = TypeRegistry::new("Default");
    ValueEnvelope::new(wire_value, type_info, registry)
}

/// Extract type info and wire value from a ValueWord value.
///
/// ValueWord-native equivalent of [`extract_typed_value`].
pub fn nb_extract_typed_value(nb: &ValueWord, ctx: &Context) -> (WireValue, Option<TypeInfo>) {
    let wire_value = nb_to_wire(nb, ctx);
    let type_name = nb.type_name();
    let type_info = TypeInfo::primitive(type_name);
    (wire_value, Some(type_info))
}

/// Convert a ValueWord value to ValueEnvelope with inferred type information.
///
/// ValueWord-native equivalent of [`typed_value_to_envelope`].
pub fn nb_typed_value_to_envelope(nb: &ValueWord, ctx: &Context) -> ValueEnvelope {
    let type_name = nb.type_name();
    nb_to_envelope(nb, type_name, ctx)
}

/// Convert an EnumValue to WireValue (shared between ValueWord and ValueWord paths)
fn enum_to_wire(enum_value: &shape_value::EnumValue, ctx: &Context) -> WireValue {
    let mut obj = BTreeMap::new();
    obj.insert(
        "__enum".to_string(),
        WireValue::String(enum_value.enum_name.clone()),
    );
    obj.insert(
        "__variant".to_string(),
        WireValue::String(enum_value.variant.clone()),
    );
    match &enum_value.payload {
        shape_value::EnumPayload::Unit => {}
        shape_value::EnumPayload::Tuple(values) => {
            obj.insert(
                "__fields".to_string(),
                WireValue::Array(values.iter().map(|v| nb_to_wire(v, ctx)).collect()),
            );
        }
        shape_value::EnumPayload::Struct(fields) => {
            let field_map: BTreeMap<String, WireValue> = fields
                .iter()
                .map(|(k, v)| (k.clone(), nb_to_wire(v, ctx)))
                .collect();
            obj.insert("__fields".to_string(), WireValue::Object(field_map));
        }
    }
    WireValue::Object(obj)
}

fn datatable_to_wire(dt: &DataTable) -> WireValue {
    datatable_to_wire_with_schema(dt, dt.schema_id())
}

fn datatable_to_wire_with_schema(dt: &DataTable, schema_id: Option<u32>) -> WireValue {
    match datatable_to_ipc_bytes(dt) {
        Ok(ipc_bytes) => WireValue::Table(WireTable {
            ipc_bytes,
            type_name: dt.type_name().map(|s| s.to_string()),
            schema_id,
            row_count: dt.row_count(),
            column_count: dt.column_count(),
        }),
        Err(_) => WireValue::String(format!("{}", dt)),
    }
}

/// Serialize a [`DataTable`] to Arrow IPC bytes.
pub fn datatable_to_ipc_bytes(dt: &DataTable) -> std::result::Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let schema = dt.inner().schema();
    let mut writer = FileWriter::try_new(&mut buf, schema.as_ref())
        .map_err(|e| format!("failed to create Arrow IPC writer: {e}"))?;
    writer
        .write(dt.inner())
        .map_err(|e| format!("failed to write Arrow IPC batch: {e}"))?;
    writer
        .finish()
        .map_err(|e| format!("failed to finalize Arrow IPC writer: {e}"))?;
    Ok(buf)
}

/// Deserialize Arrow IPC bytes into a [`DataTable`].
pub fn datatable_from_ipc_bytes(
    ipc_bytes: &[u8],
    type_name: Option<&str>,
    schema_id: Option<u32>,
) -> std::result::Result<DataTable, String> {
    if ipc_bytes.is_empty() {
        return Err("empty Arrow IPC payload".to_string());
    }

    let cursor = std::io::Cursor::new(ipc_bytes);
    let mut reader = FileReader::try_new(cursor, None)
        .map_err(|e| format!("failed to create Arrow IPC reader: {e}"))?;
    let batch = reader
        .next()
        .transpose()
        .map_err(|e| format!("failed reading Arrow IPC batch: {e}"))?
        .ok_or_else(|| "Arrow IPC payload has no record batches".to_string())?;

    let mut dt = DataTable::new(batch);
    if let Some(name) = type_name {
        dt = DataTable::with_type_name(dt.into_inner(), name.to_string());
    }
    if let Some(id) = schema_id {
        dt = dt.with_schema_id(id);
    }
    Ok(dt)
}

/// Helper to infer full metadata including from registry
/// Note: Meta definitions have been removed; formatting now uses Display trait.
fn infer_metadata_with_ctx(
    _value: &ValueWord,
    type_name: &str,
    _ctx: &Context,
) -> (TypeInfo, TypeRegistry) {
    let type_info = TypeInfo::primitive(type_name);
    let registry = TypeRegistry::new("Default");
    (type_info, registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_methods::TypeMethodRegistry;
    use crate::type_schema::typed_object_to_hashmap_nb;
    use shape_value::ValueSlot;
    use shape_value::heap_value::HeapValue;
    use std::sync::Arc;

    fn get_dummy_context() -> Context {
        Context::new_empty_with_registry(Arc::new(TypeMethodRegistry::new()))
    }

    #[test]
    fn test_basic_value_conversion() {
        let ctx = get_dummy_context();
        // Number
        let wire = value_to_wire(&ValueWord::from_f64(42.5), &ctx);
        assert_eq!(wire, WireValue::Number(42.5));

        // Whole number remains number
        let wire = value_to_wire(&ValueWord::from_f64(42.0), &ctx);
        assert_eq!(wire, WireValue::Number(42.0));

        // String
        let wire = value_to_wire(&ValueWord::from_string(Arc::new("hello".to_string())), &ctx);
        assert_eq!(wire, WireValue::String("hello".to_string()));

        // Bool
        let wire = value_to_wire(&ValueWord::from_bool(true), &ctx);
        assert_eq!(wire, WireValue::Bool(true));

        // None
        let wire = value_to_wire(&ValueWord::none(), &ctx);
        assert_eq!(wire, WireValue::Null);
    }

    #[test]
    fn test_array_conversion() {
        let ctx = get_dummy_context();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
        ]));
        let wire = value_to_wire(&arr, &ctx);

        if let WireValue::Array(items) = wire {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], WireValue::Number(1.0));
            assert_eq!(items[1], WireValue::Number(2.0));
            assert_eq!(items[2], WireValue::Number(3.0));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_object_conversion() {
        let ctx = get_dummy_context();
        let obj = crate::type_schema::typed_object_from_pairs(&[
            ("x", ValueWord::from_f64(10.0)),
            ("y", ValueWord::from_f64(20.0)),
        ]);

        let wire = value_to_wire(&obj, &ctx);

        if let WireValue::Object(map) = wire {
            assert_eq!(map.get("x"), Some(&WireValue::Number(10.0)));
            assert_eq!(map.get("y"), Some(&WireValue::Number(20.0)));
        } else {
            panic!("Expected Object");
        }
    }

    #[test]
    fn test_builtin_typed_object_converts_to_wire_object() {
        let ctx = get_dummy_context();
        let any_error_schema_id = ctx
            .type_schema_registry()
            .get("__AnyError")
            .expect("__AnyError schema should exist")
            .id as u64;

        let slots = vec![
            ValueSlot::from_heap(HeapValue::String(Arc::new("AnyError".to_string()))),
            ValueSlot::from_heap(HeapValue::String(Arc::new("boom".to_string()))),
            ValueSlot::none(), // cause: None (inline)
            ValueSlot::none(), // trace_info: None (inline)
            ValueSlot::from_heap(HeapValue::String(Arc::new("boom".to_string()))),
            ValueSlot::from_heap(HeapValue::String(Arc::new("E_BANG".to_string()))),
        ];

        let nb = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: any_error_schema_id,
            slots: slots.into_boxed_slice(),
            heap_mask: 0b11_0011, // bits 0,1,4,5 = heap; bits 2,3 = inline none
        });

        let wire = nb_to_wire(&nb, &ctx);
        match wire {
            WireValue::Object(map) => {
                assert_eq!(
                    map.get("category"),
                    Some(&WireValue::String("AnyError".into()))
                );
                assert_eq!(map.get("message"), Some(&WireValue::String("boom".into())));
                assert_eq!(map.get("code"), Some(&WireValue::String("E_BANG".into())));
            }
            other => panic!("Expected WireValue::Object, got {:?}", other),
        }
    }

    #[test]
    fn test_unknown_schema_anyerror_uses_builtin_fallback_decoder() {
        let ctx = get_dummy_context();

        let slots = vec![
            ValueSlot::from_heap(HeapValue::String(Arc::new("AnyError".to_string()))),
            ValueSlot::from_heap(HeapValue::String(Arc::new("boom".to_string()))),
            ValueSlot::none(), // cause: None (inline)
            ValueSlot::none(), // trace_info: None (inline)
            ValueSlot::from_heap(HeapValue::String(Arc::new("boom".to_string()))),
            ValueSlot::from_heap(HeapValue::String(Arc::new("E_BANG".to_string()))),
        ];

        let nb = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: 9_999_999,
            slots: slots.into_boxed_slice(),
            heap_mask: 0b11_0011, // bits 0,1,4,5 = heap; bits 2,3 = inline none
        });

        let wire = nb_to_wire(&nb, &ctx);
        match wire {
            WireValue::Object(map) => {
                assert_eq!(
                    map.get("category"),
                    Some(&WireValue::String("AnyError".into()))
                );
                assert_eq!(map.get("message"), Some(&WireValue::String("boom".into())));
                assert_eq!(map.get("code"), Some(&WireValue::String("E_BANG".into())));
            }
            other => panic!("Expected WireValue::Object, got {:?}", other),
        }
    }

    #[test]
    fn test_timestamp_conversion() {
        let ctx = get_dummy_context();
        use chrono::TimeZone;
        let dt = chrono::Utc
            .with_ymd_and_hms(2024, 1, 15, 10, 30, 0)
            .unwrap();
        let wire = value_to_wire(&ValueWord::from_time_utc(dt), &ctx);

        if let WireValue::Timestamp(ts) = wire {
            assert_eq!(ts, 1705314600000);
        } else {
            panic!("Expected Timestamp");
        }
    }

    #[test]
    fn test_result_conversion() {
        let ctx = get_dummy_context();
        let ok_val = ValueWord::from_ok(ValueWord::from_f64(42.0));
        let wire = value_to_wire(&ok_val, &ctx);

        if let WireValue::Result { ok, value } = wire {
            assert!(ok);
            assert_eq!(*value, WireValue::Number(42.0));
        } else {
            panic!("Expected Result");
        }
    }

    #[test]
    fn test_wire_to_nb_anyerror_trace_frame_key_order_is_stable() {
        use std::collections::BTreeMap;

        let mut frame = BTreeMap::new();
        frame.insert(
            "function".to_string(),
            WireValue::String("duckdb.connect".to_string()),
        );
        frame.insert(
            "file".to_string(),
            WireValue::String("extension:duckdb".to_string()),
        );
        frame.insert("line".to_string(), WireValue::Null);
        frame.insert("ip".to_string(), WireValue::Null);

        let nb = wire_to_nb(&WireValue::Object(frame));
        let decoded =
            typed_object_to_hashmap_nb(&nb).expect("Trace frame wire object should decode");
        assert_eq!(
            decoded.get("function").and_then(|v| v.as_str()),
            Some("duckdb.connect")
        );
        assert_eq!(
            decoded.get("file").and_then(|v| v.as_str()),
            Some("extension:duckdb")
        );
    }

    #[test]
    fn test_any_error_result_roundtrip() {
        use crate::type_schema::typed_object_from_pairs;
        let ctx = get_dummy_context();

        let empty_trace = typed_object_from_pairs(&[]);
        let cause = typed_object_from_pairs(&[
            (
                "category",
                ValueWord::from_string(Arc::new("AnyError".to_string())),
            ),
            (
                "payload",
                ValueWord::from_string(Arc::new("low level".to_string())),
            ),
            ("cause", ValueWord::none()),
            ("trace_info", empty_trace),
        ]);

        let empty_trace2 = typed_object_from_pairs(&[]);
        let outer = typed_object_from_pairs(&[
            (
                "category",
                ValueWord::from_string(Arc::new("AnyError".to_string())),
            ),
            (
                "payload",
                ValueWord::from_string(Arc::new("high level".to_string())),
            ),
            ("cause", cause),
            ("trace_info", empty_trace2),
            (
                "code",
                ValueWord::from_string(Arc::new("OPTION_NONE".to_string())),
            ),
        ]);

        let err = ValueWord::from_err(outer);
        let wire = value_to_wire(&err, &ctx);

        let WireValue::Result { ok, value } = &wire else {
            panic!("Expected wire Result");
        };
        assert!(!ok);
        match value.as_ref() {
            WireValue::Object(map) => {
                assert_eq!(
                    map.get("category"),
                    Some(&WireValue::String("AnyError".to_string()))
                );
                assert_eq!(
                    map.get("code"),
                    Some(&WireValue::String("OPTION_NONE".to_string()))
                );
            }
            other => panic!("Expected AnyError object payload, got {:?}", other),
        }

        let roundtrip = wire_to_nb(&wire);
        // cold-path: as_heap_ref retained — test assertion on multiple variants
        let hv = roundtrip.as_heap_ref().expect("Expected heap value"); // cold-path
        match hv {
            HeapValue::Err(inner) => {
                // cold-path: as_heap_ref retained — test assertion on multiple variants
                let inner_hv = inner.as_heap_ref().expect("Expected heap inner"); // cold-path
                assert!(
                    matches!(inner_hv, HeapValue::TypedObject { .. }),
                    "Expected TypedObject inside Err"
                );
            }
            other => panic!("Expected Err, got {:?}", other.kind()),
        }
    }

    #[test]
    fn test_envelope_creation() {
        let ctx = get_dummy_context();
        let nb = ValueWord::from_f64(3.14);
        let envelope = nb_to_envelope(&nb, "number", &ctx);

        match &envelope.value {
            WireValue::Number(n) => assert!((*n - 3.14).abs() < f64::EPSILON),
            other => panic!("Expected Number, got {:?}", other),
        }
    }

    #[test]
    fn test_roundtrip_basic() {
        let ctx = get_dummy_context();
        let nb = ValueWord::from_string(Arc::new("test".to_string()));
        let wire = nb_to_wire(&nb, &ctx);
        let back = wire_to_nb(&wire);

        if let Some(s) = back.as_str() {
            assert_eq!(s, "test");
        } else {
            panic!("Expected String");
        }
    }

    // ===== ValueWord-native conversion tests =====

    #[test]
    fn test_nb_to_wire_basic_types() {
        let ctx = get_dummy_context();

        // f64 -> Number (fractional)
        let wire = nb_to_wire(&ValueWord::from_f64(42.5), &ctx);
        assert_eq!(wire, WireValue::Number(42.5));

        // f64 whole -> Number
        let wire = nb_to_wire(&ValueWord::from_f64(42.0), &ctx);
        assert_eq!(wire, WireValue::Number(42.0));

        // i48 -> Integer
        let wire = nb_to_wire(&ValueWord::from_i64(99), &ctx);
        assert_eq!(wire, WireValue::Integer(99));

        // Negative i48 -> Integer
        let wire = nb_to_wire(&ValueWord::from_i64(-7), &ctx);
        assert_eq!(wire, WireValue::Integer(-7));

        // Bool
        let wire = nb_to_wire(&ValueWord::from_bool(true), &ctx);
        assert_eq!(wire, WireValue::Bool(true));

        let wire = nb_to_wire(&ValueWord::from_bool(false), &ctx);
        assert_eq!(wire, WireValue::Bool(false));

        // None
        let wire = nb_to_wire(&ValueWord::none(), &ctx);
        assert_eq!(wire, WireValue::Null);

        // Unit
        let wire = nb_to_wire(&ValueWord::unit(), &ctx);
        assert_eq!(wire, WireValue::Null);
    }

    #[test]
    fn test_nb_to_wire_string() {
        let ctx = get_dummy_context();
        let nb = ValueWord::from_string(Arc::new("hello".to_string()));
        let wire = nb_to_wire(&nb, &ctx);
        assert_eq!(wire, WireValue::String("hello".to_string()));
    }

    #[test]
    fn test_nb_to_wire_array() {
        let ctx = get_dummy_context();
        let nb = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_i64(2),
            ValueWord::from_bool(true),
        ]));
        let wire = nb_to_wire(&nb, &ctx);

        if let WireValue::Array(items) = wire {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], WireValue::Number(1.0));
            assert_eq!(items[1], WireValue::Integer(2));
            assert_eq!(items[2], WireValue::Bool(true));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_nb_to_wire_result() {
        let ctx = get_dummy_context();

        // Ok result
        let ok = ValueWord::from_ok(ValueWord::from_i64(42));
        let wire = nb_to_wire(&ok, &ctx);
        if let WireValue::Result { ok, value } = wire {
            assert!(ok);
            assert_eq!(*value, WireValue::Integer(42));
        } else {
            panic!("Expected Result");
        }

        // Err result
        let err = ValueWord::from_err(ValueWord::from_string(Arc::new("oops".to_string())));
        let wire = nb_to_wire(&err, &ctx);
        if let WireValue::Result { ok, value } = wire {
            assert!(!ok);
            assert_eq!(*value, WireValue::String("oops".to_string()));
        } else {
            panic!("Expected Result");
        }
    }

    #[test]
    fn test_nb_to_wire_some() {
        let ctx = get_dummy_context();
        let some = ValueWord::from_some(ValueWord::from_f64(3.14));
        let wire = nb_to_wire(&some, &ctx);
        // Some unwraps to inner value
        assert_eq!(wire, WireValue::Number(3.14));
    }

    #[test]
    fn test_nb_to_wire_matches_vmvalue() {
        // Verify that nb_to_wire produces the same output as value_to_wire for basic types
        let ctx = get_dummy_context();

        let test_values: Vec<ValueWord> = vec![
            ValueWord::from_f64(42.5),
            ValueWord::from_f64(42.0),
            ValueWord::from_i64(100),
            ValueWord::from_i64(-100),
            ValueWord::from_bool(true),
            ValueWord::from_bool(false),
            ValueWord::none(),
            ValueWord::unit(),
            ValueWord::from_string(Arc::new("test".to_string())),
            ValueWord::from_array(shape_value::vmarray_from_vec(vec![
                ValueWord::from_f64(1.0),
                ValueWord::from_i64(2),
            ])),
        ];

        for nb in &test_values {
            let vmv = nb.clone();
            let wire_from_vmv = value_to_wire(&vmv, &ctx);
            let wire_from_nb = nb_to_wire(nb, &ctx);
            assert_eq!(
                wire_from_vmv,
                wire_from_nb,
                "Mismatch for ValueWord type {:?}: ValueWord path = {:?}, ValueWord path = {:?}",
                nb.type_name(),
                wire_from_vmv,
                wire_from_nb
            );
        }
    }

    #[test]
    fn test_wire_to_nb_basic_types() {
        // Null -> None
        let nb = wire_to_nb(&WireValue::Null);
        assert!(nb.is_none());

        // Bool
        let nb = wire_to_nb(&WireValue::Bool(true));
        assert_eq!(nb.as_bool(), Some(true));

        // Number
        let nb = wire_to_nb(&WireValue::Number(3.14));
        assert_eq!(nb.as_f64(), Some(3.14));

        // Integer
        let nb = wire_to_nb(&WireValue::Integer(42));
        assert_eq!(nb.as_i64(), Some(42));

        // String
        let nb = wire_to_nb(&WireValue::String("hello".to_string()));
        assert_eq!(nb.as_str(), Some("hello"));
    }

    #[test]
    fn test_wire_to_nb_array() {
        let wire = WireValue::Array(vec![
            WireValue::Integer(1),
            WireValue::Number(2.5),
            WireValue::Bool(false),
        ]);
        let nb = wire_to_nb(&wire);
        let arr = nb.as_any_array().expect("Expected array").to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64(), Some(1));
        assert_eq!(arr[1].as_f64(), Some(2.5));
        assert_eq!(arr[2].as_bool(), Some(false));
    }

    #[test]
    fn test_wire_to_nb_result() {
        // Ok result
        let wire = WireValue::Result {
            ok: true,
            value: Box::new(WireValue::Integer(7)),
        };
        let nb = wire_to_nb(&wire);
        let inner = nb.as_ok_inner().expect("Expected Ok");
        assert_eq!(inner.as_i64(), Some(7));

        // Err result
        let wire = WireValue::Result {
            ok: false,
            value: Box::new(WireValue::String("fail".to_string())),
        };
        let nb = wire_to_nb(&wire);
        let inner = nb.as_err_inner().expect("Expected Err");
        assert_eq!(inner.as_str(), Some("fail"));
    }

    #[test]
    fn test_nb_roundtrip_basic() {
        let ctx = get_dummy_context();

        // String roundtrip
        let original = ValueWord::from_string(Arc::new("roundtrip".to_string()));
        let wire = nb_to_wire(&original, &ctx);
        let back = wire_to_nb(&wire);
        assert_eq!(back.as_str(), Some("roundtrip"));

        // Integer roundtrip
        let original = ValueWord::from_i64(12345);
        let wire = nb_to_wire(&original, &ctx);
        let back = wire_to_nb(&wire);
        assert_eq!(back.as_i64(), Some(12345));

        // Float roundtrip
        let original = ValueWord::from_f64(2.718);
        let wire = nb_to_wire(&original, &ctx);
        let back = wire_to_nb(&wire);
        assert_eq!(back.as_f64(), Some(2.718));

        // Bool roundtrip
        let original = ValueWord::from_bool(true);
        let wire = nb_to_wire(&original, &ctx);
        let back = wire_to_nb(&wire);
        assert_eq!(back.as_bool(), Some(true));

        // None roundtrip
        let wire = nb_to_wire(&ValueWord::none(), &ctx);
        let back = wire_to_nb(&wire);
        assert!(back.is_none());
    }

    #[test]
    fn test_nb_roundtrip_array() {
        let ctx = get_dummy_context();
        let original = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_i64(10),
            ValueWord::from_f64(20.5),
            ValueWord::from_string(Arc::new("x".to_string())),
        ]));
        let wire = nb_to_wire(&original, &ctx);
        let back = wire_to_nb(&wire);
        let arr = back.as_any_array().expect("Expected array").to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64(), Some(10));
        assert_eq!(arr[1].as_f64(), Some(20.5));
        assert_eq!(arr[2].as_str(), Some("x"));
    }

    #[test]
    fn test_nb_to_wire_decimal() {
        let ctx = get_dummy_context();
        let d = rust_decimal::Decimal::new(314, 2); // 3.14
        let nb = ValueWord::from_decimal(d);
        let wire = nb_to_wire(&nb, &ctx);
        assert_eq!(wire, WireValue::Number(3.14));
    }

    #[test]
    fn test_nb_to_wire_typed_object() {
        let ctx = get_dummy_context();
        let obj = crate::type_schema::typed_object_from_pairs(&[
            ("a", ValueWord::from_f64(1.0)),
            ("b", ValueWord::from_string(Arc::new("two".to_string()))),
        ]);
        let nb = obj;
        let wire = nb_to_wire(&nb, &ctx);

        if let WireValue::Object(map) = wire {
            assert_eq!(map.get("a"), Some(&WireValue::Number(1.0)));
            assert_eq!(map.get("b"), Some(&WireValue::String("two".to_string())));
        } else {
            panic!("Expected Object, got {:?}", wire);
        }
    }

    #[test]
    fn test_nb_envelope_creation() {
        let ctx = get_dummy_context();
        let nb = ValueWord::from_f64(3.14);
        let envelope = nb_to_envelope(&nb, "Number", &ctx);
        assert_eq!(envelope.type_info.name, "Number");
    }

    #[test]
    fn test_native_scalar_wire_roundtrip_preserves_width() {
        let ctx = get_dummy_context();
        let cases = vec![
            (
                ValueWord::from_native_i8(-8),
                WireValue::I8(-8),
                shape_value::heap_value::NativeScalar::I8(-8),
            ),
            (
                ValueWord::from_native_u8(255),
                WireValue::U8(255),
                shape_value::heap_value::NativeScalar::U8(255),
            ),
            (
                ValueWord::from_native_i16(-1024),
                WireValue::I16(-1024),
                shape_value::heap_value::NativeScalar::I16(-1024),
            ),
            (
                ValueWord::from_native_u16(65530),
                WireValue::U16(65530),
                shape_value::heap_value::NativeScalar::U16(65530),
            ),
            (
                ValueWord::from_native_i32(-123_456),
                WireValue::I32(-123_456),
                shape_value::heap_value::NativeScalar::I32(-123_456),
            ),
            (
                ValueWord::from_native_u32(4_000_000_000),
                WireValue::U32(4_000_000_000),
                shape_value::heap_value::NativeScalar::U32(4_000_000_000),
            ),
            (
                ValueWord::from_native_scalar(shape_value::heap_value::NativeScalar::I64(
                    -9_223_372_036_854_775_000,
                )),
                WireValue::I64(-9_223_372_036_854_775_000),
                shape_value::heap_value::NativeScalar::I64(-9_223_372_036_854_775_000),
            ),
            (
                ValueWord::from_native_u64(18_000_000_000),
                WireValue::U64(18_000_000_000),
                shape_value::heap_value::NativeScalar::U64(18_000_000_000),
            ),
            (
                ValueWord::from_native_isize(12345isize),
                WireValue::Isize(12345),
                shape_value::heap_value::NativeScalar::Isize(12345isize),
            ),
            (
                ValueWord::from_native_usize(54321usize),
                WireValue::Usize(54321),
                shape_value::heap_value::NativeScalar::Usize(54321usize),
            ),
            (
                ValueWord::from_native_ptr(0x1234usize),
                WireValue::Ptr(0x1234),
                shape_value::heap_value::NativeScalar::Ptr(0x1234usize),
            ),
            (
                ValueWord::from_native_f32(3.5f32),
                WireValue::F32(3.5f32),
                shape_value::heap_value::NativeScalar::F32(3.5f32),
            ),
        ];

        for (nb, expected_wire, expected_scalar) in cases {
            let wire = nb_to_wire(&nb, &ctx);
            assert_eq!(wire, expected_wire);

            let roundtrip = wire_to_nb(&wire);
            assert_eq!(roundtrip.as_native_scalar(), Some(expected_scalar));
        }
    }
}
