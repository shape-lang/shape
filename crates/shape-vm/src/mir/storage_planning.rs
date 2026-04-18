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
use std::sync::OnceLock;

use crate::mir::analysis::{BorrowAnalysis, FunctionBorrowSummary};
use crate::mir::types::*;
use crate::type_tracking::{
    Aliasability, BindingOwnershipClass, BindingSemantics, BindingStorageClass, EscapeStatus,
    MutationCapability,
};

/// Phase V0.a bisect-safety gate; remove once ownership Phase 1 lands.
///
/// When `true` (the default), `var` bindings — i.e. bindings whose ownership
/// class is `BindingOwnershipClass::Flexible` — are assigned
/// `BindingStorageClass::SharedCow` regardless of aliasing/mutation status.
/// This matches the ownership-aware runtime v2 contract: `var` opts into
/// SharedCow aliasing by construction. Flip the flag off (set
/// `SHAPE_V2_VAR_SHAREDCOW=0`) to fall back to the narrower pre-V0.a rule
/// that only promotes to SharedCow when the binding is both aliased AND
/// mutated. This is a temporary bisect safety net; once the rest of ownership
/// Phase 1 lands, the flag (and this helper) can be deleted.
///
/// The env var is read once per process and cached in a `OnceLock`. Values
/// `0`/`false`/`off`/`no` (case-insensitive, trimmed) or an empty string
/// disable the flag; anything else (or the env var being unset) leaves the
/// default `true`.
fn var_sharedcow_default_enabled() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| match std::env::var("SHAPE_V2_VAR_SHAREDCOW") {
        // Explicit opt-out values disable the flag.
        Ok(v) => !matches!(
            v.trim(),
            "0" | "false" | "FALSE" | "False" | "off" | "OFF" | "Off" | "no" | "NO" | "No" | ""
        ),
        // Not set → default on.
        Err(_) => true,
    })
}

/// Maximum element count for a non-escaping aggregate to be eligible for the
/// stack-allocated / inline optimization hint. Arrays larger than this are
/// never flagged, even if they don't escape. The threshold matches the
/// SmallVec inline-capacity target discussed in the Phase D design notes
/// (8 elements × 8 bytes = 64 B, one cache line).
pub const INLINE_ARRAY_MAX_ELEMENTS: usize = 8;

/// The computed storage plan for a single function.
#[derive(Debug, Clone)]
pub struct StoragePlan {
    /// Maps each local slot to its decided storage class.
    pub slot_classes: HashMap<SlotId, BindingStorageClass>,
    /// Maps each local slot to its enriched binding semantics.
    pub slot_semantics: HashMap<SlotId, BindingSemantics>,
    /// Optimization hint (Phase D.2 infrastructure): slots that hold a small,
    /// compile-time-sized aggregate (array/tuple literal) that is provably
    /// non-escaping, non-captured, and non-aliased. The value is the element
    /// count.
    ///
    /// Consumers may choose to:
    ///   * emit inline-on-stack storage (SROA — eliminate the allocation),
    ///   * pick a SmallVec-backed `HeapValue::Array` path, or
    ///   * ignore the hint entirely (semantics-preserving).
    ///
    /// Today no consumer acts on the hint; it is recorded so that future
    /// codegen passes (inline arrays, scalar replacement of aggregates) can
    /// activate without re-running escape analysis.
    pub inline_array_sizes: HashMap<SlotId, usize>,
    /// Closure Spec Phase B: slots that hold closure values (defined by a
    /// `ClosureCapture` statement) and that provably do not escape their
    /// enclosing function via any of the §2.1 escape vectors
    /// (return, container store, field/index/deref write, capture by an
    /// escaping closure, task boundary, call-site argument, or
    /// `UniqueHeap`/`SharedCow` promotion).
    ///
    /// Consumed by Phase C's per-closure specialization decision: when a
    /// closure slot is in this set, the specialization may stack-allocate the
    /// closure and inline its body into the receiving specialization instead
    /// of heap-allocating a `TypedClosure`. Slots NOT in this set are
    /// considered escaping and must use the heap variant.
    ///
    /// Today no consumer acts on the set (Phase C is not landed). It is
    /// recorded so the specialization pass can activate without re-running
    /// escape analysis.
    pub non_escaping_closure_slots: HashSet<SlotId>,
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
    /// Previously-computed callee borrow summaries, keyed by function name.
    /// Used by the closure-escape analysis (Closure Spec Phase B) to check
    /// whether a closure passed as a call-site argument may escape into the
    /// callee. `None` (or missing entries) is treated conservatively — the
    /// closure is assumed to escape. Populated from
    /// `Compiler::function_borrow_summaries`.
    pub callee_summaries: Option<&'a HashMap<String, FunctionBorrowSummary>>,
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
                    return_ownership_hint: None,
                },
            );
        }
        return StoragePlan {
            slot_classes,
            slot_semantics,
            inline_array_sizes: HashMap::new(),
            non_escaping_closure_slots: HashSet::new(),
        };
    }

    let var_sharedcow_enabled = var_sharedcow_default_enabled();
    for slot_idx in 0..input.mir.num_locals {
        let slot = SlotId(slot_idx);
        let (storage_class, semantics) =
            decide_slot_storage(slot, input, var_sharedcow_enabled);
        slot_classes.insert(slot, storage_class);
        slot_semantics.insert(slot, semantics);
    }

    // Detect small non-escaping aggregates and record the hint. This is
    // gathered after slot decisions so we can consult `slot_semantics` for the
    // authoritative escape/aliasing verdict.
    let inline_array_sizes = detect_inline_array_candidates(input, &slot_semantics);

    // Closure Spec Phase B: classify every closure slot (each slot defined by
    // a `ClosureCapture` statement) as escaping or non-escaping, using the
    // full §2.1 escape-vector table plus a fixed-point over transitive
    // closure captures.
    let non_escaping_closure_slots =
        detect_non_escaping_closure_slots(input, &slot_classes);

    // Closure Spec Phase D: promote the storage class of outer slots whose
    // ONLY heap-indirection driver is a mutable capture by a non-escaping
    // closure to `LocalMutablePtr`. The previous pass assigned `UniqueHeap`
    // to such slots (Rule 2 in `decide_slot_storage`) — that was the v1-style
    // "box into SharedCell on capture" decision. In v2 the closure env holds
    // a typed stack pointer instead; the binding can stay on the frame.
    //
    // Preconditions (all must hold):
    //   1. The slot is the root of a mutably-captured operand in a
    //      `ClosureCapture` statement.
    //   2. The closure slot that captures it is in
    //      `non_escaping_closure_slots`.
    //   3. The slot is not itself a first-class reference or promoted to a
    //      heap-aliased class for some other reason (e.g. also stored into
    //      a collection).
    //
    // Escape fallback (§4.3): for `let mut` + escaping closure with a mutable
    // capture, `compile_expr_closure` emits `B0003`. For `var` + escaping
    // closure, the SharedCow path is unchanged.
    promote_local_mutable_ptr_slots(
        input,
        &non_escaping_closure_slots,
        &mut slot_classes,
        &mut slot_semantics,
    );

    StoragePlan {
        slot_classes,
        slot_semantics,
        inline_array_sizes,
        non_escaping_closure_slots,
    }
}

