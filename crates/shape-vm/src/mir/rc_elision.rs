//! ADR-018 §3 — retain/release pair cancellation over existing solver facts.
//!
//! The bytecode compiler's ownership-aware emission (V1.1C) reads an owned
//! heap local with `CloneLocal` (a retain) and releases the slot's own share
//! with `DropLocal` at scope exit (a release). When the read is the binding's
//! only use and it happens on every path to every exit, those two operations
//! cancel: the read can take the slot's existing share instead of minting a
//! second one, and the scope-exit release then has nothing left to release.
//! `LoadLocalMove` performs exactly that transfer — `stack_take_kinded` clears
//! the slot without releasing, `push_kinded` publishes the bits without
//! retaining.
//!
//! ## Why this is sound under cycle collection
//!
//! ADR-018 §3 admits an elision only when one of two conditions holds for the
//! *entire* elided interval: a covering owning reference keeps the value live
//! across it, or the interval contains no collection safepoint. This pass
//! proves the first, in its strongest form. A retain/release cancellation
//! normally opens an interval during which a frame reference is uncounted; a
//! *transfer* opens no interval at all. Before the `LoadLocalMove` the slot
//! owns the share; after it the stack owns the same share; there is no
//! instant in between at which the share is unowned. A cycle collection
//! triggered anywhere inside the region this pass affects therefore sees the
//! same external count it saw before the pass, and cannot find the subgraph
//! internally balanced.
//!
//! The candidate-production requirement in the same ADR paragraph is met for
//! the same reason: the cancelled release was the *second* of two releases on
//! the value, and the remaining one — where the moved value is finally
//! consumed — still runs the decrement barrier and still buffers the value as
//! a possible cycle root if it survives. Elision removes a redundant
//! candidate, never the last one.
//!
//! ## What the pass does NOT do
//!
//! Executor-internal refcount operations are invisible here and out of scope
//! per ADR-018 §3's last bullet. This pass produces a fact set only; it does
//! not rewrite MIR, so the JIT's MIR consumer sees byte-identical input and
//! keeps its own ownership discipline unchanged.

use super::cfg::ControlFlowGraph;
use super::liveness::LivenessResult;
use super::types::*;
use std::collections::{HashMap, HashSet};

/// The slots whose retain/release pair the bytecode compiler may cancel.
///
/// Consumed by `compiler/helpers_binding.rs::emit_load_local_owned`, which
/// emits `LoadLocalMove` instead of `CloneLocal`. The matching releases —
/// `DropLocal` in `pop_drop_scope` / `emit_drops_for_early_exit`, and the
/// legacy `LoadLocal` + `DropCall` pass in `emit_drop_call_for_local` — key
/// off the compiler's `rc_elided_move_slots` record of what was actually
/// emitted, never off this set directly, so the halves cannot disagree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RcElisionPlan {
    /// Source names of the bindings whose single whole-local read is a
    /// terminal move.
    ///
    /// Keyed by NAME, not by slot: MIR numbers a slot per lowered temporary
    /// and the bytecode compiler numbers one per user binding, so the two
    /// numberings diverge in any body containing a temporary and no positional
    /// offset relates them (see `MirFunction::local_names`). A name that is not
    /// unique among the function's user bindings — a shadowed binding — is
    /// excluded rather than guessed at, so a hit here identifies exactly one
    /// binding on both sides.
    pub terminal_move_bindings: HashSet<String>,
}

impl RcElisionPlan {
    pub fn is_terminal_move_binding(&self, name: &str) -> bool {
        self.terminal_move_bindings.contains(name)
    }
}

/// Where a slot is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseSite {
    Statement {
        block: BasicBlockId,
        stmt_idx: usize,
    },
    Terminator {
        block: BasicBlockId,
    },
}

