//! Front-matter parser for Shape scripts
//!
//! Parses optional TOML front-matter delimited by `---` at the top of a script.
//! Also skips shebang lines (`#!/...`).
//!
//! Frontmatter is for **standalone script** metadata/dependencies/extensions.
//! It supports `[dependencies]`, `[dev-dependencies]`, and `[[extensions]]`.
//! Packaging/build sections like `[project]` and `[build]` still belong in `shape.toml`.

use crate::project::{ModulesSection, ShapeProject};
use shape_value::ValueWordExt;
use serde::Deserialize;

/// Script-level frontmatter configuration.
///
/// This is intentionally focused on script-level metadata fields used by
/// diagnostics and editor hints.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FrontmatterConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Module search paths (allowed in frontmatter for scripts)
    #[serde(default)]
    pub modules: Option<ModulesSection>,
}

/// A diagnostic produced during frontmatter validation.
#[derive(Debug, Clone)]
pub struct FrontmatterDiagnostic {
    pub message: String,
    pub severity: FrontmatterDiagnosticSeverity,
    pub location: Option<FrontmatterDiagnosticLocation>,
}

/// Severity level for frontmatter diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontmatterDiagnosticSeverity {
    Error,
    Warning,
}

/// Source location for a frontmatter diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontmatterDiagnosticLocation {
    pub line: u32,
    pub character: u32,
    pub length: u32,
}

/// Sections that are forbidden in frontmatter (they belong in `shape.toml`).
const FORBIDDEN_SECTIONS: &[(&str, &str)] = &[
    (
        "project",
        "The [project] section belongs in shape.toml, not in file frontmatter",
    ),
    (
        "build",
        "Build configuration must be specified in shape.toml",
    ),
    ("plugins", "Use [[extensions]] instead of [plugins]"),
];

/// Known top-level keys allowed in frontmatter.
pub const FRONTMATTER_TOP_LEVEL_KEYS: &[&str] =
    &["name", "description", "version", "author", "tags"];

/// Known table sections allowed in frontmatter.
pub const FRONTMATTER_SECTION_KEYS: &[&str] =
    &["modules", "dependencies", "dev-dependencies", "extensions"];

/// Keys allowed inside `[[extensions]]` entries.
pub const FRONTMATTER_EXTENSION_KEYS: &[&str] = &["name", "path", "config"];

/// Keys allowed inside `[modules]`.
pub const FRONTMATTER_MODULE_KEYS: &[&str] = &["paths"];

const ALLOWED_KEYS: &[&str] = &[
    "name",
    "description",
    "version",
    "author",
    "tags",
    "modules",
    "dependencies",
    "dev-dependencies",
    "extensions",
];

const ALLOWED_EXTENSION_KEYS: &[&str] = FRONTMATTER_EXTENSION_KEYS;

/// Result of extracting the TOML body from frontmatter delimiters.
struct FrontmatterBody<'a> {
    toml_str: &'a str,
    remaining: &'a str,
    toml_start_line: u32,
}

/// Shared logic for both `parse_frontmatter` and `parse_frontmatter_validated`.
///
/// Skips shebang, finds `---` delimiters, and extracts the TOML body and
/// remaining source. Returns `None` if no valid frontmatter block is found.
fn extract_frontmatter_body(source: &str) -> Option<FrontmatterBody<'_>> {
    let has_shebang = source.starts_with("#!");
    let rest = if has_shebang {
        match source.find('\n') {
            Some(pos) => &source[pos + 1..],
            None => return None,
        }
    } else {
        source
    };

    let trimmed = rest.trim_start_matches([' ', '\t']);
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_marker = &trimmed[3..];
    let first_newline = after_marker.find('\n');
    match first_newline {
        Some(pos) if after_marker[..pos].trim().is_empty() => {}
        None if after_marker.trim().is_empty() => return None,
        _ => return None,
    }

    let body_start = &after_marker[first_newline.unwrap() + 1..];

    let end_pos = find_closing_delimiter(body_start)?;

    let toml_str = &body_start[..end_pos];
    let after_closing_line = &body_start[end_pos..];
    let remaining = match after_closing_line.find('\n') {
        Some(pos) => &after_closing_line[pos + 1..],
        None => "",
    };

    Some(FrontmatterBody {
        toml_str,
        remaining,
        toml_start_line: if has_shebang { 2 } else { 1 },
    })
}

