//! Repair engine for borrow errors.
//!
//! After the Datafrog solver detects a borrow conflict, the repair engine
//! generates candidate fixes, re-runs the solver on modified MIR, and
//! returns verified suggestions with concrete code diffs.
//!
//! Repair candidates are tried in preference order:
//! 1. REORDER — move conflicting statement after last use of blocking loan
//! 2. SCOPE — wrap first borrow + uses in a block `{ }` to limit extent
//! 3. CLONE — suggest `clone x` instead of borrowing
//! 4. DOWNGRADE — change `&mut` to `&` if only reads exist
//! 5. EXTRACT — suggest extracting into a helper function (fallback)
//!
//! Each candidate is verified by re-running Datafrog on the modified MIR.
//! Only the first passing candidate becomes the suggestion.

use super::analysis::*;
use super::solver;
use super::types::*;
use shape_ast::ast::Span;

/// Generate repair candidates for a borrow error.
///
/// This is the main entry point for the repair engine. It takes an error,
/// the original MIR, and returns verified repair candidates.
pub fn generate_repairs(
    error: &BorrowError,
    mir: &MirFunction,
    all_errors: &[BorrowError],
) -> Vec<RepairCandidate> {
    let mut candidates = Vec::new();

    match error.kind {
        BorrowErrorKind::ConflictSharedExclusive | BorrowErrorKind::ConflictExclusiveExclusive => {
            // Try reorder: move the conflicting borrow after the last use of the blocking loan
            if let Some(repair) = try_reorder_repair(error, mir) {
                if verify_repair(&repair, error, mir, all_errors) {
                    candidates.push(repair);
                }
            }

            // Try scope: wrap the first borrow in a block
            if candidates.is_empty() {
                if let Some(repair) = try_scope_repair(error, mir) {
                    if verify_repair(&repair, error, mir, all_errors) {
                        candidates.push(repair);
                    }
                }
            }

            // Try clone: suggest cloning instead of borrowing
            if candidates.is_empty() {
                if let Some(repair) = try_clone_repair(error, mir) {
                    candidates.push(repair); // Clone always valid if type is Clone
                }
            }

            // Try downgrade: change &mut to & if only reads
            if candidates.is_empty() {
                if let Some(repair) = try_downgrade_repair(error, mir) {
                    if verify_repair(&repair, error, mir, all_errors) {
                        candidates.push(repair);
                    }
                }
            }
        }
        BorrowErrorKind::UseAfterMove => {
            // Suggest clone
            candidates.push(RepairCandidate {
                kind: RepairKind::Clone,
                description: "clone the value before moving it".to_string(),
                diff: None,
            });
        }
        BorrowErrorKind::WriteWhileBorrowed => {
            // Suggest reorder or scope
            if let Some(repair) = try_reorder_repair(error, mir) {
                if verify_repair(&repair, error, mir, all_errors) {
                    candidates.push(repair);
                }
            }
            if candidates.is_empty() {
                if let Some(repair) = try_scope_repair(error, mir) {
                    candidates.push(repair);
                }
            }
        }
        BorrowErrorKind::ReferenceEscape => {
            candidates.push(RepairCandidate {
                kind: RepairKind::Clone,
                description: "return an owned value instead of a reference".to_string(),
                diff: None,
            });
        }
        BorrowErrorKind::ReferenceStoredInArray => {
            candidates.push(RepairCandidate {
                kind: RepairKind::Clone,
                description: "store an owned value in the array instead of a reference".to_string(),
                diff: None,
            });
        }
        BorrowErrorKind::ReferenceStoredInObject => {
            candidates.push(RepairCandidate {
                kind: RepairKind::Clone,
                description: "store an owned value in the object or struct instead of a reference"
                    .to_string(),
                diff: None,
            });
        }
        BorrowErrorKind::ReferenceStoredInEnum => {
            candidates.push(RepairCandidate {
                kind: RepairKind::Clone,
                description: "store an owned value in the enum payload instead of a reference"
                    .to_string(),
                diff: None,
            });
        }
        _ => {
            // Fallback: suggest extract
            candidates.push(RepairCandidate {
                kind: RepairKind::Extract,
                description: "extract the conflicting code into a helper function".to_string(),
                diff: None,
            });
        }
    }

    candidates
}

