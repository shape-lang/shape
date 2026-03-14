//! Diagnostics conversion from Shape errors to LSP diagnostics

use crate::annotation_discovery::AnnotationDiscovery;
use crate::type_inference::unified_metadata;
use crate::util::span_to_range;
use shape_ast::ast::{Annotation, Expr, Item, Literal, Program, Span, Statement};
use shape_ast::error::{
    ErrorNote, ErrorRenderer, ErrorSeverity, ParseErrorKind, ShapeError, SourceLocation,
    StructuredParseError,
};
use tower_lsp_server::ls_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString,
    Position, Range, Uri,
};

/// LSP Error Renderer - converts structured errors to LSP Diagnostics
pub struct LspErrorRenderer {
    /// URI of the document being validated
    uri: Uri,
}

impl LspErrorRenderer {
    pub fn new(uri: Uri) -> Self {
        Self { uri }
    }

    /// Convert a structured error to an LSP diagnostic
    pub fn structured_error_to_diagnostic(&self, error: &StructuredParseError) -> Diagnostic {
        let severity = match error.severity {
            ErrorSeverity::Error => DiagnosticSeverity::ERROR,
            ErrorSeverity::Warning => DiagnosticSeverity::WARNING,
            ErrorSeverity::Info => DiagnosticSeverity::INFORMATION,
            ErrorSeverity::Hint => DiagnosticSeverity::HINT,
        };

        // Convert location to range
        let range = self.structured_location_to_range(error);

        // Build the main message
        let message = format_structured_message(&error.kind, &error.suggestions);

        // Build related information from related locations
        let related_information = if !error.related.is_empty() {
            Some(
                error
                    .related
                    .iter()
                    .map(|rel| DiagnosticRelatedInformation {
                        location: Location {
                            uri: self.uri.clone(),
                            range: self.source_location_to_range(&rel.location),
                        },
                        message: rel.message.clone(),
                    })
                    .collect(),
            )
        } else {
            None
        };

        Diagnostic {
            range,
            severity: Some(severity),
            code: Some(NumberOrString::String(error.code.as_str().to_string())),
            code_description: None,
            source: Some("shape".to_string()),
            message,
            related_information,
            tags: None,
            data: None,
        }
    }

    fn structured_location_to_range(&self, error: &StructuredParseError) -> Range {
        let line = error.location.line.saturating_sub(1) as u32;
        let column = error.location.column.saturating_sub(1) as u32;

        let start = Position {
            line,
            character: column,
        };

        let end = if let Some((end_line, end_col)) = error.span_end {
            Position {
                line: end_line.saturating_sub(1) as u32,
                character: end_col.saturating_sub(1) as u32,
            }
        } else if let Some(len) = error.location.length {
            Position {
                line,
                character: column + len as u32,
            }
        } else {
            // Default to a reasonable span
            Position {
                line,
                character: column + 1,
            }
        };

        Range { start, end }
    }

    fn source_location_to_range(&self, location: &SourceLocation) -> Range {
        let line = location.line.saturating_sub(1) as u32;
        let column = location.column.saturating_sub(1) as u32;

        Range {
            start: Position {
                line,
                character: column,
            },
            end: Position {
                line,
                character: column + location.length.unwrap_or(1) as u32,
            },
        }
    }
}

impl ErrorRenderer for LspErrorRenderer {
    type Output = Vec<Diagnostic>;

    fn render(&self, error: &StructuredParseError) -> Self::Output {
        vec![self.structured_error_to_diagnostic(error)]
    }

    fn render_all(&self, errors: &[StructuredParseError]) -> Self::Output {
        errors
            .iter()
            .map(|e| self.structured_error_to_diagnostic(e))
            .collect()
    }
}

/// Format the error message with suggestions for LSP display
fn format_structured_message(
    kind: &ParseErrorKind,
    suggestions: &[shape_ast::error::Suggestion],
) -> String {
    use shape_ast::error::parse_error::format_error_message;

    let base_message = format_error_message(kind);

    if suggestions.is_empty() {
        return base_message;
    }

    // Add suggestions as hints in the message
    let suggestion_text: Vec<String> = suggestions.iter().map(|s| s.message.clone()).collect();

    if suggestion_text.is_empty() {
        base_message
    } else {
        format!("{}\n\n{}", base_message, suggestion_text.join("\n"))
    }
}

/// Convert Shape errors to LSP diagnostics
pub fn error_to_diagnostic(error: &ShapeError) -> Vec<Diagnostic> {
    error_to_diagnostic_with_uri(error, None)
}

/// Convert Shape errors to LSP diagnostics with optional URI for structured errors
pub fn error_to_diagnostic_with_uri(error: &ShapeError, uri: Option<Uri>) -> Vec<Diagnostic> {
    // Get error code if available
    let error_code = error.error_code().map(|c| c.as_str());

    match error {
        ShapeError::StructuredParse(structured) => {
            // Use the LSP renderer for structured errors if we have a URI
            if let Some(uri) = uri {
                let renderer = LspErrorRenderer::new(uri);
                renderer.render(structured)
            } else {
                // Fallback: convert to a basic diagnostic
                vec![create_diagnostic_with_code(
                    &structured.to_string(),
                    Some(&structured.location),
                    DiagnosticSeverity::ERROR,
                    "shape",
                    Some(structured.code.as_str()),
                )]
            }
        }
        ShapeError::ParseError { message, location } => {
            vec![create_diagnostic_with_code(
                message,
                location.as_ref(),
                DiagnosticSeverity::ERROR,
                "shape",
                error_code,
            )]
        }
        ShapeError::LexError { message, location } => {
            vec![create_diagnostic_with_code(
                message,
                location.as_ref(),
                DiagnosticSeverity::ERROR,
                "shape",
                error_code,
            )]
        }
        ShapeError::SemanticError { message, location } => {
            vec![create_diagnostic_with_code(
                message,
                location.as_ref(),
                DiagnosticSeverity::ERROR,
                "shape",
                error_code,
            )]
        }
        ShapeError::RuntimeError { message, location } => {
            vec![create_diagnostic_with_code(
                message,
                location.as_ref(),
                DiagnosticSeverity::WARNING,
                "shape",
                error_code,
            )]
        }
        ShapeError::TypeError(type_error) => {
            // Type errors may have location information
            vec![create_diagnostic_with_code(
                &type_error.to_string(),
                None,
                DiagnosticSeverity::ERROR,
                "shape",
                error_code,
            )]
        }
        ShapeError::PatternError {
            message,
            pattern_name,
        } => {
            let msg = if let Some(name) = pattern_name {
                format!("Pattern '{}': {}", name, message)
            } else {
                message.clone()
            };
            vec![create_diagnostic_with_code(
                &msg,
                None,
                DiagnosticSeverity::ERROR,
                "shape",
                error_code,
            )]
        }
        ShapeError::DataError {
            message,
            symbol,
            timeframe,
        } => {
            let mut msg = message.clone();
            if let Some(sym) = symbol {
                msg.push_str(&format!(" (symbol: {})", sym));
            }
            if let Some(tf) = timeframe {
                msg.push_str(&format!(" (timeframe: {})", tf));
            }
            vec![create_diagnostic_with_code(
                &msg,
                None,
                DiagnosticSeverity::WARNING,
                "shape",
                error_code,
            )]
        }
        ShapeError::ModuleError {
            message,
            module_path,
        } => {
            let msg = if let Some(path) = module_path {
                format!("{}: {}", path.display(), message)
            } else {
                message.clone()
            };
            vec![create_diagnostic_with_code(
                &msg,
                None,
                DiagnosticSeverity::ERROR,
                "shape",
                error_code,
            )]
        }

        ShapeError::MultiError(errors) => {
            // Flatten MultiError into individual diagnostics
            errors
                .iter()
                .flat_map(|e| error_to_diagnostic_with_uri(e, uri.clone()))
                .collect()
        }

        // Other error types get generic diagnostic
        _ => vec![create_diagnostic_with_code(
            &error.to_string(),
            None,
            DiagnosticSeverity::ERROR,
            "shape",
            error_code,
        )],
    }
}

