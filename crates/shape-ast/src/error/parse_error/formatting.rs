//! Error message formatting functions

use super::{
    ExpectedToken, IdentifierContext, MissingComponentKind, NumberError, ParseErrorKind,
    StringDelimiter, TokenCategory, TokenInfo, TokenKind,
};

/// Format the main error message based on error kind (plain text, no colors)
pub fn format_error_message(kind: &ParseErrorKind) -> String {
    match kind {
        ParseErrorKind::UnexpectedToken { found, expected } => {
            let found_str = format_found_token(found);
            let expected_str = format_expected_tokens(expected);
            format!("expected {}, found {}", expected_str, found_str)
        }

        ParseErrorKind::UnexpectedEof { expected } => {
            let expected_str = format_expected_tokens(expected);
            format!("unexpected end of input, expected {}", expected_str)
        }

        ParseErrorKind::UnterminatedString { delimiter, .. } => {
            let delim_name = match delimiter {
                StringDelimiter::DoubleQuote => "double-quoted string",
                StringDelimiter::SingleQuote => "single-quoted string",
                StringDelimiter::Backtick => "template literal",
            };
            format!("unterminated {}", delim_name)
        }

        ParseErrorKind::UnterminatedComment { .. } => "unterminated block comment".to_string(),

        ParseErrorKind::UnbalancedDelimiter { opener, found, .. } => match found {
            Some(c) => format!(
                "mismatched closing delimiter: expected `{}`, found `{}`",
                matching_close(*opener),
                c
            ),
            None => format!("unclosed delimiter `{}`", opener),
        },

        ParseErrorKind::InvalidNumber { text, reason } => {
            let reason_str = match reason {
                NumberError::MultipleDecimalPoints => "multiple decimal points",
                NumberError::InvalidExponent => "invalid exponent",
                NumberError::TrailingDecimalPoint => "trailing decimal point",
                NumberError::LeadingZeros => "leading zeros not allowed",
                NumberError::InvalidDigit(c) => {
                    return format!("invalid digit `{}` in number `{}`", c, text);
                }
                NumberError::TooLarge => "number too large",
                NumberError::Empty => "empty number",
            };
            format!("invalid number literal `{}`: {}", text, reason_str)
        }

        ParseErrorKind::ReservedKeyword { keyword, context } => {
            let context_str = match context {
                IdentifierContext::VariableName => "variable name",
                IdentifierContext::FunctionName => "function name",
                IdentifierContext::ParameterName => "parameter name",
                IdentifierContext::PatternName => "pattern name",
                IdentifierContext::TypeName => "type name",
                IdentifierContext::PropertyName => "property name",
            };
            format!(
                "`{}` is a reserved keyword and cannot be used as a {}",
                keyword, context_str
            )
        }

        ParseErrorKind::InvalidEscape { sequence, .. } => {
            format!("unknown escape sequence `{}`", sequence)
        }

        ParseErrorKind::InvalidCharacter { char, codepoint } => {
            if char.is_control() {
                format!("invalid character U+{:04X}", codepoint)
            } else {
                format!("invalid character `{}`", char)
            }
        }

        ParseErrorKind::MissingComponent { component, after } => {
            let comp_str = match component {
                MissingComponentKind::Semicolon => "`;`",
                MissingComponentKind::ClosingBrace => "`}`",
                MissingComponentKind::ClosingBracket => "`]`",
                MissingComponentKind::ClosingParen => "`)`",
                MissingComponentKind::FunctionBody => "function body",
                MissingComponentKind::Expression => "expression",
                MissingComponentKind::TypeAnnotation => "type annotation",
                MissingComponentKind::Identifier => "identifier",
                MissingComponentKind::Arrow => "`->`",
                MissingComponentKind::Colon => "`:`",
            };
            match after {
                Some(ctx) => format!("expected {} after {}", comp_str, ctx),
                None => format!("expected {}", comp_str),
            }
        }

        ParseErrorKind::Custom { message } => message.clone(),
    }
}