/// Phase D — promote outer-slot storage class to `LocalMutablePtr` when the
/// slot is captured by a non-escaping closure.
///
/// This runs AFTER the main storage-class decision pass and AFTER the
/// `non_escaping_closure_slots` computation. It walks every `ClosureCapture`
/// in the MIR; for each operand whose root slot currently has a conservative
/// class (`Direct` / `Deferred` / `UniqueHeap`) it promotes to
/// `LocalMutablePtr`. The MIR statement does not distinguish
/// mutable-vs-immutable operands — the closure-compiler side inspects the
/// closure body via `EnvironmentAnalyzer::analyze_function_with_mutability`
/// and only emits `LoadCaptureMutPtr<T>` / `StoreCaptureMutPtr<T>` for
/// captures that the body actually mutates. Unused promotions are harmless:
/// `LocalMutablePtr` carries the same "slot stays on the frame" meaning as
/// `Direct` and reads through the existing non-closure code paths unchanged.
///
/// Conservative: if the outer slot has any other escape vector (it is also
/// returned, stored in a container, or captured by a different escaping
/// closure), `decide_slot_storage` already put it in a non-promotable class
/// (e.g. `SharedCow`) and we leave it alone.
fn promote_local_mutable_ptr_slots(
    input: &StoragePlannerInput<'_>,
    non_escaping_closure_slots: &HashSet<SlotId>,
    slot_classes: &mut HashMap<SlotId, BindingStorageClass>,
    slot_semantics: &mut HashMap<SlotId, BindingSemantics>,
) {
    for block in input.mir.iter_blocks() {
        for stmt in &block.statements {
            let StatementKind::ClosureCapture {
                closure_slot,
                operands,
                ..
            } = &stmt.kind
            else {
                continue;
            };

            if !non_escaping_closure_slots.contains(closure_slot) {
                continue;
            }

            for op in operands {
                let Some(root) = operand_root_slot(op) else {
                    continue;
                };
                let Some(current) = slot_classes.get(&root).copied() else {
                    continue;
                };
                match current {
                    BindingStorageClass::UniqueHeap
                    | BindingStorageClass::Direct
                    | BindingStorageClass::Deferred => {
                        slot_classes.insert(root, BindingStorageClass::LocalMutablePtr);
                        if let Some(sem) = slot_semantics.get_mut(&root) {
                            sem.storage_class = BindingStorageClass::LocalMutablePtr;
                        }
                    }
                    // SharedCow / Reference / LocalMutablePtr: leave alone.
                    _ => {}
                }
            }
        }
    }
}

/// Scan MIR for aggregate assignments that are safe to inline on the stack.
///
/// A slot is a candidate when **all** of the following hold:
/// - It is assigned exactly one `Rvalue::Aggregate` with `N <= INLINE_ARRAY_MAX_ELEMENTS`.
/// - Its computed `escape_status` is `Local` (does not flow to the return slot).
/// - It is not captured by any closure.
/// - It is not re-assigned after the aggregate initialization (otherwise the
///   slot can't be treated as a fixed-shape value).
/// - It is not mutated through index assignment (future work: inline arrays
///   with index writes would need per-element tracking).
///
/// The hint is purely advisory — emitting it does not commit to any specific
/// codegen strategy. Zero-size aggregates are also skipped since they add no
/// allocation pressure to begin with.
fn detect_inline_array_candidates(
    input: &StoragePlannerInput<'_>,
    slot_semantics: &HashMap<SlotId, BindingSemantics>,
) -> HashMap<SlotId, usize> {
    let mut aggregate_sizes: HashMap<SlotId, usize> = HashMap::new();
    let mut disqualified: HashSet<SlotId> = HashSet::new();

    for block in input.mir.iter_blocks() {
        for stmt in &block.statements {
            let StatementKind::Assign(place, rvalue) = &stmt.kind else {
                continue;
            };
            let Place::Local(slot) = place else {
                // A projection write (e.g. `arr[i] = x`) into a candidate
                // slot disqualifies it — the aggregate shape is mutable.
                disqualified.insert(place.root_local());
                continue;
            };

            match rvalue {
                Rvalue::Aggregate(ops) => {
                    // Only the first aggregate assignment counts. A later
                    // reassignment (even another aggregate) means the slot
                    // isn't a fixed-shape value.
                    if aggregate_sizes.contains_key(slot) {
                        disqualified.insert(*slot);
                    } else {
                        aggregate_sizes.insert(*slot, ops.len());
                    }
                }
                _ => {
                    // Any non-aggregate assignment (re-binding, function
                    // result, etc.) after the aggregate disqualifies the slot.
                    if aggregate_sizes.contains_key(slot) {
                        disqualified.insert(*slot);
                    }
                }
            }
        }
    }

    let mut hints = HashMap::new();
    for (slot, size) in aggregate_sizes {
        if disqualified.contains(&slot) {
            continue;
        }
        if size == 0 || size > INLINE_ARRAY_MAX_ELEMENTS {
            continue;
        }
        let Some(sem) = slot_semantics.get(&slot) else {
            continue;
        };
        if sem.escape_status != EscapeStatus::Local {
            continue;
        }
        // Be defensive: any aliasing (shared-immutable or shared-mutable)
        // rules out the SROA-style optimization, even though the aggregate
        // itself would remain addressable on the stack.
        if sem.aliasability != Aliasability::Unique {
            continue;
        }
        if input.closure_captures.contains(&slot) {
            continue;
        }
        hints.insert(slot, size);
    }
    hints
}

// ── Closure Spec Phase B: non-escape detection for closure slots ────────────
//
// The analysis implements the 10-row escape-vector table from
// `docs/v2-closure-specialization.md` §2.1. Its output is the set of closure
// slots that are provably non-escaping, recorded on `StoragePlan` for the
// Phase C specialization pass to consume.
//
// Soundness discipline: when an escape vector cannot be decided (missing
// callee summary, unknown operand shape), the slot is marked as escaping.
// Any precision lost here is recoverable by Phase C's monomorphization work;
// the cost of a false "non-escaping" verdict is a stack-pointer outliving a
// frame, which is unacceptable.

/// Collect every slot that is defined as a closure value (the destination of
/// a `ClosureCapture` statement). The `closure_slot` field of a
/// `ClosureCapture` identifies the slot that will hold the closure object
/// once the follow-up `Assign(..., ClosurePlaceholder)` completes the
/// definition. Functions with no closure literals produce an empty set.
fn collect_closure_slots(mir: &MirFunction) -> HashSet<SlotId> {
    let mut slots = HashSet::new();
    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            if let StatementKind::ClosureCapture { closure_slot, .. } = &stmt.kind {
                slots.insert(*closure_slot);
            }
        }
    }
    slots
}

/// Build the transitive closure-capture graph: if closure A captures closure
/// B, the map records `A -> {B}`. Used for the §2.4 fixed-point — if A
/// escapes, B escapes too because it is carried inside A's capture layout.
fn build_closure_capture_graph(
    mir: &MirFunction,
    closure_slots: &HashSet<SlotId>,
) -> HashMap<SlotId, HashSet<SlotId>> {
    let mut graph: HashMap<SlotId, HashSet<SlotId>> = HashMap::new();
    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            if let StatementKind::ClosureCapture {
                closure_slot,
                operands,
                ..
            } = &stmt.kind
            {
                for op in operands {
                    let Some(root) = operand_root_slot(op) else {
                        continue;
                    };
                    if closure_slots.contains(&root) {
                        graph
                            .entry(*closure_slot)
                            .or_default()
                            .insert(root);
                    }
                }
            }
        }
    }
    graph
}