/// Create a diagnostic from error information
fn create_diagnostic(
    message: &str,
    location: Option<&SourceLocation>,
    severity: DiagnosticSeverity,
    source: &str,
) -> Diagnostic {
    let range = location_to_range(location);

    // Build message with hints
    let full_message = if let Some(loc) = location {
        if !loc.hints.is_empty() {
            let hints = loc
                .hints
                .iter()
                .map(|h| format!("help: {}", h))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{}\n{}", message, hints)
        } else {
            message.to_string()
        }
    } else {
        message.to_string()
    };

    // Build related information from notes and cross-file locations
    let related_information = if let Some(loc) = location {
        let mut related = if !loc.notes.is_empty() {
            notes_to_related_info(&loc.notes, loc.file.as_deref())
        } else {
            Vec::new()
        };

        // If the error originated in a different file, add a related info entry
        // pointing to the actual source location so the user can navigate there
        if let Some(ref file) = loc.file {
            if let Some(file_uri) = Uri::from_file_path(file) {
                let line = if loc.line > 0 { loc.line - 1 } else { 0 } as u32;
                let col = if loc.column > 0 { loc.column - 1 } else { 0 } as u32;
                let end_char = loc.length.map(|l| col + l as u32).unwrap_or(col + 1);
                related.push(DiagnosticRelatedInformation {
                    location: Location {
                        uri: file_uri,
                        range: Range {
                            start: Position {
                                line,
                                character: col,
                            },
                            end: Position {
                                line,
                                character: end_char,
                            },
                        },
                    },
                    message: format!("error originates in {}", file),
                });
            }
        }

        if related.is_empty() {
            None
        } else {
            Some(related)
        }
    } else {
        None
    };

    Diagnostic {
        range,
        severity: Some(severity),
        code: None, // Error code is set per-error type
        code_description: None,
        source: Some(source.to_string()),
        message: full_message,
        related_information,
        tags: None,
        data: None,
    }
}

/// Create a diagnostic with error code
fn create_diagnostic_with_code(
    message: &str,
    location: Option<&SourceLocation>,
    severity: DiagnosticSeverity,
    source: &str,
    error_code: Option<&str>,
) -> Diagnostic {
    let mut diag = create_diagnostic(message, location, severity, source);
    if let Some(code) = error_code {
        diag.code = Some(NumberOrString::String(code.to_string()));
    }
    diag
}

/// Convert ErrorNotes to LSP DiagnosticRelatedInformation
fn notes_to_related_info(
    notes: &[ErrorNote],
    default_file: Option<&str>,
) -> Vec<DiagnosticRelatedInformation> {
    notes
        .iter()
        .filter_map(|note| {
            // Try to create a valid URI for the location
            let location = note.location.as_ref()?;
            let file = location.file.as_deref().or(default_file)?;
            let uri = Uri::from_file_path(file)?;

            let line = if location.line > 0 {
                location.line - 1
            } else {
                0
            } as u32;
            let column = if location.column > 0 {
                location.column - 1
            } else {
                0
            } as u32;

            Some(DiagnosticRelatedInformation {
                location: Location {
                    uri,
                    range: Range {
                        start: Position {
                            line,
                            character: column,
                        },
                        end: Position {
                            line,
                            character: column + 1,
                        },
                    },
                },
                message: note.message.clone(),
            })
        })
        .collect()
}

/// Convert SourceLocation to LSP Range
fn location_to_range(location: Option<&SourceLocation>) -> Range {
    // For missing or synthetic locations, highlight the full first line
    // so the diagnostic is visible rather than a zero-width invisible marker
    let full_first_line = Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 1000,
        },
    };

    if let Some(loc) = location {
        if loc.is_synthetic {
            return full_first_line;
        }

        // Shape uses 1-based line/column, LSP uses 0-based
        let line = if loc.line > 0 { loc.line - 1 } else { 0 } as u32;
        let column = if loc.column > 0 { loc.column - 1 } else { 0 } as u32;

        let start = Position {
            line,
            character: column,
        };

        // If we have length, calculate end position
        let end = if let Some(len) = loc.length {
            Position {
                line,
                character: column + len as u32,
            }
        } else {
            // Default to highlighting the whole line if no length
            Position {
                line,
                character: column + 100, // Reasonable default
            }
        };

        Range { start, end }
    } else {
        full_first_line
    }
}

/// Validate annotations in a program and return diagnostics for any issues
///
/// Checks:
/// - Whether annotations are defined (in local file or imports)
/// - Whether annotation arguments are valid
pub fn validate_annotations(
    program: &Program,
    annotation_discovery: &AnnotationDiscovery,
    source: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for item in &program.items {
        let (annotations, span) = match item {
            Item::Function(func, span) => (&func.annotations, span),
            Item::ForeignFunction(foreign_fn, span) => (&foreign_fn.annotations, span),
            _ => continue,
        };
        for annotation in annotations {
            if let Some(diag) = validate_annotation(annotation, span, annotation_discovery, source)
            {
                diagnostics.push(diag);
            }
        }
    }

    diagnostics
}

/// Validate a single annotation usage
fn validate_annotation(
    annotation: &Annotation,
    item_span: &Span,
    annotation_discovery: &AnnotationDiscovery,
    source: &str,
) -> Option<Diagnostic> {
    let name = &annotation.name;

    // Check if annotation is defined
    if !annotation_discovery.is_defined(name) {
        let available: Vec<_> = annotation_discovery
            .all_annotations()
            .iter()
            .map(|a| format!("@{}", a.name))
            .collect();

        let message = if available.is_empty() {
            format!("Undefined annotation: @{}", name)
        } else {
            format!(
                "Undefined annotation: @{}. Available: {}",
                name,
                available.join(", ")
            )
        };

        // Calculate position from span
        let range = span_to_range(source, item_span);

        return Some(Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("E0100".to_string())),
            code_description: None,
            source: Some("shape".to_string()),
            message,
            related_information: None,
            tags: None,
            data: None,
        });
    }

    // Check argument count if annotation is defined
    if let Some(ann_info) = annotation_discovery.get(name) {
        let expected = ann_info.params.len();
        let actual = annotation.args.len();

        // Allow 0 args for optional-arg annotations, or exact match
        if actual > expected || (actual < expected && expected > 0 && actual > 0) {
            let range = span_to_range(source, item_span);

            return Some(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String("W0101".to_string())),
                code_description: None,
                source: Some("shape".to_string()),
                message: format!("@{} expects {} argument(s), got {}", name, expected, actual),
                related_information: None,
                tags: None,
                data: None,
            });
        }
    }

    None
}

/// Validate async join usage: `await join` must be inside an async function.
///
/// Walks the AST to find `Expr::Join` nodes that appear outside async function bodies.
pub fn validate_async_join(program: &Program, source: &str) -> Vec<Diagnostic> {
    use shape_ast::ast::Expr;
    use shape_runtime::visitor::{Visitor, walk_program};

    struct AsyncJoinValidator<'a> {
        source: &'a str,
        async_depth_stack: Vec<bool>,
        diagnostics: Vec<Diagnostic>,
    }

    impl AsyncJoinValidator<'_> {
        fn is_in_async(&self) -> bool {
            self.async_depth_stack.last().copied().unwrap_or(false)
        }
    }

    impl Visitor for AsyncJoinValidator<'_> {
        fn visit_function(&mut self, func: &shape_ast::ast::FunctionDef) -> bool {
            self.async_depth_stack.push(func.is_async);
            true
        }

        fn leave_function(&mut self, _func: &shape_ast::ast::FunctionDef) {
            self.async_depth_stack.pop();
        }

        fn visit_expr(&mut self, expr: &Expr) -> bool {
            if let Expr::Join(_, span) = expr {
                if !self.is_in_async() {
                    let range = span_to_range(self.source, span);
                    self.diagnostics.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        code: Some(NumberOrString::String("E0200".to_string())),
                        code_description: None,
                        source: Some("shape".to_string()),
                        message: "`await join` can only be used inside an async function"
                            .to_string(),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            }
            true
        }
    }

    let mut validator = AsyncJoinValidator {
        source,
        async_depth_stack: Vec::new(),
        diagnostics: Vec::new(),
    };
    walk_program(&mut validator, program);
    validator.diagnostics
}

