//! Canonical lexical capture-analysis result.
//!
//! [`EnvironmentAnalyzer`](super::EnvironmentAnalyzer) remains the sole
//! resolver for closure captures.  This immutable result adds the exact AST
//! use sites needed by compiler and tooling queries without asking either
//! consumer to repeat lexical name resolution.

use std::collections::{HashMap, HashSet};

use shape_ast::ast::Span;

use super::EnvironmentAnalyzer;

/// Derive the callable/namespace binding token from a parsed call node.
///
/// Both call grammars begin their expression span at this binding token. The
/// derivation is therefore structural arithmetic over the AST node, never a
/// search through source text. An inconsistent or synthetic span has no
/// source location and is omitted.
pub fn callable_binding_span(name: &str, expression_span: Span) -> Option<Span> {
    if expression_span.is_dummy() {
        return None;
    }
    let end = expression_span.start.checked_add(name.len())?;
    (end <= expression_span.end).then(|| Span::new(expression_span.start, end))
}

/// Captures discovered for one function body by the canonical lexical walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureAnalysis {
    captured_vars: Vec<String>,
    mutated_captures: HashSet<String>,
    use_spans: HashMap<String, Vec<Span>>,
}

impl CaptureAnalysis {
    pub(super) fn new(
        mut captured_vars: Vec<String>,
        mutated_captures: HashSet<String>,
        mut use_spans: HashMap<String, Vec<Span>>,
    ) -> Self {
        captured_vars.sort();
        captured_vars.dedup();
        use_spans.retain(|name, _| captured_vars.binary_search(name).is_ok());
        for spans in use_spans.values_mut() {
            spans.sort_by_key(|span| (span.start, span.end));
            spans.dedup();
        }
        Self {
            captured_vars,
            mutated_captures,
            use_spans,
        }
    }

    /// Captured outer bindings, deterministically ordered by source spelling.
    pub fn captured_vars(&self) -> &[String] {
        &self.captured_vars
    }

    /// Captures assigned through by this function body.
    pub fn mutated_captures(&self) -> &HashSet<String> {
        &self.mutated_captures
    }

    /// Exact AST use spans for one captured binding.
    ///
    /// Shadowed locals are absent because the canonical lexical resolver
    /// records a span only after resolving the use across the function
    /// boundary.
    pub fn use_spans(&self, name: &str) -> &[Span] {
        self.use_spans.get(name).map_or(&[], Vec::as_slice)
    }

    /// Compatibility projection for callers that need only names/mutability.
    pub fn into_legacy_parts(self) -> (Vec<String>, HashSet<String>) {
        (self.captured_vars, self.mutated_captures)
    }
}

impl EnvironmentAnalyzer {
    /// Resolve one variable reference and, when it crosses the active
    /// function boundary, retain its exact structural AST span.
    pub(super) fn check_variable_reference_at(&mut self, name: &str, span: Option<Span>) {
        for (level, scope) in self.scope_stack.iter().enumerate().rev() {
            if !scope.contains_key(name) {
                continue;
            }
            if level < self.function_scope_level {
                self.captured_vars.insert(name.to_string(), level);
                if let Some(span) = span.filter(|span| !span.is_dummy()) {
                    self.captured_use_spans
                        .entry(name.to_string())
                        .or_default()
                        .push(span);
                }
            }
            return;
        }
    }

    /// Analyze a function once and return names, mutability, and exact use
    /// sites from the same lexical-resolution pass.
    pub fn analyze_function_captures(
        function: &shape_ast::ast::FunctionDef,
        outer_scope_vars: &[String],
    ) -> CaptureAnalysis {
        let mut analyzer = Self::new();
        for variable in outer_scope_vars {
            analyzer.define_variable(variable);
        }

        analyzer.enter_scope();
        analyzer.function_scope_level = analyzer.scope_stack.len() - 1;
        for parameter in &function.params {
            for name in parameter.get_identifiers() {
                analyzer.define_variable(&name);
            }
        }
        for statement in &function.body {
            analyzer.analyze_statement(statement);
        }

        CaptureAnalysis::new(
            analyzer.get_captured_vars(),
            analyzer.get_mutated_captures(),
            analyzer.captured_use_spans,
        )
    }
}
