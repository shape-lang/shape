//! Pest error to structured error conversion
//!
//! Converts pest's `Error<Rule>` into `StructuredParseError` for rich rendering.

use pest::error::{Error as PestError, ErrorVariant, LineColLocation};

use super::{
    ErrorCode, ExpectedToken, ParseErrorKind, SourceLocation, StructuredParseError, Suggestion,
    TextEdit, TokenCategory, TokenInfo, TokenKind, parse_error::SourceContext,
};
use crate::parser::Rule;

/// Convert a pest error into a structured parse error
pub fn convert_pest_error(pest_error: &PestError<Rule>, source: &str) -> StructuredParseError {
    // Extract location
    let location = extract_location(pest_error);

    // Extract span end for range errors
    let span_end = extract_span_end(pest_error);

    // Convert the error variant to our structured kind
    let kind = convert_variant(&pest_error.variant, source, &location);

    // Build source context
    let source_context = SourceContext::from_source(source, &location, span_end);

    // Generate suggestions based on error kind
    let suggestions = generate_suggestions(&kind, source, &location);

    // Determine error code
    let code = determine_error_code(&kind);

    StructuredParseError::new(kind, location)
        .with_source_context(source_context)
        .with_suggestions(suggestions)
        .with_code(code)
}

fn extract_location(error: &PestError<Rule>) -> SourceLocation {
    match &error.line_col {
        LineColLocation::Pos((line, col)) => SourceLocation::new(*line, *col),
        LineColLocation::Span((start_line, start_col), _) => {
            SourceLocation::new(*start_line, *start_col)
        }
    }
}

fn extract_span_end(error: &PestError<Rule>) -> Option<(usize, usize)> {
    match &error.line_col {
        LineColLocation::Span(_, (end_line, end_col)) => Some((*end_line, *end_col)),
        LineColLocation::Pos(_) => None,
    }
}

fn convert_variant(
    variant: &ErrorVariant<Rule>,
    source: &str,
    location: &SourceLocation,
) -> ParseErrorKind {
    match variant {
        ErrorVariant::ParsingError {
            positives,
            negatives: _,
        } => {
            // Convert pest's positives (expected rules) to our expected tokens
            let expected: Vec<ExpectedToken> = positives
                .iter()
                .filter_map(rule_to_expected_token)
                .collect();

            // Get the actual token at this position
            let found = extract_found_token(source, location);

            // Check if we're at end of input
            if matches!(found.kind, Some(TokenKind::EndOfInput)) {
                ParseErrorKind::UnexpectedEof { expected }
            } else {
                ParseErrorKind::UnexpectedToken { found, expected }
            }
        }
        ErrorVariant::CustomError { message } => {
            // Try to parse semantic meaning from custom errors
            parse_custom_error(message, location)
        }
    }
}

