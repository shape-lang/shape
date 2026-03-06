//! Data-source schema cache backed by unified `shape.lock` artifacts.
//!
//! This replaces the legacy `shape.database.lock.json` sidecar file and keeps
//! external schema state in the shared lockfile model.

use crate::package_lock::{ArtifactDeterminism, LockedArtifact, PackageLock};
use crate::type_schema::{
    TypeSchema, TypeSchemaBuilder, typed_object_from_nb_pairs, typed_object_to_hashmap_nb,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shape_value::ValueWord;
use shape_wire::WireValue;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::RwLock;

/// Top-level cache file path. Kept for call-site compatibility.
pub const CACHE_FILENAME: &str = "shape.lock";

const SCHEMA_CACHE_VERSION: u32 = 1;
const SCHEMA_CACHE_NAMESPACE: &str = "external.datasource.schema";
const SCHEMA_CACHE_PRODUCER: &str = "shape-runtime/schema_cache@v1";

static DEFAULT_CACHE_PATH_OVERRIDE: std::sync::LazyLock<RwLock<Option<PathBuf>>> =
    std::sync::LazyLock::new(|| RwLock::new(None));

/// Diagnostic emitted while loading schema artifacts from `shape.lock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaCacheDiagnostic {
    pub key: String,
    pub message: String,
}

/// Top-level cache model for external data-source schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceSchemaCache {
    /// Schema cache format version.
    pub version: u32,
    /// Source URI -> schema mapping.
    pub sources: HashMap<String, SourceSchema>,
}

/// Schema for a single data-source URI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSchema {
    /// The source URI (e.g., "duckdb://analytics.db").
    pub uri: String,
    /// Entity name -> schema mapping.
    pub tables: HashMap<String, EntitySchema>,
    /// RFC3339 timestamp of when schemas were last fetched.
    pub cached_at: String,
}

/// Schema for a single data-source entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySchema {
    /// Entity name.
    pub name: String,
    /// Ordered list of fields.
    pub columns: Vec<FieldSchema>,
}

/// Schema for a single field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// Column name.
    pub name: String,
    /// Shape type: `int`, `number`, `string`, `bool`, `timestamp`.
    #[serde(rename = "type")]
    pub shape_type: String,
    /// Whether the column accepts null values.
    pub nullable: bool,
}

/// Convert a `SourceSchema` into a typed-object ValueWord payload.
///
/// This is used by module-capability extensions to return rich schema
/// metadata through the shared module invocation path.
pub fn source_schema_to_nb(schema: &SourceSchema) -> ValueWord {
    let mut table_pairs: Vec<(String, ValueWord)> = schema
        .tables
        .iter()
        .map(|(table_name, entity)| {
            let columns = entity
                .columns
                .iter()
                .map(|column| {
                    typed_object_from_nb_pairs(&[
                        (
                            "name",
                            ValueWord::from_string(Arc::new(column.name.clone())),
                        ),
                        (
                            "type",
                            ValueWord::from_string(Arc::new(column.shape_type.clone())),
                        ),
                        ("nullable", ValueWord::from_bool(column.nullable)),
                    ])
                })
                .collect::<Vec<_>>();

            let entity_nb = typed_object_from_nb_pairs(&[
                (
                    "name",
                    ValueWord::from_string(Arc::new(entity.name.clone())),
                ),
                ("columns", ValueWord::from_array(Arc::new(columns))),
            ]);

            (table_name.clone(), entity_nb)
        })
        .collect();
    table_pairs.sort_by(|left, right| left.0.cmp(&right.0));

    let table_refs: Vec<(&str, ValueWord)> = table_pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect();
    let tables_nb = typed_object_from_nb_pairs(&table_refs);

    typed_object_from_nb_pairs(&[
        ("uri", ValueWord::from_string(Arc::new(schema.uri.clone()))),
        ("tables", tables_nb),
        (
            "cached_at",
            ValueWord::from_string(Arc::new(schema.cached_at.clone())),
        ),
    ])
}

