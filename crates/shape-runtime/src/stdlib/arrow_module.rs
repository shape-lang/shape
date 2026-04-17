//! Native `arrow` module for reading Arrow IPC files.
//!
//! Exports: arrow.read_table, arrow.read_tables, arrow.metadata
//!
//! All operations require `FsRead` permission.

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use arrow_ipc::reader::FileReader;
use shape_value::datatable::DataTable;
use shape_value::{ValueWord, ValueWordExt};
use std::io::Cursor;
use std::sync::Arc;

/// Create the `arrow` module with Arrow IPC file reading functions.
pub fn create_arrow_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::arrow");
    module.description = "Arrow IPC columnar file reading".to_string();

    // arrow.read_table(path: string) -> Result<DataTable, string>
    module.add_function_with_schema(
        "read_table",
        |args: &[ValueWord], ctx: &ModuleContext| {
            let path = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "arrow.read_table() requires a path string".to_string())?;

            crate::module_exports::check_fs_permission(
                ctx,
                shape_abi_v1::Permission::FsRead,
                path,
            )?;

            let bytes = std::fs::read(path)
                .map_err(|e| format!("arrow.read_table() failed to read '{}': {}", path, e))?;

            let dt = crate::wire_conversion::datatable_from_ipc_bytes(&bytes, None, None)?;
            Ok(ValueWord::from_ok(ValueWord::from_datatable(Arc::new(dt))))
        },
        ModuleFunction {
            description: "Read the first record batch from an Arrow IPC file".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to the Arrow IPC file".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<DataTable, string>".to_string()),
        },
    );

    // arrow.read_tables(path: string) -> Result<Array<DataTable>, string>
    module.add_function_with_schema(
        "read_tables",
        |args: &[ValueWord], ctx: &ModuleContext| {
            let path = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "arrow.read_tables() requires a path string".to_string())?;

            crate::module_exports::check_fs_permission(
                ctx,
                shape_abi_v1::Permission::FsRead,
                path,
            )?;

            let bytes = std::fs::read(path)
                .map_err(|e| format!("arrow.read_tables() failed to read '{}': {}", path, e))?;

            let cursor = Cursor::new(bytes);
            let reader = FileReader::try_new(cursor, None)
                .map_err(|e| format!("arrow.read_tables() invalid IPC file: {}", e))?;

            let mut tables: Vec<ValueWord> = Vec::new();
            for batch_result in reader {
                let batch = batch_result
                    .map_err(|e| format!("arrow.read_tables() failed reading batch: {}", e))?;
                let dt = DataTable::new(batch);
                tables.push(ValueWord::from_datatable(Arc::new(dt)));
            }

            Ok(ValueWord::from_ok(ValueWord::from_array(shape_value::vmarray_from_vec(tables))))
        },
        ModuleFunction {
            description: "Read all record batches from an Arrow IPC file".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to the Arrow IPC file".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<Array<DataTable>, string>".to_string()),
        },
    );

    // arrow.metadata(path: string) -> Result<HashMap<string, string>, string>
    module.add_function_with_schema(
        "metadata",
        |args: &[ValueWord], ctx: &ModuleContext| {
            let path = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "arrow.metadata() requires a path string".to_string())?;

            crate::module_exports::check_fs_permission(
                ctx,
                shape_abi_v1::Permission::FsRead,
                path,
            )?;

            let bytes = std::fs::read(path)
                .map_err(|e| format!("arrow.metadata() failed to read '{}': {}", path, e))?;

            let cursor = Cursor::new(bytes);
            let reader = FileReader::try_new(cursor, None)
                .map_err(|e| format!("arrow.metadata() invalid IPC file: {}", e))?;

            let schema = reader.schema();
            let meta = schema.metadata();

            let keys: Vec<ValueWord> = meta
                .keys()
                .map(|k| ValueWord::from_string(Arc::new(k.clone())))
                .collect();
            let values: Vec<ValueWord> = meta
                .values()
                .map(|v| ValueWord::from_string(Arc::new(v.clone())))
                .collect();

            Ok(ValueWord::from_ok(ValueWord::from_hashmap_pairs(
                keys, values,
            )))
        },
        ModuleFunction {
            description: "Read schema metadata from an Arrow IPC file header".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to the Arrow IPC file".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<HashMap<string, string>, string>".to_string()),
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Float64Array, Int64Array, RecordBatch};
    use arrow_ipc::writer::FileWriter;
    use arrow_schema::{Field, Schema};
    use std::collections::HashMap;

    fn test_ctx() -> crate::module_exports::ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        crate::module_exports::ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        }
    }

    fn write_test_arrow_file(path: &std::path::Path) {
        let mut metadata = HashMap::new();
        metadata.insert("test_key".to_string(), "test_value".to_string());
        metadata.insert("rows".to_string(), "3".to_string());

        let schema = Arc::new(
            Schema::new(vec![
                Field::new("x", arrow_schema::DataType::Float64, false),
                Field::new("y", arrow_schema::DataType::Int64, false),
            ])
            .with_metadata(metadata),
        );

        let x_col = Float64Array::from(vec![1.0, 2.0, 3.0]);
        let y_col = Int64Array::from(vec![10, 20, 30]);
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(x_col), Arc::new(y_col)]).unwrap();

        let file = std::fs::File::create(path).unwrap();
        let mut writer = FileWriter::try_new(file, &schema).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn test_arrow_module_creation() {
        let module = create_arrow_module();
        assert_eq!(module.name, "std::core::arrow");
        assert!(module.has_export("read_table"));
        assert!(module.has_export("read_tables"));
        assert!(module.has_export("metadata"));
    }

    #[test]
    fn test_arrow_read_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.arrow");
        write_test_arrow_file(&path);

        let module = create_arrow_module();
        let read_fn = module.get_export("read_table").unwrap();
        let ctx = test_ctx();
        let result = read_fn(
            &[ValueWord::from_string(Arc::new(
                path.to_str().unwrap().to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert!(inner.as_datatable().is_some());
    }

    #[test]
    fn test_arrow_read_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.arrow");
        write_test_arrow_file(&path);

        let module = create_arrow_module();
        let read_fn = module.get_export("read_tables").unwrap();
        let ctx = test_ctx();
        let result = read_fn(
            &[ValueWord::from_string(Arc::new(
                path.to_str().unwrap().to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let tables = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(tables.len(), 1);
    }

    #[test]
    fn test_arrow_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.arrow");
        write_test_arrow_file(&path);

        let module = create_arrow_module();
        let meta_fn = module.get_export("metadata").unwrap();
        let ctx = test_ctx();
        let result = meta_fn(
            &[ValueWord::from_string(Arc::new(
                path.to_str().unwrap().to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, values, _) = inner.as_hashmap().expect("should be hashmap");
        // Find the test_key
        let mut found = false;
        for (i, k) in keys.iter().enumerate() {
            if k.as_str() == Some("test_key") {
                assert_eq!(values[i].as_str(), Some("test_value"));
                found = true;
            }
        }
        assert!(found, "should have 'test_key' in metadata");
    }

    #[test]
    fn test_arrow_read_table_nonexistent() {
        let module = create_arrow_module();
        let read_fn = module.get_export("read_table").unwrap();
        let ctx = test_ctx();
        let result = read_fn(
            &[ValueWord::from_string(Arc::new(
                "/nonexistent/file.arrow".to_string(),
            ))],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_arrow_read_table_requires_string() {
        let module = create_arrow_module();
        let read_fn = module.get_export("read_table").unwrap();
        let ctx = test_ctx();
        let result = read_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_arrow_schemas() {
        let module = create_arrow_module();

        let read_table_schema = module.get_schema("read_table").unwrap();
        assert_eq!(read_table_schema.params.len(), 1);
        assert_eq!(read_table_schema.params[0].name, "path");
        assert!(read_table_schema.params[0].required);
        assert_eq!(
            read_table_schema.return_type.as_deref(),
            Some("Result<DataTable, string>")
        );

        let read_tables_schema = module.get_schema("read_tables").unwrap();
        assert_eq!(read_tables_schema.params.len(), 1);
        assert_eq!(
            read_tables_schema.return_type.as_deref(),
            Some("Result<Array<DataTable>, string>")
        );

        let metadata_schema = module.get_schema("metadata").unwrap();
        assert_eq!(metadata_schema.params.len(), 1);
        assert_eq!(
            metadata_schema.return_type.as_deref(),
            Some("Result<HashMap<string, string>, string>")
        );
    }
}