fn format_found_token(found: &TokenInfo) -> String {
    match &found.kind {
        Some(TokenKind::Keyword(kw)) => format!("keyword `{}`", kw),
        Some(TokenKind::EndOfInput) => "end of input".to_string(),
        Some(TokenKind::Identifier) => format!("identifier `{}`", found.text),
        Some(TokenKind::Number) => format!("number `{}`", found.text),
        Some(TokenKind::String) => format!("string `{}`", found.text),
        _ if found.text.is_empty() => "nothing".to_string(),
        _ => format!("`{}`", found.text),
    }
}

fn format_expected_tokens(expected: &[ExpectedToken]) -> String {
    if expected.is_empty() {
        return "something else".to_string();
    }

    let items: Vec<String> = expected
        .iter()
        .filter_map(|e| match e {
            ExpectedToken::Literal(s) => Some(format!("`{}`", s)),
            ExpectedToken::Rule(r) => {
                let name = rule_to_friendly_name(r);
                if name.is_empty() { None } else { Some(name) }
            }
            ExpectedToken::Category(c) => Some(category_to_string(*c)),
        })
        .collect();

    if items.is_empty() {
        return "valid syntax".to_string();
    }

    match items.len() {
        1 => items[0].clone(),
        2 => format!("{} or {}", items[0], items[1]),
        _ => {
            let last = items.last().unwrap();
            let rest = &items[..items.len() - 1];
            format!("{}, or {}", rest.join(", "), last)
        }
    }
}

fn category_to_string(c: TokenCategory) -> String {
    match c {
        TokenCategory::Expression => "an expression".to_string(),
        TokenCategory::Statement => "a statement".to_string(),
        TokenCategory::Type => "a type".to_string(),
        TokenCategory::Pattern => "a pattern".to_string(),
        TokenCategory::Identifier => "an identifier".to_string(),
        TokenCategory::Literal => "a literal".to_string(),
        TokenCategory::Operator => "an operator".to_string(),
        TokenCategory::Delimiter => "a delimiter".to_string(),
    }
}

/// Convert pest Rule names to user-friendly descriptions
pub fn rule_to_friendly_name(rule: &str) -> String {
    match rule {
        "expression" | "primary_expr" | "postfix_expr" => "an expression".to_string(),
        "statement" => "a statement".to_string(),
        "ident" | "identifier" => "an identifier".to_string(),
        "number" | "integer" => "a number".to_string(),
        "string" => "a string".to_string(),
        "function_def" => "a function definition".to_string(),
        "variable_decl" => "a variable declaration".to_string(),
        "type_annotation" => "a type annotation".to_string(),
        "function_params" => "function parameters".to_string(),
        "function_body" => "a function body `{ ... }`".to_string(),
        "if_stmt" | "if_expr" => "an if statement".to_string(),
        "for_loop" | "for_expr" => "a for loop".to_string(),
        "while_loop" | "while_expr" => "a while loop".to_string(),
        "return_stmt" => "a return statement".to_string(),
        "query" => "a query".to_string(),
        "find_query" => "a find query".to_string(),
        "scan_query" => "a scan query".to_string(),
        "array_literal" => "an array".to_string(),
        "object_literal" => "an object".to_string(),
        "import_stmt" => "an import statement".to_string(),
        "pub_item" => "a pub item".to_string(),
        "match_expr" => "a match expression".to_string(),
        "match_arm" => "a match arm".to_string(),
        "block_expr" => "a block `{ ... }`".to_string(),
        "join_kind" => "`all`, `race`, `any`, or `settle`".to_string(),
        "comptime_annotation_handler_phase" => "`pre` or `post`".to_string(),
        "annotation_handler_kind" => "a handler kind (`on_define`, `before`, `after`, `metadata`, `comptime pre`, `comptime post`)".to_string(),
        "return_type" => "a return type `-> Type`".to_string(),
        "stream_def" => "a stream definition".to_string(),
        "enum_def" => "an enum definition".to_string(),
        "struct_type_def" => "a struct definition".to_string(),
        "trait_def" => "a trait definition".to_string(),
        "impl_block" => "an impl block".to_string(),
        "EOI" => String::new(), // Don't show "expected end of input"
        "WHITESPACE" | "COMMENT" => String::new(),
        _ => String::new(), // Hide internal rules
    }
}

/// Get the matching closing delimiter
pub fn matching_close(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => open,
    }
}
