//! Automatic feature coverage tracking for Interpreter/VM/JIT parity
//!
//! This module provides:
//! 1. Grammar feature definitions extracted from pest (via build.rs)
//! 2. Feature test case definitions
//! 3. Coverage gap detection
//! 4. Three-way parity testing (Interpreter vs VM vs JIT)

// Include auto-generated grammar rules from build.rs
include!(concat!(env!("OUT_DIR"), "/grammar_features.rs"));

// ============================================================================
// Submodules
// ============================================================================

pub mod annotation_tests;
pub mod backends;
pub mod coverage;
pub mod definitions;
pub mod jit_analysis;
pub mod module_tests;
pub mod parity;
pub mod parity_runner;
pub mod pattern_tests;
pub mod query_tests;
pub mod stream_tests;
pub mod testing_framework_tests;
pub mod type_system_tests;
pub mod window_tests;

// ============================================================================
// Feature Registry - tracks all declared test cases
// ============================================================================

/// A single feature test case
#[derive(Debug, Clone)]
pub struct FeatureTest {
    /// Unique name of the test
    pub name: &'static str,
    /// Grammar rule(s) this test covers
    pub covers: &'static [&'static str],
    /// Shape code to execute
    pub code: &'static str,
    /// Function to call
    pub function: &'static str,
    /// Category (expression, statement, control_flow, etc.)
    pub category: FeatureCategory,
    /// Whether this test requires market data
    pub requires_data: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FeatureCategory {
    /// Literal values (numbers, strings, booleans)
    Literal,
    /// Arithmetic and comparison operators
    Operator,
    /// Control flow (if, match, loops)
    ControlFlow,
    /// Variable declarations and assignments
    Variable,
    /// Function definitions and calls
    Function,
    /// Array and object operations
    Collection,
    /// Domain-specific (data rows, series, indicators)
    Domain,
    /// Exception handling
    Exception,
    /// Type system features
    TypeSystem,
    /// Module system
    Module,
}

// ============================================================================
// Aggregated Feature Tests
// ============================================================================

/// Returns all feature tests from all modules for coverage analysis
pub fn all_feature_tests() -> Vec<&'static FeatureTest> {
    let mut all = Vec::new();
    all.extend(definitions::FEATURE_TESTS.iter());
    all.extend(type_system_tests::TESTS.iter());
    all.extend(module_tests::TESTS.iter());
    all.extend(pattern_tests::TESTS.iter());
    all.extend(query_tests::TESTS.iter());
    all.extend(window_tests::TESTS.iter());
    all.extend(stream_tests::TESTS.iter());
    all.extend(annotation_tests::TESTS.iter());
    all.extend(testing_framework_tests::TESTS.iter());
    all
}

// ============================================================================
// Re-exports
// ============================================================================

pub use backends::{BackendExecutor, InterpreterBackend, JITBackend, VMBackend};
pub use coverage::{CoverageReport, analyze_coverage};
pub use definitions::FEATURE_TESTS as MANUAL_FEATURE_TESTS;
pub use jit_analysis::{JitAnalysis, analyze_jit_support};
pub use parity::{ExecutionResult, ParityResult, ParityStatus};
pub use parity_runner::{ParityReport, ParityRunner};
