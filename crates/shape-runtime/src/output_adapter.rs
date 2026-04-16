//! Output adapter trait for handling print() results
//!
//! This allows different execution modes (script vs REPL) to control
//! how print() output is handled without heuristics.

use shape_value::{PrintResult, ValueWord, ValueWordExt};
use std::sync::{Arc, Mutex};

/// Trait for handling print() output
///
/// Different execution modes can provide different adapters:
/// - Scripts: StdoutAdapter (print and discard spans)
/// - REPL: ReplAdapter (print and preserve spans for reformatting)
/// - Tests: MockAdapter (capture output)
pub trait OutputAdapter: Send + Sync {
    /// Handle print() output
    ///
    /// # Arguments
    /// * `result` - The PrintResult with rendered string and spans
    ///
    /// # Returns
    /// The value to return from print() (Unit for scripts, PrintResult for REPL)
    fn print(&mut self, result: PrintResult) -> ValueWord;

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

/// Standard output adapter - prints to stdout and discards spans
///
/// Used for script execution where spans aren't needed.
#[derive(Debug, Clone)]
pub struct StdoutAdapter;

impl OutputAdapter for StdoutAdapter {
    fn print(&mut self, result: PrintResult) -> ValueWord {
        // Print the rendered output
        println!("{}", result.rendered);

        // Return None (traditional print() behavior)
        ValueWord::none()
    }

    fn clone_box(&self) -> Box<dyn OutputAdapter> {
        Box::new(self.clone())
    }
}

/// REPL output adapter - prints to stdout and preserves spans
///
/// Used in REPL mode to enable post-execution reformatting with :reformat
#[derive(Debug, Clone)]
pub struct ReplAdapter;

impl OutputAdapter for ReplAdapter {
    fn print(&mut self, result: PrintResult) -> ValueWord {
        // Do NOT print to stdout in REPL mode (let the REPL UI handle display)
        // Return PrintResult with spans for REPL inspection
        ValueWord::from_print_result(result)
    }

    fn clone_box(&self) -> Box<dyn OutputAdapter> {
        Box::new(self.clone())
    }
}

/// Mock adapter for testing - captures output without printing
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
    fn print(&mut self, result: PrintResult) -> ValueWord {
        // Capture instead of printing
        self.captured.push(result.rendered.clone());

        // Return None (traditional behavior)
        ValueWord::none()
    }

    fn clone_box(&self) -> Box<dyn OutputAdapter> {
        Box::new(self.clone())
    }
}

/// Shared capture adapter for host integrations (server/notebook)
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
    fn print(&mut self, result: PrintResult) -> ValueWord {
        if let Ok(mut v) = self.captured.lock() {
            v.push(result.rendered.clone());
        }
        if let Ok(mut v) = self.captured_full.lock() {
            v.push(result);
        }
        ValueWord::none()
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
    use shape_value::PrintSpan;
    use shape_value::heap_value::HeapValue;

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

        assert!(returned.is_none());
    }

    #[test]
    fn test_repl_adapter_preserves_spans() {
        let mut adapter = ReplAdapter;
        let result = make_test_result();
        let returned = adapter.print(result);

        match returned.as_heap_ref().expect("Expected heap value") {
            HeapValue::PrintResult(pr) => {
                assert_eq!(pr.rendered, "Test output");
                assert_eq!(pr.spans.len(), 1);
            }
            other => panic!("Expected PrintResult, got {:?}", other),
        }
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
