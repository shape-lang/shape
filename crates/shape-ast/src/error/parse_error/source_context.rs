//! Source context rendering types for error display

use crate::error::SourceLocation;

/// Source context for rendering
#[derive(Debug, Clone, Default)]
pub struct SourceContext {
    /// Lines around the error (typically 1-3 lines before, error line, 1-3 after)
    pub lines: Vec<SourceLine>,
    /// Index of the error line in the lines vector
    pub error_line_index: usize,
}

impl SourceContext {
    pub fn new(lines: Vec<SourceLine>, error_line_index: usize) -> Self {
        Self {
            lines,
            error_line_index,
        }
    }

    /// Build source context from source text and location
    pub fn from_source(
        source: &str,
        location: &SourceLocation,
        span_end: Option<(usize, usize)>,
    ) -> Self {
        let lines: Vec<&str> = source.lines().collect();
        let error_line_idx = location.line.saturating_sub(1);

        // Get context lines (2 before, error line, 2 after)
        let start_idx = error_line_idx.saturating_sub(2);
        let end_idx = (error_line_idx + 3).min(lines.len());

        let context_lines: Vec<SourceLine> = (start_idx..end_idx)
            .map(|i| {
                let content = lines.get(i).unwrap_or(&"").to_string();
                let highlights = if i == error_line_idx {
                    let end_col = span_end
                        .filter(|(el, _)| *el == location.line)
                        .map(|(_, ec)| ec)
                        .or(location.length.map(|l| location.column + l))
                        .unwrap_or(location.column + 1);

                    vec![Highlight {
                        start: location.column,
                        end: end_col,
                        style: HighlightStyle::Primary,
                        label: None,
                    }]
                } else {
                    vec![]
                };

                SourceLine {
                    number: i + 1,
                    content,
                    highlights,
                }
            })
            .collect();

        Self {
            lines: context_lines,
            error_line_index: error_line_idx.saturating_sub(start_idx),
        }
    }
}

/// A single source line with metadata
#[derive(Debug, Clone)]
pub struct SourceLine {
    /// Line number (1-based)
    pub number: usize,
    /// The line content
    pub content: String,
    /// Highlights/underlines on this line
    pub highlights: Vec<Highlight>,
}

/// A highlight/underline on a source line
#[derive(Debug, Clone)]
pub struct Highlight {
    /// Start column (1-based)
    pub start: usize,
    /// End column (exclusive, 1-based)
    pub end: usize,
    /// Style of highlight
    pub style: HighlightStyle,
    /// Optional label
    pub label: Option<String>,
}

impl Highlight {
    pub fn primary(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            style: HighlightStyle::Primary,
            label: None,
        }
    }

    pub fn secondary(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            style: HighlightStyle::Secondary,
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HighlightStyle {
    Primary,    // Main error location (^^^)
    Secondary,  // Related location (---)
    Suggestion, // Where a fix would go
}