/// Decode a `SourceSchema` from a typed-object ValueWord payload.
pub fn source_schema_from_nb(value: &ValueWord) -> Result<SourceSchema, String> {
    let object = typed_object_to_hashmap_nb(value)
        .ok_or_else(|| "schema payload must be a typed object".to_string())?;

    let uri = object
        .get("uri")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| "schema payload missing string field 'uri'".to_string())?;

    let cached_at = object
        .get("cached_at")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let tables_nb = object
        .get("tables")
        .ok_or_else(|| "schema payload missing object field 'tables'".to_string())?;
    let tables_obj = typed_object_to_hashmap_nb(tables_nb)
        .ok_or_else(|| "schema payload field 'tables' must be an object".to_string())?;

    let mut tables = HashMap::new();
    for (table_name, entity_nb) in tables_obj {
        let entity_obj = typed_object_to_hashmap_nb(&entity_nb)
            .ok_or_else(|| format!("table '{table_name}' schema must be an object"))?;

        let entity_name = entity_obj
            .get("name")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| table_name.clone());

        let columns_nb = entity_obj
            .get("columns")
            .ok_or_else(|| format!("table '{table_name}' missing 'columns' array"))?;
        let columns_arr = columns_nb
            .as_any_array()
            .ok_or_else(|| format!("table '{table_name}' field 'columns' must be an array"))?
            .to_generic();

        let mut columns = Vec::new();
        for column_nb in columns_arr.iter() {
            let column_obj = typed_object_to_hashmap_nb(column_nb)
                .ok_or_else(|| format!("table '{table_name}' contains non-object column entry"))?;
            let name = column_obj
                .get("name")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .ok_or_else(|| format!("table '{table_name}' column missing string 'name'"))?;
            let shape_type = column_obj
                .get("type")
                .or_else(|| column_obj.get("shape_type"))
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .ok_or_else(|| format!("table '{table_name}' column '{name}' missing type"))?;
            let nullable = column_obj
                .get("nullable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            columns.push(FieldSchema {
                name,
                shape_type,
                nullable,
            });
        }

        tables.insert(
            table_name.clone(),
            EntitySchema {
                name: entity_name,
                columns,
            },
        );
    }

    Ok(SourceSchema {
        uri,
        tables,
        cached_at,
    })
}

