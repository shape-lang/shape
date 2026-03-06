//! Token information types for error messages

/// Information about a found token
#[derive(Debug, Clone, PartialEq)]
pub struct TokenInfo {
    /// The token text (truncated if very long)
    pub text: String,
    /// The token kind (if identifiable)
    pub kind: Option<TokenKind>,
}

impl TokenInfo {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: None,
        }
    }

    pub fn with_kind(mut self, kind: TokenKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn end_of_input() -> Self {
        Self {
            text: String::new(),
            kind: Some(TokenKind::EndOfInput),
        }
    }
}

/// Token kinds for better error messages
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Identifier,
    Keyword(String),
    Number,
    String,
    Operator,
    Punctuation,
    EndOfInput,
    Whitespace,
    Comment,
    Unknown,
}

/// What token/construct was expected
#[derive(Debug, Clone, PartialEq)]
pub enum ExpectedToken {
    /// A specific literal token
    Literal(String),
    /// A grammar rule (will be converted to user-friendly name)
    Rule(String),
    /// A category of tokens
    Category(TokenCategory),
}

impl ExpectedToken {
    pub fn literal(s: impl Into<String>) -> Self {
        Self::Literal(s.into())
    }

    pub fn rule(s: impl Into<String>) -> Self {
        Self::Rule(s.into())
    }

    pub fn category(c: TokenCategory) -> Self {
        Self::Category(c)
    }
}

/// Categories of expected tokens for grouping
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TokenCategory {
    Expression,
    Statement,
    Type,
    Pattern,
    Identifier,
    Literal,
    Operator,
    Delimiter,
}
