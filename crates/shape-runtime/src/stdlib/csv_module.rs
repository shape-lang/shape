//! Native `csv` module for CSV parsing and serialization.
//!
//! Phase 2d Array cluster migration: `parse`, `stringify`, `read_file`,
//! and `is_valid` ported to the typed marshal layer using
//! `TypedArrayData::String` (rows of strings) inside
//! `TypedArrayData::HeapValue` (array of rows). `parse_records` and
//! `stringify_records` remain deferred pending the HashMap-marshal
//! micro-cluster (no `HeapValue::HashMap` variant yet — a separate
//! architectural decision tracked in `docs/defections.md`).
//!
//! Tests deferred — ValueWord-based test fixtures can't compile and
//! aren't reconstructed until the shape-vm cascade provides a typed
//! test harness, mirroring the file_ops migration in commit d716482.

use crate::marshal::{register_typed_fn_1, register_typed_fn_2_full};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::{HeapValue, TypedArrayData};
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

    // Deferred: csv.parse_records, csv.stringify_records.
    //
    // Both functions return / consume `Array<HashMap<string, string>>`.
    // `HeapValue` has no `HashMap` variant in the strict-typed runtime;
    // adding one is its own architectural decision (HashMap-marshal
    // micro-cluster). Registration is held until that micro-cluster
    // lands. Mirrors the deferral pattern from process_ops Array<string>
    // (resolved by Phase 2d Array cluster) — same shape, different
    // sub-decision.

    module
}