/// Decode a `SourceSchema` from a shape-wire object payload.
pub fn source_schema_from_wire(value: &WireValue) -> Result<SourceSchema, String> {
    let object = match value {
        WireValue::Object(map) => map,
        _ => return Err("schema payload must be an object".to_string()),
    };

    let uri = object
        .get("uri")
        .and_then(|v| match v {
            WireValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| "schema payload missing string field 'uri'".to_string())?;

    let cached_at = object
        .get("cached_at")
        .and_then(|v| match v {
            WireValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();

    let tables_value = object
        .get("tables")
        .ok_or_else(|| "schema payload missing object field 'tables'".to_string())?;
    let tables_object = match tables_value {
        WireValue::Object(map) => map,
        _ => return Err("schema payload field 'tables' must be an object".to_string()),
    };

    let mut tables = HashMap::new();
    for (table_name, entity_value) in tables_object {
        let entity_obj = match entity_value {
            WireValue::Object(map) => map,
            _ => return Err(format!("table '{table_name}' schema must be an object")),
        };

        let entity_name = entity_obj
            .get("name")
            .and_then(|v| match v {
                WireValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| table_name.clone());

        let columns_value = entity_obj
            .get("columns")
            .ok_or_else(|| format!("table '{table_name}' missing 'columns' array"))?;
        let columns_array = match columns_value {
            WireValue::Array(values) => values,
            _ => {
                return Err(format!(
                    "table '{table_name}' field 'columns' must be an array"
                ));
            }
        };

        let mut columns = Vec::new();
        for column_value in columns_array {
            let column_obj = match column_value {
                WireValue::Object(map) => map,
                _ => {
                    return Err(format!(
                        "table '{table_name}' contains non-object column entry"
                    ));
                }
            };

            let name = column_obj
                .get("name")
                .and_then(|v| match v {
                    WireValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| format!("table '{table_name}' column missing string 'name'"))?;

            let shape_type = column_obj
                .get("type")
                .or_else(|| column_obj.get("shape_type"))
                .and_then(|v| match v {
                    WireValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| format!("table '{table_name}' column '{name}' missing type"))?;

            let nullable = column_obj
                .get("nullable")
                .and_then(|v| match v {
                    WireValue::Bool(b) => Some(*b),
                    _ => None,
                })
                .unwrap_or(false);

            columns.push(FieldSchema {
                name,
                shape_type,
                nullable,
            });
        }

        tables.insert(
            table_name.clone(),
            EntitySchema {
                name: entity_name,
                columns,
            },
        );
    }

    Ok(SourceSchema {
        uri,
        tables,
        cached_at,
    })
}

impl DataSourceSchemaCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            version: SCHEMA_CACHE_VERSION,
            sources: HashMap::new(),
        }
    }

    /// Save cache entries into `shape.lock` artifacts.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut lock = PackageLock::read(path).unwrap_or_default();

        // Replace this namespace wholesale to keep the lock deterministic.
        lock.artifacts
            .retain(|artifact| artifact.namespace != SCHEMA_CACHE_NAMESPACE);

        let mut uris: Vec<_> = self.sources.keys().cloned().collect();
        uris.sort();

        for uri in uris {
            let Some(source) = self.sources.get(&uri) else {
                continue;
            };

            let schema_hash = hash_source_schema(source);
            let payload = source_to_payload(source);

            let mut inputs = BTreeMap::new();
            inputs.insert("uri".to_string(), uri.clone());
            inputs.insert("schema_hash".to_string(), schema_hash.clone());

            let determinism = ArtifactDeterminism::External {
                fingerprints: BTreeMap::from([(format!("schema:{uri}"), schema_hash)]),
            };

            let artifact = LockedArtifact::new(
                SCHEMA_CACHE_NAMESPACE,
                uri,
                SCHEMA_CACHE_PRODUCER,
                determinism,
                inputs,
                payload,
            )
            .map_err(std::io::Error::other)?;

            lock.upsert_artifact(artifact)
                .map_err(std::io::Error::other)?;
        }

        lock.write(path)
    }

    /// Load cache entries from `shape.lock` artifacts.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let (cache, diagnostics) = Self::load_with_diagnostics(path)?;
        if let Some(diag) = diagnostics.first() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid schema artifact '{}': {}", diag.key, diag.message),
            ));
        }
        Ok(cache)
    }

    /// Load cache entries from `shape.lock` artifacts and collect diagnostics for
    /// stale/invalid artifacts while keeping valid entries.
    pub fn load_with_diagnostics(
        path: &Path,
    ) -> std::io::Result<(Self, Vec<SchemaCacheDiagnostic>)> {
        let lock = PackageLock::read(path).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "shape.lock not found")
        })?;

        let mut sources = HashMap::new();
        let mut diagnostics = Vec::new();
        for artifact in lock
            .artifacts
            .iter()
            .filter(|artifact| artifact.namespace == SCHEMA_CACHE_NAMESPACE)
        {
            let payload = match artifact.payload() {
                Ok(payload) => payload,
                Err(err) => {
                    diagnostics.push(SchemaCacheDiagnostic {
                        key: artifact.key.clone(),
                        message: format!("payload decode failed: {err}"),
                    });
                    continue;
                }
            };
            let source = match payload_to_source(&artifact.key, &payload) {
                Ok(source) => source,
                Err(err) => {
                    diagnostics.push(SchemaCacheDiagnostic {
                        key: artifact.key.clone(),
                        message: format!("payload parse failed: {err}"),
                    });
                    continue;
                }
            };

            if let Some(expected_hash) = source_fingerprint_hash(artifact, &source.uri) {
                let actual_hash = hash_source_schema(&source);
                if expected_hash != actual_hash {
                    diagnostics.push(SchemaCacheDiagnostic {
                        key: artifact.key.clone(),
                        message: format!(
                            "stale schema fingerprint (expected {expected_hash}, computed {actual_hash})"
                        ),
                    });
                    continue;
                }
            } else {
                diagnostics.push(SchemaCacheDiagnostic {
                    key: artifact.key.clone(),
                    message: "missing schema fingerprint".to_string(),
                });
                continue;
            }
            sources.insert(source.uri.clone(), source);
        }

        Ok((
            Self {
                version: SCHEMA_CACHE_VERSION,
                sources,
            },
            diagnostics,
        ))
    }

    /// Try to load cache entries, returning an empty cache if lockfile is missing.
    pub fn load_or_empty(path: &Path) -> Self {
        match Self::load(path) {
            Ok(cache) => cache,
            Err(_) => Self::new(),
        }
    }

    /// Get schema for a specific source URI.
    pub fn get_source(&self, uri: &str) -> Option<&SourceSchema> {
        self.sources.get(uri)
    }

    /// Insert or update a source schema.
    pub fn upsert_source(&mut self, schema: SourceSchema) {
        self.sources.insert(schema.uri.clone(), schema);
    }

    /// Check if offline mode is enabled via `SHAPE_OFFLINE=true`.
    pub fn is_offline() -> bool {
        std::env::var("SHAPE_OFFLINE")
            .map(|value| value == "true" || value == "1")
            .unwrap_or(false)
    }
}

