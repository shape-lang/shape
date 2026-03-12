//! Mid-level Intermediate Representation (MIR) for borrow checking.
//!
//! Shape compiles AST → MIR → bytecode. The MIR is a CFG-based IR used by:
//! - The Datafrog borrow solver (NLL borrow checking)
//! - Liveness analysis (smart move/clone inference)
//! - The repair engine (error fix candidate generation)
//!
//! The MIR is lowered from AST before bytecode compilation. Analysis results
//! (`BorrowAnalysis`) are shared by the compiler, LSP, and diagnostic engine.

pub mod analysis;
pub mod cfg;
pub mod field_analysis;
pub mod liveness;
pub mod lowering;
pub mod repair;
pub mod solver;
pub mod storage_planning;
pub mod types;

pub use analysis::{BorrowAnalysis, BorrowErrorCode, BorrowErrorKind, FunctionBorrowSummary};
pub use cfg::ControlFlowGraph;
pub use field_analysis::FieldAnalysis;
pub use liveness::LivenessResult;
pub use storage_planning::StoragePlan;
pub use types::*;
