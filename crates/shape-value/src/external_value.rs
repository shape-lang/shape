//! ExternalValue: a serde-serializable value for display, wire, and debug.
//!
//! ExternalValue is the canonical format for values that cross system boundaries:
//! - Wire serialization (JSON, MessagePack, etc.)
//! - Debugger display
//! - REPL output
//! - Remote protocol
//!
//! It contains NO function refs, closures, raw pointers, or VM internals.
//! All variants are safe to serialize with serde.

use crate::heap_value::HeapValue;
use crate::tags;
use crate::value_word::{ValueWord, ValueWordExt};
use std::collections::BTreeMap;
use std::fmt;

/// A serde-serializable value with no VM internals.
///
/// This is the "external" representation of a Shape value, suitable for
/// display, wire serialization, and debugging. It contains no function
/// references, closures, or raw pointers.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum ExternalValue {
    /// 64-bit float
    Number(f64),
    /// 64-bit signed integer
    Int(i64),
    /// Boolean
    Bool(bool),
    /// String
    String(String),
    /// None / null
    None,
    /// Unit (void)
    Unit,
    /// Homogeneous or heterogeneous array
    Array(Vec<ExternalValue>),
    /// Untyped object (field name -> value)
    Object(BTreeMap<String, ExternalValue>),
    /// Typed object with schema name
    TypedObject {
        name: String,
        fields: BTreeMap<String, ExternalValue>,
    },
    /// Enum variant
    Enum {
        name: String,
        variant: String,
        data: Box<ExternalValue>,
    },
    /// Duration
    Duration { secs: u64, nanos: u32 },
    /// Timestamp (ISO 8601 string for portability)
    Time(String),
    /// Decimal (string representation for precision)
    Decimal(String),
    /// Error message
    Error(String),
    /// Result::Ok
    Ok(Box<ExternalValue>),
    /// Range
    Range {
        start: Option<Box<ExternalValue>>,
        end: Option<Box<ExternalValue>>,
        inclusive: bool,
    },
    /// DataTable summary (not full data — just metadata)
    DataTable { rows: usize, columns: Vec<String> },
    /// Opaque value that cannot be externalized (function, closure, etc.)
    Opaque(String),
}

/// Trait for looking up schema metadata from a schema_id.
///
/// Implemented by `TypeSchemaRegistry` in shape-runtime.
/// This abstraction lets shape-value convert TypedObjects to ExternalValue
/// without depending on shape-runtime.
pub trait SchemaLookup {
    /// Get the type name for a schema_id, or None if unknown.
    fn type_name(&self, schema_id: u64) -> Option<&str>;

    /// Get ordered field names for a schema_id, or None if unknown.
    fn field_names(&self, schema_id: u64) -> Option<Vec<&str>>;
}

/// A no-op schema lookup for contexts where schema info is unavailable.
/// TypedObjects will be converted with placeholder names.
pub struct NoSchemaLookup;

impl SchemaLookup for NoSchemaLookup {
    fn type_name(&self, _schema_id: u64) -> Option<&str> {
        std::option::Option::None
    }
    fn field_names(&self, _schema_id: u64) -> Option<Vec<&str>> {
        std::option::Option::None
    }
}