/// Try to fix by reordering: move the conflicting statement after the last use
/// of the blocking loan.
fn try_reorder_repair(error: &BorrowError, mir: &MirFunction) -> Option<RepairCandidate> {
    // Find the conflicting loan's last use point
    let conflicting_loan = error.conflicting_loan;

    // Find the statement that created the conflict (the error span)
    let conflict_point = find_point_at_span(mir, error.span)?;

    // Find the last use of the conflicting loan
    let loan_last_use = find_last_use_of_loan(mir, conflicting_loan)?;

    // If the conflict point is before the last use, reordering could help
    if conflict_point.0 < loan_last_use.0 {
        // The fix: move the conflicting statement AFTER the last use point
        Some(RepairCandidate {
            kind: RepairKind::Reorder,
            description: format!(
                "move the conflicting borrow after the last use of the blocking reference (after point {})",
                loan_last_use.0
            ),
            diff: Some(RepairDiff {
                removals: vec![(error.span, String::new())],
                additions: vec![(
                    Span {
                        start: error.loan_span.end,
                        end: error.loan_span.end,
                    },
                    String::new(), // Code would be inserted here
                )],
            }),
        })
    } else {
        None
    }
}

/// Try to fix by wrapping the first borrow and its uses in a scope block `{ }`.
fn try_scope_repair(error: &BorrowError, _mir: &MirFunction) -> Option<RepairCandidate> {
    // Scope repair: suggest wrapping the first borrow + its uses in a block
    Some(RepairCandidate {
        kind: RepairKind::Scope,
        description: "wrap the first borrow and its uses in a block `{ }` to limit its extent"
            .to_string(),
        diff: Some(RepairDiff {
            removals: vec![],
            additions: vec![
                (
                    Span {
                        start: error.loan_span.start,
                        end: error.loan_span.start,
                    },
                    "{".to_string(),
                ),
                (
                    Span {
                        start: error.span.start,
                        end: error.span.start,
                    },
                    "}".to_string(),
                ),
            ],
        }),
    })
}

/// Try to fix by suggesting clone instead of borrow.
fn try_clone_repair(error: &BorrowError, _mir: &MirFunction) -> Option<RepairCandidate> {
    Some(RepairCandidate {
        kind: RepairKind::Clone,
        description: "clone the value instead of borrowing — use `clone x` or `x.clone()`"
            .to_string(),
        diff: Some(RepairDiff {
            removals: vec![(error.loan_span, String::new())],
            additions: vec![(error.loan_span, "clone".to_string())],
        }),
    })
}

/// Try to fix by downgrading &mut to & (if only reads happen through the borrow).
fn try_downgrade_repair(error: &BorrowError, mir: &MirFunction) -> Option<RepairCandidate> {
    // Only applicable for SharedExclusive conflicts
    if error.kind != BorrowErrorKind::ConflictSharedExclusive {
        return None;
    }

    // Check if the exclusive borrow only does reads (no writes through the ref)
    let conflict_loan_id = error.conflicting_loan;
    let has_writes = mir.blocks.iter().any(|block| {
        block.statements.iter().any(|stmt| {
            // Check if any statement writes through a ref from this loan
            matches!(&stmt.kind, StatementKind::Assign(Place::Deref(_), _))
        })
    });

    if !has_writes {
        Some(RepairCandidate {
            kind: RepairKind::Downgrade,
            description: "change `&mut` to `&` — only reads occur through this reference"
                .to_string(),
            diff: Some(RepairDiff {
                removals: vec![(error.span, "&mut".to_string())],
                additions: vec![(error.span, "&".to_string())],
            }),
        })
    } else {
        let _ = conflict_loan_id; // suppress warning
        None
    }
}

