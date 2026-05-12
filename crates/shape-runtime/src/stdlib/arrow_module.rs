//! Native `arrow` module for reading Arrow IPC files.
//!
//! Exports: arrow.read_table, arrow.read_tables, arrow.metadata
//!
//! All operations require `FsRead` permission.
//!
//! W17-out-of-bundle-A-followups (2026-05-12): `arrow.read_tables`
//! currently surfaces structured-stop. Per the C+ precedent in
//! `phase-2d-playbook.md` §3, `Array<DataTable>` is genuinely
//! homogeneous in `HeapKind::DataTable` — the natural Q25.A specialized
//! variant is a `TypedArrayData::DataTable(Arc<TypedBuffer<Arc<DataTable>>>)`
//! arm, which is out of scope for this sub-cluster (the prompt
//! explicitly forbids new HeapKind variants and an added
//! TypedArrayData variant would require ~40 exhaustive-match updates).
//! The surface message names the natural follow-up sub-cluster so
//! production callers see the structured error rather than a panic.
//!
//! Phase 2d Array cluster migration (historical context, 2026-05-07):
//! ported to the typed marshal layer. `arrow.read_tables` returned
//! `Array<DataTable>` via `ConcreteReturn::ArrayHeapValue` — each
//! element was an `Arc<HeapValue::DataTable>`. Post-Q25.A,
//! `build_specialized_from_heap_arcs` does not have a DataTable arm,
//! so the marshal projection surfaces a structured error.
//!
//! Tests deferred — ValueWord-based test fixtures can't compile and
//! aren't reconstructed until the shape-vm cascade provides a typed
//! test harness, mirroring the file_ops migration in commit d716482.

use crate::marshal::register_typed_fn_1;
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use arrow_ipc::reader::FileReader;
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
    //
    // W17-out-of-bundle-A-followups (2026-05-12): surface-and-stop. The
    // `Array<DataTable>` return shape is genuinely homogeneous in
    // `HeapKind::DataTable` — the natural Q25.A specialized variant is
    // `TypedArrayData::DataTable(Arc<TypedBuffer<Arc<DataTable>>>)`, but
    // adding a TypedArrayData variant is out of bundle-A-followups
    // scope (prompt forbids new HeapKind variants AND a new
    // TypedArrayData arm cascades through ~40 exhaustive matches).
    // Body returns a structured `Err` payload so callers see the
    // tracked follow-up rather than a marshal-layer panic.
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
        |path, _ctx| {
            let _ = path; // suppress unused; the SURFACE response is path-independent
            // phase-2d-hardening:(f) — arrow.read_tables surface-and-stop:
            // Array<DataTable> needs TypedArrayData::DataTable variant
            // (homogeneous-in-HeapKind::DataTable case per ADR-006 §2.7.24
            // Q25.A spec list). Tracked as
            // W17-typed-carrier-array-datatable follow-up.
            Err(format!(
                "arrow.read_tables(): SURFACE — `Array<DataTable>` needs a \
                 `TypedArrayData::DataTable` specialized variant in ADR-006 \
                 §2.7.24 Q25.A's spec list. Tracked as \
                 W17-typed-carrier-array-datatable follow-up \
                 (out of bundle-A-followups scope: new TypedArrayData arm \
                 cascades through exhaustive matches across ~40 files). \
                 ADR-006 §2.7.24 Q25.A."
            ))
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