/// Convert a ValueWord value to an ExternalValue.
///
/// This is the canonical way to externalize a VM value for display, wire, or debug.
/// Schema lookup is needed to resolve TypedObject field names.
pub fn nb_to_external(nb: &ValueWord, schemas: &dyn SchemaLookup) -> ExternalValue {
    let bits = nb.raw_bits();

    if !tags::is_tagged(bits) {
        let f = f64::from_bits(bits);
        return if f.is_nan() {
            ExternalValue::Number(f64::NAN)
        } else {
            ExternalValue::Number(f)
        };
    }

    match tags::get_tag(bits) {
        tags::TAG_INT => ExternalValue::Int(tags::sign_extend_i48(tags::get_payload(bits))),
        tags::TAG_BOOL => ExternalValue::Bool(tags::get_payload(bits) != 0),
        tags::TAG_NONE => ExternalValue::None,
        tags::TAG_UNIT => ExternalValue::Unit,
        tags::TAG_FUNCTION => {
            ExternalValue::Opaque(format!("<function:{}>", tags::get_payload(bits) as u16))
        }
        tags::TAG_MODULE_FN => {
            ExternalValue::Opaque(format!("<module_fn:{}>", tags::get_payload(bits) as u32))
        }
        tags::TAG_REF => ExternalValue::Opaque("<ref>".to_string()),
        tags::TAG_HEAP => {
            // Handle unified arrays (bit-47 set)
            if tags::is_unified_heap(bits) {
                let kind = unsafe { tags::unified_heap_kind(bits) };
                if kind == tags::HEAP_KIND_ARRAY as u16 {
                    let arr = unsafe { crate::unified_array::UnifiedArray::from_heap_bits(bits) };
                    let items: Vec<ExternalValue> = (0..arr.len())
                        .map(|i| {
                            let elem = unsafe { ValueWord::clone_from_bits(*arr.get(i).unwrap()) };
                            nb_to_external(&elem, schemas)
                        })
                        .collect();
                    return ExternalValue::Array(items);
                }
                return ExternalValue::Opaque(format!("<unified:{}>", kind));
            }
            // cold-path: as_heap_ref retained — external value multi-variant conversion
            if let Some(hv) = nb.as_heap_ref() { // cold-path
                heap_to_external(hv, schemas)
            } else {
                ExternalValue::Opaque("<invalid_heap>".to_string())
            }
        }
        _ => ExternalValue::Opaque("<unknown_tag>".to_string()),
    }
}

