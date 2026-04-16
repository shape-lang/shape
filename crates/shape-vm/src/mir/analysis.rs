//! Borrow analysis results — the single source of truth.
//!
//! `BorrowAnalysis` is the shared result struct consumed by:
//! - The compiler (codegen decisions: move vs clone)
//! - The LSP (inlay hints, borrow windows, hover info)
//! - The diagnostic engine (error messages, repair suggestions)
//!
//! **DRY rule**: Analysis runs ONCE. No consumer re-derives these results.

use super::liveness::LivenessResult;
use shape_value::ValueWordExt;
use super::types::*;
use shape_ast::ast::Span;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReturnReferenceSummary {
    pub param_index: usize,
    pub kind: BorrowKind,
    /// Exact projection chain when every successful return path agrees on it.
    /// `None` means "same parameter root, but projection differs across paths".
    pub projection: Option<Vec<ProjectionStep>>,
}

/// A normalized origin for a first-class reference value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceOrigin {
    pub root: ReferenceOriginRoot,
    pub projection: Vec<ProjectionStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceOriginRoot {
    Param(usize),
    Local(SlotId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoanSinkKind {
    ReturnSlot,
    ClosureEnv,
    ArrayStore,
    ObjectStore,
    EnumStore,
    ArrayAssignment,
    ObjectAssignment,
    StructuredTaskBoundary,
    DetachedTaskBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoanSink {
    pub loan_id: u32,
    pub kind: LoanSinkKind,
    /// The slot that owns the sink when this is a closure or aggregate sink.
    pub sink_slot: Option<SlotId>,
    pub span: Span,
}

/// The complete borrow analysis for a single function.
/// Produced by the Datafrog solver + liveness analysis.
/// Consumed (read-only) by compiler, LSP, and diagnostics.
#[derive(Debug, Clone)]
pub struct BorrowAnalysis {
    /// Liveness results for move/clone decisions.
    pub liveness: LivenessResult,
    /// Active loans at each program point (from Datafrog solver).
    pub loans_at_point: HashMap<Point, Vec<LoanId>>,
    /// Loan metadata.
    pub loans: HashMap<LoanId, LoanInfo>,
    /// Borrow errors detected by the solver.
    pub errors: Vec<BorrowError>,
    /// Move/clone decisions for each assignment of a non-Copy type.
    pub ownership_decisions: HashMap<Point, OwnershipDecision>,
    /// Immutability violations (writing to immutable bindings).
    pub mutability_errors: Vec<MutabilityError>,
    /// If this function safely returns one reference parameter (possibly with a
    /// projection), records which parameter flows out and whether it is
    /// shared/exclusive.
    pub return_reference_summary: Option<ReturnReferenceSummary>,
}

/// Information about a single loan (borrow).
#[derive(Debug, Clone)]
pub struct LoanInfo {
    pub id: LoanId,
    /// The place being borrowed.
    pub borrowed_place: Place,
    /// Kind of borrow (shared or exclusive).
    pub kind: BorrowKind,
    /// Where the loan was issued.
    pub issued_at: Point,
    /// Source span of the borrow expression.
    pub span: Span,
    /// Nesting depth of the borrow's scope: 0 = parameter, 1 = function body local.
    pub region_depth: u32,
}

/// A borrow conflict error with structured data for diagnostics.
/// The diagnostic engine formats this; consumers never generate error text.
#[derive(Debug, Clone)]
pub struct BorrowError {
    pub kind: BorrowErrorKind,
    /// Primary span (the conflicting operation).
    pub span: Span,
    /// The loan that conflicts.
    pub conflicting_loan: LoanId,
    /// Where the conflicting loan was created.
    pub loan_span: Span,
    /// Where the loan is still needed (last use).
    pub last_use_span: Option<Span>,
    /// Repair candidates, ordered by preference.
    pub repairs: Vec<RepairCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BorrowErrorKind {
    /// Cannot borrow as mutable while shared borrow is active.
    ConflictSharedExclusive,
    /// Cannot borrow as mutable while another mutable borrow is active.
    ConflictExclusiveExclusive,
    /// Cannot read while exclusively borrowed.
    ReadWhileExclusivelyBorrowed,
    /// Cannot write while any borrow is active.
    WriteWhileBorrowed,
    /// Reference escapes its scope.
    ReferenceEscape,
    /// Reference stored into an array.
    ReferenceStoredInArray,
    /// Reference stored into an object or struct literal.
    ReferenceStoredInObject,
    /// Reference stored into an enum payload.
    ReferenceStoredInEnum,
    /// Reference escapes into a closure environment.
    ReferenceEscapeIntoClosure,
    /// Use after move.
    UseAfterMove,
    /// Cannot share exclusive reference across task boundary.
    ExclusiveRefAcrossTaskBoundary,
    /// Cannot share any reference across detached task boundary.
    SharedRefAcrossDetachedTask,
    /// Reference returns must produce a reference on every path from the same
    /// borrowed origin and borrow kind.
    InconsistentReferenceReturn,
    /// Two arguments at a call site alias the same variable but the callee
    /// requires them to be non-aliased (one is mutated, the other is read).
    CallSiteAliasConflict,
    /// Non-sendable value (e.g., closure with mutable captures) sent across
    /// a detached task boundary.
    NonSendableAcrossTaskBoundary,
}

/// Stable, user-facing borrow error codes.
///
/// These provide a documented mapping from internal `BorrowErrorKind` variants
/// to the `[B00XX]` codes shown in compiler and LSP diagnostics.  Both the
/// lexical borrow checker (`borrow_checker.rs`) and the MIR-based checker use
/// the same code space so users see consistent identifiers regardless of which
/// analysis detected the problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BorrowErrorCode {
    /// Borrow conflict (aliasing violation): shared+exclusive or exclusive+exclusive.
    B0001,
    /// Write to the owner while a borrow is active.
    B0002,
    /// Reference escapes its scope (return, store in collection, closure capture).
    B0003,
    /// Reference stored in a collection (array, object, enum).
    B0004,
    /// Use after move.
    B0005,
    /// Exclusive reference sent across a task/async boundary.
    B0006,
    /// Inconsistent return-reference summary across branches.
    B0007,
    /// Shared reference sent across a detached task boundary.
    B0012,
    /// Call-site alias conflict: same variable passed to conflicting parameters.
    B0013,
    /// Non-sendable value across detached task boundary.
    B0014,
}

impl BorrowErrorCode {
    /// The string form used in diagnostic messages, e.g. `"B0001"`.
    pub fn as_str(self) -> &'static str {
        match self {
            BorrowErrorCode::B0001 => "B0001",
            BorrowErrorCode::B0002 => "B0002",
            BorrowErrorCode::B0003 => "B0003",
            BorrowErrorCode::B0004 => "B0004",
            BorrowErrorCode::B0005 => "B0005",
            BorrowErrorCode::B0006 => "B0006",
            BorrowErrorCode::B0007 => "B0007",
            BorrowErrorCode::B0012 => "B0012",
            BorrowErrorCode::B0013 => "B0013",
            BorrowErrorCode::B0014 => "B0014",
        }
    }
}

impl std::fmt::Display for BorrowErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl BorrowErrorKind {
    /// Map this error kind to the stable user-facing error code.
    pub fn code(&self) -> BorrowErrorCode {
        match self {
            BorrowErrorKind::ConflictSharedExclusive
            | BorrowErrorKind::ConflictExclusiveExclusive
            | BorrowErrorKind::ReadWhileExclusivelyBorrowed => BorrowErrorCode::B0001,

            BorrowErrorKind::WriteWhileBorrowed => BorrowErrorCode::B0002,

            BorrowErrorKind::ReferenceEscape
            | BorrowErrorKind::ReferenceEscapeIntoClosure => BorrowErrorCode::B0003,

            BorrowErrorKind::ReferenceStoredInArray
            | BorrowErrorKind::ReferenceStoredInObject
            | BorrowErrorKind::ReferenceStoredInEnum => BorrowErrorCode::B0004,

            BorrowErrorKind::UseAfterMove => BorrowErrorCode::B0005,

            BorrowErrorKind::ExclusiveRefAcrossTaskBoundary => BorrowErrorCode::B0006,

            BorrowErrorKind::SharedRefAcrossDetachedTask => BorrowErrorCode::B0012,

            BorrowErrorKind::InconsistentReferenceReturn => BorrowErrorCode::B0007,

            BorrowErrorKind::CallSiteAliasConflict => BorrowErrorCode::B0013,

            BorrowErrorKind::NonSendableAcrossTaskBoundary => BorrowErrorCode::B0014,
        }
    }
}

/// A repair candidate (fix suggestion) verified by re-running the solver.
#[derive(Debug, Clone)]
pub struct RepairCandidate {
    pub kind: RepairKind,
    /// Human-readable description of the fix.
    pub description: String,
    /// Concrete code diff (if available).
    pub diff: Option<RepairDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairKind {
    /// Reorder: move the conflicting statement after the last use of the blocking loan.
    Reorder,
    /// Scope: wrap the first borrow + its uses in a block `{ }`.
    Scope,
    /// Clone: suggest `clone x` instead of borrowing.
    Clone,
    /// Downgrade: change `&mut` to `&` if only reads exist.
    Downgrade,
    /// Extract: suggest extracting into a helper function.
    Extract,
}

/// A concrete code change for a repair suggestion.
#[derive(Debug, Clone)]
pub struct RepairDiff {
    /// Lines to remove (span + original text).
    pub removals: Vec<(Span, String)>,
    /// Lines to add (span + replacement text).
    pub additions: Vec<(Span, String)>,
}

/// The ownership decision for an assignment of a non-Copy type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipDecision {
    /// Move: source is dead after this point. Zero cost.
    Move,
    /// Clone: source is live after this point. Requires T: Clone.
    Clone,
    /// Copy: type is Copy (primitive). Trivially copied.
    Copy,
}

/// Summary of a function's parameter borrow requirements.
/// Used for interprocedural alias checking at call sites.
#[derive(Debug, Clone)]
pub struct FunctionBorrowSummary {
    /// Per-parameter borrow mode: None = owned, Some(Shared/Exclusive) = by reference.
    pub param_borrows: Vec<Option<BorrowKind>>,
    /// Pairs of parameter indices that must not alias (one is mutated, the other is read).
    pub conflict_pairs: Vec<(usize, usize)>,
    /// If the function returns a reference derived from a parameter, records which
    /// parameter and borrow kind. Used for interprocedural composition.
    pub return_summary: Option<ReturnReferenceSummary>,
}

/// Error for writing to an immutable binding.
#[derive(Debug, Clone)]
pub struct MutabilityError {
    /// The span of the write attempt.
    pub span: Span,
    /// The name of the immutable variable.
    pub variable_name: String,
    /// The span of the original declaration.
    pub declaration_span: Span,
    /// Whether this is an explicit immutable `let`.
    pub is_explicit_let: bool,
    /// Whether this is a `const` binding.
    pub is_const: bool,
}

impl BorrowAnalysis {
    /// Create an empty analysis (used as default before solver runs).
    pub fn empty() -> Self {
        BorrowAnalysis {
            liveness: LivenessResult {
                live_in: HashMap::new(),
                live_out: HashMap::new(),
            },
            loans_at_point: HashMap::new(),
            loans: HashMap::new(),
            errors: Vec::new(),
            ownership_decisions: HashMap::new(),
            mutability_errors: Vec::new(),
            return_reference_summary: None,
        }
    }

    /// Check if the analysis found any errors.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || !self.mutability_errors.is_empty()
    }

    /// Get the ownership decision for a given point.
    /// Returns Copy for primitive types.
    pub fn ownership_at(&self, point: Point) -> OwnershipDecision {
        self.ownership_decisions
            .get(&point)
            .copied()
            .unwrap_or(OwnershipDecision::Copy)
    }

    /// Get all active loans at a given point (for LSP borrow windows).
    pub fn active_loans_at(&self, point: Point) -> &[LoanId] {
        self.loans_at_point
            .get(&point)
            .map_or(&[], |v| v.as_slice())
    }

    /// Get loan info by ID.
    pub fn loan(&self, id: LoanId) -> Option<&LoanInfo> {
        self.loans.get(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_analysis() {
        let analysis = BorrowAnalysis::empty();
        assert!(!analysis.has_errors());
        assert_eq!(analysis.ownership_at(Point(0)), OwnershipDecision::Copy);
        assert!(analysis.active_loans_at(Point(0)).is_empty());
    }

    // =========================================================================
    // Error code mapping tests (Task 4)
    // =========================================================================

    #[test]
    fn test_conflict_shared_exclusive_maps_to_b0001() {
        assert_eq!(
            BorrowErrorKind::ConflictSharedExclusive.code(),
            BorrowErrorCode::B0001
        );
    }

    #[test]
    fn test_conflict_exclusive_exclusive_maps_to_b0001() {
        assert_eq!(
            BorrowErrorKind::ConflictExclusiveExclusive.code(),
            BorrowErrorCode::B0001
        );
    }

    #[test]
    fn test_read_while_exclusively_borrowed_maps_to_b0001() {
        assert_eq!(
            BorrowErrorKind::ReadWhileExclusivelyBorrowed.code(),
            BorrowErrorCode::B0001
        );
    }

    #[test]
    fn test_write_while_borrowed_maps_to_b0002() {
        assert_eq!(
            BorrowErrorKind::WriteWhileBorrowed.code(),
            BorrowErrorCode::B0002
        );
    }

    #[test]
    fn test_reference_escape_maps_to_b0003() {
        assert_eq!(
            BorrowErrorKind::ReferenceEscape.code(),
            BorrowErrorCode::B0003
        );
    }

    #[test]
    fn test_reference_escape_into_closure_maps_to_b0003() {
        assert_eq!(
            BorrowErrorKind::ReferenceEscapeIntoClosure.code(),
            BorrowErrorCode::B0003
        );
    }

    #[test]
    fn test_reference_stored_in_array_maps_to_b0004() {
        assert_eq!(
            BorrowErrorKind::ReferenceStoredInArray.code(),
            BorrowErrorCode::B0004
        );
    }

    #[test]
    fn test_reference_stored_in_object_maps_to_b0004() {
        assert_eq!(
            BorrowErrorKind::ReferenceStoredInObject.code(),
            BorrowErrorCode::B0004
        );
    }

    #[test]
    fn test_reference_stored_in_enum_maps_to_b0004() {
        assert_eq!(
            BorrowErrorKind::ReferenceStoredInEnum.code(),
            BorrowErrorCode::B0004
        );
    }

    #[test]
    fn test_use_after_move_maps_to_b0005() {
        assert_eq!(
            BorrowErrorKind::UseAfterMove.code(),
            BorrowErrorCode::B0005
        );
    }

    #[test]
    fn test_exclusive_ref_across_task_boundary_maps_to_b0006() {
        assert_eq!(
            BorrowErrorKind::ExclusiveRefAcrossTaskBoundary.code(),
            BorrowErrorCode::B0006
        );
    }

    #[test]
    fn test_inconsistent_reference_return_maps_to_b0007() {
        assert_eq!(
            BorrowErrorKind::InconsistentReferenceReturn.code(),
            BorrowErrorCode::B0007
        );
    }

    #[test]
    fn test_borrow_error_code_as_str() {
        assert_eq!(BorrowErrorCode::B0001.as_str(), "B0001");
        assert_eq!(BorrowErrorCode::B0002.as_str(), "B0002");
        assert_eq!(BorrowErrorCode::B0003.as_str(), "B0003");
        assert_eq!(BorrowErrorCode::B0004.as_str(), "B0004");
        assert_eq!(BorrowErrorCode::B0005.as_str(), "B0005");
        assert_eq!(BorrowErrorCode::B0006.as_str(), "B0006");
        assert_eq!(BorrowErrorCode::B0007.as_str(), "B0007");
    }

    #[test]
    fn test_borrow_error_code_display() {
        assert_eq!(format!("{}", BorrowErrorCode::B0001), "B0001");
        assert_eq!(format!("{}", BorrowErrorCode::B0007), "B0007");
    }

    #[test]
    fn test_all_error_kinds_have_codes() {
        // Exhaustive check: every BorrowErrorKind variant must map to some code.
        let all_kinds = vec![
            BorrowErrorKind::ConflictSharedExclusive,
            BorrowErrorKind::ConflictExclusiveExclusive,
            BorrowErrorKind::ReadWhileExclusivelyBorrowed,
            BorrowErrorKind::WriteWhileBorrowed,
            BorrowErrorKind::ReferenceEscape,
            BorrowErrorKind::ReferenceStoredInArray,
            BorrowErrorKind::ReferenceStoredInObject,
            BorrowErrorKind::ReferenceStoredInEnum,
            BorrowErrorKind::ReferenceEscapeIntoClosure,
            BorrowErrorKind::UseAfterMove,
            BorrowErrorKind::ExclusiveRefAcrossTaskBoundary,
            BorrowErrorKind::SharedRefAcrossDetachedTask,
            BorrowErrorKind::InconsistentReferenceReturn,
        ];
        for kind in all_kinds {
            // Should not panic — every variant is covered.
            let _code = kind.code();
        }
    }
}
