//! Storage Planning Pass — decides the runtime storage class for each binding.
//!
//! After MIR lowering and borrow analysis, this pass examines each local slot
//! and assigns a `BindingStorageClass`:
//!
//! - `Direct`: Default for bindings that are never captured, never aliased, never escape.
//! - `UniqueHeap`: For bindings that escape into closures with mutation (need Arc wrapper).
//! - `SharedCow`: For `var` bindings that are aliased AND mutated (copy-on-write),
//!   or for escaped mutable aliased bindings.
//! - `Reference`: For bindings that hold first-class references.
//! - `Deferred`: Only if analysis was incomplete (had fallbacks).
//!
//! The pass also computes `EscapeStatus` for each slot:
//! - `Local`: Stays within the declaring scope.
//! - `Captured`: Captured by a closure.
//! - `Escaped`: Flows to the return slot (escapes the function).
//!
//! Escape status drives storage decisions (escaped+aliased+mutated → SharedCow)
//! and is consumed by the post-solve relaxation pass to determine whether
//! local containers can safely hold references.
//!
//! The pass runs once per function and produces a `StoragePlan` consumed by codegen.

use std::collections::{HashMap, HashSet};

use crate::mir::analysis::BorrowAnalysis;
use crate::mir::types::*;
use crate::type_tracking::{
    Aliasability, BindingOwnershipClass, BindingSemantics, BindingStorageClass, EscapeStatus,
    MutationCapability,
};

/// The computed storage plan for a single function.
#[derive(Debug, Clone)]
pub struct StoragePlan {
    /// Maps each local slot to its decided storage class.
    pub slot_classes: HashMap<SlotId, BindingStorageClass>,
    /// Maps each local slot to its enriched binding semantics.
    pub slot_semantics: HashMap<SlotId, BindingSemantics>,
}

/// Input bundle for the storage planner.
pub struct StoragePlannerInput<'a> {
    /// The MIR function to plan storage for.
    pub mir: &'a MirFunction,
    /// Borrow analysis results (includes liveness).
    pub analysis: &'a BorrowAnalysis,
    /// Per-slot ownership/storage semantics from the compiler's type tracker.
    pub binding_semantics: &'a HashMap<u16, BindingSemantics>,
    /// Slots captured by any closure in this function.
    pub closure_captures: &'a HashSet<SlotId>,
    /// Slots that are mutated inside a closure body.
    pub mutable_captures: &'a HashSet<SlotId>,
    /// Whether MIR lowering had fallbacks (incomplete analysis).
    pub had_fallbacks: bool,
}

/// Scan MIR statements and terminators to find slots captured by closures.
///
/// Returns `(all_captures, mutable_captures)`:
/// - `all_captures`: slots referenced in `ClosureCapture` statements
/// - `mutable_captures`: subset of captured slots that are assigned more than
///   once in the function (i.e., re-assigned after initial definition). A slot
///   with only its initial definition assignment is not considered mutably captured.
pub fn collect_closure_captures(mir: &MirFunction) -> (HashSet<SlotId>, HashSet<SlotId>) {
    let mut all_captures = HashSet::new();
    let mut assign_counts: HashMap<SlotId, u32> = HashMap::new();

    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::ClosureCapture { operands, .. } => {
                    for op in operands {
                        if let Some(slot) = operand_root_slot(op) {
                            all_captures.insert(slot);
                        }
                    }
                }
                StatementKind::Assign(place, _) => {
                    if let Place::Local(slot) = place {
                        *assign_counts.entry(*slot).or_insert(0) += 1;
                    }
                }
                _ => {}
            }
        }
    }

    // A slot is "mutably captured" if it is captured AND assigned more than once
    // (meaning it has re-assignments beyond its initial definition).
    let mutable_captures: HashSet<SlotId> = all_captures
        .iter()
        .filter(|slot| assign_counts.get(slot).copied().unwrap_or(0) > 1)
        .copied()
        .collect();

    (all_captures, mutable_captures)
}

/// Extract the root SlotId from an operand, if it references a local.
fn operand_root_slot(op: &Operand) -> Option<SlotId> {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            Some(place.root_local())
        }
        Operand::Constant(_) => None,
    }
}

/// Check whether a slot has any active loan (borrow) in the analysis.
/// A slot with loans is holding or being borrowed as a reference.
fn slot_has_active_loans(slot: SlotId, analysis: &BorrowAnalysis) -> bool {
    for loan_info in analysis.loans.values() {
        if loan_info.borrowed_place.root_local() == slot {
            return true;
        }
    }
    false
}

