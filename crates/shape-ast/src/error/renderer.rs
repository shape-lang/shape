//! Error rendering for different output targets
//!
//! Provides trait-based rendering of structured errors for CLI and other targets.

use super::{
    ErrorCode, ExpectedToken, ParseErrorKind, StructuredParseError, TokenCategory, TokenKind,
    parse_error::{HighlightStyle, Suggestion},
};

/// Trait for rendering structured parse errors to different output formats
pub trait ErrorRenderer {
    type Output;

    /// Render a single error
    fn render(&self, error: &StructuredParseError) -> Self::Output;

    /// Render multiple errors
    fn render_all(&self, errors: &[StructuredParseError]) -> Self::Output;
}

/// Configuration for CLI error rendering
#[derive(Debug, Clone)]
pub struct CliRendererConfig {
    /// Use ANSI colors in output
    pub use_colors: bool,
    /// Number of context lines to show before/after error
    pub context_lines: usize,
    /// Show error codes (e.g., E0001)
    pub show_error_codes: bool,
    /// Show suggestions
    pub show_suggestions: bool,
    /// Show related information
    pub show_related: bool,
    /// Terminal width for wrapping (0 = no wrap)
    pub terminal_width: usize,
}

impl Default for CliRendererConfig {
    fn default() -> Self {
        Self {
            use_colors: true,
            context_lines: 2,
            show_error_codes: true,
            show_suggestions: true,
            show_related: true,
            terminal_width: 80,
        }
    }
}

impl CliRendererConfig {
    /// Create a config without colors (for testing or non-terminal output)
    pub fn plain() -> Self {
        Self {
            use_colors: false,
            ..Default::default()
        }
    }
}

/// CLI error renderer with ANSI color support
pub struct CliErrorRenderer {
    config: CliRendererConfig,
}

impl CliErrorRenderer {
    pub fn new(config: CliRendererConfig) -> Self {
        Self { config }
    }

    pub fn with_colors() -> Self {
        Self::new(CliRendererConfig::default())
    }

    pub fn without_colors() -> Self {
        Self::new(CliRendererConfig::plain())
    }

    // ANSI color codes
    fn bold_red(&self, s: &str) -> String {
        if self.config.use_colors {
            format!("\x1b[1;31m{}\x1b[0m", s)
        } else {
            s.to_string()
        }
    }

    fn yellow(&self, s: &str) -> String {
        if self.config.use_colors {
            format!("\x1b[33m{}\x1b[0m", s)
        } else {
            s.to_string()
        }
    }

    fn blue(&self, s: &str) -> String {
        if self.config.use_colors {
            format!("\x1b[34m{}\x1b[0m", s)
        } else {
            s.to_string()
        }
    }

    fn cyan(&self, s: &str) -> String {
        if self.config.use_colors {
            format!("\x1b[36m{}\x1b[0m", s)
        } else {
            s.to_string()
        }
    }

    fn bold(&self, s: &str) -> String {
        if self.config.use_colors {
            format!("\x1b[1m{}\x1b[0m", s)
        } else {
            s.to_string()
        }
    }

    fn dim(&self, s: &str) -> String {
        if self.config.use_colors {
            format!("\x1b[2m{}\x1b[0m", s)
        } else {
            s.to_string()
        }
    }

    /// Format the error header line
    fn format_header(&self, error: &StructuredParseError) -> String {
        let severity = match error.severity {
            super::ErrorSeverity::Error => self.bold_red("error"),
            super::ErrorSeverity::Warning => self.yellow("warning"),
            super::ErrorSeverity::Info => self.blue("info"),
            super::ErrorSeverity::Hint => self.cyan("hint"),
        };

        let code = if self.config.show_error_codes {
            format!("[{}]", self.format_error_code(error.code))
        } else {
            String::new()
        };

        let message = self.bold(&self.format_error_message(&error.kind));

        format!("{}{}: {}", severity, code, message)
    }

    /// Format error code
    fn format_error_code(&self, code: ErrorCode) -> String {
        code.as_str().to_string()
    }