/// Convert a HeapValue to an ExternalValue.
fn heap_to_external(hv: &HeapValue, schemas: &dyn SchemaLookup) -> ExternalValue {
    match hv {
        HeapValue::String(s) => ExternalValue::String((**s).clone()),
        HeapValue::Array(arr) => {
            let items: Vec<ExternalValue> =
                arr.iter().map(|v| nb_to_external(v, schemas)).collect();
            ExternalValue::Array(items)
        }
        HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask,
        } => {
            let type_name = schemas
                .type_name(*schema_id)
                .unwrap_or("unknown")
                .to_string();
            let field_names_opt = schemas.field_names(*schema_id);

            let mut fields = BTreeMap::new();
            let names: Vec<String> = if let Some(names) = field_names_opt {
                names.iter().map(|s| s.to_string()).collect()
            } else {
                (0..slots.len()).map(|i| format!("_{i}")).collect()
            };

            for (i, name) in names.into_iter().enumerate() {
                if i >= slots.len() {
                    break;
                }
                let is_heap = (heap_mask >> i) & 1 == 1;
                let ev = if is_heap {
                    // Heap slot: convert via HeapValue
                    let nb_val = slots[i].as_heap_nb();
                    nb_to_external(&nb_val, schemas)
                } else {
                    // Non-heap slot: raw bits, interpret as f64 (most common for non-heap fields)
                    ExternalValue::Number(slots[i].as_f64())
                };
                fields.insert(name, ev);
            }

            ExternalValue::TypedObject {
                name: type_name,
                fields,
            }
        }
        HeapValue::Closure { function_id, .. } => {
            ExternalValue::Opaque(format!("<closure:{function_id}>"))
        }
        HeapValue::Decimal(d) => ExternalValue::Decimal(d.to_string()),
        HeapValue::BigInt(i) => ExternalValue::Int(*i),
        HeapValue::HostClosure(_) => ExternalValue::Opaque("<host_closure>".to_string()),

        // DataTable family
        HeapValue::DataTable(dt) => ExternalValue::DataTable {
            rows: dt.row_count(),
            columns: dt.column_names().iter().map(|s| s.to_string()).collect(),
        },
        HeapValue::TableView(crate::heap_value::TableViewData::TypedTable { table, .. }) => ExternalValue::DataTable {
            rows: table.row_count(),
            columns: table.column_names().iter().map(|s| s.to_string()).collect(),
        },
        HeapValue::TableView(crate::heap_value::TableViewData::RowView { .. }) => ExternalValue::Opaque("<row_view>".to_string()),
        HeapValue::TableView(crate::heap_value::TableViewData::ColumnRef { .. }) => ExternalValue::Opaque("<column_ref>".to_string()),
        HeapValue::TableView(crate::heap_value::TableViewData::IndexedTable { table, .. }) => ExternalValue::DataTable {
            rows: table.row_count(),
            columns: table.column_names().iter().map(|s| s.to_string()).collect(),
        },
        HeapValue::ProjectedRef(..) => ExternalValue::Opaque("<ref>".to_string()),

        // Container types
        HeapValue::Range {
            start,
            end,
            inclusive,
        } => ExternalValue::Range {
            start: start.as_ref().map(|v| Box::new(nb_to_external(v, schemas))),
            end: end.as_ref().map(|v| Box::new(nb_to_external(v, schemas))),
            inclusive: *inclusive,
        },
        HeapValue::Enum(e) => ExternalValue::Enum {
            name: e.enum_name.clone(),
            variant: e.variant.clone(),
            data: Box::new(match &e.payload {
                crate::enums::EnumPayload::Unit => ExternalValue::None,
                crate::enums::EnumPayload::Tuple(nbs) => {
                    if nbs.len() == 1 {
                        nb_to_external(&nbs[0], schemas)
                    } else {
                        ExternalValue::Array(
                            nbs.iter().map(|v| nb_to_external(v, schemas)).collect(),
                        )
                    }
                }
                crate::enums::EnumPayload::Struct(fields) => {
                    let mut map = BTreeMap::new();
                    for (k, v) in fields {
                        map.insert(k.clone(), nb_to_external(v, schemas));
                    }
                    ExternalValue::Object(map)
                }
            }),
        },
        HeapValue::Some(v) => nb_to_external(v, schemas),
        HeapValue::Ok(v) => ExternalValue::Ok(Box::new(nb_to_external(v, schemas))),
        HeapValue::Err(v) => ExternalValue::Error(format!("{:?}", nb_to_external(v, schemas))),

        // Async
        HeapValue::Future(id) => ExternalValue::Opaque(format!("<future:{id}>")),
        HeapValue::TaskGroup { kind, task_ids } => {
            ExternalValue::Opaque(format!("<task_group:kind={kind},tasks={}>", task_ids.len()))
        }

        // Trait dispatch
        HeapValue::TraitObject { value, .. } => nb_to_external(value, schemas),

        // Temporal
        HeapValue::Temporal(crate::heap_value::TemporalData::DateTime(t)) => ExternalValue::Time(t.to_rfc3339()),
        HeapValue::Temporal(crate::heap_value::TemporalData::Duration(d)) => {
            let secs = d.value as u64;
            ExternalValue::Duration { secs, nanos: 0 }
        }
        HeapValue::Temporal(crate::heap_value::TemporalData::TimeSpan(ts)) => ExternalValue::Duration {
            secs: ts.num_seconds().unsigned_abs(),
            nanos: (ts.subsec_nanos().unsigned_abs()),
        },
        HeapValue::Temporal(crate::heap_value::TemporalData::Timeframe(tf)) => ExternalValue::String(format!("{tf:?}")),
        HeapValue::Temporal(crate::heap_value::TemporalData::TimeReference(_)) => ExternalValue::Opaque("<time_reference>".to_string()),
        HeapValue::Temporal(crate::heap_value::TemporalData::DateTimeExpr(_)) => ExternalValue::Opaque("<datetime_expr>".to_string()),
        HeapValue::Temporal(crate::heap_value::TemporalData::DataDateTimeRef(_)) => ExternalValue::Opaque("<data_datetime_ref>".to_string()),

        // Rare
        HeapValue::Rare(crate::heap_value::RareHeapData::ExprProxy(s)) => ExternalValue::Opaque(format!("<expr_proxy:{s}>")),
        HeapValue::Rare(crate::heap_value::RareHeapData::FilterExpr(_)) => ExternalValue::Opaque("<filter_expr>".to_string()),
        HeapValue::Rare(crate::heap_value::RareHeapData::TypeAnnotation(_)) => ExternalValue::Opaque("<type_annotation>".to_string()),
        HeapValue::Rare(crate::heap_value::RareHeapData::TypeAnnotatedValue { type_name, value }) => {
            let inner = nb_to_external(value, schemas);
            ExternalValue::TypedObject {
                name: type_name.clone(),
                fields: BTreeMap::from([("value".to_string(), inner)]),
            }
        }
        HeapValue::Rare(crate::heap_value::RareHeapData::PrintResult(pr)) => ExternalValue::String(pr.rendered.clone()),
        HeapValue::Rare(crate::heap_value::RareHeapData::SimulationCall(data)) => ExternalValue::Opaque(format!(
            "<simulation_call:{} params={}>",
            data.name,
            data.params.len()
        )),
        HeapValue::Rare(crate::heap_value::RareHeapData::DataReference(data)) => {
            let mut fields = BTreeMap::new();
            fields.insert(
                "datetime".to_string(),
                ExternalValue::Time(data.datetime.to_rfc3339()),
            );
            fields.insert("id".to_string(), ExternalValue::String(data.id.clone()));
            fields.insert(
                "timeframe".to_string(),
                ExternalValue::String(format!("{:?}", data.timeframe)),
            );
            ExternalValue::Object(fields)
        }
        HeapValue::FunctionRef { name, .. } => {
            ExternalValue::Opaque(format!("<function_ref:{name}>"))
        }

        HeapValue::Instant(t) => ExternalValue::Opaque(format!("<instant:{:?}>", t.elapsed())),

        HeapValue::IoHandle(data) => {
            let status = if data.is_open() { "open" } else { "closed" };
            ExternalValue::Opaque(format!("<io_handle:{}:{}>", data.path, status))
        }

        HeapValue::NativeScalar(v) => {
            if let Some(i) = v.as_i64() {
                ExternalValue::Int(i)
            } else {
                ExternalValue::Number(v.as_f64())
            }
        }
        HeapValue::NativeView(v) => ExternalValue::Opaque(format!(
            "<{}:{}@0x{:x}>",
            if v.mutable { "cmut" } else { "cview" },
            v.layout.name,
            v.ptr
        )),
        HeapValue::HashMap(d) => {
            let mut fields = BTreeMap::new();
            for (k, v) in d.keys.iter().zip(d.values.iter()) {
                fields.insert(format!("{}", k), nb_to_external(v, schemas));
            }
            ExternalValue::Object(fields)
        }
        HeapValue::Set(d) => {
            ExternalValue::Array(d.items.iter().map(|v| nb_to_external(v, schemas)).collect())
        }
        HeapValue::Deque(d) => {
            ExternalValue::Array(d.items.iter().map(|v| nb_to_external(v, schemas)).collect())
        }
        HeapValue::PriorityQueue(d) => {
            ExternalValue::Array(d.items.iter().map(|v| nb_to_external(v, schemas)).collect())
        }
        HeapValue::Content(node) => ExternalValue::String(format!("{}", node)),
        HeapValue::SharedCell(arc) => nb_to_external(&arc.read().unwrap(), schemas),
        // TypedArray
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::I64(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Int(v)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::F64(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Number(v)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::Bool(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Bool(v != 0)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::I8(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Int(v as i64)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::I16(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Int(v as i64)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::I32(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Int(v as i64)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::U8(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Int(v as i64)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::U16(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Int(v as i64)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::U32(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Int(v as i64)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::U64(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Int(v as i64)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::F32(a)) => {
            ExternalValue::Array(a.iter().map(|&v| ExternalValue::Number(v as f64)).collect())
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::Matrix(m)) => {
            ExternalValue::Opaque(format!("<Mat<number>:{}x{}>", m.rows, m.cols))
        }
        HeapValue::TypedArray(crate::heap_value::TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        }) => {
            let slice = &parent.data[*offset as usize..(*offset + *len) as usize];
            ExternalValue::Array(slice.iter().map(|&v| ExternalValue::Number(v)).collect())
        }
        HeapValue::Iterator(_) => ExternalValue::Opaque("<iterator>".to_string()),
        HeapValue::Generator(_) => ExternalValue::Opaque("<generator>".to_string()),
        // Concurrency
        HeapValue::Concurrency(crate::heap_value::ConcurrencyData::Mutex(_)) => ExternalValue::Opaque("<mutex>".to_string()),
        HeapValue::Concurrency(crate::heap_value::ConcurrencyData::Atomic(a)) => {
            ExternalValue::Int(a.inner.load(std::sync::atomic::Ordering::Relaxed))
        }
        HeapValue::Concurrency(crate::heap_value::ConcurrencyData::Channel(c)) => {
            if c.is_sender() {
                ExternalValue::Opaque("<channel:sender>".to_string())
            } else {
                ExternalValue::Opaque("<channel:receiver>".to_string())
            }
        }
        HeapValue::Concurrency(crate::heap_value::ConcurrencyData::Lazy(l)) => {
            if let Ok(guard) = l.value.lock() {
                if let Some(val) = guard.as_ref() {
                    return nb_to_external(val, schemas);
                }
            }
            ExternalValue::Opaque("<lazy:uninitialized>".to_string())
        }
        HeapValue::Char(c) => ExternalValue::String(c.to_string()),
    }
}

/// Convert an ExternalValue back to a ValueWord value.
///
/// This is used for deserializing wire values back into the VM.
/// Note: Opaque values cannot be round-tripped.
pub fn external_to_nb(ev: &ExternalValue, schemas: &dyn SchemaLookup) -> ValueWord {
    let _ = schemas; // schemas needed for TypedObject reconstruction (future use)
    match ev {
        ExternalValue::Number(n) => ValueWord::from_f64(*n),
        ExternalValue::Int(i) => ValueWord::from_i64(*i),
        ExternalValue::Bool(b) => ValueWord::from_bool(*b),
        ExternalValue::String(s) => ValueWord::from_string(std::sync::Arc::new(s.clone())),
        ExternalValue::None => ValueWord::none(),
        ExternalValue::Unit => ValueWord::unit(),
        ExternalValue::Array(items) => {
            let nbs: crate::value::VMArrayBuf =
                items.iter().map(|v| external_to_nb(v, schemas)).collect();
            ValueWord::from_array(crate::value::vmarray_from_vec(nbs))
        }
        ExternalValue::Decimal(s) => {
            if let Ok(d) = s.parse::<rust_decimal::Decimal>() {
                ValueWord::from_decimal(d)
            } else {
                ValueWord::from_string(std::sync::Arc::new(s.clone()))
            }
        }
        ExternalValue::Ok(inner) => ValueWord::from_ok(external_to_nb(inner, schemas)),
        ExternalValue::Error(msg) => {
            ValueWord::from_err(ValueWord::from_string(std::sync::Arc::new(msg.clone())))
        }
        ExternalValue::Range {
            start,
            end,
            inclusive,
        } => ValueWord::from_range(
            start.as_ref().map(|v| external_to_nb(v, schemas)),
            end.as_ref().map(|v| external_to_nb(v, schemas)),
            *inclusive,
        ),
        // Complex types that can't be fully round-tripped return None
        ExternalValue::Object(_)
        | ExternalValue::TypedObject { .. }
        | ExternalValue::Enum { .. }
        | ExternalValue::Duration { .. }
        | ExternalValue::Time(_)
        | ExternalValue::DataTable { .. }
        | ExternalValue::Opaque(_) => ValueWord::none(),
    }
}

impl fmt::Display for ExternalValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalValue::Number(n) => {
                if n.is_nan() {
                    write!(f, "NaN")
                } else if n.is_infinite() {
                    if n.is_sign_positive() {
                        write!(f, "Infinity")
                    } else {
                        write!(f, "-Infinity")
                    }
                } else if *n == (*n as i64) as f64 && n.is_finite() {
                    // Display whole numbers without decimal point
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{n}")
                }
            }
            ExternalValue::Int(i) => write!(f, "{i}"),
            ExternalValue::Bool(b) => write!(f, "{b}"),
            ExternalValue::String(s) => write!(f, "{s}"),
            ExternalValue::None => write!(f, "none"),
            ExternalValue::Unit => write!(f, "()"),
            ExternalValue::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            ExternalValue::Object(fields) => {
                write!(f, "{{")?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
            ExternalValue::TypedObject { name, fields } => {
                write!(f, "{name} {{")?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
            ExternalValue::Enum {
                name,
                variant,
                data,
            } => {
                write!(f, "{name}::{variant}")?;
                if **data != ExternalValue::None {
                    write!(f, "({data})")?;
                }
                Ok(())
            }
            ExternalValue::Duration { secs, nanos } => {
                if *nanos > 0 {
                    write!(f, "{secs}.{:09}s", nanos)
                } else {
                    write!(f, "{secs}s")
                }
            }
            ExternalValue::Time(iso) => write!(f, "{iso}"),
            ExternalValue::Decimal(d) => write!(f, "{d}"),
            ExternalValue::Error(msg) => write!(f, "Error({msg})"),
            ExternalValue::Ok(inner) => write!(f, "Ok({inner})"),
            ExternalValue::Range {
                start,
                end,
                inclusive,
            } => {
                if let Some(s) = start {
                    write!(f, "{s}")?;
                }
                if *inclusive {
                    write!(f, "..=")?;
                } else {
                    write!(f, "..")?;
                }
                if let Some(e) = end {
                    write!(f, "{e}")?;
                }
                Ok(())
            }
            ExternalValue::DataTable { rows, columns } => {
                write!(f, "DataTable({rows} rows, {} cols)", columns.len())
            }
            ExternalValue::Opaque(desc) => write!(f, "{desc}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value_word::{ValueWord, ValueWordExt};

    #[test]
    fn test_number_roundtrip() {
        let nb = ValueWord::from_f64(3.14);
        let ev = nb_to_external(&nb, &NoSchemaLookup);
        assert!(matches!(ev, ExternalValue::Number(n) if (n - 3.14).abs() < f64::EPSILON));
        let back = external_to_nb(&ev, &NoSchemaLookup);
        assert!((back.as_f64().unwrap() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_int_roundtrip() {
        let nb = ValueWord::from_i64(42);
        let ev = nb_to_external(&nb, &NoSchemaLookup);
        assert_eq!(ev, ExternalValue::Int(42));
        let back = external_to_nb(&ev, &NoSchemaLookup);
        assert_eq!(back.as_i64().unwrap(), 42);
    }

    #[test]
    fn test_bool_roundtrip() {
        let nb = ValueWord::from_bool(true);
        let ev = nb_to_external(&nb, &NoSchemaLookup);
        assert_eq!(ev, ExternalValue::Bool(true));
    }

    #[test]
    fn test_string_roundtrip() {
        let nb = ValueWord::from_string(std::sync::Arc::new("hello".to_string()));
        let ev = nb_to_external(&nb, &NoSchemaLookup);
        assert_eq!(ev, ExternalValue::String("hello".to_string()));
        let back = external_to_nb(&ev, &NoSchemaLookup);
        assert_eq!(back.as_str().unwrap(), "hello");
    }

    #[test]
    fn test_none_and_unit() {
        assert_eq!(
            nb_to_external(&ValueWord::none(), &NoSchemaLookup),
            ExternalValue::None
        );
        assert_eq!(
            nb_to_external(&ValueWord::unit(), &NoSchemaLookup),
            ExternalValue::Unit
        );
    }

    #[test]
    fn test_function_is_opaque() {
        let nb = ValueWord::from_function(42);
        let ev = nb_to_external(&nb, &NoSchemaLookup);
        assert!(matches!(ev, ExternalValue::Opaque(_)));
    }

    #[test]
    fn test_array() {
        let arr = vec![ValueWord::from_i64(1), ValueWord::from_i64(2)];
        let nb = ValueWord::from_array(crate::value::vmarray_from_vec(arr));
        let ev = nb_to_external(&nb, &NoSchemaLookup);
        assert_eq!(
            ev,
            ExternalValue::Array(vec![ExternalValue::Int(1), ExternalValue::Int(2)])
        );
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ExternalValue::Number(3.14)), "3.14");
        assert_eq!(format!("{}", ExternalValue::Int(42)), "42");
        assert_eq!(format!("{}", ExternalValue::Bool(true)), "true");
        assert_eq!(format!("{}", ExternalValue::String("hi".into())), "hi");
        assert_eq!(format!("{}", ExternalValue::None), "none");
        assert_eq!(format!("{}", ExternalValue::Unit), "()");
        assert_eq!(
            format!(
                "{}",
                ExternalValue::Array(vec![ExternalValue::Int(1), ExternalValue::Int(2)])
            ),
            "[1, 2]"
        );
    }

    #[test]
    fn test_serde_json_roundtrip() {
        let ev = ExternalValue::TypedObject {
            name: "Candle".to_string(),
            fields: BTreeMap::from([
                ("open".to_string(), ExternalValue::Number(100.0)),
                ("close".to_string(), ExternalValue::Number(105.5)),
            ]),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: ExternalValue = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }
}
