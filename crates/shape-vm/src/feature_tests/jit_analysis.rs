//! JIT support detection and analysis
//!
//! This module provides functionality for analyzing whether a given
//! piece of Shape code can be JIT-compiled based on the opcodes it uses.

// ============================================================================
// JIT Support Detection
// ============================================================================

/// Check which opcodes a compiled program uses.
///
/// NOTE: Full JIT analysis requires the `crate::jit` module and parser
/// integration which are not yet wired. This stub always returns
/// "not yet wired" until those modules exist.
#[cfg(feature = "jit")]
pub fn analyze_jit_support(_code: &str) -> JitAnalysis {
    JitAnalysis {
        can_jit: false,
        unsupported_opcodes: vec![],
        error: Some("JIT analysis not yet wired (pending jit module integration)".to_string()),
    }
}

#[cfg(not(feature = "jit"))]
pub fn analyze_jit_support(_code: &str) -> JitAnalysis {
    JitAnalysis {
        can_jit: false,
        unsupported_opcodes: vec![],
        error: Some("JIT feature not enabled".to_string()),
    }
}

#[derive(Debug)]
pub struct JitAnalysis {
    pub can_jit: bool,
    pub unsupported_opcodes: Vec<String>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jit_analysis() {
        let analysis = analyze_jit_support("function test() { return 1 + 2; }");
        println!("JIT Analysis: {:?}", analysis);
    }
}