/// Convert a pest Rule to an ExpectedToken
fn rule_to_expected_token(rule: &Rule) -> Option<ExpectedToken> {
    // Map rules to user-friendly expectations
    match rule {
        Rule::ident => Some(ExpectedToken::Category(TokenCategory::Identifier)),
        Rule::expression | Rule::primary_expr | Rule::postfix_expr => {
            Some(ExpectedToken::Category(TokenCategory::Expression))
        }
        Rule::statement => Some(ExpectedToken::Category(TokenCategory::Statement)),
        Rule::number | Rule::integer => Some(ExpectedToken::Category(TokenCategory::Literal)),
        Rule::string => Some(ExpectedToken::Rule("string".to_string())),
        Rule::function_def => Some(ExpectedToken::Rule("function_def".to_string())),
        Rule::variable_decl => Some(ExpectedToken::Rule("variable_decl".to_string())),
        Rule::type_annotation => Some(ExpectedToken::Rule("type_annotation".to_string())),
        Rule::if_stmt | Rule::if_expr => Some(ExpectedToken::Rule("if_stmt".to_string())),
        Rule::for_loop | Rule::for_expr => Some(ExpectedToken::Rule("for_loop".to_string())),
        Rule::while_loop | Rule::while_expr => Some(ExpectedToken::Rule("while_loop".to_string())),
        Rule::return_stmt => Some(ExpectedToken::Rule("return_stmt".to_string())),
        Rule::query => Some(ExpectedToken::Rule("query".to_string())),
        Rule::import_stmt => Some(ExpectedToken::Rule("import_stmt".to_string())),
        Rule::pub_item => Some(ExpectedToken::Rule("pub_item".to_string())),
        Rule::array_literal => Some(ExpectedToken::Rule("array_literal".to_string())),
        Rule::object_literal => Some(ExpectedToken::Rule("object_literal".to_string())),
        Rule::match_expr => Some(ExpectedToken::Rule("match_expr".to_string())),
        Rule::match_arm => Some(ExpectedToken::Rule("match_arm".to_string())),
        Rule::block_expr => Some(ExpectedToken::Rule("block_expr".to_string())),
        Rule::function_body => Some(ExpectedToken::Rule("function_body".to_string())),
        Rule::function_params => Some(ExpectedToken::Rule("function_params".to_string())),
        Rule::pattern => Some(ExpectedToken::Category(TokenCategory::Pattern)),
        Rule::primary_type | Rule::basic_type | Rule::generic_type => {
            Some(ExpectedToken::Category(TokenCategory::Type))
        }
        Rule::join_kind => Some(ExpectedToken::Rule("join_kind".to_string())),
        Rule::comptime_annotation_handler_phase => Some(ExpectedToken::Rule(
            "comptime_annotation_handler_phase".to_string(),
        )),
        Rule::annotation_handler_kind => {
            Some(ExpectedToken::Rule("annotation_handler_kind".to_string()))
        }
        Rule::stream_def => Some(ExpectedToken::Rule("stream_def".to_string())),
        Rule::enum_def => Some(ExpectedToken::Rule("enum_def".to_string())),
        Rule::struct_type_def => Some(ExpectedToken::Rule("struct_type_def".to_string())),
        Rule::trait_def => Some(ExpectedToken::Rule("trait_def".to_string())),
        Rule::impl_block => Some(ExpectedToken::Rule("impl_block".to_string())),
        Rule::return_type => Some(ExpectedToken::Rule("return_type".to_string())),

        // Internal rules we don't want to show
        Rule::EOI | Rule::WHITESPACE | Rule::COMMENT => None,
        Rule::program | Rule::item => None,

        // For unknown rules, return None to filter them out
        _ => None,
    }
}

fn extract_found_token(source: &str, location: &SourceLocation) -> TokenInfo {
    let lines: Vec<&str> = source.lines().collect();
    if location.line == 0 || location.line > lines.len() {
        return TokenInfo::end_of_input();
    }

    let line = lines[location.line - 1];
    if location.column == 0 {
        return TokenInfo::new("").with_kind(TokenKind::Unknown);
    }

    // Convert char-based column to byte offset (Pest columns count characters, not bytes)
    let col0 = location.column - 1;
    let byte_offset = line
        .char_indices()
        .nth(col0)
        .map(|(i, _)| i);

    let Some(byte_offset) = byte_offset else {
        // Column is past the end of the line
        if location.line >= lines.len() {
            return TokenInfo::end_of_input();
        }
        return TokenInfo::new("").with_kind(TokenKind::Unknown);
    };

    // Extract a token starting at the position
    let rest = &line[byte_offset..];
    let token_text = extract_token_text(rest);
    let kind = classify_token(&token_text);

    TokenInfo::new(token_text).with_kind(kind)
}

fn extract_token_text(s: &str) -> String {
    let mut chars = s.chars().peekable();
    let first = match chars.next() {
        Some(c) => c,
        None => return String::new(),
    };

    // Identifier or keyword
    if first.is_alphabetic() || first == '_' {
        let mut text = String::from(first);
        while let Some(&c) = chars.peek() {
            if c.is_alphanumeric() || c == '_' {
                text.push(chars.next().unwrap());
            } else {
                break;
            }
        }
        return text;
    }

    // Number
    if first.is_numeric() {
        let mut text = String::from(first);
        while let Some(&c) = chars.peek() {
            if c.is_numeric() || c == '.' || c == 'e' || c == 'E' {
                text.push(chars.next().unwrap());
            } else {
                break;
            }
        }
        return text;
    }

    // Single character token
    first.to_string()
}

