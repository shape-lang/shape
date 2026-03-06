//! Semantic validation rules for Shape

/// Validator for semantic rules
pub struct Validator {
    /// Source code for error location reporting (optional)
    source: Option<String>,
}

impl Validator {
    /// Create a new validator with default configuration
    pub fn new() -> Self {
        Self { source: None }
    }

    /// Set the source code for better error location reporting
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set the source code (mutable reference version)
    pub fn set_source(&mut self, source: impl Into<String>) {
        self.source = Some(source.into());
    }
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
    }
}
