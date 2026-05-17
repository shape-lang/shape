//! Output adapter trait for handling print() results.
//!
//! Different execution modes (script vs REPL vs notebook) control how
//! print() output is surfaced.
//!
//! Per ADR-006 §2.7.4 (output adapter ruling), [`PrintResult`] and
//! [`PrintSpan`] live in `shape-runtime` (see [`crate::print_result`])
//! and `OutputAdapter::print` returns a [`KindedSlot`] — the
//! GENERIC_CARRIER single-value shape (§2.7.1.2). Adapters that have no
//! sensible heap value to return (script mode) return
//! `KindedSlot::none()`; adapters that surface the structured output
//! (REPL) attach the `PrintResult` to a typed-object heap value before
//! returning. In Phase 1.B the `from_print_result` heap-construction is
//! deferred — REPL mode currently returns `none()` and a follow-up wires
//! `PrintResult` through a typed schema.

use crate::print_result::PrintResult;
use shape_value::KindedSlot;
use std::sync::{Arc, Mutex};

/// Trait for handling print() output.
///
/// Different execution modes can provide different adapters:
/// - Scripts: [`StdoutAdapter`] (print and discard spans)
/// - REPL: [`ReplAdapter`] (preserve spans for reformatting)
/// - Tests: [`MockAdapter`] (capture output)
/// - Hosts (server / notebook): [`SharedCaptureAdapter`]
pub trait OutputAdapter: Send + Sync {
    /// Handle print() output.
    ///
    /// Returns the value that print() yields. Scripts return
    /// [`KindedSlot::none()`]; REPL adapters MAY surface the
    /// `PrintResult` via a future typed-schema heap value but currently
    /// also return `none()` until that schema lands.
    fn print(&mut self, result: PrintResult) -> KindedSlot;

    /// Handle Content HTML from printing a Content value.
    /// Default implementation does nothing (terminal adapters don't need HTML).
    fn print_content_html(&mut self, _html: String) {}

    /// Clone the adapter (for trait object cloning)
    fn clone_box(&self) -> Box<dyn OutputAdapter>;
}

// Implement Clone for Box<dyn OutputAdapter>
impl Clone for Box<dyn OutputAdapter> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Standard output adapter — prints to stdout and discards spans.
///
/// Used for script execution where spans aren't needed.
#[derive(Debug, Clone)]
pub struct StdoutAdapter;

impl OutputAdapter for StdoutAdapter {
    fn print(&mut self, result: PrintResult) -> KindedSlot {
        println!("{}", result.rendered);
        KindedSlot::none()
    }

    fn clone_box(&self) -> Box<dyn OutputAdapter> {
        Box::new(self.clone())
    }
}

/// REPL output adapter — preserves spans for reformatting.
///
/// Pre-bulldozer this returned a `ValueWord::from_print_result(..)`
/// pointing at a `RareHeapData::PrintResult` heap arm. Post-ADR-006 that
/// path is gone; the typed-schema replacement (`PrintResult` as a
/// `HeapValue::TypedObject` with a runtime-registered schema) is a
/// follow-up. For now the REPL adapter consumes the rendered text and
/// returns `none()`; the structured `PrintResult` is dropped until the
/// schema lands.
#[derive(Debug, Clone)]
pub struct ReplAdapter;

impl OutputAdapter for ReplAdapter {
    fn print(&mut self, _result: PrintResult) -> KindedSlot {
        KindedSlot::none()
    }

    fn clone_box(&self) -> Box<dyn OutputAdapter> {
        Box::new(self.clone())
    }
}

/// Mock adapter for testing — captures output without printing.
#[derive(Debug, Clone, Default)]
pub struct MockAdapter {
    /// Captured print outputs
    pub captured: Vec<String>,
}

impl MockAdapter {
    pub fn new() -> Self {
        MockAdapter {
            captured: Vec::new(),
        }
    }

    /// Get all captured output
    pub fn output(&self) -> Vec<String> {
        self.captured.clone()
    }

    /// Clear captured output
    pub fn clear(&mut self) {
        self.captured.clear();
    }
}

