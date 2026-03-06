//! Hover information for shape.toml files.
//!
//! Shows documentation for section headers and keys on hover.

use super::schema;
use tower_lsp_server::ls_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

/// Get hover information for a shape.toml file at the given position.
pub fn get_toml_hover(text: &str, position: Position) -> Option<Hover> {
    let line_str = text.lines().nth(position.line as usize)?;

    // Case 1: Hovering over a section header
    if let Some(section_name) = parse_section_header(line_str.trim()) {
        let base_name = section_name.split('.').next().unwrap_or(&section_name);
        let section = schema::find_section(base_name)?;

        let mut content = format!("## `[{}]`\n\n{}", section.name, section.description);

        if !section.keys.is_empty() {
            content.push_str("\n\n**Keys:**\n");
            for key in section.keys {
                let required = if key.required { " (required)" } else { "" };
                content.push_str(&format!(
                    "\n- `{}`: {} — {}{}",
                    key.name,
                    key.value_type.display_name(),
                    key.description,
                    required,
                ));
            }
        }

        if section.is_free_form {
            content.push_str("\n\nThis section accepts arbitrary key-value pairs.");
        }

        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        });
    }

    // Case 2: Hovering over a key
    let trimmed = line_str.trim();
    if let Some(eq_pos) = trimmed.find('=') {
        let key_name = trimmed[..eq_pos].trim();
        let col = position.character as usize;

        // Only show hover if cursor is on the key part (left of `=`)
        let key_start = line_str.find(key_name).unwrap_or(0);
        let key_end = key_start + key_name.len();
        if col > key_end {
            return None;
        }

        // Find the current section
        let section_name = current_section(text, position.line)?;
        let base_section = section_name.split('.').next().unwrap_or(&section_name);

        let key_def = schema::find_key(base_section, key_name)?;

        let required = if key_def.required {
            "**Required**"
        } else {
            "Optional"
        };

        let mut content = format!(
            "## `{}.{}`\n\n{}\n\n- **Type**: `{}`\n- **Status**: {}",
            base_section,
            key_def.name,
            key_def.description,
            key_def.value_type.display_name(),
            required,
        );

        if !key_def.known_values.is_empty() {
            content.push_str(&format!(
                "\n- **Allowed values**: {}",
                key_def
                    .known_values
                    .iter()
                    .map(|v| format!("`{}`", v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        });
    }

    None
}

/// Parse a TOML section header and return the section name.
fn parse_section_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
        let inner = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
        Some(inner.to_string())
    } else if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.starts_with("[[") {
        let inner = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
        Some(inner.to_string())
    } else {
        None
    }
}

/// Find which section the cursor is inside.
fn current_section(text: &str, line: u32) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    for l in (0..=line as usize).rev() {
        if let Some(line_text) = lines.get(l) {
            if let Some(name) = parse_section_header(line_text.trim()) {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hover_on_section_header() {
        let text = "[project]\nname = \"test\"\n";
        let hover = get_toml_hover(
            text,
            Position {
                line: 0,
                character: 3,
            },
        );
        assert!(hover.is_some());
        let h = hover.unwrap();
        if let HoverContents::Markup(m) = h.contents {
            assert!(m.value.contains("[project]"));
            assert!(m.value.contains("Project metadata"));
        } else {
            panic!("Expected markup content");
        }
    }

    #[test]
    fn test_hover_on_key() {
        let text = "[project]\nname = \"test\"\n";
        let hover = get_toml_hover(
            text,
            Position {
                line: 1,
                character: 1,
            },
        );
        assert!(hover.is_some());
        let h = hover.unwrap();
        if let HoverContents::Markup(m) = h.contents {
            assert!(m.value.contains("project.name"));
            assert!(m.value.contains("Required"));
        } else {
            panic!("Expected markup content");
        }
    }

    #[test]
    fn test_hover_on_value_returns_none() {
        let text = "[project]\nname = \"test\"\n";
        // Position on the value side (after =)
        let hover = get_toml_hover(
            text,
            Position {
                line: 1,
                character: 10,
            },
        );
        assert!(hover.is_none());
    }

    #[test]
    fn test_hover_on_build_target() {
        let text = "[build]\ntarget = \"bytecode\"\n";
        let hover = get_toml_hover(
            text,
            Position {
                line: 1,
                character: 2,
            },
        );
        assert!(hover.is_some());
        let h = hover.unwrap();
        if let HoverContents::Markup(m) = h.contents {
            assert!(m.value.contains("build.target"));
            assert!(m.value.contains("bytecode"));
            assert!(m.value.contains("native"));
        } else {
            panic!("Expected markup content");
        }
    }

    #[test]
    fn test_hover_on_array_table_header() {
        let text = "[[extensions]]\nname = \"duckdb\"\n";
        let hover = get_toml_hover(
            text,
            Position {
                line: 0,
                character: 5,
            },
        );
        assert!(hover.is_some());
        let h = hover.unwrap();
        if let HoverContents::Markup(m) = h.contents {
            assert!(m.value.contains("[extensions]"));
            assert!(m.value.contains("Extension module"));
        } else {
            panic!("Expected markup content");
        }
    }

    #[test]
    fn test_hover_on_unknown_section() {
        let text = "[unknown]\nfoo = \"bar\"\n";
        let hover = get_toml_hover(
            text,
            Position {
                line: 0,
                character: 3,
            },
        );
        assert!(hover.is_none());
    }

    #[test]
    fn test_hover_on_optional_key() {
        let text = "[project]\nlicense = \"MIT\"\n";
        let hover = get_toml_hover(
            text,
            Position {
                line: 1,
                character: 2,
            },
        );
        assert!(hover.is_some());
        let h = hover.unwrap();
        if let HoverContents::Markup(m) = h.contents {
            assert!(m.value.contains("Optional"));
            assert!(!m.value.contains("**Required**"));
        } else {
            panic!("Expected markup content");
        }
    }
}
