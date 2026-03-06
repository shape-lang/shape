//! `shape schema` subcommands for data-source schema caching.
//!
//! - `shape schema fetch [URI]` — discover source schemas and write to shape.lock artifacts
//! - `shape schema status` — show cached source schemas and staleness

use anyhow::{Context, Result};
use shape_runtime::engine::ShapeEngine;
use shape_runtime::project::{self, ExternalLockMode};
use shape_runtime::schema_cache::{
    CACHE_FILENAME, DataSourceSchemaCache, EntitySchema, FieldSchema, SourceSchema,
    source_schema_from_wire,
};
use shape_wire::WireValue;
use std::collections::HashMap;
use std::path::PathBuf;

/// Find the project root (directory containing shape.lock or cwd).
fn project_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(project) = project::find_project_root(&cwd) {
        project.root_path
    } else {
        cwd
    }
}

fn external_lock_mode(root: &std::path::Path) -> ExternalLockMode {
    project::find_project_root(root)
        .map(|project| project.config.build.external.mode)
        .unwrap_or(ExternalLockMode::Update)
}

/// Scan `.shape` source files for `connect("...")` URI literals.
fn scan_connect_uris(dir: &std::path::Path) -> Vec<String> {
    let mut uris = Vec::new();
    let pattern = regex::Regex::new(r#"connect\(\s*"([^"]+)"\s*\)"#).unwrap();

    scan_dir_for_uris(dir, &pattern, &mut uris);
    uris.sort();
    uris.dedup();
    uris
}

fn scan_dir_for_uris(dir: &std::path::Path, pattern: &regex::Regex, uris: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.starts_with('.') && name != "node_modules" && name != "target" {
                scan_dir_for_uris(&path, pattern, uris);
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("shape") {
            if let Ok(source) = std::fs::read_to_string(&path) {
                for cap in pattern.captures_iter(&source) {
                    if let Some(uri) = cap.get(1) {
                        uris.push(uri.as_str().to_string());
                    }
                }
            }
        }
    }
}

/// `shape schema fetch [URI]` — fetch source schemas and cache them.
pub async fn run_schema_fetch(
    uri: Option<String>,
    provider_opts: &super::ProviderOptions,
    cli_extensions: &[PathBuf],
) -> Result<()> {
    let root = project_root();
    if matches!(external_lock_mode(&root), ExternalLockMode::Frozen) {
        anyhow::bail!(
            "build.external.mode is 'frozen'; schema refresh is disabled. Switch to 'update' to refresh data-source schema artifacts."
        );
    }
    let cache_path = root.join(CACHE_FILENAME);
    let mut cache = DataSourceSchemaCache::load_or_empty(&cache_path);

    let uris = if let Some(uri) = uri {
        vec![uri]
    } else {
        eprintln!("Scanning source files for connect() URIs...");
        let found = scan_connect_uris(&root);
        if found.is_empty() {
            eprintln!("No connect() calls found in source files.");
            return Ok(());
        }
        eprintln!("Found {} URI(s): {:?}", found.len(), found);
        found
    };

    let mut engine = ShapeEngine::new().context("failed to create Shape engine")?;
    load_schema_extensions(&mut engine, provider_opts, cli_extensions)?;

    for uri in &uris {
        eprintln!("Fetching schema for {}...", uri);
        match fetch_source_schema_with_extensions(&engine, uri).await {
            Ok(mut source_schema) => {
                source_schema.cached_at = chrono::Utc::now().to_rfc3339();
                let entity_count = source_schema.tables.len();
                let field_count: usize =
                    source_schema.tables.values().map(|t| t.columns.len()).sum();
                cache.upsert_source(source_schema);
                eprintln!(
                    "  Cached {} entity(s), {} field(s) for {}",
                    entity_count, field_count, uri
                );
            }
            Err(e) => {
                eprintln!("  Error fetching {}: {}", uri, e);
            }
        }
    }

    cache
        .save(&cache_path)
        .context("Failed to save data-source schema cache")?;
    eprintln!("Saved to {}", cache_path.display());

    Ok(())
}

fn load_schema_extensions(
    engine: &mut ShapeEngine,
    provider_opts: &super::ProviderOptions,
    cli_extensions: &[PathBuf],
) -> Result<()> {
    let project = project::find_project_root(&project_root());
    let specs = crate::extension_loading::collect_startup_specs(
        provider_opts,
        project.as_ref(),
        None,
        None,
        cli_extensions,
    );

    let loaded =
        crate::extension_loading::load_specs(engine, &specs, |_spec, _info| {}, |_spec, _err| {});
    if loaded == 0 {
        anyhow::bail!(
            "No extension modules were loaded. Configure [[extensions]] in shape.toml or pass --extension."
        );
    }
    Ok(())
}

async fn fetch_source_schema_with_extensions(
    engine: &ShapeEngine,
    uri: &str,
) -> Result<SourceSchema, String> {
    if !uri.contains("://") {
        return Err(format!("Schema URI must include a scheme, got: {}", uri));
    }

    let mut attempts = Vec::new();
    let schema_args = [WireValue::String(uri.to_string())];

    // Primary path: module-capability schema discovery (`source_schema(uri)`).
    for extension_name in engine.list_extensions() {
        match engine.invoke_extension_module_wire(&extension_name, "source_schema", &schema_args) {
            Ok(value) => match source_schema_from_wire(&value) {
                Ok(schema) => return Ok(schema),
                Err(err) => attempts.push(format!(
                    "{}: invalid source_schema payload ({})",
                    extension_name, err
                )),
            },
            Err(err) => attempts.push(format!(
                "{}: module source_schema unavailable ({})",
                extension_name, err
            )),
        }
    }

    // Fallback path: legacy datasource schema discovery.
    for extension_name in engine.list_extensions() {
        let Some(plugin) = engine.get_extension(&extension_name) else {
            continue;
        };

        if !plugin.supports_schema_discovery() {
            attempts.push(format!(
                "{}: schema discovery not supported",
                extension_name
            ));
            continue;
        }

        match plugin.get_source_schema(uri) {
            Ok(plugin_schema) => {
                let entity_name = entity_name_from_uri(uri, &extension_name);
                let columns = plugin_schema
                    .columns
                    .into_iter()
                    .map(|col| {
                        let db_type = format!("{:?}", col.data_type);
                        let shape_type = match db_type.as_str() {
                            "Number" => "number",
                            "Integer" => "int",
                            "Boolean" => "bool",
                            "Timestamp" => "timestamp",
                            _ => "string",
                        };
                        FieldSchema {
                            name: col.name,
                            shape_type: shape_type.to_string(),
                            nullable: true,
                        }
                    })
                    .collect::<Vec<_>>();

                let mut tables = HashMap::new();
                tables.insert(
                    entity_name.clone(),
                    EntitySchema {
                        name: entity_name,
                        columns,
                    },
                );

                return Ok(SourceSchema {
                    uri: uri.to_string(),
                    tables,
                    cached_at: String::new(),
                });
            }
            Err(err) => attempts.push(format!("{}: {}", extension_name, err)),
        }
    }

    if attempts.is_empty() {
        return Err(format!(
            "No loaded data-source extension can resolve schema for '{}'",
            uri
        ));
    }

    Err(format!(
        "No extension could fetch schema for '{}':\n  {}",
        uri,
        attempts.join("\n  ")
    ))
}

fn entity_name_from_uri(uri: &str, extension_name: &str) -> String {
    let raw = uri
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(uri)
        .trim_matches('/');
    let tail = raw
        .split('/')
        .next_back()
        .unwrap_or("source")
        .split('?')
        .next()
        .unwrap_or("source")
        .split('#')
        .next()
        .unwrap_or("source");

    let mut name = String::with_capacity(tail.len());
    for ch in tail.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
        } else {
            name.push('_');
        }
    }
    if name.is_empty() {
        format!("{}_source", extension_name)
    } else {
        name
    }
}

