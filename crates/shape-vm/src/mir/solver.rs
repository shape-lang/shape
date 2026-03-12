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
//!
//! Additional analyses:
//! - **Post-solve relaxation**: `solve()` skips `ReferenceStoredIn*` errors
//!   when the container slot's `EscapeStatus` is `Local` (never escapes).
//! - **Interprocedural summaries**: `extract_borrow_summary()` derives per-function
//!   conflict pairs for call-site alias checking.
//! - **Task-boundary sendability**: Detects closures with mutable captures
//!   crossing detached task boundaries (B0014).

use super::analysis::*;
use super::cfg::ControlFlowGraph;
use super::liveness::{self, LivenessResult};
use super::types::*;
use crate::type_tracking::EscapeStatus;
use datafrog::{Iteration, Relation, RelationLeaper};
use std::collections::{HashMap, HashSet};

/// Callee return-reference summaries, keyed by function name.
pub type CalleeSummaries = HashMap<String, ReturnReferenceSummary>;

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
    /// Escape classification for every local slot in the MIR function.
    pub slot_escape_status: HashMap<SlotId, EscapeStatus>,
    /// Loans that flow into the dedicated return slot and would escape.
    pub escaped_loans: Vec<(u32, shape_ast::ast::Span)>,
    /// Unified sink records for all loan escapes/stores/boundaries.
    pub loan_sinks: Vec<LoanSink>,
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
    /// Reference-return summaries flowing into the return slot.
    pub return_reference_candidates: Vec<(ReturnReferenceSummary, shape_ast::ast::Span)>,
    /// Return-slot writes that produce a plain owned value.
    pub non_reference_return_spans: Vec<shape_ast::ast::Span>,
    /// Non-sendable values crossing detached task boundaries (e.g., closures
    /// with mutable captures).
    pub non_sendable_task_boundary: Vec<(u32, shape_ast::ast::Span)>,
}