/// Load cached schemas and convert matching sources into runtime `TypeSchema`s.
///
/// `uri_prefixes` filters which sources are included (for example:
/// `["duckdb://"]` or `["postgres://", "postgresql://"]`).
pub fn load_cached_type_schemas_for_uri_prefixes(
    cache_path: &Path,
    uri_prefixes: &[&str],
) -> std::io::Result<Vec<TypeSchema>> {
    let (schemas, _diagnostics) =
        load_cached_type_schemas_for_uri_prefixes_with_diagnostics(cache_path, uri_prefixes)?;
    Ok(schemas)
}

/// Like [`load_cached_type_schemas_for_uri_prefixes`] but also returns
/// non-fatal diagnostics for invalid/stale artifacts that were ignored.
pub fn load_cached_type_schemas_for_uri_prefixes_with_diagnostics(
    cache_path: &Path,
    uri_prefixes: &[&str],
) -> std::io::Result<(Vec<TypeSchema>, Vec<SchemaCacheDiagnostic>)> {
    let (cache, diagnostics) = DataSourceSchemaCache::load_with_diagnostics(cache_path)?;
    let mut schemas = Vec::new();

    for source in cache.sources.values() {
        if !uri_prefixes.is_empty()
            && !uri_prefixes
                .iter()
                .any(|prefix| source.uri.starts_with(prefix))
        {
            continue;
        }

        for table in source.tables.values() {
            schemas.push(type_schema_from_entity(table));
        }
    }

    Ok((schemas, diagnostics))
}

/// Resolve the default `shape.lock` path from the current working directory.
pub fn default_cache_path() -> PathBuf {
    if let Ok(guard) = DEFAULT_CACHE_PATH_OVERRIDE.read() {
        if let Some(path) = guard.as_ref() {
            return path.clone();
        }
    }

    std::env::current_dir()
        .unwrap_or_default()
        .join(CACHE_FILENAME)
}

/// Override the default lock/cache path used by extension-side schema cache
/// helpers during this process.
pub fn set_default_cache_path(path: Option<PathBuf>) {
    if let Ok(mut guard) = DEFAULT_CACHE_PATH_OVERRIDE.write() {
        *guard = path;
    }
}