/// Check whether a slot is aliased — it appears as an operand in more than
/// one `Assign` rvalue across the function, or it is captured.
fn slot_is_aliased(slot: SlotId, mir: &MirFunction, closure_captures: &HashSet<SlotId>) -> bool {
    if closure_captures.contains(&slot) {
        return true;
    }

    let mut use_count = 0u32;
    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            if let StatementKind::Assign(_, rvalue) = &stmt.kind {
                if rvalue_uses_slot(rvalue, slot) {
                    use_count += 1;
                    if use_count > 1 {
                        return true;
                    }
                }
            }
        }
        // Also check terminators for uses
        if let TerminatorKind::Call { func, args, .. } = &block.terminator.kind {
            if operand_uses_slot(func, slot) {
                use_count += 1;
            }
            for arg in args {
                if operand_uses_slot(arg, slot) {
                    use_count += 1;
                }
            }
            if use_count > 1 {
                return true;
            }
        }
    }
    false
}

/// Check if a slot is mutated in the function (assigned to after initial definition).
fn slot_is_mutated(slot: SlotId, mir: &MirFunction) -> bool {
    let mut assign_count = 0u32;
    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            if let StatementKind::Assign(Place::Local(s), _) = &stmt.kind {
                if *s == slot {
                    assign_count += 1;
                    if assign_count > 1 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Check whether an rvalue uses (reads from) a given slot.
fn rvalue_uses_slot(rvalue: &Rvalue, slot: SlotId) -> bool {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => {
            operand_uses_slot(op, slot)
        }
        Rvalue::Borrow(_, place) => place.root_local() == slot,
        Rvalue::BinaryOp(_, lhs, rhs) => {
            operand_uses_slot(lhs, slot) || operand_uses_slot(rhs, slot)
        }
        Rvalue::Aggregate(ops) => ops.iter().any(|op| operand_uses_slot(op, slot)),
    }
}

/// Check whether an operand references a given slot.
fn operand_uses_slot(op: &Operand, slot: SlotId) -> bool {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            place.root_local() == slot
        }
        Operand::Constant(_) => false,
    }
}

/// Run the storage planning pass on a single function.
///
/// The algorithm examines each local slot and decides its storage class:
///
/// 1. If `had_fallbacks` is true, all slots remain `Deferred` (analysis incomplete).
/// 2. For each slot, check closure captures, mutations, aliasing, and loans.
/// 3. Assign the appropriate `BindingStorageClass`.
pub fn plan_storage(input: &StoragePlannerInput<'_>) -> StoragePlan {
    let mut slot_classes = HashMap::new();
    let mut slot_semantics = HashMap::new();

    // If MIR lowering had fallbacks, we cannot trust the analysis.
    // Leave everything Deferred so codegen uses conservative paths.
    if input.had_fallbacks {
        for slot_idx in 0..input.mir.num_locals {
            let slot = SlotId(slot_idx);
            slot_classes.insert(slot, BindingStorageClass::Deferred);
            slot_semantics.insert(
                slot,
                BindingSemantics {
                    ownership_class: BindingOwnershipClass::OwnedImmutable,
                    storage_class: BindingStorageClass::Deferred,
                    aliasability: Aliasability::Unique,
                    mutation_capability: MutationCapability::Immutable,
                    escape_status: EscapeStatus::Local,
                },
            );
        }
        return StoragePlan {
            slot_classes,
            slot_semantics,
        };
    }

    for slot_idx in 0..input.mir.num_locals {
        let slot = SlotId(slot_idx);
        let (storage_class, semantics) = decide_slot_storage(slot, input);
        slot_classes.insert(slot, storage_class);
        slot_semantics.insert(slot, semantics);
    }

    StoragePlan {
        slot_classes,
        slot_semantics,
    }
}

