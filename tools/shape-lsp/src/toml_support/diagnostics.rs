//! Diagnostics for shape.toml files.
//!
//! Validates TOML syntax, known sections/keys, required fields, and types.

use super::schema::{self, ValueType};
use crate::util::offset_to_line_col;
use tower_lsp_server::ls_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Validate a shape.toml file and return diagnostics.
pub fn validate_toml(text: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Step 1: Parse the TOML
    let table = match text.parse::<toml::Table>() {
        Ok(t) => t,
        Err(err) => {
            diagnostics.push(toml_parse_error_diagnostic(text, &err));
            return diagnostics;
        }
    };

    // Step 1b: Validate against the canonical ShapeProject parser so LSP and
    // runtime share one manifest source-of-truth.
    if let Err(err) = shape_runtime::project::parse_shape_project_toml(text) {
        diagnostics.push(shape_project_parse_error_diagnostic(text, &err));
    }

    // Step 2: Check for unknown top-level sections
    for (key, _value) in &table {
        if schema::find_section(key).is_none() {
            let (line, col) = find_key_position(text, key);
            diagnostics.push(Diagnostic {
                range: range_for_word(line, col, key.len()),
                severity: Some(DiagnosticSeverity::WARNING),
                message: format!("Unknown section `[{}]`.", key),
                source: Some("shape-toml".to_string()),
                ..Default::default()
            });
        }
    }

    // Step 3: Check required fields
    check_required_fields(&table, text, &mut diagnostics);

    // Step 4: Check unknown keys within known sections
    check_unknown_keys(&table, text, &mut diagnostics);

    // Step 5: Type validation for known keys
    check_value_types(&table, text, &mut diagnostics);

    diagnostics
}