    /// Format the main error message
    fn format_error_message(&self, kind: &ParseErrorKind) -> String {
        match kind {
            ParseErrorKind::UnexpectedToken { found, expected } => {
                let found_str = self.format_token_info(found);
                let expected_str = self.format_expected_list(expected);
                format!("unexpected {}, expected {}", found_str, expected_str)
            }
            ParseErrorKind::UnexpectedEof { expected } => {
                let expected_str = self.format_expected_list(expected);
                format!("unexpected end of file, expected {}", expected_str)
            }
            ParseErrorKind::UnterminatedString { delimiter, .. } => {
                let delim_char = match delimiter {
                    super::parse_error::StringDelimiter::DoubleQuote => '"',
                    super::parse_error::StringDelimiter::SingleQuote => '\'',
                    super::parse_error::StringDelimiter::Backtick => '`',
                };
                format!(
                    "unterminated string literal, missing closing `{}`",
                    delim_char
                )
            }
            ParseErrorKind::UnterminatedComment { .. } => {
                "unterminated block comment, missing `*/`".to_string()
            }
            ParseErrorKind::UnbalancedDelimiter { opener, found, .. } => {
                let closer = matching_close(*opener);
                match found {
                    Some(c) => {
                        format!("mismatched delimiter: expected `{}`, found `{}`", closer, c)
                    }
                    None => format!("unclosed `{}`, missing `{}`", opener, closer),
                }
            }
            ParseErrorKind::InvalidNumber { text, reason } => {
                let reason_str = match reason {
                    super::parse_error::NumberError::InvalidDigit(c) => {
                        return format!("invalid digit `{}` in number `{}`", c, text);
                    }
                    super::parse_error::NumberError::TooLarge => "number too large",
                    super::parse_error::NumberError::MultipleDecimalPoints => {
                        "multiple decimal points"
                    }
                    super::parse_error::NumberError::InvalidExponent => "invalid exponent",
                    super::parse_error::NumberError::TrailingDecimalPoint => {
                        "trailing decimal point"
                    }
                    super::parse_error::NumberError::LeadingZeros => "leading zeros not allowed",
                    super::parse_error::NumberError::Empty => "empty number",
                };
                format!("invalid number `{}`: {}", text, reason_str)
            }
            ParseErrorKind::InvalidEscape { sequence, .. } => {
                format!("invalid escape sequence `{}`", sequence)
            }
            ParseErrorKind::InvalidCharacter { char, codepoint } => {
                if char.is_control() {
                    format!("invalid character U+{:04X}", codepoint)
                } else {
                    format!("invalid character `{}`", char)
                }
            }
            ParseErrorKind::ReservedKeyword { keyword, .. } => {
                format!("`{}` is a reserved keyword", keyword)
            }
            ParseErrorKind::MissingComponent { component, after } => {
                let comp_str = match component {
                    super::parse_error::MissingComponentKind::Semicolon => "`;`",
                    super::parse_error::MissingComponentKind::Colon => "`:`",
                    super::parse_error::MissingComponentKind::Arrow => "`->`",
                    super::parse_error::MissingComponentKind::ClosingParen => "`)`",
                    super::parse_error::MissingComponentKind::ClosingBrace => "`}`",
                    super::parse_error::MissingComponentKind::ClosingBracket => "`]`",
                    super::parse_error::MissingComponentKind::FunctionBody => "function body",
                    super::parse_error::MissingComponentKind::Expression => "expression",
                    super::parse_error::MissingComponentKind::TypeAnnotation => "type annotation",
                    super::parse_error::MissingComponentKind::Identifier => "identifier",
                };
                match after {
                    Some(a) => format!("missing {} after `{}`", comp_str, a),
                    None => format!("missing {}", comp_str),
                }
            }
            ParseErrorKind::Custom { message } => message.clone(),
        }
    }

    /// Format token info for display
    fn format_token_info(&self, token: &super::parse_error::TokenInfo) -> String {
        match &token.kind {
            Some(TokenKind::EndOfInput) => "end of input".to_string(),
            Some(TokenKind::Keyword(k)) => format!("keyword `{}`", k),
            Some(TokenKind::Identifier) => format!("identifier `{}`", token.text),
            Some(TokenKind::Number) => format!("number `{}`", token.text),
            Some(TokenKind::String) => format!("string `{}`", token.text),
            Some(TokenKind::Punctuation) | Some(TokenKind::Operator) => {
                format!("`{}`", token.text)
            }
            Some(TokenKind::Whitespace) => "whitespace".to_string(),
            Some(TokenKind::Comment) => "comment".to_string(),
            Some(TokenKind::Unknown) | None => {
                if token.text.is_empty() {
                    "unknown token".to_string()
                } else {
                    format!("`{}`", token.text)
                }
            }
        }
    }