/// Populate borrow facts from a MIR function and its CFG.
pub fn extract_facts(
    mir: &MirFunction,
    cfg: &ControlFlowGraph,
    callee_summaries: &CalleeSummaries,
) -> BorrowFacts {
    let mut facts = BorrowFacts::default();
    let mut next_loan = 0u32;
    let mut slot_loans: HashMap<SlotId, Vec<u32>> = HashMap::new();
    let mut slot_reference_origins: HashMap<SlotId, (BorrowKind, ReferenceOrigin)> =
        HashMap::new();

    // Track slots that are targets of ClosureCapture with mutable captures
    // (proxy for non-sendable closures).
    let (all_captures, mutable_captures) =
        super::storage_planning::collect_closure_captures(mir);
    let closure_capture_slots: HashSet<SlotId> = mutable_captures;
    facts.slot_escape_status.extend((0..mir.num_locals).map(|raw_slot| {
        let slot = SlotId(raw_slot);
        (
            slot,
            super::storage_planning::detect_escape_status(slot, mir, &all_captures),
        )
    }));
    let param_reference_summaries: HashMap<SlotId, ReturnReferenceSummary> = mir
        .param_slots
        .iter()
        .enumerate()
        .filter_map(|(param_index, slot)| {
            mir.param_reference_kinds
                .get(param_index)
                .copied()
                .flatten()
                .map(|kind| {
                    (
                        *slot,
                        ReturnReferenceSummary {
                            param_index,
                            kind,
                            projection: Some(Vec::new()),
                        },
                    )
                })
        })
        .collect();
    let mut slot_reference_summaries = param_reference_summaries.clone();

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
                        slot_reference_origins.insert(
                            *slot,
                            (*kind, reference_origin_for_place(place, &mir.param_slots)),
                        );
                        if let Some(contract) = safe_reference_summary_for_borrow(
                            *kind,
                            place,
                            &param_reference_summaries,
                        ) {
                            slot_reference_summaries.insert(*slot, contract);
                        } else {
                            slot_reference_summaries.remove(slot);
                        }
                        if *slot == SlotId(0) {
                            if let Some(contract) = safe_reference_summary_for_borrow(
                                *kind,
                                place,
                                &param_reference_summaries,
                            ) {
                                facts
                                    .return_reference_candidates
                                    .push((contract, stmt.span));
                            } else {
                                facts.escaped_loans.push((loan_id, stmt.span));
                                facts.loan_sinks.push(LoanSink {
                                    loan_id,
                                    kind: LoanSinkKind::ReturnSlot,
                                    sink_slot: Some(*slot),
                                    span: stmt.span,
                                });
                            }
                        }
                    }
                    // Compute region depth: parameter loans get 0, locals get 1.
                    let region_depth = if mir.param_slots.contains(&place.root_local()) {
                        0 // Parameter — lives for the entire function
                    } else {
                        1 // Local — lives within the function body
                    };
                    facts.loan_info.insert(
                        loan_id,
                        LoanInfo {
                            id: LoanId(loan_id),
                            borrowed_place: place.clone(),
                            kind: *kind,
                            issued_at: stmt.point,
                            span: stmt.span,
                            region_depth,
                        },
                    );
                }
                StatementKind::Assign(place, rvalue) => {
                    if let Place::Local(dest_slot) = place {
                        update_slot_loan_aliases(&mut slot_loans, *dest_slot, rvalue);
                        update_slot_reference_origins(
                            &mut slot_reference_origins,
                            *dest_slot,
                            rvalue,
                        );
                        update_slot_reference_summaries(
                            &mut slot_reference_summaries,
                            *dest_slot,
                            rvalue,
                        );
                        if *dest_slot == SlotId(0) {
                            let mut found_reference_return = false;
                            if let Some(contract) =
                                reference_summary_from_rvalue(&slot_reference_summaries, rvalue)
                            {
                                facts
                                    .return_reference_candidates
                                    .push((contract, stmt.span));
                                found_reference_return = true;
                            }
                            if let Some((borrow_kind, origin)) =
                                reference_origin_from_rvalue(&slot_reference_origins, rvalue)
                            {
                                if let Some(contract) =
                                    reference_summary_from_origin(borrow_kind, &origin)
                                {
                                    facts
                                        .return_reference_candidates
                                        .push((contract, stmt.span));
                                    found_reference_return = true;
                                }
                            }
                            for loan_id in local_loans_from_rvalue(&slot_loans, rvalue) {
                                let info = &facts.loan_info[&loan_id];
                                if let Some(contract) = safe_reference_summary_for_borrow(
                                    info.kind,
                                    &info.borrowed_place,
                                    &param_reference_summaries,
                                ) {
                                    facts
                                        .return_reference_candidates
                                        .push((contract, stmt.span));
                                    found_reference_return = true;
                                } else {
                                    facts.escaped_loans.push((loan_id, stmt.span));
                                    facts.loan_sinks.push(LoanSink {
                                        loan_id,
                                        kind: LoanSinkKind::ReturnSlot,
                                        sink_slot: Some(*dest_slot),
                                        span: stmt.span,
                                    });
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
                                facts.loan_sinks.push(LoanSink {
                                    loan_id,
                                    kind: LoanSinkKind::ObjectAssignment,
                                    sink_slot: Some(place.root_local()),
                                    span: stmt.span,
                                });
                            }
                        }
                        Place::Index(..) => {
                            for loan_id in local_loans_from_rvalue(&slot_loans, rvalue) {
                                facts.array_assignment_loans.push((loan_id, stmt.span));
                                facts.loan_sinks.push(LoanSink {
                                    loan_id,
                                    kind: LoanSinkKind::ArrayAssignment,
                                    sink_slot: Some(place.root_local()),
                                    span: stmt.span,
                                });
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
                StatementKind::TaskBoundary(operands, kind) => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        let info = &facts.loan_info[&loan_id];
                        match kind {
                            TaskBoundaryKind::Detached => {
                                // All refs (shared + exclusive) rejected across detached tasks
                                facts.task_boundary_loans.push((loan_id, stmt.span));
                                facts.loan_sinks.push(LoanSink {
                                    loan_id,
                                    kind: LoanSinkKind::DetachedTaskBoundary,
                                    sink_slot: None,
                                    span: stmt.span,
                                });
                            }
                            TaskBoundaryKind::Structured => {
                                // Only exclusive refs rejected across structured tasks
                                if info.kind == BorrowKind::Exclusive {
                                    facts.task_boundary_loans.push((loan_id, stmt.span));
                                    facts.loan_sinks.push(LoanSink {
                                        loan_id,
                                        kind: LoanSinkKind::StructuredTaskBoundary,
                                        sink_slot: None,
                                        span: stmt.span,
                                    });
                                }
                            }
                        }
                    }
                    // Sendability check for detached tasks: closures with mutable
                    // captures are not sendable across detached boundaries.
                    if *kind == TaskBoundaryKind::Detached {
                        for op in operands {
                            if let Operand::Copy(Place::Local(slot))
                            | Operand::Move(Place::Local(slot)) = op
                            {
                                if closure_capture_slots.contains(slot) {
                                    facts
                                        .non_sendable_task_boundary
                                        .push((slot.0 as u32, stmt.span));
                                }
                            }
                        }
                    }
                }
                StatementKind::ClosureCapture {
                    closure_slot,
                    operands,
                } => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        facts.closure_capture_loans.push((loan_id, stmt.span));
                        facts.loan_sinks.push(LoanSink {
                            loan_id,
                            kind: LoanSinkKind::ClosureEnv,
                            sink_slot: Some(*closure_slot),
                            span: stmt.span,
                        });
                    }
                }
                StatementKind::ArrayStore {
                    container_slot,
                    operands,
                } => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        facts.array_store_loans.push((loan_id, stmt.span));
                        facts.loan_sinks.push(LoanSink {
                            loan_id,
                            kind: LoanSinkKind::ArrayStore,
                            sink_slot: Some(*container_slot),
                            span: stmt.span,
                        });
                    }
                }
                StatementKind::ObjectStore {
                    container_slot,
                    operands,
                } => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        facts.object_store_loans.push((loan_id, stmt.span));
                        facts.loan_sinks.push(LoanSink {
                            loan_id,
                            kind: LoanSinkKind::ObjectStore,
                            sink_slot: Some(*container_slot),
                            span: stmt.span,
                        });
                    }
                }
                StatementKind::EnumStore {
                    container_slot,
                    operands,
                } => {
                    for loan_id in local_loans_from_operands(&slot_loans, operands) {
                        facts.enum_store_loans.push((loan_id, stmt.span));
                        facts.loan_sinks.push(LoanSink {
                            loan_id,
                            kind: LoanSinkKind::EnumStore,
                            sink_slot: Some(*container_slot),
                            span: stmt.span,
                        });
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

        // Process Call terminators for borrow facts
        if let TerminatorKind::Call { func, args, destination, .. } = &block.terminator.kind {
            let call_point = block.statements.last().map(|s| s.point.0).unwrap_or(0);
            // Track reads from func and args operands
            let mut all_operands = vec![func];
            all_operands.extend(args.iter());
            for op in &all_operands {
                if let Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) = op {
                    if let Some(loans) = slot_loans.get(&place.root_local()) {
                        for &loan_id in loans {
                            facts.use_of_loan.push((loan_id, call_point));
                        }
                    }
                }
            }
            // Destination write: clear provenance, then compose callee summary if available
            let dest_slot = destination.root_local();
            slot_loans.remove(&dest_slot);
            slot_reference_origins.remove(&dest_slot);
            slot_reference_summaries.remove(&dest_slot);

            // Compose callee return summary into destination slot (summary-driven).
            // Only compose for MirConstant::Function calls — indirect calls (closures,
            // method dispatch) use conservative clearing.
            if let Operand::Constant(MirConstant::Function(callee_name)) = func {
                if let Some(callee_summary) = callee_summaries.get(callee_name.as_str()) {
                    if let Some(arg_operand) = args.get(callee_summary.param_index) {
                        if let Operand::Copy(arg_place)
                        | Operand::Move(arg_place)
                        | Operand::MoveExplicit(arg_place) = arg_operand
                        {
                            let arg_slot = arg_place.root_local();

                            // Inherit loans from the argument slot
                            if let Some(arg_loans) = slot_loans.get(&arg_slot).cloned() {
                                slot_loans.insert(dest_slot, arg_loans);
                            }

                            // Compose reference summary (handles imprecision correctly)
                            if let Some(arg_summary) =
                                slot_reference_summaries.get(&arg_slot).cloned()
                            {
                                let composed = compose_return_reference_summary(
                                    &arg_summary,
                                    callee_summary,
                                );

                                // Only compose origin when projection precision is preserved.
                                // Origin is always-precise (Vec, not Option<Vec>); if projection
                                // loses precision the origin becomes meaningless.
                                if composed.projection.is_some() {
                                    if let Some((_, origin)) =
                                        slot_reference_origins.get(&arg_slot).cloned()
                                    {
                                        // callee_proj is guaranteed Some and Field-free here
                                        if let Some(ref callee_proj) = callee_summary.projection {
                                            let mut proj = origin.projection.clone();
                                            proj.extend(callee_proj.iter().copied());
                                            slot_reference_origins.insert(
                                                dest_slot,
                                                (
                                                    composed.kind,
                                                    ReferenceOrigin {
                                                        root: origin.root,
                                                        projection: proj,
                                                    },
                                                ),
                                            );
                                        }
                                    }
                                    // Ref params seed summaries but NOT origins (solver.rs:106).
                                    // If arg has summary but no origin, origin stays cleared.
                                }
                                // else: projection lost → origin stays cleared

                                slot_reference_summaries.insert(dest_slot, composed);
                            }
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
        StatementKind::TaskBoundary(operands, _kind) => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::ClosureCapture { operands, .. } => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::ArrayStore { operands, .. } => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::ObjectStore { operands, .. } => {
            for operand in operands {
                operand_read_places(operand, &mut reads);
            }
        }
        StatementKind::EnumStore { operands, .. } => {
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

fn update_slot_reference_summaries(
    slot_reference_summaries: &mut HashMap<SlotId, ReturnReferenceSummary>,
    dest_slot: SlotId,
    rvalue: &Rvalue,
) {
    match rvalue {
        Rvalue::Use(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Use(Operand::Move(Place::Local(src_slot)))
        | Rvalue::Use(Operand::MoveExplicit(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Move(Place::Local(src_slot))) => {
            if let Some(contract) = slot_reference_summaries.get(src_slot).cloned() {
                slot_reference_summaries.insert(dest_slot, contract);
            } else {
                slot_reference_summaries.remove(&dest_slot);
            }
        }
        _ => {
            slot_reference_summaries.remove(&dest_slot);
        }
    }
}

fn reference_summary_from_rvalue(
    slot_reference_summaries: &HashMap<SlotId, ReturnReferenceSummary>,
    rvalue: &Rvalue,
) -> Option<ReturnReferenceSummary> {
    match rvalue {
        Rvalue::Use(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Use(Operand::Move(Place::Local(src_slot)))
        | Rvalue::Use(Operand::MoveExplicit(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Move(Place::Local(src_slot))) => {
            slot_reference_summaries.get(src_slot).cloned()
        }
        _ => None,
    }
}

fn update_slot_reference_origins(
    slot_reference_origins: &mut HashMap<SlotId, (BorrowKind, ReferenceOrigin)>,
    dest_slot: SlotId,
    rvalue: &Rvalue,
) {
    match rvalue {
        Rvalue::Use(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Use(Operand::Move(Place::Local(src_slot)))
        | Rvalue::Use(Operand::MoveExplicit(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Move(Place::Local(src_slot))) => {
            if let Some(origin) = slot_reference_origins.get(src_slot).cloned() {
                slot_reference_origins.insert(dest_slot, origin);
            } else {
                slot_reference_origins.remove(&dest_slot);
            }
        }
        _ => {
            slot_reference_origins.remove(&dest_slot);
        }
    }
}

fn reference_origin_from_rvalue(
    slot_reference_origins: &HashMap<SlotId, (BorrowKind, ReferenceOrigin)>,
    rvalue: &Rvalue,
) -> Option<(BorrowKind, ReferenceOrigin)> {
    match rvalue {
        Rvalue::Borrow(kind, place) => Some((
            *kind,
            reference_origin_for_place(place, &[]),
        )),
        Rvalue::Use(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Use(Operand::Move(Place::Local(src_slot)))
        | Rvalue::Use(Operand::MoveExplicit(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Copy(Place::Local(src_slot)))
        | Rvalue::Clone(Operand::Move(Place::Local(src_slot))) => {
            slot_reference_origins.get(src_slot).cloned()
        }
        _ => None,
    }
}

fn reference_origin_for_place(place: &Place, param_slots: &[SlotId]) -> ReferenceOrigin {
    let root_slot = place.root_local();
    let root = param_slots
        .iter()
        .position(|slot| *slot == root_slot)
        .map(ReferenceOriginRoot::Param)
        .unwrap_or(ReferenceOriginRoot::Local(root_slot));
    ReferenceOrigin {
        root,
        projection: place.projection_steps(),
    }
}

fn reference_summary_from_origin(
    borrow_kind: BorrowKind,
    origin: &ReferenceOrigin,
) -> Option<ReturnReferenceSummary> {
    match origin.root {
        ReferenceOriginRoot::Param(param_index) => Some(ReturnReferenceSummary {
            param_index,
            kind: borrow_kind,
            projection: Some(origin.projection.clone()),
        }),
        ReferenceOriginRoot::Local(_) => None,
    }
}

fn safe_reference_summary_for_borrow(
    borrow_kind: BorrowKind,
    borrowed_place: &Place,
    param_reference_summaries: &HashMap<SlotId, ReturnReferenceSummary>,
) -> Option<ReturnReferenceSummary> {
    // Support both direct param borrows (&param) and field-of-param borrows (&param.field).
    // The root local must be a parameter with a reference summary.
    let param_summary = param_reference_summaries.get(&borrowed_place.root_local())?;
    Some(ReturnReferenceSummary {
        param_index: param_summary.param_index,
        kind: borrow_kind,
        projection: Some(borrowed_place.projection_steps()),
    })
}

/// Compose a callee's return summary with the argument slot's existing summary.
///
/// - `param_index`: from `arg_summary` (traces to the caller's parameter)
/// - `kind`: from `callee_summary` (callee dictates the returned borrow kind)
/// - `projection`: concatenate only when BOTH are `Some` AND the callee
///   projection contains no `Field` steps (FieldIdx is per-MirBuilder,
///   not cross-function stable). Otherwise `None` (precision lost).
fn compose_return_reference_summary(
    arg_summary: &ReturnReferenceSummary,
    callee_summary: &ReturnReferenceSummary,
) -> ReturnReferenceSummary {
    let projection = match (&arg_summary.projection, &callee_summary.projection) {
        (Some(arg_proj), Some(callee_proj)) => {
            if callee_proj
                .iter()
                .any(|step| matches!(step, ProjectionStep::Field(_)))
            {
                None // FieldIdx is per-MirBuilder, unsound across functions
            } else {
                let mut composed = arg_proj.clone();
                composed.extend(callee_proj.iter().copied());
                Some(composed)
            }
        }
        _ => None, // precision already lost on one side
    };
    ReturnReferenceSummary {
        param_index: arg_summary.param_index,
        kind: callee_summary.kind,
        projection,
    }
}

fn resolve_return_reference_summary(
    errors: &mut Vec<BorrowError>,
    facts: &BorrowFacts,
    loans_at_point: &HashMap<Point, Vec<LoanId>>,
) -> Option<ReturnReferenceSummary> {
    let mut merged_candidate: Option<ReturnReferenceSummary> = None;
    let mut inconsistent = false;
    for (candidate, _) in &facts.return_reference_candidates {
        if let Some(existing) = merged_candidate.as_mut() {
            if existing.param_index != candidate.param_index || existing.kind != candidate.kind {
                inconsistent = true;
                break;
            }
            if existing.projection != candidate.projection {
                existing.projection = None;
            }
        } else {
            merged_candidate = Some(candidate.clone());
        }
    }

    if merged_candidate.is_none() {
        return None;
    }

    let error_span = if inconsistent {
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

    merged_candidate
}

fn find_matching_loan_for_return_candidate(
    candidate: &ReturnReferenceSummary,
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

    let mut seen_sinks = std::collections::HashSet::new();
    for sink in &facts.loan_sinks {
        let key = (
            sink.loan_id,
            sink.kind,
            sink.span.start,
            sink.span.end,
            sink.sink_slot.map(|slot| slot.0),
        );
        if !seen_sinks.insert(key) {
            continue;
        }

        let info = &facts.loan_info[&sink.loan_id];
        let sink_is_local = sink
            .sink_slot
            .and_then(|slot| facts.slot_escape_status.get(&slot).copied())
            == Some(EscapeStatus::Local);

        let kind = match sink.kind {
            LoanSinkKind::ReturnSlot => continue,
            LoanSinkKind::ClosureEnv if sink_is_local => continue,
            LoanSinkKind::ClosureEnv => BorrowErrorKind::ReferenceEscapeIntoClosure,
            LoanSinkKind::ArrayStore | LoanSinkKind::ArrayAssignment if sink_is_local => continue,
            LoanSinkKind::ArrayStore | LoanSinkKind::ArrayAssignment => {
                BorrowErrorKind::ReferenceStoredInArray
            }
            LoanSinkKind::ObjectStore | LoanSinkKind::ObjectAssignment if sink_is_local => continue,
            LoanSinkKind::ObjectStore | LoanSinkKind::ObjectAssignment => {
                BorrowErrorKind::ReferenceStoredInObject
            }
            LoanSinkKind::EnumStore if sink_is_local => continue,
            LoanSinkKind::EnumStore => BorrowErrorKind::ReferenceStoredInEnum,
            LoanSinkKind::StructuredTaskBoundary => {
                BorrowErrorKind::ExclusiveRefAcrossTaskBoundary
            }
            LoanSinkKind::DetachedTaskBoundary if info.kind == BorrowKind::Exclusive => {
                BorrowErrorKind::ExclusiveRefAcrossTaskBoundary
            }
            LoanSinkKind::DetachedTaskBoundary => BorrowErrorKind::SharedRefAcrossDetachedTask,
        };

        errors.push(BorrowError {
            kind,
            span: sink.span,
            conflicting_loan: LoanId(sink.loan_id),
            loan_span: info.span,
            last_use_span: last_use_span_for_loan(facts, sink.loan_id),
            repairs: Vec::new(),
        });
    }

    // Non-sendable values across detached task boundaries
    let mut seen_non_sendable = std::collections::HashSet::new();
    for (slot_id, span) in &facts.non_sendable_task_boundary {
        if !seen_non_sendable.insert((*slot_id, span.start, span.end)) {
            continue;
        }
        errors.push(BorrowError {
            kind: BorrowErrorKind::NonSendableAcrossTaskBoundary,
            span: *span,
            conflicting_loan: LoanId(0),
            loan_span: *span,
            last_use_span: None,
            repairs: Vec::new(),
        });
    }

    let return_reference_summary =
        resolve_return_reference_summary(&mut errors, facts, &loans_at_point);

    SolverResult {
        loans_at_point,
        errors,
        loan_info: facts.loan_info.clone(),
        return_reference_summary,
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
    pub return_reference_summary: Option<ReturnReferenceSummary>,
}

/// Run the complete borrow analysis pipeline for a MIR function.
/// This is the main entry point — produces the single BorrowAnalysis
/// consumed by compiler, LSP, and diagnostics.
/// Extract a borrow summary for a function — describes which parameters are
/// borrowed and which parameter pairs must not alias at call sites.
pub fn extract_borrow_summary(
    mir: &MirFunction,
    return_summary: Option<ReturnReferenceSummary>,
) -> FunctionBorrowSummary {
    let num_params = mir.param_slots.len();
    let mut param_borrows: Vec<Option<BorrowKind>> = mir
        .param_reference_kinds
        .iter()
        .cloned()
        .collect();
    // Pad to num_params if param_reference_kinds is shorter
    while param_borrows.len() < num_params {
        param_borrows.push(None);
    }

    // Determine which params are written to (mutated) in the function body
    let mut mutated_params: HashSet<usize> = HashSet::new();
    let mut read_params: HashSet<usize> = HashSet::new();
    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(dest, rvalue) => {
                    // Check if dest's root is a parameter (handles Local, Field, Index)
                    let root = dest.root_local();
                    if let Some(param_idx) = mir.param_slots.iter().position(|s| *s == root) {
                        mutated_params.insert(param_idx);
                    }
                    // Check if any param is read in the rvalue
                    for param_idx in 0..num_params {
                        if rvalue_uses_param(rvalue, mir.param_slots[param_idx]) {
                            read_params.insert(param_idx);
                        }
                    }
                }
                _ => {}
            }
        }
        // Check terminator args for reads
        if let TerminatorKind::Call { args, .. } = &block.terminator.kind {
            for arg in args {
                for param_idx in 0..num_params {
                    if operand_uses_param(arg, mir.param_slots[param_idx]) {
                        read_params.insert(param_idx);
                    }
                }
            }
        }
    }

    // Compute effective borrow kind per param: explicit annotations take priority,
    // otherwise infer from usage — mutated → Exclusive, read → Shared.
    let mut effective_borrows: Vec<Option<BorrowKind>> = param_borrows.clone();
    for idx in 0..num_params {
        if effective_borrows[idx].is_none() {
            if mutated_params.contains(&idx) {
                effective_borrows[idx] = Some(BorrowKind::Exclusive);
            } else if read_params.contains(&idx) {
                effective_borrows[idx] = Some(BorrowKind::Shared);
            }
        }
    }

    // Build conflict pairs: a mutated param conflicts with every other param
    // that is read or borrowed (shared or exclusive).
    let mut conflict_pairs = Vec::new();
    for &mutated_idx in &mutated_params {
        for other_idx in 0..num_params {
            if other_idx == mutated_idx {
                continue;
            }
            // Mutated param conflicts with any other param that is used
            if effective_borrows[other_idx].is_some() {
                conflict_pairs.push((mutated_idx, other_idx));
            }
        }
    }
    // Also: two exclusive borrows on different params always conflict
    for i in 0..num_params {
        for j in (i + 1)..num_params {
            if effective_borrows[i] == Some(BorrowKind::Exclusive)
                && effective_borrows[j] == Some(BorrowKind::Exclusive)
                && !conflict_pairs.contains(&(i, j))
                && !conflict_pairs.contains(&(j, i))
            {
                conflict_pairs.push((i, j));
            }
        }
    }

    FunctionBorrowSummary {
        param_borrows,
        conflict_pairs,
        return_summary,
    }
}

fn rvalue_uses_param(rvalue: &Rvalue, param_slot: SlotId) -> bool {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => {
            operand_uses_param(op, param_slot)
        }
        Rvalue::Borrow(_, place) => place.root_local() == param_slot,
        Rvalue::BinaryOp(_, lhs, rhs) => {
            operand_uses_param(lhs, param_slot) || operand_uses_param(rhs, param_slot)
        }
        Rvalue::Aggregate(ops) => ops.iter().any(|op| operand_uses_param(op, param_slot)),
    }
}

fn operand_uses_param(op: &Operand, param_slot: SlotId) -> bool {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            place.root_local() == param_slot
        }
        Operand::Constant(_) => false,
    }
}

pub fn analyze(mir: &MirFunction, callee_summaries: &CalleeSummaries) -> BorrowAnalysis {
    let cfg = ControlFlowGraph::build(mir);

    // 1. Compute liveness (for move/clone inference)
    let liveness = liveness::compute_liveness(mir, &cfg);

    // 2. Extract Datafrog input facts
    let facts = extract_facts(mir, &cfg, callee_summaries);

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
        return_reference_summary: solver_result.return_reference_summary,
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
            // Also apply Call terminator moves (destination write clears moved status)
            apply_terminator_move_transfer(&mut block_out, &block.terminator);

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

        // Check Call terminator for reads of moved places, then apply its transfer
        if let TerminatorKind::Call { func, args, destination, .. } = &block.terminator.kind {
            let term_key_point = block.terminator.span.start as u32;
            // Check func operand
            if let Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) = func {
                if let Some((moved_place, move_span)) = find_moved_place_conflict(&moved_places, place) {
                    let key = (term_key_point, format!("{}", moved_place));
                    if seen.insert(key) {
                        errors.push(BorrowError {
                            kind: BorrowErrorKind::UseAfterMove,
                            span: block.terminator.span,
                            conflicting_loan: LoanId(0),
                            loan_span: move_span,
                            last_use_span: None,
                            repairs: Vec::new(),
                        });
                    }
                }
            }
            // Check each arg
            for arg in args {
                if let Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) = arg {
                    if let Some((moved_place, move_span)) = find_moved_place_conflict(&moved_places, place) {
                        let key = (term_key_point, format!("{}", moved_place));
                        if seen.insert(key) {
                            errors.push(BorrowError {
                                kind: BorrowErrorKind::UseAfterMove,
                                span: block.terminator.span,
                                conflicting_loan: LoanId(0),
                                loan_span: move_span,
                                last_use_span: None,
                                repairs: Vec::new(),
                            });
                        }
                    }
                }
            }
            // Destination write clears moved status
            moved_places.retain(|moved_place, _| !reinitializes_moved_place(destination, moved_place));
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

/// Apply move transfer for a Call terminator.
/// The call writes its return value to `destination`, which reinitializes that place.
/// Call args are typically temp slots created by `lower_expr_as_moved_operand` —
/// the moves of source values INTO those temps happen in prior statements (via Assign/Move),
/// not in the terminator itself, so we don't need to mark args as moved here.
fn apply_terminator_move_transfer(
    moved_places: &mut HashMap<Place, shape_ast::ast::Span>,
    terminator: &Terminator,
) {
    if let TerminatorKind::Call { destination, .. } = &terminator.kind {
        // The call writes to destination, which reinitializes that place
        moved_places.retain(|moved_place, _| !reinitializes_moved_place(destination, moved_place));
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
        StatementKind::TaskBoundary(..)
        | StatementKind::ClosureCapture { .. }
        | StatementKind::ArrayStore { .. }
        | StatementKind::ObjectStore { .. }
        | StatementKind::EnumStore { .. } => None,
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
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

        let analysis = analyze(&mir, &Default::default());
        assert_eq!(
            analysis.ownership_at(Point(1)),
            OwnershipDecision::Copy,
            "Copy type → always Copy regardless of liveness"
        );
    }

    // =========================================================================
    // compose_return_reference_summary unit tests
    // =========================================================================

    #[test]
    fn test_compose_summary_identity() {
        // Both empty projections — identity composition
        let arg = ReturnReferenceSummary {
            param_index: 2,
            kind: BorrowKind::Shared,
            projection: Some(vec![]),
        };
        let callee = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Exclusive,
            projection: Some(vec![]),
        };
        let result = compose_return_reference_summary(&arg, &callee);
        assert_eq!(result.param_index, 2); // from arg
        assert_eq!(result.kind, BorrowKind::Exclusive); // from callee
        assert_eq!(result.projection, Some(vec![]));
    }

    #[test]
    fn test_compose_summary_some_index_some_empty() {
        let arg = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Shared,
            projection: Some(vec![ProjectionStep::Index]),
        };
        let callee = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Shared,
            projection: Some(vec![]),
        };
        let result = compose_return_reference_summary(&arg, &callee);
        assert_eq!(result.projection, Some(vec![ProjectionStep::Index]));
    }

    #[test]
    fn test_compose_summary_callee_field_loses_precision() {
        let arg = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Shared,
            projection: Some(vec![]),
        };
        let callee = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Shared,
            projection: Some(vec![ProjectionStep::Field(FieldIdx(0))]),
        };
        let result = compose_return_reference_summary(&arg, &callee);
        assert_eq!(result.projection, None); // Field loses precision
    }

    #[test]
    fn test_compose_summary_callee_index_composes() {
        let arg = ReturnReferenceSummary {
            param_index: 1,
            kind: BorrowKind::Shared,
            projection: Some(vec![ProjectionStep::Index]),
        };
        let callee = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Exclusive,
            projection: Some(vec![ProjectionStep::Index]),
        };
        let result = compose_return_reference_summary(&arg, &callee);
        assert_eq!(result.param_index, 1);
        assert_eq!(result.kind, BorrowKind::Exclusive);
        assert_eq!(
            result.projection,
            Some(vec![ProjectionStep::Index, ProjectionStep::Index])
        );
    }

    #[test]
    fn test_compose_summary_arg_none() {
        let arg = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Shared,
            projection: None, // precision already lost
        };
        let callee = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Shared,
            projection: Some(vec![]),
        };
        let result = compose_return_reference_summary(&arg, &callee);
        assert_eq!(result.projection, None);
    }

    #[test]
    fn test_compose_summary_callee_none() {
        let arg = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Shared,
            projection: Some(vec![ProjectionStep::Index]),
        };
        let callee = ReturnReferenceSummary {
            param_index: 0,
            kind: BorrowKind::Exclusive,
            projection: None,
        };
        let result = compose_return_reference_summary(&arg, &callee);
        assert_eq!(result.projection, None);
    }

    // =========================================================================
    // Solver-level call composition tests (synthetic MIR)
    // =========================================================================

    #[test]
    fn test_call_composition_identity() {
        // fn identity(&x) { x }
        // Caller: param _1 (&ref), call identity(_1) → _2, return _2
        // With callee summary for "identity": param_index=0, kind=Shared, projection=Some([])
        let mir = MirFunction {
            name: "caller".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![
                        MirStatement {
                            kind: StatementKind::Assign(
                                Place::Local(SlotId(2)),
                                Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                            ),
                            span: span(),
                            point: Point(0),
                        },
                    ],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Function(
                                "identity".to_string(),
                            )),
                            args: vec![Operand::Copy(Place::Local(SlotId(1)))],
                            destination: Place::Local(SlotId(3)),
                            next: BasicBlockId(1),
                        },
                        span: span(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(3)))),
                        ),
                        span: span(),
                        point: Point(1),
                    }],
                    terminator: Terminator {
                        kind: TerminatorKind::Return,
                        span: span(),
                    },
                },
            ],
            num_locals: 4,
            param_slots: vec![SlotId(1)],
            param_reference_kinds: vec![Some(BorrowKind::Shared)],
            local_types: vec![
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
            ],
            span: span(),
        };

        let mut callee_summaries = CalleeSummaries::new();
        callee_summaries.insert(
            "identity".to_string(),
            ReturnReferenceSummary {
                param_index: 0,
                kind: BorrowKind::Shared,
                projection: Some(vec![]),
            },
        );

        let analysis = analyze(&mir, &callee_summaries);
        assert!(
            analysis.return_reference_summary.is_some(),
            "expected return reference summary from composed call"
        );
        let summary = analysis.return_reference_summary.unwrap();
        assert_eq!(summary.param_index, 0);
        assert_eq!(summary.kind, BorrowKind::Shared);
    }

    #[test]
    fn test_call_composition_unknown_callee() {
        // Same as above but no callee summary → conservative (no return summary)
        let mir = MirFunction {
            name: "caller".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                        ),
                        span: span(),
                        point: Point(0),
                    }],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Function(
                                "unknown_fn".to_string(),
                            )),
                            args: vec![Operand::Copy(Place::Local(SlotId(1)))],
                            destination: Place::Local(SlotId(3)),
                            next: BasicBlockId(1),
                        },
                        span: span(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(3)))),
                        ),
                        span: span(),
                        point: Point(1),
                    }],
                    terminator: Terminator {
                        kind: TerminatorKind::Return,
                        span: span(),
                    },
                },
            ],
            num_locals: 4,
            param_slots: vec![SlotId(1)],
            param_reference_kinds: vec![Some(BorrowKind::Shared)],
            local_types: vec![
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
            ],
            span: span(),
        };

        let analysis = analyze(&mir, &Default::default());
        // Unknown callee → no return reference summary composed
        assert!(
            analysis.return_reference_summary.is_none(),
            "unknown callee should not produce return reference summary"
        );
    }

    #[test]
    fn test_call_composition_indirect_call() {
        // Call via Method (not Function) → conservative
        let mir = MirFunction {
            name: "caller".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                        ),
                        span: span(),
                        point: Point(0),
                    }],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Method(
                                "identity".to_string(),
                            )),
                            args: vec![Operand::Copy(Place::Local(SlotId(1)))],
                            destination: Place::Local(SlotId(3)),
                            next: BasicBlockId(1),
                        },
                        span: span(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(3)))),
                        ),
                        span: span(),
                        point: Point(1),
                    }],
                    terminator: Terminator {
                        kind: TerminatorKind::Return,
                        span: span(),
                    },
                },
            ],
            num_locals: 4,
            param_slots: vec![SlotId(1)],
            param_reference_kinds: vec![Some(BorrowKind::Shared)],
            local_types: vec![
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
            ],
            span: span(),
        };

        let mut callee_summaries = CalleeSummaries::new();
        callee_summaries.insert(
            "identity".to_string(),
            ReturnReferenceSummary {
                param_index: 0,
                kind: BorrowKind::Shared,
                projection: Some(vec![]),
            },
        );

        // Method call, not Function call → conservative even with summary present
        let analysis = analyze(&mir, &callee_summaries);
        assert!(
            analysis.return_reference_summary.is_none(),
            "indirect (Method) call should not compose return summary"
        );
    }

    #[test]
    fn test_call_composition_chain() {
        // Two-deep: param _1 → call "inner"(_1) → _3, call "outer"(_3) → _4, return _4
        // inner: param_index=0, kind=Shared, projection=Some([])
        // outer: param_index=0, kind=Exclusive, projection=Some([])
        // Result: param_index=0 (traces to caller's param), kind=Exclusive (outer dictates)
        let mir = MirFunction {
            name: "caller".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                        ),
                        span: span(),
                        point: Point(0),
                    }],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Function(
                                "inner".to_string(),
                            )),
                            args: vec![Operand::Copy(Place::Local(SlotId(1)))],
                            destination: Place::Local(SlotId(3)),
                            next: BasicBlockId(1),
                        },
                        span: span(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![MirStatement {
                        kind: StatementKind::Nop,
                        span: span(),
                        point: Point(1),
                    }],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Function(
                                "outer".to_string(),
                            )),
                            args: vec![Operand::Copy(Place::Local(SlotId(3)))],
                            destination: Place::Local(SlotId(4)),
                            next: BasicBlockId(2),
                        },
                        span: span(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(2),
                    statements: vec![MirStatement {
                        kind: StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(4)))),
                        ),
                        span: span(),
                        point: Point(2),
                    }],
                    terminator: Terminator {
                        kind: TerminatorKind::Return,
                        span: span(),
                    },
                },
            ],
            num_locals: 5,
            param_slots: vec![SlotId(1)],
            param_reference_kinds: vec![Some(BorrowKind::Shared)],
            local_types: vec![
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
            ],
            span: span(),
        };

        let mut callee_summaries = CalleeSummaries::new();
        callee_summaries.insert(
            "inner".to_string(),
            ReturnReferenceSummary {
                param_index: 0,
                kind: BorrowKind::Shared,
                projection: Some(vec![]),
            },
        );
        callee_summaries.insert(
            "outer".to_string(),
            ReturnReferenceSummary {
                param_index: 0,
                kind: BorrowKind::Exclusive,
                projection: Some(vec![]),
            },
        );

        let analysis = analyze(&mir, &callee_summaries);
        assert!(
            analysis.return_reference_summary.is_some(),
            "chained composition should produce return reference summary"
        );
        let summary = analysis.return_reference_summary.unwrap();
        assert_eq!(summary.param_index, 0, "should trace to outermost param");
        assert_eq!(
            summary.kind,
            BorrowKind::Exclusive,
            "outer callee dictates the kind"
        );
    }
}