/// Load one cached source schema by URI.
pub fn load_cached_source_for_uri(cache_path: &Path, uri: &str) -> std::io::Result<SourceSchema> {
    let (source, diagnostics) = load_cached_source_for_uri_with_diagnostics(cache_path, uri)?;
    if let Some(diag) = diagnostics.first() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid schema artifact '{}': {}", diag.key, diag.message),
        ));
    }
    Ok(source)
}

/// Load one cached source schema by URI and keep non-fatal diagnostics for
/// stale/invalid artifacts that were ignored.
pub fn load_cached_source_for_uri_with_diagnostics(
    cache_path: &Path,
    uri: &str,
) -> std::io::Result<(SourceSchema, Vec<SchemaCacheDiagnostic>)> {
    let (cache, diagnostics) = DataSourceSchemaCache::load_with_diagnostics(cache_path)?;
    let source = cache.get_source(uri).cloned().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no cached schema for '{uri}'"),
        )
    })?;
    Ok((source, diagnostics))
}

impl Default for DataSourceSchemaCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceSchema {
    /// Get schema for a specific entity.
    pub fn get_entity(&self, name: &str) -> Option<&EntitySchema> {
        self.tables.get(name)
    }

    /// Get all entity names.
    pub fn entity_names(&self) -> Vec<&str> {
        self.tables.keys().map(|name| name.as_str()).collect()
    }
}

impl EntitySchema {
    /// Get a field by name.
    pub fn get_field(&self, name: &str) -> Option<&FieldSchema> {
        self.columns.iter().find(|column| column.name == name)
    }

    /// Get all field names.
    pub fn field_names(&self) -> Vec<&str> {
        self.columns
            .iter()
            .map(|column| column.name.as_str())
            .collect()
    }
}

fn hash_source_schema(source: &SourceSchema) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.uri.as_bytes());
    hasher.update([0]);

    let mut entity_names: Vec<_> = source.tables.keys().cloned().collect();
    entity_names.sort();
    for entity_name in entity_names {
        hasher.update(entity_name.as_bytes());
        hasher.update([0]);

        if let Some(entity) = source.tables.get(&entity_name) {
            for field in &entity.columns {
                hasher.update(field.name.as_bytes());
                hasher.update([0]);
                hasher.update(field.shape_type.as_bytes());
                hasher.update([0]);
                hasher.update([if field.nullable { 1 } else { 0 }]);
            }
        }
    }

    format!("sha256:{:x}", hasher.finalize())
}

fn source_fingerprint_hash<'a>(artifact: &'a LockedArtifact, uri: &str) -> Option<&'a str> {
    if let ArtifactDeterminism::External { fingerprints } = &artifact.determinism {
        let key = format!("schema:{uri}");
        if let Some(value) = fingerprints.get(&key) {
            return Some(value.as_str());
        }
    }
    artifact.inputs.get("schema_hash").map(String::as_str)
}

fn type_schema_from_entity(entity: &EntitySchema) -> TypeSchema {
    let schema_name = format!("DbRow_{}", entity.name);
    let builder =
        entity
            .columns
            .iter()
            .fold(
                TypeSchemaBuilder::new(&schema_name),
                |builder, field| match field.shape_type.as_str() {
                    "int" => builder.i64_field(&field.name),
                    "number" => builder.f64_field(&field.name),
                    "string" => builder.string_field(&field.name),
                    "bool" => builder.bool_field(&field.name),
                    "timestamp" => builder.timestamp_field(&field.name),
                    _ => builder.any_field(&field.name),
                },
            );
    builder.build()
}

