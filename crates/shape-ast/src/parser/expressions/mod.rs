//! Expression parsing module
//!
//! This module delegates expression parsing to specialized submodules.
//! The actual implementation is split across multiple files for better organization.

pub mod binary_ops;
pub mod comprehensions;
pub mod control_flow;
pub mod data_refs;
pub mod functions;
pub mod literals;
pub mod primary;
pub mod temporal;
pub mod window;

// Re-export the main parsing functions
pub use binary_ops::parse_comparison_op;
pub use control_flow::parse_pattern;
pub use primary::{parse_expression, parse_postfix_expr, parse_primary_expr};
pub use window::{parse_window_from_function_call, parse_window_function_call};

// All implementation has been moved to submodules
