//! Tests for parse error types and formatting

use super::*;

#[test]
fn test_format_unexpected_token() {
    let kind = ParseErrorKind::UnexpectedToken {
        found: TokenInfo::new(")").with_kind(TokenKind::Punctuation),
        expected: vec![ExpectedToken::rule("ident")],
    };
    let msg = format_error_message(&kind);
    assert!(msg.contains("expected"));
    assert!(msg.contains("identifier"));
    assert!(msg.contains("`)`"));
}

#[test]
fn test_format_unexpected_eof() {
    let kind = ParseErrorKind::UnexpectedEof {
        expected: vec![ExpectedToken::literal("}"), ExpectedToken::literal(";")],
    };
    let msg = format_error_message(&kind);
    assert!(msg.contains("unexpected end of input"));
    assert!(msg.contains("`}`"));
    assert!(msg.contains("`;`"));
}

#[test]
fn test_format_unterminated_string() {
    let kind = ParseErrorKind::UnterminatedString {
        start_location: SourceLocation::new(1, 1),
        delimiter: StringDelimiter::DoubleQuote,
    };
    let msg = format_error_message(&kind);
    assert!(msg.contains("unterminated"));
    assert!(msg.contains("double-quoted string"));
}

#[test]
fn test_suggestion_builder() {
    let suggestion = Suggestion::likely("did you mean `foo`?").with_edit(TextEdit::replace(
        (1, 5),
        (1, 8),
        "foo",
    ));

    assert_eq!(suggestion.confidence, SuggestionConfidence::Likely);
    assert!(suggestion.edit.is_some());
}

#[test]
fn test_source_context_from_source() {
    let source = "let x = 1\nlet y = 2\nlet z = foo\nlet w = 4";
    let loc = SourceLocation::new(3, 9).with_length(3);
    let ctx = SourceContext::from_source(source, &loc, None);

    assert!(!ctx.lines.is_empty());
    // Error line should have a highlight
    let error_line = &ctx.lines[ctx.error_line_index];
    assert!(!error_line.highlights.is_empty());
}
