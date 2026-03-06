//! Unified wire-value rendering with extensible adapters.
//!
//! The default renderer handles AnyError values and chooses ANSI/plain output
//! based on terminal capabilities.

use crate::{
    WireValue, render_any_error_ansi, render_any_error_html, render_any_error_plain,
    render_any_error_terminal,
};

/// Output capabilities for terminal rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalRenderCaps {
    pub ansi: bool,
}

impl TerminalRenderCaps {
    pub fn detect() -> Self {
        Self {
            ansi: terminal_supports_ansi(),
        }
    }
}

/// Adapter trait for rendering specific wire-value families (errors/content/etc).
///
/// Implement this trait to extend the unified render path for custom error
/// object shapes.
pub trait WireRenderAdapter: Send + Sync {
    fn render_terminal(&self, value: &WireValue, caps: TerminalRenderCaps) -> Option<String>;

    fn render_html(&self, value: &WireValue) -> Option<String> {
        let _ = value;
        None
    }
}

/// Built-in adapter for AnyError wire objects.
#[derive(Debug, Default, Clone, Copy)]
pub struct AnyErrorWireRenderAdapter;

impl WireRenderAdapter for AnyErrorWireRenderAdapter {
    fn render_terminal(&self, value: &WireValue, caps: TerminalRenderCaps) -> Option<String> {
        if caps.ansi {
            render_any_error_ansi(value)
        } else {
            render_any_error_plain(value)
        }
    }

    fn render_html(&self, value: &WireValue) -> Option<String> {
        render_any_error_html(value)
    }
}

/// Unified renderer that dispatches through registered adapters.
#[derive(Default)]
pub struct WireRenderer {
    adapters: Vec<Box<dyn WireRenderAdapter>>,
}

impl WireRenderer {
    pub fn with_default_adapters() -> Self {
        let mut renderer = Self::default();
        renderer.adapters.push(Box::new(AnyErrorWireRenderAdapter));
        renderer
    }

    pub fn register_adapter<A: WireRenderAdapter + 'static>(&mut self, adapter: A) {
        self.adapters.push(Box::new(adapter));
    }

    pub fn render_terminal(&self, value: &WireValue) -> Option<String> {
        let caps = TerminalRenderCaps::detect();
        for adapter in &self.adapters {
            if let Some(rendered) = adapter.render_terminal(value, caps) {
                return Some(rendered);
            }
        }
        None
    }

    pub fn render_html(&self, value: &WireValue) -> Option<String> {
        for adapter in &self.adapters {
            if let Some(rendered) = adapter.render_html(value) {
                return Some(rendered);
            }
        }
        None
    }
}

/// Render a wire value to terminal text using built-in adapters.
pub fn render_wire_terminal(value: &WireValue) -> Option<String> {
    // Keep backward-compatible behavior for AnyError while routing through
    // adapter-based dispatch for extensibility.
    if let Some(rendered) = render_any_error_terminal(value) {
        return Some(rendered);
    }
    WireRenderer::with_default_adapters().render_terminal(value)
}

/// Render a wire value to HTML using built-in adapters.
pub fn render_wire_html(value: &WireValue) -> Option<String> {
    WireRenderer::with_default_adapters().render_html(value)
}

fn terminal_supports_ansi() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }

    if std::env::var("CLICOLOR").ok().as_deref() == Some("0") {
        return false;
    }

    if std::env::var_os("FORCE_COLOR").is_some()
        || std::env::var("CLICOLOR_FORCE")
            .map(|v| v != "0")
            .unwrap_or(false)
    {
        return true;
    }

    matches!(std::env::var("TERM"), Ok(term) if !term.is_empty() && term != "dumb")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Debug, Default, Clone, Copy)]
    struct CustomAdapter;

    impl WireRenderAdapter for CustomAdapter {
        fn render_terminal(&self, value: &WireValue, _caps: TerminalRenderCaps) -> Option<String> {
            match value {
                WireValue::Object(obj) => obj
                    .get("kind")
                    .and_then(WireValue::as_str)
                    .filter(|kind| *kind == "CustomError")
                    .map(|_| "custom-rendered".to_string()),
                _ => None,
            }
        }
    }

    #[test]
    fn custom_adapter_extends_terminal_render_path() {
        let mut renderer = WireRenderer::with_default_adapters();
        renderer.register_adapter(CustomAdapter);

        let mut obj = BTreeMap::new();
        obj.insert(
            "kind".to_string(),
            WireValue::String("CustomError".to_string()),
        );
        let value = WireValue::Object(obj);

        let rendered = renderer.render_terminal(&value);
        assert_eq!(rendered.as_deref(), Some("custom-rendered"));
    }

    #[test]
    fn default_renderer_handles_anyerror_html() {
        let mut payload = BTreeMap::new();
        payload.insert(
            "category".to_string(),
            WireValue::String("AnyError".to_string()),
        );
        payload.insert("message".to_string(), WireValue::String("boom".to_string()));
        payload.insert("payload".to_string(), WireValue::String("boom".to_string()));
        payload.insert("trace_info".to_string(), WireValue::Null);
        payload.insert("cause".to_string(), WireValue::Null);

        let html = render_wire_html(&WireValue::Object(payload)).expect("expected html");
        assert!(html.contains("shape-error"));
        assert!(html.contains("boom"));
    }
}