fn classify_token(text: &str) -> TokenKind {
    // Check for keywords
    const KEYWORDS: &[&str] = &[
        "let", "var", "const", "function", "return", "if", "else", "for", "while", "break",
        "continue", "pattern", "query", "true", "false", "null", "import", "module", "extend",
        "method", "stream", "find", "scan", "analyze", "on", "and", "or",
    ];

    if KEYWORDS.contains(&text) {
        return TokenKind::Keyword(text.to_string());
    }

    if text
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
    {
        return TokenKind::Identifier;
    }

    if text.chars().next().is_some_and(|c| c.is_numeric()) {
        return TokenKind::Number;
    }

    if text.starts_with('"') || text.starts_with('\'') || text.starts_with('`') {
        return TokenKind::String;
    }

    TokenKind::Punctuation
}

fn parse_custom_error(message: &str, _location: &SourceLocation) -> ParseErrorKind {
    // Try to recognize common patterns in custom error messages
    let msg_lower = message.to_lowercase();

    if msg_lower.contains("unterminated") && msg_lower.contains("string") {
        return ParseErrorKind::UnterminatedString {
            start_location: SourceLocation::new(0, 0), // Would need more context
            delimiter: super::StringDelimiter::DoubleQuote,
        };
    }

    if msg_lower.contains("unterminated") && msg_lower.contains("comment") {
        return ParseErrorKind::UnterminatedComment {
            start_location: SourceLocation::new(0, 0),
        };
    }

    ParseErrorKind::Custom {
        message: message.to_string(),
    }
}

fn generate_suggestions(
    kind: &ParseErrorKind,
    source: &str,
    location: &SourceLocation,
) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();

    match kind {
        ParseErrorKind::UnexpectedToken { found, expected } => {
            // Check for common typos using Levenshtein distance
            if let Some(TokenKind::Identifier) = &found.kind {
                // Check if the identifier might be a typo of a keyword
                for keyword in &["function", "return", "pattern", "import"] {
                    if levenshtein_distance(&found.text, keyword) <= 2 {
                        suggestions.push(
                            Suggestion::likely(format!("did you mean `{}`?", keyword)).with_edit(
                                TextEdit::replace(
                                    (location.line, location.column),
                                    (location.line, location.column + found.text.len()),
                                    keyword.to_string(),
                                ),
                            ),
                        );
                        break;
                    }
                }
            }

            // Suggest missing semicolon
            if expected
                .iter()
                .any(|e| matches!(e, ExpectedToken::Literal(s) if s == ";"))
            {
                suggestions.push(
                    Suggestion::likely("try adding a semicolon here").with_edit(TextEdit::insert(
                        location.line,
                        location.column,
                        ";",
                    )),
                );
            }

            // Suggest missing closing delimiter
            for delim in &[")", "]", "}"] {
                if expected
                    .iter()
                    .any(|e| matches!(e, ExpectedToken::Literal(s) if s == *delim))
                {
                    suggestions.push(Suggestion::likely(format!(
                        "you may be missing a `{}`",
                        delim
                    )));
                    break;
                }
            }

            // Suggest missing `=>` in match arms
            if expected
                .iter()
                .any(|e| matches!(e, ExpectedToken::Rule(s) if s == "match_arm"))
            {
                suggestions.push(Suggestion::likely(
                    "match arms require `=>` after the pattern, e.g. `pattern => expression`",
                ));
            }

            // Suggest `pre` or `post` for comptime handler phase (BUG-15)
            if expected.iter().any(
                |e| matches!(e, ExpectedToken::Rule(s) if s == "comptime_annotation_handler_phase"),
            ) {
                suggestions.push(Suggestion::likely(
                    "use `comptime pre(...)` or `comptime post(...)` to specify the handler phase",
                ));
            }

            // Suggest valid join strategies
            if expected
                .iter()
                .any(|e| matches!(e, ExpectedToken::Rule(s) if s == "join_kind"))
            {
                suggestions.push(Suggestion::likely(
                    "expected a join strategy: `all`, `race`, `any`, or `settle`",
                ));
            }

            if let Some(suggestion) =
                struct_literal_named_field_suggestion(source, location, found, expected)
            {
                suggestions.push(suggestion);
            }
        }

        ParseErrorKind::UnexpectedEof { expected } => {
            if !expected.is_empty() {
                let needs_brace = expected
                    .iter()
                    .any(|e| matches!(e, ExpectedToken::Literal(s) if s == "}"));
                let needs_body = expected.iter().any(|e| {
                    matches!(e, ExpectedToken::Rule(s) if s == "function_body" || s == "block_expr")
                });

                if needs_brace || needs_body {
                    suggestions.push(Suggestion::likely(
                        "you may have an unclosed block - check for missing `}`",
                    ));
                } else {
                    suggestions.push(Suggestion::new(
                        "the file ended unexpectedly - check for unclosed delimiters",
                    ));
                }
            }

            // If no expected tokens, check source for unclosed delimiters
            if expected.is_empty() {
                let open_braces = source.chars().filter(|c| *c == '{').count();
                let close_braces = source.chars().filter(|c| *c == '}').count();
                if open_braces > close_braces {
                    suggestions.push(Suggestion::likely(
                        "you may have an unclosed block - check for missing `}`",
                    ));
                }
            }

            // Suggest `pre` or `post` for comptime handler phase at EOF (BUG-15)
            if expected.iter().any(
                |e| matches!(e, ExpectedToken::Rule(s) if s == "comptime_annotation_handler_phase"),
            ) {
                suggestions.push(Suggestion::likely(
                    "use `comptime pre(...)` or `comptime post(...)` to specify the handler phase",
                ));
            }
        }

        ParseErrorKind::UnterminatedString { delimiter, .. } => {
            let close_char = match delimiter {
                super::StringDelimiter::DoubleQuote => '"',
                super::StringDelimiter::SingleQuote => '\'',
                super::StringDelimiter::Backtick => '`',
            };
            suggestions.push(Suggestion::certain(format!(
                "add closing `{}` to terminate the string",
                close_char
            )));
        }

        ParseErrorKind::UnbalancedDelimiter { opener, .. } => {
            let closer = super::parse_error::matching_close(*opener);
            suggestions.push(Suggestion::certain(format!(
                "add `{}` to close the `{}`",
                closer, opener
            )));
        }

        ParseErrorKind::ReservedKeyword { keyword, .. } => {
            suggestions.push(Suggestion::new(format!(
                "try using a different name, such as `{}_value` or `my_{}`",
                keyword, keyword
            )));
        }

        ParseErrorKind::InvalidEscape {
            sequence: _,
            valid_escapes,
        } => {
            if !valid_escapes.is_empty() {
                suggestions.push(Suggestion::certain(format!(
                    "valid escape sequences are: {}",
                    valid_escapes.join(", ")
                )));
            }
        }

        _ => {}
    }

    suggestions
}

