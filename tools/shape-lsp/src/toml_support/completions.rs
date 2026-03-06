//! Completions for shape.toml files.
//!
//! Provides section header, key, and value completions based on cursor position.

use super::schema::{self, ValueType};
use shape_runtime::frontmatter::{FRONTMATTER_SECTION_KEYS, FRONTMATTER_TOP_LEVEL_KEYS};
use tower_lsp_server::ls_types::{CompletionItem, CompletionItemKind, InsertTextFormat, Position};

/// Determine which section the cursor is inside by scanning lines above.
fn current_section(text: &str, line: u32) -> Option<String> {
    let lines: Vec<&str> = split_lines(text);
    for l in (0..=line as usize).rev() {
        let line_text = lines.get(l)?;
        let trimmed = line_text.trim();
        // Match [[section]] (array table) or [section]
        if let Some(name) = parse_section_header(trimmed) {
            return Some(name);
        }
    }
    None
}

/// Split text into lines, preserving trailing empty line after final `\n`.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.lines().collect();
    if text.ends_with('\n') {
        lines.push("");
    }
    lines
}

/// Parse a TOML section header like `[project]` or `[[extensions]]` and return the name.
fn parse_section_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
        let inner = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
        // Handle dotted names like `extensions.config`
        Some(inner.to_string())
    } else if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.starts_with("[[") {
        let inner = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
        Some(inner.to_string())
    } else {
        None
    }
}

/// Get the text of the line at the given position.
fn line_text_at(text: &str, line: u32) -> Option<&str> {
    let lines = split_lines(text);
    lines.into_iter().nth(line as usize)
}

/// Collect keys already used in the current section (to avoid duplicate suggestions).
fn existing_keys_in_section(text: &str, line: u32) -> Vec<String> {
    let mut keys = Vec::new();
    // Scan upward to find section start, then scan downward collecting keys
    let lines: Vec<&str> = split_lines(text);
    let mut start = 0;
    for l in (0..=line as usize).rev() {
        if let Some(lt) = lines.get(l) {
            if parse_section_header(lt.trim()).is_some() {
                start = l + 1;
                break;
            }
        }
    }
    // Scan from section start to next section or end
    for l in start..lines.len() {
        if l != start as usize {
            if let Some(lt) = lines.get(l) {
                if parse_section_header(lt.trim()).is_some() {
                    break;
                }
            }
        }
        if let Some(lt) = lines.get(l) {
            let trimmed = lt.trim();
            if let Some(eq_pos) = trimmed.find('=') {
                let key = trimmed[..eq_pos].trim();
                if !key.is_empty() {
                    keys.push(key.to_string());
                }
            }
        }
    }
    keys
}

/// Produce completions for a shape.toml file at the given cursor position.
pub fn get_toml_completions(text: &str, position: Position) -> Vec<CompletionItem> {
    let line_str = line_text_at(text, position.line).unwrap_or("");

    let trimmed = line_str[..std::cmp::min(position.character as usize, line_str.len())].trim();

    // Case 1: Completing a section header (line starts with `[`)
    if trimmed.starts_with('[') {
        return section_header_completions();
    }

    // Case 2: Line is empty or at start of a new key — suggest section headers or keys
    if trimmed.is_empty() {
        return match current_section(text, position.line) {
            Some(section_name) => key_completions(&section_name, text, position.line),
            // At top level, suggest section headers
            None => section_header_completions(),
        };
    }

    // Case 3: After `=` on a line — suggest values
    if let Some(eq_idx) = trimmed.find('=') {
        let key = trimmed[..eq_idx].trim();
        if let Some(section_name) = current_section(text, position.line) {
            return value_completions(&section_name, key);
        }
    }

    // Case 4: Typing a key name inside a section
    if let Some(section_name) = current_section(text, position.line) {
        return key_completions(&section_name, text, position.line);
    }

    section_header_completions()
}

/// Produce completions for script frontmatter (`--- ... ---`) in `.shape` files.
pub fn get_frontmatter_completions(text: &str, position: Position) -> Vec<CompletionItem> {
    let line_str = line_text_at(text, position.line).unwrap_or("");
    let trimmed = line_str[..std::cmp::min(position.character as usize, line_str.len())].trim();
    let section_name = current_section(text, position.line);

    if trimmed.starts_with('[') {
        return frontmatter_section_header_completions();
    }

    if trimmed.is_empty() {
        return match section_name {
            Some(section) => frontmatter_key_completions(&section, text, position.line),
            None => {
                let mut items = frontmatter_root_key_completions(text, position.line);
                items.extend(frontmatter_section_header_completions());
                items
            }
        };
    }

    if let Some(eq_idx) = trimmed.find('=') {
        let key = trimmed[..eq_idx].trim();
        return match section_name {
            Some(section) => frontmatter_value_completions(&section, key),
            None => frontmatter_root_value_completions(key),
        };
    }

    if let Some(section) = section_name {
        return frontmatter_key_completions(&section, text, position.line);
    }

    let mut items = frontmatter_root_key_completions(text, position.line);
    items.extend(frontmatter_section_header_completions());
    items
}