/// Validate structured concurrency constructs (`async let`, `async scope`, `for await`)
/// are only used inside async function bodies.
pub fn validate_async_structured_concurrency(program: &Program, source: &str) -> Vec<Diagnostic> {
    use shape_ast::ast::Expr;
    use shape_runtime::visitor::{Visitor, walk_program};

    struct AsyncStructuredValidator<'a> {
        source: &'a str,
        async_depth_stack: Vec<bool>,
        diagnostics: Vec<Diagnostic>,
    }

    impl AsyncStructuredValidator<'_> {
        fn is_in_async(&self) -> bool {
            self.async_depth_stack.last().copied().unwrap_or(false)
        }
    }

    impl Visitor for AsyncStructuredValidator<'_> {
        fn visit_function(&mut self, func: &shape_ast::ast::FunctionDef) -> bool {
            self.async_depth_stack.push(func.is_async);
            true
        }

        fn leave_function(&mut self, _func: &shape_ast::ast::FunctionDef) {
            self.async_depth_stack.pop();
        }

        fn visit_expr(&mut self, expr: &Expr) -> bool {
            match expr {
                Expr::AsyncLet(_, span) => {
                    if !self.is_in_async() {
                        let range = span_to_range(self.source, span);
                        self.diagnostics.push(Diagnostic {
                            range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String("E0201".to_string())),
                            code_description: None,
                            source: Some("shape".to_string()),
                            message: "`async let` can only be used inside an async function"
                                .to_string(),
                            related_information: None,
                            tags: None,
                            data: None,
                        });
                    }
                }
                Expr::AsyncScope(_, span) => {
                    if !self.is_in_async() {
                        let range = span_to_range(self.source, span);
                        self.diagnostics.push(Diagnostic {
                            range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String("E0202".to_string())),
                            code_description: None,
                            source: Some("shape".to_string()),
                            message: "`async scope` can only be used inside an async function"
                                .to_string(),
                            related_information: None,
                            tags: None,
                            data: None,
                        });
                    }
                }
                Expr::For(for_expr, span) if for_expr.is_async => {
                    if !self.is_in_async() {
                        let range = span_to_range(self.source, span);
                        self.diagnostics.push(Diagnostic {
                            range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String("E0203".to_string())),
                            code_description: None,
                            source: Some("shape".to_string()),
                            message: "`for await` can only be used inside an async function"
                                .to_string(),
                            related_information: None,
                            tags: None,
                            data: None,
                        });
                    }
                }
                _ => {}
            }
            true
        }

        fn visit_stmt(&mut self, stmt: &shape_ast::ast::Statement) -> bool {
            if let shape_ast::ast::Statement::For(for_loop, span) = stmt {
                if for_loop.is_async && !self.is_in_async() {
                    let range = span_to_range(self.source, span);
                    self.diagnostics.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        code: Some(NumberOrString::String("E0203".to_string())),
                        code_description: None,
                        source: Some("shape".to_string()),
                        message: "`for await` can only be used inside an async function"
                            .to_string(),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            }
            true
        }
    }

    let mut validator = AsyncStructuredValidator {
        source,
        async_depth_stack: Vec::new(),
        diagnostics: Vec::new(),
    };
    walk_program(&mut validator, program);
    validator.diagnostics
}

/// Validate formatted interpolation specs in `f"..."` string literals.
///
/// This catches invalid typed specs (unknown keys, invalid enum values, malformed
/// spec calls) in the editor without waiting for a full compile pass.
pub fn validate_interpolation_format_specs(program: &Program, source: &str) -> Vec<Diagnostic> {
    use shape_ast::interpolation::parse_interpolation_with_mode;
    use shape_runtime::visitor::{Visitor, walk_program};

    struct InterpolationFormatSpecValidator<'a> {
        source: &'a str,
        diagnostics: Vec<Diagnostic>,
    }

    impl Visitor for InterpolationFormatSpecValidator<'_> {
        fn visit_expr(&mut self, expr: &Expr) -> bool {
            if let Expr::Literal(
                Literal::FormattedString { value, mode } | Literal::ContentString { value, mode },
                span,
            ) = expr
            {
                if let Err(err) = parse_interpolation_with_mode(value, *mode) {
                    let range = span_to_range(self.source, span);
                    self.diagnostics.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        code: Some(NumberOrString::String("E0300".to_string())),
                        code_description: None,
                        source: Some("shape".to_string()),
                        message: format!("Invalid interpolation format spec: {}", err),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            }
            true
        }
    }

    let mut validator = InterpolationFormatSpecValidator {
        source,
        diagnostics: Vec::new(),
    };
    walk_program(&mut validator, program);
    validator.diagnostics
}

/// Validate type alias overrides: only comptime fields can be overridden.
///
/// Checks `type EUR = Currency { symbol: "EUR" }` and reports an error if `symbol`
/// is not a comptime field of `Currency`.
pub fn validate_comptime_overrides(program: &Program, source: &str) -> Vec<Diagnostic> {
    use std::collections::HashMap;

    let mut diagnostics = Vec::new();

    // Collect struct type definitions with their comptime field names
    let mut struct_comptime_fields: HashMap<String, Vec<String>> = HashMap::new();
    for item in &program.items {
        if let Item::StructType(struct_def, _) = item {
            let comptime_names: Vec<String> = struct_def
                .fields
                .iter()
                .filter(|f| f.is_comptime)
                .map(|f| f.name.clone())
                .collect();
            struct_comptime_fields.insert(struct_def.name.clone(), comptime_names);
        }
    }

    // Check type alias overrides
    for item in &program.items {
        if let Item::TypeAlias(alias_def, span) = item {
            if let Some(overrides) = &alias_def.meta_param_overrides {
                // Get the base type name from the type annotation
                let base_type = match &alias_def.type_annotation {
                    shape_ast::ast::TypeAnnotation::Basic(name) => name.clone(),
                    _ => continue,
                };

                if let Some(comptime_fields) = struct_comptime_fields.get(&base_type) {
                    for (field_name, _value) in overrides {
                        if !comptime_fields.contains(field_name) {
                            // Find the override position within the span for better error location
                            let range = span_to_range(source, span);
                            diagnostics.push(Diagnostic {
                                range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: Some(NumberOrString::String("E0300".to_string())),
                                code_description: None,
                                source: Some("shape".to_string()),
                                message: format!(
                                    "Cannot override field '{}': only comptime fields can be overridden in type alias. \
                                     '{}' is not a comptime field of '{}'.",
                                    field_name, field_name, base_type
                                ),
                                related_information: None,
                                tags: None,
                                data: None,
                            });
                        }
                    }
                }
            }
        }
    }

    diagnostics
}

/// Warn when a comptime block contains side-effecting expressions.
///
/// Comptime blocks run at compile time, so side effects (print, I/O) are
/// unexpected and likely mistakes.
pub fn validate_comptime_side_effects(program: &Program, source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Walk top-level items for Item::Comptime and expressions containing Expr::Comptime
    for item in &program.items {
        match item {
            Item::Comptime(stmts, span) => {
                check_stmts_for_side_effects(stmts, span, source, &mut diagnostics);
            }
            _ => {
                visit_item_exprs(item, source, &mut diagnostics);
            }
        }
    }

    diagnostics
}

/// Known side-effecting function names that should not appear in comptime blocks.
const SIDE_EFFECT_FNS: &[&str] = &["print", "println", "debug", "log", "write", "fetch"];

fn check_stmts_for_side_effects(
    stmts: &[Statement],
    _block_span: &Span,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        check_stmt_for_side_effects(stmt, source, diagnostics);
    }
}

fn check_stmt_for_side_effects(stmt: &Statement, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    match stmt {
        Statement::Expression(expr, _) => check_expr_for_side_effects(expr, source, diagnostics),
        Statement::VariableDecl(decl, _) => {
            if let Some(init) = &decl.value {
                check_expr_for_side_effects(init, source, diagnostics);
            }
        }
        Statement::Return(Some(expr), _) => check_expr_for_side_effects(expr, source, diagnostics),
        Statement::For(for_loop, _) => {
            for s in &for_loop.body {
                check_stmt_for_side_effects(s, source, diagnostics);
            }
        }
        Statement::While(while_loop, _) => {
            for s in &while_loop.body {
                check_stmt_for_side_effects(s, source, diagnostics);
            }
        }
        Statement::If(if_stmt, _) => {
            for s in &if_stmt.then_body {
                check_stmt_for_side_effects(s, source, diagnostics);
            }
            if let Some(else_body) = &if_stmt.else_body {
                for s in else_body {
                    check_stmt_for_side_effects(s, source, diagnostics);
                }
            }
        }
        _ => {}
    }
}