fn struct_literal_named_field_suggestion(
    source: &str,
    location: &SourceLocation,
    found: &TokenInfo,
    _expected: &[ExpectedToken],
) -> Option<Suggestion> {
    if !matches!(found.kind, Some(TokenKind::String)) {
        return None;
    }

    let offset = line_col_to_offset(source, location.line, location.column)?;
    let prefix = &source[..offset.min(source.len())];
    let trimmed_len = prefix.trim_end_matches(char::is_whitespace).len();
    if trimmed_len == 0 {
        return None;
    }

    let bytes = prefix.as_bytes();
    let prev = bytes[trimmed_len - 1] as char;
    if prev != '{' && prev != ',' {
        return None;
    }

    if prev == '{' {
        // Try to recover `TypeName` from `TypeName { "..." }` for a concrete hint.
        let mut end = trimmed_len - 1;
        while end > 0 && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 {
            let c = bytes[start - 1] as char;
            if c.is_ascii_alphanumeric() || c == '_' {
                start -= 1;
            } else {
                break;
            }
        }
        if start < end {
            let ty_name = &prefix[start..end];
            if ty_name
                .chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false)
            {
                return Some(Suggestion::likely(format!(
                    "struct literals require named fields, e.g. `{} {{ name: {} }}`",
                    ty_name, found.text
                )));
            }
        }
    }

    Some(Suggestion::likely(
        "struct literals require named fields: `TypeName { field: value }`",
    ))
}

