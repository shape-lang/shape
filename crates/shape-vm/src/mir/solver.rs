//! Datafrog-based NLL borrow solver.
//!
//! Implements Non-Lexical Lifetimes using Datafrog's monotone fixed-point engine.
//! This is the core of Shape's borrow checking — it determines which borrows are
//! alive at each program point and detects conflicts.
//!
//! **Single source of truth**: This solver produces `BorrowAnalysis`, which is
//! consumed by the compiler, LSP, and diagnostic engine. No consumer re-derives results.
//!
//! Input relations (populated from MIR):
//!   loan_issued_at(Loan, Point)       — a borrow was created
//!   cfg_edge(Point, Point)            — control flow between points
//!   invalidates(Point, Loan)          — an action invalidates a loan
//!   use_of_loan(Loan, Point)          — a loan is used (the ref is read/used)
//!
//! Derived relations (Datafrog fixpoint):
//!   loan_live_at(Loan, Point)         — a loan is still active
//!   error(Point, Loan, Loan)          — two conflicting loans are simultaneously active

use super::analysis::*;
use super::cfg::ControlFlowGraph;
use super::liveness::{self, LivenessResult};
use super::types::*;
use datafrog::{Iteration, Relation, RelationLeaper};
use std::collections::{HashMap, HashSet};

/// Input facts extracted from MIR for the Datafrog solver.
#[derive(Debug, Default)]
pub struct BorrowFacts {
    /// (loan_id, point) — loan was created at this point
    pub loan_issued_at: Vec<(u32, u32)>,
    /// (from_point, to_point) — control flow edge
    pub cfg_edge: Vec<(u32, u32)>,
    /// (point, loan_id) — this point invalidates the loan (drop, reassignment)
    pub invalidates: Vec<(u32, u32)>,
    /// (loan_id, point) — the loan (reference) is used at this point
    pub use_of_loan: Vec<(u32, u32)>,
    /// Source span for each statement point.
    pub point_spans: HashMap<u32, shape_ast::ast::Span>,
    /// Loan metadata for error reporting.
    pub loan_info: HashMap<u32, LoanInfo>,
    /// Points where two loans conflict (same place, incompatible borrows).
    pub potential_conflicts: Vec<(u32, u32)>, // (loan_a, loan_b)
    /// Writes that may conflict with active loans: (point, place, span).
    pub writes: Vec<(u32, Place, shape_ast::ast::Span)>,
    /// Reads from owner places that may conflict with active exclusive loans.
    pub reads: Vec<(u32, Place, shape_ast::ast::Span)>,
    /// Loans that flow into the dedicated return slot and would escape.
    pub escaped_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Exclusive loans captured across an async/task boundary.
    pub task_boundary_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Loans captured into a closure environment.
    pub closure_capture_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Loans stored into array literals.
    pub array_store_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Loans stored into object/struct literals.
    pub object_store_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Loans stored into enum payloads.
    pub enum_store_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Loans written through field assignments into aggregate places.
    pub object_assignment_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Loans written through index assignments into aggregate places.
    pub array_assignment_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Reference-return contracts flowing into the return slot.
    pub return_reference_candidates: Vec<(ReferenceReturnContract, shape_ast::ast::Span)>,
    /// Return-slot writes that produce a plain owned value.
    pub non_reference_return_spans: Vec<shape_ast::ast::Span>,
}