/// Decide the storage class for a single slot, returning both the storage class
/// and enriched binding semantics.
/// Decide the storage class and enriched semantics for a single slot.
///
/// ## Decision matrix
///
/// Priority order (first matching rule wins):
///
/// | # | Condition                                      | Storage class  |
/// |---|------------------------------------------------|----------------|
/// | 0 | Explicit `Reference` already set               | `Reference`    |
/// | 1 | Slot holds a first-class reference              | `Reference`    |
/// | 2 | Captured by closure with mutation               | `UniqueHeap`   |
/// | 3 | `var` (Flexible) + aliased + mutated            | `SharedCow`    |
/// | 3b| Escaped + aliased + mutated (any ownership)     | `SharedCow`    |
/// | 4 | Everything else                                 | `Direct`       |
///
/// Notes:
/// - "Aliased" means either captured by a closure or referenced from multiple
///   MIR places (e.g. through a borrow chain).
/// - `UniqueHeap` and `SharedCow` both result in heap boxing at runtime, but
///   `SharedCow` adds copy-on-write semantics for safe shared mutation.
/// - Immutable closure captures stay `Direct` — the closure gets a plain copy.
fn decide_slot_storage(
    slot: SlotId,
    input: &StoragePlannerInput<'_>,
) -> (BindingStorageClass, BindingSemantics) {
    let is_captured = input.closure_captures.contains(&slot);
    let is_mutably_captured = input.mutable_captures.contains(&slot);
    let _has_loans = slot_has_active_loans(slot, input.analysis);
    let is_mutated = slot_is_mutated(slot, input.mir);
    let is_aliased = slot_is_aliased(slot, input.mir, input.closure_captures);

    // Look up ownership class from binding semantics
    let ownership = input
        .binding_semantics
        .get(&slot.0)
        .map(|s| s.ownership_class);

    // Check if the binding already has an explicit storage class set
    let explicit_storage = input
        .binding_semantics
        .get(&slot.0)
        .map(|s| s.storage_class);

    let is_escaped = detect_escape_status(slot, input.mir, input.closure_captures)
        == EscapeStatus::Escaped;

    let storage_class = if let Some(BindingStorageClass::Reference) = explicit_storage {
        // Already marked as a reference binding — preserve it.
        BindingStorageClass::Reference
    } else if slot_holds_reference(slot, input.mir) {
        // Rule 1: Bindings that hold first-class references.
        BindingStorageClass::Reference
    } else if is_mutably_captured {
        // Rule 2: Captured by closure with mutation → UniqueHeap.
        BindingStorageClass::UniqueHeap
    } else if matches!(ownership, Some(BindingOwnershipClass::Flexible))
        && is_aliased
        && is_mutated
    {
        // Rule 3: `var` bindings that are aliased AND mutated → SharedCow.
        BindingStorageClass::SharedCow
    } else if is_escaped && is_aliased && is_mutated {
        // Rule 3b: Escaped mutable aliased bindings → SharedCow.
        // Even non-Flexible bindings need COW when they escape with aliasing.
        BindingStorageClass::SharedCow
    } else {
        // Rule 4: Captured by closure (immutably) — still Direct.
        // Default: Direct storage (stack slot).
        BindingStorageClass::Direct
    };

    // Compute enriched metadata
    let aliasability = if is_captured || is_aliased {
        if is_mutated {
            Aliasability::SharedMutable
        } else {
            Aliasability::SharedImmutable
        }
    } else {
        Aliasability::Unique
    };

    let mutation_capability = match (ownership, is_mutated) {
        (Some(BindingOwnershipClass::OwnedImmutable), _) => MutationCapability::Immutable,
        (Some(BindingOwnershipClass::OwnedMutable), _) => MutationCapability::LocalMutable,
        (Some(BindingOwnershipClass::Flexible), true) => MutationCapability::SharedMutable,
        (Some(BindingOwnershipClass::Flexible), false) => MutationCapability::Immutable,
        (None, true) => MutationCapability::LocalMutable,
        (None, false) => MutationCapability::Immutable,
    };

    let escape_status = detect_escape_status(slot, input.mir, input.closure_captures);

    let enriched = BindingSemantics {
        ownership_class: ownership.unwrap_or(BindingOwnershipClass::OwnedImmutable),
        storage_class: storage_class,
        aliasability,
        mutation_capability,
        escape_status,
    };

    (storage_class, enriched)
}

/// Detect the escape status of a slot by examining MIR dataflow.
///
/// - `Escaped`: The slot's value flows, directly or through local aliases, into
///   the return slot (`SlotId(0)`).
/// - `Captured`: The slot is captured by a closure.
/// - `Local`: The slot stays within the declaring scope.
pub fn detect_escape_status(
    slot: SlotId,
    mir: &MirFunction,
    closure_captures: &HashSet<SlotId>,
) -> EscapeStatus {
    if slot != SlotId(0) {
        let mut visited = HashSet::new();
        if slot_flows_to_return(slot, mir, &mut visited) {
            return EscapeStatus::Escaped;
        }
    }

    if closure_captures.contains(&slot) {
        EscapeStatus::Captured
    } else {
        EscapeStatus::Local
    }
}

