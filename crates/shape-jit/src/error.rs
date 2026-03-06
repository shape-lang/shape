//! JIT compiler error types.

/// Errors produced by the JIT compiler.
#[derive(Debug, thiserror::Error)]
pub enum JitError {
    /// Cranelift ISA or settings configuration failed.
    #[error("JIT setup: {0}")]
    Setup(String),

    /// Cranelift compilation or module error.
    #[error("JIT compilation: {0}")]
    Compilation(String),

    /// An opcode or bytecode construct is not supported by the JIT.
    #[error("unsupported opcode: {0}")]
    UnsupportedOpcode(String),

    /// Translation from bytecode to Cranelift IR failed.
    #[error("JIT translation: {0}")]
    Translation(String),

    /// A function referenced by the bytecode was not found.
    #[error("function not found: {0}")]
    FunctionNotFound(String),

    /// Type error during JIT compilation.
    #[error("JIT type error: {0}")]
    TypeError(String),
}

impl From<String> for JitError {
    fn from(s: String) -> Self {
        JitError::Compilation(s)
    }
}

impl From<JitError> for String {
    fn from(e: JitError) -> Self {
        e.to_string()
    }
}