fn check_expr_for_side_effects(expr: &Expr, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    match expr {
        Expr::FunctionCall {
            name,
            span,
            args,
            named_args,
        } => {
            if SIDE_EFFECT_FNS.contains(&name.as_str()) {
                let range = span_to_range(source, span);
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("W0100".to_string())),
                    code_description: None,
                    source: Some("shape".to_string()),
                    message: format!(
                        "Side effect in comptime block: `{}()` performs I/O at compile time. \
                         Consider removing or using a comptime-safe alternative.",
                        name
                    ),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
            // Recurse into args
            for arg in args {
                check_expr_for_side_effects(arg, source, diagnostics);
            }
            for (_, arg) in named_args {
                check_expr_for_side_effects(arg, source, diagnostics);
            }
        }
        Expr::Comptime(stmts, span) => {
            // Nested comptime — still check
            check_stmts_for_side_effects(stmts, span, source, diagnostics);
        }
        _ => {}
    }
}

/// Walk an item's expressions looking for Expr::Comptime blocks to validate.
fn visit_item_exprs(item: &Item, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    // We only need to find Expr::Comptime inside function bodies, variable decls, etc.
    match item {
        Item::Function(func_def, _) => {
            for stmt in &func_def.body {
                visit_stmt_for_comptime(stmt, source, diagnostics);
            }
        }
        Item::VariableDecl(decl, _) => {
            if let Some(init) = &decl.value {
                visit_expr_for_comptime(init, source, diagnostics);
            }
        }
        Item::Expression(expr, _) => {
            visit_expr_for_comptime(expr, source, diagnostics);
        }
        Item::Statement(stmt, _) => {
            visit_stmt_for_comptime(stmt, source, diagnostics);
        }
        _ => {}
    }
}

fn visit_stmt_for_comptime(stmt: &Statement, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    match stmt {
        Statement::Expression(expr, _) => visit_expr_for_comptime(expr, source, diagnostics),
        Statement::VariableDecl(decl, _) => {
            if let Some(init) = &decl.value {
                visit_expr_for_comptime(init, source, diagnostics);
            }
        }
        Statement::Return(Some(expr), _) => visit_expr_for_comptime(expr, source, diagnostics),
        Statement::For(for_loop, _) => {
            for s in &for_loop.body {
                visit_stmt_for_comptime(s, source, diagnostics);
            }
        }
        Statement::While(while_loop, _) => {
            for s in &while_loop.body {
                visit_stmt_for_comptime(s, source, diagnostics);
            }
        }
        Statement::If(if_stmt, _) => {
            for s in &if_stmt.then_body {
                visit_stmt_for_comptime(s, source, diagnostics);
            }
            if let Some(else_body) = &if_stmt.else_body {
                for s in else_body {
                    visit_stmt_for_comptime(s, source, diagnostics);
                }
            }
        }
        _ => {}
    }
}