/// Per-slot use census.
#[derive(Debug, Default)]
struct SlotUses {
    /// Assignments whose destination is exactly `Place::Local(slot)`.
    defs: usize,
    /// Where the single definition is, when there is exactly one.
    def_site: Option<UseSite>,
    /// Reads of the whole local in a context whose consumer takes ownership.
    transferable_reads: Vec<UseSite>,
    /// Every other appearance: projections (`x.f`, `x[i]`, `*x`), borrows,
    /// closure captures, task boundaries, module-binding stores, explicit
    /// drops, and assignment destinations that are projections of the slot.
    /// A single one of these disqualifies the slot.
    other_uses: usize,
}

/// Compute the elision plan for one MIR function.
pub fn compute_plan(
    mir: &MirFunction,
    cfg: &ControlFlowGraph,
    liveness: &LivenessResult,
    loans: &HashMap<LoanId, super::analysis::LoanInfo>,
) -> RcElisionPlan {
    let mut plan = RcElisionPlan::default();

    // A function with no reachable `Return` terminator has no exit whose drop
    // emission the move could dominate; there is nothing to prove domination
    // against. Lowering leaves an unwired exit block behind when a body ends
    // in a tail expression — no path reaches it, so no drop it would host ever
    // runs, and demanding domination over it would reject every function
    // shaped that way.
    let return_blocks: Vec<BasicBlockId> = mir
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator.kind, TerminatorKind::Return))
        .map(|b| b.id)
        .filter(|&id| cfg.is_reachable(id))
        .collect();
    if return_blocks.is_empty() {
        return plan;
    }

    // Any loan on a slot means a reference observes it; a move out from under
    // a live reference is exactly the uncounted-frame-reference hazard.
    let borrowed: HashSet<SlotId> = loans
        .values()
        .map(|info| info.borrowed_place.root_local())
        .collect();

    let census = census_slot_uses(mir);
    let dominators = cfg.dominators();
    let post_dominators = post_dominators(cfg, mir, &return_blocks);

    for (&slot, uses) in census.iter() {
        if !slot_shape_qualifies(mir, slot, uses, &borrowed) {
            continue;
        }
        let read = uses.transferable_reads[0];
        let Some(def) = uses.def_site else {
            continue;
        };

        // The definition must reach the move on every execution that reaches
        // the move at all, and the move must happen on every execution that
        // passes the definition. Together: exactly one move per definition.
        //
        // Post-domination, not "the move dominates every exit", is the
        // condition the release sites actually need. A binding declared inside
        // a loop body and consumed inside the same iteration is redefined
        // before the next read, so moving its share out is sound even though
        // the move sits on a cycle and dominates no exit — and that shape is
        // the bulk of allocation-heavy code. What must be excluded is a move
        // that can run twice against one definition (a read inside a loop of a
        // binding declared outside it), and liveness already excludes it: such
        // a slot is live after the read via the back edge.
        if !dominates(&dominators, block_of(def), block_of(read))
            || !precedes_within_block(def, read)
        {
            continue;
        }
        if !post_dominates(&post_dominators, block_of(read), block_of(def)) {
            continue;
        }

        // Liveness must agree the slot is dead once the read has happened.
        // This is what rules out a move running more than once per definition:
        // a binding declared outside a loop and read inside it is live after
        // its read via the back edge.
        if slot_live_after(mir, liveness, read, slot) {
            continue;
        }

        if let Some(name) = unique_binding_name(mir, slot) {
            plan.terminal_move_bindings.insert(name);
        }
    }

    plan
}

/// The source name of `slot`, when that name identifies exactly one user
/// binding in the function. `None` for synthesized temporaries and for
/// shadowed names, which no name-keyed lookup could resolve unambiguously.
fn unique_binding_name(mir: &MirFunction, slot: SlotId) -> Option<String> {
    let name = mir.local_names.get(slot.0 as usize)?;
    if name.starts_with("__mir_") {
        return None;
    }
    let occurrences = mir
        .local_names
        .iter()
        .enumerate()
        .filter(|(idx, n)| *n == name && mir.binding_slots.contains(&SlotId(*idx as u16)))
        .count();
    (occurrences == 1).then(|| name.clone())
}

