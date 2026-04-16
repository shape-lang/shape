//! Schema inference for data files.
//!
//! Reads just the schema (column names + types) from CSV, JSON, and Parquet files
//! without loading the full data. Used for compile-time schema validation.

pub mod lockfile;

use arrow_schema::Schema as ArrowSchema;
use shape_value::ValueWordExt;
use std::path::Path;

/// Error type for schema inference operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SchemaInferError {
    /// File does not exist or cannot be opened.
    #[error("File not found: {0}")]
    FileNotFound(String),
    /// File extension is not supported (.csv, .json, .ndjson, .parquet).
    #[error("Unsupported file format: '{0}'. Supported: .csv, .json, .ndjson, .parquet")]
    UnsupportedFormat(String),
    /// Failed to parse file header or infer schema.
    #[error("Schema inference failed: {0}")]
    ParseError(String),
}

/// Infer the Arrow schema from a data file by extension.
///
/// Dispatches to the appropriate reader based on file extension:
/// - `.csv` → CSV header + sample inference
/// - `.json` / `.ndjson` → JSON schema inference
/// - `.parquet` → Parquet footer metadata
///
/// Only reads the minimum data needed (header/sample rows/footer), not the full file.
pub fn infer_schema(path: &Path) -> Result<ArrowSchema, SchemaInferError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "csv" => infer_csv_schema(path),
        "json" | "ndjson" => infer_json_schema(path),
        "parquet" => infer_parquet_schema(path),
        other => Err(SchemaInferError::UnsupportedFormat(other.to_string())),
    }
}

/// Infer schema from a CSV file using header + sample rows.
pub fn infer_csv_schema(path: &Path) -> Result<ArrowSchema, SchemaInferError> {
    use arrow_csv::reader::Format;
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SchemaInferError::FileNotFound(path.display().to_string())
        } else {
            SchemaInferError::ParseError(format!("Cannot open '{}': {}", path.display(), e))
        }
    })?;

    let format = Format::default().with_header(true);
    let (schema, _records_read) = format
        .infer_schema(BufReader::new(&file), Some(100))
        .map_err(|e| SchemaInferError::ParseError(format!("CSV schema inference: {}", e)))?;

    Ok(schema)
}

/// Infer schema from a JSON/NDJSON file using sample rows.
pub fn infer_json_schema(path: &Path) -> Result<ArrowSchema, SchemaInferError> {
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SchemaInferError::FileNotFound(path.display().to_string())
        } else {
            SchemaInferError::ParseError(format!("Cannot open '{}': {}", path.display(), e))
        }
    })?;

    let (schema, _records_read) =
        arrow_json::reader::infer_json_schema(BufReader::new(file), Some(100))
            .map_err(|e| SchemaInferError::ParseError(format!("JSON schema inference: {}", e)))?;

    Ok(schema)
}

/// Infer schema from a Parquet file by reading only the footer metadata.
pub fn infer_parquet_schema(path: &Path) -> Result<ArrowSchema, SchemaInferError> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use std::fs::File;

    let file = File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SchemaInferError::FileNotFound(path.display().to_string())
        } else {
            SchemaInferError::ParseError(format!("Cannot open '{}': {}", path.display(), e))
        }
    })?;

    let reader = SerializedFileReader::new(file)
        .map_err(|e| SchemaInferError::ParseError(format!("Parquet reader: {}", e)))?;

    let parquet_schema = reader.metadata().file_metadata().schema_descr_ptr();
    let arrow_schema = parquet::arrow::parquet_to_arrow_schema(
        &parquet_schema,
        None, // no key-value metadata filter
    )
    .map_err(|e| SchemaInferError::ParseError(format!("Parquet→Arrow schema: {}", e)))?;

    Ok(arrow_schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;

    fn temp_csv(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_infer_csv_schema() {
        let path = temp_csv(
            "test_infer_csv.csv",
            "name,value,active\nalpha,1.5,true\nbeta,2.7,false\n",
        );
        let schema = infer_schema(&path).unwrap();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec!["name", "value", "active"]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_infer_json_schema() {
        let path = temp_csv(
            "test_infer_json.ndjson",
            r#"{"name":"alpha","value":1.5}
{"name":"beta","value":2.7}
"#,
        );
        let schema = infer_schema(&path).unwrap();
        let mut names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["name", "value"]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_infer_parquet_schema() {
        use arrow_array::{Float64Array, RecordBatch, StringArray};
        use arrow_schema::{DataType, Field, Schema};

        // Create a small Parquet file
        let schema = Arc::new(Schema::new(vec![
            Field::new("symbol", DataType::Utf8, false),
            Field::new("price", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["AAPL", "GOOG"])),
                Arc::new(Float64Array::from(vec![150.0, 2800.0])),
            ],
        )
        .unwrap();

        let path = std::env::temp_dir().join("test_infer_parquet.parquet");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer =
            parquet::arrow::arrow_writer::ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let inferred = infer_schema(&path).unwrap();
        let names: Vec<&str> = inferred
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        assert_eq!(names, vec!["symbol", "price"]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_unsupported_extension() {
        let path = std::env::temp_dir().join("test_unsupported.xlsx");
        std::fs::File::create(&path).unwrap();
        let err = infer_schema(&path).unwrap_err();
        assert!(matches!(err, SchemaInferError::UnsupportedFormat(_)));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_missing_file() {
        let path = Path::new("/nonexistent/file.csv");
        let err = infer_schema(path).unwrap_err();
        assert!(matches!(err, SchemaInferError::FileNotFound(_)));
    }
}