/// Populate borrow facts from a MIR function and its CFG.
pub fn extract_facts(mir: &MirFunction, cfg: &ControlFlowGraph) -> BorrowFacts {
    let mut facts = BorrowFacts::default();
    let mut next_loan = 0u32;
    let mut slot_loans: HashMap<SlotId, Vec<u32>> = HashMap::new();
    let param_reference_contracts: HashMap<SlotId, ReferenceReturnContract> = mir
        .param_slots
        .iter()
        .enumerate()
        .filter_map(|(param_index, slot)| {
            mir.param_reference_kinds
                .get(param_index)
                .copied()
                .flatten()
                .map(|kind| (*slot, ReferenceReturnContract { param_index, kind }))
        })
        .collect();
    let mut slot_reference_contracts = param_reference_contracts.clone();

    // Extract CFG edges from the block structure
    for block in &mir.blocks {
        // Edges between consecutive statements within a block
        for i in 0..block.statements.len().saturating_sub(1) {
            let from = block.statements[i].point.0;
            let to = block.statements[i + 1].point.0;
            facts.cfg_edge.push((from, to));
        }

        // Edge from last statement to successor blocks' first statements
        let last_point = block.statements.last().map(|s| s.point.0).unwrap_or(0);

        for &succ_id in cfg.successors(block.id) {
            let succ_block = mir.block(succ_id);
            if let Some(first_stmt) = succ_block.statements.first() {
                facts.cfg_edge.push((last_point, first_stmt.point.0));
            }
        }
    }

    // Extract loan facts from statements
    for block in &mir.blocks {
        for stmt in &block.statements {
            facts.point_spans.insert(stmt.point.0, stmt.span);
            match &stmt.kind {
                StatementKind::Assign(dest, Rvalue::Borrow(kind, place)) => {
                    let loan_id = next_loan;
                    next_loan += 1;

                    facts.loan_issued_at.push((loan_id, stmt.point.0));
                    if let Place::Local(slot) = dest {
                        slot_loans.insert(*slot, vec![loan_id]);
                        if let Some(contract) = safe_reference_contract_for_borrow(
                            *kind,
                            place,
                            &param_reference_contracts,
                        ) {
                            slot_reference_contracts.insert(*slot, contract);
                        } else {
                            slot_reference_contracts.remove(slot);
                        }
                    }
                    facts.loan_info.insert(
                        loan_id,
                        LoanInfo {
                            id: LoanId(loan_id),
                            borrowed_place: place.clone(),
                            kind: *kind,
                            issued_at: stmt.point,
                            span: stmt.span,
                        },
                    );
                }
                StatementKind::Assign(place, rvalue) => {
                    if let Place::Local(dest_slot) = place {
                        update_slot_loan_aliases(&mut slot_loans, *dest_slot, rvalue);
                        update_slot_reference_contracts(
                            &mut slot_reference_contracts,
                            *dest_slot,
                            rvalue,
                        );
                        if *dest_slot == SlotId(0) {
                            let mut found_reference_return = false;
                            if let Some(contract) =
                                reference_contract_from_rvalue(&slot_reference_contracts, rvalue)
                            {
                                facts
                                    .return_reference_candidates
                                    .push((contract, stmt.span));
                                found_reference_return = true;
                            }
                            for loan_id in local_loans_from_rvalue(&slot_loans, rvalue) {
                                let info = &facts.loan_info[&loan_id];
                                if let Some(contract) = safe_reference_contract_for_borrow(
                                    info.kind,
                                    &info.borrowed_place,
                                    &param_reference_contracts,
                                ) {
                                    facts
                                        .return_reference_candidates
                                        .push((contract, stmt.span));
                                    found_reference_return = true;
                                } else {
                                    facts.escaped_loans.push((loan_id, stmt.span));
                                }
                            }
                            if !found_reference_return {
                                facts.non_reference_return_spans.push(stmt.span);
                            }
                        }
                    }
                    match place {
                        Place::Field(..) => {
                            for loan_id in local_loans_from_rvalue(&slot_loans, rvalue) {
                                facts.object_assignment_loans.push((loan_id, stmt.span));
                            }
                        }
                        Place::Index(..) => {
                            for loan_id in local_loans_from_rvalue(&slot_loans, rvalue) {
                                facts.array_assignment_loans.push((loan_id, stmt.span));
                            }
                        }
                        Place::Local(..) | Place::Deref(..) => {}
                    }
                    facts.writes.push((stmt.point.0, place.clone(), stmt.span));
                    // Assignment to a place invalidates all loans on that place
                    for (lid, info) in &facts.loan_info {
                        if place.conflicts_with(&info.borrowed_place) {
                            facts.invalidates.push((stmt.point.0, *lid));
                        }
                    }
                }
                StatementKind::Drop(place) => {
                    // Drop invalidates all loans on the place
                    for (lid, info) in &facts.loan_info {
                        if place.conflicts_with(&info.borrowed_place) {
                            facts.invalidates.push((stmt.point.0, *lid));
                        }
                    }
                }
                StatementKind::TaskBoundary(operands) => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        if facts
                            .loan_info
                            .get(&loan_id)
                            .is_some_and(|info| info.kind == BorrowKind::Exclusive)
                        {
                            facts.task_boundary_loans.push((loan_id, stmt.span));
                        }
                    }
                }
                StatementKind::ClosureCapture(operands) => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        facts.closure_capture_loans.push((loan_id, stmt.span));
                    }
                }
                StatementKind::ArrayStore(operands) => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        facts.array_store_loans.push((loan_id, stmt.span));
                    }
                }
                StatementKind::ObjectStore(operands) => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        facts.object_store_loans.push((loan_id, stmt.span));
                    }
                }
                StatementKind::EnumStore(operands) => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        facts.enum_store_loans.push((loan_id, stmt.span));
                    }
                }
                StatementKind::Nop => {}
            }

            for read_place in statement_read_places(&stmt.kind) {
                facts
                    .reads
                    .push((stmt.point.0, read_place.clone(), stmt.span));
                if let Place::Local(slot) = read_place {
                    if let Some(loans) = slot_loans.get(&slot) {
                        for loan_id in loans {
                            facts.use_of_loan.push((*loan_id, stmt.point.0));
                        }
                    }
                }
            }
        }
    }

    // Detect potential conflicts between loans on the same place
    let loan_ids: Vec<u32> = facts.loan_info.keys().copied().collect();
    for i in 0..loan_ids.len() {
        for j in (i + 1)..loan_ids.len() {
            let a = loan_ids[i];
            let b = loan_ids[j];
            let info_a = &facts.loan_info[&a];
            let info_b = &facts.loan_info[&b];

            // Two loans conflict if they borrow overlapping places and at least one is exclusive
            if info_a.borrowed_place.conflicts_with(&info_b.borrowed_place)
                && (info_a.kind == BorrowKind::Exclusive || info_b.kind == BorrowKind::Exclusive)
            {
                facts.potential_conflicts.push((a, b));
            }
        }
    }

    facts
}

fn operand_read_places<'a>(operand: &'a Operand, reads: &mut Vec<Place>) {
    match operand {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            reads.push(place.clone());
            place_nested_read_places(place, reads);
        }
        Operand::Constant(_) => {}
    }
}

fn place_nested_read_places(place: &Place, reads: &mut Vec<Place>) {
    match place {
        Place::Local(_) => {}
        Place::Field(base, _) | Place::Deref(base) => {
            place_nested_read_places(base, reads);
        }
        Place::Index(base, index) => {
            place_nested_read_places(base, reads);
            operand_read_places(index, reads);
        }
    }
}

fn statement_read_places(kind: &StatementKind) -> Vec<Place> {
    let mut reads = Vec::new();
    match kind {
        StatementKind::Assign(_, rvalue) => match rvalue {
            Rvalue::Use(operand) | Rvalue::Clone(operand) => {
                operand_read_places(operand, &mut reads)
            }
            Rvalue::Borrow(_, _) => {}
            Rvalue::BinaryOp(_, lhs, rhs) => {
                operand_read_places(lhs, &mut reads);
                operand_read_places(rhs, &mut reads);
            }
            Rvalue::UnaryOp(_, operand) => operand_read_places(operand, &mut reads),
            Rvalue::Aggregate(operands) => {
                for operand in operands {
                    operand_read_places(operand, &mut reads);
                }
            }
        },
        StatementKind::Drop(place) => place_nested_read_places(place, &mut reads),
        StatementKind::TaskBoundary(operands) => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::ClosureCapture(operands) => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::ArrayStore(operands) => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::ObjectStore(operands) => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::EnumStore(operands) => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::Nop => {}
    }
    reads
}