/// Completions for section headers (e.g. `[project]`).
fn section_header_completions() -> Vec<CompletionItem> {
    schema::SECTIONS
        .iter()
        .map(|s| {
            let insert = if s.is_array_table {
                format!("[[{}]]", s.name)
            } else {
                format!("[{}]", s.name)
            };
            CompletionItem {
                label: insert.clone(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some(s.description.to_string()),
                insert_text: Some(insert),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            }
        })
        .collect()
}

fn frontmatter_section_header_completions() -> Vec<CompletionItem> {
    FRONTMATTER_SECTION_KEYS
        .iter()
        .map(|section_name| {
            let section = schema::find_section(section_name);
            let is_array = section.map(|s| s.is_array_table).unwrap_or(false);
            let insert = if is_array {
                format!("[[{}]]", section_name)
            } else {
                format!("[{}]", section_name)
            };
            CompletionItem {
                label: insert.clone(),
                kind: Some(CompletionItemKind::MODULE),
                detail: section.map(|s| s.description.to_string()),
                insert_text: Some(insert),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            }
        })
        .collect()
}

/// Completions for keys within a section.
fn key_completions(section_name: &str, text: &str, line: u32) -> Vec<CompletionItem> {
    // Handle dotted section names like `extensions.config`
    let base_section = section_name.split('.').next().unwrap_or(section_name);

    let section = match schema::find_section(base_section) {
        Some(s) => s,
        None => return vec![],
    };

    if section.is_free_form {
        // For free-form sections (dependencies), suggest a template
        return vec![CompletionItem {
            label: "package-name".to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some("Add a dependency".to_string()),
            insert_text: Some("package-name = \"0.1.0\"".to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        }];
    }

    let existing = existing_keys_in_section(text, line);
    section
        .keys
        .iter()
        .filter(|k| !existing.contains(&k.name.to_string()))
        .map(|k| {
            let required_marker = if k.required { " (required)" } else { "" };
            CompletionItem {
                label: k.name.to_string(),
                kind: Some(CompletionItemKind::PROPERTY),
                detail: Some(format!(
                    "{}: {}{}",
                    k.name,
                    k.value_type.display_name(),
                    required_marker,
                )),
                documentation: Some(tower_lsp_server::ls_types::Documentation::String(
                    k.description.to_string(),
                )),
                insert_text: Some(format!("{} = ", k.name)),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            }
        })
        .collect()
}

fn frontmatter_key_completions(section_name: &str, text: &str, line: u32) -> Vec<CompletionItem> {
    let base_section = section_name.split('.').next().unwrap_or(section_name);
    if !FRONTMATTER_SECTION_KEYS.contains(&base_section) {
        return Vec::new();
    }
    key_completions(section_name, text, line)
}

/// Completions for values of a key.
fn value_completions(section_name: &str, key: &str) -> Vec<CompletionItem> {
    let base_section = section_name.split('.').next().unwrap_or(section_name);
    let key_def = match schema::find_key(base_section, key) {
        Some(k) => k,
        None => return vec![],
    };

    if !key_def.known_values.is_empty() {
        return key_def
            .known_values
            .iter()
            .map(|v| {
                let insert = match key_def.value_type {
                    ValueType::Str => format!("\"{}\"", v),
                    _ => v.to_string(),
                };
                CompletionItem {
                    label: v.to_string(),
                    kind: Some(CompletionItemKind::VALUE),
                    insert_text: Some(insert),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                }
            })
            .collect();
    }

    // Generic type hint
    match key_def.value_type {
        ValueType::Bool => vec![
            CompletionItem {
                label: "true".to_string(),
                kind: Some(CompletionItemKind::VALUE),
                ..Default::default()
            },
            CompletionItem {
                label: "false".to_string(),
                kind: Some(CompletionItemKind::VALUE),
                ..Default::default()
            },
        ],
        _ => vec![],
    }
}

fn frontmatter_value_completions(section_name: &str, key: &str) -> Vec<CompletionItem> {
    let base_section = section_name.split('.').next().unwrap_or(section_name);
    if !FRONTMATTER_SECTION_KEYS.contains(&base_section) {
        return Vec::new();
    }
    value_completions(section_name, key)
}

fn existing_root_keys(text: &str, line: u32) -> Vec<String> {
    let mut keys = Vec::new();
    let lines = split_lines(text);

    for l in 0..=line as usize {
        let Some(line_text) = lines.get(l) else {
            break;
        };
        let trimmed = line_text.trim();

        if parse_section_header(trimmed).is_some() {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            continue;
        }

        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            if !key.is_empty() {
                keys.push(key.to_string());
            }
        }
    }

    keys
}

fn frontmatter_root_key_completions(text: &str, line: u32) -> Vec<CompletionItem> {
    let existing = existing_root_keys(text, line);

    FRONTMATTER_TOP_LEVEL_KEYS
        .iter()
        .filter(|key| !existing.contains(&key.to_string()))
        .map(|key| CompletionItem {
            label: (*key).to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some(frontmatter_root_key_detail(key).to_string()),
            insert_text: Some(format!("{} = ", key)),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        })
        .collect()
}

fn frontmatter_root_key_detail(key: &str) -> &'static str {
    match key {
        "name" => "name: string",
        "description" => "description: string",
        "version" => "version: string",
        "author" => "author: string",
        "tags" => "tags: array of strings",
        _ => "frontmatter key",
    }
}

