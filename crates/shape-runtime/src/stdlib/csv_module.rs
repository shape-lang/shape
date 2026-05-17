//! Native `csv` module for CSV parsing and serialization.
//!
//! Phase 2d Array cluster migration: `parse`, `stringify`, `read_file`,
//! and `is_valid` ported to the typed marshal layer using
//! `TypedArrayData::String` (rows of strings) inside
//! `the-deleted-heterogeneous-element-carrier` (array of rows).
//!
//! Stage C HashMap-marshal P1(b) activation (2026-05-07): `parse_records`
//! and `stringify_records` activated using `HeapValue::HashMap(HashMapData)`
//! variant. Each record is `Arc<HeapValue::HashMap>` carrying string keys
//! (header row) → string values (record fields). Insertion order
//! preserved via the eager-bucket-only HashMapData buffer pair.
//!
//! Tests deferred — ValueWord-based test fixtures can't compile and
//! aren't reconstructed until the shape-vm cascade provides a typed
//! test harness, mirroring the file_ops migration in commit d716482.

use crate::marshal::{register_typed_fn_1, register_typed_fn_2_full};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::type_schema::register_predeclared_any_schema;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::{HeapValue, TypedObjectStorage};
use shape_value::{NativeKind, ValueSlot};
use std::sync::Arc;

// W17-out-of-bundle-A-followups (2026-05-12): `row_to_heap` was the
// per-row `Arc<HeapValue::TypedArray(TypedArrayData::String)>` builder
// for the pre-rewire `csv.parse` / `csv.read_file` `Array<Array<string>>`
// shape. Both now surface-and-stop pending the
// W17-typed-carrier-array-typedarray follow-up; the helper is removed
// alongside its construction call sites.

/// Read a `Vec<Vec<String>>` from a `Vec<Arc<HeapValue>>` whose elements
/// are each the deleted outer typed-array arm.
///
/// V3-S5 ckpt-5-prime²c (2026-05-15) SURFACE-AND-STOP: this consumer
/// pattern-matched `HeapValue::TypedArray(Arc<TypedArrayData>)` to extract
/// the per-row `Vec<String>`. Both the outer arm and the inner
/// `TypedArrayData::String` shape are deleted (V3-S5 ckpt-1/ckpt-4/ckpt-5)
/// — the per-row carrier is now a `*mut TypedArray<*const StringObj>` raw
/// pointer with no `HeapValue::*` wrapper, so `Vec<Arc<HeapValue>>` cannot
/// express it. Pairs with the Round 2 `Vec<Arc<HeapValue>>` rewire
/// follow-up at `marshal.rs:FromSlot<Vec<Arc<HeapValue>>>` and the
/// `from_typed_array_<T>` constructor wave at `slot.rs:142`.
fn rows_from_heap_array(
    rows: &[Arc<HeapValue>],
    fn_name: &str,
) -> Result<Vec<Vec<String>>, String> {
    let _ = rows;
    Err(format!(
        "{}: V3-S5 ckpt-5-prime²c SURFACE — per-row outer-array-arm \
         consumer needs Vec<Arc<HeapValue>> rewire for the deleted \
         outer-array-arm. Round 2 follow-up. ADR-006 §2.7.24 Q25.A \
         SUPERSEDED.",
        fn_name
    ))
}