impl OutputAdapter for MockAdapter {
    fn print(&mut self, result: PrintResult) -> KindedSlot {
        self.captured.push(result.rendered.clone());
        KindedSlot::none()
    }

    fn clone_box(&self) -> Box<dyn OutputAdapter> {
        Box::new(self.clone())
    }
}

/// Shared capture adapter for host integrations (server/notebook).
///
/// Captures rendered print output into shared state so the host can
/// surface it in API responses without scraping stdout.
/// Also captures Content HTML when Content values are printed.
#[derive(Debug, Clone, Default)]
pub struct SharedCaptureAdapter {
    captured: Arc<Mutex<Vec<String>>>,
    captured_full: Arc<Mutex<Vec<PrintResult>>>,
    content_html: Arc<Mutex<Vec<String>>>,
}

impl SharedCaptureAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all captured output lines.
    pub fn output(&self) -> Vec<String> {
        self.captured
            .lock()
            .map(|v| v.clone())
            .unwrap_or_else(|_| Vec::new())
    }

    /// Clear captured output lines.
    pub fn clear(&self) {
        if let Ok(mut v) = self.captured.lock() {
            v.clear();
        }
    }

    /// Push Content HTML captured from print(content_value).
    pub fn push_content_html(&self, html: String) {
        if let Ok(mut v) = self.content_html.lock() {
            v.push(html);
        }
    }

    /// Get all captured Content HTML fragments.
    pub fn content_html(&self) -> Vec<String> {
        self.content_html
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    /// Get all captured full PrintResults (with spans).
    pub fn print_results(&self) -> Vec<PrintResult> {
        self.captured_full
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
    }
}

impl OutputAdapter for SharedCaptureAdapter {
    fn print(&mut self, result: PrintResult) -> KindedSlot {
        if let Ok(mut v) = self.captured.lock() {
            v.push(result.rendered.clone());
        }
        if let Ok(mut v) = self.captured_full.lock() {
            v.push(result);
        }
        KindedSlot::none()
    }

    fn print_content_html(&mut self, html: String) {
        self.push_content_html(html);
    }

    fn clone_box(&self) -> Box<dyn OutputAdapter> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print_result::PrintSpan;

    fn make_test_result() -> PrintResult {
        PrintResult {
            rendered: "Test output".to_string(),
            spans: vec![PrintSpan::Literal {
                text: "Test output".to_string(),
                start: 0,
                end: 11,
                span_id: "span_1".to_string(),
            }],
        }
    }

    #[test]
    fn test_stdout_adapter_returns_none() {
        let mut adapter = StdoutAdapter;
        let result = make_test_result();
        let returned = adapter.print(result);

        assert_eq!(returned.slot().raw(), 0, "script-mode print returns none");
    }

    #[test]
    fn test_repl_adapter_returns_none_phase1b() {
        // Pre-ADR-006 this returned `ValueWord::from_print_result(..)`. The
        // typed-schema replacement is deferred; current behaviour is to
        // drop the structured payload and return `none()`.
        let mut adapter = ReplAdapter;
        let result = make_test_result();
        let returned = adapter.print(result);
        assert_eq!(returned.slot().raw(), 0);
    }

    #[test]
    fn test_mock_adapter_captures() {
        let mut adapter = MockAdapter::new();

        adapter.print(PrintResult {
            rendered: "Output 1".to_string(),
            spans: vec![],
        });
        adapter.print(PrintResult {
            rendered: "Output 2".to_string(),
            spans: vec![],
        });

        assert_eq!(adapter.output(), vec!["Output 1", "Output 2"]);

        adapter.clear();
        assert_eq!(adapter.output().len(), 0);
    }

    #[test]
    fn test_shared_capture_adapter_captures() {
        let mut adapter = SharedCaptureAdapter::new();

        adapter.print(PrintResult {
            rendered: "Output A".to_string(),
            spans: vec![],
        });
        adapter.print(PrintResult {
            rendered: "Output B".to_string(),
            spans: vec![],
        });

        assert_eq!(adapter.output(), vec!["Output A", "Output B"]);

        adapter.clear();
        assert!(adapter.output().is_empty());
    }
}