/// §2.1 direct-vector escape check for a single closure slot `c`.
///
/// Walks the MIR once per call. Returns `true` if any of rows 1-3, 5-7, 9-10
/// fire; row 4 (transitive capture) and row 8 (call-argument through callee
/// summary) are handled by `detect_non_escaping_closure_slots` after direct
/// vectors are classified.
///
/// The walk uses a single-pass monotonic scan: starting from the singleton
/// tracked set `{c}`, each `Assign(Place::Local(dest), Rvalue::Use(Copy/Move
/// of a tracked slot))` widens the tracked set to include `dest`. This mirrors
/// the existing `slot_flows_to_return` pattern but generalized to every sink
/// the table names, without requiring a separate dataflow pass.
fn closure_slot_escapes_direct(
    c: SlotId,
    input: &StoragePlannerInput<'_>,
) -> bool {
    let mir = input.mir;

    // Row 10: the semantics planner promoted the slot to heap-aliased
    // storage. Any such promotion escapes by construction.
    if let Some(sem) = input.binding_semantics.get(&c.0) {
        match sem.storage_class {
            BindingStorageClass::UniqueHeap | BindingStorageClass::SharedCow => {
                return true;
            }
            _ => {}
        }
    }

    // Iteratively widen the tracked set as the closure flows through local
    // aliases. Each pass also evaluates every escape vector against the
    // current tracked set — reaching any escape sink is a definite "yes".
    let mut tracked: HashSet<SlotId> = HashSet::new();
    tracked.insert(c);

    let mut changed = true;
    while changed {
        changed = false;

        for block in mir.iter_blocks() {
            for stmt in &block.statements {
                match &stmt.kind {
                    // Row 1 (return), Row 2 (Aggregate), Row 3 (struct
                    // field), Row 9 (deref), and local-alias propagation.
                    StatementKind::Assign(place, rvalue) => {
                        let reads_tracked = rvalue_uses_any_slot(rvalue, &tracked);
                        match place {
                            Place::Local(dest) => {
                                // Row 1: dest is the return slot.
                                if *dest == SlotId(0) && reads_tracked {
                                    return true;
                                }
                                // Local-alias propagation: widen the tracked
                                // set so downstream vectors see the alias.
                                if reads_tracked && tracked.insert(*dest) {
                                    changed = true;
                                }
                            }
                            // Row 3: struct-field write `foo.f = c`.
                            Place::Field(..) => {
                                if reads_tracked {
                                    return true;
                                }
                            }
                            // Row 2 subcase / Row 9 variant: index/deref
                            // writes both materialize the value into some
                            // other container.
                            Place::Index(..) | Place::Deref(..) => {
                                if reads_tracked {
                                    return true;
                                }
                            }
                        }

                        // Row 2: `Rvalue::Aggregate` feeding any slot — the
                        // aggregate itself is the escaping container.
                        if let Rvalue::Aggregate(ops) = rvalue {
                            if ops.iter().any(|op| operand_uses_any_slot(op, &tracked)) {
                                return true;
                            }
                        }
                    }
                    // Row 2: store into array / object / enum literal.
                    StatementKind::ArrayStore { operands, .. }
                    | StatementKind::ObjectStore { operands, .. }
                    | StatementKind::EnumStore { operands, .. } => {
                        if operands.iter().any(|op| operand_uses_any_slot(op, &tracked)) {
                            return true;
                        }
                    }
                    // Rows 5 + 6: detached or structured task boundary.
                    // §5.5 allows stack closures across structured boundaries
                    // in the future, but Phase B takes the conservative
                    // verdict — any boundary is escape.
                    StatementKind::TaskBoundary(operands, _) => {
                        if operands.iter().any(|op| operand_uses_any_slot(op, &tracked)) {
                            return true;
                        }
                    }
                    // Row 4 direct contribution: if we find a closure
                    // capturing a tracked slot and that enclosing closure
                    // later turns out to escape, the tracked slot escapes
                    // transitively. Handled by the fixed-point in
                    // `detect_non_escaping_closure_slots`.
                    StatementKind::ClosureCapture { .. }
                    | StatementKind::Drop(_)
                    | StatementKind::Nop => {}
                }
            }

            // Row 8: call-site argument. Conservative default — any call
            // with a tracked slot in its args escapes, unless the callee is a
            // statically-known function whose `FunctionBorrowSummary` marks
            // the corresponding param as non-escaping.
            if let TerminatorKind::Call { func, args, .. } = &block.terminator.kind {
                // Row 7: `snapshot()` is an opaque FFI in v1 — there is no
                // dedicated MIR terminator, so any named call to `snapshot`
                // must be treated as escape. The full treatment lives in §9
                // open question #7.
                let callee_name = match func {
                    Operand::Constant(MirConstant::Function(name)) => Some(name.as_str()),
                    _ => None,
                };
                if callee_name == Some("snapshot") {
                    if args.iter().any(|op| operand_uses_any_slot(op, &tracked)) {
                        return true;
                    }
                }

                let callee_summary = callee_name
                    .and_then(|n| input.callee_summaries.and_then(|m| m.get(n)));

                for (arg_idx, arg) in args.iter().enumerate() {
                    if !operand_uses_any_slot(arg, &tracked) {
                        continue;
                    }
                    // With a summary + a matching param slot, we can trust
                    // the callee's per-param escape bit. Otherwise: escape.
                    match callee_summary {
                        Some(summary) if arg_idx < summary.closure_param_escapes.len() => {
                            if summary.closure_param_escapes[arg_idx] {
                                return true;
                            }
                            // else: callee is transparent for this arg;
                            // continue checking other vectors.
                        }
                        _ => return true,
                    }
                }

                // Indirect call target itself: if the call *target* is the
                // tracked slot, we treat it as non-escape (the closure is
                // invoked in-place). Not a separate vector.
                let _ = func; // explicitly documenting intent
            }
        }
    }

    false
}

/// §2.4 fixed-point over the closure capture graph: if closure A escapes,
/// every closure it captures (B in `graph[A]`) also escapes. The worklist
/// iterates until the escaping set stops growing — monotone, terminates in
/// at most `|closure_slots|` passes.
fn propagate_transitive_closure_escape(
    closure_slots: &HashSet<SlotId>,
    capture_graph: &HashMap<SlotId, HashSet<SlotId>>,
    escaping: &mut HashSet<SlotId>,
) {
    let mut changed = true;
    while changed {
        changed = false;
        for &a in closure_slots {
            if !escaping.contains(&a) {
                continue;
            }
            let Some(captured_closures) = capture_graph.get(&a) else {
                continue;
            };
            for &b in captured_closures {
                if escaping.insert(b) {
                    changed = true;
                }
            }
        }
    }
}

/// Does the MIR contain any named call to `snapshot`, regardless of args?
///
/// Closure spec Phase G §5.6: `snapshot()` captures the full VM state into
/// `SerializableCallFrame` via the interpreter's locals array. Stack-allocated
/// closures live in Cranelift StackSlots and are invisible to that
/// serializer, so any function that may invoke `snapshot()` must not
/// stack-allocate its closures — independent of whether a specific closure
/// flows into `snapshot()`'s argument list. The spec's "forces deopt first"
/// policy reduces to "promote every closure in the function to heap." Phase G
/// implements the conservative half (§5.6 short-circuit); the precise
/// per-closure deopt-with-rematerialization path is future work once MIR
/// gains a first-class `TerminatorKind::Snapshot` (§9 open question #7).
fn mir_contains_snapshot_call(mir: &MirFunction) -> bool {
    for block in mir.iter_blocks() {
        if let TerminatorKind::Call { func, .. } = &block.terminator.kind {
            if let Operand::Constant(MirConstant::Function(name)) = func {
                if name == "snapshot" {
                    return true;
                }
            }
        }
    }
    false
}

