//! Schema cache helpers backed by generic `shape.lock` artifacts.

use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

use crate::package_lock::{ArtifactDeterminism, LockedArtifact, PackageLock};

/// Unified schema lock type backed by `shape.lock`.
pub type SchemaLockfile = PackageLock;

const SCHEMA_ARTIFACT_NAMESPACE: &str = "schema.infer";
const SCHEMA_ARTIFACT_PRODUCER: &str = "shape-runtime/schema_inference@v1";

/// Infer a schema, using the unified lockfile cache when possible.
///
/// Returns `(schema, from_cache)` — `from_cache` is true if the lockfile had
/// a valid entry for the current external file fingerprint.
pub fn infer_or_cached(
    file_path: &Path,
    source_key: &str,
    lockfile: &mut SchemaLockfile,
) -> Result<(ArrowSchema, bool), super::SchemaInferError> {
    let format = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let file_hash = compute_file_hash(file_path).unwrap_or_else(|_| "unknown".to_string());

    let (inputs, determinism) = schema_artifact_inputs(source_key, &format, &file_hash);
    let inputs_hash = PackageLock::artifact_inputs_hash(inputs.clone(), &determinism)
        .map_err(super::SchemaInferError::ParseError)?;

    if let Some(artifact) = lockfile.artifact(SCHEMA_ARTIFACT_NAMESPACE, source_key, &inputs_hash) {
        if let Ok(schema) = artifact_to_arrow_schema(artifact) {
            return Ok((schema, true));
        }
    }

    let schema = super::infer_schema(file_path)?;
    let payload = schema_to_payload(&format, &schema);
    let artifact = LockedArtifact::new(
        SCHEMA_ARTIFACT_NAMESPACE,
        source_key,
        SCHEMA_ARTIFACT_PRODUCER,
        determinism,
        inputs,
        payload,
    )
    .map_err(super::SchemaInferError::ParseError)?;
    lockfile
        .upsert_artifact(artifact)
        .map_err(super::SchemaInferError::ParseError)?;

    Ok((schema, false))
}

/// Compute SHA-256 hash of the first 4KB of a file.
pub fn compute_file_hash(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 4096];
    let bytes_read = file.read(&mut buffer)?;
    buffer.truncate(bytes_read);

    let mut hasher = Sha256::new();
    hasher.update(&buffer);
    let result = hasher.finalize();
    Ok(format!("sha256:{:x}", result))
}

fn schema_artifact_inputs(
    source_key: &str,
    format: &str,
    file_hash: &str,
) -> (BTreeMap<String, String>, ArtifactDeterminism) {
    let mut inputs = BTreeMap::new();
    inputs.insert("source".to_string(), source_key.to_string());
    inputs.insert("format".to_string(), format.to_string());
    inputs.insert("file_hash".to_string(), file_hash.to_string());

    let determinism = ArtifactDeterminism::External {
        fingerprints: BTreeMap::from([(format!("file:{source_key}"), file_hash.to_string())]),
    };
    (inputs, determinism)
}

fn artifact_to_arrow_schema(artifact: &LockedArtifact) -> Result<ArrowSchema, String> {
    let payload = artifact.payload()?;
    payload_to_schema(&payload)
}

fn schema_to_payload(format: &str, schema: &ArrowSchema) -> shape_wire::WireValue {
    let columns = schema
        .fields()
        .iter()
        .map(|field| {
            shape_wire::WireValue::Object(BTreeMap::from([
                (
                    "name".to_string(),
                    shape_wire::WireValue::String(field.name().clone()),
                ),
                (
                    "data_type".to_string(),
                    shape_wire::WireValue::String(format_data_type(field.data_type())),
                ),
                (
                    "nullable".to_string(),
                    shape_wire::WireValue::Bool(field.is_nullable()),
                ),
            ]))
        })
        .collect::<Vec<_>>();

    shape_wire::WireValue::Object(BTreeMap::from([
        (
            "format".to_string(),
            shape_wire::WireValue::String(format.to_string()),
        ),
        ("columns".to_string(), shape_wire::WireValue::Array(columns)),
    ]))
}

fn payload_to_schema(payload: &shape_wire::WireValue) -> Result<ArrowSchema, String> {
    let shape_wire::WireValue::Object(map) = payload else {
        return Err("schema artifact payload is not an object".to_string());
    };
    let columns = map
        .get("columns")
        .ok_or_else(|| "schema artifact payload missing columns".to_string())?;
    let shape_wire::WireValue::Array(column_values) = columns else {
        return Err("schema artifact payload columns must be an array".to_string());
    };

    let mut fields = Vec::with_capacity(column_values.len());
    for column in column_values {
        let shape_wire::WireValue::Object(col) = column else {
            return Err("schema artifact column must be an object".to_string());
        };
        let name = col
            .get("name")
            .and_then(shape_wire::WireValue::as_str)
            .ok_or_else(|| "schema artifact column missing name".to_string())?;
        let data_type = col
            .get("data_type")
            .and_then(shape_wire::WireValue::as_str)
            .ok_or_else(|| "schema artifact column missing data_type".to_string())?;
        let nullable = col
            .get("nullable")
            .and_then(shape_wire::WireValue::as_bool)
            .ok_or_else(|| "schema artifact column missing nullable".to_string())?;

        fields.push(Field::new(name, parse_data_type(data_type), nullable));
    }

    Ok(ArrowSchema::new(fields))
}