fn block_of(site: UseSite) -> BasicBlockId {
    match site {
        UseSite::Statement { block, .. } | UseSite::Terminator { block } => block,
    }
}

/// When `def` and `read` share a block, the def must come first — block-level
/// dominance says nothing about order inside a block. Vacuously true when they
/// are in different blocks, where the block relations carry the ordering.
fn precedes_within_block(def: UseSite, read: UseSite) -> bool {
    match (def, read) {
        (
            UseSite::Statement {
                block: db,
                stmt_idx: di,
            },
            UseSite::Statement {
                block: rb,
                stmt_idx: ri,
            },
        ) if db == rb => di < ri,
        (UseSite::Terminator { block: db }, UseSite::Statement { block: rb, .. }) if db == rb => {
            // The terminator runs after every statement in its block, so a
            // read earlier in the same block precedes its own definition.
            false
        }
        _ => true,
    }
}

/// The slot-level (use-count and type) half of the eligibility test.
fn slot_shape_qualifies(
    mir: &MirFunction,
    slot: SlotId,
    uses: &SlotUses,
    borrowed: &HashSet<SlotId>,
) -> bool {
    // `SlotId(0)` is the return slot, not a user binding.
    if slot.0 == 0 {
        return false;
    }
    // Parameters are handed to the frame by the caller and are not tracked
    // for ownership drops; their release path is the call convention's, not
    // this pass's to cancel.
    if mir.param_slots.contains(&slot) {
        return false;
    }
    // Only real user bindings — synthesized `__mir_*` temporaries do not
    // reach `emit_load_local_owned` and have no scope-exit `DropLocal`.
    if !mir.binding_slots.contains(&slot) {
        return false;
    }
    // `Copy` slots hold inline scalars: they never take the ownership-aware
    // read this pass rewrites, so admitting them would buy nothing. `Unknown`
    // is admitted — it is what lowering records for an inferred binding such as
    // `let g = build_graph(n)`, which is the dominant shape in allocation-heavy
    // code, and the ownership proof this pass makes does not rest on the
    // classification: the read is rewritten only where the compiler was
    // already emitting a retain, and what replaces it transfers the slot's
    // share whatever kind that share has.
    if matches!(
        mir.local_types.get(slot.0 as usize),
        Some(LocalTypeInfo::Copy) | None
    ) {
        return false;
    }
    if borrowed.contains(&slot) {
        return false;
    }
    uses.defs == 1 && uses.transferable_reads.len() == 1 && uses.other_uses == 0
}

/// Does `slot` remain live after the use at `site`?
fn slot_live_after(
    mir: &MirFunction,
    liveness: &LivenessResult,
    site: UseSite,
    slot: SlotId,
) -> bool {
    match site {
        UseSite::Statement { block, stmt_idx } => {
            liveness.is_live_after(block, stmt_idx, slot, mir)
        }
        UseSite::Terminator { block } => liveness
            .live_out
            .get(&block)
            .is_some_and(|set| set.contains(&slot)),
    }
}

/// Immediate-dominator-chain walk. `cfg.dominators()` returns the immediate
/// dominator of each block; the entry block is its own.
fn dominates(
    idom: &HashMap<BasicBlockId, BasicBlockId>,
    candidate: BasicBlockId,
    target: BasicBlockId,
) -> bool {
    let mut cur = target;
    loop {
        if cur == candidate {
            return true;
        }
        match idom.get(&cur) {
            Some(&next) if next != cur => cur = next,
            _ => return false,
        }
    }
}