/// Parse optional front-matter from a Shape source string, with validation.
///
/// Returns `(config, diagnostics, remaining_source)` where:
/// - `config` is `Some` if a `---` delimited TOML block was found and parsed
/// - `diagnostics` contains any validation errors/warnings
/// - `remaining_source` is the Shape code after the front-matter
///
/// Shebang lines (`#!...`) at the very start are skipped before checking
/// for front-matter.
pub fn parse_frontmatter_validated(
    source: &str,
) -> (Option<FrontmatterConfig>, Vec<FrontmatterDiagnostic>, &str) {
    let body = match extract_frontmatter_body(source) {
        Some(b) => b,
        None => return (None, vec![], source),
    };

    let mut diagnostics = validate_frontmatter_toml(body.toml_str, body.toml_start_line);

    match toml::from_str::<FrontmatterConfig>(body.toml_str) {
        Ok(config) => (Some(config), diagnostics, body.remaining),
        Err(err) => {
            diagnostics.push(frontmatter_parse_error_diagnostic(
                body.toml_str,
                body.toml_start_line,
                &err,
            ));
            (None, diagnostics, body.remaining)
        }
    }
}

/// Validate raw TOML string for forbidden project-level sections and unknown keys.
fn validate_frontmatter_toml(toml_str: &str, toml_start_line: u32) -> Vec<FrontmatterDiagnostic> {
    let mut diagnostics = Vec::new();

    let table = match toml_str.parse::<toml::Table>() {
        Ok(t) => t,
        Err(_) => return diagnostics, // parse error handled elsewhere
    };

    for (key, value) in &table {
        // Check forbidden sections
        let mut is_forbidden = false;
        for (section, message) in FORBIDDEN_SECTIONS {
            if key == section {
                diagnostics.push(FrontmatterDiagnostic {
                    message: message.to_string(),
                    severity: FrontmatterDiagnosticSeverity::Error,
                    location: find_section_header_location(toml_str, key, toml_start_line),
                });
                is_forbidden = true;
                break;
            }
        }

        // Warn about unknown keys (not forbidden, not in allowed list)
        if !is_forbidden && !ALLOWED_KEYS.contains(&key.as_str()) {
            if matches!(value, toml::Value::Table(_)) {
                // Table-valued unknown keys may be extension sections
                diagnostics.push(FrontmatterDiagnostic {
                    message: format!(
                        "Unknown section '[{}]' may be an extension section \
                         — will be passed to extensions if claimed",
                        key
                    ),
                    severity: FrontmatterDiagnosticSeverity::Warning,
                    location: find_section_header_location(toml_str, key, toml_start_line),
                });
            } else {
                diagnostics.push(FrontmatterDiagnostic {
                    message: format!(
                        "Unknown frontmatter key '{}'. Allowed keys: name, description, \
                         version, author, tags, modules, dependencies, dev-dependencies, extensions",
                        key
                    ),
                    severity: FrontmatterDiagnosticSeverity::Warning,
                    location: find_top_level_key_location(toml_str, key, toml_start_line),
                });
            }
        }
    }

    diagnostics.extend(validate_extension_entries(toml_str, toml_start_line));

    diagnostics
}