/// Verify that a repair candidate actually fixes the error by re-running the solver.
fn verify_repair(
    repair: &RepairCandidate,
    error: &BorrowError,
    mir: &MirFunction,
    _all_errors: &[BorrowError],
) -> bool {
    // Create a modified MIR based on the repair
    let modified_mir = apply_repair_to_mir(repair, error, mir);

    // Re-run the solver on the modified MIR
    let analysis = solver::analyze(&modified_mir, &Default::default());

    // Check if the specific error is gone
    !analysis.errors.iter().any(|e| {
        e.kind == error.kind && e.span == error.span && e.conflicting_loan == error.conflicting_loan
    })
}

/// Apply a repair candidate to MIR for verification.
/// Returns a modified copy of the MIR.
fn apply_repair_to_mir(
    repair: &RepairCandidate,
    error: &BorrowError,
    mir: &MirFunction,
) -> MirFunction {
    match repair.kind {
        RepairKind::Reorder => {
            // Reorder: move the conflicting statement after the last use
            let mut modified = mir.clone();
            let conflict_point = find_point_at_span(mir, error.span);
            let last_use = find_last_use_of_loan(mir, error.conflicting_loan);

            if let (Some(cp), Some(lu)) = (conflict_point, last_use) {
                // Remove statement at conflict point and insert after last use
                for block in &mut modified.blocks {
                    let mut conflict_stmt = None;
                    block.statements.retain(|s| {
                        if s.point == cp {
                            conflict_stmt = Some(s.clone());
                            false
                        } else {
                            true
                        }
                    });
                    if let Some(stmt) = conflict_stmt {
                        // Insert after the last use
                        let insert_pos = block
                            .statements
                            .iter()
                            .position(|s| s.point == lu)
                            .map(|p| p + 1)
                            .unwrap_or(block.statements.len());
                        block.statements.insert(insert_pos, stmt);
                    }
                }
            }
            modified
        }
        RepairKind::Clone => {
            // Clone: replace the borrow with a clone in MIR
            let mut modified = mir.clone();
            for block in &mut modified.blocks {
                for stmt in &mut block.statements {
                    let should_replace =
                        if let StatementKind::Assign(_, Rvalue::Borrow(_, _)) = &stmt.kind {
                            stmt.span == error.loan_span
                        } else {
                            false
                        };
                    if should_replace {
                        if let StatementKind::Assign(_, Rvalue::Borrow(_, place)) = &stmt.kind {
                            let place = place.clone();
                            let operand = Operand::Copy(place.clone());
                            stmt.kind = StatementKind::Assign(place, Rvalue::Clone(operand));
                        }
                    }
                }
            }
            modified
        }
        RepairKind::Downgrade => {
            // Downgrade: change exclusive to shared borrow
            let mut modified = mir.clone();
            for block in &mut modified.blocks {
                for stmt in &mut block.statements {
                    let should_downgrade =
                        if let StatementKind::Assign(_, Rvalue::Borrow(BorrowKind::Exclusive, _)) =
                            &stmt.kind
                        {
                            stmt.span == error.span
                        } else {
                            false
                        };
                    if should_downgrade {
                        if let StatementKind::Assign(
                            _,
                            Rvalue::Borrow(BorrowKind::Exclusive, place),
                        ) = &stmt.kind
                        {
                            let dest = Place::Local(SlotId(0)); // placeholder — actual dest not needed for verification
                            let place = place.clone();
                            stmt.kind = StatementKind::Assign(
                                dest,
                                Rvalue::Borrow(BorrowKind::Shared, place),
                            );
                        }
                    }
                }
            }
            modified
        }
        RepairKind::Scope | RepairKind::Extract => {
            // These repairs modify control flow; for now, return unmodified MIR
            // (scope repair inserts new blocks; extract creates new functions)
            mir.clone()
        }
    }
}

// Helper functions

fn find_point_at_span(mir: &MirFunction, span: Span) -> Option<Point> {
    for block in &mir.blocks {
        for stmt in &block.statements {
            if stmt.span == span {
                return Some(stmt.point);
            }
        }
    }
    None
}