/// Post-dominator sets, keyed by block: `pdom[b]` is every block through which
/// every path from `b` to a function exit must pass.
///
/// `ControlFlowGraph` ships forward dominators only. This is the same iterative
/// dataflow on the reverse graph:
///
/// ```text
/// pdom(exit) = {exit}
/// pdom(b)    = {b} ∪ ⋂ { pdom(s) : s ∈ succ(b) that reaches an exit }
/// ```
///
/// Blocks with no path to any exit (divergence, unreachable tails) get no
/// entry, so `post_dominates` answers `false` for them — the conservative
/// direction, since a move there never reaches the release it would cancel.
fn post_dominators(
    cfg: &ControlFlowGraph,
    mir: &MirFunction,
    exits: &[BasicBlockId],
) -> HashMap<BasicBlockId, HashSet<BasicBlockId>> {
    let exit_set: HashSet<BasicBlockId> = exits.iter().copied().collect();

    // Blocks that can reach an exit at all — the domain of the analysis.
    let mut reaches_exit: HashSet<BasicBlockId> = exit_set.clone();
    loop {
        let mut grew = false;
        for block in &mir.blocks {
            if reaches_exit.contains(&block.id) {
                continue;
            }
            if cfg
                .successors(block.id)
                .iter()
                .any(|s| reaches_exit.contains(s))
            {
                reaches_exit.insert(block.id);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    let universe: HashSet<BasicBlockId> = reaches_exit.iter().copied().collect();
    let mut pdom: HashMap<BasicBlockId, HashSet<BasicBlockId>> = HashMap::new();
    for &b in &reaches_exit {
        if exit_set.contains(&b) {
            pdom.insert(b, HashSet::from([b]));
        } else {
            pdom.insert(b, universe.clone());
        }
    }

    let order: Vec<BasicBlockId> = cfg
        .reverse_postorder()
        .into_iter()
        .filter(|b| reaches_exit.contains(b) && !exit_set.contains(b))
        .collect();

    loop {
        let mut changed = false;
        for &b in order.iter().rev() {
            let mut acc: Option<HashSet<BasicBlockId>> = None;
            for s in cfg.successors(b) {
                if !reaches_exit.contains(s) {
                    continue;
                }
                let s_pdom = &pdom[s];
                acc = Some(match acc {
                    None => s_pdom.clone(),
                    Some(cur) => cur.intersection(s_pdom).copied().collect(),
                });
            }
            let mut next = acc.unwrap_or_default();
            next.insert(b);
            if next != pdom[&b] {
                pdom.insert(b, next);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    pdom
}

fn post_dominates(
    pdom: &HashMap<BasicBlockId, HashSet<BasicBlockId>>,
    candidate: BasicBlockId,
    target: BasicBlockId,
) -> bool {
    pdom.get(&target)
        .is_some_and(|set| set.contains(&candidate))
}

// ── Use census ───────────────────────────────────────────────────────

fn census_slot_uses(mir: &MirFunction) -> HashMap<SlotId, SlotUses> {
    let mut census: HashMap<SlotId, SlotUses> = HashMap::new();

    for block in &mir.blocks {
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            let site = UseSite::Statement {
                block: block.id,
                stmt_idx,
            };
            census_statement(&mut census, &stmt.kind, site);
        }
        let site = UseSite::Terminator { block: block.id };
        census_terminator(&mut census, &block.terminator.kind, site);
    }

    census
}

fn entry<'a>(census: &'a mut HashMap<SlotId, SlotUses>, slot: SlotId) -> &'a mut SlotUses {
    census.entry(slot).or_default()
}

/// Record an ownership-transferring read of a whole local, or an
/// disqualifying use for anything else.
fn transferable_operand(census: &mut HashMap<SlotId, SlotUses>, op: &Operand, site: UseSite) {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            match place {
                Place::Local(slot) => entry(census, *slot).transferable_reads.push(site),
                // A projection reads *through* the local; moving the whole
                // local out from under a field read would clear the base.
                _ => entry(census, place.root_local()).other_uses += 1,
            }
        }
        Operand::Constant(_) => {}
    }
}

/// Record a use that disqualifies the slot regardless of context.
fn opaque_operand(census: &mut HashMap<SlotId, SlotUses>, op: &Operand) {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            entry(census, place.root_local()).other_uses += 1;
        }
        Operand::Constant(_) => {}
    }
}