fn line_col_to_offset(source: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 || column == 0 {
        return None;
    }

    let mut byte_offset = 0usize;
    let mut lines = source.split('\n');
    let line_text = lines.nth(line - 1)?;
    for prev in source.split('\n').take(line - 1) {
        byte_offset = byte_offset.saturating_add(prev.len() + 1);
    }

    let col0 = column.saturating_sub(1);
    let col_byte = if col0 == 0 {
        0
    } else {
        line_text
            .char_indices()
            .nth(col0)
            .map(|(i, _)| i)
            .unwrap_or(line_text.len())
    };

    Some(byte_offset.saturating_add(col_byte))
}

fn determine_error_code(kind: &ParseErrorKind) -> ErrorCode {
    match kind {
        ParseErrorKind::UnexpectedToken { .. } => ErrorCode::E0001,
        ParseErrorKind::UnexpectedEof { .. } => ErrorCode::E0001,
        ParseErrorKind::UnterminatedString { .. } => ErrorCode::E0002,
        ParseErrorKind::UnterminatedComment { .. } => ErrorCode::E0002,
        ParseErrorKind::InvalidNumber { .. } => ErrorCode::E0003,
        ParseErrorKind::MissingComponent {
            component: super::MissingComponentKind::Semicolon,
            ..
        } => ErrorCode::E0004,
        ParseErrorKind::UnbalancedDelimiter { .. } => ErrorCode::E0005,
        _ => ErrorCode::E0001, // Default to unexpected token
    }
}

