//! Standard library loading
//!
//! Only `stdlib/core/` is autoloaded. Domain-specific modules (finance, iot, etc.)
//! must be explicitly imported by user code.

use shape_ast::error::Result;

impl super::ShapeEngine {
    /// Load stdlib core modules only.
    /// Domain-specific modules (finance, iot, etc.) require explicit import.
    pub fn load_stdlib(&mut self) -> Result<()> {
        self.runtime
            .load_core_stdlib_into_context(&self.default_data)
    }
}
