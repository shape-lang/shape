//! Context analysis for intelligent completions
//!
//! Determines the completion context based on cursor position and surrounding text.

use crate::util::position_to_offset;
use shape_ast::ast::InterpolationMode;
use tower_lsp_server::ls_types::Position;

#[derive(Debug, Clone, PartialEq)]
enum FormattedCursorContext {
    OutsideFormattedString,
    InFormattedLiteral,
    InInterpolationExpr { expr_prefix: String },
}

/// Completion context based on cursor position
#[derive(Debug, Clone, PartialEq)]
pub enum CompletionContext {
    /// General context - show all completions
    General,
    /// After a dot operator - show properties/methods
    PropertyAccess {
        /// The expression before the dot
        object: String,
    },
    /// Inside a function call - show parameters
    FunctionCall {
        /// The function being called
        function: String,
        /// Detailed argument context
        arg_context: ArgumentContext,
    },
    /// Inside a pattern definition
    PatternBody,
    /// Inside a query (find, scan, etc.)
    Query {
        /// The type of query (find, scan, analyze, backtest, alert)
        query_type: String,
    },
    /// After "pattern" keyword - suggest pattern names
    PatternReference,
    /// Type annotation context
    TypeAnnotation,
    /// After typing "@" at function/pattern start
    Annotation,
    /// Inside annotation arguments @foo(|)
    AnnotationArgs {
        /// The annotation being used
        annotation: String,
    },
    /// After "use " — suggest extension modules for namespace import
    ImportModule,
    /// After "from " — suggest importable modules for named import
    FromModule,
    /// After "from <module-prefix>." — suggest next namespace segments
    FromModulePartial {
        /// The prefix typed so far (e.g., "std.core", "mydep")
        prefix: String,
    },
    /// Inside "from <module> use { " — suggest module exports
    ImportItems {
        /// The module being imported from
        module: String,
    },
    /// After pipe operator `|>` — suggest functions/methods that accept the piped type
    PipeTarget {
        /// The inferred type of the expression before `|>`
        pipe_input_type: Option<String>,
    },
    /// Inside an impl block body — suggest unimplemented trait methods
    ImplBlock {
        /// The trait name being implemented
        trait_name: String,
        /// The target type implementing the trait
        target_type: String,
        /// Methods already implemented in self impl block
        existing_methods: Vec<String>,
    },
    /// Inside a type alias override: `type EUR = Currency { | }`
    /// Suggests comptime field names from the base type
    TypeAliasOverride {
        /// The base type being aliased (e.g., "Currency")
        base_type: String,
    },
    /// After `await join ` — suggest join strategies (all, race, any, settle)
    JoinStrategy,
    /// Inside a join block body — suggest labeled branch snippets
    JoinBody {
        /// The join strategy (all, race, any, settle)
        strategy: String,
    },
    /// In a trait bound position: `fn foo<T: |>` — suggest trait names
    TraitBound,
    /// Inside a `comptime { }` block — suggest comptime builtins + normal expressions
    ComptimeBlock,
    /// After `@` in expression position — suggest annotations for expression-level decoration
    ExprAnnotation,
    /// Inside formatted interpolation spec after `expr:` in `f"{expr:...}"`.
    InterpolationFormatSpec {
        /// The currently typed prefix after `:`.
        spec_prefix: String,
    },
}

/// Detailed context about argument position
#[derive(Debug, Clone, PartialEq)]
pub enum ArgumentContext {
    /// Cursor is on argument N of function call
    FunctionArgument { function: String, arg_index: usize },
    /// Inside object literal at property value position
    ObjectLiteralValue {
        containing_function: Option<String>,
        property_name: String,
    },
    /// Inside object literal at property name position
    ObjectLiteralPropertyName { containing_function: Option<String> },
    /// Unknown/general argument context
    General,
}

/// Analyze the text and position to determine completion context
pub fn analyze_context(text: &str, position: Position) -> CompletionContext {
    // Get the line where cursor is
    let lines: Vec<&str> = text.lines().collect();
    if position.line as usize >= lines.len() {
        return CompletionContext::General;
    }

    let current_line = lines[position.line as usize];
    let char_pos = position.character as usize;

    // Get text before cursor on current line
    let line_text_before_cursor = if char_pos <= current_line.len() {
        &current_line[..char_pos]
    } else {
        current_line
    };

    let cursor_offset = match position_to_offset(text, position) {
        Some(offset) => offset,
        None => return CompletionContext::General,
    };

    let (text_before_cursor, inside_interpolation) =
        match formatted_cursor_context(text, cursor_offset) {
            FormattedCursorContext::InFormattedLiteral => {
                // Keep completion strict inside non-expression string content.
                return CompletionContext::General;
            }
            FormattedCursorContext::InInterpolationExpr { expr_prefix } => (expr_prefix, true),
            FormattedCursorContext::OutsideFormattedString => {
                (line_text_before_cursor.to_string(), false)
            }
        };

    if inside_interpolation {
        if let Some(spec_prefix) = interpolation_format_spec_prefix(&text_before_cursor) {
            return CompletionContext::InterpolationFormatSpec { spec_prefix };
        }
    }

    if !inside_interpolation {
        // Check if we're in an import statement
        if let Some(import_ctx) = detect_import_context(&text_before_cursor) {
            return import_ctx;
        }
    }

    // Check if we're after a pipe operator `|>`
    if !inside_interpolation {
        if let Some(pipe_ctx) = detect_pipe_context(&text_before_cursor) {
            return pipe_ctx;
        }
    }

    // Check if we're after `await join ` — suggest join strategies
    if !inside_interpolation {
        let trimmed = text_before_cursor.trim_end();
        if trimmed.ends_with("join") || trimmed.ends_with("await join") {
            return CompletionContext::JoinStrategy;
        }
    }

    // Check if we're inside a join block body: `await join all { | }`
    if !inside_interpolation {
        if let Some(join_body_ctx) =
            detect_join_body_context(text, position.line as usize, char_pos)
        {
            return join_body_ctx;
        }
    }

    // Check if we're after a dot (property access)
    // But NOT if the text after the dot contains '(' — that means
    // we're inside a method call like `module.fn(`, which should be FunctionCall.
    if let Some(dot_pos) = text_before_cursor.rfind('.') {
        let after_dot = &text_before_cursor[dot_pos + 1..];
        if !after_dot.contains('(') {
            let before_dot = &text_before_cursor[..dot_pos];
            let object = extract_object_before_dot(before_dot);

            return CompletionContext::PropertyAccess {
                object: object.to_string(),
            };
        }
    }

    // Check if we're after "find" keyword (pattern reference)
    if !inside_interpolation && text_before_cursor.trim_end().ends_with("find") {
        return CompletionContext::PatternReference;
    }

    // Check if we're in a query context
    if !inside_interpolation {
        for query_type in &["find", "scan", "analyze", "backtest", "alert"] {
            if text_before_cursor.contains(query_type) {
                return CompletionContext::Query {
                    query_type: query_type.to_string(),
                };
            }
        }
    }

    // Check if we're inside a type alias override: `type EUR = Currency { | }`
    if !inside_interpolation {
        if let Some(base_type) =
            detect_type_alias_override_context(text, position.line as usize, char_pos)
        {
            return CompletionContext::TypeAliasOverride { base_type };
        }
    }

    // Check if we're inside an impl block
    if !inside_interpolation {
        if let Some(impl_ctx) = detect_impl_block_context(text, position.line as usize) {
            return impl_ctx;
        }
    }

    // Check if we're inside a comptime { } block
    if !inside_interpolation && is_inside_comptime_block(text, position.line as usize) {
        return CompletionContext::ComptimeBlock;
    }

    // Check if we're in a pattern body
    // Look backwards through lines to see if we're inside a pattern definition
    if !inside_interpolation && is_inside_pattern_body(text, position.line as usize) {
        return CompletionContext::PatternBody;
    }

    // Check if we're in a trait bound position: `fn foo<T: |>`
    if !inside_interpolation && is_in_trait_bound_position(&text_before_cursor) {
        return CompletionContext::TraitBound;
    }

    // Check if we're in a type annotation position
    if !inside_interpolation && is_in_type_annotation_position(&text_before_cursor) {
        return CompletionContext::TypeAnnotation;
    }

    // Check if we're inside a function call
    if let Some(func_name) = extract_function_call(&text_before_cursor) {
        let arg_context = analyze_argument_context(&text_before_cursor, &func_name);
        return CompletionContext::FunctionCall {
            function: func_name,
            arg_context,
        };
    }

    // Check if cursor is after "@" in expression position (not at statement start)
    if !inside_interpolation && is_at_expr_annotation_position(&text_before_cursor) {
        return CompletionContext::ExprAnnotation;
    }

    // Check if cursor is after "@" at statement start (item-level annotation)
    if !inside_interpolation && is_at_annotation_position(&text_before_cursor) {
        return CompletionContext::Annotation;
    }

    // Default to general context
    CompletionContext::General
}