fn census_statement(census: &mut HashMap<SlotId, SlotUses>, kind: &StatementKind, site: UseSite) {
    match kind {
        StatementKind::Assign(place, rvalue) => {
            match place {
                Place::Local(slot) => {
                    let e = entry(census, *slot);
                    e.defs += 1;
                    e.def_site = Some(site);
                }
                // `x.f = v` neither defines nor releases `x` as a whole.
                _ => entry(census, place.root_local()).other_uses += 1,
            }
            census_rvalue(census, rvalue, site);
        }
        StatementKind::Drop(place) => {
            entry(census, place.root_local()).other_uses += 1;
        }
        // A closure capture is paired with `DropClosureCaptures` emission that
        // re-reads the live slot at scope exit; a task boundary and a
        // module-binding store both hand the value to a lifetime this pass
        // does not model. All three are disqualifying.
        StatementKind::TaskBoundary(operands, _) => {
            for op in operands {
                opaque_operand(census, op);
            }
        }
        StatementKind::ClosureCapture { operands, .. } => {
            for op in operands {
                opaque_operand(census, op);
            }
        }
        StatementKind::ModuleBindingStore { operands, .. } => {
            for op in operands {
                opaque_operand(census, op);
            }
        }
        // Container stores do consume their operands, but the bytecode
        // compiler reaches the element values through emission paths this
        // slice has not audited against `emit_load_local_owned`. Held opaque
        // until a later slice measures whether widening pays.
        StatementKind::ArrayStore { operands, .. }
        | StatementKind::ObjectStore { operands, .. }
        | StatementKind::EnumStore { operands, .. } => {
            for op in operands {
                opaque_operand(census, op);
            }
        }
        StatementKind::Nop => {}
    }
}

fn census_rvalue(census: &mut HashMap<SlotId, SlotUses>, rvalue: &Rvalue, site: UseSite) {
    match rvalue {
        Rvalue::Use(op) => transferable_operand(census, op, site),
        // An explicit clone is a retain the source program asked for; a borrow
        // is a loan. Neither may consume the source.
        Rvalue::Clone(op) => opaque_operand(census, op),
        Rvalue::Borrow(_, place) => {
            entry(census, place.root_local()).other_uses += 1;
        }
        Rvalue::UnaryOp(_, op) => opaque_operand(census, op),
        Rvalue::BinaryOp(_, lhs, rhs) => {
            opaque_operand(census, lhs);
            opaque_operand(census, rhs);
        }
        Rvalue::FuzzyComparison { lhs, rhs, .. } => {
            opaque_operand(census, lhs);
            opaque_operand(census, rhs);
        }
        Rvalue::Aggregate(ops) => {
            for op in ops {
                opaque_operand(census, op);
            }
        }
        Rvalue::EnumTest { operand, .. }
        | Rvalue::EnumPayload { operand, .. }
        | Rvalue::TypePatternTest { operand, .. }
        | Rvalue::EnumDiscriminantTest { operand, .. }
        | Rvalue::PrimitiveCast { operand, .. }
        | Rvalue::FormatValue { operand, .. } => opaque_operand(census, operand),
    }
}

fn census_terminator(census: &mut HashMap<SlotId, SlotUses>, kind: &TerminatorKind, site: UseSite) {
    match kind {
        TerminatorKind::Call {
            func,
            args,
            destination,
            ..
        } => {
            opaque_operand(census, func);
            for op in args {
                transferable_operand(census, op, site);
            }
            match destination {
                Place::Local(slot) => {
                    let e = entry(census, *slot);
                    e.defs += 1;
                    e.def_site = Some(site);
                }
                _ => entry(census, destination.root_local()).other_uses += 1,
            }
        }
        TerminatorKind::SwitchBool { operand, .. } => opaque_operand(census, operand),
        TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
    }
}
