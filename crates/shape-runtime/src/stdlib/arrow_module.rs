//! Native `arrow` module for reading Arrow IPC files.
//!
//! Exports: arrow.read_table, arrow.read_tables, arrow.metadata
//!
//! All operations require `FsRead` permission.
//!
//! Phase 2d Array cluster migration: ported to the typed marshal layer.
//! `arrow.read_tables` returns `Array<DataTable>` via
//! `ConcreteReturn::ArrayHeapValue` — each element is an
//! `Arc<HeapValue::DataTable(...))>` per the option ε body-side
//! type contract. See `docs/defections.md` Phase 2d Array cluster
//! entry.
//!
//! Tests deferred — ValueWord-based test fixtures can't compile and
//! aren't reconstructed until the shape-vm cascade provides a typed
//! test harness, mirroring the file_ops migration in commit d716482.

use crate::marshal::register_typed_fn_1;
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use arrow_ipc::reader::FileReader;
use shape_value::datatable::DataTable;
use shape_value::heap_value::HeapValue;
use std::io::Cursor;
use std::sync::Arc;

/// Create the `arrow` module with Arrow IPC file reading functions.
pub fn create_arrow_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::arrow");
    module.description = "Arrow IPC columnar file reading".to_string();

    // arrow.read_table(path: string) -> Result<DataTable, string>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "read_table",
        "Read the first record batch from an Arrow IPC file",
        "path",
        "string",
        ConcreteType::Result2(
            Box::new(ConcreteType::DataTable),
            Box::new(ConcreteType::String),
        ),
        |path, ctx| {
            crate::module_exports::check_fs_permission(
                ctx,
                shape_abi_v1::Permission::FsRead,
                path.as_str(),
            )?;

            let bytes = std::fs::read(path.as_str())
                .map_err(|e| format!("arrow.read_table() failed to read '{}': {}", path, e))?;

            let dt = crate::wire_conversion::datatable_from_ipc_bytes(&bytes, None, None)?;
            Ok(TypedReturn::Ok(ConcreteReturn::DataTable(Arc::new(dt))))
        },
    );

    // arrow.read_tables(path: string) -> Result<Array<DataTable>, string>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "read_tables",
        "Read all record batches from an Arrow IPC file",
        "path",
        "string",
        ConcreteType::Result2(
            Box::new(ConcreteType::ArrayHeapValue("Array<DataTable>".to_string())),
            Box::new(ConcreteType::String),
        ),
        |path, ctx| {
            crate::module_exports::check_fs_permission(
                ctx,
                shape_abi_v1::Permission::FsRead,
                path.as_str(),
            )?;

            let bytes = std::fs::read(path.as_str())
                .map_err(|e| format!("arrow.read_tables() failed to read '{}': {}", path, e))?;

            let cursor = Cursor::new(bytes);
            let reader = FileReader::try_new(cursor, None)
                .map_err(|e| format!("arrow.read_tables() invalid IPC file: {}", e))?;

            let mut tables: Vec<Arc<HeapValue>> = Vec::new();
            for batch_result in reader {
                let batch = batch_result
                    .map_err(|e| format!("arrow.read_tables() failed reading batch: {}", e))?;
                let dt = DataTable::new(batch);
                tables.push(Arc::new(HeapValue::DataTable(Arc::new(dt))));
            }

            Ok(TypedReturn::Ok(ConcreteReturn::ArrayHeapValue(tables)))
        },
    );

    // arrow.metadata(path: string) -> Result<HashMap<string, string>, string>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "metadata",
        "Read schema metadata from an Arrow IPC file header",
        "path",
        "string",
        ConcreteType::Result2(
            Box::new(ConcreteType::HashMapStringString),
            Box::new(ConcreteType::String),
        ),
        |path, ctx| {
            crate::module_exports::check_fs_permission(
                ctx,
                shape_abi_v1::Permission::FsRead,
                path.as_str(),
            )?;

            let bytes = std::fs::read(path.as_str())
                .map_err(|e| format!("arrow.metadata() failed to read '{}': {}", path, e))?;

            let cursor = Cursor::new(bytes);
            let reader = FileReader::try_new(cursor, None)
                .map_err(|e| format!("arrow.metadata() invalid IPC file: {}", e))?;

            let schema = reader.schema();
            let meta = schema.metadata();

            let pairs: Vec<(String, String)> = meta
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Ok(TypedReturn::Ok(ConcreteReturn::HashMapStringString(pairs)))
        },
    );

    module
}