/// Returns true when cursor is inside an interpolation expression body,
/// such as `f"{expr}"`, `f"${expr}"`, or `f#"{expr}"`.
pub fn is_inside_interpolation_expression(text: &str, position: Position) -> bool {
    let Some(cursor_offset) = position_to_offset(text, position) else {
        return false;
    };
    matches!(
        formatted_cursor_context(text, cursor_offset),
        FormattedCursorContext::InInterpolationExpr { .. }
    )
}

fn formatted_cursor_context(text: &str, cursor_offset: usize) -> FormattedCursorContext {
    #[derive(Debug, Clone, Copy)]
    enum State {
        Normal,
        String {
            escaped: bool,
        },
        TripleString,
        FormattedString {
            mode: InterpolationMode,
            escaped: bool,
            interpolation_depth: usize,
            interpolation_start: Option<usize>,
            expr_quote: Option<char>,
            expr_escaped: bool,
        },
        FormattedTripleString {
            mode: InterpolationMode,
            interpolation_depth: usize,
            interpolation_start: Option<usize>,
            expr_quote: Option<char>,
            expr_escaped: bool,
        },
    }

    fn formatted_prefix(rem: &str) -> Option<(InterpolationMode, bool, usize)> {
        if rem.starts_with("f$\"\"\"") {
            Some((InterpolationMode::Dollar, true, 5))
        } else if rem.starts_with("f#\"\"\"") {
            Some((InterpolationMode::Hash, true, 5))
        } else if rem.starts_with("f\"\"\"") {
            Some((InterpolationMode::Braces, true, 4))
        } else if rem.starts_with("f$\"") {
            Some((InterpolationMode::Dollar, false, 3))
        } else if rem.starts_with("f#\"") {
            Some((InterpolationMode::Hash, false, 3))
        } else if rem.starts_with("f\"") {
            Some((InterpolationMode::Braces, false, 2))
        } else {
            None
        }
    }

    let mut state = State::Normal;
    let mut i = 0usize;
    let capped_offset = cursor_offset.min(text.len());

    while i < capped_offset {
        let rem = &text[i..];
        state = match state {
            State::Normal => {
                if let Some((mode, true, prefix_len)) = formatted_prefix(rem) {
                    i += prefix_len;
                    State::FormattedTripleString {
                        mode,
                        interpolation_depth: 0,
                        interpolation_start: None,
                        expr_quote: None,
                        expr_escaped: false,
                    }
                } else if rem.starts_with("\"\"\"") {
                    i += 3;
                    State::TripleString
                } else if let Some((mode, false, prefix_len)) = formatted_prefix(rem) {
                    i += prefix_len;
                    State::FormattedString {
                        mode,
                        escaped: false,
                        interpolation_depth: 0,
                        interpolation_start: None,
                        expr_quote: None,
                        expr_escaped: false,
                    }
                } else if rem.starts_with('"') {
                    i += 1;
                    State::String { escaped: false }
                } else if let Some(ch) = rem.chars().next() {
                    i += ch.len_utf8();
                    State::Normal
                } else {
                    break;
                }
            }
            State::String { mut escaped } => {
                if let Some(ch) = rem.chars().next() {
                    if escaped {
                        escaped = false;
                        i += ch.len_utf8();
                        State::String { escaped }
                    } else if ch == '\\' {
                        i += 1;
                        State::String { escaped: true }
                    } else if ch == '"' {
                        i += 1;
                        State::Normal
                    } else {
                        i += ch.len_utf8();
                        State::String { escaped }
                    }
                } else {
                    break;
                }
            }
            State::TripleString => {
                if rem.starts_with("\"\"\"") {
                    i += 3;
                    State::Normal
                } else if let Some(ch) = rem.chars().next() {
                    i += ch.len_utf8();
                    State::TripleString
                } else {
                    break;
                }
            }
            State::FormattedString {
                mode,
                mut escaped,
                mut interpolation_depth,
                mut interpolation_start,
                mut expr_quote,
                mut expr_escaped,
            } => {
                if interpolation_depth == 0 {
                    if mode == InterpolationMode::Braces
                        && (rem.starts_with("{{") || rem.starts_with("}}"))
                    {
                        i += 2;
                        State::FormattedString {
                            mode,
                            escaped,
                            interpolation_depth,
                            interpolation_start,
                            expr_quote,
                            expr_escaped,
                        }
                    } else if mode != InterpolationMode::Braces {
                        let sigil = mode.sigil().expect("sigil mode must provide sigil");
                        let mut esc = String::new();
                        esc.push(sigil);
                        esc.push(sigil);
                        esc.push('{');
                        let mut opener = String::new();
                        opener.push(sigil);
                        opener.push('{');

                        if rem.starts_with(&esc) {
                            i += esc.len();
                            State::FormattedString {
                                mode,
                                escaped,
                                interpolation_depth,
                                interpolation_start,
                                expr_quote,
                                expr_escaped,
                            }
                        } else if rem.starts_with(&opener) {
                            interpolation_depth = 1;
                            interpolation_start = Some(i + opener.len());
                            i += opener.len();
                            State::FormattedString {
                                mode,
                                escaped,
                                interpolation_depth,
                                interpolation_start,
                                expr_quote,
                                expr_escaped,
                            }
                        } else if let Some(ch) = rem.chars().next() {
                            if escaped {
                                escaped = false;
                                i += ch.len_utf8();
                            } else if ch == '\\' {
                                escaped = true;
                                i += 1;
                            } else if ch == '"' {
                                return FormattedCursorContext::OutsideFormattedString;
                            } else {
                                i += ch.len_utf8();
                            }
                            State::FormattedString {
                                mode,
                                escaped,
                                interpolation_depth,
                                interpolation_start,
                                expr_quote,
                                expr_escaped,
                            }
                        } else {
                            break;
                        }
                    } else if let Some(ch) = rem.chars().next() {
                        if escaped {
                            escaped = false;
                            i += ch.len_utf8();
                        } else if ch == '\\' {
                            escaped = true;
                            i += 1;
                        } else if ch == '"' {
                            return FormattedCursorContext::OutsideFormattedString;
                        } else if ch == '{' {
                            interpolation_depth = 1;
                            interpolation_start = Some(i + 1);
                            i += 1;
                        } else {
                            i += ch.len_utf8();
                        }
                        State::FormattedString {
                            mode,
                            escaped,
                            interpolation_depth,
                            interpolation_start,
                            expr_quote,
                            expr_escaped,
                        }
                    } else {
                        break;
                    }
                } else if let Some(ch) = rem.chars().next() {
                    if let Some(quote) = expr_quote {
                        if expr_escaped {
                            expr_escaped = false;
                            i += ch.len_utf8();
                        } else if ch == '\\' {
                            expr_escaped = true;
                            i += 1;
                        } else if ch == quote {
                            expr_quote = None;
                            i += ch.len_utf8();
                        } else {
                            i += ch.len_utf8();
                        }
                    } else if ch == '"' || ch == '\'' {
                        expr_quote = Some(ch);
                        i += ch.len_utf8();
                    } else if ch == '{' {
                        interpolation_depth += 1;
                        i += 1;
                    } else if ch == '}' {
                        interpolation_depth = interpolation_depth.saturating_sub(1);
                        i += 1;
                        if interpolation_depth == 0 {
                            interpolation_start = None;
                        }
                    } else {
                        i += ch.len_utf8();
                    }
                    State::FormattedString {
                        mode,
                        escaped,
                        interpolation_depth,
                        interpolation_start,
                        expr_quote,
                        expr_escaped,
                    }
                } else {
                    break;
                }
            }
            State::FormattedTripleString {
                mode,
                mut interpolation_depth,
                mut interpolation_start,
                mut expr_quote,
                mut expr_escaped,
            } => {
                if interpolation_depth == 0 {
                    if rem.starts_with("\"\"\"") {
                        i += 3;
                        State::Normal
                    } else if mode == InterpolationMode::Braces
                        && (rem.starts_with("{{") || rem.starts_with("}}"))
                    {
                        i += 2;
                        State::FormattedTripleString {
                            mode,
                            interpolation_depth,
                            interpolation_start,
                            expr_quote,
                            expr_escaped,
                        }
                    } else if mode != InterpolationMode::Braces {
                        let sigil = mode.sigil().expect("sigil mode must provide sigil");
                        let mut esc = String::new();
                        esc.push(sigil);
                        esc.push(sigil);
                        esc.push('{');
                        let mut opener = String::new();
                        opener.push(sigil);
                        opener.push('{');

                        if rem.starts_with(&esc) {
                            i += esc.len();
                            State::FormattedTripleString {
                                mode,
                                interpolation_depth,
                                interpolation_start,
                                expr_quote,
                                expr_escaped,
                            }
                        } else if rem.starts_with(&opener) {
                            interpolation_depth = 1;
                            interpolation_start = Some(i + opener.len());
                            i += opener.len();
                            State::FormattedTripleString {
                                mode,
                                interpolation_depth,
                                interpolation_start,
                                expr_quote,
                                expr_escaped,
                            }
                        } else if let Some(ch) = rem.chars().next() {
                            i += ch.len_utf8();
                            State::FormattedTripleString {
                                mode,
                                interpolation_depth,
                                interpolation_start,
                                expr_quote,
                                expr_escaped,
                            }
                        } else {
                            break;
                        }
                    } else if let Some(ch) = rem.chars().next() {
                        if ch == '{' {
                            interpolation_depth = 1;
                            interpolation_start = Some(i + 1);
                            i += 1;
                        } else {
                            i += ch.len_utf8();
                        }
                        State::FormattedTripleString {
                            mode,
                            interpolation_depth,
                            interpolation_start,
                            expr_quote,
                            expr_escaped,
                        }
                    } else {
                        break;
                    }
                } else if let Some(ch) = rem.chars().next() {
                    if let Some(quote) = expr_quote {
                        if expr_escaped {
                            expr_escaped = false;
                            i += ch.len_utf8();
                        } else if ch == '\\' {
                            expr_escaped = true;
                            i += 1;
                        } else if ch == quote {
                            expr_quote = None;
                            i += ch.len_utf8();
                        } else {
                            i += ch.len_utf8();
                        }
                    } else if ch == '"' || ch == '\'' {
                        expr_quote = Some(ch);
                        i += ch.len_utf8();
                    } else if ch == '{' {
                        interpolation_depth += 1;
                        i += 1;
                    } else if ch == '}' {
                        interpolation_depth = interpolation_depth.saturating_sub(1);
                        i += 1;
                        if interpolation_depth == 0 {
                            interpolation_start = None;
                        }
                    } else {
                        i += ch.len_utf8();
                    }
                    State::FormattedTripleString {
                        mode,
                        interpolation_depth,
                        interpolation_start,
                        expr_quote,
                        expr_escaped,
                    }
                } else {
                    break;
                }
            }
        };
    }

    match state {
        State::FormattedString {
            interpolation_depth,
            interpolation_start,
            ..
        }
        | State::FormattedTripleString {
            interpolation_depth,
            interpolation_start,
            ..
        } => {
            if interpolation_depth == 0 {
                FormattedCursorContext::InFormattedLiteral
            } else if let Some(start) = interpolation_start {
                let prefix = text
                    .get(start..capped_offset)
                    .unwrap_or_default()
                    .to_string();
                FormattedCursorContext::InInterpolationExpr {
                    expr_prefix: prefix,
                }
            } else {
                FormattedCursorContext::InFormattedLiteral
            }
        }
        _ => FormattedCursorContext::OutsideFormattedString,
    }
}