    /// Format list of expected tokens
    fn format_expected_list(&self, expected: &[ExpectedToken]) -> String {
        if expected.is_empty() {
            return "something else".to_string();
        }

        let formatted: Vec<String> = expected
            .iter()
            .filter_map(|e| match e {
                ExpectedToken::Literal(s) => Some(format!("`{}`", s)),
                ExpectedToken::Category(cat) => Some(match cat {
                    TokenCategory::Identifier => "identifier".to_string(),
                    TokenCategory::Expression => "expression".to_string(),
                    TokenCategory::Statement => "statement".to_string(),
                    TokenCategory::Literal => "literal".to_string(),
                    TokenCategory::Operator => "operator".to_string(),
                    TokenCategory::Type => "type".to_string(),
                    TokenCategory::Pattern => "pattern".to_string(),
                    TokenCategory::Delimiter => "delimiter".to_string(),
                }),
                ExpectedToken::Rule(r) => {
                    let name = super::parse_error::rule_to_friendly_name(r);
                    if name.is_empty() { None } else { Some(name) }
                }
            })
            .collect();

        if formatted.is_empty() {
            return "valid syntax".to_string();
        }

        if formatted.len() == 1 {
            formatted[0].clone()
        } else if formatted.len() == 2 {
            format!("{} or {}", formatted[0], formatted[1])
        } else {
            let (last, rest) = formatted.split_last().unwrap();
            format!("{}, or {}", rest.join(", "), last)
        }
    }

    /// Format source location line
    fn format_location(&self, error: &StructuredParseError, filename: Option<&str>) -> String {
        let file = filename.unwrap_or("<input>");
        let location = format!("{}:{}:{}", file, error.location.line, error.location.column);
        format!("  {} {}", self.blue("-->"), location)
    }

    /// Format source context with line numbers and highlights
    fn format_source_context(&self, error: &StructuredParseError) -> String {
        let ctx = &error.source_context;
        if ctx.lines.is_empty() {
            return String::new();
        }

        let max_line_num = ctx.lines.iter().map(|l| l.number).max().unwrap_or(1);
        let gutter_width = max_line_num.to_string().len();

        let mut output = Vec::new();

        // Empty gutter line for visual spacing
        output.push(format!("{} {}", " ".repeat(gutter_width), self.blue("|")));

        for source_line in &ctx.lines {
            // Line number and content
            let line_num = format!("{:>width$}", source_line.number, width = gutter_width);
            output.push(format!(
                "{} {} {}",
                self.blue(&line_num),
                self.blue("|"),
                source_line.content
            ));

            // Render highlights for this line
            for highlight in &source_line.highlights {
                let prefix_spaces = " ".repeat(highlight.start.saturating_sub(1));
                let marker_len = (highlight.end - highlight.start).max(1);
                let marker_char = match highlight.style {
                    HighlightStyle::Primary => '^',
                    HighlightStyle::Secondary => '-',
                    HighlightStyle::Suggestion => '~',
                };
                let marker = marker_char.to_string().repeat(marker_len);

                let colored_marker = match highlight.style {
                    HighlightStyle::Primary => self.bold_red(&marker),
                    HighlightStyle::Secondary => self.blue(&marker),
                    HighlightStyle::Suggestion => self.cyan(&marker),
                };

                let label = highlight
                    .label
                    .as_ref()
                    .map(|l| format!(" {}", l))
                    .unwrap_or_default();
                let colored_label = match highlight.style {
                    HighlightStyle::Primary => self.bold_red(&label),
                    HighlightStyle::Secondary => self.blue(&label),
                    HighlightStyle::Suggestion => self.cyan(&label),
                };

                output.push(format!(
                    "{} {} {}{}{}",
                    " ".repeat(gutter_width),
                    self.blue("|"),
                    prefix_spaces,
                    colored_marker,
                    colored_label
                ));
            }
        }

        // Empty gutter line at end
        output.push(format!("{} {}", " ".repeat(gutter_width), self.blue("|")));

        output.join("\n")
    }