fn source_to_payload(source: &SourceSchema) -> shape_wire::WireValue {
    let mut entities: Vec<_> = source.tables.values().collect();
    entities.sort_by(|left, right| left.name.cmp(&right.name));

    let entity_values = entities
        .into_iter()
        .map(|entity| {
            let field_values = entity
                .columns
                .iter()
                .map(|field| {
                    shape_wire::WireValue::Object(BTreeMap::from([
                        (
                            "name".to_string(),
                            shape_wire::WireValue::String(field.name.clone()),
                        ),
                        (
                            "shape_type".to_string(),
                            shape_wire::WireValue::String(field.shape_type.clone()),
                        ),
                        (
                            "nullable".to_string(),
                            shape_wire::WireValue::Bool(field.nullable),
                        ),
                    ]))
                })
                .collect::<Vec<_>>();

            shape_wire::WireValue::Object(BTreeMap::from([
                (
                    "name".to_string(),
                    shape_wire::WireValue::String(entity.name.clone()),
                ),
                (
                    "columns".to_string(),
                    shape_wire::WireValue::Array(field_values),
                ),
            ]))
        })
        .collect::<Vec<_>>();

    shape_wire::WireValue::Object(BTreeMap::from([
        (
            "uri".to_string(),
            shape_wire::WireValue::String(source.uri.clone()),
        ),
        (
            "cached_at".to_string(),
            shape_wire::WireValue::String(source.cached_at.clone()),
        ),
        (
            "tables".to_string(),
            shape_wire::WireValue::Array(entity_values),
        ),
    ]))
}

fn payload_to_source(
    key_hint: &str,
    payload: &shape_wire::WireValue,
) -> Result<SourceSchema, String> {
    let shape_wire::WireValue::Object(map) = payload else {
        return Err("source payload must be an object".to_string());
    };

    let uri = map
        .get("uri")
        .and_then(shape_wire::WireValue::as_str)
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| key_hint.to_string());
    let cached_at = map
        .get("cached_at")
        .and_then(shape_wire::WireValue::as_str)
        .unwrap_or("")
        .to_string();

    let tables_value = map
        .get("tables")
        .ok_or_else(|| "source payload missing 'tables'".to_string())?;
    let shape_wire::WireValue::Array(table_values) = tables_value else {
        return Err("source payload 'tables' must be an array".to_string());
    };

    let mut tables = HashMap::new();
    for table_value in table_values {
        let entity = payload_to_entity(table_value)?;
        tables.insert(entity.name.clone(), entity);
    }

    Ok(SourceSchema {
        uri,
        tables,
        cached_at,
    })
}

fn payload_to_entity(value: &shape_wire::WireValue) -> Result<EntitySchema, String> {
    let shape_wire::WireValue::Object(table_map) = value else {
        return Err("table payload must be an object".to_string());
    };

    let name = table_map
        .get("name")
        .and_then(shape_wire::WireValue::as_str)
        .ok_or_else(|| "table payload missing 'name'".to_string())?
        .to_string();
    let columns_value = table_map
        .get("columns")
        .ok_or_else(|| "table payload missing 'columns'".to_string())?;
    let shape_wire::WireValue::Array(column_values) = columns_value else {
        return Err("table payload 'columns' must be an array".to_string());
    };

    let mut columns = Vec::with_capacity(column_values.len());
    for column_value in column_values {
        columns.push(payload_to_field(column_value)?);
    }

    Ok(EntitySchema { name, columns })
}

