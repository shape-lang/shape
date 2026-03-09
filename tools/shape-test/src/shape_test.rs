//! Unified ShapeTest fluent builder for LSP + runtime assertions.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tower_lsp_server::ls_types::{
    CodeActionOrCommand, CompletionItem, Diagnostic, FormattingOptions, Hover, HoverContents,
    MarkupContent, Position, Range, Uri,
};

use shape_lsp::context::CompletionContext;
use shape_lsp::diagnostics::error_to_diagnostic;
use shape_lsp::inlay_hints::InlayHintConfig;

use shape_runtime::initialize_shared_runtime;
use shape_runtime::output_adapter::OutputAdapter;
use shape_runtime::engine::ShapeEngine;
use shape_vm::BytecodeExecutor;
use shape_value::PrintResult;

// ---------------------------------------------------------------------------
// Capture adapter — shared output buffer readable after execution
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct CaptureAdapter {
    lines: Arc<Mutex<Vec<String>>>,
}

impl CaptureAdapter {
    fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        (
            CaptureAdapter {
                lines: lines.clone(),
            },
            lines,
        )
    }
}

impl OutputAdapter for CaptureAdapter {
    fn print(&mut self, result: PrintResult) -> shape_value::ValueWord {
        self.lines.lock().unwrap().push(result.rendered);
        shape_value::ValueWord::none()
    }

    fn clone_box(&self) -> Box<dyn OutputAdapter> {
        Box::new(self.clone())
    }
}

// Safety: Arc<Mutex> is Send + Sync
unsafe impl Send for CaptureAdapter {}
unsafe impl Sync for CaptureAdapter {}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

/// Create a `Position` (0-indexed line and character).
pub fn pos(line: u32, character: u32) -> Position {
    Position { line, character }
}