fn frontmatter_root_value_completions(key: &str) -> Vec<CompletionItem> {
    match key {
        "tags" => vec![CompletionItem {
            label: "[]".to_string(),
            kind: Some(CompletionItemKind::VALUE),
            insert_text: Some("[\"\"]".to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_header_completions() {
        let items = section_header_completions();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"[project]"));
        assert!(labels.contains(&"[build]"));
        assert!(labels.contains(&"[[extensions]]"));
    }

    #[test]
    fn test_completions_inside_project_section() {
        let text = "[project]\n";
        let pos = Position {
            line: 1,
            character: 0,
        };
        let items = get_toml_completions(text, pos);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"name"));
        assert!(labels.contains(&"version"));
    }

    #[test]
    fn test_completions_exclude_existing_keys() {
        let text = "[project]\nname = \"test\"\n";
        let pos = Position {
            line: 2,
            character: 0,
        };
        let items = get_toml_completions(text, pos);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(!labels.contains(&"name"));
        assert!(labels.contains(&"version"));
    }

    #[test]
    fn test_value_completions_for_build_target() {
        let text = "[build]\ntarget = ";
        let pos = Position {
            line: 1,
            character: 9,
        };
        let items = get_toml_completions(text, pos);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"bytecode"));
        assert!(labels.contains(&"native"));
    }

    #[test]
    fn test_empty_file_suggests_sections() {
        let text = "";
        let pos = Position {
            line: 0,
            character: 0,
        };
        let items = get_toml_completions(text, pos);
        assert!(!items.is_empty());
        // Should contain section headers
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"[project]"));
    }

    #[test]
    fn test_parse_section_header() {
        assert_eq!(
            parse_section_header("[project]"),
            Some("project".to_string())
        );
        assert_eq!(
            parse_section_header("[[extensions]]"),
            Some("extensions".to_string())
        );
        assert_eq!(
            parse_section_header("[extensions.config]"),
            Some("extensions.config".to_string())
        );
        assert_eq!(parse_section_header("name = \"test\""), None);
    }

    #[test]
    fn test_completions_for_dependencies() {
        let text = "[dependencies]\n";
        let pos = Position {
            line: 1,
            character: 0,
        };
        let items = get_toml_completions(text, pos);
        // Should suggest template for free-form section
        assert!(!items.is_empty());
    }

    #[test]
    fn test_value_completions_for_known_values() {
        let text = "[build]\ntarget = ";
        let pos = Position {
            line: 1,
            character: 9,
        };
        let items = get_toml_completions(text, pos);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"bytecode"));
        assert!(labels.contains(&"native"));
    }

    #[test]
    fn test_frontmatter_top_level_completions() {
        let text = "---\n";
        let pos = Position {
            line: 1,
            character: 0,
        };
        let items = get_frontmatter_completions(text, pos);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"name"));
        assert!(labels.contains(&"description"));
        assert!(labels.contains(&"version"));
        assert!(labels.contains(&"[modules]"));
        assert!(labels.contains(&"[[extensions]]"));
        assert!(!labels.contains(&"[project]"));
    }

    #[test]
    fn test_frontmatter_extensions_entry_completions() {
        let text = "---\n[[extensions]]\n";
        let pos = Position {
            line: 2,
            character: 0,
        };
        let items = get_frontmatter_completions(text, pos);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"name"));
        assert!(labels.contains(&"path"));
        assert!(labels.contains(&"config"));
        assert!(!labels.contains(&"entry"));
    }

    #[test]
    fn test_frontmatter_root_key_value_completion_for_tags() {
        let text = "---\ntags = ";
        let pos = Position {
            line: 1,
            character: 7,
        };
        let items = get_frontmatter_completions(text, pos);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"[]"));
    }
}