    /// Format suggestions
    fn format_suggestions(&self, suggestions: &[Suggestion]) -> String {
        if suggestions.is_empty() || !self.config.show_suggestions {
            return String::new();
        }

        let mut output = Vec::new();
        for suggestion in suggestions {
            let prefix = match suggestion.confidence {
                super::parse_error::SuggestionConfidence::Certain => {
                    self.bold(&self.cyan("= fix: "))
                }
                super::parse_error::SuggestionConfidence::Likely => {
                    self.bold(&self.yellow("= help: "))
                }
                super::parse_error::SuggestionConfidence::Maybe => self.bold(&self.dim("= note: ")),
            };
            output.push(format!("  {}{}", prefix, suggestion.message));
        }

        output.join("\n")
    }

    /// Format related information
    fn format_related(&self, related: &[super::parse_error::RelatedInfo]) -> String {
        if related.is_empty() || !self.config.show_related {
            return String::new();
        }

        let mut output = Vec::new();
        for info in related {
            let location = format!("{}:{}", info.location.line, info.location.column);
            output.push(format!(
                "  {} {}: {}",
                self.blue("note:"),
                self.dim(&location),
                info.message
            ));
        }

        output.join("\n")
    }
}

impl ErrorRenderer for CliErrorRenderer {
    type Output = String;

    fn render(&self, error: &StructuredParseError) -> String {
        self.render_with_filename(error, None)
    }

    fn render_all(&self, errors: &[StructuredParseError]) -> String {
        errors
            .iter()
            .map(|e| self.render(e))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl CliErrorRenderer {
    /// Render with an optional filename for location display
    pub fn render_with_filename(
        &self,
        error: &StructuredParseError,
        filename: Option<&str>,
    ) -> String {
        let mut parts = Vec::new();

        // Header: error[E0001]: message
        parts.push(self.format_header(error));

        // Location: --> file.shape:1:5
        parts.push(self.format_location(error, filename));

        // Source context with highlights
        let source = self.format_source_context(error);
        if !source.is_empty() {
            parts.push(source);
        }

        // Related info
        let related = self.format_related(&error.related);
        if !related.is_empty() {
            parts.push(related);
        }

        // Suggestions
        let suggestions = self.format_suggestions(&error.suggestions);
        if !suggestions.is_empty() {
            parts.push(suggestions);
        }

        parts.join("\n")
    }
}

/// Get the matching close delimiter for an opener
fn matching_close(opener: char) -> char {
    match opener {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => opener,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{SourceLocation, parse_error::TokenInfo};

    #[test]
    fn test_format_expected_single() {
        let renderer = CliErrorRenderer::without_colors();
        let expected = vec![ExpectedToken::Literal(";".to_string())];
        assert_eq!(renderer.format_expected_list(&expected), "`;`");
    }

    #[test]
    fn test_format_expected_two() {
        let renderer = CliErrorRenderer::without_colors();
        let expected = vec![
            ExpectedToken::Category(TokenCategory::Identifier),
            ExpectedToken::Literal("(".to_string()),
        ];
        assert_eq!(
            renderer.format_expected_list(&expected),
            "identifier or `(`"
        );
    }

    #[test]
    fn test_format_expected_many() {
        let renderer = CliErrorRenderer::without_colors();
        let expected = vec![
            ExpectedToken::Category(TokenCategory::Identifier),
            ExpectedToken::Literal("(".to_string()),
            ExpectedToken::Literal("{".to_string()),
        ];
        assert_eq!(
            renderer.format_expected_list(&expected),
            "identifier, `(`, or `{`"
        );
    }

    #[test]
    fn test_format_token_info() {
        let renderer = CliErrorRenderer::without_colors();

        let token = TokenInfo::new(")").with_kind(TokenKind::Punctuation);
        assert_eq!(renderer.format_token_info(&token), "`)`");

        let token = TokenInfo::new("foo").with_kind(TokenKind::Identifier);
        assert_eq!(renderer.format_token_info(&token), "identifier `foo`");

        let token = TokenInfo::end_of_input();
        assert_eq!(renderer.format_token_info(&token), "end of input");
    }

    #[test]
    fn test_render_unexpected_token() {
        let renderer = CliErrorRenderer::without_colors();

        let error = StructuredParseError::new(
            ParseErrorKind::UnexpectedToken {
                found: TokenInfo::new(")").with_kind(TokenKind::Punctuation),
                expected: vec![ExpectedToken::Category(TokenCategory::Identifier)],
            },
            SourceLocation::new(1, 10),
        );

        let output = renderer.render(&error);
        assert!(output.contains("unexpected `)`"));
        assert!(output.contains("expected identifier"));
    }
}