fn validate_extension_entries(toml_str: &str, toml_start_line: u32) -> Vec<FrontmatterDiagnostic> {
    #[derive(Debug, Clone, Copy)]
    struct ExtensionEntryState {
        header_line: u32,
        has_name: bool,
        has_path: bool,
    }

    fn finalize_entry(
        diagnostics: &mut Vec<FrontmatterDiagnostic>,
        entry: Option<ExtensionEntryState>,
    ) {
        let Some(entry) = entry else {
            return;
        };

        if !entry.has_name {
            diagnostics.push(FrontmatterDiagnostic {
                message: "Missing required key 'name' in [[extensions]] entry".to_string(),
                severity: FrontmatterDiagnosticSeverity::Error,
                location: Some(FrontmatterDiagnosticLocation {
                    line: entry.header_line,
                    character: 0,
                    length: 14,
                }),
            });
        }

        if !entry.has_path {
            diagnostics.push(FrontmatterDiagnostic {
                message: "Missing required key 'path' in [[extensions]] entry".to_string(),
                severity: FrontmatterDiagnosticSeverity::Error,
                location: Some(FrontmatterDiagnosticLocation {
                    line: entry.header_line,
                    character: 0,
                    length: 14,
                }),
            });
        }
    }

    let mut diagnostics = Vec::new();
    let mut in_extensions = false;
    let mut current_entry: Option<ExtensionEntryState> = None;

    for (idx, raw_line) in toml_str.lines().enumerate() {
        let trimmed = raw_line.trim();
        let absolute_line = toml_start_line + idx as u32;

        if trimmed.starts_with("[[extensions]]") {
            finalize_entry(&mut diagnostics, current_entry.take());
            in_extensions = true;
            current_entry = Some(ExtensionEntryState {
                header_line: absolute_line,
                has_name: false,
                has_path: false,
            });
            continue;
        }

        if trimmed.starts_with("[[") || (trimmed.starts_with('[') && trimmed.ends_with(']')) {
            finalize_entry(&mut diagnostics, current_entry.take());
            in_extensions = false;
            continue;
        }

        if !in_extensions {
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some(eq_pos) = raw_line.find('=') else {
            continue;
        };

        let key = raw_line[..eq_pos].trim();
        let key_start = raw_line
            .find(key)
            .or_else(|| raw_line[..eq_pos].find(|c: char| !c.is_whitespace()))
            .unwrap_or(0) as u32;

        if let Some(entry) = current_entry.as_mut() {
            match key {
                "name" => entry.has_name = true,
                "path" => entry.has_path = true,
                "config" => {}
                _ => {
                    if !ALLOWED_EXTENSION_KEYS.contains(&key) {
                        diagnostics.push(FrontmatterDiagnostic {
                            message: format!(
                                "Unknown key '{}' in [[extensions]] entry. Allowed keys: name, path, config",
                                key
                            ),
                            severity: FrontmatterDiagnosticSeverity::Error,
                            location: Some(FrontmatterDiagnosticLocation {
                                line: absolute_line,
                                character: key_start,
                                length: key.len() as u32,
                            }),
                        });
                    }
                }
            }
        }
    }

    finalize_entry(&mut diagnostics, current_entry);
    diagnostics
}

fn frontmatter_parse_error_diagnostic(
    toml_str: &str,
    toml_start_line: u32,
    err: &toml::de::Error,
) -> FrontmatterDiagnostic {
    let location = err.span().map(|span| {
        let (line, character) = offset_to_line_col(toml_str, span.start);
        FrontmatterDiagnosticLocation {
            line: toml_start_line + line,
            character,
            length: 1,
        }
    });

    FrontmatterDiagnostic {
        message: format!("Frontmatter TOML parse error: {}", err.message()),
        severity: FrontmatterDiagnosticSeverity::Error,
        location,
    }
}

fn find_section_header_location(
    toml_str: &str,
    section: &str,
    toml_start_line: u32,
) -> Option<FrontmatterDiagnosticLocation> {
    let header = format!("[{}]", section);
    for (idx, raw_line) in toml_str.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed == header {
            let start = raw_line.find('[').unwrap_or(0) as u32;
            return Some(FrontmatterDiagnosticLocation {
                line: toml_start_line + idx as u32,
                character: start,
                length: header.len() as u32,
            });
        }
    }
    None
}

fn find_top_level_key_location(
    toml_str: &str,
    key: &str,
    toml_start_line: u32,
) -> Option<FrontmatterDiagnosticLocation> {
    let mut in_section = false;
    for (idx, raw_line) in toml_str.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = true;
            continue;
        }
        if in_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq_pos) = raw_line.find('=') else {
            continue;
        };
        let current_key = raw_line[..eq_pos].trim();
        if current_key == key {
            let key_start = raw_line.find(key).unwrap_or(0) as u32;
            return Some(FrontmatterDiagnosticLocation {
                line: toml_start_line + idx as u32,
                character: key_start,
                length: key.len() as u32,
            });
        }
    }
    None
}