/// Create the `csv` module with CSV parsing and serialization functions.
pub fn create_csv_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::csv");
    module.description = "CSV parsing and serialization".to_string();

    // csv.parse(text: string) -> Array<Array<string>>
    //
    // W17-out-of-bundle-A-followups (2026-05-12): surface-and-stop.
    // `Array<Array<string>>` is homogeneous in
    // `HeapKind::TypedArray (TypedArrayData::String)` — the natural
    // Q25.A specialized variant is
    // `TypedArrayData::TypedArray(Arc<TypedBuffer<Arc<TypedArrayData>>>)`,
    // but adding a nested-TypedArray variant is out of bundle-A-followups
    // scope (the prompt forbids new HeapKind variants and an added
    // TypedArrayData variant cascades through ~40 exhaustive matches).
    // Users wanting per-record dispatch should use `csv.parse_records`
    // which lowers to `Array<TypedObject>` via the C+ precedent.
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse",
        "Parse CSV text into an array of rows (each row is an array of strings)",
        "text",
        "string",
        ConcreteType::ArrayHeapValue("Array<Array<string>>".to_string()),
        |text, _ctx| {
            let _ = text;
            // phase-2d-hardening:(f) — csv.parse surface-and-stop:
            // Array<Array<string>> needs TypedArrayData::TypedArray
            // (nested-TypedArray) variant. Use csv.parse_records for
            // per-record TypedObject dispatch in the meantime.
            Err(format!(
                "csv.parse(): SURFACE — `Array<Array<string>>` needs a \
                 nested-array variant in ADR-006 \
                 §2.7.24 Q25.A's spec list. Tracked as \
                 W17-typed-carrier-array-typedarray follow-up (out of \
                 bundle-A-followups scope). Use `csv.parse_records` for \
                 per-record TypedObject access. ADR-006 §2.7.24 Q25.A."
            ))
        },
    );

    // csv.stringify(data: Array<Array<string>>, delimiter?: string) -> string
    register_typed_fn_2_full::<_, Vec<Arc<HeapValue>>, Arc<String>>(
        &mut module,
        "stringify",
        "Convert an array of rows to a CSV string",
        [
            ModuleParam {
                name: "data".to_string(),
                type_name: "Array<Array<string>>".to_string(),
                required: true,
                description: "Array of rows, each row is an array of field strings".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "delimiter".to_string(),
                type_name: "string".to_string(),
                required: false,
                description: "Field delimiter character (default: comma)".to_string(),
                default_snippet: Some("\",\"".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        |data, delimiter, _ctx| {
            let rows = rows_from_heap_array(&data, "csv.stringify()")?;

            let delim_byte = delimiter
                .as_bytes()
                .first()
                .copied()
                .unwrap_or(b',');

            let mut writer = csv::WriterBuilder::new()
                .delimiter(delim_byte)
                .from_writer(Vec::new());

            for row in &rows {
                writer
                    .write_record(row)
                    .map_err(|e| format!("csv.stringify() failed: {}", e))?;
            }

            let bytes = writer
                .into_inner()
                .map_err(|e| format!("csv.stringify() failed to flush: {}", e))?;
            let output = String::from_utf8(bytes)
                .map_err(|e| format!("csv.stringify() UTF-8 error: {}", e))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::String(output)))
        },
    );

    // csv.read_file(path: string) -> Result<Array<Array<string>>>
    //
    // W17-out-of-bundle-A-followups (2026-05-12): surface-and-stop, same
    // shape as csv.parse above — `Array<Array<string>>` needs the
    // nested-TypedArray variant in Q25.A's spec list. Tracked as
    // W17-typed-carrier-array-typedarray follow-up.
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "read_file",
        "Read and parse a CSV file into an array of rows",
        "path",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::ArrayHeapValue(
            "Array<Array<string>>".to_string(),
        ))),
        |path, _ctx| {
            let _ = path;
            // phase-2d-hardening:(f) — csv.read_file surface-and-stop:
            // same nested-TypedArray gap as csv.parse.
            Err(format!(
                "csv.read_file(): SURFACE — `Array<Array<string>>` needs a \
                 nested-array variant in ADR-006 \
                 §2.7.24 Q25.A's spec list. Tracked as \
                 W17-typed-carrier-array-typedarray follow-up. ADR-006 §2.7.24 Q25.A."
            ))
        },
    );

    // csv.is_valid(text: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "is_valid",
        "Check if a string is valid CSV",
        "text",
        "string",
        ConcreteType::Bool,
        |text, _ctx| {
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(text.as_bytes());

            let valid = reader.records().all(|r| r.is_ok());
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(valid)))
        },
    );

    // csv.parse_records(text: string) -> Array<{header→string}>
    //
    // Parses CSV text using the first row as header keys; each subsequent
    // row becomes a TypedObject keyed by header column names. Header
    // order = field order = column order.
    //
    // W17-out-of-bundle-A-followups (2026-05-12): per the C+ precedent
    // recorded in `phase-2d-playbook.md` §3, each record is constructed
    // as `Arc<HeapValue::TypedObject>` with a schema derived from the
    // CSV header row. The outer array lowers to
    // `TypedArrayData::TypedObject` via the marshal-boundary
    // `build_specialized_from_heap_arcs` dispatch. The pre-rewire
    // `HashMap<string, string>` shape — which routed through the
    // deleted `TypedArrayData::HeapValue` carrier — is replaced by the
    // per-header field schema, which is what user code naturally
    // addresses (`record.column_name` rather than `record["column_name"]`).
    //
    // Schema is auto-registered per unique header set on first
    // invocation via `register_predeclared_any_schema`. Field types are
    // all string (csv records carry string-shaped cells); the schema's
    // `FieldType::Any` annotation is fine because the marshal-boundary
    // reader does its own kind validation when consumers downstream
    // read the slots.
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse_records",
        "Parse CSV text using the header row as keys, returning an array of typed records",
        "text",
        "string",
        ConcreteType::ArrayHeapValue("Array<object>".to_string()),
        |text, _ctx| {
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(true)
                .from_reader(text.as_bytes());

            let headers: Vec<String> = reader
                .headers()
                .map_err(|e| format!("csv.parse_records() failed to read headers: {}", e))?
                .iter()
                .map(|h| h.to_string())
                .collect();

            // Auto-register the schema for this header set. The registry
            // dedupes by field-name list; subsequent CSV files with the
            // same header columns reuse the same SchemaId.
            let schema_id = register_predeclared_any_schema(&headers);
            let field_kinds: Arc<[NativeKind]> = Arc::from(
                vec![NativeKind::String; headers.len()].into_boxed_slice(),
            );
            // Heap mask: every field is a string (heap-resident).
            let heap_mask: u64 = if headers.len() >= 64 {
                u64::MAX
            } else {
                (1u64 << headers.len()) - 1
            };

            let mut records: Vec<Arc<HeapValue>> = Vec::new();
            for result in reader.records() {
                let record =
                    result.map_err(|e| format!("csv.parse_records() failed: {}", e))?;
                let n = headers.len().min(record.len());
                let mut slots: Vec<ValueSlot> = Vec::with_capacity(headers.len());
                // Use min(headers, record) length plus pad with empty
                // string for short rows so the slot count matches the
                // schema (TypedObjectStorage::new enforces this).
                for i in 0..headers.len() {
                    let cell = if i < n {
                        record.get(i).unwrap_or("").to_string()
                    } else {
                        String::new()
                    };
                    slots.push(ValueSlot::from_string_arc(Arc::new(cell)));
                }
                // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): variant
                // signature flipped to `HeapValue::TypedObject(TypedObjectPtr)`.
                // `_new` returns `*mut TypedObjectStorage` with refcount=1; we
                // wrap it in `TypedObjectPtr` (transferring the share to the
                // wrapper).
                let storage = TypedObjectStorage::_new(
                    schema_id as u64,
                    slots.into_boxed_slice(),
                    heap_mask,
                    Arc::clone(&field_kinds),
                );
                records.push(Arc::new(HeapValue::TypedObject(
                    shape_value::heap_value::TypedObjectPtr::new(storage),
                )));
            }

            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayHeapValue(
                records,
            )))
        },
    );

    // csv.stringify_records(data: Array<HashMap<string, string>>, headers?: Array<string>) -> string
    //
    // Serializes an array of HashMap records to CSV. Header order is
    // either the explicit `headers` argument OR the keys from the first
    // record (using its HashMapData insertion order — same semantics as
    // the legacy `from_hashmap_pairs(keys, values)` shape).
    register_typed_fn_2_full::<_, Vec<Arc<HeapValue>>, Vec<Arc<String>>>(
        &mut module,
        "stringify_records",
        "Convert an array of hashmaps to a CSV string with headers",
        [
            ModuleParam {
                name: "data".to_string(),
                type_name: "Array<HashMap<string, string>>".to_string(),
                required: true,
                description: "Array of records (hashmaps with string keys and values)"
                    .to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "headers".to_string(),
                type_name: "Array<string>".to_string(),
                required: false,
                description: "Explicit header order (default: keys from first record)"
                    .to_string(),
                default_snippet: Some("[]".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        |data, explicit_headers, _ctx| {
            // W17-out-of-bundle-A-followups (2026-05-12): accept TypedObject
            // records in addition to legacy HashMap records. parse_records
            // now emits TypedObjects; round-trip via stringify_records must
            // therefore read the TypedObject shape. HashMap input remains
            // supported for users still passing legacy HashMap records.
            //
            // Determine header order: explicit argument (if non-empty) or
            // the first record's keys (TypedObject schema field-order, or
            // HashMap insertion order).
            let headers: Vec<String> = if !explicit_headers.is_empty() {
                explicit_headers.iter().map(|s| (**s).clone()).collect()
            } else if let Some(first) = data.first() {
                match &**first {
                    HeapValue::HashMap(kref) => {
                        // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14):
                        // per-V walk of `*mut TypedArray<*const StringObj>`
                        // keys. V-agnostic (keys are always string-typed).
                        let keys_ptr = match kref {
                            shape_value::heap_value::HashMapKindedRef::I64(arc) => arc.keys,
                            shape_value::heap_value::HashMapKindedRef::F64(arc) => arc.keys,
                            shape_value::heap_value::HashMapKindedRef::Bool(arc) => arc.keys,
                            shape_value::heap_value::HashMapKindedRef::Char(arc) => arc.keys,
                            shape_value::heap_value::HashMapKindedRef::String(arc) => arc.keys,
                            shape_value::heap_value::HashMapKindedRef::Decimal(arc) => arc.keys,
                            shape_value::heap_value::HashMapKindedRef::TypedObject(arc) => arc.keys,
                            shape_value::heap_value::HashMapKindedRef::TraitObject(arc) => arc.keys,
                            shape_value::heap_value::HashMapKindedRef::HashMap(arc) => arc.keys,
                        };
                        let n = unsafe { shape_value::v2::typed_array::TypedArray::len(keys_ptr) as usize };
                        (0..n)
                            .map(|i| unsafe {
                                let ptr = shape_value::v2::typed_array::TypedArray::get_unchecked(keys_ptr, i as u32);
                                shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned()
                            })
                            .collect()
                    }
                    HeapValue::TypedObject(s) => {
                        let schema = crate::type_schema::lookup_schema_by_id_public(
                            s.schema_id as u32,
                        )
                        .ok_or_else(|| {
                            format!(
                                "csv.stringify_records(): TypedObject schema id {} \
                                 not registered",
                                s.schema_id
                            )
                        })?;
                        schema.fields.iter().map(|f| f.name.clone()).collect()
                    }
                    other => {
                        return Err(format!(
                            "csv.stringify_records(): each element must be a record \
                             (HashMap or TypedObject), got {}",
                            other.type_name()
                        ));
                    }
                }
            } else {
                return Ok(TypedReturn::Concrete(ConcreteReturn::String(
                    String::new(),
                )));
            };

            let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());
            writer
                .write_record(&headers)
                .map_err(|e| format!("csv.stringify_records() header write failed: {}", e))?;

            for record_arc in data.iter() {
                let row: Vec<String> = match &**record_arc {
                    HeapValue::HashMap(kref) => {
                        // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14):
                        // per-V get(header) → cell extraction. CSV records
                        // are conventionally HashMap<string, string>
                        // (V=String); other V variants surface as a
                        // structured error.
                        use shape_value::heap_value::HashMapKindedRef;
                        match kref {
                            HashMapKindedRef::String(arc) => headers
                                .iter()
                                .map(|h| {
                                    arc.get_index(h.as_str())
                                        .map(|idx| {
                                            let ptr: *const shape_value::v2::string_obj::StringObj =
                                                unsafe { *(*arc.values).data.add(idx) };
                                            unsafe {
                                                shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned()
                                            }
                                        })
                                        .unwrap_or_default()
                                })
                                .collect(),
                            other => {
                                return Err(format!(
                                    "csv.stringify_records(): HashMap records must be \
                                     HashMap<string, string>, got V={:?}",
                                    other.values_kind()
                                ));
                            }
                        }
                    }
                    HeapValue::TypedObject(storage) => {
                        let schema = crate::type_schema::lookup_schema_by_id_public(
                            storage.schema_id as u32,
                        )
                        .ok_or_else(|| {
                            format!(
                                "csv.stringify_records(): TypedObject schema id {} \
                                 not registered",
                                storage.schema_id
                            )
                        })?;
                        let mut r = Vec::with_capacity(headers.len());
                        for header in &headers {
                            // Resolve header → slot index via the schema's
                            // field list. Empty cell when the record's
                            // schema doesn't have the requested header.
                            let cell = match schema
                                .fields
                                .iter()
                                .position(|f| f.name == *header)
                            {
                                Some(idx) if idx < storage.slots.len() => {
                                    // Slot is a string per parse_records'
                                    // construction; read via the kind table.
                                    let bits = storage.slots[idx].raw();
                                    if bits == 0 {
                                        String::new()
                                    } else {
                                        // SAFETY: parse_records writes each
                                        // slot via `ValueSlot::from_string_arc`
                                        // — slot bits = `Arc::into_raw::<String>`.
                                        // Borrow without releasing the storage's
                                        // share (which owns the Arc).
                                        unsafe {
                                            let arc_ptr = bits as *const String;
                                            Arc::increment_strong_count(arc_ptr);
                                            let arc = Arc::from_raw(arc_ptr);
                                            let owned = (*arc).clone();
                                            // arc Drop here releases our
                                            // bumped share; the storage's
                                            // share is untouched.
                                            owned
                                        }
                                    }
                                }
                                _ => String::new(),
                            };
                            r.push(cell);
                        }
                        r
                    }
                    other => {
                        return Err(format!(
                            "csv.stringify_records(): each element must be a record \
                             (HashMap or TypedObject), got {}",
                            other.type_name()
                        ));
                    }
                };
                writer
                    .write_record(&row)
                    .map_err(|e| format!("csv.stringify_records() row write failed: {}", e))?;
            }

            let bytes = writer
                .into_inner()
                .map_err(|e| format!("csv.stringify_records() flush failed: {}", e))?;
            let output = String::from_utf8(bytes)
                .map_err(|e| format!("csv.stringify_records() UTF-8 error: {}", e))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::String(output)))
        },
    );

    module
}