fn payload_to_field(value: &shape_wire::WireValue) -> Result<FieldSchema, String> {
    let shape_wire::WireValue::Object(column_map) = value else {
        return Err("column payload must be an object".to_string());
    };

    let name = column_map
        .get("name")
        .and_then(shape_wire::WireValue::as_str)
        .ok_or_else(|| "column payload missing 'name'".to_string())?
        .to_string();
    let shape_type = column_map
        .get("shape_type")
        .and_then(shape_wire::WireValue::as_str)
        .ok_or_else(|| "column payload missing 'shape_type'".to_string())?
        .to_string();
    let nullable = column_map
        .get("nullable")
        .and_then(shape_wire::WireValue::as_bool)
        .ok_or_else(|| "column payload missing 'nullable'".to_string())?;

    Ok(FieldSchema {
        name,
        shape_type,
        nullable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_lock::ArtifactDeterminism;

    fn sample_cache() -> DataSourceSchemaCache {
        let mut cache = DataSourceSchemaCache::new();
        let mut tables = HashMap::new();
        tables.insert(
            "users".to_string(),
            EntitySchema {
                name: "users".to_string(),
                columns: vec![
                    FieldSchema {
                        name: "id".to_string(),
                        shape_type: "int".to_string(),
                        nullable: false,
                    },
                    FieldSchema {
                        name: "name".to_string(),
                        shape_type: "string".to_string(),
                        nullable: false,
                    },
                    FieldSchema {
                        name: "age".to_string(),
                        shape_type: "int".to_string(),
                        nullable: true,
                    },
                ],
            },
        );
        cache.upsert_source(SourceSchema {
            uri: "duckdb://analytics.db".to_string(),
            tables,
            cached_at: "2026-02-12T10:00:00Z".to_string(),
        });
        cache
    }

    #[test]
    fn test_roundtrip_serialization() {
        let cache = sample_cache();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILENAME);
        cache.save(&path).unwrap();

        let loaded = DataSourceSchemaCache::load(&path).unwrap();
        assert_eq!(loaded.version, SCHEMA_CACHE_VERSION);
        assert_eq!(loaded.sources.len(), 1);

        let conn = loaded.get_source("duckdb://analytics.db").unwrap();
        assert_eq!(conn.tables.len(), 1);

        let users = conn.get_entity("users").unwrap();
        assert_eq!(users.columns.len(), 3);
        assert_eq!(users.columns[0].name, "id");
        assert_eq!(users.columns[0].shape_type, "int");
        assert!(!users.columns[0].nullable);
        assert_eq!(users.columns[2].name, "age");
        assert!(users.columns[2].nullable);
    }

    #[test]
    fn test_load_or_empty_missing_file() {
        let cache = DataSourceSchemaCache::load_or_empty(Path::new("/nonexistent/path.toml"));
        assert_eq!(cache.version, SCHEMA_CACHE_VERSION);
        assert!(cache.sources.is_empty());
    }

    #[test]
    fn test_source_helpers() {
        let cache = sample_cache();
        let conn = cache.get_source("duckdb://analytics.db").unwrap();

        let names = conn.entity_names();
        assert!(names.contains(&"users"));

        let users = conn.get_entity("users").unwrap();
        assert_eq!(users.field_names(), vec!["id", "name", "age"]);
        assert!(users.get_field("id").is_some());
        assert!(users.get_field("nonexistent").is_none());
    }

    #[test]
    fn test_upsert_source() {
        let mut cache = DataSourceSchemaCache::new();
        assert!(cache.sources.is_empty());

        cache.upsert_source(SourceSchema {
            uri: "duckdb://test.db".to_string(),
            tables: HashMap::new(),
            cached_at: "2026-01-01T00:00:00Z".to_string(),
        });
        assert_eq!(cache.sources.len(), 1);

        cache.upsert_source(SourceSchema {
            uri: "duckdb://test.db".to_string(),
            tables: HashMap::new(),
            cached_at: "2026-02-01T00:00:00Z".to_string(),
        });
        assert_eq!(cache.sources.len(), 1);
        assert_eq!(
            cache.get_source("duckdb://test.db").unwrap().cached_at,
            "2026-02-01T00:00:00Z"
        );
    }

    #[test]
    fn test_load_with_diagnostics_reports_stale_fingerprint() {
        let cache = sample_cache();
        let source = cache.get_source("duckdb://analytics.db").unwrap();

        let payload = source_to_payload(source);
        let artifact = LockedArtifact::new(
            SCHEMA_CACHE_NAMESPACE,
            source.uri.clone(),
            SCHEMA_CACHE_PRODUCER,
            ArtifactDeterminism::External {
                fingerprints: BTreeMap::from([(
                    format!("schema:{}", source.uri),
                    "sha256:deadbeef".to_string(),
                )]),
            },
            BTreeMap::from([
                ("uri".to_string(), source.uri.clone()),
                ("schema_hash".to_string(), "sha256:deadbeef".to_string()),
            ]),
            payload,
        )
        .unwrap();

        let mut lock = PackageLock::new();
        lock.upsert_artifact(artifact).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILENAME);
        lock.write(&path).unwrap();

        let (loaded, diagnostics) = DataSourceSchemaCache::load_with_diagnostics(&path).unwrap();
        assert!(loaded.sources.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("stale schema fingerprint"));
    }

    #[test]
    fn test_load_with_diagnostics_reports_invalid_payload() {
        let artifact = LockedArtifact::new(
            SCHEMA_CACHE_NAMESPACE,
            "broken://source",
            SCHEMA_CACHE_PRODUCER,
            ArtifactDeterminism::External {
                fingerprints: BTreeMap::from([(
                    "schema:broken://source".to_string(),
                    "sha256:abc".to_string(),
                )]),
            },
            BTreeMap::from([
                ("uri".to_string(), "broken://source".to_string()),
                ("schema_hash".to_string(), "sha256:abc".to_string()),
            ]),
            shape_wire::WireValue::String("bad".to_string()),
        )
        .unwrap();

        let mut lock = PackageLock::new();
        lock.upsert_artifact(artifact).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILENAME);
        lock.write(&path).unwrap();

        let (loaded, diagnostics) = DataSourceSchemaCache::load_with_diagnostics(&path).unwrap();
        assert!(loaded.sources.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("payload parse failed"));
    }

    #[test]
    fn test_load_cached_type_schemas_for_uri_prefixes_filters_sources() {
        let mut cache = DataSourceSchemaCache::new();

        let mut duck_tables = HashMap::new();
        duck_tables.insert(
            "users".to_string(),
            EntitySchema {
                name: "users".to_string(),
                columns: vec![FieldSchema {
                    name: "id".to_string(),
                    shape_type: "int".to_string(),
                    nullable: false,
                }],
            },
        );
        cache.upsert_source(SourceSchema {
            uri: "duckdb://analytics.db".to_string(),
            tables: duck_tables,
            cached_at: "2026-02-17T00:00:00Z".to_string(),
        });

        let mut pg_tables = HashMap::new();
        pg_tables.insert(
            "orders".to_string(),
            EntitySchema {
                name: "orders".to_string(),
                columns: vec![FieldSchema {
                    name: "id".to_string(),
                    shape_type: "int".to_string(),
                    nullable: false,
                }],
            },
        );
        cache.upsert_source(SourceSchema {
            uri: "postgres://localhost/app".to_string(),
            tables: pg_tables,
            cached_at: "2026-02-17T00:00:00Z".to_string(),
        });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILENAME);
        cache.save(&path).unwrap();

        let duck_schemas =
            load_cached_type_schemas_for_uri_prefixes(&path, &["duckdb://"]).unwrap();
        assert_eq!(duck_schemas.len(), 1);
        assert_eq!(duck_schemas[0].name, "DbRow_users");

        let pg_schemas =
            load_cached_type_schemas_for_uri_prefixes(&path, &["postgres://", "postgresql://"])
                .unwrap();
        assert_eq!(pg_schemas.len(), 1);
        assert_eq!(pg_schemas[0].name, "DbRow_orders");
    }

    #[test]
    fn test_load_cached_source_for_uri_with_diagnostics() {
        let cache = sample_cache();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILENAME);
        cache.save(&path).unwrap();

        let (source, diagnostics) =
            load_cached_source_for_uri_with_diagnostics(&path, "duckdb://analytics.db").unwrap();
        assert_eq!(source.uri, "duckdb://analytics.db");
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_default_cache_path_ends_with_shape_lock() {
        let path = default_cache_path();
        assert!(path.ends_with(CACHE_FILENAME));
    }
}