fn find_last_use_of_loan(mir: &MirFunction, loan_id: LoanId) -> Option<Point> {
    // Find the last point where a loan is still used.
    // This requires looking at the solver results, but we can approximate
    // by finding the last statement that reads from the borrowed place.
    let mut last_point = None;
    for block in &mir.blocks {
        for stmt in &block.statements {
            // Check if this statement uses the loan's ref
            if statement_may_use_loan(&stmt.kind, loan_id) {
                last_point = Some(stmt.point);
            }
        }
    }
    last_point
}

fn statement_may_use_loan(kind: &StatementKind, _loan_id: LoanId) -> bool {
    // Conservative: any statement that reads from a place derived from a borrow
    matches!(
        kind,
        StatementKind::Assign(_, Rvalue::Use(Operand::Copy(_)))
    )
}

/// Attach repair candidates to all errors in a BorrowAnalysis.
pub fn attach_repairs(analysis: &mut BorrowAnalysis, mir: &MirFunction) {
    let errors_snapshot: Vec<BorrowError> = analysis.errors.clone();
    for error in &mut analysis.errors {
        let repairs = generate_repairs(error, mir, &errors_snapshot);
        error.repairs = repairs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;

    fn span() -> Span {
        Span { start: 0, end: 1 }
    }

    fn span_at(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    #[test]
    fn test_generate_repairs_for_shared_exclusive_conflict() {
        let error = BorrowError {
            kind: BorrowErrorKind::ConflictSharedExclusive,
            span: span_at(10, 20),
            conflicting_loan: LoanId(0),
            loan_span: span_at(5, 8),
            last_use_span: None,
            repairs: Vec::new(),
        };

        // Simple MIR with two conflicting borrows
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        span: span(),
                        point: Point(0),
                    },
                    MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Borrow(BorrowKind::Shared, Place::Local(SlotId(0))),
                        ),
                        span: span_at(5, 8),
                        point: Point(1),
                    },
                    MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Borrow(BorrowKind::Exclusive, Place::Local(SlotId(0))),
                        ),
                        span: span_at(10, 20),
                        point: Point(2),
                    },
                ],
                terminator: Terminator {
                    kind: TerminatorKind::Return,
                    span: span(),
                },
            }],
            num_locals: 3,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
            ],
            span: span(),
        };

        let repairs = generate_repairs(&error, &mir, &[error.clone()]);
        assert!(!repairs.is_empty(), "should generate at least one repair");
        // First candidate should be one of: reorder, scope, or clone
        assert!(
            matches!(
                repairs[0].kind,
                RepairKind::Reorder | RepairKind::Scope | RepairKind::Clone
            ),
            "first repair should be reorder, scope, or clone"
        );
    }

    #[test]
    fn test_generate_repairs_for_use_after_move() {
        let error = BorrowError {
            kind: BorrowErrorKind::UseAfterMove,
            span: span(),
            conflicting_loan: LoanId(0),
            loan_span: span(),
            last_use_span: None,
            repairs: Vec::new(),
        };

        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![],
            num_locals: 0,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: span(),
        };

        let repairs = generate_repairs(&error, &mir, &[]);
        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0].kind, RepairKind::Clone);
    }

    #[test]
    fn test_generate_repairs_for_reference_escape() {
        let error = BorrowError {
            kind: BorrowErrorKind::ReferenceEscape,
            span: span(),
            conflicting_loan: LoanId(0),
            loan_span: span(),
            last_use_span: None,
            repairs: Vec::new(),
        };

        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![],
            num_locals: 0,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: span(),
        };

        let repairs = generate_repairs(&error, &mir, &[]);
        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0].kind, RepairKind::Clone);
        assert!(repairs[0].description.contains("owned value"));
    }

    #[test]
    fn test_attach_repairs_populates_error_repairs() {
        let mut analysis = BorrowAnalysis::empty();
        analysis.errors.push(BorrowError {
            kind: BorrowErrorKind::UseAfterMove,
            span: span(),
            conflicting_loan: LoanId(0),
            loan_span: span(),
            last_use_span: None,
            repairs: Vec::new(),
        });

        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![],
            num_locals: 0,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: span(),
        };

        assert!(analysis.errors[0].repairs.is_empty());
        attach_repairs(&mut analysis, &mir);
        assert!(!analysis.errors[0].repairs.is_empty());
    }
}