/// Format an Arrow DataType as a string for lockfile storage.
fn format_data_type(dt: &DataType) -> String {
    match dt {
        DataType::Float64 => "Float64".to_string(),
        DataType::Float32 => "Float32".to_string(),
        DataType::Int64 => "Int64".to_string(),
        DataType::Int32 => "Int32".to_string(),
        DataType::Int16 => "Int16".to_string(),
        DataType::Int8 => "Int8".to_string(),
        DataType::UInt64 => "UInt64".to_string(),
        DataType::UInt32 => "UInt32".to_string(),
        DataType::Boolean => "Boolean".to_string(),
        DataType::Utf8 => "Utf8".to_string(),
        DataType::LargeUtf8 => "LargeUtf8".to_string(),
        DataType::Timestamp(unit, tz) => {
            let unit_str = match unit {
                arrow_schema::TimeUnit::Second => "s",
                arrow_schema::TimeUnit::Millisecond => "ms",
                arrow_schema::TimeUnit::Microsecond => "us",
                arrow_schema::TimeUnit::Nanosecond => "ns",
            };
            match tz {
                Some(tz) => format!("Timestamp({},{})", unit_str, tz),
                None => format!("Timestamp({})", unit_str),
            }
        }
        DataType::Date32 => "Date32".to_string(),
        DataType::Date64 => "Date64".to_string(),
        other => format!("{other:?}"),
    }
}

/// Parse a data type string from the lockfile back into an Arrow DataType.
fn parse_data_type(s: &str) -> DataType {
    match s {
        "Float64" => DataType::Float64,
        "Float32" => DataType::Float32,
        "Int64" => DataType::Int64,
        "Int32" => DataType::Int32,
        "Int16" => DataType::Int16,
        "Int8" => DataType::Int8,
        "UInt64" => DataType::UInt64,
        "UInt32" => DataType::UInt32,
        "Boolean" => DataType::Boolean,
        "Utf8" => DataType::Utf8,
        "LargeUtf8" => DataType::LargeUtf8,
        "Date32" => DataType::Date32,
        "Date64" => DataType::Date64,
        s if s.starts_with("Timestamp(") => {
            let inner = &s[10..s.len() - 1];
            let parts: Vec<&str> = inner.splitn(2, ',').collect();
            let unit = match parts[0] {
                "s" => arrow_schema::TimeUnit::Second,
                "ms" => arrow_schema::TimeUnit::Millisecond,
                "us" => arrow_schema::TimeUnit::Microsecond,
                "ns" => arrow_schema::TimeUnit::Nanosecond,
                _ => arrow_schema::TimeUnit::Nanosecond,
            };
            let tz = parts.get(1).map(|value| value.to_string().into());
            DataType::Timestamp(unit, tz)
        }
        _ => DataType::Utf8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_lockfile_roundtrip() {
        let mut lockfile = SchemaLockfile::new();
        let payload = shape_wire::WireValue::Object(BTreeMap::from([(
            "columns".to_string(),
            shape_wire::WireValue::Array(vec![shape_wire::WireValue::Object(BTreeMap::from([
                (
                    "name".to_string(),
                    shape_wire::WireValue::String("price".to_string()),
                ),
                (
                    "data_type".to_string(),
                    shape_wire::WireValue::String("Float64".to_string()),
                ),
                ("nullable".to_string(), shape_wire::WireValue::Bool(false)),
            ]))]),
        )]));
        let artifact = LockedArtifact::new(
            SCHEMA_ARTIFACT_NAMESPACE,
            "data.csv",
            SCHEMA_ARTIFACT_PRODUCER,
            ArtifactDeterminism::External {
                fingerprints: BTreeMap::from([(
                    "file:data.csv".to_string(),
                    "sha256:deadbeef".to_string(),
                )]),
            },
            BTreeMap::new(),
            payload,
        )
        .unwrap();
        lockfile.upsert_artifact(artifact).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shape.lock");
        lockfile.write(&path).unwrap();

        let loaded = SchemaLockfile::read(&path).unwrap();
        assert_eq!(loaded.artifacts.len(), 1);
    }

    #[test]
    fn test_compute_file_hash_changes_on_content_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.csv");

        std::fs::write(&path, "a,b\n1,2\n").unwrap();
        let h1 = compute_file_hash(&path).unwrap();

        std::fs::write(&path, "a,b\n1,2\n3,4\n").unwrap();
        let h2 = compute_file_hash(&path).unwrap();

        assert_ne!(h1, h2);
        assert!(h1.starts_with("sha256:"));
        assert!(h2.starts_with("sha256:"));
    }

    #[test]
    fn test_infer_or_cached() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("cached_test.csv");

        let mut file = std::fs::File::create(&csv_path).unwrap();
        writeln!(file, "x,y").unwrap();
        writeln!(file, "1,2").unwrap();
        writeln!(file, "3,4").unwrap();
        drop(file);

        let mut lockfile = SchemaLockfile::new();

        let (_schema1, from_cache1) =
            infer_or_cached(&csv_path, "cached_test.csv", &mut lockfile).unwrap();
        assert!(!from_cache1);

        let (_schema2, from_cache2) =
            infer_or_cached(&csv_path, "cached_test.csv", &mut lockfile).unwrap();
        assert!(from_cache2);

        let mut file = std::fs::File::create(&csv_path).unwrap();
        writeln!(file, "x,y").unwrap();
        writeln!(file, "1,2").unwrap();
        writeln!(file, "3,4").unwrap();
        writeln!(file, "5,6").unwrap();
        drop(file);

        let (_schema3, from_cache3) =
            infer_or_cached(&csv_path, "cached_test.csv", &mut lockfile).unwrap();
        assert!(!from_cache3);
    }
}