/// Simple Levenshtein distance implementation
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row = vec![0; b_len + 1];

    for (i, a_char) in a_chars.iter().enumerate() {
        curr_row[0] = i + 1;
        for (j, b_char) in b_chars.iter().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1)
                .min(curr_row[j] + 1)
                .min(prev_row[j] + cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Rule, ShapeParser};
    use pest::Parser;

    #[test]
    fn suggests_named_fields_for_positional_struct_literal_value() {
        let source = r#"User {"John"}"#;
        let pest_err =
            ShapeParser::parse(Rule::struct_literal, source).expect_err("expected parse error");
        let structured = convert_pest_error(&pest_err, source);
        let has_hint = structured
            .suggestions
            .iter()
            .any(|s| s.message.contains("struct literals require named fields"));
        assert!(
            has_hint,
            "expected named-field struct literal hint, got: {:?}",
            structured
                .suggestions
                .iter()
                .map(|s| s.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_classify_keyword() {
        assert!(matches!(classify_token("function"), TokenKind::Keyword(_)));
        assert!(matches!(classify_token("return"), TokenKind::Keyword(_)));
    }

    #[test]
    fn test_classify_identifier() {
        assert!(matches!(classify_token("foo"), TokenKind::Identifier));
        assert!(matches!(classify_token("myVar"), TokenKind::Identifier));
        assert!(matches!(classify_token("_private"), TokenKind::Identifier));
    }

    #[test]
    fn test_classify_number() {
        assert!(matches!(classify_token("42"), TokenKind::Number));
        assert!(matches!(classify_token("3.14"), TokenKind::Number));
    }

    #[test]
    fn test_extract_token_text() {
        assert_eq!(extract_token_text("foo + bar"), "foo");
        assert_eq!(extract_token_text("123.45"), "123.45");
        assert_eq!(extract_token_text(")"), ")");
        assert_eq!(extract_token_text(""), "");
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("function", "function"), 0);
        assert_eq!(levenshtein_distance("fucntion", "function"), 2);
        assert_eq!(levenshtein_distance("funciton", "function"), 2);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
    }

    // BUG-9: Generic parse error quality tests

    #[test]
    fn match_arm_missing_fat_arrow_produces_helpful_error() {
        let source = "match x { 1 2 }";
        let pest_err =
            ShapeParser::parse(Rule::expression, source).expect_err("expected parse error");
        let structured = convert_pest_error(&pest_err, source);
        let msg = format!("{}", structured);
        assert!(
            !msg.contains("expected something else"),
            "error should be specific, got: {}",
            msg
        );
    }

    #[test]
    fn missing_function_body_produces_helpful_error() {
        let source = "function foo()";
        let pest_err =
            ShapeParser::parse(Rule::function_def, source).expect_err("expected parse error");
        let structured = convert_pest_error(&pest_err, source);
        let msg = format!("{}", structured);
        assert!(
            !msg.contains("expected something else"),
            "error should mention function body, got: {}",
            msg
        );
    }

    #[test]
    fn missing_closing_brace_produces_helpful_suggestion() {
        let source = "{ let x = 1;";
        let pest_err =
            ShapeParser::parse(Rule::block_expr, source).expect_err("expected parse error");
        let structured = convert_pest_error(&pest_err, source);
        let msg = format!("{}", structured);
        let has_brace_hint = msg.contains("`}`")
            || msg.contains("unclosed")
            || structured
                .suggestions
                .iter()
                .any(|s| s.message.contains("`}`") || s.message.contains("unclosed"));
        assert!(
            has_brace_hint,
            "expected closing brace hint, got message: '{}', suggestions: {:?}",
            msg,
            structured
                .suggestions
                .iter()
                .map(|s| s.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn rule_to_expected_token_covers_match_arm() {
        let token = rule_to_expected_token(&Rule::match_arm);
        assert!(token.is_some(), "match_arm should produce an ExpectedToken");
    }

    #[test]
    fn rule_to_expected_token_covers_match_expr() {
        let token = rule_to_expected_token(&Rule::match_expr);
        assert!(
            token.is_some(),
            "match_expr should produce an ExpectedToken"
        );
    }

    #[test]
    fn rule_to_expected_token_covers_block_expr() {
        let token = rule_to_expected_token(&Rule::block_expr);
        assert!(
            token.is_some(),
            "block_expr should produce an ExpectedToken"
        );
    }

    #[test]
    fn rule_to_expected_token_covers_function_body() {
        let token = rule_to_expected_token(&Rule::function_body);
        assert!(
            token.is_some(),
            "function_body should produce an ExpectedToken"
        );
    }

    #[test]
    fn rule_to_expected_token_covers_function_params() {
        let token = rule_to_expected_token(&Rule::function_params);
        assert!(
            token.is_some(),
            "function_params should produce an ExpectedToken"
        );
    }

    #[test]
    fn rule_to_expected_token_covers_pattern() {
        let token = rule_to_expected_token(&Rule::pattern);
        assert!(token.is_some(), "pattern should produce an ExpectedToken");
    }

    // BUG-15: Comptime error quality tests

    #[test]
    fn rule_to_expected_token_covers_comptime_handler_phase() {
        let token = rule_to_expected_token(&Rule::comptime_annotation_handler_phase);
        assert!(
            token.is_some(),
            "comptime_annotation_handler_phase should produce an ExpectedToken"
        );
    }

    #[test]
    fn comptime_invalid_phase_produces_suggestion() {
        let source = "comptime target";
        let pest_err = ShapeParser::parse(Rule::annotation_handler_kind, source)
            .expect_err("expected parse error");
        let structured = convert_pest_error(&pest_err, source);
        let has_comptime_hint = structured
            .suggestions
            .iter()
            .any(|s| s.message.contains("pre") && s.message.contains("post"));
        assert!(
            has_comptime_hint,
            "expected comptime pre/post suggestion, got suggestions: {:?}",
            structured
                .suggestions
                .iter()
                .map(|s| s.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_found_token_with_multibyte_utf8() {
        // em-dash is 3 bytes in UTF-8 — this used to panic with
        // "byte index N is not a char boundary"
        let source = "// comment — rest\nlet x = 1";
        // Exercise extract_found_token with a location pointing past the em-dash
        let loc = SourceLocation::new(1, 14); // char position past "— "
        let token = extract_found_token(source, &loc);
        // Should not panic, and should extract "rest" or something reasonable
        assert!(!token.text.is_empty() || token.kind == Some(TokenKind::Unknown));
    }

    #[test]
    fn test_extract_found_token_multibyte_at_error_position() {
        // Trigger a parse error where the error position is on a multi-byte char
        let source = "let — = 1";
        let pest_err =
            ShapeParser::parse(Rule::program, source).expect_err("expected parse error");
        // Should not panic
        let structured = convert_pest_error(&pest_err, source);
        // kind should be set (not a default/empty error)
        assert!(!matches!(structured.kind, ParseErrorKind::MissingComponent { .. }));
    }
}