fn slot_flows_to_return(
    slot: SlotId,
    mir: &MirFunction,
    visited: &mut HashSet<SlotId>,
) -> bool {
    if !visited.insert(slot) {
        return false;
    }

    let return_slot = SlotId(0);
    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            let StatementKind::Assign(Place::Local(dest), rvalue) = &stmt.kind else {
                continue;
            };
            if !rvalue_uses_slot(rvalue, slot) {
                continue;
            }
            if *dest == return_slot {
                return true;
            }
            if *dest != slot && slot_flows_to_return(*dest, mir, visited) {
                return true;
            }
        }
    }

    false
}

/// Check if a slot was assigned a `Borrow` rvalue anywhere in the function.
fn slot_holds_reference(slot: SlotId, mir: &MirFunction) -> bool {
    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            if let StatementKind::Assign(Place::Local(s), Rvalue::Borrow(_, _)) = &stmt.kind {
                if *s == slot {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::analysis::BorrowAnalysis;
    use crate::mir::liveness::LivenessResult;
    use crate::mir::types::*;
    use crate::type_tracking::{
        Aliasability, BindingOwnershipClass, BindingSemantics, BindingStorageClass, EscapeStatus,
        MutationCapability,
    };

    fn span() -> shape_ast::ast::Span {
        shape_ast::ast::Span { start: 0, end: 1 }
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

    fn empty_analysis() -> BorrowAnalysis {
        BorrowAnalysis::empty()
    }

    /// Helper: create a simple MIR function with the given blocks.
    fn make_mir(name: &str, blocks: Vec<BasicBlock>, num_locals: u16) -> MirFunction {
        MirFunction {
            name: name.to_string(),
            blocks,
            num_locals,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: (0..num_locals).map(|_| LocalTypeInfo::Unknown).collect(),
            span: span(),
        }
    }

    // ── Test: Direct storage for simple binding ──────────────────────────

    #[test]
    fn test_simple_binding_gets_direct() {
        // bb0: _0 = 42; return
        let mir = make_mir(
            "test_direct",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(0)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                    ),
                    0,
                )],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            1,
        );

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::Direct)
        );
    }

    // ── Test: Deferred when had_fallbacks ─────────────────────────────────

    #[test]
    fn test_fallback_gives_deferred() {
        let mir = make_mir(
            "test_deferred",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            2,
        );

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: true,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::Deferred)
        );
        assert_eq!(
            plan.slot_classes.get(&SlotId(1)),
            Some(&BindingStorageClass::Deferred)
        );
    }

    // ── Test: UniqueHeap for mutably captured slot ────────────────────────

    #[test]
    fn test_mutable_capture_gets_unique_heap() {
        // bb0: _0 = 0; ClosureCapture(copy _0); _0 = 1; return
        let mir = make_mir(
            "test_unique_heap",
            vec![BasicBlock {
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
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(0),
                            operands: vec![Operand::Copy(Place::Local(SlotId(0)))],
                            function_id: None,
                        },
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            1,
        );

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();

        // Simulate what collect_closure_captures would find
        let mut closure_captures = HashSet::new();
        closure_captures.insert(SlotId(0));
        let mut mutable_captures = HashSet::new();
        mutable_captures.insert(SlotId(0));

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::UniqueHeap)
        );
    }

    // ── Test: SharedCow for aliased+mutated var binding ──────────────────

    #[test]
    fn test_aliased_mutated_var_gets_shared_cow() {
        // bb0: _0 = "hello"; _1 = copy _0; _2 = copy _0; _0 = "world"; return
        let mir = make_mir(
            "test_shared_cow",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::StringId(0))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(0)))),
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
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::StringId(1))),
                        ),
                        3,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            3,
        );

        let analysis = empty_analysis();
        let mut binding_semantics = HashMap::new();
        // Mark slot 0 as a `var` (Flexible) binding
        binding_semantics.insert(
            0u16,
            BindingSemantics::deferred(BindingOwnershipClass::Flexible),
        );

        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::SharedCow),
            "aliased + mutated + Flexible => SharedCow"
        );
    }

    // ── Test: Reference for borrow-holding slot ──────────────────────────

    #[test]
    fn test_borrow_holder_gets_reference() {
        // bb0: _0 = 42; _1 = &_0; return
        let mir = make_mir(
            "test_reference",
            vec![BasicBlock {
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
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            2,
        );

        // Create analysis with a loan on slot 0
        let mut analysis = empty_analysis();
        analysis.loans.insert(
            LoanId(0),
            crate::mir::analysis::LoanInfo {
                id: LoanId(0),
                borrowed_place: Place::Local(SlotId(0)),
                kind: BorrowKind::Shared,
                issued_at: Point(1),
                span: span(),
                region_depth: 1,
            },
        );

        let binding_semantics = HashMap::new();
        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        // _1 holds a borrow rvalue → Reference
        assert_eq!(
            plan.slot_classes.get(&SlotId(1)),
            Some(&BindingStorageClass::Reference),
            "_1 holds &_0 borrow → Reference"
        );
    }

    // ── Test: Explicit Reference preserved ───────────────────────────────

    #[test]
    fn test_explicit_reference_preserved() {
        let mir = make_mir(
            "test_explicit_ref",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            1,
        );

        let analysis = empty_analysis();
        let mut binding_semantics = HashMap::new();
        binding_semantics.insert(
            0u16,
            BindingSemantics {
                ownership_class: BindingOwnershipClass::OwnedImmutable,
                storage_class: BindingStorageClass::Reference,
                aliasability: Aliasability::Unique,
                mutation_capability: MutationCapability::Immutable,
                escape_status: EscapeStatus::Local,
            },
        );

        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::Reference),
            "explicit Reference annotation preserved"
        );
    }

    // ── Test: collect_closure_captures ────────────────────────────────────

    #[test]
    fn test_collect_closure_captures() {
        // bb0: _0 = 1; _1 = 2; ClosureCapture(copy _0, copy _1); _0 = 3; return
        let mir = make_mir(
            "test_collect",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(2))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(2),
                            operands: vec![
                                Operand::Copy(Place::Local(SlotId(0))),
                                Operand::Copy(Place::Local(SlotId(1))),
                            ],
                            function_id: None,
                        },
                        2,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(3))),
                        ),
                        3,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            2,
        );

        let (captures, mutable) = collect_closure_captures(&mir);
        assert!(captures.contains(&SlotId(0)));
        assert!(captures.contains(&SlotId(1)));
        // _0 is assigned twice (before and after capture) → mutably captured
        assert!(mutable.contains(&SlotId(0)));
        // _1 is assigned only once (initial definition) → not mutably captured
        // Note: our conservative check counts any assignment, but _1 only has one
        assert!(!mutable.contains(&SlotId(1)));
    }

    // ── Test: Immutable captured slot stays Direct ───────────────────────

    #[test]
    fn test_immutable_capture_stays_direct() {
        // bb0: _0 = 1; ClosureCapture(copy _0); return
        let mir = make_mir(
            "test_immutable_capture",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(0),
                            operands: vec![Operand::Copy(Place::Local(SlotId(0)))],
                            function_id: None,
                        },
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            1,
        );

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let mut closure_captures = HashSet::new();
        closure_captures.insert(SlotId(0));
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::Direct),
            "immutable capture stays Direct"
        );
    }

    // ── Test: Non-Flexible ownership doesn't get SharedCow ───────────────

    #[test]
    fn test_owned_mutable_aliased_mutated_stays_direct() {
        // A `let mut` binding that is aliased and mutated does NOT get
        // SharedCow — only `var` (Flexible) does.
        let mir = make_mir(
            "test_let_mut_no_cow",
            vec![BasicBlock {
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
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(0)))),
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
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(99))),
                        ),
                        3,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            3,
        );

        let analysis = empty_analysis();
        let mut binding_semantics = HashMap::new();
        binding_semantics.insert(
            0u16,
            BindingSemantics::deferred(BindingOwnershipClass::OwnedMutable),
        );

        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::Direct),
            "OwnedMutable (let mut) stays Direct even when aliased+mutated"
        );
    }

    // ── Test: All slots planned ──────────────────────────────────────────

    #[test]
    fn test_all_slots_planned() {
        let mir = make_mir(
            "test_all_planned",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            5,
        );

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(plan.slot_classes.len(), 5, "all slots must be planned");
        for i in 0..5 {
            assert!(
                plan.slot_classes.contains_key(&SlotId(i)),
                "slot {} must be in plan",
                i
            );
        }
    }

    // ── Test: UniqueHeap takes priority over SharedCow ───────────────────

    #[test]
    fn test_mutable_capture_beats_shared_cow() {
        // A `var` binding that is both mutably captured AND aliased+mutated
        // should get UniqueHeap (closure mutation takes priority over COW).
        let mir = make_mir(
            "test_priority",
            vec![BasicBlock {
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
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(0),
                            operands: vec![Operand::Copy(Place::Local(SlotId(0)))],
                            function_id: None,
                        },
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            1,
        );

        let analysis = empty_analysis();
        let mut binding_semantics = HashMap::new();
        binding_semantics.insert(
            0u16,
            BindingSemantics::deferred(BindingOwnershipClass::Flexible),
        );

        let mut closure_captures = HashSet::new();
        closure_captures.insert(SlotId(0));
        let mut mutable_captures = HashSet::new();
        mutable_captures.insert(SlotId(0));

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::UniqueHeap),
            "mutable capture → UniqueHeap overrides SharedCow"
        );
    }

    // ── Test: detect_escape_status ───────────────────────────────────────

    #[test]
    fn test_escape_status_local() {
        // bb0: _1 = 42; return
        // _1 never flows to _0 (return slot) → Local
        let mir = make_mir(
            "test_local_escape",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                    ),
                    0,
                )],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            2,
        );

        let captures = HashSet::new();
        assert_eq!(
            detect_escape_status(SlotId(1), &mir, &captures),
            EscapeStatus::Local,
            "slot that doesn't escape should be Local"
        );
    }

    #[test]
    fn test_escape_status_escaped_via_return() {
        // bb0: _1 = 42; _0 = copy _1; return
        // _1 flows to return slot _0 → Escaped
        let mir = make_mir(
            "test_escaped",
            vec![BasicBlock {
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
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            2,
        );

        let captures = HashSet::new();
        assert_eq!(
            detect_escape_status(SlotId(1), &mir, &captures),
            EscapeStatus::Escaped,
            "slot assigned to return slot should be Escaped"
        );
    }

    #[test]
    fn test_escape_status_escaped_via_local_alias_chain() {
        // bb0: _2 = 42; _1 = copy _2; _0 = copy _1; return
        // _2 reaches the return slot transitively through _1.
        let mir = make_mir(
            "test_transitive_escape",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(2)))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            3,
        );

        let captures = HashSet::new();
        assert_eq!(
            detect_escape_status(SlotId(2), &mir, &captures),
            EscapeStatus::Escaped,
            "slot flowing into a returned local alias should be Escaped"
        );
    }

    #[test]
    fn test_escape_status_captured() {
        // bb0: _1 = 42; ClosureCapture(copy _1); return
        let mir = make_mir(
            "test_captured",
            vec![BasicBlock {
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
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(1),
                            operands: vec![Operand::Copy(Place::Local(SlotId(1)))],
                            function_id: None,
                        },
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            2,
        );

        let mut captures = HashSet::new();
        captures.insert(SlotId(1));
        assert_eq!(
            detect_escape_status(SlotId(1), &mir, &captures),
            EscapeStatus::Captured,
            "slot captured by closure should be Captured"
        );
    }

    #[test]
    fn test_escape_status_escaped_beats_captured() {
        // A slot that both escapes to return AND is captured → Escaped takes priority
        // bb0: _1 = 42; ClosureCapture(copy _1); _0 = copy _1; return
        let mir = make_mir(
            "test_escaped_captured",
            vec![BasicBlock {
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
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(1),
                            operands: vec![Operand::Copy(Place::Local(SlotId(1)))],
                            function_id: None,
                        },
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            2,
        );

        let mut captures = HashSet::new();
        captures.insert(SlotId(1));
        assert_eq!(
            detect_escape_status(SlotId(1), &mir, &captures),
            EscapeStatus::Escaped,
            "Escaped takes priority over Captured"
        );
    }

    #[test]
    fn test_escape_semantics_in_plan() {
        // Verify that the storage plan captures Escaped status on semantics
        // bb0: _1 = 42; _0 = copy _1; return
        let mir = make_mir(
            "test_escape_in_plan",
            vec![BasicBlock {
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
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            2,
        );

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_semantics.get(&SlotId(1)).map(|s| s.escape_status),
            Some(EscapeStatus::Escaped),
            "slot flowing to return should have Escaped status in plan"
        );
    }
}