fn offset_to_line_col(text: &str, offset: usize) -> (u32, u32) {
    let mut line = 0u32;
    let mut col = 0u32;
    for (i, ch) in text.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Parse optional front-matter from a Shape source string.
///
/// Returns `(config, remaining_source)` where `config` is `Some` if a
/// `---` delimited TOML block was found, and `remaining_source` is the
/// Shape code after the front-matter (or the full source if none).
///
/// This is the backwards-compatible version. For validation diagnostics,
/// use [`parse_frontmatter_validated`] instead.
///
/// Shebang lines (`#!...`) at the very start are skipped before checking
/// for front-matter.
pub fn parse_frontmatter(source: &str) -> (Option<ShapeProject>, &str) {
    let body = match extract_frontmatter_body(source) {
        Some(b) => b,
        None => return (None, source),
    };

    match crate::project::parse_shape_project_toml(body.toml_str) {
        Ok(config) => (Some(config), body.remaining),
        Err(_) => (None, body.remaining),
    }
}

/// Find the byte offset of a line that is exactly `---` (with optional whitespace).
fn find_closing_delimiter(s: &str) -> Option<usize> {
    let mut offset = 0;
    for line in s.lines() {
        if line.trim() == "---" {
            return Some(offset);
        }
        offset += line.len() + 1; // +1 for newline
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Legacy parse_frontmatter tests ----

    #[test]
    fn test_no_frontmatter() {
        let source = "let x = 1;\nprint(x);\n";
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_none());
        assert_eq!(rest, source);
    }

    #[test]
    fn test_with_frontmatter() {
        let source = r#"---
[modules]
paths = ["lib"]
---
let x = 1;
"#;
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_some());
        let cfg = config.unwrap();
        assert_eq!(cfg.modules.paths, vec!["lib"]);
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_with_frontmatter_extensions() {
        let source = r#"---
[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---
let x = 1;
"#;
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_some());
        let cfg = config.unwrap();
        assert_eq!(cfg.extensions.len(), 1);
        assert_eq!(cfg.extensions[0].name, "duckdb");
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_shebang_with_frontmatter() {
        let source = r#"#!/usr/bin/env shape
---
[project]
name = "script"

[modules]
paths = ["lib", "vendor"]
---
print("hello");
"#;
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_some());
        let cfg = config.unwrap();
        assert_eq!(cfg.project.name, "script");
        assert_eq!(cfg.modules.paths, vec!["lib", "vendor"]);
        assert_eq!(rest, "print(\"hello\");\n");
    }

    #[test]
    fn test_shebang_without_frontmatter() {
        let source = "#!/usr/bin/env shape\nlet x = 1;\n";
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_none());
        assert_eq!(rest, source);
    }

    #[test]
    fn test_malformed_toml() {
        let source = "---\nthis is not valid toml {{{\n---\nlet x = 1;\n";
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_none());
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_no_closing_delimiter() {
        let source = "---\n[modules]\npaths = [\"lib\"]\nlet x = 1;\n";
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_none());
        assert_eq!(rest, source);
    }

    // ---- Validated frontmatter tests ----

    #[test]
    fn test_validated_no_frontmatter() {
        let source = "let x = 1;\nprint(x);\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_none());
        assert!(diagnostics.is_empty());
        assert_eq!(rest, source);
    }

    #[test]
    fn test_validated_valid_frontmatter() {
        let source = r#"---
name = "my-script"
description = "A test script"
version = "1.0.0"
author = "dev"
tags = ["analysis", "test"]

[modules]
paths = ["lib"]
---
let x = 1;
"#;
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert!(
            diagnostics.is_empty(),
            "Expected no diagnostics but got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        let cfg = config.unwrap();
        assert_eq!(cfg.name.as_deref(), Some("my-script"));
        assert_eq!(cfg.description.as_deref(), Some("A test script"));
        assert_eq!(cfg.version.as_deref(), Some("1.0.0"));
        assert_eq!(cfg.author.as_deref(), Some("dev"));
        assert_eq!(
            cfg.tags.as_deref(),
            Some(&["analysis".to_string(), "test".to_string()][..])
        );
        assert_eq!(cfg.modules.as_ref().unwrap().paths, vec!["lib"]);
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_validated_empty_frontmatter() {
        let source = "---\n---\nlet x = 1;\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert!(diagnostics.is_empty());
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_validated_project_section_error() {
        let source = r#"---
[project]
name = "bad"
---
let x = 1;
"#;
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        // Config is still parsed (with unknown fields ignored), but diagnostics are emitted
        assert!(config.is_some());
        assert_eq!(rest, "let x = 1;\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].severity,
            FrontmatterDiagnosticSeverity::Error
        );
        assert!(diagnostics[0].message.contains("[project]"));
        assert!(diagnostics[0].message.contains("shape.toml"));
    }

    #[test]
    fn test_validated_dependencies_allowed() {
        let source = "---\n[dependencies]\nfoo = \"1.0\"\n---\nlet x = 1;\n";
        let (_config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(diagnostics.is_empty());
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_validated_build_section_error() {
        let source = "---\n[build]\noptimize = true\n---\nlet x = 1;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].severity,
            FrontmatterDiagnosticSeverity::Error
        );
        assert!(diagnostics[0].message.contains("Build configuration"));
    }

    #[test]
    fn test_validated_extensions_allowed() {
        let source = r#"---
[[extensions]]
name = "csv"
path = "./libshape_plugin_csv.so"
---
let x = 1;
"#;
        let (_config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(diagnostics.is_empty());
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_validated_plugins_error() {
        let source = "---\n[plugins]\nname = \"plug\"\n---\nlet x = 1;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].severity,
            FrontmatterDiagnosticSeverity::Error
        );
        assert!(diagnostics[0].message.contains("[[extensions]]"));
    }

    #[test]
    fn test_validated_dev_dependencies_allowed() {
        let source = "---\n[dev-dependencies]\ntest-lib = \"2.0\"\n---\nlet x = 1;\n";
        let (_config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(diagnostics.is_empty());
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_validated_multiple_forbidden_sections() {
        let source = r#"---
[project]
name = "bad"

[dependencies]
foo = "1.0"

[build]
optimize = true
---
let x = 1;
"#;
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == FrontmatterDiagnosticSeverity::Error)
        );
    }

    #[test]
    fn test_validated_unknown_key_warning() {
        let source = "---\nfoo = \"bar\"\n---\nlet x = 1;\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert_eq!(rest, "let x = 1;\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].severity,
            FrontmatterDiagnosticSeverity::Warning
        );
        assert!(
            diagnostics[0]
                .message
                .contains("Unknown frontmatter key 'foo'")
        );
        assert_eq!(
            diagnostics[0].location,
            Some(FrontmatterDiagnosticLocation {
                line: 1,
                character: 0,
                length: 3,
            })
        );
    }

    #[test]
    fn test_validated_unknown_extensions_key_error() {
        let source = r#"---
[[extensions]]
nm = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---
let x = 1;
"#;
        let (_config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert_eq!(rest, "let x = 1;\n");
        assert!(diagnostics.iter().any(|d| {
            d.severity == FrontmatterDiagnosticSeverity::Error
                && d.message
                    .contains("Unknown key 'nm' in [[extensions]] entry")
                && d.location
                    == Some(FrontmatterDiagnosticLocation {
                        line: 2,
                        character: 0,
                        length: 2,
                    })
        }));
    }

    #[test]
    fn test_validated_shebang_with_validation() {
        let source = r#"#!/usr/bin/env shape
---
name = "my-script"

[modules]
paths = ["lib"]
---
print("hello");
"#;
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert!(diagnostics.is_empty());
        let cfg = config.unwrap();
        assert_eq!(cfg.name.as_deref(), Some("my-script"));
        assert_eq!(cfg.modules.as_ref().unwrap().paths, vec!["lib"]);
        assert_eq!(rest, "print(\"hello\");\n");
    }

    #[test]
    fn test_validated_malformed_toml() {
        let source = "---\nthis is not valid toml {{{\n---\nlet x = 1;\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_none());
        assert_eq!(rest, "let x = 1;\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].severity,
            FrontmatterDiagnosticSeverity::Error
        );
        assert!(
            diagnostics[0]
                .message
                .contains("Frontmatter TOML parse error")
        );
    }

    #[test]
    fn test_validated_no_closing_delimiter() {
        let source = "---\nname = \"test\"\nlet x = 1;\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_none());
        assert!(diagnostics.is_empty());
        assert_eq!(rest, source);
    }

    #[test]
    fn test_validated_extension_section_softer_diagnostic() {
        let source = "---\n[native-dependencies]\nlibm = \"libm.so\"\n---\nlet x = 1;\n";
        let (_config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert_eq!(rest, "let x = 1;\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].severity,
            FrontmatterDiagnosticSeverity::Warning
        );
        assert!(
            diagnostics[0].message.contains("extension section"),
            "Table-valued unknown key should get softer message, got: {}",
            diagnostics[0].message
        );
    }

    #[test]
    fn test_validated_scalar_unknown_key_still_warns() {
        let source = "---\nfoo = \"bar\"\n---\nlet x = 1;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message.contains("Unknown frontmatter key"),
            "Scalar unknown key should get existing warning, got: {}",
            diagnostics[0].message
        );
    }
}