/// Detect if cursor is inside a join block body: `await join all { ... }`
/// Returns JoinBody context with the strategy name if inside the block braces.
fn detect_join_body_context(
    text: &str,
    current_line: usize,
    cursor_char: usize,
) -> Option<CompletionContext> {
    let lines: Vec<&str> = text.lines().collect();
    let strategies = ["all", "race", "any", "settle"];

    // Walk backwards from current line looking for unclosed `join <strategy> {`
    let mut brace_depth: i32 = 0;
    let mut i = current_line;
    loop {
        let line = lines.get(i)?;
        // On cursor line, only count braces up to cursor position
        let effective = if i == current_line {
            let end = cursor_char.min(line.len());
            &line[..end]
        } else {
            line
        };
        for ch in effective.chars().rev() {
            match ch {
                '}' => brace_depth += 1,
                '{' => brace_depth -= 1,
                _ => {}
            }
        }
        // Found a net opening brace — check if self line has a join pattern
        if brace_depth < 0 {
            let trimmed = line.trim();
            for strategy in &strategies {
                // Match patterns like: `await join all {`, `join race {`
                let join_pattern = format!("join {} {{", strategy);
                let join_pattern_no_brace = format!("join {}", strategy);
                if trimmed.contains(&join_pattern)
                    || (trimmed.ends_with('{') && trimmed.contains(&join_pattern_no_brace))
                {
                    return Some(CompletionContext::JoinBody {
                        strategy: strategy.to_string(),
                    });
                }
            }
            return None;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

fn interpolation_format_spec_prefix(interpolation_expr_prefix: &str) -> Option<String> {
    let idx = shape_ast::interpolation::find_top_level_format_colon(interpolation_expr_prefix)?;
    let spec = interpolation_expr_prefix.get(idx + 1..)?.to_string();
    Some(spec)
}

/// Extract the object/expression before a dot
fn extract_object_before_dot(text: &str) -> &str {
    let trimmed = text.trim_end();

    // Find the start of the identifier by looking backwards for whitespace or operators
    // but we need to handle array indexing like data[0]
    let mut bracket_depth = 0;
    let mut start = trimmed.len();

    for (i, ch) in trimmed.char_indices().rev() {
        if ch == ']' {
            bracket_depth += 1;
        } else if ch == '[' {
            bracket_depth -= 1;
        } else if bracket_depth == 0 && (ch.is_whitespace() || "(){}< >+-*/=!,;".contains(ch)) {
            start = i + ch.len_utf8();
            break;
        }
        if i == 0 {
            start = 0;
        }
    }

    &trimmed[start..]
}

/// Public API: check if cursor is inside a type alias override, returning the base type name
pub fn detect_type_alias_override_context_pub(
    text: &str,
    line: usize,
    cursor_char: usize,
) -> Option<String> {
    detect_type_alias_override_context(text, line, cursor_char)
}

/// Detect if cursor is inside a type alias override: `type EUR = Currency { | }`
/// Returns the base type name (e.g., "Currency") if inside the override braces.
/// `cursor_char` is the cursor's character offset on the current line, used to
/// only count braces before the cursor when the override is on a single line.
fn detect_type_alias_override_context(
    text: &str,
    current_line: usize,
    cursor_char: usize,
) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();

    // Walk backwards from current line to find `type X = Y {`
    // This pattern is on a single line typically, but the braces content may span lines
    let mut brace_depth: i32 = 0;
    let mut i = current_line;
    loop {
        let line = lines.get(i)?;
        // On the cursor line, only count braces up to the cursor position
        // so that a closing brace after the cursor doesn't cancel the opening brace
        let effective_line = if i == current_line {
            let end = cursor_char.min(line.len());
            &line[..end]
        } else {
            line
        };
        // Count braces in reverse order
        for ch in effective_line.chars().rev() {
            match ch {
                '}' => brace_depth += 1,
                '{' => brace_depth -= 1,
                _ => {}
            }
        }
        // If we found a net opening brace, check if self line has the type alias pattern
        if brace_depth < 0 {
            let trimmed = line.trim();
            // Match pattern: `type NAME = BASE_TYPE {`
            if trimmed.starts_with("type ") {
                let rest = trimmed.strip_prefix("type ")?.trim();
                // Skip the alias name
                let after_name = rest
                    .split(|c: char| c.is_whitespace() || c == '<')
                    .next()
                    .map(|name| &rest[name.len()..])?;
                // Skip generic params if present
                let after_generics = if after_name.trim_start().starts_with('<') {
                    // Find closing '>'
                    let mut depth = 0;
                    let mut end = 0;
                    for (j, c) in after_name.trim_start().char_indices() {
                        match c {
                            '<' => depth += 1,
                            '>' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = j + 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    &after_name.trim_start()[end..]
                } else {
                    after_name
                };
                // Expect `= BASE_TYPE {`
                let after_eq = after_generics.trim_start().strip_prefix('=')?;
                let base_type = after_eq
                    .trim()
                    .split(|c: char| c == '{' || c.is_whitespace())
                    .next()?
                    .trim();
                if !base_type.is_empty() {
                    return Some(base_type.to_string());
                }
            }
            return None;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

/// Detect if cursor is inside an impl block and return context with trait/type info
fn detect_impl_block_context(text: &str, current_line: usize) -> Option<CompletionContext> {
    let lines: Vec<&str> = text.lines().collect();

    let mut in_impl = false;
    let mut trait_name = String::new();
    let mut target_type = String::new();
    let mut existing_methods = Vec::new();
    let mut brace_count: i32 = 0;

    for (i, line) in lines.iter().enumerate() {
        if i > current_line {
            break;
        }

        let trimmed = line.trim();
        // Check for "impl TraitName for TypeName {" pattern
        if trimmed.starts_with("impl ") && !in_impl {
            // Parse: impl TraitName for TypeName {
            let rest = trimmed.strip_prefix("impl ").unwrap().trim();
            let parts: Vec<&str> = rest.splitn(4, ' ').collect();
            if parts.len() >= 3 && parts[1] == "for" {
                trait_name = parts[0].to_string();
                // Extract type name (strip trailing `{` if present)
                target_type = parts[2].trim_end_matches('{').trim().to_string();
                in_impl = true;
                existing_methods.clear();
            }
        }

        // Collect method names inside the impl block
        if in_impl && trimmed.starts_with("method ") {
            let method_rest = trimmed.strip_prefix("method ").unwrap().trim();
            if let Some(name) = method_rest
                .split(|c: char| c == '(' || c.is_whitespace())
                .next()
            {
                if !name.is_empty() {
                    existing_methods.push(name.to_string());
                }
            }
        }

        brace_count += line.matches('{').count() as i32;
        brace_count -= line.matches('}').count() as i32;

        if in_impl && brace_count == 0 && line.contains('}') {
            in_impl = false;
            trait_name.clear();
            target_type.clear();
            existing_methods.clear();
        }
    }

    if in_impl && brace_count > 0 && !trait_name.is_empty() {
        Some(CompletionContext::ImplBlock {
            trait_name,
            target_type,
            existing_methods,
        })
    } else {
        None
    }
}

/// Check if cursor is inside a pattern body
fn is_inside_pattern_body(text: &str, current_line: usize) -> bool {
    let lines: Vec<&str> = text.lines().collect();

    let mut in_pattern = false;
    let mut brace_count = 0;

    for (i, line) in lines.iter().enumerate() {
        if i > current_line {
            break;
        }

        if line.trim().starts_with("pattern") {
            in_pattern = true;
        }

        brace_count += line.matches('{').count() as i32;
        brace_count -= line.matches('}').count() as i32;

        if in_pattern && brace_count == 0 && line.contains('}') {
            in_pattern = false;
        }
    }

    in_pattern && brace_count > 0
}

/// Extract function name if cursor is inside a function call.
/// Returns the full qualified name including module prefix (e.g., "csv.load").
fn extract_function_call(text: &str) -> Option<String> {
    // Find the last opening parenthesis
    if let Some(paren_pos) = text.rfind('(') {
        let before_paren = text[..paren_pos].trim_end();

        // Find the start of the expression (go backwards through identifier chars and dots)
        let start = before_paren
            .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
            .map(|i| i + 1)
            .unwrap_or(0);
        let func_name = &before_paren[start..];

        if !func_name.is_empty() {
            return Some(func_name.to_string());
        }
    }
    None
}

/// Analyze detailed argument context (position, object literals, etc.)
fn analyze_argument_context(text_before_cursor: &str, function: &str) -> ArgumentContext {
    // Find the opening paren of the function call
    if let Some(paren_pos) = text_before_cursor.rfind('(') {
        let params_text = &text_before_cursor[paren_pos + 1..];

        // Check if we're inside an object literal
        if is_inside_object_literal(params_text) {
            // Try to extract property name
            if let Some(property_name) = extract_property_name(params_text) {
                return ArgumentContext::ObjectLiteralValue {
                    containing_function: Some(function.to_string()),
                    property_name,
                };
            } else {
                return ArgumentContext::ObjectLiteralPropertyName {
                    containing_function: Some(function.to_string()),
                };
            }
        }

        // Count commas to determine argument index
        let arg_index = count_commas_outside_nested(params_text);

        return ArgumentContext::FunctionArgument {
            function: function.to_string(),
            arg_index,
        };
    }

    ArgumentContext::General
}

/// Check if cursor is inside an object literal
fn is_inside_object_literal(text: &str) -> bool {
    let open_braces = text.matches('{').count();
    let close_braces = text.matches('}').count();
    open_braces > close_braces
}

/// Extract property name from object literal context
fn extract_property_name(text: &str) -> Option<String> {
    // Find last '{' or ','
    let start = text.rfind(['{', ',']).map(|i| i + 1).unwrap_or(0);
    let fragment = text[start..].trim();

    // Check if there's a ':' (we're past property name, at value position)
    if let Some(colon_pos) = fragment.find(':') {
        let prop = fragment[..colon_pos].trim();
        if !prop.is_empty() {
            return Some(prop.to_string());
        }
    }

    None
}

/// Count commas outside of nested parens/braces to determine argument position
fn count_commas_outside_nested(text: &str) -> usize {
    let mut count: usize = 0;
    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for ch in text.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => escape_next = true,
            '"' | '\'' => in_string = !in_string,
            '(' if !in_string => paren_depth += 1,
            ')' if !in_string => paren_depth = paren_depth.saturating_sub(1),
            '{' if !in_string => brace_depth += 1,
            '}' if !in_string => brace_depth = brace_depth.saturating_sub(1),
            '[' if !in_string => bracket_depth += 1,
            ']' if !in_string => bracket_depth = bracket_depth.saturating_sub(1),
            ',' if !in_string && paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                count += 1
            }
            _ => {}
        }
    }

    count
}

/// Check if cursor is in a type annotation position
/// Returns true when:
/// - Cursor is right after a colon in a variable/parameter declaration
/// - Cursor is after a colon and user has started typing a type name
/// - Cursor is after a return type arrow "->"
fn is_in_type_annotation_position(text: &str) -> bool {
    let trimmed = text.trim_end();

    // Case 1: Right after colon
    if trimmed.ends_with(':') {
        return true;
    }

    // Case 2: After colon in variable/param declaration
    // Look for pattern: "let/const name:" or "(param:" with no "=" after
    if let Some(colon_idx) = find_unquoted_colon(trimmed) {
        let after_colon = &trimmed[colon_idx + 1..];
        // Not in type position if we've passed the type (hit '=' or '{')
        if after_colon.contains('=') || after_colon.contains('{') {
            return false;
        }
        let before_colon = &trimmed[..colon_idx];
        // Check for variable declaration pattern or parameter context
        if is_var_decl_context(before_colon) || is_param_context(before_colon) {
            return true;
        }
    }

    // Case 3: After return type arrow "->"
    if let Some(arrow_idx) = trimmed.rfind("->") {
        let after_arrow = &trimmed[arrow_idx + 2..];
        if !after_arrow.contains('=') && !after_arrow.contains('{') {
            return true;
        }
    }

    false
}

/// Find the last unquoted colon in the text
fn find_unquoted_colon(text: &str) -> Option<usize> {
    let mut in_string = false;
    let mut last_colon = None;

    for (i, ch) in text.char_indices() {
        if ch == '"' {
            in_string = !in_string;
        }
        if ch == ':' && !in_string {
            last_colon = Some(i);
        }
    }
    last_colon
}

/// Check if the text before colon indicates a variable declaration context
fn is_var_decl_context(before_colon: &str) -> bool {
    let trimmed = before_colon.trim();
    // Match "let x" or "const x" patterns
    trimmed.starts_with("let ") || trimmed.starts_with("const ")
}

/// Check if we're in a parameter context (inside unclosed parentheses)
fn is_param_context(before_colon: &str) -> bool {
    // We're in param context if there's an unclosed "("
    let open = before_colon.matches('(').count();
    let close = before_colon.matches(')').count();
    open > close
}

/// Detect import statement contexts:
///   "use "                 → ImportModule (extension modules for namespace import)
///   "from "                → FromModule (importable modules for named import)
///   "from std."            → FromModulePartial { prefix: "std" }
///   "from mydep.tools."    → FromModulePartial { prefix: "mydep.tools" }
///   "from csv use {"       → ImportItems { module: "csv" }
fn detect_import_context(text_before_cursor: &str) -> Option<CompletionContext> {
    let trimmed = text_before_cursor.trim();

    // "from <module> use { <TAB>" or "from <module> use { a, <TAB>"
    if let Some(rest) = trimmed.strip_prefix("from ") {
        if let Some(use_pos) = rest.find(" use") {
            let module = rest[..use_pos].trim().to_string();
            if !module.is_empty() && rest[use_pos..].contains('{') {
                return Some(CompletionContext::ImportItems { module });
            }
        }
        // Check for partial module path with dots: "from std." or "from mydep.tools."
        let module_text = rest.trim();
        if module_text.contains('.') {
            // Extract prefix: "mydep.tools." → "mydep.tools", "mydep.tools.sub" → "mydep.tools"
            // If ends with dot, prefix is everything before the trailing dot
            // If not, prefix is everything up to and including the last dot segment minus the partial
            let prefix = if module_text.ends_with('.') {
                module_text.trim_end_matches('.').to_string()
            } else if let Some(dot_pos) = module_text.rfind('.') {
                module_text[..dot_pos].to_string()
            } else {
                module_text.to_string()
            };
            return Some(CompletionContext::FromModulePartial { prefix });
        }
        // "from <TAB>" — suggest importable modules
        return Some(CompletionContext::FromModule);
    }

    if let Some(rest) = trimmed.strip_prefix("use ") {
        let rest = rest.trim();
        if !rest.starts_with('{') {
            return Some(CompletionContext::ImportModule);
        }
    }

    // Bare keywords
    if trimmed == "use" {
        return Some(CompletionContext::ImportModule);
    }
    if trimmed == "from" {
        return Some(CompletionContext::FromModule);
    }

    None
}

/// Detect if cursor is after a pipe operator `|>`.
/// Returns PipeTarget context with the inferred input type (placeholder; type
/// is resolved later using the type_context in the completion handler).
fn detect_pipe_context(text_before_cursor: &str) -> Option<CompletionContext> {
    let trimmed = text_before_cursor.trim_end();

    // Check for `|> ` or `|>` at end — the cursor is right after the pipe
    if trimmed.ends_with("|>") {
        return Some(CompletionContext::PipeTarget {
            pipe_input_type: None,
        });
    }

    // Check for `|> partial_ident` — user is typing after pipe
    // Find last `|>` and check nothing complex follows (no dots, parens, etc.)
    if let Some(pipe_pos) = trimmed.rfind("|>") {
        let after_pipe = trimmed[pipe_pos + 2..].trim();
        // If what follows is just an identifier prefix (no dots, parens, braces)
        // then we're still in pipe target context
        if !after_pipe.is_empty() && after_pipe.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Some(CompletionContext::PipeTarget {
                pipe_input_type: None,
            });
        }
    }

    None
}

/// Check if cursor is at a position where annotations can be written
fn is_at_annotation_position(text: &str) -> bool {
    let trimmed = text.trim_end();

    // Check if we just typed "@"
    if trimmed.ends_with('@') {
        // Check if it's at the start of a line or after whitespace (valid annotation position)
        let before_at = trimmed.trim_end_matches('@').trim_end();
        return before_at.is_empty() || before_at.ends_with('\n');
    }

    false
}

/// Check if cursor is in a trait bound position inside angle brackets.
/// Returns true for patterns like: `fn foo<T: |>`, `fn foo<T: Comparable + |>`, `trait Foo<T: |>`
fn is_in_trait_bound_position(text: &str) -> bool {
    let trimmed = text.trim_end();

    // Must be inside unclosed `<` brackets (angle_depth > 0)
    let mut angle_depth: i32 = 0;
    let mut last_angle_open = None;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '<' => {
                angle_depth += 1;
                last_angle_open = Some(i);
            }
            '>' => angle_depth -= 1,
            _ => {}
        }
    }
    if angle_depth <= 0 {
        return false;
    }

    // Extract text inside the last unclosed `<`
    let inside_angles = if let Some(start) = last_angle_open {
        &trimmed[start + 1..]
    } else {
        return false;
    };

    // Look for a colon after a type param name: `T:` or `T: Comp + `
    // The last segment (after last comma) should contain a ':'
    let last_segment = inside_angles
        .rsplit(',')
        .next()
        .unwrap_or(inside_angles)
        .trim();

    if let Some(colon_pos) = last_segment.find(':') {
        let before_colon = last_segment[..colon_pos].trim();
        // Before colon should be a simple identifier (type param name)
        if !before_colon.is_empty()
            && before_colon
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_')
        {
            // Check that the text before `<` starts with fn, function, trait, or type
            let before_angle = if let Some(start) = last_angle_open {
                trimmed[..start].trim()
            } else {
                ""
            };
            let is_type_param_context = before_angle.starts_with("fn ")
                || before_angle.starts_with("function ")
                || before_angle.starts_with("trait ")
                || before_angle.starts_with("type ")
                || before_angle.contains(" fn ")
                || before_angle.contains(" function ")
                || before_angle.ends_with("fn")
                || before_angle.ends_with("function")
                || before_angle.ends_with("trait")
                || before_angle.ends_with("type");
            return is_type_param_context;
        }
    }

    false
}

/// Check if the cursor is inside a `comptime { }` block.
///
/// Scans backwards through lines to find an unmatched `comptime {` opening.
fn is_inside_comptime_block(text: &str, current_line: usize) -> bool {
    let lines: Vec<&str> = text.lines().collect();

    let mut in_comptime = false;
    let mut brace_count: i32 = 0;

    for (i, line) in lines.iter().enumerate() {
        if i > current_line {
            break;
        }

        let trimmed = line.trim();
        // Check for `comptime {` pattern (item-level or expression-level)
        if (trimmed.starts_with("comptime {")
            || trimmed.starts_with("comptime{")
            || trimmed == "comptime")
            && !in_comptime
        {
            in_comptime = true;
        }
        // Also check for `= comptime {` or `let x = comptime {` (expression position)
        if !in_comptime && trimmed.contains("comptime {") {
            in_comptime = true;
        }

        brace_count += line.matches('{').count() as i32;
        brace_count -= line.matches('}').count() as i32;

        if in_comptime && brace_count == 0 && line.contains('}') {
            in_comptime = false;
        }
    }

    in_comptime && brace_count > 0
}

/// Check if cursor is after `@` in expression position (not at statement start).
///
/// Expression-level annotations: `let x = @timeout(5s) fetch()`
/// vs item-level annotations: `@strategy\nfn foo() { }`
fn is_at_expr_annotation_position(text: &str) -> bool {
    let trimmed = text.trim_end();

    if !trimmed.ends_with('@') {
        return false;
    }

    // Get text before the `@`
    let before_at = trimmed.trim_end_matches('@').trim_end();

    // If it's at the start of a line (empty before_at or ends with newline),
    // that's an item-level annotation, not expression-level.
    if before_at.is_empty() || before_at.ends_with('\n') {
        return false;
    }

    // Expression-level: after `=`, `(`, `,`, `return`, `=>`, or other expression starters
    let last_char = before_at.chars().last().unwrap_or(' ');
    matches!(last_char, '=' | '(' | ',' | '>' | '{' | '[' | ';' | '|')
        || before_at.ends_with("return")
        || before_at.ends_with("return ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_general_context() {
        let text = "let x = ";
        let position = Position {
            line: 0,
            character: 8,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::General);
    }

    #[test]
    fn test_property_access_context() {
        let text = "data[0].";
        let position = Position {
            line: 0,
            character: 8,
        };

        let context = analyze_context(text, position);
        match context {
            CompletionContext::PropertyAccess { object } => {
                assert_eq!(object, "data[0]");
            }
            _ => panic!("Expected PropertyAccess context"),
        }
    }

    #[test]
    fn test_property_access_context_inside_formatted_string_expression() {
        let text = r#"let msg = f"value: {user.}";"#;
        let position = Position {
            line: 0,
            character: text.find("user.").unwrap() as u32 + 5,
        };

        let context = analyze_context(text, position);
        match context {
            CompletionContext::PropertyAccess { object } => {
                assert_eq!(object, "user");
            }
            _ => panic!("Expected PropertyAccess context inside interpolation"),
        }
    }

    #[test]
    fn test_no_property_context_inside_formatted_string_literal_text() {
        let text = r#"let msg = f"price.path {user}";"#;
        let position = Position {
            line: 0,
            character: text.find("path").unwrap() as u32 + 2,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::General);
    }

    #[test]
    fn test_function_call_context_inside_formatted_string_expression() {
        let text = r#"let msg = f"value: {sma(}";"#;
        let position = Position {
            line: 0,
            character: text.find("sma(").unwrap() as u32 + 4,
        };

        let context = analyze_context(text, position);
        match context {
            CompletionContext::FunctionCall { function, .. } => {
                assert_eq!(function, "sma");
            }
            _ => panic!("Expected FunctionCall context inside interpolation"),
        }
    }

    #[test]
    fn test_property_access_context_inside_dollar_formatted_string_expression() {
        let text = r#"let msg = f$"value: ${user.}";"#;
        let position = Position {
            line: 0,
            character: text.find("user.").unwrap() as u32 + 5,
        };

        let context = analyze_context(text, position);
        match context {
            CompletionContext::PropertyAccess { object } => {
                assert_eq!(object, "user");
            }
            _ => panic!("Expected PropertyAccess context inside dollar interpolation"),
        }
    }

    #[test]
    fn test_function_call_context_inside_hash_formatted_string_expression() {
        let text = "let cmd = f#\"run #{build(}\"";
        let position = Position {
            line: 0,
            character: text.find("build(").unwrap() as u32 + 6,
        };

        let context = analyze_context(text, position);
        match context {
            CompletionContext::FunctionCall { function, .. } => {
                assert_eq!(function, "build");
            }
            _ => panic!("Expected FunctionCall context inside hash interpolation"),
        }
    }

    #[test]
    fn test_pattern_reference_context() {
        let text = "find ";
        let position = Position {
            line: 0,
            character: 5,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::PatternReference);
    }

    #[test]
    fn test_function_call_context() {
        let text = "sma(";
        let position = Position {
            line: 0,
            character: 4,
        };

        let context = analyze_context(text, position);
        match context {
            CompletionContext::FunctionCall { function, .. } => {
                assert_eq!(function, "sma");
            }
            _ => panic!("Expected FunctionCall context"),
        }
    }

    #[test]
    fn test_extract_object_before_dot() {
        assert_eq!(extract_object_before_dot("data[0]"), "data[0]");
        assert_eq!(extract_object_before_dot("let x = myvar"), "myvar");
        assert_eq!(extract_object_before_dot("data"), "data");
    }

    #[test]
    fn test_type_annotation_context_after_colon() {
        let text = "let series: ";
        let position = Position {
            line: 0,
            character: 12,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::TypeAnnotation);
    }

    #[test]
    fn test_type_annotation_context_typing_type() {
        // User has typed "let series: S" - should still be in type annotation context
        let text = "let series: S";
        let position = Position {
            line: 0,
            character: 13,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::TypeAnnotation);
    }

    #[test]
    fn test_type_annotation_context_typing_full_type() {
        // User has typed "let table: Table" - should still be in type annotation context
        let text = "let table: Table";
        let position = Position {
            line: 0,
            character: 16,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::TypeAnnotation);
    }

    #[test]
    fn test_type_annotation_context_after_equals() {
        // After "=", we're no longer in type annotation context
        let text = "let table: Table = ";
        let position = Position {
            line: 0,
            character: 19,
        };

        let context = analyze_context(text, position);
        assert_ne!(context, CompletionContext::TypeAnnotation);
    }

    #[test]
    fn test_type_annotation_context_function_param() {
        // Function parameter with type annotation
        let text = "function foo(x: ";
        let position = Position {
            line: 0,
            character: 16,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::TypeAnnotation);
    }

    #[test]
    fn test_type_annotation_context_return_type() {
        // After return type arrow
        let text = "function foo() -> ";
        let position = Position {
            line: 0,
            character: 18,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::TypeAnnotation);
    }

    #[test]
    fn test_type_annotation_context_return_type_typing() {
        // Typing after return type arrow
        let text = "function foo() -> Res";
        let position = Position {
            line: 0,
            character: 21,
        };

        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::TypeAnnotation);
    }

    #[test]
    fn test_use_module_context() {
        let context = analyze_context(
            "use ",
            Position {
                line: 0,
                character: 4,
            },
        );
        assert_eq!(context, CompletionContext::ImportModule);
    }

    #[test]
    fn test_from_module_context() {
        let context = analyze_context(
            "from ",
            Position {
                line: 0,
                character: 5,
            },
        );
        assert_eq!(context, CompletionContext::FromModule);
    }

    #[test]
    fn test_from_import_no_longer_triggers_items() {
        // The deprecated `from X import { }` syntax is removed;
        // LSP should fall back to FromModule context
        let context = analyze_context(
            "from csv import { ",
            Position {
                line: 0,
                character: 18,
            },
        );
        assert_eq!(context, CompletionContext::FromModule);
    }

    #[test]
    fn test_from_use_items_context() {
        let context = analyze_context(
            "from csv use { ",
            Position {
                line: 0,
                character: 15,
            },
        );
        assert_eq!(
            context,
            CompletionContext::ImportItems {
                module: "csv".to_string()
            }
        );
    }

    #[test]
    fn test_use_not_object_literal() {
        // "use ml" should still be ImportModule, not General
        let context = analyze_context(
            "use ml",
            Position {
                line: 0,
                character: 6,
            },
        );
        assert_eq!(context, CompletionContext::ImportModule);
    }

    #[test]
    fn test_module_dot_access() {
        let context = analyze_context(
            "csv.",
            Position {
                line: 0,
                character: 4,
            },
        );
        assert_eq!(
            context,
            CompletionContext::PropertyAccess {
                object: "csv".to_string()
            }
        );
    }

    #[test]
    fn test_module_method_call_context() {
        let context = analyze_context(
            "duckdb.query(",
            Position {
                line: 0,
                character: 13,
            },
        );
        match context {
            CompletionContext::FunctionCall { function, .. } => {
                assert!(
                    function.contains("duckdb.query"),
                    "Expected function to contain 'duckdb.query', got '{}'",
                    function
                );
            }
            _ => panic!("Expected FunctionCall context, got {:?}", context),
        }
    }

    #[test]
    fn test_pipe_context_detection() {
        let context = analyze_context(
            "data |> ",
            Position {
                line: 0,
                character: 8,
            },
        );
        assert!(
            matches!(context, CompletionContext::PipeTarget { .. }),
            "Expected PipeTarget context, got {:?}",
            context
        );
    }

    #[test]
    fn test_pipe_context_with_chain() {
        let context = analyze_context(
            "data |> filter(p) |> ",
            Position {
                line: 0,
                character: 21,
            },
        );
        assert!(
            matches!(context, CompletionContext::PipeTarget { .. }),
            "Expected PipeTarget context after chained pipe, got {:?}",
            context
        );
    }

    #[test]
    fn test_pipe_not_detected_in_bitwise_or() {
        // `a | b` should NOT be PipeTarget — that's bitwise OR, not pipe
        let context = analyze_context(
            "a | b",
            Position {
                line: 0,
                character: 5,
            },
        );
        assert!(
            !matches!(context, CompletionContext::PipeTarget { .. }),
            "Bitwise OR should NOT be PipeTarget, got {:?}",
            context
        );
    }

    #[test]
    fn test_pipe_context_typing_identifier() {
        // User is typing an identifier after pipe: `data |> fi`
        let context = analyze_context(
            "data |> fi",
            Position {
                line: 0,
                character: 10,
            },
        );
        assert!(
            matches!(context, CompletionContext::PipeTarget { .. }),
            "Expected PipeTarget while typing after pipe, got {:?}",
            context
        );
    }

    #[test]
    fn test_fstring_empty_interpolation() {
        // Cursor inside empty interpolation: f"hello {|}"
        let text = r#"let s = f"hello {}""#;
        let cursor = text.find("{}").unwrap() as u32 + 1; // after {
        let context = analyze_context(
            text,
            Position {
                line: 0,
                character: cursor,
            },
        );
        // Empty interpolation should still offer general completions (not literal text)
        assert_eq!(
            context,
            CompletionContext::General,
            "Empty f-string interpolation should give General context for variable completions"
        );
    }

    #[test]
    fn test_fstring_identifier_completion() {
        // Cursor typing identifier in interpolation: f"hello {x|}"
        let text = r#"let s = f"hello {x}""#;
        let cursor = text.find("{x").unwrap() as u32 + 2; // after x
        let context = analyze_context(
            text,
            Position {
                line: 0,
                character: cursor,
            },
        );
        // Should be General context (variable name completion)
        assert_eq!(
            context,
            CompletionContext::General,
            "f-string identifier should give General context, got {:?}",
            context
        );
    }

    #[test]
    fn test_fstring_method_call() {
        // Cursor inside method call in interpolation: f"val: {obj.method(|)}"
        let text = r#"let s = f"val: {obj.method()}""#;
        let cursor = text.find("method(").unwrap() as u32 + 7; // after (
        let context = analyze_context(
            text,
            Position {
                line: 0,
                character: cursor,
            },
        );
        match context {
            CompletionContext::FunctionCall { function, .. } => {
                assert!(
                    function.contains("method"),
                    "Expected function to contain 'method', got '{}'",
                    function
                );
            }
            _ => panic!(
                "Expected FunctionCall context in f-string interpolation, got {:?}",
                context
            ),
        }
    }

    #[test]
    fn test_fstring_format_spec_context_after_colon() {
        let text = r#"let s = f"value: {price:}""#;
        let cursor = text.find("price:").unwrap() as u32 + 6; // right after ':'
        let context = analyze_context(
            text,
            Position {
                line: 0,
                character: cursor,
            },
        );
        assert!(
            matches!(context, CompletionContext::InterpolationFormatSpec { .. }),
            "Expected InterpolationFormatSpec context, got {:?}",
            context
        );
    }

    #[test]
    fn test_fstring_table_format_spec_context() {
        let text = r#"let s = f"{rows:table(align=)}""#;
        let cursor = text.find("align=").unwrap() as u32 + 6;
        let context = analyze_context(
            text,
            Position {
                line: 0,
                character: cursor,
            },
        );
        assert!(
            matches!(context, CompletionContext::InterpolationFormatSpec { .. }),
            "Expected InterpolationFormatSpec context, got {:?}",
            context
        );
    }

    #[test]
    fn test_impl_block_context() {
        let text = "trait Q {\n    filter(p): any\n}\nimpl Q for T {\n    \n}\n";
        let position = Position {
            line: 4,
            character: 4,
        };
        let context = analyze_context(text, position);
        match context {
            CompletionContext::ImplBlock {
                trait_name,
                target_type,
                existing_methods,
            } => {
                assert_eq!(trait_name, "Q");
                assert_eq!(target_type, "T");
                assert!(existing_methods.is_empty());
            }
            _ => panic!("Expected ImplBlock context, got {:?}", context),
        }
    }

    #[test]
    fn test_impl_block_context_with_existing_methods() {
        let text = "impl Queryable for MyTable {\n    method filter(pred) { self }\n    \n}\n";
        let position = Position {
            line: 2,
            character: 4,
        };
        let context = analyze_context(text, position);
        match context {
            CompletionContext::ImplBlock {
                trait_name,
                existing_methods,
                ..
            } => {
                assert_eq!(trait_name, "Queryable");
                assert_eq!(existing_methods, vec!["filter".to_string()]);
            }
            _ => panic!("Expected ImplBlock context, got {:?}", context),
        }
    }

    #[test]
    fn test_impl_block_context_after_close() {
        // After closing brace, should NOT be in impl block
        let text = "impl Q for T {\n    method foo() { self }\n}\nlet x = ";
        let position = Position {
            line: 3,
            character: 8,
        };
        let context = analyze_context(text, position);
        assert!(
            !matches!(context, CompletionContext::ImplBlock { .. }),
            "Should not be ImplBlock after closing brace, got {:?}",
            context
        );
    }

    #[test]
    fn test_type_alias_override_context() {
        let text = "type Currency { comptime symbol: string = \"$\", amount: number }\ntype EUR = Currency { ";
        let position = Position {
            line: 1,
            character: 22,
        };
        let context = analyze_context(text, position);
        match context {
            CompletionContext::TypeAliasOverride { base_type } => {
                assert_eq!(base_type, "Currency");
            }
            _ => panic!("Expected TypeAliasOverride context, got {:?}", context),
        }
    }

    #[test]
    fn test_type_alias_override_context_not_struct_def() {
        // A normal struct type definition should NOT trigger TypeAliasOverride
        let text = "type Currency { ";
        let position = Position {
            line: 0,
            character: 16,
        };
        let context = analyze_context(text, position);
        assert!(
            !matches!(context, CompletionContext::TypeAliasOverride { .. }),
            "Struct def should not be TypeAliasOverride, got {:?}",
            context
        );
    }

    #[test]
    fn test_join_body_context() {
        let text = "async fn foo() {\n  await join all {\n    ";
        let position = Position {
            line: 2,
            character: 4,
        };
        let context = analyze_context(text, position);
        match context {
            CompletionContext::JoinBody { strategy } => {
                assert_eq!(strategy, "all");
            }
            _ => panic!("Expected JoinBody context, got {:?}", context),
        }
    }

    #[test]
    fn test_join_body_context_race() {
        let text = "async fn foo() {\n  await join race {\n    branch1,\n    ";
        let position = Position {
            line: 3,
            character: 4,
        };
        let context = analyze_context(text, position);
        match context {
            CompletionContext::JoinBody { strategy } => {
                assert_eq!(strategy, "race");
            }
            _ => panic!("Expected JoinBody context with race, got {:?}", context),
        }
    }

    #[test]
    fn test_join_body_not_after_close() {
        let text = "async fn foo() {\n  await join all {\n    1, 2\n  }\n  ";
        let position = Position {
            line: 4,
            character: 2,
        };
        let context = analyze_context(text, position);
        assert!(
            !matches!(context, CompletionContext::JoinBody { .. }),
            "Should not be JoinBody after closing brace, got {:?}",
            context
        );
    }

    #[test]
    fn test_trait_bound_context_after_colon() {
        let context = analyze_context(
            "fn foo<T: ",
            Position {
                line: 0,
                character: 10,
            },
        );
        assert_eq!(context, CompletionContext::TraitBound);
    }

    #[test]
    fn test_trait_bound_context_typing_trait_name() {
        let context = analyze_context(
            "fn foo<T: Comp",
            Position {
                line: 0,
                character: 14,
            },
        );
        assert_eq!(context, CompletionContext::TraitBound);
    }

    #[test]
    fn test_trait_bound_context_after_plus() {
        let context = analyze_context(
            "fn foo<T: Comparable + ",
            Position {
                line: 0,
                character: 23,
            },
        );
        assert_eq!(context, CompletionContext::TraitBound);
    }

    #[test]
    fn test_trait_bound_not_in_comparison() {
        // `a < b` should NOT be trait bound
        let context = analyze_context(
            "let x = a < b",
            Position {
                line: 0,
                character: 14,
            },
        );
        assert!(
            !matches!(context, CompletionContext::TraitBound),
            "Comparison should not be TraitBound, got {:?}",
            context
        );
    }

    #[test]
    fn test_trait_bound_function_keyword() {
        let context = analyze_context(
            "function sort<T: ",
            Position {
                line: 0,
                character: 17,
            },
        );
        assert_eq!(context, CompletionContext::TraitBound);
    }

    #[test]
    fn test_trait_bound_trait_keyword() {
        let context = analyze_context(
            "trait Sortable<T: ",
            Position {
                line: 0,
                character: 18,
            },
        );
        assert_eq!(context, CompletionContext::TraitBound);
    }

    #[test]
    fn test_comptime_block_context() {
        let text = "comptime {\n    ";
        let position = Position {
            line: 1,
            character: 4,
        };
        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::ComptimeBlock);
    }

    #[test]
    fn test_comptime_block_context_expression() {
        let text = "let x = comptime {\n    ";
        let position = Position {
            line: 1,
            character: 4,
        };
        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::ComptimeBlock);
    }

    #[test]
    fn test_comptime_block_not_after_close() {
        let text = "comptime {\n    implements(\"Foo\", \"Display\")\n}\nlet x = ";
        let position = Position {
            line: 3,
            character: 8,
        };
        let context = analyze_context(text, position);
        assert!(
            !matches!(context, CompletionContext::ComptimeBlock),
            "Should not be ComptimeBlock after closing brace, got {:?}",
            context
        );
    }

    #[test]
    fn test_expr_annotation_after_equals() {
        let text = "let x = @";
        let position = Position {
            line: 0,
            character: 9,
        };
        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::ExprAnnotation);
    }

    #[test]
    fn test_expr_annotation_after_comma() {
        // After comma in expression context, `@` should trigger ExprAnnotation
        let text = "let x = [a, @";
        let position = Position {
            line: 0,
            character: 13,
        };
        let context = analyze_context(text, position);
        assert_eq!(context, CompletionContext::ExprAnnotation);
    }

    #[test]
    fn test_item_annotation_not_expr_annotation() {
        // `@` at start of line is item-level, not expression-level
        let text = "@";
        let position = Position {
            line: 0,
            character: 1,
        };
        let context = analyze_context(text, position);
        // Should be Annotation (item-level), not ExprAnnotation
        assert!(
            !matches!(context, CompletionContext::ExprAnnotation),
            "Item-level @ should not be ExprAnnotation, got {:?}",
            context
        );
    }
}