/// Create a diagnostic from a TOML parse error.
fn toml_parse_error_diagnostic(text: &str, err: &toml::de::Error) -> Diagnostic {
    let (line, character) = if let Some(span) = err.span() {
        offset_to_line_col(text, span.start)
    } else {
        (0, 0)
    };

    Diagnostic {
        range: Range {
            start: Position { line, character },
            end: Position {
                line,
                character: character + 1,
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        message: format!("TOML syntax error: {}", err.message()),
        source: Some("shape-toml".to_string()),
        ..Default::default()
    }
}

/// Create a diagnostic from a canonical ShapeProject parse error.
fn shape_project_parse_error_diagnostic(text: &str, err: &toml::de::Error) -> Diagnostic {
    let (line, character) = if let Some(span) = err.span() {
        offset_to_line_col(text, span.start)
    } else {
        (0, 0)
    };

    Diagnostic {
        range: Range {
            start: Position { line, character },
            end: Position {
                line,
                character: character + 1,
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        message: format!("shape.toml parse error: {}", err.message()),
        source: Some("shape-toml".to_string()),
        ..Default::default()
    }
}

/// Check that required fields are present.
fn check_required_fields(table: &toml::Table, text: &str, diagnostics: &mut Vec<Diagnostic>) {
    for section_def in schema::SECTIONS {
        if section_def.is_array_table || section_def.is_free_form {
            continue;
        }

        let required_keys: Vec<&str> = section_def
            .keys
            .iter()
            .filter(|k| k.required)
            .map(|k| k.name)
            .collect();

        if required_keys.is_empty() {
            continue;
        }

        match table.get(section_def.name) {
            Some(toml::Value::Table(section_table)) => {
                for req_key in &required_keys {
                    if !section_table.contains_key(*req_key) {
                        // Find the section header position for the diagnostic
                        let (line, col) = find_section_header_position(text, section_def.name);
                        diagnostics.push(Diagnostic {
                            range: range_for_word(line, col, section_def.name.len() + 2), // include brackets
                            severity: Some(DiagnosticSeverity::ERROR),
                            message: format!(
                                "Missing required key `{}` in `[{}]`.",
                                req_key, section_def.name
                            ),
                            source: Some("shape-toml".to_string()),
                            ..Default::default()
                        });
                    } else if let Some(toml::Value::String(s)) = section_table.get(*req_key) {
                        if s.is_empty() {
                            let (line, col) = find_key_in_section(text, section_def.name, req_key);
                            diagnostics.push(Diagnostic {
                                range: range_for_word(line, col, req_key.len()),
                                severity: Some(DiagnosticSeverity::ERROR),
                                message: format!(
                                    "`{}.{}` must not be empty.",
                                    section_def.name, req_key
                                ),
                                source: Some("shape-toml".to_string()),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            Some(_) => {
                // Section exists but is not a table — weird, flag it
                let (line, col) = find_key_position(text, section_def.name);
                diagnostics.push(Diagnostic {
                    range: range_for_word(line, col, section_def.name.len()),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: format!("`{}` must be a table section.", section_def.name),
                    source: Some("shape-toml".to_string()),
                    ..Default::default()
                });
            }
            None => {
                // Section is entirely absent but has required keys — warn at top
                if section_def.name == "project" {
                    diagnostics.push(Diagnostic {
                        range: Range {
                            start: Position {
                                line: 0,
                                character: 0,
                            },
                            end: Position {
                                line: 0,
                                character: 1,
                            },
                        },
                        severity: Some(DiagnosticSeverity::WARNING),
                        message: format!(
                            "Missing `[{}]` section with required keys: {}.",
                            section_def.name,
                            required_keys.join(", "),
                        ),
                        source: Some("shape-toml".to_string()),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Validate array-of-tables sections (extensions)
    for section_def in schema::SECTIONS.iter().filter(|s| s.is_array_table) {
        if let Some(toml::Value::Array(entries)) = table.get(section_def.name) {
            for (i, entry) in entries.iter().enumerate() {
                if let toml::Value::Table(entry_table) = entry {
                    for key_def in section_def.keys.iter().filter(|k| k.required) {
                        if !entry_table.contains_key(key_def.name) {
                            let (line, col) =
                                find_nth_array_table_header(text, section_def.name, i);
                            diagnostics.push(Diagnostic {
                                range: range_for_word(line, col, section_def.name.len() + 4),
                                severity: Some(DiagnosticSeverity::ERROR),
                                message: format!(
                                    "Missing required key `{}` in `[[{}]]` entry #{}.",
                                    key_def.name,
                                    section_def.name,
                                    i + 1
                                ),
                                source: Some("shape-toml".to_string()),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Check for unknown keys within known sections.
fn check_unknown_keys(table: &toml::Table, text: &str, diagnostics: &mut Vec<Diagnostic>) {
    for (section_name, section_value) in table {
        let section_def = match schema::find_section(section_name) {
            Some(s) => s,
            None => continue, // already reported as unknown section
        };

        if section_def.is_free_form {
            continue;
        }

        if section_def.is_array_table {
            if let toml::Value::Array(entries) = section_value {
                for entry in entries {
                    if let toml::Value::Table(entry_table) = entry {
                        check_keys_in_table(
                            entry_table,
                            section_def,
                            section_name,
                            text,
                            diagnostics,
                        );
                    }
                }
            }
        } else if let toml::Value::Table(section_table) = section_value {
            check_keys_in_table(section_table, section_def, section_name, text, diagnostics);
        }
    }
}

fn check_keys_in_table(
    table: &toml::Table,
    section_def: &schema::SectionDef,
    section_name: &str,
    text: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for key in table.keys() {
        if section_def.keys.iter().all(|k| k.name != key.as_str()) {
            let (line, col) = find_key_in_section(text, section_name, key);
            diagnostics.push(Diagnostic {
                range: range_for_word(line, col, key.len()),
                severity: Some(DiagnosticSeverity::WARNING),
                message: format!("Unknown key `{}` in `[{}]`.", key, section_name),
                source: Some("shape-toml".to_string()),
                ..Default::default()
            });
        }
    }
}

/// Validate types of known keys.
fn check_value_types(table: &toml::Table, text: &str, diagnostics: &mut Vec<Diagnostic>) {
    for (section_name, section_value) in table {
        let section_def = match schema::find_section(section_name) {
            Some(s) => s,
            None => continue,
        };

        if section_def.is_free_form || section_def.is_array_table {
            continue;
        }

        if let toml::Value::Table(section_table) = section_value {
            for (key, value) in section_table {
                if let Some(key_def) = section_def.keys.iter().find(|k| k.name == key.as_str()) {
                    let type_ok = match key_def.value_type {
                        ValueType::Str => matches!(value, toml::Value::String(_)),
                        ValueType::Integer => matches!(value, toml::Value::Integer(_)),
                        ValueType::Bool => matches!(value, toml::Value::Boolean(_)),
                        ValueType::ArrayOfStrings => {
                            if let toml::Value::Array(arr) = value {
                                arr.iter().all(|v| matches!(v, toml::Value::String(_)))
                            } else {
                                false
                            }
                        }
                        ValueType::Table => matches!(value, toml::Value::Table(_)),
                    };

                    if !type_ok {
                        let (line, col) = find_key_in_section(text, section_name, key);
                        diagnostics.push(Diagnostic {
                            range: range_for_word(line, col, key.len()),
                            severity: Some(DiagnosticSeverity::ERROR),
                            message: format!(
                                "`{}.{}` expects type {}, got {}.",
                                section_name,
                                key,
                                key_def.value_type.display_name(),
                                toml_value_type_name(value),
                            ),
                            source: Some("shape-toml".to_string()),
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }
}

fn toml_value_type_name(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

// ---- Position-finding helpers ----

fn find_key_position(text: &str, key: &str) -> (u32, u32) {
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(key) {
            let leading_ws = line.len() - trimmed.len();
            return (i as u32, leading_ws as u32);
        }
    }
    (0, 0)
}

fn find_section_header_position(text: &str, section_name: &str) -> (u32, u32) {
    let pattern = format!("[{}]", section_name);
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == pattern || trimmed == format!("[[{}]]", section_name) {
            return (i as u32, 0);
        }
    }
    (0, 0)
}

fn find_key_in_section(text: &str, section_name: &str, key: &str) -> (u32, u32) {
    let mut in_section = false;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == format!("[{}]", section_name) || trimmed == format!("[[{}]]", section_name) {
            in_section = true;
            continue;
        }
        if in_section {
            if trimmed.starts_with('[') {
                in_section = false;
                continue;
            }
            if let Some(eq_pos) = trimmed.find('=') {
                let k = trimmed[..eq_pos].trim();
                if k == key {
                    let leading_ws = line.len() - line.trim_start().len();
                    return (i as u32, leading_ws as u32);
                }
            }
        }
    }
    (0, 0)
}

fn find_nth_array_table_header(text: &str, section_name: &str, n: usize) -> (u32, u32) {
    let pattern = format!("[[{}]]", section_name);
    let mut count = 0;
    for (i, line) in text.lines().enumerate() {
        if line.trim() == pattern {
            if count == n {
                return (i as u32, 0);
            }
            count += 1;
        }
    }
    (0, 0)
}

fn range_for_word(line: u32, col: u32, len: usize) -> Range {
    Range {
        start: Position {
            line,
            character: col,
        },
        end: Position {
            line,
            character: col + len as u32,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_toml() {
        let text = r#"
[project]
name = "test"
version = "0.1.0"
"#;
        let diags = validate_toml(text);
        assert!(
            diags.is_empty(),
            "Expected no diagnostics, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_syntax_error() {
        let text = "[project\nname = ";
        let diags = validate_toml(text);
        assert!(!diags.is_empty());
        assert!(diags[0].message.contains("syntax error"));
    }

    #[test]
    fn test_unknown_section() {
        let text = r#"
[project]
name = "test"
version = "0.1.0"

[nonexistent]
foo = "bar"
"#;
        let diags = validate_toml(text);
        let unknown: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("Unknown section"))
            .collect();
        assert_eq!(unknown.len(), 1);
        assert!(unknown[0].message.contains("nonexistent"));
    }

    #[test]
    fn test_missing_required_key() {
        let text = r#"
[project]
name = "test"
"#;
        let diags = validate_toml(text);
        let missing: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("Missing required"))
            .collect();
        assert!(
            missing.iter().any(|d| d.message.contains("version")),
            "Should flag missing version: {:?}",
            diags
        );
    }

    #[test]
    fn test_empty_required_key() {
        let text = r#"
[project]
name = ""
version = "0.1.0"
"#;
        let diags = validate_toml(text);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("must not be empty")),
            "Should flag empty name: {:?}",
            diags
        );
    }

    #[test]
    fn test_unknown_key_in_section() {
        let text = r#"
[project]
name = "test"
version = "0.1.0"
unknown_field = "value"
"#;
        let diags = validate_toml(text);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("Unknown key") && d.message.contains("unknown_field")),
            "Should flag unknown key: {:?}",
            diags
        );
    }

    #[test]
    fn test_type_mismatch() {
        let text = r#"
[project]
name = 42
version = "0.1.0"
"#;
        let diags = validate_toml(text);
        assert!(
            diags.iter().any(|d| d.message.contains("expects type")),
            "Should flag type mismatch: {:?}",
            diags
        );
    }

    #[test]
    fn test_missing_project_section_warning() {
        let text = r#"
[build]
target = "bytecode"
"#;
        let diags = validate_toml(text);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("Missing `[project]`")),
            "Should warn about missing [project]: {:?}",
            diags
        );
    }

    #[test]
    fn test_valid_full_config() {
        let text = r#"
[project]
name = "my-analysis"
version = "0.1.0"
entry = "src/main.shape"
authors = ["Alice", "Bob"]
license = "MIT"

[modules]
paths = ["lib", "vendor"]

[dependencies]
finance = "0.1.0"

[build]
target = "bytecode"
opt_level = 2

[[extensions]]
name = "duckdb"
path = "./ext/libduckdb.so"

[[extensions]]
name = "market-data"
path = "./plugins/market.so"
"#;
        let diags = validate_toml(text);
        assert!(
            diags.is_empty(),
            "Expected no diagnostics, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_extensions_missing_required() {
        let text = r#"
[project]
name = "test"
version = "0.1.0"

[[extensions]]
name = "duckdb"
"#;
        let diags = validate_toml(text);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("Missing required key `path`")),
            "Should flag missing path in extensions: {:?}",
            diags
        );
    }
}