/// Create a `Range` from four coordinates.
pub fn range(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Range {
    Range {
        start: Position {
            line: start_line,
            character: start_char,
        },
        end: Position {
            line: end_line,
            character: end_char,
        },
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

pub struct ShapeTest {
    text: String,
    position: Position,
    selected_range: Option<Range>,
    use_stdlib: bool,
    snapshot_dir: Option<tempfile::TempDir>,
}

impl ShapeTest {
    /// Create a new test with the given Shape source code.
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            position: Position {
                line: 0,
                character: 0,
            },
            selected_range: None,
            use_stdlib: false,
            snapshot_dir: None,
        }
    }

    /// Enable stdlib loading for runtime assertions.
    pub fn with_stdlib(mut self) -> Self {
        self.use_stdlib = true;
        self
    }

    /// Enable snapshot support with a temporary store directory.
    pub fn with_snapshots(mut self) -> Self {
        self.snapshot_dir = Some(tempfile::tempdir().unwrap());
        self
    }

    /// Set the cursor position for subsequent assertions.
    pub fn at(mut self, position: Position) -> Self {
        self.position = position;
        self
    }

    /// Set a range for range-based assertions (code actions, etc.).
    pub fn in_range(mut self, range: Range) -> Self {
        self.selected_range = Some(range);
        self
    }

    // -- internal helpers ---------------------------------------------------

    fn uri(&self) -> Uri {
        Uri::from_file_path("/test.shape").unwrap()
    }

    fn format_options(&self) -> FormattingOptions {
        FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        }
    }

    fn full_range(&self) -> Range {
        let lines: Vec<&str> = self.text.lines().collect();
        let last_line = if lines.is_empty() { 0 } else { lines.len() - 1 };
        let last_char = lines.last().map_or(0, |l| l.len());
        Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: last_line as u32,
                character: last_char as u32,
            },
        }
    }

    fn get_hover(&self) -> Option<Hover> {
        shape_lsp::hover::get_hover(&self.text, self.position, None, None, None)
    }

    fn get_completions(&self) -> Vec<CompletionItem> {
        let (items, _, _) =
            shape_lsp::completion::get_completions(&self.text, self.position, &[], &HashMap::new());
        items
    }

    fn completion_labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    fn extract_hover_text(h: &Hover) -> String {
        match &h.contents {
            HoverContents::Markup(MarkupContent { value, .. }) => value.clone(),
            _ => panic!("Expected Markup hover contents"),
        }
    }

    fn collect_semantic_diagnostics(&self) -> Vec<Diagnostic> {
        let program = match shape_ast::parser::parse_program(&self.text) {
            Ok(program) => program,
            Err(err) => return error_to_diagnostic(&err),
        };
        shape_lsp::analysis::analyze_program_semantics(&program, &self.text, None, None, None)
    }

    // -- Runtime helpers ----------------------------------------------------

    fn eval_with_output(&self) -> Result<(serde_json::Value, Vec<String>), String> {
        let _ = initialize_shared_runtime();
        // Enter the shared Tokio runtime so that Handle::current() works
        // for async module exports (e.g. http).
        let handle = shape_runtime::get_runtime_handle().map_err(|e| e.to_string())?;
        let _guard = handle.enter();

        let mut engine = ShapeEngine::new().map_err(|e| e.to_string())?;
        if self.use_stdlib {
            engine.load_stdlib().map_err(|e| e.to_string())?;
        }

        // Enable snapshot store if configured
        if let Some(dir) = &self.snapshot_dir {
            let store = shape_runtime::snapshot::SnapshotStore::new(dir.path())
                .map_err(|e| e.to_string())?;
            engine.enable_snapshot_store(store);
        }

        // Install capture adapter to collect print output
        let (adapter, captured_lines) = CaptureAdapter::new();
        if let Some(ctx) = engine.runtime.persistent_context_mut() {
            ctx.set_output_adapter(Box::new(adapter));
        }

        let mut executor = BytecodeExecutor::new();
        let result = engine
            .execute(&mut executor, &self.text)
            .map_err(|e| e.to_string())?;
        let value = serde_json::to_value(&result.value).map_err(|e| e.to_string())?;
        let output = captured_lines.lock().unwrap().clone();
        Ok((value, output))
    }

    fn eval(&self) -> Result<serde_json::Value, String> {
        self.eval_with_output().map(|(val, _)| val)
    }

    fn extract_number(val: &serde_json::Value) -> f64 {
        match val {
            serde_json::Value::Number(n) => n.as_f64().unwrap(),
            serde_json::Value::Object(map) if map.contains_key("Integer") => {
                match &map["Integer"] {
                    serde_json::Value::Number(n) => n.as_f64().unwrap(),
                    other => panic!("Expected number in Integer, got: {:?}", other),
                }
            }
            serde_json::Value::Object(map) if map.contains_key("Number") => match &map["Number"] {
                serde_json::Value::Number(n) => n.as_f64().unwrap(),
                other => panic!("Expected number in Number, got: {:?}", other),
            },
            other => panic!("Expected number, got: {:?}", other),
        }
    }

    fn extract_bool(val: &serde_json::Value) -> bool {
        match val {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::Object(map) if map.contains_key("Bool") => match &map["Bool"] {
                serde_json::Value::Bool(b) => *b,
                other => panic!("Expected bool in Object, got: {:?}", other),
            },
            other => panic!("Expected bool, got: {:?}", other),
        }
    }

    fn extract_string(val: &serde_json::Value) -> String {
        match val {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(map) if map.contains_key("String") => match &map["String"] {
                serde_json::Value::String(s) => s.clone(),
                other => panic!("Expected string in Object, got: {:?}", other),
            },
            other => panic!("Expected string, got: {:?}", other),
        }
    }

    // =====================================================================
    // LSP assertions
    // =====================================================================

    // -- Hover assertions ---------------------------------------------------

    /// Assert hover at current position contains `expected` substring.
    pub fn expect_hover_contains(self, expected: &str) -> Self {
        let h = self.get_hover();
        assert!(
            h.is_some(),
            "Expected hover at ({}, {})",
            self.position.line,
            self.position.character
        );
        let text = Self::extract_hover_text(&h.unwrap());
        assert!(
            text.contains(expected),
            "Hover should contain '{}', got:\n{}",
            expected,
            text
        );
        self
    }

    /// Assert no hover at current position.
    pub fn expect_no_hover(self) -> Self {
        let h = self.get_hover();
        assert!(
            h.is_none(),
            "Expected no hover at ({}, {})",
            self.position.line,
            self.position.character
        );
        self
    }

    /// Assert hover exists at current position.
    pub fn expect_hover_exists(self) -> Self {
        let h = self.get_hover();
        assert!(
            h.is_some(),
            "Expected hover at ({}, {})",
            self.position.line,
            self.position.character
        );
        self
    }

    /// Assert hover at current position does NOT contain `unexpected` substring.
    pub fn expect_hover_not_contains(self, unexpected: &str) -> Self {
        let h = self.get_hover();
        assert!(
            h.is_some(),
            "Expected hover at ({}, {})",
            self.position.line,
            self.position.character
        );
        let text = Self::extract_hover_text(&h.unwrap());
        assert!(
            !text.contains(unexpected),
            "Hover should not contain '{}', got:\n{}",
            unexpected,
            text
        );
        self
    }

    // -- Completion assertions ----------------------------------------------

    /// Assert completions at current position include `label`.
    pub fn expect_completion(self, label: &str) -> Self {
        let items = self.get_completions();
        assert!(
            items.iter().any(|i| i.label == label),
            "Expected completion '{}' in {:?}",
            label,
            Self::completion_labels(&items)
        );
        self
    }

    /// Assert completions at current position do NOT include `label`.
    pub fn expect_no_completion(self, label: &str) -> Self {
        let items = self.get_completions();
        assert!(
            !items.iter().any(|i| i.label == label),
            "Did not expect completion '{}' but found it",
            label
        );
        self
    }

    /// Assert completions at current position are not empty.
    pub fn expect_completions_not_empty(self) -> Self {
        let items = self.get_completions();
        assert!(
            !items.is_empty(),
            "Expected non-empty completions at ({}, {})",
            self.position.line,
            self.position.character
        );
        self
    }

    /// Assert completions contain any of the given labels.
    pub fn expect_completion_any_of(self, labels: &[&str]) -> Self {
        let items = self.get_completions();
        let found = Self::completion_labels(&items);
        assert!(
            labels.iter().any(|l| found.contains(l)),
            "Expected any of {:?} in completions {:?}",
            labels,
            found
        );
        self
    }

    // -- Definition & References assertions ---------------------------------

    /// Assert go-to-definition at current position returns a result.
    pub fn expect_definition(self) -> Self {
        let uri = self.uri();
        let def = shape_lsp::definition::get_definition(
            &self.text,
            self.position,
            &uri,
            None,
            None,
            None,
        );
        assert!(
            def.is_some(),
            "Expected definition at ({}, {})",
            self.position.line,
            self.position.character
        );
        self
    }

    /// Assert find-references at current position returns at least `min_count` results.
    pub fn expect_references_min(self, min_count: usize) -> Self {
        let uri = self.uri();
        let refs = shape_lsp::definition::get_references(&self.text, self.position, &uri);
        assert!(refs.is_some(), "Expected references result");
        let refs = refs.unwrap();
        assert!(
            refs.len() >= min_count,
            "Expected at least {} references, got {}",
            min_count,
            refs.len()
        );
        self
    }

    // -- Signature Help assertions ------------------------------------------

    /// Assert signature help at current position exists and has signatures.
    pub fn expect_signature_help(self) -> Self {
        let sig = shape_lsp::signature_help::get_signature_help(&self.text, self.position);
        assert!(
            sig.is_some(),
            "Expected signature help at ({}, {})",
            self.position.line,
            self.position.character
        );
        assert!(
            !sig.unwrap().signatures.is_empty(),
            "Expected at least one signature"
        );
        self
    }

    /// Assert active parameter >= `min_param` (0-indexed).
    pub fn expect_active_parameter_min(self, min_param: u32) -> Self {
        let sig = shape_lsp::signature_help::get_signature_help(&self.text, self.position);
        if let Some(sig) = sig {
            if let Some(active) = sig.active_parameter {
                assert!(
                    active >= min_param,
                    "Expected active parameter >= {}, got {}",
                    min_param,
                    active
                );
            }
        }
        self
    }

    /// Assert signature help exists if available (no crash), optionally with signatures.
    pub fn expect_signature_help_if_available(self) -> Self {
        let sig = shape_lsp::signature_help::get_signature_help(&self.text, self.position);
        if let Some(sig) = sig {
            assert!(
                !sig.signatures.is_empty(),
                "If signature help is provided, should have signatures"
            );
        }
        self
    }

    // -- Context assertions -------------------------------------------------

    /// Assert completion context at current position is General.
    pub fn expect_context_general(self) -> Self {
        let ctx = shape_lsp::context::analyze_context(&self.text, self.position);
        assert!(
            matches!(ctx, CompletionContext::General),
            "Expected General context, got: {:?}",
            ctx
        );
        self
    }

    /// Assert completion context is PropertyAccess.
    pub fn expect_context_property_access(self) -> Self {
        let ctx = shape_lsp::context::analyze_context(&self.text, self.position);
        assert!(
            matches!(ctx, CompletionContext::PropertyAccess { .. }),
            "Expected PropertyAccess context, got: {:?}",
            ctx
        );
        self
    }

    /// Assert completion context is ImportModule.
    pub fn expect_context_import_module(self) -> Self {
        let ctx = shape_lsp::context::analyze_context(&self.text, self.position);
        assert!(
            matches!(ctx, CompletionContext::ImportModule),
            "Expected ImportModule context, got: {:?}",
            ctx
        );
        self
    }

    // -- Formatting assertions ----------------------------------------------

    /// Assert formatting produces output that contains `content`.
    pub fn expect_format_preserves(self, content: &str) -> Self {
        let edits = shape_lsp::formatting::format_document(&self.text, &self.format_options());
        if !edits.is_empty() {
            let result = &edits[0].new_text;
            assert!(
                result.contains(content),
                "Formatted output should contain '{}', got:\n{}",
                content,
                result
            );
        }
        self
    }

    /// Assert formatting produces indented output.
    pub fn expect_format_has_indentation(self) -> Self {
        let edits = shape_lsp::formatting::format_document(&self.text, &self.format_options());
        if !edits.is_empty() {
            let result = &edits[0].new_text;
            let has_indent = result.lines().any(|l| l.starts_with("    "));
            assert!(has_indent, "Expected indentation, got:\n{}", result);
        }
        self
    }

    /// Assert format-on-type at current position with trigger char doesn't crash.
    pub fn expect_format_on_type(self, ch: &str) -> Self {
        let _edits = shape_lsp::formatting::format_on_type(
            &self.text,
            self.position,
            ch,
            &self.format_options(),
        );
        self
    }

    // -- Rename assertions --------------------------------------------------

    /// Assert rename at current position produces at least `min_edits` edits.
    pub fn expect_rename_edits(self, new_name: &str, min_edits: usize) -> Self {
        let uri = self.uri();
        let result = shape_lsp::rename::rename(&self.text, &uri, self.position, new_name, None);
        assert!(
            result.is_some(),
            "Expected rename result at ({}, {})",
            self.position.line,
            self.position.character
        );
        let edit = result.unwrap();
        let changes = edit.changes.unwrap();
        let total_edits: usize = changes.values().map(|e| e.len()).sum();
        assert!(
            total_edits >= min_edits,
            "Expected at least {} rename edits, got {}",
            min_edits,
            total_edits
        );
        self
    }

    /// Assert prepare-rename at current position returns None (can't rename).
    pub fn expect_prepare_rename_none(self) -> Self {
        let result = shape_lsp::rename::prepare_rename(&self.text, self.position);
        assert!(
            result.is_none(),
            "Expected prepare_rename to return None at ({}, {})",
            self.position.line,
            self.position.character
        );
        self
    }

    // -- Code Actions assertions --------------------------------------------

    /// Assert code actions for the selected range don't crash.
    pub fn expect_code_actions_ok(self) -> Self {
        let uri = self.uri();
        let r = self.selected_range.unwrap_or(self.full_range());
        let _actions: Vec<CodeActionOrCommand> =
            shape_lsp::code_actions::get_code_actions(&self.text, &uri, r, &[], None, None);
        self
    }

    // -- Code Lens assertions -----------------------------------------------

    /// Assert code lenses are not empty.
    pub fn expect_code_lens_not_empty(self) -> Self {
        let uri = self.uri();
        let lenses = shape_lsp::code_lens::get_code_lenses(&self.text, &uri);
        assert!(!lenses.is_empty(), "Expected at least one code lens");
        self
    }

    /// Assert a code lens exists at the given line.
    pub fn expect_code_lens_at_line(self, line: u32) -> Self {
        let uri = self.uri();
        let lenses = shape_lsp::code_lens::get_code_lenses(&self.text, &uri);
        assert!(
            lenses.iter().any(|l| l.range.start.line == line),
            "Expected code lens at line {}, found lenses at lines: {:?}",
            line,
            lenses
                .iter()
                .map(|l| l.range.start.line)
                .collect::<Vec<_>>()
        );
        self
    }

    /// Assert code lenses have commands (resolved).
    pub fn expect_code_lens_has_commands(self) -> Self {
        let uri = self.uri();
        let lenses = shape_lsp::code_lens::get_code_lenses(&self.text, &uri);
        let has_command = lenses.iter().any(|l| l.command.is_some());
        assert!(has_command, "Expected code lenses with commands");
        self
    }

    // -- Semantic Tokens assertions -----------------------------------------

    /// Assert semantic tokens exist.
    pub fn expect_semantic_tokens(self) -> Self {
        let tokens = shape_lsp::semantic_tokens::get_semantic_tokens(&self.text);
        assert!(tokens.is_some(), "Expected semantic tokens");
        self
    }

    /// Assert semantic tokens have at least `min_count` tokens.
    pub fn expect_semantic_tokens_min(self, min_count: usize) -> Self {
        let tokens = shape_lsp::semantic_tokens::get_semantic_tokens(&self.text);
        assert!(tokens.is_some(), "Expected semantic tokens");
        let data = &tokens.unwrap().data;
        assert!(
            data.len() >= min_count,
            "Expected at least {} tokens, got {}",
            min_count,
            data.len()
        );
        self
    }

    // -- Inlay Hints assertions ---------------------------------------------

    /// Assert inlay hints are not empty (default config).
    pub fn expect_inlay_hints_not_empty(self) -> Self {
        let range = self.full_range();
        let config = InlayHintConfig::default();
        let hints = shape_lsp::inlay_hints::get_inlay_hints(&self.text, range, &config, None);
        assert!(!hints.is_empty(), "Expected inlay hints");
        self
    }

    /// Assert a parameter hint exists at the given position.
    pub fn expect_parameter_hint_at(self, position: Position) -> Self {
        let range = self.full_range();
        let config = InlayHintConfig::default();
        let hints = shape_lsp::inlay_hints::get_inlay_hints(&self.text, range, &config, None);
        let param_hints: Vec<_> = hints
            .iter()
            .filter(|h| h.kind == Some(tower_lsp_server::ls_types::InlayHintKind::PARAMETER))
            .collect();
        assert!(
            !param_hints.is_empty(),
            "Expected at least one parameter hint"
        );
        assert!(
            param_hints.iter().any(|h| h.position == position),
            "Expected parameter hint at ({}, {}), found hints at: {:?}",
            position.line,
            position.character,
            param_hints
                .iter()
                .map(|h| (h.position.line, h.position.character))
                .collect::<Vec<_>>()
        );
        self
    }

    /// Assert a type hint with the given label exists.
    pub fn expect_type_hint_label(self, expected_label: &str) -> Self {
        let range = self.full_range();
        let config = InlayHintConfig::default();
        let hints = shape_lsp::inlay_hints::get_inlay_hints(&self.text, range, &config, None);
        let type_hints: Vec<_> = hints
            .iter()
            .filter(|h| h.kind == Some(tower_lsp_server::ls_types::InlayHintKind::TYPE))
            .collect();
        assert!(!type_hints.is_empty(), "Expected at least one type hint");
        let labels: Vec<String> = type_hints
            .iter()
            .filter_map(|h| match &h.label {
                tower_lsp_server::ls_types::InlayHintLabel::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert!(
            labels.iter().any(|l| l == expected_label),
            "Expected type hint '{}', found: {:?}",
            expected_label,
            labels
        );
        self
    }

    /// Assert no type hint with the given label exists.
    pub fn expect_no_type_hint_label(self, excluded_label: &str) -> Self {
        let range = self.full_range();
        let config = InlayHintConfig::default();
        let hints = shape_lsp::inlay_hints::get_inlay_hints(&self.text, range, &config, None);
        let type_hints: Vec<String> = hints
            .iter()
            .filter(|h| h.kind == Some(tower_lsp_server::ls_types::InlayHintKind::TYPE))
            .filter_map(|h| match &h.label {
                tower_lsp_server::ls_types::InlayHintLabel::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !type_hints.iter().any(|l| l == excluded_label),
            "Did not expect type hint '{}', but found it in: {:?}",
            excluded_label,
            type_hints
        );
        self
    }

    /// Assert inlay hints are empty with a custom config.
    pub fn expect_no_inlay_hints_with_config(self, config: &InlayHintConfig) -> Self {
        let range = self.full_range();
        let hints = shape_lsp::inlay_hints::get_inlay_hints(&self.text, range, config, None);
        assert!(
            hints.is_empty(),
            "Expected no inlay hints, got {}",
            hints.len()
        );
        self
    }

    // -- Document Symbols assertions ----------------------------------------

    /// Assert document symbols exist.
    pub fn expect_document_symbols(self) -> Self {
        let symbols = shape_lsp::document_symbols::get_document_symbols(&self.text);
        assert!(symbols.is_some(), "Expected document symbols");
        self
    }

    /// Assert no document symbols (empty file).
    pub fn expect_no_document_symbols(self) -> Self {
        let symbols = shape_lsp::document_symbols::get_document_symbols(&self.text);
        assert!(symbols.is_none(), "Expected no document symbols");
        self
    }

    /// Assert at least one semantic diagnostic contains the given message substring.
    pub fn expect_semantic_diagnostic_contains(self, expected: &str) -> Self {
        let diagnostics = self.collect_semantic_diagnostics();
        assert!(
            diagnostics
                .iter()
                .any(|diag| diag.message.contains(expected)),
            "Expected semantic diagnostic containing '{}', found: {:?}",
            expected,
            diagnostics
                .iter()
                .map(|diag| diag.message.as_str())
                .collect::<Vec<_>>()
        );
        self
    }

    /// Assert no semantic diagnostic contains the given message substring.
    pub fn expect_no_semantic_diagnostic_contains(self, unexpected: &str) -> Self {
        let diagnostics = self.collect_semantic_diagnostics();
        assert!(
            diagnostics
                .iter()
                .all(|diag| !diag.message.contains(unexpected)),
            "Did not expect semantic diagnostic containing '{}', found: {:?}",
            unexpected,
            diagnostics
                .iter()
                .map(|diag| diag.message.as_str())
                .collect::<Vec<_>>()
        );
        self
    }

    /// Assert semantic diagnostics are empty.
    pub fn expect_no_semantic_diagnostics(self) -> Self {
        let diagnostics = self.collect_semantic_diagnostics();
        assert!(
            diagnostics.is_empty(),
            "Expected no semantic diagnostics, found: {:?}",
            diagnostics
                .iter()
                .map(|diag| (&diag.message, &diag.range))
                .collect::<Vec<_>>()
        );
        self
    }

    /// Assert semantic diagnostics count matches exactly.
    pub fn expect_semantic_diagnostic_count(self, expected: usize) -> Self {
        let diagnostics = self.collect_semantic_diagnostics();
        assert_eq!(
            diagnostics.len(),
            expected,
            "Expected {} semantic diagnostics, found {:?}",
            expected,
            diagnostics
                .iter()
                .map(|diag| (
                    diag.range.start.line,
                    diag.range.start.character,
                    diag.message.as_str()
                ))
                .collect::<Vec<_>>()
        );
        self
    }

    /// Assert semantic diagnostics count for a given start line matches exactly.
    pub fn expect_semantic_diagnostic_count_at_line(self, line: u32, expected: usize) -> Self {
        let diagnostics = self.collect_semantic_diagnostics();
        let count = diagnostics
            .iter()
            .filter(|diag| diag.range.start.line == line)
            .count();
        assert_eq!(
            count,
            expected,
            "Expected {} semantic diagnostics at line {}, found {:?}",
            expected,
            line,
            diagnostics
                .iter()
                .map(|diag| (
                    diag.range.start.line,
                    diag.range.start.character,
                    diag.message.as_str()
                ))
                .collect::<Vec<_>>()
        );
        self
    }

    /// Assert at least one semantic diagnostic contains `expected` and starts on `line` (0-based).
    pub fn expect_semantic_diagnostic_at_line_contains(self, line: u32, expected: &str) -> Self {
        let diagnostics = self.collect_semantic_diagnostics();
        assert!(
            diagnostics
                .iter()
                .any(|diag| { diag.range.start.line == line && diag.message.contains(expected) }),
            "Expected semantic diagnostic containing '{}' at line {}, found: {:?}",
            expected,
            line,
            diagnostics
                .iter()
                .map(|diag| (
                    diag.range.start.line,
                    diag.range.start.character,
                    diag.message.as_str()
                ))
                .collect::<Vec<_>>()
        );
        self
    }

    /// Assert at least one semantic diagnostic contains `expected` and starts at `line:character` (0-based).
    pub fn expect_semantic_diagnostic_at_position_contains(
        self,
        line: u32,
        character: u32,
        expected: &str,
    ) -> Self {
        let diagnostics = self.collect_semantic_diagnostics();
        assert!(
            diagnostics.iter().any(|diag| {
                diag.range.start.line == line
                    && diag.range.start.character == character
                    && diag.message.contains(expected)
            }),
            "Expected semantic diagnostic containing '{}' at {}:{}, found: {:?}",
            expected,
            line,
            character,
            diagnostics
                .iter()
                .map(|diag| (
                    diag.range.start.line,
                    diag.range.start.character,
                    diag.message.as_str()
                ))
                .collect::<Vec<_>>()
        );
        self
    }

    // =====================================================================
    // Runtime assertions
    // =====================================================================

    /// Assert the code executes without error.
    pub fn expect_run_ok(self) -> Self {
        let result = self.eval();
        assert!(
            result.is_ok(),
            "Expected run ok, got error: {:?}",
            result.err()
        );
        self
    }

    /// Assert the result is None/null.
    pub fn expect_none(self) -> Self {
        let result = self.eval();
        assert!(
            result.is_ok(),
            "Expected run ok, got error: {:?}",
            result.err()
        );
        let val = result.unwrap();
        let is_none = val.is_null()
            || val == serde_json::Value::String("Null".to_string())
            || val == serde_json::Value::String("None".to_string());
        assert!(is_none, "Expected None/null, got: {:?}", val);
        self
    }

    /// Assert the code produces a runtime error.
    pub fn expect_run_err(self) -> Self {
        let result = self.eval();
        assert!(
            result.is_err(),
            "Expected run error, but got: {:?}",
            result.ok()
        );
        self
    }

    /// Assert the code produces a runtime error containing `msg`.
    pub fn expect_run_err_contains(self, msg: &str) -> Self {
        let result = self.eval();
        assert!(
            result.is_err(),
            "Expected run error, but got: {:?}",
            result.ok()
        );
        let err = result.unwrap_err();
        assert!(
            err.contains(msg),
            "Error should contain '{}', got: {}",
            msg,
            err
        );
        self
    }

    /// Assert the result is the expected number.
    pub fn expect_number(self, expected: f64) -> Self {
        let result = self.eval();
        assert!(
            result.is_ok(),
            "Expected run ok, got error: {:?}",
            result.err()
        );
        let val = result.unwrap();
        let num = Self::extract_number(&val);
        assert!(
            (num - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            num
        );
        self
    }

    /// Assert the result is the expected bool.
    pub fn expect_bool(self, expected: bool) -> Self {
        let result = self.eval();
        assert!(
            result.is_ok(),
            "Expected run ok, got error: {:?}",
            result.err()
        );
        let val = result.unwrap();
        let b = Self::extract_bool(&val);
        assert_eq!(b, expected, "Expected {}, got {}", expected, b);
        self
    }

    /// Assert the result is the expected string.
    pub fn expect_string(self, expected: &str) -> Self {
        let result = self.eval();
        assert!(
            result.is_ok(),
            "Expected run ok, got error: {:?}",
            result.err()
        );
        let val = result.unwrap();
        let s = Self::extract_string(&val);
        assert_eq!(s, expected, "Expected '{}', got '{}'", expected, s);
        self
    }

    /// Assert the code parses without error (no execution).
    pub fn expect_parse_ok(self) -> Self {
        let result = shape_ast::parse_program(&self.text);
        assert!(
            result.is_ok(),
            "Expected parse ok, got error: {:?}",
            result.err()
        );
        self
    }

    /// Assert the code fails to parse.
    pub fn expect_parse_err(self) -> Self {
        let result = shape_ast::parse_program(&self.text);
        assert!(result.is_err(), "Expected parse error, but parsed ok");
        self
    }

    // -- Output assertions --------------------------------------------------

    /// Assert the captured stdout matches `expected` exactly (multiline).
    /// Each `print()` call produces one line. Lines are joined with `\n`.
    pub fn expect_output(self, expected: &str) -> Self {
        let result = self.eval_with_output();
        assert!(
            result.is_ok(),
            "Expected run ok, got error: {:?}",
            result.as_ref().err()
        );
        let (_, lines) = result.unwrap();
        let actual = lines.join("\n");
        let expected = expected.trim_end_matches('\n');
        assert_eq!(
            actual, expected,
            "Output mismatch.\nExpected:\n{}\nActual:\n{}",
            expected, actual
        );
        self
    }

    /// Assert the captured stdout contains `substr`.
    pub fn expect_output_contains(self, substr: &str) -> Self {
        let result = self.eval_with_output();
        assert!(
            result.is_ok(),
            "Expected run ok, got error: {:?}",
            result.as_ref().err()
        );
        let (_, lines) = result.unwrap();
        let actual = lines.join("\n");
        assert!(
            actual.contains(substr),
            "Output should contain '{}', got:\n{}",
            substr,
            actual
        );
        self
    }
}
