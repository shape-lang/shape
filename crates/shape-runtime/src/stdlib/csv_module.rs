//! Native `csv` module for CSV parsing and serialization.
//!
//! Phase 2d Array cluster migration: `parse`, `stringify`, `read_file`,
//! and `is_valid` ported to the typed marshal layer using
//! `TypedArrayData::String` (rows of strings) inside
//! `TypedArrayData::HeapValue` (array of rows).
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
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::{HashMapData, HeapValue, TypedArrayData};
use shape_value::TypedBuffer;
use std::sync::Arc;

/// Build a `HeapValue::TypedArray(TypedArrayData::String(...))` from a
/// `Vec<String>`. Each element is wrapped into `Arc<String>` for the
/// typed buffer's element-storage shape.
fn row_to_heap(row: Vec<String>) -> Arc<HeapValue> {
    let strings: Vec<Arc<String>> = row.into_iter().map(Arc::new).collect();
    Arc::new(HeapValue::TypedArray(TypedArrayData::String(Arc::new(
        TypedBuffer::from_vec(strings),
    ))))
}

/// Read a `Vec<Vec<String>>` from a `Vec<Arc<HeapValue>>` whose elements
/// are each `HeapValue::TypedArray(TypedArrayData::String(...))`. Used by
/// `csv.stringify` which takes `Array<Array<string>>` as input.
fn rows_from_heap_array(
    rows: &[Arc<HeapValue>],
    fn_name: &str,
) -> Result<Vec<Vec<String>>, String> {
    rows.iter()
        .map(|row_arc| match &**row_arc {
            HeapValue::TypedArray(TypedArrayData::String(buf)) => {
                Ok(buf.data.iter().map(|s| (**s).clone()).collect())
            }
            other => Err(format!(
                "{}: each row must be Array<string>, got {}",
                fn_name,
                other.type_name()
            )),
        })
        .collect()
}

/// Create the `csv` module with CSV parsing and serialization functions.
pub fn create_csv_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::csv");
    module.description = "CSV parsing and serialization".to_string();

    // csv.parse(text: string) -> Array<Array<string>>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse",
        "Parse CSV text into an array of rows (each row is an array of strings)",
        "text",
        "string",
        ConcreteType::ArrayHeapValue("Array<Array<string>>".to_string()),
        |text, _ctx| {
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(text.as_bytes());

            let mut rows: Vec<Arc<HeapValue>> = Vec::new();
            for result in reader.records() {
                let record = result.map_err(|e| format!("csv.parse() failed: {}", e))?;
                let row: Vec<String> = record.iter().map(|f| f.to_string()).collect();
                rows.push(row_to_heap(row));
            }

            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayHeapValue(rows)))
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
            let text = std::fs::read_to_string(path.as_str())
                .map_err(|e| format!("csv.read_file() failed to read '{}': {}", path, e))?;

            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(text.as_bytes());

            let mut rows: Vec<Arc<HeapValue>> = Vec::new();
            for result in reader.records() {
                let record = result.map_err(|e| format!("csv.read_file() parse error: {}", e))?;
                let row: Vec<String> = record.iter().map(|f| f.to_string()).collect();
                rows.push(row_to_heap(row));
            }

            Ok(TypedReturn::Ok(ConcreteReturn::ArrayHeapValue(rows)))
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

    // csv.parse_records(text: string) -> Array<HashMap<string, string>>
    //
    // Parses CSV text using the first row as header keys; each subsequent
    // row becomes a HashMap mapping header → field value. Insertion-order-
    // preserved per HashMapData semantics (column order = header order).
    //
    // Stage C HashMap-marshal P1(b) activation: returns
    // `ConcreteReturn::ArrayHeapValue` of `Arc<HeapValue::HashMap>`. Each
    // record's HashMap is built via `HashMapData::from_pairs(keys, values)`
    // with eager bucket-index for O(1) `record.get(header)` lookup at
    // user-API time.
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse_records",
        "Parse CSV text using the header row as keys, returning an array of hashmaps",
        "text",
        "string",
        ConcreteType::ArrayHeapValue("Array<HashMap<string, string>>".to_string()),
        |text, _ctx| {
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(true)
                .from_reader(text.as_bytes());

            let headers: Vec<Arc<String>> = reader
                .headers()
                .map_err(|e| format!("csv.parse_records() failed to read headers: {}", e))?
                .iter()
                .map(|h| Arc::new(h.to_string()))
                .collect();

            let mut records: Vec<Arc<HeapValue>> = Vec::new();
            for result in reader.records() {
                let record =
                    result.map_err(|e| format!("csv.parse_records() failed: {}", e))?;
                let n = headers.len().min(record.len());
                let keys: Vec<Arc<String>> = headers.iter().take(n).cloned().collect();
                let values: Vec<Arc<HeapValue>> = record
                    .iter()
                    .take(n)
                    .map(|f| Arc::new(HeapValue::String(Arc::new(f.to_string()))))
                    .collect();
                records.push(Arc::new(HeapValue::HashMap(Arc::new(
                    HashMapData::from_pairs(keys, values),
                ))));
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
            // Determine header order: explicit argument (if non-empty) or
            // the first record's keys (insertion order).
            let headers: Vec<String> = if !explicit_headers.is_empty() {
                explicit_headers.iter().map(|s| (**s).clone()).collect()
            } else if let Some(first) = data.first() {
                if let HeapValue::HashMap(d) = &**first {
                    d.keys.data.iter().map(|s| (**s).clone()).collect()
                } else {
                    return Err(format!(
                        "csv.stringify_records(): each element must be a HashMap, got {}",
                        first.type_name()
                    ));
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
                let d = match &**record_arc {
                    HeapValue::HashMap(d) => d,
                    other => {
                        return Err(format!(
                            "csv.stringify_records(): each element must be a HashMap, got {}",
                            other.type_name()
                        ));
                    }
                };
                let mut row = Vec::with_capacity(headers.len());
                for header in &headers {
                    // O(1) lookup via eager bucket-index (per Step 1 P1(b)
                    // refinement: `HashMapData::get` uses the index when
                    // present, falls back to nothing when key missing).
                    let cell = match d.get(header) {
                        Some(v) => match &**v {
                            HeapValue::String(s) => (**s).clone(),
                            other => other.to_string(),
                        },
                        None => String::new(),
                    };
                    row.push(cell);
                }
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
