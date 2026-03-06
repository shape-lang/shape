// ShapeError carries location info for good diagnostics, making it larger than clippy's threshold.
// Boxing it everywhere would hurt ergonomics across the entire codebase.
#![allow(clippy::result_large_err)]

pub mod ast;
pub mod data;
pub mod error;
pub mod int_width;
pub mod interpolation;
pub mod parser;
pub mod transform;

pub use ast::*;
pub use data::{Timeframe, TimeframeUnit};
pub use error::{Result, ShapeError, SourceLocation};
pub use int_width::IntWidth;
pub use parser::parse_program;
pub use parser::resilient::{ParseError, ParseErrorKind, PartialProgram, parse_program_resilient};
pub use transform::desugar_program;
