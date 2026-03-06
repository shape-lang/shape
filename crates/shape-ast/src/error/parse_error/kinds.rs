//! Parse error kinds and related enum types

use super::{ExpectedToken, TokenInfo};
use crate::error::SourceLocation;

/// Specific parse error variants with structured context
#[derive(Debug, Clone, PartialEq)]
pub enum ParseErrorKind {
    /// Parser expected specific tokens but found something else
    UnexpectedToken {
        /// What was found at this position
        found: TokenInfo,
        /// What tokens/rules were expected
        expected: Vec<ExpectedToken>,
    },

    /// End of input reached unexpectedly
    UnexpectedEof {
        /// What tokens/rules were expected
        expected: Vec<ExpectedToken>,
    },

    /// Unterminated string literal
    UnterminatedString {
        /// Where the string started
        start_location: SourceLocation,
        /// The delimiter used
        delimiter: StringDelimiter,
    },

    /// Unterminated block comment
    UnterminatedComment {
        /// Where the comment started
        start_location: SourceLocation,
    },

    /// Invalid number literal
    InvalidNumber {
        /// The invalid number text
        text: String,
        /// Why it's invalid
        reason: NumberError,
    },

    /// Unbalanced delimiter (brackets, braces, parentheses)
    UnbalancedDelimiter {
        /// The opening delimiter
        opener: char,
        /// Where it was opened
        open_location: SourceLocation,
        /// What was found instead of closing (if any)
        found: Option<char>,
    },

    /// Reserved keyword used as identifier
    ReservedKeyword {
        /// The keyword that was used
        keyword: String,
        /// Context where it was used
        context: IdentifierContext,
    },

    /// Invalid escape sequence in string
    InvalidEscape {
        /// The escape sequence text
        sequence: String,
        /// Valid escapes for reference
        valid_escapes: Vec<String>,
    },

    /// Invalid character in source
    InvalidCharacter {
        /// The invalid character
        char: char,
        /// Unicode codepoint for non-printable chars
        codepoint: u32,
    },

    /// Missing required component
    MissingComponent {
        /// What's missing
        component: MissingComponentKind,
        /// Where it should appear
        after: Option<String>,
    },

    /// Custom parse error from semantic validation during parsing
    Custom {
        /// Error message
        message: String,
    },
}

/// String delimiter types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StringDelimiter {
    DoubleQuote,
    SingleQuote,
    Backtick,
}

/// Why a number is invalid
#[derive(Debug, Clone, PartialEq)]
pub enum NumberError {
    MultipleDecimalPoints,
    InvalidExponent,
    TrailingDecimalPoint,
    LeadingZeros,
    InvalidDigit(char),
    TooLarge,
    Empty,
}

/// What component is missing
#[derive(Debug, Clone, PartialEq)]
pub enum MissingComponentKind {
    Semicolon,
    ClosingBrace,
    ClosingBracket,
    ClosingParen,
    FunctionBody,
    Expression,
    TypeAnnotation,
    Identifier,
    Arrow,
    Colon,
}

/// Context where an identifier was expected
#[derive(Debug, Clone, PartialEq)]
pub enum IdentifierContext {
    VariableName,
    FunctionName,
    ParameterName,
    PatternName,
    TypeName,
    PropertyName,
}

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorSeverity {
    #[default]
    Error,
    Warning,
    Info,
    Hint,
}
