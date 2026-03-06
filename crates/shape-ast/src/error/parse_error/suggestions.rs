//! Suggestion and related information types for error fixes

use crate::error::SourceLocation;

/// A suggestion for fixing the error
#[derive(Debug, Clone)]
pub struct Suggestion {
    /// Human-readable description
    pub message: String,
    /// Machine-applicable text edit (if applicable)
    pub edit: Option<TextEdit>,
    /// Confidence level
    pub confidence: SuggestionConfidence,
}

impl Suggestion {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            edit: None,
            confidence: SuggestionConfidence::Maybe,
        }
    }

    pub fn certain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            edit: None,
            confidence: SuggestionConfidence::Certain,
        }
    }

    pub fn likely(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            edit: None,
            confidence: SuggestionConfidence::Likely,
        }
    }

    pub fn with_edit(mut self, edit: TextEdit) -> Self {
        self.edit = Some(edit);
        self
    }
}

/// Text edit for auto-fix
#[derive(Debug, Clone)]
pub struct TextEdit {
    /// Start position (line, column, both 1-based)
    pub start: (usize, usize),
    /// End position
    pub end: (usize, usize),
    /// Replacement text
    pub new_text: String,
}

impl TextEdit {
    pub fn insert(line: usize, col: usize, text: impl Into<String>) -> Self {
        Self {
            start: (line, col),
            end: (line, col),
            new_text: text.into(),
        }
    }

    pub fn replace(start: (usize, usize), end: (usize, usize), text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            new_text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SuggestionConfidence {
    Certain, // Definitely correct fix
    Likely,  // Probably correct
    Maybe,   // One of several possibilities
}

/// Related information (e.g., "unclosed brace opened here")
#[derive(Debug, Clone)]
pub struct RelatedInfo {
    /// Location of related code
    pub location: SourceLocation,
    /// Description
    pub message: String,
}

impl RelatedInfo {
    pub fn new(message: impl Into<String>, location: SourceLocation) -> Self {
        Self {
            location,
            message: message.into(),
        }
    }
}