/// `shape schema status` — show cached source schemas.
pub async fn run_schema_status() -> Result<()> {
    let root = project_root();
    let cache_path = root.join(CACHE_FILENAME);

    if !cache_path.exists() {
        eprintln!("No data-source schema cache found in {}.", CACHE_FILENAME);
        eprintln!("Run `shape schema fetch` to create one.");
        return Ok(());
    }

    let (cache, diagnostics) = DataSourceSchemaCache::load_with_diagnostics(&cache_path)
        .context("Failed to read data-source schema cache")?;

    eprintln!("Data-source schema cache: {}", cache_path.display());
    eprintln!("Version: {}", cache.version);
    eprintln!("Sources: {}", cache.sources.len());
    if !diagnostics.is_empty() {
        eprintln!("Diagnostics: {}", diagnostics.len());
        for diag in &diagnostics {
            eprintln!("  ! {}: {}", diag.key, diag.message);
        }
    }

    for (uri, source) in &cache.sources {
        eprintln!("\n  {} (cached at {})", uri, source.cached_at);
        for (entity_name, entity) in &source.tables {
            eprintln!("    {} ({} fields)", entity_name, entity.columns.len());
            for field in &entity.columns {
                let nullable = if field.nullable { "?" } else { "" };
                eprintln!("      {}: {}{}", field.name, field.shape_type, nullable);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_scan_connect_uris() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.shape");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(
            f,
            r#"
            let conn = connect("duckdb://analytics.db")
            let pg = connect("postgres://localhost/mydb")
            "#
        )
        .unwrap();

        let uris = scan_connect_uris(dir.path());
        assert_eq!(uris.len(), 2);
        assert!(uris.contains(&"duckdb://analytics.db".to_string()));
        assert!(uris.contains(&"postgres://localhost/mydb".to_string()));
    }

    #[test]
    fn test_scan_connect_uris_empty() {
        let dir = tempfile::tempdir().unwrap();
        let uris = scan_connect_uris(dir.path());
        assert!(uris.is_empty());
    }

    #[test]
    fn test_scan_connect_uris_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("a.shape");
        let f2 = dir.path().join("b.shape");
        std::fs::write(&f1, r#"connect("duckdb://db.duckdb")"#).unwrap();
        std::fs::write(&f2, r#"connect("duckdb://db.duckdb")"#).unwrap();

        let uris = scan_connect_uris(dir.path());
        assert_eq!(uris.len(), 1);
    }

    #[test]
    fn test_external_lock_mode_defaults_to_update() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            external_lock_mode(dir.path()),
            ExternalLockMode::Update
        ));
    }

    #[test]
    fn test_external_lock_mode_reads_frozen_from_shape_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("shape.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[build.external]
mode = "frozen"
"#,
        )
        .unwrap();

        assert!(matches!(
            external_lock_mode(dir.path()),
            ExternalLockMode::Frozen
        ));
    }
}
