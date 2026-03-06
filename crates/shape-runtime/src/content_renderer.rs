//! ContentRenderer trait for rendering ContentNode to various output formats.
//!
//! Implementations can target different output formats:
//! - Terminal (ANSI escape codes)
//! - Plain text (no formatting)
//! - HTML (span/table/svg)
//! - Markdown (GFM tables, fenced code)
//! - JSON (structured tree)

use shape_value::content::ContentNode;

/// Environment context for rendering — terminal width, theme, row limits.
///
/// Renderers that need environment awareness (e.g. terminal column sizing)
/// store a `RenderContext` as a field and use it internally.
#[derive(Debug, Clone)]
pub struct RenderContext {
    /// Max output width in columns (e.g. terminal width), or None for unlimited.
    pub max_width: Option<usize>,
    /// Color theme hint.
    pub theme: Theme,
    /// Max rows to display in tables (overrides ContentTable.max_rows when set).
    pub max_rows: Option<usize>,
    /// Whether the output target supports interactive elements (e.g. ECharts).
    pub interactive: bool,
}

/// Color theme hint for renderers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Theme {
    Dark,
    Light,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            max_width: Some(80),
            theme: Theme::Dark,
            max_rows: Some(50),
            interactive: false,
        }
    }
}

impl RenderContext {
    /// Create a terminal-aware context.
    pub fn terminal() -> Self {
        Self {
            max_width: terminal_width(),
            theme: Theme::Dark,
            max_rows: Some(50),
            interactive: false,
        }
    }

    /// Create an HTML context (unlimited width, interactive).
    pub fn html() -> Self {
        Self {
            max_width: None,
            theme: Theme::Dark,
            max_rows: Some(100),
            interactive: true,
        }
    }
}

/// Try to detect terminal width via COLUMNS env var.
fn terminal_width() -> Option<usize> {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .or(Some(80))
}

/// Describes the capabilities of a renderer.
#[derive(Debug, Clone)]
pub struct RendererCapabilities {
    /// Whether the renderer supports ANSI escape codes.
    pub ansi: bool,
    /// Whether the renderer supports unicode box-drawing characters.
    pub unicode: bool,
    /// Whether the renderer supports color output.
    pub color: bool,
    /// Whether the renderer supports interactive/hyperlink features.
    pub interactive: bool,
}

impl RendererCapabilities {
    /// Full terminal capabilities (ANSI + unicode + color).
    pub fn terminal() -> Self {
        Self {
            ansi: true,
            unicode: true,
            color: true,
            interactive: false,
        }
    }

    /// Plain text only — no ANSI, no unicode, no color.
    pub fn plain() -> Self {
        Self {
            ansi: false,
            unicode: false,
            color: false,
            interactive: false,
        }
    }

    /// HTML capabilities — color via CSS, no ANSI.
    pub fn html() -> Self {
        Self {
            ansi: false,
            unicode: true,
            color: true,
            interactive: true,
        }
    }

    /// Markdown capabilities — limited formatting.
    pub fn markdown() -> Self {
        Self {
            ansi: false,
            unicode: false,
            color: false,
            interactive: false,
        }
    }
}

/// Trait for rendering a ContentNode tree to a string output.
///
/// Implementations should handle all ContentNode variants:
/// Text, Table, Code, Chart, KeyValue, Fragment.
pub trait ContentRenderer: Send + Sync {
    /// Describe what this renderer can handle.
    fn capabilities(&self) -> RendererCapabilities;

    /// Render the content node tree to a string.
    fn render(&self, content: &ContentNode) -> String;
}