/// Populate `StoragePlan.non_escaping_closure_slots` for the function under
/// analysis. Orchestrates: (1) collect closure slots, (2) classify each via
/// §2.1 direct vectors, (3) run the §2.4 transitive-capture fixed-point, (4)
/// invert — any closure slot NOT in the escaping set is non-escaping.
fn detect_non_escaping_closure_slots(
    input: &StoragePlannerInput<'_>,
    _slot_classes: &HashMap<SlotId, BindingStorageClass>,
) -> HashSet<SlotId> {
    let closure_slots = collect_closure_slots(input.mir);
    if closure_slots.is_empty() {
        return HashSet::new();
    }

    // Closure spec Phase G §5.6: function-wide `snapshot()` escape.
    // If any terminator in the function calls `snapshot`, force every
    // closure slot to heap — stack closures cannot survive the VM-state
    // serialization path that snapshot takes. This is the conservative
    // complement to Phase G's JIT-side "snapshot forces deopt first"
    // policy: the MIR planner takes the easy win (heap-allocate every
    // closure in a snapshottable function) so we do not need to emit a
    // JIT deopt guard for every snapshot-reachable stack slot.
    if mir_contains_snapshot_call(input.mir) {
        // All closure slots escape; return the empty "non-escaping" set.
        return HashSet::new();
    }

    // Step 1: direct-vector escape verdict per closure slot.
    let mut escaping: HashSet<SlotId> = HashSet::new();
    for &c in &closure_slots {
        if closure_slot_escapes_direct(c, input) {
            escaping.insert(c);
        }
    }

    // Step 2: transitive propagation via the capture graph.
    let capture_graph = build_closure_capture_graph(input.mir, &closure_slots);
    propagate_transitive_closure_escape(&closure_slots, &capture_graph, &mut escaping);

    // Step 3: invert — slots not in `escaping` are non-escaping.
    closure_slots
        .difference(&escaping)
        .copied()
        .collect()
}

/// Does `rvalue` read from any slot in `slots`? Covers all rvalue shapes.
fn rvalue_uses_any_slot(rvalue: &Rvalue, slots: &HashSet<SlotId>) -> bool {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => {
            operand_uses_any_slot(op, slots)
        }
        Rvalue::Borrow(_, place) => slots.contains(&place.root_local()),
        Rvalue::BinaryOp(_, lhs, rhs) => {
            operand_uses_any_slot(lhs, slots) || operand_uses_any_slot(rhs, slots)
        }
        Rvalue::Aggregate(ops) => ops.iter().any(|op| operand_uses_any_slot(op, slots)),
    }
}

