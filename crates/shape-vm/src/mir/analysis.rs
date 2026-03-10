//! Borrow analysis results — the single source of truth.
//!
//! `BorrowAnalysis` is the shared result struct consumed by:
//! - The compiler (codegen decisions: move vs clone)
//! - The LSP (inlay hints, borrow windows, hover info)
//! - The diagnostic engine (error messages, repair suggestions)
//!
//! **DRY rule**: Analysis runs ONCE. No consumer re-derives these results.

use super::liveness::LivenessResult;
use super::types::*;
use shape_ast::ast::Span;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceReturnContract {
    pub param_index: usize,
    pub kind: BorrowKind,
}

/// The complete borrow analysis for a single function.
/// Produced by the Datafrog solver + liveness analysis.
/// Consumed (read-only) by compiler, LSP, and diagnostics.
#[derive(Debug)]
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
    /// If this function safely returns one reference parameter unchanged,
    /// records which parameter flows out and whether it is shared/exclusive.
    pub return_reference_contract: Option<ReferenceReturnContract>,
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
    /// Reference returns must consistently return the same parameter with the same borrow kind.
    InconsistentReferenceReturn,
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

/// Error for writing to an immutable binding.
#[derive(Debug, Clone)]
pub struct MutabilityError {
    /// The span of the write attempt.
    pub span: Span,
    /// The name of the immutable variable.
    pub variable_name: String,
    /// The span of the original declaration.
    pub declaration_span: Span,
    /// Whether this is a `let` (explicit immutable) or `var` (inferred immutable).
    pub is_explicit_let: bool,
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
            return_reference_contract: None,
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
}