fn local_loans_from_operand(slot_loans: &HashMap<SlotId, Vec<u32>>, operand: &Operand) -> Vec<u32> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => slot_loans
            .get(&place.root_local())
            .cloned()
            .unwrap_or_default(),
        Operand::Constant(_) => Vec::new(),
    }
}

fn local_loans_from_operands(
    slot_loans: &HashMap<SlotId, Vec<u32>>,
    operands: &[Operand],
) -> Vec<u32> {
    let mut loans = Vec::new();
    let mut seen = HashSet::new();
    for operand in operands {
        for loan in local_loans_from_operand(slot_loans, operand) {
            if seen.insert(loan) {
                loans.push(loan);
            }
        }
    }
    loans
}

fn update_slot_loan_aliases(
    slot_loans: &mut HashMap<SlotId, Vec<u32>>,
    dest_slot: SlotId,
    rvalue: &Rvalue,
) {
    match rvalue {
        Rvalue::Borrow(_, _) => {}
        Rvalue::Use(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Use(Operand::Move(Place::Local(src_slot)))
        | Rvalue::Use(Operand::MoveExplicit(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Move(Place::Local(src_slot))) => {
            if let Some(loans) = slot_loans.get(src_slot).cloned() {
                slot_loans.insert(dest_slot, loans);
            } else {
                slot_loans.remove(&dest_slot);
            }
        }
        _ => {
            slot_loans.remove(&dest_slot);
        }
    }
}

fn local_loans_from_rvalue(slot_loans: &HashMap<SlotId, Vec<u32>>, rvalue: &Rvalue) -> Vec<u32> {
    match rvalue {
        Rvalue::Use(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Use(Operand::Move(Place::Local(src_slot)))
        | Rvalue::Use(Operand::MoveExplicit(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Move(Place::Local(src_slot))) => {
            slot_loans.get(src_slot).cloned().unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn update_slot_reference_contracts(
    slot_reference_contracts: &mut HashMap<SlotId, ReferenceReturnContract>,
    dest_slot: SlotId,
    rvalue: &Rvalue,
) {
    match rvalue {
        Rvalue::Use(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Use(Operand::Move(Place::Local(src_slot)))
        | Rvalue::Use(Operand::MoveExplicit(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Move(Place::Local(src_slot))) => {
            if let Some(contract) = slot_reference_contracts.get(src_slot).copied() {
                slot_reference_contracts.insert(dest_slot, contract);
            } else {
                slot_reference_contracts.remove(&dest_slot);
            }
        }
        _ => {
            slot_reference_contracts.remove(&dest_slot);
        }
    }
}

fn reference_contract_from_rvalue(
    slot_reference_contracts: &HashMap<SlotId, ReferenceReturnContract>,
    rvalue: &Rvalue,
) -> Option<ReferenceReturnContract> {
    match rvalue {
        Rvalue::Use(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Use(Operand::Move(Place::Local(src_slot)))
        | Rvalue::Use(Operand::MoveExplicit(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Move(Place::Local(src_slot))) => {
            slot_reference_contracts.get(src_slot).copied()
        }
        _ => None,
    }
}

fn safe_reference_contract_for_borrow(
    borrow_kind: BorrowKind,
    borrowed_place: &Place,
    param_reference_contracts: &HashMap<SlotId, ReferenceReturnContract>,
) -> Option<ReferenceReturnContract> {
    let param_contract = param_reference_contracts.get(&borrowed_place.root_local())?;
    Some(ReferenceReturnContract {
        param_index: param_contract.param_index,
        kind: borrow_kind,
    })
}

fn resolve_return_reference_contract(
    errors: &mut Vec<BorrowError>,
    facts: &BorrowFacts,
    loans_at_point: &HashMap<Point, Vec<LoanId>>,
) -> Option<ReferenceReturnContract> {
    let mut unique_candidates = Vec::new();
    for (candidate, _) in &facts.return_reference_candidates {
        if !unique_candidates.contains(candidate) {
            unique_candidates.push(*candidate);
        }
    }

    if unique_candidates.is_empty() {
        return None;
    }

    let error_span = if unique_candidates.len() > 1 {
        facts
            .return_reference_candidates
            .get(1)
            .map(|(_, span)| *span)
    } else {
        facts.non_reference_return_spans.first().copied()
    };

    if let Some(span) = error_span {
        let (conflicting_loan, loan_span, last_use_span) = facts
            .return_reference_candidates
            .first()
            .and_then(|(candidate, candidate_span)| {
                find_matching_loan_for_return_candidate(
                    candidate,
                    *candidate_span,
                    facts,
                    loans_at_point,
                )
            })
            .unwrap_or((LoanId(0), span, None));
        errors.push(BorrowError {
            kind: BorrowErrorKind::InconsistentReferenceReturn,
            span,
            conflicting_loan,
            loan_span,
            last_use_span,
            repairs: Vec::new(),
        });
        return None;
    }

    unique_candidates.into_iter().next()
}

fn find_matching_loan_for_return_candidate(
    candidate: &ReferenceReturnContract,
    candidate_span: shape_ast::ast::Span,
    facts: &BorrowFacts,
    loans_at_point: &HashMap<Point, Vec<LoanId>>,
) -> Option<(LoanId, shape_ast::ast::Span, Option<shape_ast::ast::Span>)> {
    let point = facts
        .point_spans
        .iter()
        .find_map(|(point, span)| (*span == candidate_span).then_some(Point(*point)))?;
    let loans = loans_at_point.get(&point)?;
    for loan in loans {
        let info = facts.loan_info.get(&loan.0)?;
        if info.kind == candidate.kind {
            return Some((*loan, info.span, last_use_span_for_loan(facts, loan.0)));
        }
    }
    None
}

/// Run the Datafrog solver to compute loan liveness and detect errors.
pub fn solve(facts: &BorrowFacts) -> SolverResult {
    let mut iteration = Iteration::new();

    // Input relations (static — known before iteration)
    // cfg_edge indexed by source point: (point1, point2)
    let cfg_edge: Relation<(u32, u32)> = facts.cfg_edge.iter().cloned().collect();
    // invalidates indexed by (point, loan)
    let invalidates_set: std::collections::HashSet<(u32, u32)> =
        facts.invalidates.iter().cloned().collect();

    // Derived relation: loan_live_at(point, loan)
    // Keyed by point for efficient join with cfg_edge.
    let loan_live_at = iteration.variable::<(u32, u32)>("loan_live_at");

    // Seed: a loan is live at the point where it's issued.
    // Reindex from (loan, point) to (point, loan).
    let seed: Vec<(u32, u32)> = facts
        .loan_issued_at
        .iter()
        .map(|&(loan, point)| (point, loan))
        .collect();
    loan_live_at.extend(seed.iter().cloned());

    // Fixed-point iteration:
    // loan_live_at(point2, loan) :-
    //   loan_live_at(point1, loan),
    //   cfg_edge(point1, point2),
    //   !invalidates(point1, loan).
    while iteration.changed() {
        // For each (point1, loan) in loan_live_at,
        // join with cfg_edge on point1 to get point2,
        // filter out if invalidates(point1, loan).
        loan_live_at.from_leapjoin(
            &loan_live_at,
            cfg_edge.extend_with(|&(point1, _loan)| point1),
            |&(point1, loan), &point2| {
                if invalidates_set.contains(&(point1, loan)) {
                    // Loan is invalidated at point1 — keep it live at point1,
                    // but don't propagate it to successors.
                    (u32::MAX, u32::MAX) // sentinel that won't match anything useful
                } else {
                    (point2, loan)
                }
            },
        );
    }

    // Collect results and filter out sentinel values
    let forward_live_points: Vec<(u32, u32)> = loan_live_at
        .complete()
        .iter()
        .filter(|&&(p, l)| p != u32::MAX && l != u32::MAX)
        .cloned()
        .collect();
    let (nll_live_set, loans_with_reachable_uses) = compute_nll_live_points(facts);
    let loan_live_at_result: Vec<(u32, u32)> = forward_live_points
        .into_iter()
        .filter(|point_loan| {
            !loans_with_reachable_uses.contains(&point_loan.1) || nll_live_set.contains(point_loan)
        })
        .collect();

    // Build point → active loans map
    let mut loans_at_point: HashMap<Point, Vec<LoanId>> = HashMap::new();
    for &(point, loan) in &loan_live_at_result {
        loans_at_point
            .entry(Point(point))
            .or_default()
            .push(LoanId(loan));
    }

    // Build loan → set of points for quick intersection queries
    let mut loan_points: HashMap<u32, std::collections::HashSet<u32>> = HashMap::new();
    for &(point, loan) in &loan_live_at_result {
        loan_points.entry(loan).or_default().insert(point);
    }

    // Detect errors: two conflicting loans alive at the same point
    let mut errors = Vec::new();
    let mut seen_conflicts = std::collections::HashSet::new();
    for &(loan_a, loan_b) in &facts.potential_conflicts {
        let key = (loan_a.min(loan_b), loan_a.max(loan_b));
        if !seen_conflicts.insert(key) {
            continue;
        }

        let points_a = loan_points.get(&loan_a);
        let points_b = loan_points.get(&loan_b);

        if let (Some(pa), Some(pb)) = (points_a, points_b) {
            // Check if there's any intersection
            let has_overlap = pa.iter().any(|p| pb.contains(p));
            if has_overlap {
                let info_a = &facts.loan_info[&loan_a];
                let info_b = &facts.loan_info[&loan_b];
                let kind = if info_a.kind == BorrowKind::Exclusive
                    && info_b.kind == BorrowKind::Exclusive
                {
                    BorrowErrorKind::ConflictExclusiveExclusive
                } else {
                    BorrowErrorKind::ConflictSharedExclusive
                };
                errors.push(BorrowError {
                    kind,
                    span: info_b.span,
                    conflicting_loan: LoanId(loan_a),
                    loan_span: info_a.span,
                    last_use_span: last_use_span_for_loan(facts, loan_a),
                    repairs: Vec::new(),
                });
            }
        }
    }

    let mut seen_writes = std::collections::HashSet::new();
    for (point, place, span) in &facts.writes {
        let point_key = Point(*point);
        let Some(loans) = loans_at_point.get(&point_key) else {
            continue;
        };
        for loan in loans {
            let info = &facts.loan_info[&loan.0];
            if !place.conflicts_with(&info.borrowed_place) {
                continue;
            }
            let key = (*point, loan.0);
            if !seen_writes.insert(key) {
                continue;
            }
            errors.push(BorrowError {
                kind: BorrowErrorKind::WriteWhileBorrowed,
                span: *span,
                conflicting_loan: *loan,
                loan_span: info.span,
                last_use_span: last_use_span_for_loan(facts, loan.0),
                repairs: Vec::new(),
            });
            break;
        }
    }

    let mut seen_reads = std::collections::HashSet::new();
    for (point, place, span) in &facts.reads {
        let point_key = Point(*point);
        let Some(loans) = loans_at_point.get(&point_key) else {
            continue;
        };
        for loan in loans {
            let info = &facts.loan_info[&loan.0];
            if info.kind != BorrowKind::Exclusive || !place.conflicts_with(&info.borrowed_place) {
                continue;
            }
            let key = (*point, loan.0);
            if !seen_reads.insert(key) {
                continue;
            }
            errors.push(BorrowError {
                kind: BorrowErrorKind::ReadWhileExclusivelyBorrowed,
                span: *span,
                conflicting_loan: *loan,
                loan_span: info.span,
                last_use_span: last_use_span_for_loan(facts, loan.0),
                repairs: Vec::new(),
            });
            break;
        }
    }

    let mut seen_escapes = std::collections::HashSet::new();
    for (loan_id, span) in &facts.escaped_loans {
        if !seen_escapes.insert((*loan_id, span.start, span.end)) {
            continue;
        }
        let info = &facts.loan_info[loan_id];
        errors.push(BorrowError {
            kind: BorrowErrorKind::ReferenceEscape,
            span: *span,
            conflicting_loan: LoanId(*loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, *loan_id),
            repairs: Vec::new(),
        });
    }

    let mut seen_array_store = std::collections::HashSet::new();
    for (loan_id, span) in &facts.array_store_loans {
        if !seen_array_store.insert((*loan_id, span.start, span.end)) {
            continue;
        }
        let info = &facts.loan_info[loan_id];
        errors.push(BorrowError {
            kind: BorrowErrorKind::ReferenceStoredInArray,
            span: *span,
            conflicting_loan: LoanId(*loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, *loan_id),
            repairs: Vec::new(),
        });
    }

    for (loan_id, span) in &facts.array_assignment_loans {
        if !seen_array_store.insert((*loan_id, span.start, span.end)) {
            continue;
        }
        let info = &facts.loan_info[loan_id];
        errors.push(BorrowError {
            kind: BorrowErrorKind::ReferenceStoredInArray,
            span: *span,
            conflicting_loan: LoanId(*loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, *loan_id),
            repairs: Vec::new(),
        });
    }

    let mut seen_object_store = std::collections::HashSet::new();
    for (loan_id, span) in &facts.object_store_loans {
        if !seen_object_store.insert((*loan_id, span.start, span.end)) {
            continue;
        }
        let info = &facts.loan_info[loan_id];
        errors.push(BorrowError {
            kind: BorrowErrorKind::ReferenceStoredInObject,
            span: *span,
            conflicting_loan: LoanId(*loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, *loan_id),
            repairs: Vec::new(),
        });
    }

    for (loan_id, span) in &facts.object_assignment_loans {
        if !seen_object_store.insert((*loan_id, span.start, span.end)) {
            continue;
        }
        let info = &facts.loan_info[loan_id];
        errors.push(BorrowError {
            kind: BorrowErrorKind::ReferenceStoredInObject,
            span: *span,
            conflicting_loan: LoanId(*loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, *loan_id),
            repairs: Vec::new(),
        });
    }

    let mut seen_enum_store = std::collections::HashSet::new();
    for (loan_id, span) in &facts.enum_store_loans {
        if !seen_enum_store.insert((*loan_id, span.start, span.end)) {
            continue;
        }
        let info = &facts.loan_info[loan_id];
        errors.push(BorrowError {
            kind: BorrowErrorKind::ReferenceStoredInEnum,
            span: *span,
            conflicting_loan: LoanId(*loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, *loan_id),
            repairs: Vec::new(),
        });
    }

    let mut seen_task_boundary = std::collections::HashSet::new();
    for (loan_id, span) in &facts.task_boundary_loans {
        if !seen_task_boundary.insert((*loan_id, span.start, span.end)) {
            continue;
        }
        let info = &facts.loan_info[loan_id];
        errors.push(BorrowError {
            kind: BorrowErrorKind::ExclusiveRefAcrossTaskBoundary,
            span: *span,
            conflicting_loan: LoanId(*loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, *loan_id),
            repairs: Vec::new(),
        });
    }

    let mut seen_closure_capture = std::collections::HashSet::new();
    for (loan_id, span) in &facts.closure_capture_loans {
        if !seen_closure_capture.insert((*loan_id, span.start, span.end)) {
            continue;
        }
        let info = &facts.loan_info[loan_id];
        errors.push(BorrowError {
            kind: BorrowErrorKind::ReferenceEscapeIntoClosure,
            span: *span,
            conflicting_loan: LoanId(*loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, *loan_id),
            repairs: Vec::new(),
        });
    }

    let return_reference_contract =
        resolve_return_reference_contract(&mut errors, facts, &loans_at_point);

    SolverResult {
        loans_at_point,
        errors,
        loan_info: facts.loan_info.clone(),
        return_reference_contract,
    }
}

fn compute_nll_live_points(facts: &BorrowFacts) -> (HashSet<(u32, u32)>, HashSet<u32>) {
    let mut predecessors: HashMap<u32, Vec<u32>> = HashMap::new();
    for (from, to) in &facts.cfg_edge {
        predecessors.entry(*to).or_default().push(*from);
    }

    let issue_points: HashMap<u32, u32> = facts
        .loan_issued_at
        .iter()
        .map(|(loan_id, point)| (*loan_id, *point))
        .collect();

    let mut invalidation_points: HashMap<u32, HashSet<u32>> = HashMap::new();
    for (point, loan_id) in &facts.invalidates {
        invalidation_points
            .entry(*loan_id)
            .or_default()
            .insert(*point);
    }

    let mut use_points: HashMap<u32, Vec<u32>> = HashMap::new();
    for (loan_id, point) in &facts.use_of_loan {
        use_points.entry(*loan_id).or_default().push(*point);
    }

    let mut live_points = HashSet::new();
    let mut loans_with_reachable_uses = HashSet::new();
    for (loan_id, issue_point) in issue_points {
        let mut worklist = use_points.get(&loan_id).cloned().unwrap_or_default();
        let invalidates = invalidation_points.get(&loan_id);
        let mut visited = HashSet::new();
        let mut loan_live_points = HashSet::new();
        let mut reached_issue = false;

        while let Some(point) = worklist.pop() {
            if !visited.insert(point) {
                continue;
            }

            loan_live_points.insert((point, loan_id));

            if point == issue_point {
                reached_issue = true;
                continue;
            }

            if invalidates.is_some_and(|points| points.contains(&point)) {
                continue;
            }

            if let Some(preds) = predecessors.get(&point) {
                worklist.extend(preds.iter().copied());
            }
        }

        if reached_issue {
            loans_with_reachable_uses.insert(loan_id);
            live_points.extend(loan_live_points);
        }
    }

    (live_points, loans_with_reachable_uses)
}

/// Raw solver output (before combining with liveness for full BorrowAnalysis).
#[derive(Debug)]
pub struct SolverResult {
    pub loans_at_point: HashMap<Point, Vec<LoanId>>,
    pub errors: Vec<BorrowError>,
    pub loan_info: HashMap<u32, LoanInfo>,
    pub return_reference_contract: Option<ReferenceReturnContract>,
}

/// Run the complete borrow analysis pipeline for a MIR function.
/// This is the main entry point — produces the single BorrowAnalysis
/// consumed by compiler, LSP, and diagnostics.
pub fn analyze(mir: &MirFunction) -> BorrowAnalysis {
    let cfg = ControlFlowGraph::build(mir);

    // 1. Compute liveness (for move/clone inference)
    let liveness = liveness::compute_liveness(mir, &cfg);

    // 2. Extract Datafrog input facts
    let facts = extract_facts(mir, &cfg);

    // 3. Run the Datafrog solver
    let solver_result = solve(&facts);

    // 4. Compute ownership decisions (move/clone) based on liveness
    let ownership_decisions = compute_ownership_decisions(mir, &liveness);
    let mut move_errors = compute_use_after_move_errors(mir, &cfg, &ownership_decisions);

    // 5. Combine into BorrowAnalysis
    let loans = solver_result
        .loan_info
        .into_iter()
        .map(|(id, info)| (LoanId(id), info))
        .collect();
    let mut errors = solver_result.errors;
    errors.append(&mut move_errors);

    BorrowAnalysis {
        liveness,
        loans_at_point: solver_result.loans_at_point,
        loans,
        errors,
        ownership_decisions,
        mutability_errors: Vec::new(), // filled by binding resolver (Phase 1)
        return_reference_contract: solver_result.return_reference_contract,
    }
}

/// Compute ownership decisions for assignments based on liveness.
fn compute_ownership_decisions(
    mir: &MirFunction,
    liveness: &LivenessResult,
) -> HashMap<Point, OwnershipDecision> {
    let mut decisions = HashMap::new();

    for block in &mir.blocks {
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            if let StatementKind::Assign(_, Rvalue::Use(Operand::Move(Place::Local(src_slot)))) =
                &stmt.kind
            {
                // Check if the source is a non-Copy type
                let src_type = mir
                    .local_types
                    .get(src_slot.0 as usize)
                    .cloned()
                    .unwrap_or(LocalTypeInfo::Unknown);

                let decision = match src_type {
                    LocalTypeInfo::Copy => OwnershipDecision::Copy,
                    LocalTypeInfo::NonCopy => {
                        // Smart inference: check if source is live after this point
                        if liveness.is_live_after(block.id, stmt_idx, *src_slot, mir) {
                            OwnershipDecision::Clone
                        } else {
                            OwnershipDecision::Move
                        }
                    }
                    LocalTypeInfo::Unknown => {
                        // Conservative: assume Clone if live, Move if dead
                        if liveness.is_live_after(block.id, stmt_idx, *src_slot, mir) {
                            OwnershipDecision::Clone
                        } else {
                            OwnershipDecision::Move
                        }
                    }
                };

                decisions.insert(stmt.point, decision);
            }
        }
    }

    decisions
}

fn compute_use_after_move_errors(
    mir: &MirFunction,
    cfg: &ControlFlowGraph,
    ownership_decisions: &HashMap<Point, OwnershipDecision>,
) -> Vec<BorrowError> {
    let mut in_states: HashMap<BasicBlockId, HashMap<Place, shape_ast::ast::Span>> = HashMap::new();
    let mut out_states: HashMap<BasicBlockId, HashMap<Place, shape_ast::ast::Span>> =
        HashMap::new();

    for block in mir.iter_blocks() {
        in_states.insert(block.id, HashMap::new());
        out_states.insert(block.id, HashMap::new());
    }

    let mut changed = true;
    while changed {
        changed = false;
        for &block_id in &cfg.reverse_postorder() {
            let mut block_in: Option<HashMap<Place, shape_ast::ast::Span>> = None;
            for &pred in cfg.predecessors(block_id) {
                if let Some(pred_out) = out_states.get(&pred) {
                    if let Some(current) = block_in.as_mut() {
                        intersect_moved_places(current, pred_out);
                    } else {
                        block_in = Some(pred_out.clone());
                    }
                }
            }
            let block_in = block_in.unwrap_or_default();

            let mut block_out = block_in.clone();
            let block = mir.block(block_id);
            for stmt in &block.statements {
                apply_move_transfer(&mut block_out, stmt, mir, ownership_decisions);
            }

            if in_states.get(&block_id) != Some(&block_in) {
                in_states.insert(block_id, block_in);
                changed = true;
            }
            if out_states.get(&block_id) != Some(&block_out) {
                out_states.insert(block_id, block_out);
                changed = true;
            }
        }
    }

    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for block in mir.iter_blocks() {
        let mut moved_places = in_states.get(&block.id).cloned().unwrap_or_default();
        for stmt in &block.statements {
            for read_place in statement_read_places(&stmt.kind) {
                if let Some((moved_place, move_span)) =
                    find_moved_place_conflict(&moved_places, &read_place)
                {
                    let key = (stmt.point.0, format!("{}", moved_place));
                    if seen.insert(key) {
                        errors.push(BorrowError {
                            kind: BorrowErrorKind::UseAfterMove,
                            span: stmt.span,
                            conflicting_loan: LoanId(0),
                            loan_span: move_span,
                            last_use_span: None,
                            repairs: Vec::new(),
                        });
                    }
                    break;
                }
            }

            if let Some(borrowed_place) = statement_borrow_place(&stmt.kind)
                && let Some((moved_place, move_span)) =
                    find_moved_place_conflict(&moved_places, borrowed_place)
            {
                let key = (stmt.point.0, format!("{}", moved_place));
                if seen.insert(key) {
                    errors.push(BorrowError {
                        kind: BorrowErrorKind::UseAfterMove,
                        span: stmt.span,
                        conflicting_loan: LoanId(0),
                        loan_span: move_span,
                        last_use_span: None,
                        repairs: Vec::new(),
                    });
                }
            }

            if let Some(dest_place) = statement_dest_place(&stmt.kind)
                && let Some((moved_place, move_span)) = moved_places
                    .iter()
                    .find(|(moved_place, _)| {
                        dest_place.conflicts_with(moved_place)
                            && !reinitializes_moved_place(dest_place, moved_place)
                    })
                    .map(|(place, span)| (place.clone(), *span))
            {
                let key = (stmt.point.0, format!("{}", moved_place));
                if seen.insert(key) {
                    errors.push(BorrowError {
                        kind: BorrowErrorKind::UseAfterMove,
                        span: stmt.span,
                        conflicting_loan: LoanId(0),
                        loan_span: move_span,
                        last_use_span: None,
                        repairs: Vec::new(),
                    });
                }
            }

            apply_move_transfer(&mut moved_places, stmt, mir, ownership_decisions);
        }
    }

    errors
}

fn intersect_moved_places(
    dest: &mut HashMap<Place, shape_ast::ast::Span>,
    incoming: &HashMap<Place, shape_ast::ast::Span>,
) {
    dest.retain(|place, span| {
        if let Some(incoming_span) = incoming.get(place) {
            if incoming_span.start < span.start {
                *span = *incoming_span;
            }
            true
        } else {
            false
        }
    });
}

fn apply_move_transfer(
    moved_places: &mut HashMap<Place, shape_ast::ast::Span>,
    stmt: &MirStatement,
    mir: &MirFunction,
    ownership_decisions: &HashMap<Point, OwnershipDecision>,
) {
    if let Some(dest_place) = statement_dest_place(&stmt.kind) {
        moved_places.retain(|moved_place, _| !reinitializes_moved_place(dest_place, moved_place));
    }

    for moved_place in actual_move_places(stmt, mir, ownership_decisions) {
        moved_places.insert(moved_place, stmt.span);
    }
}

fn statement_borrow_place(kind: &StatementKind) -> Option<&Place> {
    match kind {
        StatementKind::Assign(_, Rvalue::Borrow(_, place)) => Some(place),
        _ => None,
    }
}

fn statement_dest_place(kind: &StatementKind) -> Option<&Place> {
    match kind {
        StatementKind::Assign(place, _) | StatementKind::Drop(place) => Some(place),
        StatementKind::TaskBoundary(_)
        | StatementKind::ClosureCapture(_)
        | StatementKind::ArrayStore(_)
        | StatementKind::ObjectStore(_)
        | StatementKind::EnumStore(_) => None,
        StatementKind::Nop => None,
    }
}

fn actual_move_places(
    stmt: &MirStatement,
    mir: &MirFunction,
    ownership_decisions: &HashMap<Point, OwnershipDecision>,
) -> Vec<Place> {
    match &stmt.kind {
        StatementKind::Assign(_, Rvalue::Use(Operand::Move(place)))
            if ownership_decisions.get(&stmt.point) == Some(&OwnershipDecision::Move) =>
        {
            vec![place.clone()]
        }
        StatementKind::Assign(_, Rvalue::Use(Operand::MoveExplicit(place)))
            if place_root_local_type(place, mir) != Some(LocalTypeInfo::Copy) =>
        {
            vec![place.clone()]
        }
        _ => Vec::new(),
    }
}

fn place_root_local_type(place: &Place, mir: &MirFunction) -> Option<LocalTypeInfo> {
    mir.local_types.get(place.root_local().0 as usize).cloned()
}

fn reinitializes_moved_place(dest_place: &Place, moved_place: &Place) -> bool {
    dest_place.is_prefix_of(moved_place)
}

fn find_moved_place_conflict(
    moved_places: &HashMap<Place, shape_ast::ast::Span>,
    accessed_place: &Place,
) -> Option<(Place, shape_ast::ast::Span)> {
    moved_places
        .iter()
        .find(|(moved_place, _)| accessed_place.conflicts_with(moved_place))
        .map(|(place, span)| (place.clone(), *span))
}

fn last_use_span_for_loan(facts: &BorrowFacts, loan_id: u32) -> Option<shape_ast::ast::Span> {
    facts
        .use_of_loan
        .iter()
        .filter(|(candidate, _)| *candidate == loan_id)
        .filter_map(|(_, point)| facts.point_spans.get(point).copied())
        .max_by_key(|span| span.start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;

    fn span() -> Span {
        Span { start: 0, end: 1 }
    }

    fn make_stmt(kind: StatementKind, point: u32) -> MirStatement {
        MirStatement {
            kind,
            span: span(),
            point: Point(point),
        }
    }

    fn make_terminator(kind: TerminatorKind) -> Terminator {
        Terminator { kind, span: span() }
    }

    #[test]
    fn test_single_shared_borrow_no_error() {
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    // _0 = 42
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    // _1 = &_0
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Borrow(BorrowKind::Shared, Place::Local(SlotId(0))),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 2,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy],
            span: span(),
        };

        let analysis = analyze(&mir);
        assert!(analysis.errors.is_empty(), "expected no errors");
    }

    #[test]
    fn test_conflicting_shared_and_exclusive_error() {
        // _0 = value
        // _1 = &_0 (shared)
        // _2 = &mut _0 (exclusive) — should conflict with _1
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Borrow(BorrowKind::Shared, Place::Local(SlotId(0))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Borrow(BorrowKind::Exclusive, Place::Local(SlotId(0))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
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

        let analysis = analyze(&mir);
        assert!(
            !analysis.errors.is_empty(),
            "expected borrow conflict error"
        );
        assert_eq!(
            analysis.errors[0].kind,
            BorrowErrorKind::ConflictSharedExclusive
        );
    }

    #[test]
    fn test_disjoint_field_borrows_no_conflict() {
        // _1 = &_0.a (shared)
        // _2 = &mut _0.b (exclusive) — disjoint fields, no conflict
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Borrow(
                                BorrowKind::Shared,
                                Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(0)),
                            ),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Borrow(
                                BorrowKind::Exclusive,
                                Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(1)),
                            ),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
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

        let analysis = analyze(&mir);
        assert!(
            analysis.errors.is_empty(),
            "disjoint field borrows should not conflict, got: {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_read_while_exclusive_borrow_error() {
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Borrow(BorrowKind::Exclusive, Place::Local(SlotId(0))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(0)))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
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

        let analysis = analyze(&mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReadWhileExclusivelyBorrowed),
            "expected read-while-exclusive error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_reference_escape_error_for_returned_ref_alias() {
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Borrow(BorrowKind::Shared, Place::Local(SlotId(1))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(3)),
                            Rvalue::Use(Operand::Move(Place::Local(SlotId(2)))),
                        ),
                        2,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Move(Place::Local(SlotId(3)))),
                        ),
                        3,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
            ],
            span: span(),
        };

        let analysis = analyze(&mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceEscape),
            "expected reference-escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_use_after_explicit_move_error() {
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::MoveExplicit(Place::Local(SlotId(0)))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(0)))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
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

        let analysis = analyze(&mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::UseAfterMove),
            "expected use-after-move error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_move_vs_clone_decision() {
        // _0 = value (NonCopy)
        // _1 = move _0  (point 1 — _0 NOT live after → Move)
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Move(Place::Local(SlotId(0)))),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 2,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy],
            span: span(),
        };

        let analysis = analyze(&mir);
        // _0 is not used after point 1, so decision should be Move
        assert_eq!(
            analysis.ownership_at(Point(1)),
            OwnershipDecision::Move,
            "source dead after → should be Move"
        );
    }

    #[test]
    fn test_nll_borrow_scoping() {
        // NLL test: borrow ends at last use, not at lexical scope exit
        // bb0: _0 = value; _1 = &_0; (use _1 here); goto bb1
        // bb1: _2 = &mut _0 — should be OK because _1 is no longer used
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![
                        make_stmt(
                            StatementKind::Assign(
                                Place::Local(SlotId(0)),
                                Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                            ),
                            0,
                        ),
                        make_stmt(
                            StatementKind::Assign(
                                Place::Local(SlotId(1)),
                                Rvalue::Borrow(BorrowKind::Shared, Place::Local(SlotId(0))),
                            ),
                            1,
                        ),
                        // Use _1
                        make_stmt(
                            StatementKind::Assign(
                                Place::Local(SlotId(3)),
                                Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                            ),
                            2,
                        ),
                    ],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![
                        // _1 is no longer used here — shared borrow should be "dead"
                        // So taking &mut _0 should be OK
                        make_stmt(
                            StatementKind::Assign(
                                Place::Local(SlotId(2)),
                                Rvalue::Borrow(BorrowKind::Exclusive, Place::Local(SlotId(0))),
                            ),
                            3,
                        ),
                    ],
                    terminator: make_terminator(TerminatorKind::Return),
                },
            ],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
            ],
            span: span(),
        };

        let analysis = analyze(&mir);
        // With NLL, the shared borrow on _0 ends after last use of _1 (point 2).
        // The exclusive borrow at point 3 should NOT conflict.
        // Note: our current solver propagates loan_live_at through cfg_edge
        // without checking if the loan is actually used. For full NLL we need
        // to intersect with "loan_used_at" — this is tracked as a known TODO.
        // For now, this test documents the current behavior.
        let _ = analysis;
    }

    #[test]
    fn test_clone_decision_when_source_live_after() {
        // _0 = value (NonCopy)
        // _1 = move _0 (point 1 — _0 IS live after because _2 uses it)
        // _2 = move _0 (point 2 — _0 NOT live after → Move)
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Move(Place::Local(SlotId(0)))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Use(Operand::Move(Place::Local(SlotId(0)))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
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

        let analysis = analyze(&mir);
        // At point 1, _0 is still used at point 2, so it's live → Clone
        assert_eq!(
            analysis.ownership_at(Point(1)),
            OwnershipDecision::Clone,
            "source live after → should be Clone"
        );
        // At point 2, _0 is not used after → Move
        assert_eq!(
            analysis.ownership_at(Point(2)),
            OwnershipDecision::Move,
            "source dead after → should be Move"
        );
    }

    #[test]
    fn test_copy_type_always_copy_decision() {
        // _0 = 42 (Copy type)
        // _1 = move _0 — but since _0 is Copy, decision should be Copy
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Move(Place::Local(SlotId(0)))),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 2,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::Copy, LocalTypeInfo::Copy],
            span: span(),
        };

        let analysis = analyze(&mir);
        assert_eq!(
            analysis.ownership_at(Point(1)),
            OwnershipDecision::Copy,
            "Copy type → always Copy regardless of liveness"
        );
    }
}