fn visit_expr_for_comptime(expr: &Expr, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    match expr {
        Expr::Comptime(stmts, span) => {
            check_stmts_for_side_effects(stmts, span, source, diagnostics);
        }
        Expr::FunctionCall {
            args, named_args, ..
        } => {
            for arg in args {
                visit_expr_for_comptime(arg, source, diagnostics);
            }
            for (_, arg) in named_args {
                visit_expr_for_comptime(arg, source, diagnostics);
            }
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            visit_expr_for_comptime(condition, source, diagnostics);
            visit_expr_for_comptime(then_expr, source, diagnostics);
            if let Some(e) = else_expr {
                visit_expr_for_comptime(e, source, diagnostics);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            visit_expr_for_comptime(left, source, diagnostics);
            visit_expr_for_comptime(right, source, diagnostics);
        }
        Expr::UnaryOp { operand, .. } => {
            visit_expr_for_comptime(operand, source, diagnostics);
        }
        _ => {}
    }
}

/// Diagnose comptime-only builtins called outside a `comptime { }` block.
pub fn validate_comptime_builtins_context(program: &Program, source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for item in &program.items {
        match item {
            Item::Comptime(_, _) => {
                // Inside comptime — everything is allowed
            }
            Item::Function(func_def, _) => {
                for stmt in &func_def.body {
                    check_stmt_comptime_only(stmt, false, source, &mut diagnostics);
                }
            }
            Item::VariableDecl(decl, _) => {
                if let Some(init) = &decl.value {
                    check_expr_comptime_only(init, false, source, &mut diagnostics);
                }
            }
            Item::Expression(expr, _) => {
                check_expr_comptime_only(expr, false, source, &mut diagnostics);
            }
            Item::Statement(stmt, _) => {
                check_stmt_comptime_only(stmt, false, source, &mut diagnostics);
            }
            _ => {}
        }
    }

    diagnostics
}

fn check_stmt_comptime_only(
    stmt: &Statement,
    in_comptime: bool,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match stmt {
        Statement::Expression(expr, _) => {
            check_expr_comptime_only(expr, in_comptime, source, diagnostics);
        }
        Statement::VariableDecl(decl, _) => {
            if let Some(init) = &decl.value {
                check_expr_comptime_only(init, in_comptime, source, diagnostics);
            }
        }
        Statement::Return(Some(expr), _) => {
            check_expr_comptime_only(expr, in_comptime, source, diagnostics);
        }
        Statement::For(for_loop, _) => {
            for s in &for_loop.body {
                check_stmt_comptime_only(s, in_comptime, source, diagnostics);
            }
        }
        Statement::While(while_loop, _) => {
            for s in &while_loop.body {
                check_stmt_comptime_only(s, in_comptime, source, diagnostics);
            }
        }
        Statement::If(if_stmt, _) => {
            for s in &if_stmt.then_body {
                check_stmt_comptime_only(s, in_comptime, source, diagnostics);
            }
            if let Some(else_body) = &if_stmt.else_body {
                for s in else_body {
                    check_stmt_comptime_only(s, in_comptime, source, diagnostics);
                }
            }
        }
        _ => {}
    }
}

fn check_expr_comptime_only(
    expr: &Expr,
    in_comptime: bool,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Comptime(stmts, _) => {
            // Inside comptime block — builtins are allowed
            for stmt in stmts {
                check_stmt_comptime_only(stmt, true, source, diagnostics);
            }
        }
        Expr::FunctionCall {
            name,
            span,
            args,
            named_args,
        } => {
            let is_comptime_only = unified_metadata()
                .get_function(name)
                .map(|f| f.comptime_only)
                .unwrap_or(false);
            if !in_comptime && is_comptime_only {
                let range = span_to_range(source, span);
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: Some(NumberOrString::String("E0301".to_string())),
                    code_description: None,
                    source: Some("shape".to_string()),
                    message: format!(
                        "`{}()` is a comptime-only builtin and can only be called inside a `comptime {{ }}` block.",
                        name
                    ),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
            for arg in args {
                check_expr_comptime_only(arg, in_comptime, source, diagnostics);
            }
            for (_, arg) in named_args {
                check_expr_comptime_only(arg, in_comptime, source, diagnostics);
            }
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            check_expr_comptime_only(condition, in_comptime, source, diagnostics);
            check_expr_comptime_only(then_expr, in_comptime, source, diagnostics);
            if let Some(e) = else_expr {
                check_expr_comptime_only(e, in_comptime, source, diagnostics);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            check_expr_comptime_only(left, in_comptime, source, diagnostics);
            check_expr_comptime_only(right, in_comptime, source, diagnostics);
        }
        Expr::UnaryOp { operand, .. } => {
            check_expr_comptime_only(operand, in_comptime, source, diagnostics);
        }
        _ => {}
    }
}

/// Validate trait bound satisfaction in impl blocks and function type parameters.
///
/// Checks:
/// - Functions with bounded type params: `fn foo<T: Comparable>(x: T)` — the trait must exist
/// - Impl blocks: all required trait methods are implemented
pub fn validate_trait_bounds(program: &Program, source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Collect known trait definitions and their required method names
    let mut trait_methods: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut trait_spans: std::collections::HashMap<String, Span> = std::collections::HashMap::new();
    for item in &program.items {
        if let Item::Trait(trait_def, span) = item {
            let required: Vec<String> = trait_def
                .members
                .iter()
                .filter_map(|m| match m {
                    shape_ast::ast::TraitMember::Required(
                        shape_ast::ast::InterfaceMember::Method { name, .. },
                    ) => Some(name.clone()),
                    _ => None,
                })
                .collect();
            trait_methods.insert(trait_def.name.clone(), required);
            trait_spans.insert(trait_def.name.clone(), *span);
        }
    }

    // Check function type parameter trait bounds reference existing traits
    for item in &program.items {
        if let Item::Function(func, span) = item {
            if let Some(type_params) = &func.type_params {
                for tp in type_params {
                    for bound in &tp.trait_bounds {
                        if !trait_methods.contains_key(bound.as_str()) {
                            let range = span_to_range(source, span);
                            diagnostics.push(Diagnostic {
                                range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: Some(NumberOrString::String("E0400".to_string())),
                                code_description: None,
                                source: Some("shape".to_string()),
                                message: format!(
                                    "Trait bound '{}' on type parameter '{}' refers to an undefined trait.",
                                    bound, tp.name
                                ),
                                related_information: None,
                                tags: None,
                                data: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // Check impl blocks: all required methods are implemented
    for item in &program.items {
        if let Item::Impl(impl_block, span) = item {
            let trait_name = match &impl_block.trait_name {
                shape_ast::ast::TypeName::Simple(n) => n.to_string(),
                shape_ast::ast::TypeName::Generic { name, .. } => name.to_string(),
            };
            let target_type = match &impl_block.target_type {
                shape_ast::ast::TypeName::Simple(n) => n.to_string(),
                shape_ast::ast::TypeName::Generic { name, .. } => name.to_string(),
            };

            if let Some(required_methods) = trait_methods.get(&trait_name) {
                let implemented: Vec<String> =
                    impl_block.methods.iter().map(|m| m.name.clone()).collect();
                for required in required_methods {
                    if !implemented.contains(required) {
                        let range = span_to_range(source, span);
                        diagnostics.push(Diagnostic {
                            range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String("E0401".to_string())),
                            code_description: None,
                            source: Some("shape".to_string()),
                            message: format!(
                                "Missing required method '{}' in impl {} for {}.",
                                required, trait_name, target_type
                            ),
                            related_information: None,
                            tags: None,
                            data: None,
                        });
                    }
                }
            }
        }
    }

    diagnostics
}

/// Validate content string usage and Content API calls.
///
/// - Error on empty interpolation `{}` in c-strings
/// - Warn on `Color.rgb()` with values outside 0-255
pub fn validate_content_strings(program: &Program, source: &str) -> Vec<Diagnostic> {
    use shape_runtime::visitor::{Visitor, walk_program};

    struct ContentStringValidator<'a> {
        source: &'a str,
        diagnostics: Vec<Diagnostic>,
    }

    impl Visitor for ContentStringValidator<'_> {
        fn visit_expr(&mut self, expr: &Expr) -> bool {
            match expr {
                Expr::Literal(Literal::ContentString { value, .. }, span) => {
                    // Check for empty interpolation `{}`
                    if value.contains("{}") {
                        let range = span_to_range(self.source, span);
                        self.diagnostics.push(Diagnostic {
                            range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String("E0310".to_string())),
                            code_description: None,
                            source: Some("shape".to_string()),
                            message: "Empty interpolation `{}` in content string. Provide an expression inside the braces.".to_string(),
                            related_information: None,
                            tags: None,
                            data: None,
                        });
                    }
                }
                // Check Color.rgb(r, g, b) for out-of-range values
                Expr::MethodCall {
                    receiver,
                    method,
                    args,
                    span,
                    ..
                } if method == "rgb" => {
                    if let Expr::Identifier(name, _) = receiver.as_ref() {
                        if name == "Color" {
                            for arg in args {
                                let out_of_range = match arg {
                                    Expr::Literal(Literal::Int(v), _) => *v < 0 || *v > 255,
                                    Expr::Literal(Literal::Number(v), _) => {
                                        (*v as i64) < 0 || (*v as i64) > 255
                                    }
                                    _ => false,
                                };
                                if out_of_range {
                                    let val_str = match arg {
                                        Expr::Literal(Literal::Int(v), _) => v.to_string(),
                                        Expr::Literal(Literal::Number(v), _) => v.to_string(),
                                        _ => String::new(),
                                    };
                                    let range = span_to_range(self.source, span);
                                    self.diagnostics.push(Diagnostic {
                                        range,
                                        severity: Some(DiagnosticSeverity::WARNING),
                                        code: Some(NumberOrString::String("W0310".to_string())),
                                        code_description: None,
                                        source: Some("shape".to_string()),
                                        message: format!(
                                            "Color.rgb() component value {} is outside the valid range 0-255.",
                                            val_str
                                        ),
                                        related_information: None,
                                        tags: None,
                                        data: None,
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            true
        }
    }

    let mut validator = ContentStringValidator {
        source,
        diagnostics: Vec::new(),
    };
    walk_program(&mut validator, program);
    validator.diagnostics
}

/// Validate that foreign function parameters and return types are explicitly annotated.
///
/// Foreign function bodies are opaque — the type system cannot infer types from them.
/// Uses `ForeignFunctionDef::validate_type_annotations()` (shared with the compiler).
pub fn validate_foreign_function_types(program: &Program, source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for item in &program.items {
        let foreign_fn = match item {
            Item::ForeignFunction(f, _) => f,
            Item::Export(export, _) => {
                if let shape_ast::ast::ExportItem::ForeignFunction(f) = &export.item {
                    f
                } else {
                    continue;
                }
            }
            _ => continue,
        };

        for (msg, span) in foreign_fn.validate_type_annotations(true) {
            let range = if span.is_dummy() {
                span_to_range(source, &foreign_fn.name_span)
            } else {
                span_to_range(source, &span)
            };
            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("E0400".to_string())),
                code_description: None,
                source: Some("shape".to_string()),
                message: msg,
                related_information: None,
                tags: None,
                data: None,
            });
        }
    }

    diagnostics
}

// ─── MIR borrow analysis → LSP diagnostics ─────────────────────────────────
//
// The compiler already converts MIR `BorrowError`/`MutabilityError` into
// `ShapeError::SemanticError` which flows through `error_to_diagnostic`.
// This module provides an *alternative* path that produces richer LSP
// diagnostics directly from the structured analysis data (proper error
// codes, `DiagnosticRelatedInformation` for borrow origins, etc.).
//
// Usage: call `borrow_analysis_to_diagnostics` after a successful or
// recovered compilation to surface borrow warnings that the compiler's
// `RecoverAll` mode collected but did not promote to hard errors.

/// Convert structured MIR borrow errors into LSP diagnostics.
///
/// This produces higher-fidelity diagnostics than the `error_to_diagnostic`
/// path because it has direct access to `BorrowError` fields:
/// - Sets `code` to the unified `BorrowErrorCode` (`B0001`..`B0007`).
/// - Attaches `DiagnosticRelatedInformation` for the loan origin span
///   and the last-use span (if available).
pub fn borrow_analysis_to_diagnostics(
    analysis: &shape_vm::mir::analysis::BorrowAnalysis,
    source: &str,
    uri: &Uri,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for error in &analysis.errors {
        let code = error.kind.code();

        let primary_range = span_to_range(source, &error.span);

        let message = borrow_error_message(&error.kind, code);

        // Build related-information entries.
        let mut related = Vec::new();

        // 1. Where the conflicting loan was created.
        let loan_range = span_to_range(source, &error.loan_span);
        related.push(DiagnosticRelatedInformation {
            location: Location {
                uri: uri.clone(),
                range: loan_range,
            },
            message: borrow_origin_note(&error.kind),
        });

        // 2. Where the loan is still needed (last use).
        if let Some(last_use) = error.last_use_span {
            let last_use_range = span_to_range(source, &last_use);
            related.push(DiagnosticRelatedInformation {
                location: Location {
                    uri: uri.clone(),
                    range: last_use_range,
                },
                message: "borrow is still needed here".to_string(),
            });
        }

        // Build hint text from repair suggestions.
        let hint = if let Some(repair) = error.repairs.first() {
            format!(
                "help: {}\nhelp: {}",
                borrow_error_hint(&error.kind),
                repair.description
            )
        } else {
            format!("help: {}", borrow_error_hint(&error.kind))
        };

        diagnostics.push(Diagnostic {
            range: primary_range,
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String(code.as_str().to_string())),
            code_description: None,
            source: Some("shape-borrow".to_string()),
            message: format!("{}\n{}", message, hint),
            related_information: Some(related),
            tags: None,
            data: None,
        });
    }

    for error in &analysis.mutability_errors {
        let primary_range = span_to_range(source, &error.span);

        let binding_kind = if error.is_const {
            "const"
        } else if error.is_explicit_let {
            "let"
        } else {
            "immutable"
        };

        let message = format!(
            "cannot assign to {} binding '{}'",
            binding_kind, error.variable_name
        );

        let decl_range = span_to_range(source, &error.declaration_span);
        let related = vec![DiagnosticRelatedInformation {
            location: Location {
                uri: uri.clone(),
                range: decl_range,
            },
            message: format!("'{}' declared here", error.variable_name),
        }];

        diagnostics.push(Diagnostic {
            range: primary_range,
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("E0384".to_string())),
            code_description: None,
            source: Some("shape-borrow".to_string()),
            message: format!(
                "{}\nhelp: consider changing '{}' to 'let mut {}' or 'var {}'",
                message, error.variable_name, error.variable_name, error.variable_name
            ),
            related_information: Some(related),
            tags: None,
            data: None,
        });
    }

    diagnostics
}

/// Human-readable message for a borrow error kind (with code prefix).
fn borrow_error_message(
    kind: &shape_vm::mir::analysis::BorrowErrorKind,
    code: shape_vm::mir::analysis::BorrowErrorCode,
) -> String {
    use shape_vm::mir::analysis::BorrowErrorKind;
    let body = match kind {
        BorrowErrorKind::ConflictSharedExclusive => {
            "cannot mutably borrow this value while shared borrows are active"
        }
        BorrowErrorKind::ConflictExclusiveExclusive => {
            "cannot mutably borrow this value because it is already borrowed"
        }
        BorrowErrorKind::ReadWhileExclusivelyBorrowed => {
            "cannot read this value while it is mutably borrowed"
        }
        BorrowErrorKind::WriteWhileBorrowed => {
            "cannot write to this value while it is borrowed"
        }
        BorrowErrorKind::ReferenceEscape => {
            "cannot return or store a reference that outlives its owner"
        }
        BorrowErrorKind::ReferenceStoredInArray => {
            "cannot store a reference in an array"
        }
        BorrowErrorKind::ReferenceStoredInObject => {
            "cannot store a reference in an object or struct literal"
        }
        BorrowErrorKind::ReferenceStoredInEnum => {
            "cannot store a reference in an enum payload"
        }
        BorrowErrorKind::ReferenceEscapeIntoClosure => {
            "reference cannot escape into a closure"
        }
        BorrowErrorKind::UseAfterMove => {
            "cannot use this value after it was moved"
        }
        BorrowErrorKind::ExclusiveRefAcrossTaskBoundary => {
            "cannot move an exclusive reference across a task boundary"
        }
        BorrowErrorKind::SharedRefAcrossDetachedTask => {
            "cannot send a shared reference across a detached task boundary"
        }
        BorrowErrorKind::InconsistentReferenceReturn => {
            "reference-returning functions must return a reference on every path from the same borrowed origin and borrow kind"
        }
        BorrowErrorKind::CallSiteAliasConflict => {
            "cannot pass the same variable to multiple parameters that conflict on aliasing"
        }
        BorrowErrorKind::NonSendableAcrossTaskBoundary => {
            "cannot send a non-sendable value across a task boundary"
        }
    };
    format!("[{}] {}", code, body)
}

/// Hint text for a borrow error kind.
fn borrow_error_hint(kind: &shape_vm::mir::analysis::BorrowErrorKind) -> &'static str {
    use shape_vm::mir::analysis::BorrowErrorKind;
    match kind {
        BorrowErrorKind::ConflictSharedExclusive => {
            "move the mutable borrow later, or end the shared borrow sooner"
        }
        BorrowErrorKind::ConflictExclusiveExclusive => {
            "end the previous mutable borrow before creating another one"
        }
        BorrowErrorKind::ReadWhileExclusivelyBorrowed => {
            "read through the existing reference, or move the read after the borrow ends"
        }
        BorrowErrorKind::WriteWhileBorrowed => "move this write after the borrow ends",
        BorrowErrorKind::ReferenceEscape => "return an owned value instead of a reference",
        BorrowErrorKind::ReferenceStoredInArray
        | BorrowErrorKind::ReferenceStoredInObject
        | BorrowErrorKind::ReferenceStoredInEnum => {
            "store owned values instead of references"
        }
        BorrowErrorKind::ReferenceEscapeIntoClosure => {
            "capture an owned value instead of a reference"
        }
        BorrowErrorKind::UseAfterMove => {
            "clone the value before moving it, or stop using the original after the move"
        }
        BorrowErrorKind::ExclusiveRefAcrossTaskBoundary => {
            "keep the mutable reference within the current task or pass an owned value instead"
        }
        BorrowErrorKind::SharedRefAcrossDetachedTask => {
            "clone the value before sending it to a detached task, or use a structured task instead"
        }
        BorrowErrorKind::InconsistentReferenceReturn => {
            "return a reference from the same borrowed origin on every path, or return owned values instead"
        }
        BorrowErrorKind::CallSiteAliasConflict => {
            "use separate variables for each argument, or clone one of them"
        }
        BorrowErrorKind::NonSendableAcrossTaskBoundary => {
            "clone the captured state or use an owned value that is safe to send across tasks"
        }
    }
}

/// Note text for the related-information entry pointing at the loan origin.
fn borrow_origin_note(kind: &shape_vm::mir::analysis::BorrowErrorKind) -> String {
    use shape_vm::mir::analysis::BorrowErrorKind;
    match kind {
        BorrowErrorKind::ConflictSharedExclusive
        | BorrowErrorKind::ConflictExclusiveExclusive
        | BorrowErrorKind::ReadWhileExclusivelyBorrowed
        | BorrowErrorKind::WriteWhileBorrowed => "conflicting borrow originates here".to_string(),
        BorrowErrorKind::ReferenceEscape
        | BorrowErrorKind::ReferenceStoredInArray
        | BorrowErrorKind::ReferenceStoredInObject
        | BorrowErrorKind::ReferenceStoredInEnum
        | BorrowErrorKind::ReferenceEscapeIntoClosure
        | BorrowErrorKind::ExclusiveRefAcrossTaskBoundary
        | BorrowErrorKind::SharedRefAcrossDetachedTask => {
            "reference originates here".to_string()
        }
        BorrowErrorKind::UseAfterMove => "value was moved here".to_string(),
        BorrowErrorKind::InconsistentReferenceReturn => {
            "borrowed origin on another return path originates here".to_string()
        }
        BorrowErrorKind::CallSiteAliasConflict => {
            "conflicting arguments originate here".to_string()
        }
        BorrowErrorKind::NonSendableAcrossTaskBoundary => {
            "non-sendable value originates here".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::offset_to_line_col;

    #[test]
    fn test_location_to_range() {
        // Test with location
        let loc = SourceLocation::new(5, 10);
        let range = location_to_range(Some(&loc));

        assert_eq!(range.start.line, 4); // 0-based
        assert_eq!(range.start.character, 9); // 0-based

        // Test without location
        let range = location_to_range(None);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
    }

    #[test]
    fn test_parse_error_diagnostic() {
        let error = ShapeError::ParseError {
            message: "Expected expression".to_string(),
            location: Some(SourceLocation::new(10, 5)),
        };

        let diagnostics = error_to_diagnostic(&error);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "Expected expression");
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(diagnostics[0].source.as_deref(), Some("shape"));
        assert_eq!(diagnostics[0].range.start.line, 9); // 0-based
    }

    #[test]
    fn test_semantic_error_diagnostic() {
        let error = ShapeError::SemanticError {
            message: "Undefined variable 'x'".to_string(),
            location: Some(SourceLocation::new(3, 7)),
        };

        let diagnostics = error_to_diagnostic(&error);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "Undefined variable 'x'");
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(diagnostics[0].source.as_deref(), Some("shape"));
    }

    #[test]
    fn test_multi_error_flattening() {
        let multi_error = ShapeError::MultiError(vec![
            ShapeError::SemanticError {
                message: "Undefined variable 'x'".to_string(),
                location: Some(SourceLocation::new(1, 1)),
            },
            ShapeError::SemanticError {
                message: "Undefined variable 'y'".to_string(),
                location: Some(SourceLocation::new(2, 1)),
            },
        ]);

        let diagnostics = error_to_diagnostic(&multi_error);
        assert_eq!(
            diagnostics.len(),
            2,
            "MultiError should flatten into 2 diagnostics"
        );
        assert!(diagnostics[0].message.contains("x"));
        assert!(diagnostics[1].message.contains("y"));
    }

    #[test]
    fn test_multi_error_display() {
        let multi_error = ShapeError::MultiError(vec![
            ShapeError::SemanticError {
                message: "Error one".to_string(),
                location: None,
            },
            ShapeError::SemanticError {
                message: "Error two".to_string(),
                location: None,
            },
        ]);

        let display = multi_error.to_string();
        assert!(
            display.contains("Error one"),
            "Display should contain first error"
        );
        assert!(
            display.contains("Error two"),
            "Display should contain second error"
        );
    }

    #[test]
    fn test_offset_to_line_col() {
        let source = "line1\nline2\nline3";

        // Start of file
        assert_eq!(offset_to_line_col(source, 0), (0, 0));

        // End of first line
        assert_eq!(offset_to_line_col(source, 5), (0, 5));

        // Start of second line
        assert_eq!(offset_to_line_col(source, 6), (1, 0));

        // Middle of second line
        assert_eq!(offset_to_line_col(source, 8), (1, 2));
    }

    #[test]
    fn test_validate_annotations_with_defined() {
        use shape_ast::parser::parse_program;

        // Define @my_ann locally, then use it on a function
        let source = r#"
annotation my_ann() {
    on_define(fn, ctx) {
        ctx.registry("items").set(fn.name, fn)
    }
}

@my_ann
function my_func(x) {
    return x + 1;
}
"#;

        let program = parse_program(source).unwrap();
        let mut discovery = AnnotationDiscovery::new();
        discovery.discover_from_program(&program);

        let diagnostics = validate_annotations(&program, &discovery, source);

        // @my_ann is defined locally, so no errors
        assert!(
            diagnostics.is_empty(),
            "Expected no diagnostics for defined annotation, got: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_validate_annotations_with_undefined() {
        use shape_ast::parser::parse_program;

        let source = r#"
@undefined_annotation
function my_func() {
    return None;
}
"#;

        let program = parse_program(source).unwrap();
        let mut discovery = AnnotationDiscovery::new();
        discovery.discover_from_program(&program);

        let diagnostics = validate_annotations(&program, &discovery, source);

        // @undefined_annotation is not defined anywhere
        assert_eq!(
            diagnostics.len(),
            1,
            "Expected 1 diagnostic for undefined annotation"
        );
        assert!(diagnostics[0].message.contains("Undefined annotation"));
        assert!(diagnostics[0].message.contains("undefined_annotation"));
    }

    #[test]
    fn test_validate_trait_bounds_missing_method() {
        use shape_ast::parser::parse_program;

        let source = "trait Queryable {\n    filter(pred): any;\n    select(cols): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}\n";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_trait_bounds(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report 1 missing method error, got: {:?}",
            diagnostics
        );
        assert!(diagnostics[0].message.contains("Missing required method"));
        assert!(diagnostics[0].message.contains("select"));
    }

    #[test]
    fn test_validate_trait_bounds_all_implemented() {
        use shape_ast::parser::parse_program;

        let source = "trait Queryable {\n    filter(pred): any;\n    select(cols): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    method select(cols) { self }\n}\n";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_trait_bounds(&program, source);

        assert_eq!(
            diagnostics.len(),
            0,
            "Should report no errors when all methods implemented"
        );
    }

    #[test]
    fn test_validate_trait_bounds_undefined_trait_in_bound() {
        use shape_ast::parser::parse_program;

        let source = "fn foo<T: NonExistent>(x: T) {\n    x\n}\n";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_trait_bounds(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report undefined trait in bound"
        );
        assert!(diagnostics[0].message.contains("NonExistent"));
        assert!(diagnostics[0].message.contains("undefined trait"));
    }

    #[test]
    fn test_validate_trait_bounds_valid_bound() {
        use shape_ast::parser::parse_program;

        let source = "trait Comparable {\n    compare(other): number\n}\nfn foo<T: Comparable>(x: T) {\n    x\n}\n";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_trait_bounds(&program, source);

        assert_eq!(
            diagnostics.len(),
            0,
            "Should report no errors for valid trait bound"
        );
    }

    #[test]
    fn test_validate_async_join_outside_async() {
        use shape_ast::parser::parse_program;

        let source = "fn foo() {\n  let x = await join all {\n    1,\n    2\n  }\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_join(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report error for join outside async function"
        );
        assert!(
            diagnostics[0].message.contains("async"),
            "Error should mention async, got: {}",
            diagnostics[0].message
        );
    }

    #[test]
    fn test_validate_async_join_inside_async() {
        use shape_ast::parser::parse_program;

        let source = "async fn foo() {\n  let x = await join all {\n    1,\n    2\n  }\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_join(&program, source);

        assert_eq!(
            diagnostics.len(),
            0,
            "Should not report error for join inside async function"
        );
    }

    #[test]
    fn test_validate_async_join_top_level() {
        use shape_ast::parser::parse_program;

        // Join at top level (not inside any function) should be an error
        let source = "let x = await join race {\n  1,\n  2\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_join(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report error for join at top level"
        );
    }

    #[test]
    fn test_validate_comptime_side_effects_with_print() {
        use shape_ast::parser::parse_program;

        let source = "comptime {\n  print(\"hello\")\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_comptime_side_effects(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should warn about print() in comptime block"
        );
        assert!(
            diagnostics[0].message.contains("print"),
            "Warning should mention print"
        );
        assert_eq!(
            diagnostics[0].severity,
            Some(DiagnosticSeverity::WARNING),
            "Should be a warning, not an error"
        );
    }

    #[test]
    fn test_validate_comptime_side_effects_clean() {
        use shape_ast::parser::parse_program;

        let source = "comptime {\n  let x = 42\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_comptime_side_effects(&program, source);

        assert_eq!(
            diagnostics.len(),
            0,
            "Pure comptime block should have no warnings"
        );
    }

    #[test]
    fn test_validate_comptime_side_effects_nested_in_function() {
        use shape_ast::parser::parse_program;

        let source = "fn foo() {\n  let x = comptime {\n    print(\"debug\")\n  }\n}\n";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_comptime_side_effects(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should warn about print() in nested comptime block, got: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_validate_comptime_side_effects_fetch() {
        use shape_ast::parser::parse_program;

        let source = "comptime {\n  let data = fetch(\"http://example.com\")\n}\n";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_comptime_side_effects(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should warn about fetch() in comptime block"
        );
        assert!(diagnostics[0].message.contains("fetch"));
    }

    #[test]
    fn test_validate_comptime_builtins_outside_comptime() {
        use shape_ast::parser::parse_program;

        let source = r#"let x = implements("Point", "Display")"#;
        let program = parse_program(source).unwrap();
        let diagnostics = validate_comptime_builtins_context(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report error for comptime builtin outside comptime"
        );
        assert!(diagnostics[0].message.contains("comptime-only"));
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn test_validate_comptime_builtins_inside_comptime_ok() {
        use shape_ast::parser::parse_program;

        let source = "comptime {\n  let has = implements(\"Point\", \"Display\")\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_comptime_builtins_context(&program, source);

        assert_eq!(
            diagnostics.len(),
            0,
            "comptime builtin inside comptime should be allowed"
        );
    }

    #[test]
    fn test_validate_comptime_builtins_build_config_outside() {
        use shape_ast::parser::parse_program;

        let source = "let cfg = build_config()";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_comptime_builtins_context(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report error for build_config() outside comptime"
        );
    }

    #[test]
    fn test_validate_async_let_outside_async() {
        use shape_ast::parser::parse_program;

        let source = "fn foo() {\n  async let x = fetch(\"url\")\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_structured_concurrency(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report error for async let outside async: {:?}",
            diagnostics
        );
        assert!(diagnostics[0].message.contains("async let"));
        assert_eq!(
            diagnostics[0].code,
            Some(NumberOrString::String("E0201".to_string()))
        );
    }

    #[test]
    fn test_validate_async_let_inside_async() {
        use shape_ast::parser::parse_program;

        let source = "async fn foo() {\n  async let x = fetch(\"url\")\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_structured_concurrency(&program, source);

        assert!(
            diagnostics.is_empty(),
            "Should have no errors for async let inside async fn: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_validate_async_scope_outside_async() {
        use shape_ast::parser::parse_program;

        let source = "fn foo() {\n  let result = async scope { 42 }\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_structured_concurrency(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report error for async scope outside async: {:?}",
            diagnostics
        );
        assert!(diagnostics[0].message.contains("async scope"));
        assert_eq!(
            diagnostics[0].code,
            Some(NumberOrString::String("E0202".to_string()))
        );
    }

    #[test]
    fn test_validate_async_scope_inside_async() {
        use shape_ast::parser::parse_program;

        let source = "async fn foo() {\n  let result = async scope { 42 }\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_structured_concurrency(&program, source);

        assert!(
            diagnostics.is_empty(),
            "Should have no errors for async scope inside async fn: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_validate_for_await_outside_async() {
        use shape_ast::parser::parse_program;

        let source = "fn foo() {\n  for await x in stream {\n    x\n  }\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_structured_concurrency(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "Should report error for for-await outside async: {:?}",
            diagnostics
        );
        assert!(diagnostics[0].message.contains("for await"));
        assert_eq!(
            diagnostics[0].code,
            Some(NumberOrString::String("E0203".to_string()))
        );
    }

    #[test]
    fn test_validate_for_await_inside_async() {
        use shape_ast::parser::parse_program;

        let source = "async fn foo() {\n  for await x in stream {\n    x\n  }\n}";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_async_structured_concurrency(&program, source);

        assert!(
            diagnostics.is_empty(),
            "Should have no errors for for-await inside async fn: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_validate_interpolation_format_specs_ok() {
        use shape_ast::parser::parse_program;

        let source = r#"let s = f"value={price:fixed(2)}""#;
        let program = parse_program(source).unwrap();
        let diagnostics = validate_interpolation_format_specs(&program, source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_validate_interpolation_format_specs_reports_invalid_table_key() {
        use shape_ast::parser::parse_program;

        let source = r#"let s = f"{rows:table(foo=1)}""#;
        let program = parse_program(source).unwrap();
        let diagnostics = validate_interpolation_format_specs(&program, source);
        assert_eq!(diagnostics.len(), 1, "expected a single diagnostic");
        assert!(
            diagnostics[0].message.contains("Unknown table format key"),
            "unexpected diagnostic message: {}",
            diagnostics[0].message
        );
        assert_eq!(
            diagnostics[0].code,
            Some(NumberOrString::String("E0300".to_string()))
        );
        assert_eq!(
            diagnostics[0].range.start.line, 0,
            "diagnostic should point to formatted string line"
        );
    }

    #[test]
    fn test_validate_content_strings_empty_interpolation() {
        use shape_ast::parser::parse_program;

        let source = r#"let x = c"hello {}""#;
        let program = parse_program(source).unwrap();
        let diagnostics = validate_content_strings(&program, source);

        assert_eq!(
            diagnostics.len(),
            1,
            "expected 1 diagnostic for empty interpolation, got: {:?}",
            diagnostics
        );
        assert!(
            diagnostics[0].message.contains("Empty interpolation"),
            "unexpected message: {}",
            diagnostics[0].message
        );
        assert_eq!(
            diagnostics[0].code,
            Some(NumberOrString::String("E0310".to_string()))
        );
    }

    #[test]
    fn test_validate_content_strings_valid_interpolation_ok() {
        use shape_ast::parser::parse_program;

        let source = r#"let x = c"hello {name}""#;
        let program = parse_program(source).unwrap();
        let diagnostics = validate_content_strings(&program, source);

        assert!(
            diagnostics.is_empty(),
            "valid content string should produce no diagnostics: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_validate_color_rgb_out_of_range() {
        use shape_ast::parser::parse_program;

        let source = "let c = Color.rgb(300, 100, 256)";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_content_strings(&program, source);

        assert_eq!(
            diagnostics.len(),
            2,
            "expected 2 diagnostics for out-of-range RGB values (300 and 256), got: {:?}",
            diagnostics
        );
        assert!(diagnostics[0].message.contains("300"));
        assert!(diagnostics[1].message.contains("256"));
        assert_eq!(
            diagnostics[0].code,
            Some(NumberOrString::String("W0310".to_string()))
        );
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn test_validate_color_rgb_valid_range_ok() {
        use shape_ast::parser::parse_program;

        let source = "let c = Color.rgb(255, 128, 0)";
        let program = parse_program(source).unwrap();
        let diagnostics = validate_content_strings(&program, source);

        assert!(
            diagnostics.is_empty(),
            "valid Color.rgb should produce no diagnostics: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_borrow_analysis_to_diagnostics_empty() {
        let analysis = shape_vm::mir::analysis::BorrowAnalysis::empty();
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let diagnostics = borrow_analysis_to_diagnostics(&analysis, "", &uri);
        assert!(
            diagnostics.is_empty(),
            "Empty analysis should produce no diagnostics"
        );
    }

    #[test]
    fn test_borrow_analysis_to_diagnostics_with_error() {
        use shape_vm::mir::analysis::*;
        use shape_vm::mir::types::*;

        let mut analysis = BorrowAnalysis::empty();
        analysis.errors.push(BorrowError {
            kind: BorrowErrorKind::ConflictExclusiveExclusive,
            span: Span { start: 10, end: 20 },
            conflicting_loan: LoanId(0),
            loan_span: Span { start: 0, end: 5 },
            last_use_span: Some(Span { start: 25, end: 30 }),
            repairs: Vec::new(),
        });

        let source = "let mut x = 10\nlet m1 = &mut x\nlet m2 = &mut x\nprint(m1)\nprint(m2)";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let diagnostics = borrow_analysis_to_diagnostics(&analysis, source, &uri);

        assert_eq!(diagnostics.len(), 1, "Should produce one diagnostic");
        let diag = &diagnostics[0];
        assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(
            diag.code,
            Some(NumberOrString::String("B0001".to_string()))
        );
        assert_eq!(diag.source.as_deref(), Some("shape-borrow"));
        assert!(
            diag.message.contains("cannot mutably borrow"),
            "Message should describe the conflict: {}",
            diag.message
        );
        // Should have related information (loan origin + last use)
        let related = diag.related_information.as_ref().unwrap();
        assert_eq!(
            related.len(),
            2,
            "Should have loan origin + last use entries"
        );
        assert!(related[0].message.contains("conflicting borrow"));
        assert!(related[1].message.contains("still needed"));
    }

    #[test]
    fn test_borrow_analysis_to_diagnostics_mutability_error() {
        use shape_vm::mir::analysis::*;

        let mut analysis = BorrowAnalysis::empty();
        analysis.mutability_errors.push(MutabilityError {
            span: Span { start: 10, end: 15 },
            variable_name: "x".to_string(),
            declaration_span: Span { start: 0, end: 5 },
            is_explicit_let: true,
            is_const: false,
        });

        let source = "let x = 42\nx = 100\n";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let diagnostics = borrow_analysis_to_diagnostics(&analysis, source, &uri);

        assert_eq!(diagnostics.len(), 1);
        let diag = &diagnostics[0];
        assert!(diag.message.contains("cannot assign to let binding"));
        assert_eq!(
            diag.code,
            Some(NumberOrString::String("E0384".to_string()))
        );
        let related = diag.related_information.as_ref().unwrap();
        assert_eq!(related.len(), 1);
        assert!(related[0].message.contains("declared here"));
    }
}