/// Does `op` reference a slot in `slots`?
fn operand_uses_any_slot(op: &Operand, slots: &HashSet<SlotId>) -> bool {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            slots.contains(&place.root_local())
        }
        Operand::Constant(_) => false,
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
/// | #  | Condition                                      | Storage class  |
/// |----|------------------------------------------------|----------------|
/// | 0  | Explicit `Reference` already set               | `Reference`    |
/// | 1  | Slot holds a first-class reference              | `Reference`    |
/// | 1b | **V0.a**: `var` (Flexible) + flag on           | `SharedCow`    |
/// | 2  | Captured by closure with mutation               | `UniqueHeap`   |
/// | 3  | `var` (Flexible) + aliased + mutated            | `SharedCow`    |
/// | 3b | Escaped + aliased + mutated (any ownership)     | `SharedCow`    |
/// | 4  | Everything else                                 | `Direct`       |
///
/// Rule 1b (Phase V0.a): when `SHAPE_V2_VAR_SHAREDCOW` is enabled (the
/// default), every `var` binding is promoted to `SharedCow` regardless of
/// aliasing/mutation. The legacy Rule 3 remains as a safety net and as the
/// behavior observed when the flag is disabled. Rule 1b runs BEFORE Rule 2
/// so that a `var` slot that happens to be mutably captured stays `SharedCow`
/// rather than being routed through the `UniqueHeap` → `LocalMutablePtr`
/// promotion: the spec's Phase 4 contract is that `var` is aliased by
/// construction.
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
    var_sharedcow_enabled: bool,
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
    } else if matches!(ownership, Some(BindingOwnershipClass::Flexible)) && var_sharedcow_enabled {
        // Rule 1b (Phase V0.a): `var` bindings are SharedCow by default when
        // the `SHAPE_V2_VAR_SHAREDCOW` flag is on. `var`'s ownership class is
        // `Flexible`, which the spec's Phase 4 defines as "aliased by
        // construction" — SharedCow regardless of whether the MIR pass
        // observed an actual alias+mutation pair. The flag defaults to on;
        // setting the env var to `0` falls back to the narrower Rule 3.
        BindingStorageClass::SharedCow
    } else if is_mutably_captured {
        // Rule 2: Captured by closure with mutation → UniqueHeap.
        BindingStorageClass::UniqueHeap
    } else if matches!(ownership, Some(BindingOwnershipClass::Flexible))
        && is_aliased
        && is_mutated
    {
        // Rule 3: `var` bindings that are aliased AND mutated → SharedCow.
        // Still present as a safety net: when the V0.a flag is disabled the
        // planner falls back to this narrower rule.
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

    // Preserve the Phase 5.B return-ownership hint through storage planning:
    // it was populated on the incoming binding semantics by the compiler's
    // let-statement path and is consumed at codegen time.
    let return_ownership_hint = input
        .binding_semantics
        .get(&slot.0)
        .and_then(|s| s.return_ownership_hint);

    let enriched = BindingSemantics {
        ownership_class: ownership.unwrap_or(BindingOwnershipClass::OwnedImmutable),
        storage_class: storage_class,
        aliasability,
        mutation_capability,
        escape_status,
        return_ownership_hint,
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
            field_name_table: std::collections::HashMap::new(),
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
            callee_summaries: None,
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
            callee_summaries: None,
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

    // ── Test: Mutable capture → LocalMutablePtr (Phase D) or UniqueHeap ───
    //
    // Before Phase D this scenario produced `UniqueHeap` unconditionally
    // (Rule 2 in `decide_slot_storage`). Phase D adds a post-processing pass:
    // when the closure itself is non-escaping, the outer slot is demoted to
    // `LocalMutablePtr` so the slot can stay on the stack and the closure env
    // holds a typed pointer instead of an `Arc<RwLock<ValueWord>>` cell.

    #[test]
    fn test_mutable_capture_gets_local_mutable_ptr() {
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        // Phase D: when the closure is non-escaping, mutable-capture slots
        // drop from `UniqueHeap` to `LocalMutablePtr`. The test MIR has the
        // closure_slot at SlotId(0) and no escape vectors, so the closure is
        // non-escaping and the demotion fires.
        assert_eq!(
            plan.slot_classes.get(&SlotId(0)),
            Some(&BindingStorageClass::LocalMutablePtr)
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
            callee_summaries: None,
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
            callee_summaries: None,
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
                return_ownership_hint: None,
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
            callee_summaries: None,
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        // Phase D: when a non-escaping closure captures the slot, the planner
        // promotes `Direct` → `LocalMutablePtr` unconditionally (the compiler
        // side decides whether to actually emit the typed-pointer opcodes
        // based on the closure body's mutation analysis). `LocalMutablePtr`
        // is semantically equivalent to `Direct` for immutable captures:
        // the slot stays on the frame and no heap indirection is introduced.
        let class = plan.slot_classes.get(&SlotId(0)).copied();
        assert!(
            matches!(
                class,
                Some(BindingStorageClass::Direct)
                    | Some(BindingStorageClass::LocalMutablePtr)
            ),
            "immutable capture stays on stack (Direct or LocalMutablePtr), got {:?}",
            class
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
            callee_summaries: None,
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
            callee_summaries: None,
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
            callee_summaries: None,
        };

        // Phase V0.a changes the storage class depending on the
        // `SHAPE_V2_VAR_SHAREDCOW` flag state observed at process start:
        //
        //   * Flag ON (default): Rule 1b fires first and promotes the `var`
        //     slot to `SharedCow`; Phase D's promotion leaves `SharedCow`
        //     untouched.
        //   * Flag OFF: Rule 1b is suppressed; Rule 2 fires → `UniqueHeap`;
        //     Phase D then demotes to `LocalMutablePtr` because the closure
        //     is non-escaping.
        //
        // `plan_storage` consults the cached env-backed flag, so we accept
        // whichever outcome matches the process's flag state and cross-check
        // the other outcome via `decide_slot_storage` directly.
        let plan = plan_storage(&input);
        let actual = plan.slot_classes.get(&SlotId(0)).copied();
        if var_sharedcow_default_enabled() {
            assert_eq!(
                actual,
                Some(BindingStorageClass::SharedCow),
                "Phase V0.a flag on: mutably-captured `var` is SharedCow"
            );
            // Cross-check: with the flag forced off, the legacy
            // UniqueHeap → Phase-D-demote path is still exercised.
            assert_eq!(
                decide_slot_storage(SlotId(0), &input, /* var_sharedcow_enabled */ false).0,
                BindingStorageClass::UniqueHeap,
                "legacy path (flag off) still assigns UniqueHeap before Phase D demotion"
            );
        } else {
            assert_eq!(
                actual,
                Some(BindingStorageClass::LocalMutablePtr),
                "Phase V0.a flag off: non-escaping mutable capture → LocalMutablePtr (Phase D)"
            );
        }
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_semantics.get(&SlotId(1)).map(|s| s.escape_status),
            Some(EscapeStatus::Escaped),
            "slot flowing to return should have Escaped status in plan"
        );
    }

    // ── Tests: inline-array-candidate detection (Phase D.2 infrastructure) ──

    #[test]
    fn test_inline_hint_for_small_local_aggregate() {
        // bb0: _1 = [1, 2, 3]; return
        // _1 is a 3-element aggregate that never flows to _0 → candidate.
        let mir = make_mir(
            "test_inline_small_local",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Aggregate(vec![
                            Operand::Constant(MirConstant::Int(1)),
                            Operand::Constant(MirConstant::Int(2)),
                            Operand::Constant(MirConstant::Int(3)),
                        ]),
                    ),
                    0,
                )],
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.inline_array_sizes.get(&SlotId(1)),
            Some(&3),
            "3-element non-escaping aggregate should be hinted"
        );
    }

    #[test]
    fn test_no_inline_hint_when_aggregate_escapes() {
        // bb0: _1 = [1, 2]; _0 = copy _1; return
        // _1 flows to return → escapes, so no hint.
        let mir = make_mir(
            "test_no_inline_escape",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Aggregate(vec![
                                Operand::Constant(MirConstant::Int(1)),
                                Operand::Constant(MirConstant::Int(2)),
                            ]),
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        assert!(
            !plan.inline_array_sizes.contains_key(&SlotId(1)),
            "escaping aggregate must not be hinted"
        );
    }

    #[test]
    fn test_no_inline_hint_when_aggregate_too_large() {
        // bb0: _1 = [1; 9]; return
        // 9 > INLINE_ARRAY_MAX_ELEMENTS (8) → no hint.
        let big_ops: Vec<Operand> = (0..9)
            .map(|_| Operand::Constant(MirConstant::Int(0)))
            .collect();
        let mir = make_mir(
            "test_no_inline_too_large",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Aggregate(big_ops),
                    ),
                    0,
                )],
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        assert!(
            !plan.inline_array_sizes.contains_key(&SlotId(1)),
            "oversize aggregate must not be hinted"
        );
    }

    #[test]
    fn test_no_inline_hint_when_aggregate_captured() {
        // bb0: _1 = [1, 2]; ClosureCapture(copy _1); return
        let mir = make_mir(
            "test_no_inline_captured",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Aggregate(vec![
                                Operand::Constant(MirConstant::Int(1)),
                                Operand::Constant(MirConstant::Int(2)),
                            ]),
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

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let mut closure_captures = HashSet::new();
        closure_captures.insert(SlotId(1));
        let mutable_captures = HashSet::new();

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        assert!(
            !plan.inline_array_sizes.contains_key(&SlotId(1)),
            "captured aggregate must not be hinted"
        );
    }

    #[test]
    fn test_no_inline_hint_when_reassigned() {
        // bb0: _1 = [1, 2]; _1 = [3, 4]; return
        // Two aggregate assignments to the same slot → disqualify.
        let mir = make_mir(
            "test_no_inline_reassigned",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Aggregate(vec![
                                Operand::Constant(MirConstant::Int(1)),
                                Operand::Constant(MirConstant::Int(2)),
                            ]),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Aggregate(vec![
                                Operand::Constant(MirConstant::Int(3)),
                                Operand::Constant(MirConstant::Int(4)),
                            ]),
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        assert!(
            !plan.inline_array_sizes.contains_key(&SlotId(1)),
            "re-assigned slot must not be hinted"
        );
    }

    #[test]
    fn test_inline_hint_at_boundary_size() {
        // Exactly INLINE_ARRAY_MAX_ELEMENTS elements → eligible.
        let ops: Vec<Operand> = (0..INLINE_ARRAY_MAX_ELEMENTS)
            .map(|_| Operand::Constant(MirConstant::Int(0)))
            .collect();
        let mir = make_mir(
            "test_inline_boundary",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Aggregate(ops),
                    ),
                    0,
                )],
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        assert_eq!(
            plan.inline_array_sizes.get(&SlotId(1)),
            Some(&INLINE_ARRAY_MAX_ELEMENTS),
            "boundary-size aggregate should be hinted"
        );
    }

    #[test]
    fn test_no_inline_hint_with_fallbacks() {
        // had_fallbacks=true → planner bails to Deferred and records no hints.
        let mir = make_mir(
            "test_no_inline_fallback",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Aggregate(vec![
                            Operand::Constant(MirConstant::Int(1)),
                        ]),
                    ),
                    0,
                )],
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
            callee_summaries: None,
        };

        let plan = plan_storage(&input);
        assert!(
            plan.inline_array_sizes.is_empty(),
            "fallback path must not record any hints"
        );
    }

    // ── Closure Spec Phase B: non-escape detection for closure slots ────────
    //
    // These tests exercise the 10-row escape-vector table from
    // `docs/v2-closure-specialization.md` §2.1 plus the §2.4 transitive
    // fixed-point. They operate directly on synthetic MIR so they can target
    // specific vectors without relying on the higher-level compiler pipeline.
    //
    // Convention: closure slot := the `closure_slot` field of a
    // `StatementKind::ClosureCapture`. A followup
    // `Assign(_, ClosurePlaceholder)` would complete the definition in real
    // MIR; the tests omit it because `collect_closure_slots` keys solely on
    // the `ClosureCapture` statement.
    //
    // Helper: build MIR with a single block + terminator.
    fn single_block_mir(name: &str, statements: Vec<MirStatement>, num_locals: u16) -> MirFunction {
        make_mir(
            name,
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements,
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals,
        )
    }

    fn run_planner(mir: &MirFunction) -> StoragePlan {
        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();
        let input = StoragePlannerInput {
            mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
            callee_summaries: None,
        };
        plan_storage(&input)
    }

    #[test]
    fn test_phase_b_pure_local_closure_is_non_escaping() {
        // let f = || 1; f()  — the closure slot is defined via
        // ClosureCapture (no captures) and then only invoked. No escape
        // vector fires.
        // Slots: _0 = return, _1 = closure, _2 = call result
        let mir = single_block_mir(
            "phase_b_pure_local",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "pure let f = || 1 should be non-escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_closure_returned_is_escaping() {
        // fn make() { || 42 } — the closure slot flows into the return slot.
        // Slots: _0 = return, _1 = closure
        let mir = single_block_mir(
            "phase_b_returned",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
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
            2,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "returned closure must be escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_closure_in_array_literal_is_escaping() {
        // let a = [|x| x]  — Rvalue::Aggregate row 2.
        // Slots: _0 = return, _1 = closure, _2 = array
        let mir = single_block_mir(
            "phase_b_in_array_literal",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Aggregate(vec![Operand::Copy(Place::Local(SlotId(1)))]),
                    ),
                    2,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure stored in array literal must be escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_closure_pushed_via_array_store_is_escaping() {
        // let a = []; a.push(|x| x)  — StatementKind::ArrayStore row 2.
        // Slots: _0 = return, _1 = array, _2 = closure
        let mir = single_block_mir(
            "phase_b_array_store",
            vec![
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Aggregate(vec![]),
                    ),
                    0,
                ),
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(2),
                        operands: vec![],
                        function_id: None,
                    },
                    1,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    2,
                ),
                make_stmt(
                    StatementKind::ArrayStore {
                        container_slot: SlotId(1),
                        operands: vec![Operand::Copy(Place::Local(SlotId(2)))],
                    },
                    3,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(2)),
            "closure pushed into array via ArrayStore must be escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_closure_stored_in_object_field_is_escaping() {
        // foo.f = || 1  — Place::Field write row 3.
        // Slots: _0 = return, _1 = foo (struct), _2 = closure
        let mir = single_block_mir(
            "phase_b_field_store",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(2),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Field(Box::new(Place::Local(SlotId(1))), FieldIdx(0)),
                        Rvalue::Use(Operand::Copy(Place::Local(SlotId(2)))),
                    ),
                    2,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(2)),
            "closure stored in struct field must be escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_closure_across_detached_task_is_escaping() {
        // async scope { || 1 } — TaskBoundary(Detached) row 5.
        let mir = single_block_mir(
            "phase_b_detached_task",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::TaskBoundary(
                        vec![Operand::Copy(Place::Local(SlotId(1)))],
                        TaskBoundaryKind::Detached,
                    ),
                    2,
                ),
            ],
            2,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure crossing detached task boundary must be escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_closure_across_structured_task_is_escaping() {
        // Structured task boundaries conservatively escape in Phase B per
        // §5.5 — stack allocation through structured boundaries is deferred.
        let mir = single_block_mir(
            "phase_b_structured_task",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::TaskBoundary(
                        vec![Operand::Copy(Place::Local(SlotId(1)))],
                        TaskBoundaryKind::Structured,
                    ),
                    2,
                ),
            ],
            2,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure crossing structured task boundary must be escaping (conservative); got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_closure_written_through_deref_is_escaping() {
        // *p = |x| x — Place::Deref write row 9.
        // Slots: _0 = return, _1 = pointer-holding slot, _2 = closure
        let mir = single_block_mir(
            "phase_b_deref_write",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(2),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Deref(Box::new(Place::Local(SlotId(1)))),
                        Rvalue::Use(Operand::Copy(Place::Local(SlotId(2)))),
                    ),
                    2,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(2)),
            "closure written through deref must be escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_closure_promoted_to_shared_cow_is_escaping() {
        // Row 10: if the semantics planner assigns the closure slot
        // SharedCow/UniqueHeap storage, the closure value is escaping by
        // construction.
        let mir = single_block_mir(
            "phase_b_shared_cow_promotion",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
            ],
            2,
        );

        let analysis = empty_analysis();
        let mut binding_semantics = HashMap::new();
        binding_semantics.insert(
            1u16,
            BindingSemantics {
                ownership_class: BindingOwnershipClass::Flexible,
                storage_class: BindingStorageClass::SharedCow,
                aliasability: Aliasability::SharedMutable,
                mutation_capability: MutationCapability::SharedMutable,
                escape_status: EscapeStatus::Local,
                return_ownership_hint: None,
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
            callee_summaries: None,
        };
        let plan = plan_storage(&input);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "SharedCow-promoted closure slot must be escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_transitive_closure_capture_escapes_together() {
        // let f = |x| x; let g = |y| f(y) + y — g captures f. If g escapes
        // (returned), f must also be classified as escaping. Row 4 +
        // transitive fixed-point (§2.4).
        // Slots: _0 = return, _1 = f, _2 = g
        let mir = single_block_mir(
            "phase_b_transitive_escape",
            vec![
                // Define f
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                // Define g, capturing f
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(2),
                        operands: vec![Operand::Copy(Place::Local(SlotId(1)))],
                        function_id: None,
                    },
                    2,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    3,
                ),
                // g flows to the return slot — g escapes.
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(0)),
                        Rvalue::Use(Operand::Copy(Place::Local(SlotId(2)))),
                    ),
                    4,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(2)),
            "escaping g must not be classified non-escaping"
        );
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "f is captured by escaping g → f must also escape (§2.4); got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_transitive_capture_both_non_escaping() {
        // Dual of the test above — when g is non-escaping, the closure it
        // captures (f) should also be non-escaping. Proves the fixed-point
        // does not over-mark as escaping.
        // Slots: _0 = return, _1 = f, _2 = g
        let mir = single_block_mir(
            "phase_b_transitive_non_escape",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(2),
                        operands: vec![Operand::Copy(Place::Local(SlotId(1)))],
                        function_id: None,
                    },
                    2,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    3,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "non-escaping g keeps f non-escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
        assert!(
            plan.non_escaping_closure_slots.contains(&SlotId(2)),
            "g itself is non-escaping"
        );
    }

    #[test]
    fn test_phase_b_call_arg_conservative_without_summary() {
        // arr.map(|x| x+1) — without a callee summary, Phase B must
        // classify the closure as escaping (sub-task 6 conservative
        // fallback). Phase C will refine this via the mono key.
        // Slots: _0 = return, _1 = closure, _2 = result, _3 = arr
        let mir = make_mir(
            "phase_b_call_arg_conservative",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(1),
                            operands: vec![],
                            function_id: None,
                        },
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                        ),
                        1,
                    ),
                ],
                terminator: Terminator {
                    kind: TerminatorKind::Call {
                        func: Operand::Constant(MirConstant::Function("map".to_string())),
                        args: vec![
                            Operand::Copy(Place::Local(SlotId(3))),
                            Operand::Copy(Place::Local(SlotId(1))),
                        ],
                        destination: Place::Local(SlotId(2)),
                        next: BasicBlockId(0),
                    },
                    span: span(),
                },
            }],
            4,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure passed as call arg without a callee summary must be conservative = escaping"
        );
    }

    #[test]
    fn test_phase_b_call_arg_with_non_escaping_summary() {
        // arr.map(|x| x+1) — with a callee summary marking the
        // corresponding parameter as non-escaping, the closure is allowed to
        // be classified as non-escaping.
        use crate::mir::analysis::{FunctionBorrowSummary, ReturnOwnershipMode};

        let mir = make_mir(
            "phase_b_call_arg_with_summary",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(1),
                            operands: vec![],
                            function_id: None,
                        },
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                        ),
                        1,
                    ),
                ],
                terminator: Terminator {
                    kind: TerminatorKind::Call {
                        func: Operand::Constant(MirConstant::Function(
                            "trusted_non_escaping".to_string(),
                        )),
                        args: vec![
                            Operand::Copy(Place::Local(SlotId(3))),
                            Operand::Copy(Place::Local(SlotId(1))),
                        ],
                        destination: Place::Local(SlotId(2)),
                        next: BasicBlockId(0),
                    },
                    span: span(),
                },
            }],
            4,
        );

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();
        let mut summaries: HashMap<String, FunctionBorrowSummary> = HashMap::new();
        summaries.insert(
            "trusted_non_escaping".to_string(),
            FunctionBorrowSummary {
                param_borrows: vec![None, None],
                conflict_pairs: vec![],
                return_summary: None,
                return_ownership_mode: ReturnOwnershipMode::Unknown,
                // Both params non-escaping.
                closure_param_escapes: vec![false, false],
            },
        );

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
            callee_summaries: Some(&summaries),
        };
        let plan = plan_storage(&input);
        assert!(
            plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure passed to a callee with a non-escaping param summary is non-escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    #[test]
    fn test_phase_b_snapshot_call_forces_escape() {
        // Row 7: `snapshot()` is opaque FFI — any closure flowing into it
        // must be treated as escaping.
        let mir = make_mir(
            "phase_b_snapshot_escape",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::ClosureCapture {
                            closure_slot: SlotId(1),
                            operands: vec![],
                            function_id: None,
                        },
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                        ),
                        1,
                    ),
                ],
                terminator: Terminator {
                    kind: TerminatorKind::Call {
                        func: Operand::Constant(MirConstant::Function("snapshot".to_string())),
                        args: vec![Operand::Copy(Place::Local(SlotId(1)))],
                        destination: Place::Local(SlotId(2)),
                        next: BasicBlockId(0),
                    },
                    span: span(),
                },
            }],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure fed to snapshot() must be escaping"
        );
    }

    #[test]
    fn test_phase_b_enum_store_is_escaping() {
        // Rvalue 2 subcase: closure stored into an enum payload.
        // Slots: _0 = return, _1 = closure, _2 = enum
        let mir = single_block_mir(
            "phase_b_enum_store",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::EnumStore {
                        container_slot: SlotId(2),
                        operands: vec![Operand::Copy(Place::Local(SlotId(1)))],
                    },
                    2,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure stored in enum payload must be escaping"
        );
    }

    #[test]
    fn test_phase_b_object_store_is_escaping() {
        // Rvalue 2 subcase: closure stored into an object/struct literal.
        // Slots: _0 = return, _1 = closure, _2 = object
        let mir = single_block_mir(
            "phase_b_object_store",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::ObjectStore {
                        container_slot: SlotId(2),
                        operands: vec![Operand::Copy(Place::Local(SlotId(1)))],
                        field_names: vec!["f".to_string()],
                    },
                    2,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure stored in object literal must be escaping"
        );
    }

    #[test]
    fn test_phase_b_two_independent_closures() {
        // Two independent closures in the same function, one escapes the
        // other does not — verifies the per-slot classification doesn't
        // over-report.
        // Slots: _0 = return, _1 = escaping, _2 = local, _3 = result of local call
        let mir = single_block_mir(
            "phase_b_two_independent",
            vec![
                // First closure: escapes via return.
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                // Second closure: never used beyond its assignment.
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(2),
                        operands: vec![],
                        function_id: None,
                    },
                    2,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    3,
                ),
                // Return the first.
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(0)),
                        Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                    ),
                    4,
                ),
            ],
            3,
        );

        let plan = run_planner(&mir);
        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "first closure escapes via return"
        );
        assert!(
            plan.non_escaping_closure_slots.contains(&SlotId(2)),
            "second closure is independent and does not escape; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    // ── Closure Spec Phase D: LocalMutablePtr promotion ──────────────────

    /// Build a minimal MIR: outer mutable slot `_1`, closure slot `_2`
    /// captures `_1`, closure is invoked. Phase B classifies `_2` as
    /// non-escaping; Phase D demotes `_1` from `UniqueHeap` → `LocalMutablePtr`.
    fn phase_d_basic_mir() -> MirFunction {
        single_block_mir(
            "phase_d_local_ptr",
            vec![
                // _1 = 0
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
                    ),
                    0,
                ),
                // ClosureCapture { closure_slot: _2, operands: [copy _1] }
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(2),
                        operands: vec![Operand::Copy(Place::Local(SlotId(1)))],
                        function_id: None,
                    },
                    1,
                ),
                // _2 = <closure placeholder>
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    2,
                ),
                // Reassign outer (proves slot is mutably captured).
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(5))),
                    ),
                    3,
                ),
            ],
            3,
        )
    }

    #[test]
    fn test_phase_d_non_escaping_closure_promotes_outer_slot_to_local_mutable_ptr() {
        let mir = phase_d_basic_mir();
        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let mut closure_captures = HashSet::new();
        closure_captures.insert(SlotId(1));
        let mut mutable_captures = HashSet::new();
        mutable_captures.insert(SlotId(1));

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
            callee_summaries: None,
        };
        let plan = plan_storage(&input);

        assert_eq!(
            plan.slot_classes.get(&SlotId(1)),
            Some(&BindingStorageClass::LocalMutablePtr),
            "non-escaping mutable capture → LocalMutablePtr"
        );
        assert!(
            plan.non_escaping_closure_slots.contains(&SlotId(2)),
            "closure in _2 is non-escaping"
        );
    }

    #[test]
    fn test_phase_d_escaping_closure_leaves_outer_slot_as_unique_heap() {
        // The closure escapes via return, so its mutable capture must stay
        // on the heap — Phase D's demotion is gated on `non_escaping`.
        let mut mir = phase_d_basic_mir();
        // Replace the trailing statement with `_0 = copy _2` so the closure
        // flows into the return slot.
        let last_block_idx = mir.blocks.len() - 1;
        let stmts = &mut mir.blocks[last_block_idx].statements;
        // Drop the reassignment (index 3) and add a return-flow statement.
        stmts.pop();
        stmts.push(make_stmt(
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Copy(Place::Local(SlotId(2)))),
            ),
            3,
        ));

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let mut closure_captures = HashSet::new();
        closure_captures.insert(SlotId(1));
        let mut mutable_captures = HashSet::new();
        mutable_captures.insert(SlotId(1));

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
            callee_summaries: None,
        };
        let plan = plan_storage(&input);

        assert!(
            !plan.non_escaping_closure_slots.contains(&SlotId(2)),
            "closure is escaping via return"
        );
        assert_eq!(
            plan.slot_classes.get(&SlotId(1)),
            Some(&BindingStorageClass::UniqueHeap),
            "escaping closure → outer slot stays UniqueHeap"
        );
    }

    #[test]
    fn test_phase_d_multiple_disjoint_captures_all_local_mutable_ptr() {
        // fn f() { let mut a = 0; let mut b = 0; let c = |x| { a = a + 1; b = b + 2 }; c(); }
        // Two disjoint mutably-captured slots by the same non-escaping closure.
        // Both should be `LocalMutablePtr`.
        let mir = single_block_mir(
            "phase_d_disjoint",
            vec![
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
                    ),
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(3),
                        operands: vec![
                            Operand::Copy(Place::Local(SlotId(1))),
                            Operand::Copy(Place::Local(SlotId(2))),
                        ],
                        function_id: None,
                    },
                    2,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(3)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    3,
                ),
                // Reassignments to force `mutable_captures` membership.
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                    ),
                    4,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(2))),
                    ),
                    5,
                ),
            ],
            4,
        );

        let analysis = empty_analysis();
        let binding_semantics = HashMap::new();
        let mut closure_captures = HashSet::new();
        closure_captures.insert(SlotId(1));
        closure_captures.insert(SlotId(2));
        let mut mutable_captures = HashSet::new();
        mutable_captures.insert(SlotId(1));
        mutable_captures.insert(SlotId(2));

        let input = StoragePlannerInput {
            mir: &mir,
            analysis: &analysis,
            binding_semantics: &binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
            callee_summaries: None,
        };
        let plan = plan_storage(&input);
        assert_eq!(
            plan.slot_classes.get(&SlotId(1)),
            Some(&BindingStorageClass::LocalMutablePtr)
        );
        assert_eq!(
            plan.slot_classes.get(&SlotId(2)),
            Some(&BindingStorageClass::LocalMutablePtr)
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Closure Spec Phase G — snapshot forces all closures to heap
    // (docs/v2-closure-specialization.md §5.6)
    // ─────────────────────────────────────────────────────────────────────

    /// A function that calls `snapshot()` must not stack-allocate any of
    /// its closures. The storage planner short-circuits the non-escaping
    /// set because `snapshot()` serializes `SerializableCallFrame` from the
    /// interpreter's locals array and cannot see Cranelift stack slots.
    #[test]
    fn test_phase_g_snapshot_call_forces_all_closures_escaping() {
        // Bytecode MIR shape (no escape vector fires for the closure slot
        // itself):
        //   _1 = ClosureCapture(|| 1)
        //   _2 = snapshot()        ← terminator: Call(snapshot)
        //   _3 = f(5)              ← second block, irrelevant
        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
            ],
            terminator: Terminator {
                kind: TerminatorKind::Call {
                    func: Operand::Constant(MirConstant::Function(
                        "snapshot".to_string(),
                    )),
                    args: vec![],
                    destination: Place::Local(SlotId(2)),
                    next: BasicBlockId(1),
                },
                span: span(),
            },
        };
        let bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: vec![],
            terminator: make_terminator(TerminatorKind::Return),
        };
        let mir = make_mir("phase_g_snapshot_escape", vec![bb0, bb1], 3);

        let plan = run_planner(&mir);
        assert!(
            plan.non_escaping_closure_slots.is_empty(),
            "snapshot() in function body must force every closure to heap; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    /// Control: the same-shape function WITHOUT `snapshot()` allows the
    /// closure to remain non-escaping (no escape vector fires). This
    /// proves the previous test's failure mode is specifically the
    /// snapshot short-circuit, not a generic escape artifact.
    #[test]
    fn test_phase_g_no_snapshot_keeps_closure_non_escaping() {
        let mir = single_block_mir(
            "phase_g_no_snapshot",
            vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
            ],
            2,
        );

        let plan = run_planner(&mir);
        assert!(
            plan.non_escaping_closure_slots.contains(&SlotId(1)),
            "closure with no escape vector (and no snapshot) must be non-escaping; got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    /// Snapshot call with a closure also present: both the closure slot
    /// AND any local alias of it must end up heap-allocated. Exercises
    /// the "snapshot forces deopt / re-materialize" policy from §5.6
    /// without needing to actually run the JIT deopt path.
    #[test]
    fn test_phase_g_snapshot_forces_aliased_closure_to_heap() {
        // _1 = ClosureCapture  ; _2 = _1 (alias)  ; snapshot()
        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: vec![
                make_stmt(
                    StatementKind::ClosureCapture {
                        closure_slot: SlotId(1),
                        operands: vec![],
                        function_id: None,
                    },
                    0,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(1)),
                        Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                    ),
                    1,
                ),
                make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(2)),
                        Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                    ),
                    2,
                ),
            ],
            terminator: Terminator {
                kind: TerminatorKind::Call {
                    func: Operand::Constant(MirConstant::Function(
                        "snapshot".to_string(),
                    )),
                    args: vec![],
                    destination: Place::Local(SlotId(3)),
                    next: BasicBlockId(1),
                },
                span: span(),
            },
        };
        let bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: vec![],
            terminator: make_terminator(TerminatorKind::Return),
        };
        let mir = make_mir("phase_g_snapshot_aliased", vec![bb0, bb1], 4);

        let plan = run_planner(&mir);
        assert!(
            plan.non_escaping_closure_slots.is_empty(),
            "snapshot() with aliased closure must still force every closure slot to heap; \
             got {:?}",
            plan.non_escaping_closure_slots
        );
    }

    // ─── Phase V0.a: `var` → `SharedCow` default ─────────────────────────
    //
    // These tests exercise `decide_slot_storage` directly with an explicit
    // `var_sharedcow_enabled` flag so they don't depend on — and aren't
    // affected by — the process-wide `SHAPE_V2_VAR_SHAREDCOW` env-var cache
    // that `plan_storage` consults.

    fn run_decide(
        slot: SlotId,
        mir: &MirFunction,
        binding_semantics: &HashMap<u16, BindingSemantics>,
        flag_on: bool,
    ) -> BindingStorageClass {
        let analysis = empty_analysis();
        let closure_captures = HashSet::new();
        let mutable_captures = HashSet::new();
        let input = StoragePlannerInput {
            mir,
            analysis: &analysis,
            binding_semantics,
            closure_captures: &closure_captures,
            mutable_captures: &mutable_captures,
            had_fallbacks: false,
            callee_summaries: None,
        };
        decide_slot_storage(slot, &input, flag_on).0
    }

    #[test]
    fn test_v0a_var_gets_sharedcow_when_flag_on() {
        // `var x = 0; x = x + 1;` — no closure capture, no cross-scope
        // reference. With the V0.a flag on, Rule 1b fires unconditionally
        // because the binding is `Flexible`; with the flag off, the binding
        // is neither aliased nor (by the planner's single-rvalue heuristic)
        // alias-chained, so it falls through to `Direct`.
        let mir = make_mir(
            "v0a_var_flag_on",
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
                            Place::Local(SlotId(0)),
                            Rvalue::BinaryOp(
                                BinOp::Add,
                                Operand::Copy(Place::Local(SlotId(0))),
                                Operand::Constant(MirConstant::Int(1)),
                            ),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            1,
        );

        let mut binding_semantics = HashMap::new();
        binding_semantics.insert(
            0u16,
            BindingSemantics::deferred(BindingOwnershipClass::Flexible),
        );

        // Flag on: Rule 1b promotes `var` → SharedCow regardless of aliasing.
        assert_eq!(
            run_decide(SlotId(0), &mir, &binding_semantics, true),
            BindingStorageClass::SharedCow,
            "Phase V0.a: `var` binding with flag on must be SharedCow"
        );

        // Flag off: Rule 1b is suppressed. The binding is not aliased
        // (only one rvalue read), so Rule 3 does not fire. The planner
        // falls through to `Direct`.
        assert_eq!(
            run_decide(SlotId(0), &mir, &binding_semantics, false),
            BindingStorageClass::Direct,
            "Phase V0.a: `var` binding with flag off falls back to Direct when not aliased+mutated"
        );
    }

    #[test]
    fn test_v0a_let_stays_direct_regardless_of_flag() {
        // `let x = 0;` — `OwnedImmutable`. Never promoted to SharedCow
        // under either flag state.
        let mir = make_mir(
            "v0a_let_direct",
            vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![make_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(0)),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
                    ),
                    0,
                )],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            1,
        );

        let mut binding_semantics = HashMap::new();
        binding_semantics.insert(
            0u16,
            BindingSemantics::deferred(BindingOwnershipClass::OwnedImmutable),
        );

        for flag_on in [true, false] {
            assert_eq!(
                run_decide(SlotId(0), &mir, &binding_semantics, flag_on),
                BindingStorageClass::Direct,
                "Phase V0.a: `let` binding stays Direct (flag_on = {flag_on})"
            );
        }
    }

    #[test]
    fn test_v0a_let_mut_not_sharedcow_under_flag() {
        // `let mut x = 0; x = x + 1;` — `OwnedMutable`, no closure capture,
        // no cross-scope reference. Rule 1b must NOT fire (only Flexible
        // bindings get the V0.a promotion). Rule 3 also does not fire
        // (OwnedMutable is not Flexible). Slot stays `Direct` under both
        // flag states.
        let mir = make_mir(
            "v0a_let_mut_no_cow",
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
                            Place::Local(SlotId(0)),
                            Rvalue::BinaryOp(
                                BinOp::Add,
                                Operand::Copy(Place::Local(SlotId(0))),
                                Operand::Constant(MirConstant::Int(1)),
                            ),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            1,
        );

        let mut binding_semantics = HashMap::new();
        binding_semantics.insert(
            0u16,
            BindingSemantics::deferred(BindingOwnershipClass::OwnedMutable),
        );

        for flag_on in [true, false] {
            let class = run_decide(SlotId(0), &mir, &binding_semantics, flag_on);
            assert!(
                matches!(
                    class,
                    BindingStorageClass::Direct | BindingStorageClass::UniqueHeap
                ),
                "Phase V0.a: `let mut` must not become SharedCow under V0.a \
                 flag (flag_on = {flag_on}); got {class:?}"
            );
            assert_ne!(
                class,
                BindingStorageClass::SharedCow,
                "Phase V0.a: `let mut` must NEVER become SharedCow (flag_on = {flag_on})"
            );
        }
    }
}
