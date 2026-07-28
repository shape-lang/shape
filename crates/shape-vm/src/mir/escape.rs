//! Value-escape and inbound-reference analysis (ADR-018 §4 prerequisites 1+2).
//!
//! Two products, both computed over the MIR the borrow solver already consumes:
//!
//! - **Outbound value-escape** ([`OutboundEscape`]): "this allocation dies with
//!   the frame". A per-allocation-site fact, distinct from the solver's
//!   sink-discriminated *reference*-escape promotions (`solver.rs:1453`), which
//!   answer a different question — whether a *reference* to a local outlives the
//!   frame. An allocation can be frame-confined while a reference to it is
//!   promoted, and a reference-escape promotion is one of the vectors that
//!   disproves confinement (see [`EscapeVector::ReferenceEscape`]).
//!
//! - **Inbound-reference** ([`InboundProof`]): "no outside cycle-capable value is
//!   stored into this allocation". ADR-018 §4 names this a distinct proof
//!   obligation, not a corollary of the outbound product: an outside value
//!   stored *into* a region-confined object creates a boundary-crossing edge
//!   that outbound analysis says nothing about. The two are reported
//!   independently and the arena exemption query
//!   ([`EscapeFacts::region_exemption_candidates`]) is the only API that hands
//!   out the conjunction.
//!
//! # Why not the sink-blind heuristic
//!
//! `storage_planning::detect_escape_status` answers "does this slot flow to the
//! return slot", by chasing `Assign(Place::Local(dest), rvalue)` edges only. The
//! storage planner's own promotion rule (`storage_planning.rs:996`) refuses to
//! consume it because it is sink-blind: it counts `Rvalue::Aggregate [&x]` as
//! return-flow (the B0004 container-referent false positive), and — the
//! direction that matters for *this* product — it cannot see a value that leaves
//! through a container, a field write, a task boundary, a module store, or a
//! call argument, because none of those are `Assign(Place::Local(_), _)`. A
//! confinement claim built on it would be an optimistic escape fact, which for
//! the downstream arena consumer is a use-after-free factory.
//!
//! This analysis instead evaluates the full escape-vector table against a
//! transitive "may hold or contain" set per allocation, so containment is
//! *seen through*: storing an allocation into a container does not by itself
//! escape it, but the container's own escape does escape every member.
//!
//! # Soundness discipline
//!
//! Every fact is three-valued. [`OutboundEscape::FrameConfined`] is a *proof*;
//! anything undecidable is [`OutboundEscape::NotProven`] or a definite
//! [`OutboundEscape::Escapes`]. Consumers must act only on the positive verdict.
//! The conservative direction is always "escapes / not proven", so imprecision
//! costs optimization, never memory safety.
//!
//! This analysis is INERT: it publishes facts on [`super::StoragePlan`] and
//! nothing consumes them to change codegen. Consumers are PERF-RC-ELISION
//! (#190) breadth, stack promotion, and PERF-ARENA (#195).

use std::collections::HashMap;

use shape_ast::ast::Span;

use super::analysis::{BorrowAnalysis, FunctionBorrowSummary};
use super::types::{
    BasicBlockId, LocalTypeInfo, MirConstant, MirFunction, Operand, Place, Rvalue, SlotId,
    StatementKind, TerminatorKind,
};

// ── Public fact types ────────────────────────────────────────────────────

/// Deterministic identity of an allocation site: the statement that
/// materializes a fresh heap value into a slot.
///
/// Position-based rather than slot-based because one slot can be the
/// destination of several allocations (a loop body re-materializing a literal),
/// and each is a separately-dispositioned allocation for arena purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AllocSite {
    /// Index of the containing basic block (blocks are id-sorted by
    /// `MirBuilder::build`, so this is stable across runs).
    pub block: BasicBlockId,
    /// Index of the statement within that block.
    pub statement: u32,
}

/// What kind of heap value the site materializes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AllocKind {
    /// `StatementKind::ArrayStore` — array literal / comprehension result.
    Array,
    /// `StatementKind::ObjectStore` — object or struct literal.
    Object,
    /// `StatementKind::EnumStore` — enum payload construction.
    Enum,
    /// `StatementKind::ClosureCapture` — closure environment.
    ClosureEnv,
}

/// The vector through which an allocation left the frame.
///
/// One variant per row of the escape-vector table
/// (`docs/v2-closure-specialization.md` §2.1), generalized from closure slots to
/// every allocation site. The five the PERF-ESCAPE tripwires name as negative
/// controls are [`Return`](Self::Return),
/// [`ClosureCapture`](Self::ClosureCapture),
/// [`ModuleStore`](Self::ModuleStore),
/// [`ContainerInsert`](Self::ContainerInsert) and
/// [`TaskSpawn`](Self::TaskSpawn).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EscapeVector {
    /// Flowed into the return slot `SlotId(0)`.
    Return,
    /// Captured into a closure environment that is not itself frame-confined.
    ClosureCapture,
    /// Stored into a module-level binding (`StatementKind::ModuleBindingStore`).
    ModuleStore,
    /// Inserted into a container whose own provenance is not a local
    /// allocation — the container may outlive the frame, so its members do too.
    ContainerInsert,
    /// Crossed a task boundary (`StatementKind::TaskBoundary`).
    TaskSpawn,
    /// Passed to a call whose callee is not proven to keep the argument
    /// non-escaping, or used as the callee of a call (a closure body can stash
    /// its own captures where this analysis cannot see).
    CallArgument,
    /// Written into a field/index projection rooted at a slot of foreign
    /// provenance (a parameter, a call result, a module read).
    ForeignPlaceStore,
    /// Written through a dereference — the target is not statically known.
    DerefStore,
    /// The solver proved a reference to the holding slot escapes through a
    /// floor sink (ADR-006 §2.7.30 R2 promotion). The referent cannot die with
    /// the frame if a reference to it outlives the frame.
    ReferenceEscape,
}

/// Why a fact could not be decided either way.
///
/// One variant today, and only one producer: a function-level precondition
/// failure. Every *statement*-level uncertainty resolves to a definite
/// `Escapes` / `ForeignStore` instead, because the conservative direction of
/// each product is a refusal, not an abstention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NotProvenReason {
    /// MIR lowering fell back for this function, so the MIR is not a faithful
    /// over-approximation of the program's dataflow. Mirrors the storage
    /// planner's `had_fallbacks` → `Deferred` rule (`storage_planning.rs:275`).
    MirLoweringIncomplete,
}

/// Outbound product: does this allocation die with the frame?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundEscape {
    /// Proven: no reference to this allocation outlives the frame through any
    /// vector in the table.
    FrameConfined,
    /// Disproven, with the vector that disproves it and the statement span.
    Escapes(EscapeVector, Span),
    /// Undecidable.
    NotProven(NotProvenReason),
}

impl OutboundEscape {
    /// The only positive verdict. Consumers must gate on this, never on
    /// `!matches!(.., Escapes(..))`.
    pub fn is_frame_confined(&self) -> bool {
        matches!(self, OutboundEscape::FrameConfined)
    }
}

/// What kind of outside value was stored into a region-confined allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ForeignSource {
    /// A function parameter — owned by the caller's frame/region.
    Parameter,
    /// The destination of a call — the callee may hand back a value that is
    /// also reachable from somewhere else.
    CallResult,
    /// A non-scalar constant. Notably `MirConstant::Function(name)`: MIR
    /// lowering emits that variant for *any* identifier that does not resolve
    /// to a local (`lowering/expr.rs:2014`), including module-level bindings,
    /// so it cannot be treated as an inert literal.
    OpaqueConstant,
    /// A read through a field/index/deref projection — the projected value's
    /// own provenance is not tracked.
    Projection,
    /// A value whose defining slot is itself of foreign provenance.
    ForeignSlot,
}

/// Inbound product: is this allocation free of stored-in outside values?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundProof {
    /// Proven: every value stored into this allocation is a scalar, a leaf
    /// literal, or a value of proven local-allocation provenance.
    NoForeignStores,
    /// Disproven, with the classification of the offending source.
    ForeignStore(ForeignSource, Span),
    /// Undecidable.
    NotProven(NotProvenReason),
}

impl InboundProof {
    /// The only positive verdict.
    pub fn is_clean(&self) -> bool {
        matches!(self, InboundProof::NoForeignStores)
    }
}

/// Both products for one allocation site.
#[derive(Debug, Clone, PartialEq)]
pub struct AllocationFacts {
    pub site: AllocSite,
    /// The slot the allocation is materialized into.
    pub dest: SlotId,
    pub kind: AllocKind,
    pub span: Span,
    pub outbound: OutboundEscape,
    pub inbound: InboundProof,
}

/// Per-function escape facts, published on [`super::StoragePlan`].
///
/// `allocations` is sorted by [`AllocSite`] — a total order derived from block
/// and statement position, never from hash iteration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EscapeFacts {
    pub allocations: Vec<AllocationFacts>,
}

impl EscapeFacts {
    /// Allocations proven to die with the frame. The outbound product on its
    /// own — the consumer for RC elision and stack promotion.
    pub fn frame_confined(&self) -> impl Iterator<Item = &AllocationFacts> {
        self.allocations
            .iter()
            .filter(|a| a.outbound.is_frame_confined())
    }

    /// Allocations carrying BOTH proofs — the only set ADR-018 §4 licenses for
    /// the collector's candidate-buffer exemption.
    ///
    /// Deliberately the sole API exposing the conjunction: a consumer that
    /// wants the exemption cannot reach it by filtering `frame_confined()`
    /// alone and silently drop the inbound obligation.
    pub fn region_exemption_candidates(&self) -> impl Iterator<Item = &AllocationFacts> {
        self.allocations
            .iter()
            .filter(|a| a.outbound.is_frame_confined() && a.inbound.is_clean())
    }

    /// `(confined, exemption-eligible, total)` allocation counts — the
    /// precision measurement the charter requires per ticket.
    pub fn precision(&self) -> (usize, usize, usize) {
        (
            self.frame_confined().count(),
            self.region_exemption_candidates().count(),
            self.allocations.len(),
        )
    }
}

/// Everything the analysis reads. Mirrors the fields of
/// `StoragePlannerInput` it needs, so the analysis stays a pure function of
/// its inputs and is testable without building a planner input.
pub struct EscapeInput<'a> {
    pub mir: &'a MirFunction,
    /// Solver output — consumed for the sink-discriminated reference-escape
    /// promotions. `None` in tests that exercise MIR shapes only.
    pub analysis: Option<&'a BorrowAnalysis>,
    /// `true` when MIR lowering fell back; every fact becomes `NotProven`.
    pub had_fallbacks: bool,
    /// Callee summaries keyed by function name, for the per-parameter
    /// escape-bit refinement on call arguments.
    pub callee_summaries: Option<&'a HashMap<String, FunctionBorrowSummary>>,
}

// ── Bitset ───────────────────────────────────────────────────────────────

/// Dense bitset over allocation indices. `Vec<u64>` rather than a `HashSet` so
/// that every traversal order in this module is positional and deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AllocSet {
    words: Vec<u64>,
}

impl AllocSet {
    fn with_capacity(n: usize) -> Self {
        Self {
            words: vec![0; n.div_ceil(64)],
        }
    }

    fn insert(&mut self, idx: usize) -> bool {
        let (w, b) = (idx / 64, idx % 64);
        let before = self.words[w];
        self.words[w] |= 1u64 << b;
        self.words[w] != before
    }

    fn contains(&self, idx: usize) -> bool {
        self.words[idx / 64] & (1u64 << (idx % 64)) != 0
    }

    fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// Union `other` into `self`; returns whether anything changed.
    fn union_with(&mut self, other: &AllocSet) -> bool {
        let mut changed = false;
        for (dst, src) in self.words.iter_mut().zip(other.words.iter()) {
            let before = *dst;
            *dst |= *src;
            changed |= *dst != before;
        }
        changed
    }

    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(w, bits)| {
            (0..64).filter_map(move |b| (bits & (1u64 << b) != 0).then_some(w * 64 + b))
        })
    }
}

// ── Allocation-site collection ───────────────────────────────────────────

struct RawAlloc {
    site: AllocSite,
    dest: SlotId,
    kind: AllocKind,
    span: Span,
}

/// Collect the statements that materialize a fresh heap value.
///
/// `Rvalue::Aggregate` is deliberately NOT an allocation site. MIR lowering
/// emits it as a generic multi-operand carrier for shapes that are not
/// allocations at all (`lowering/helpers.rs:404` emits `Aggregate(vec![l, r])`
/// for a binary-op surface, `lowering/expr.rs:452` for match-branch operand
/// merging). Treating those as allocations would invent sites that no allocator
/// call corresponds to. Aggregates are still honoured as *flow* edges below, so
/// an allocation carried inside one is tracked.
fn collect_allocations(mir: &MirFunction) -> Vec<RawAlloc> {
    let mut allocs = Vec::new();
    for block in mir.iter_blocks() {
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            let (dest, kind) = match &stmt.kind {
                StatementKind::ArrayStore { container_slot, .. } => {
                    (*container_slot, AllocKind::Array)
                }
                StatementKind::ObjectStore { container_slot, .. } => {
                    (*container_slot, AllocKind::Object)
                }
                StatementKind::EnumStore { container_slot, .. } => {
                    (*container_slot, AllocKind::Enum)
                }
                StatementKind::ClosureCapture { closure_slot, .. } => {
                    (*closure_slot, AllocKind::ClosureEnv)
                }
                StatementKind::Assign(..)
                | StatementKind::Drop(_)
                | StatementKind::TaskBoundary(..)
                | StatementKind::ModuleBindingStore { .. }
                | StatementKind::Nop => continue,
            };
            allocs.push(RawAlloc {
                site: AllocSite {
                    block: block.id,
                    statement: stmt_idx as u32,
                },
                dest,
                kind,
                span: stmt.span,
            });
        }
    }
    allocs.sort_by_key(|a| a.site);
    allocs
}

// ── Provenance ───────────────────────────────────────────────────────────

/// Per-slot provenance: is every value this slot can hold locally produced?
///
/// Computed as a greatest fixed point — every slot starts `true` and is demoted
/// when a definition is found that this analysis cannot classify as local. The
/// seed demotions (parameters, call destinations) plus an exhaustive match over
/// statement and rvalue shapes mean an unrecognized shape demotes rather than
/// survives.
///
/// Two consumers:
/// - the outbound product uses it to decide whether a container store is
///   containment (local container) or an escape (foreign container);
/// - the inbound product uses it to classify a stored value's source.
fn compute_local_provenance(mir: &MirFunction) -> Vec<bool> {
    let n = mir.num_locals as usize;
    let mut local_only = vec![true; n];

    // The return slot holds values destined for the caller; treat it as
    // foreign so a store through it is never mistaken for local containment.
    if n > 0 {
        local_only[0] = false;
    }
    for param in &mir.param_slots {
        if let Some(entry) = local_only.get_mut(param.0 as usize) {
            *entry = false;
        }
    }
    for block in mir.iter_blocks() {
        if let TerminatorKind::Call { destination, .. } = &block.terminator.kind {
            if let Some(entry) = local_only.get_mut(destination.root_local().0 as usize) {
                *entry = false;
            }
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for block in mir.iter_blocks() {
            for stmt in &block.statements {
                let StatementKind::Assign(Place::Local(dest), rvalue) = &stmt.kind else {
                    continue;
                };
                let idx = dest.0 as usize;
                if idx >= n || !local_only[idx] {
                    continue;
                }
                if !rvalue_is_locally_produced(rvalue, &local_only) {
                    local_only[idx] = false;
                    changed = true;
                }
            }
        }
    }

    local_only
}

/// Is this rvalue's result locally produced given the current provenance map?
///
/// The question is about the *identity* of the produced value, not its
/// contents. A locally-built container holding a parameter is itself locally
/// produced; whether an outside value sits inside it is the inbound product's
/// separate verdict on that container.
fn rvalue_is_locally_produced(rvalue: &Rvalue, local_only: &[bool]) -> bool {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) => operand_is_locally_produced(op, local_only),
        // A reference is exactly as local as its referent: `&local` cannot
        // hand an outside object to anyone, `&param` can.
        Rvalue::Borrow(_, place) => local_only
            .get(place.root_local().0 as usize)
            .copied()
            .unwrap_or(false),
        // Freshly computed scalars and strings — no incoming heap identity.
        Rvalue::BinaryOp(..)
        | Rvalue::UnaryOp(..)
        | Rvalue::FuzzyComparison { .. }
        | Rvalue::FormatValue { .. }
        | Rvalue::EnumTest { .. }
        | Rvalue::TypePatternTest { .. }
        | Rvalue::EnumDiscriminantTest { .. }
        | Rvalue::PrimitiveCast { .. } => true,
        // Reads a payload back out of an enum: the value's origin is whatever
        // was stored into that enum, which this map does not track.
        Rvalue::EnumPayload { .. } => false,
        // The construction form of a container literal — a fresh value built
        // in this frame.
        Rvalue::Aggregate(_) => true,
    }
}

fn operand_is_locally_produced(op: &Operand, local_only: &[bool]) -> bool {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            match place {
                Place::Local(slot) => local_only.get(slot.0 as usize).copied().unwrap_or(false),
                // Reading through a projection yields whatever was stored into
                // the container; that provenance is not tracked here.
                Place::Field(..) | Place::Index(..) | Place::Deref(..) => false,
            }
        }
        Operand::Constant(c) => constant_is_leaf_literal(c),
    }
}

/// Scalar/leaf literals only.
///
/// `Function`/`Method`/`ClosurePlaceholder` are excluded on purpose: MIR
/// lowering emits `MirConstant::Function(name)` for every identifier that does
/// not resolve to a local slot (`lowering/expr.rs:2014`), which includes
/// module-level bindings. Treating that as an inert literal would let a
/// module-resident, possibly cycle-capable value be classified local.
fn constant_is_leaf_literal(c: &MirConstant) -> bool {
    match c {
        MirConstant::Int(_)
        | MirConstant::Bool(_)
        | MirConstant::None
        | MirConstant::StringId(_)
        | MirConstant::Str(_)
        | MirConstant::Float(_)
        | MirConstant::Decimal(_)
        | MirConstant::Char(_) => true,
        MirConstant::Function(_) | MirConstant::Method(_) | MirConstant::ClosurePlaceholder => {
            false
        }
    }
}

// ── The analysis ─────────────────────────────────────────────────────────

/// Run both escape products over one MIR function.
pub fn analyze_escapes(input: &EscapeInput<'_>) -> EscapeFacts {
    let mir = input.mir;
    let allocs = collect_allocations(mir);
    if allocs.is_empty() {
        return EscapeFacts::default();
    }

    if input.had_fallbacks {
        // The MIR is not a faithful over-approximation of the program's
        // dataflow, so no vector table run over it can prove anything.
        return EscapeFacts {
            allocations: allocs
                .into_iter()
                .map(|a| AllocationFacts {
                    site: a.site,
                    dest: a.dest,
                    kind: a.kind,
                    span: a.span,
                    outbound: OutboundEscape::NotProven(NotProvenReason::MirLoweringIncomplete),
                    inbound: InboundProof::NotProven(NotProvenReason::MirLoweringIncomplete),
                })
                .collect(),
        };
    }

    let local_only = compute_local_provenance(mir);
    let state = run_outbound(input, &allocs, &local_only);
    let inbound = run_inbound(input, &allocs, &state, &local_only);

    EscapeFacts {
        allocations: allocs
            .iter()
            .enumerate()
            .map(|(i, a)| AllocationFacts {
                site: a.site,
                dest: a.dest,
                kind: a.kind,
                span: a.span,
                outbound: match state.escape_cause[i] {
                    Some((vector, span)) => OutboundEscape::Escapes(vector, span),
                    None => OutboundEscape::FrameConfined,
                },
                inbound: inbound[i],
            })
            .collect(),
    }
}

/// Working state of the outbound fixed point.
struct OutboundState {
    /// Per slot: the allocations that slot may hold directly or contain
    /// transitively. Indexed by `SlotId.0`.
    holds: Vec<AllocSet>,
    /// Per slot: the allocations that slot may name *directly* — copy, move,
    /// clone and borrow chains only, never containment. A subset of `holds`.
    ///
    /// The outbound product uses `holds` (an escape of the container escapes
    /// its members); the inbound product uses `aliases` (a foreign value stored
    /// into a container is not stored into that container's members — each
    /// member carries its own inbound verdict, and the arena consumer takes the
    /// conjunction over the region's allocations).
    aliases: Vec<AllocSet>,
    /// Per allocation: the vector that disproved confinement, if any.
    escape_cause: Vec<Option<(EscapeVector, Span)>>,
}

impl OutboundState {
    /// Record an escape for every allocation in `set`. First cause wins, so the
    /// reported vector is the earliest one in traversal order — deterministic.
    fn escape(&mut self, set: &AllocSet, vector: EscapeVector, span: Span) -> bool {
        let mut changed = false;
        for idx in set.iter() {
            if self.escape_cause[idx].is_none() {
                self.escape_cause[idx] = Some((vector, span));
                changed = true;
            }
        }
        changed
    }
}

fn run_outbound(
    input: &EscapeInput<'_>,
    allocs: &[RawAlloc],
    local_only: &[bool],
) -> OutboundState {
    let mir = input.mir;
    let n_slots = mir.num_locals as usize;
    let n_allocs = allocs.len();

    let mut state = OutboundState {
        holds: vec![AllocSet::with_capacity(n_allocs); n_slots],
        aliases: vec![AllocSet::with_capacity(n_allocs); n_slots],
        escape_cause: vec![None; n_allocs],
    };
    for (i, alloc) in allocs.iter().enumerate() {
        if let Some(set) = state.holds.get_mut(alloc.dest.0 as usize) {
            set.insert(i);
        }
        if let Some(set) = state.aliases.get_mut(alloc.dest.0 as usize) {
            set.insert(i);
        }
    }

    // Sink-discriminated reference escapes from the solver: if a reference to a
    // slot outlives the frame (ADR-006 §2.7.30 R2 floor-sink promotion), no
    // allocation that slot holds can die with the frame. This is the solver's
    // own verdict, not the sink-blind heuristic.
    let promoted_referents: Vec<SlotId> = input
        .analysis
        .map(|a| {
            let mut v: Vec<SlotId> = a
                .reference_escape_promotions
                .iter()
                .map(|t| t.referent_local)
                .collect();
            v.sort();
            v.dedup();
            v
        })
        .unwrap_or_default();

    let mut changed = true;
    while changed {
        changed = false;

        for &referent in &promoted_referents {
            let Some(set) = state.holds.get(referent.0 as usize).cloned() else {
                continue;
            };
            if !set.is_empty() {
                changed |= state.escape(&set, EscapeVector::ReferenceEscape, mir.span);
            }
        }

        for block in mir.iter_blocks() {
            for stmt in &block.statements {
                match &stmt.kind {
                    StatementKind::Assign(place, rvalue) => {
                        let read = read_set(rvalue, &state.holds, n_allocs);
                        if read.is_empty() {
                            continue;
                        }
                        match place {
                            Place::Local(dest) if *dest == SlotId(0) => {
                                changed |= state.escape(&read, EscapeVector::Return, stmt.span);
                            }
                            Place::Local(dest) => {
                                if let Some(slot) = state.holds.get_mut(dest.0 as usize) {
                                    changed |= slot.union_with(&read);
                                }
                                // Alias track (inbound product only): which
                                // slots may name the allocation *itself*, as
                                // opposed to a container holding it. A store
                                // into a container is not a store into its
                                // members.
                                let alias_read = alias_read_set(rvalue, &state, n_allocs);
                                if let Some(slot) = state.aliases.get_mut(dest.0 as usize) {
                                    changed |= slot.union_with(&alias_read);
                                }
                            }
                            Place::Field(base, _) | Place::Index(base, _) => {
                                let root = base.root_local();
                                if local_only.get(root.0 as usize).copied().unwrap_or(false) {
                                    // Containment: writing into a container of
                                    // proven local provenance. The container's
                                    // own escape carries its members out.
                                    if let Some(slot) = state.holds.get_mut(root.0 as usize) {
                                        changed |= slot.union_with(&read);
                                    }
                                } else {
                                    changed |= state.escape(
                                        &read,
                                        EscapeVector::ForeignPlaceStore,
                                        stmt.span,
                                    );
                                }
                            }
                            Place::Deref(_) => {
                                changed |= state.escape(&read, EscapeVector::DerefStore, stmt.span);
                            }
                        }
                    }
                    StatementKind::ArrayStore {
                        container_slot,
                        operands,
                    }
                    | StatementKind::ObjectStore {
                        container_slot,
                        operands,
                        ..
                    }
                    | StatementKind::EnumStore {
                        container_slot,
                        operands,
                        ..
                    } => {
                        let read = operands_read_set(operands, &state.holds, n_allocs);
                        if read.is_empty() {
                            continue;
                        }
                        if local_only
                            .get(container_slot.0 as usize)
                            .copied()
                            .unwrap_or(false)
                        {
                            if let Some(slot) = state.holds.get_mut(container_slot.0 as usize) {
                                changed |= slot.union_with(&read);
                            }
                        } else {
                            changed |=
                                state.escape(&read, EscapeVector::ContainerInsert, stmt.span);
                        }
                    }
                    StatementKind::ClosureCapture {
                        closure_slot,
                        operands,
                        ..
                    } => {
                        let read = operands_read_set(operands, &state.holds, n_allocs);
                        if read.is_empty() {
                            continue;
                        }
                        if local_only
                            .get(closure_slot.0 as usize)
                            .copied()
                            .unwrap_or(false)
                        {
                            if let Some(slot) = state.holds.get_mut(closure_slot.0 as usize) {
                                changed |= slot.union_with(&read);
                            }
                        } else {
                            changed |= state.escape(&read, EscapeVector::ClosureCapture, stmt.span);
                        }
                    }
                    StatementKind::ModuleBindingStore { operands, .. } => {
                        let read = operands_read_set(operands, &state.holds, n_allocs);
                        changed |= state.escape(&read, EscapeVector::ModuleStore, stmt.span);
                    }
                    StatementKind::TaskBoundary(operands, _) => {
                        let read = operands_read_set(operands, &state.holds, n_allocs);
                        changed |= state.escape(&read, EscapeVector::TaskSpawn, stmt.span);
                    }
                    StatementKind::Drop(_) | StatementKind::Nop => {}
                }
            }

            let terminator = &block.terminator;
            match &terminator.kind {
                TerminatorKind::Call { func, args, .. } => {
                    // The callee operand: invoking a closure hands the callee
                    // its own environment, and the closure body can store a
                    // capture anywhere. Without a body summary for the target
                    // that is not provable, so it is an escape. This is the
                    // analysis's largest precision cost and is deliberate —
                    // the alternative claims confinement it cannot support.
                    let callee_read = operand_read_set(func, &state.holds, n_allocs);
                    changed |=
                        state.escape(&callee_read, EscapeVector::CallArgument, terminator.span);

                    let callee_name = match func {
                        Operand::Constant(MirConstant::Function(name)) => Some(name.as_str()),
                        _ => None,
                    };
                    // `snapshot()` captures whole VM state through an opaque
                    // FFI with no MIR representation — every argument escapes.
                    let opaque_callee = callee_name == Some("snapshot");
                    let summary = callee_name
                        .and_then(|name| input.callee_summaries.and_then(|m| m.get(name)));

                    for (arg_idx, arg) in args.iter().enumerate() {
                        let read = operand_read_set(arg, &state.holds, n_allocs);
                        if read.is_empty() {
                            continue;
                        }
                        let param_confined = !opaque_callee
                            && matches!(
                                summary,
                                Some(s) if arg_idx < s.closure_param_escapes.len()
                                    && !s.closure_param_escapes[arg_idx]
                            );
                        if !param_confined {
                            changed |=
                                state.escape(&read, EscapeVector::CallArgument, terminator.span);
                        }
                    }
                }
                // No operand of these carries a value out of the frame:
                // `SwitchBool` reads a bool, `Return` transfers whatever is
                // already in `SlotId(0)` (handled at its assignment).
                TerminatorKind::Goto(_)
                | TerminatorKind::SwitchBool { .. }
                | TerminatorKind::Return
                | TerminatorKind::Unreachable => {}
            }
        }
    }

    state
}

fn read_set(rvalue: &Rvalue, holds: &[AllocSet], n: usize) -> AllocSet {
    let mut set = AllocSet::with_capacity(n);
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => {
            set.union_with(&operand_read_set(op, holds, n));
        }
        // A borrow of a place makes the referent reachable through the
        // borrow's destination — that is exactly how a container-held
        // allocation leaves through `return &x`.
        Rvalue::Borrow(_, place) => {
            if let Some(h) = holds.get(place.root_local().0 as usize) {
                set.union_with(h);
            }
        }
        Rvalue::BinaryOp(_, lhs, rhs) | Rvalue::FuzzyComparison { lhs, rhs, .. } => {
            set.union_with(&operand_read_set(lhs, holds, n));
            set.union_with(&operand_read_set(rhs, holds, n));
        }
        Rvalue::Aggregate(ops) => {
            set.union_with(&operands_read_set(ops, holds, n));
        }
        Rvalue::EnumTest { operand, .. }
        | Rvalue::EnumPayload { operand, .. }
        | Rvalue::TypePatternTest { operand, .. }
        | Rvalue::EnumDiscriminantTest { operand, .. }
        | Rvalue::PrimitiveCast { operand, .. }
        | Rvalue::FormatValue { operand, .. } => {
            set.union_with(&operand_read_set(operand, holds, n));
        }
    }
    set
}

/// The allocations an rvalue's *result* is an alias of (same object), as
/// opposed to those it merely carries inside a fresh container.
fn alias_read_set(rvalue: &Rvalue, state: &OutboundState, n: usize) -> AllocSet {
    let mut set = AllocSet::with_capacity(n);
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) => {
            set.union_with(&operand_read_set(op, &state.aliases, n));
        }
        // `&x` names x.
        Rvalue::Borrow(_, place) => {
            if let Some(h) = state.aliases.get(place.root_local().0 as usize) {
                set.union_with(h);
            }
        }
        // Reading a payload back out of an enum can hand back the contained
        // object itself; `holds` (not `aliases`) is the conservative source.
        Rvalue::EnumPayload { operand, .. } => {
            set.union_with(&operand_read_set(operand, &state.holds, n));
        }
        // `Aggregate` builds a NEW container around its operands — the result
        // is not an alias of any of them. Everything else produces a fresh
        // scalar, bool or string.
        Rvalue::Aggregate(_)
        | Rvalue::BinaryOp(..)
        | Rvalue::UnaryOp(..)
        | Rvalue::FuzzyComparison { .. }
        | Rvalue::FormatValue { .. }
        | Rvalue::EnumTest { .. }
        | Rvalue::TypePatternTest { .. }
        | Rvalue::EnumDiscriminantTest { .. }
        | Rvalue::PrimitiveCast { .. } => {}
    }
    set
}

fn operands_read_set(ops: &[Operand], holds: &[AllocSet], n: usize) -> AllocSet {
    let mut set = AllocSet::with_capacity(n);
    for op in ops {
        set.union_with(&operand_read_set(op, holds, n));
    }
    set
}

fn operand_read_set(op: &Operand, holds: &[AllocSet], n: usize) -> AllocSet {
    let mut set = AllocSet::with_capacity(n);
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            if let Some(h) = holds.get(place.root_local().0 as usize) {
                set.union_with(h);
            }
        }
        Operand::Constant(_) => {}
    }
    set
}

// ── Inbound product ──────────────────────────────────────────────────────

/// For each allocation, classify every value stored into a slot that may hold
/// or contain it.
///
/// The "may hold or contain" set is an over-approximation of "stores into this
/// allocation" — a store into an enclosing container is attributed to the
/// member too. That is the sound direction: it can only fail a proof that would
/// otherwise be granted.
fn run_inbound(
    input: &EscapeInput<'_>,
    allocs: &[RawAlloc],
    state: &OutboundState,
    local_only: &[bool],
) -> Vec<InboundProof> {
    let mir = input.mir;
    let n_allocs = allocs.len();
    let mut verdict: Vec<InboundProof> = vec![InboundProof::NoForeignStores; n_allocs];

    let mut fail = |set: &AllocSet, source: ForeignSource, span: Span| {
        for idx in set.iter() {
            if verdict[idx].is_clean() {
                verdict[idx] = InboundProof::ForeignStore(source, span);
            }
        }
    };

    // Which allocations does a store into `slot` land *in*? The alias track,
    // not `holds`: writing into a container is not writing into its members.
    let reached = |slot: SlotId| -> AllocSet {
        state
            .aliases
            .get(slot.0 as usize)
            .cloned()
            .unwrap_or_else(|| AllocSet::with_capacity(n_allocs))
    };

    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::ArrayStore {
                    container_slot,
                    operands,
                }
                | StatementKind::ObjectStore {
                    container_slot,
                    operands,
                    ..
                }
                | StatementKind::EnumStore {
                    container_slot,
                    operands,
                    ..
                }
                | StatementKind::ClosureCapture {
                    closure_slot: container_slot,
                    operands,
                    ..
                } => {
                    let target = reached(*container_slot);
                    if target.is_empty() {
                        continue;
                    }
                    for op in operands {
                        if let Some(source) = classify_stored_operand(op, mir, local_only) {
                            fail(&target, source, stmt.span);
                        }
                    }
                }
                StatementKind::Assign(place, rvalue) => {
                    // Only projections write *into* an object. `Place::Local`
                    // rebinds a slot, and a bare `Place::Deref` overwrites the
                    // referent binding rather than storing into its contents —
                    // the store-through-a-reference case is
                    // `Place::Field(Deref(r), f)`, whose `root_local()` is `r`,
                    // so it arrives here.
                    let (Place::Field(base, _) | Place::Index(base, _)) = place else {
                        continue;
                    };
                    let target = reached(base.root_local());
                    if target.is_empty() {
                        continue;
                    }
                    if let Some(source) = classify_stored_rvalue(rvalue, mir, local_only) {
                        fail(&target, source, stmt.span);
                    }
                }
                StatementKind::Drop(_)
                | StatementKind::Nop
                | StatementKind::TaskBoundary(..)
                | StatementKind::ModuleBindingStore { .. } => {}
            }
        }

        // A call writing its result straight into a container field stores a
        // value of caller-invisible provenance into that container.
        if let TerminatorKind::Call { destination, .. } = &block.terminator.kind {
            if matches!(destination, Place::Field(..) | Place::Index(..)) {
                let target = reached(destination.root_local());
                if !target.is_empty() {
                    fail(&target, ForeignSource::CallResult, block.terminator.span);
                }
            }
        }
    }

    verdict
}

/// Classify a value being stored into a tracked allocation.
///
/// Returns `None` when the value is provably not an outside cycle-capable
/// value, `Some(source)` otherwise. A whitelist: anything unrecognized is
/// foreign.
///
/// Cycle-capability itself is a runtime `NativeKind`/`HeapKind` property
/// (`shape-value/src/gc.rs::cycle_capable_direct_header`) that MIR does not
/// carry — `LocalTypeInfo` distinguishes only Copy / NonCopy / Unknown. The
/// analysis therefore treats every non-`Copy`, non-local value as potentially
/// cycle-capable. That over-approximates the offending set, which can only
/// withhold the exemption, never grant it wrongly.
fn classify_stored_operand(
    op: &Operand,
    mir: &MirFunction,
    local_only: &[bool],
) -> Option<ForeignSource> {
    match op {
        Operand::Constant(c) => {
            if constant_is_leaf_literal(c) {
                None
            } else {
                Some(ForeignSource::OpaqueConstant)
            }
        }
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => match place {
            Place::Local(slot) => {
                let idx = slot.0 as usize;
                if mir.local_types.get(idx) == Some(&LocalTypeInfo::Copy) {
                    // A scalar cannot hold a heap edge, so it cannot be a
                    // cycle participant regardless of where it came from.
                    return None;
                }
                if local_only.get(idx).copied().unwrap_or(false) {
                    None
                } else if mir.param_slots.contains(slot) {
                    Some(ForeignSource::Parameter)
                } else {
                    Some(ForeignSource::ForeignSlot)
                }
            }
            Place::Field(..) | Place::Index(..) | Place::Deref(..) => {
                Some(ForeignSource::Projection)
            }
        },
    }
}

fn classify_stored_rvalue(
    rvalue: &Rvalue,
    mir: &MirFunction,
    local_only: &[bool],
) -> Option<ForeignSource> {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) => classify_stored_operand(op, mir, local_only),
        Rvalue::Borrow(_, place) => {
            let root = place.root_local();
            if local_only.get(root.0 as usize).copied().unwrap_or(false) {
                None
            } else {
                Some(ForeignSource::ForeignSlot)
            }
        }
        Rvalue::BinaryOp(..)
        | Rvalue::UnaryOp(..)
        | Rvalue::FuzzyComparison { .. }
        | Rvalue::FormatValue { .. }
        | Rvalue::EnumTest { .. }
        | Rvalue::TypePatternTest { .. }
        | Rvalue::EnumDiscriminantTest { .. }
        | Rvalue::PrimitiveCast { .. } => None,
        Rvalue::EnumPayload { operand, .. } => classify_stored_operand(operand, mir, local_only),
        Rvalue::Aggregate(ops) => ops
            .iter()
            .find_map(|op| classify_stored_operand(op, mir, local_only)),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Item;

    /// Lower one function from Shape source and run both products over it.
    ///
    /// Fixtures are real source through the real lowering path (the
    /// `return_ownership.rs` `infer_from_source` pattern) rather than
    /// hand-built MIR: an analysis that agrees with hand-built MIR but not with
    /// the shapes lowering actually emits proves nothing.
    fn facts_of(code: &str, func: &str) -> (EscapeFacts, MirFunction) {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        for item in &program.items {
            let Item::Function(def, _) = item else {
                continue;
            };
            if def.name != func {
                continue;
            }
            let lowering = crate::mir::lowering::lower_function_detailed(
                &def.name,
                &def.params,
                &def.body,
                def.name_span,
            );
            let facts = analyze_escapes(&EscapeInput {
                mir: &lowering.mir,
                analysis: None,
                had_fallbacks: lowering.had_fallbacks,
                callee_summaries: None,
            });
            return (facts, lowering.mir);
        }
        panic!("function `{func}` not found in fixture");
    }

    /// The allocation whose destination slot is the n-th allocation site in
    /// source order.
    fn alloc(facts: &EscapeFacts, index: usize) -> &AllocationFacts {
        facts
            .allocations
            .get(index)
            .unwrap_or_else(|| panic!("no allocation #{index}; facts = {facts:#?}"))
    }

    fn assert_refuses_confinement(facts: &EscapeFacts, index: usize, what: &str) {
        let a = alloc(facts, index);
        assert!(
            !a.outbound.is_frame_confined(),
            "{what}: allocation {:?} was claimed frame-confined; outbound = {:?}",
            a.site,
            a.outbound
        );
        assert!(
            a.inbound.is_clean() || !a.inbound.is_clean(),
            "inbound is a total function"
        );
        assert_eq!(
            facts.region_exemption_candidates().count(),
            facts
                .allocations
                .iter()
                .filter(|x| x.outbound.is_frame_confined() && x.inbound.is_clean())
                .count(),
            "{what}: exemption query must be the conjunction"
        );
    }

    // ── Positive controls ────────────────────────────────────────────────
    //
    // A negative-control suite passes vacuously if the analysis never proves
    // anything. These fixtures assert the analysis has discrimination power.

    #[test]
    fn local_array_literal_is_frame_confined() {
        let (facts, _) = facts_of("fn f() { let a = [1, 2, 3] }", "f");
        assert_eq!(facts.allocations.len(), 1, "facts = {facts:#?}");
        let a = alloc(&facts, 0);
        assert_eq!(a.kind, AllocKind::Array);
        assert_eq!(a.outbound, OutboundEscape::FrameConfined);
        assert_eq!(a.inbound, InboundProof::NoForeignStores);
        assert_eq!(facts.precision(), (1, 1, 1));
    }

    #[test]
    fn local_object_literal_is_frame_confined() {
        let (facts, _) = facts_of("fn f() { let o = { x: 1, y: 2 } }", "f");
        let a = facts
            .allocations
            .iter()
            .find(|a| a.kind == AllocKind::Object)
            .unwrap_or_else(|| panic!("no object allocation; facts = {facts:#?}"));
        assert_eq!(a.outbound, OutboundEscape::FrameConfined);
        assert_eq!(a.inbound, InboundProof::NoForeignStores);
    }

    #[test]
    fn array_nested_in_non_escaping_local_container_stays_confined() {
        // The precision the arena consumer needs: containment alone is not
        // escape. Both the inner and the outer array die with the frame.
        let (facts, _) = facts_of("fn f() { let inner = [1, 2]; let outer = [inner] }", "f");
        assert_eq!(facts.allocations.len(), 2, "facts = {facts:#?}");
        for a in &facts.allocations {
            assert_eq!(
                a.outbound,
                OutboundEscape::FrameConfined,
                "allocation {:?} should be confined",
                a.site
            );
        }
    }

    // ── Tripwire 1: five known-escaping negative controls ────────────────

    #[test]
    fn tripwire_return_refuses_frame_confinement() {
        let (facts, _) = facts_of("fn f() -> Array<int> { let a = [1, 2, 3]; a }", "f");
        assert_refuses_confinement(&facts, 0, "return");
        assert!(
            matches!(
                alloc(&facts, 0).outbound,
                OutboundEscape::Escapes(EscapeVector::Return, _)
            ),
            "outbound = {:?}",
            alloc(&facts, 0).outbound
        );
    }

    #[test]
    fn tripwire_closure_capture_refuses_frame_confinement() {
        // The array is captured into a closure that is itself returned, so the
        // capture outlives the frame.
        let (facts, _) = facts_of(
            "fn f() { let a = [1, 2, 3]; let g = || { a }; return g }",
            "f",
        );
        let array = facts
            .allocations
            .iter()
            .find(|x| x.kind == AllocKind::Array)
            .unwrap_or_else(|| panic!("no array allocation; facts = {facts:#?}"));
        assert!(
            !array.outbound.is_frame_confined(),
            "captured-then-returned array claimed confined: {:?}",
            array.outbound
        );
    }

    #[test]
    fn tripwire_module_binding_store_refuses_frame_confinement() {
        // `g` has no local slot inside `f`, so the assignment lowers to
        // `StatementKind::ModuleBindingStore` (lowering/expr.rs:2149).
        let (facts, mir) = facts_of("fn f() { let a = [1, 2, 3]; g = a }", "f");
        assert!(
            mir.iter_blocks().any(|b| b
                .statements
                .iter()
                .any(|s| matches!(s.kind, StatementKind::ModuleBindingStore { .. }))),
            "fixture did not lower to a ModuleBindingStore"
        );
        assert_refuses_confinement(&facts, 0, "module store");
        assert!(matches!(
            alloc(&facts, 0).outbound,
            OutboundEscape::Escapes(EscapeVector::ModuleStore, _)
        ));
    }

    #[test]
    fn tripwire_container_insert_into_escaping_container_refuses_frame_confinement() {
        let (facts, _) = facts_of(
            "fn f() -> Array<Array<int>> { let inner = [1, 2]; let outer = [inner]; outer }",
            "f",
        );
        assert_eq!(facts.allocations.len(), 2, "facts = {facts:#?}");
        for a in &facts.allocations {
            assert!(
                !a.outbound.is_frame_confined(),
                "allocation {:?} claimed confined despite the container escaping: {:?}",
                a.site,
                a.outbound
            );
        }
    }

    #[test]
    fn tripwire_task_boundary_refuses_frame_confinement() {
        let (facts, mir) = facts_of("async fn f() { let a = [1, 2, 3]; async let fut = a }", "f");
        assert!(
            mir.iter_blocks().any(|b| b
                .statements
                .iter()
                .any(|s| matches!(s.kind, StatementKind::TaskBoundary(..)))),
            "fixture did not lower to a TaskBoundary"
        );
        assert_refuses_confinement(&facts, 0, "task spawn");
    }

    #[test]
    fn callee_summary_keeps_a_non_escaping_argument_confined() {
        // The refinement hook the storage planner already feeds
        // (`StoragePlannerInput::callee_summaries`): when the callee's own
        // summary proves the parameter does not escape, passing an allocation
        // to it is not a vector. This is the seam a per-builtin-method escape
        // contract would extend — the verdict comes from the callee's analyzed
        // body, never from the callee's spelling.
        let program = shape_ast::parser::parse_program("fn f() { let a = [1, 2, 3]; sink(a) }")
            .expect("parse failed");
        let Item::Function(def, _) = &program.items[0] else {
            panic!("expected a function item");
        };
        let lowering = crate::mir::lowering::lower_function_detailed(
            &def.name,
            &def.params,
            &def.body,
            def.name_span,
        );

        let mut summaries = HashMap::new();
        summaries.insert(
            "sink".to_string(),
            FunctionBorrowSummary {
                param_borrows: vec![None],
                conflict_pairs: Vec::new(),
                return_summary: None,
                return_ownership_mode: crate::mir::ReturnOwnershipMode::Unknown,
                closure_param_escapes: vec![false],
            },
        );

        let with_summary = analyze_escapes(&EscapeInput {
            mir: &lowering.mir,
            analysis: None,
            had_fallbacks: false,
            callee_summaries: Some(&summaries),
        });
        assert_eq!(
            with_summary.allocations[0].outbound,
            OutboundEscape::FrameConfined,
            "a callee summary proving the parameter non-escaping was ignored"
        );

        // Without it, the conservative verdict stands.
        let without = analyze_escapes(&EscapeInput {
            mir: &lowering.mir,
            analysis: None,
            had_fallbacks: false,
            callee_summaries: None,
        });
        assert!(!without.allocations[0].outbound.is_frame_confined());
    }

    #[test]
    fn tripwire_call_argument_refuses_frame_confinement() {
        // The JIT prior art's "not passed to any call" criterion, carried over.
        let (facts, _) = facts_of("fn f() { let a = [1, 2, 3]; sink(a) }", "f");
        assert_refuses_confinement(&facts, 0, "call argument");
        assert!(matches!(
            alloc(&facts, 0).outbound,
            OutboundEscape::Escapes(EscapeVector::CallArgument, _)
        ));
    }

    // ── Tripwire 2: the B0004 container-referent false-positive class ────

    #[test]
    fn tripwire_b0004_container_referent_class_is_seen_through() {
        // The class that disqualified `detect_escape_status` for the storage
        // planner's Rule 3c, in the direction that matters for a confinement
        // claim: a value that leaves the frame *through a container*.
        //
        // `slot_flows_to_return` only follows `Assign(Place::Local(d), rv)`
        // edges, so an element written through `a[0] = inner` — an
        // `Assign(Place::Index(..), ..)` — is invisible to it, and it reports
        // the escaping `inner` as `Local`. This analysis must see through the
        // container membership and refuse confinement. The test pins BOTH
        // sides: if the heuristic is ever fixed, the first assertion fails and
        // the fixture must be re-pointed at a still-missed shape, not deleted.
        let code = "fn f() -> Array<int> { let a = [0]; let inner = [1, 2]; a[0] = inner; a }";
        let (facts, mir) = facts_of(code, "f");

        let inner = facts
            .allocations
            .iter()
            .find(|x| x.span.start > facts.allocations[0].span.start)
            .unwrap_or_else(|| panic!("no second allocation; facts = {facts:#?}"));

        let heuristic = crate::mir::storage_planning::detect_escape_status(
            inner.dest,
            &mir,
            &std::collections::HashSet::new(),
        );
        assert_eq!(
            heuristic,
            crate::type_tracking::EscapeStatus::Local,
            "fixture no longer reproduces the sink-blind heuristic's blind spot; \
             if the heuristic was fixed, this test must be re-pointed, not deleted"
        );

        assert!(
            !inner.outbound.is_frame_confined(),
            "a value reachable only through an escaping container was claimed \
             frame-confined — the B0004 container class regressed: {:?}",
            inner.outbound
        );
        // And the container itself, which the heuristic does catch.
        assert!(!alloc(&facts, 0).outbound.is_frame_confined());
    }

    #[test]
    fn tripwire_b0004_container_referent_stays_confined_when_nothing_escapes() {
        // The heuristic's own false positive, kept as a discrimination check:
        // it counts `Rvalue::Aggregate [&x]` as return-flow, so a referent held
        // by a purely local container is reported `Escaped` when a container
        // holding it is later returned. This analysis distinguishes the two
        // cases — nothing leaves here, so the referent is confined.
        let (facts, _) = facts_of("fn f() { let x = [1, 2]; let c = [&x] }", "f");
        assert_eq!(facts.allocations.len(), 2, "facts = {facts:#?}");
        for a in &facts.allocations {
            assert_eq!(
                a.outbound,
                OutboundEscape::FrameConfined,
                "allocation {:?} should be confined — nothing leaves this frame",
                a.site
            );
        }
    }

    #[test]
    fn tripwire_b0004_reference_in_escaping_container_is_seen_through() {
        // The literal `Rvalue::Aggregate [&x]` shape the Rule 3c comment names.
        // A reference to the local array is placed in a container that is
        // returned; the referent cannot be frame-confined.
        let (facts, _) = facts_of("fn f() { let a = [1, 2]; let c = [&a]; return c }", "f");
        let array = facts
            .allocations
            .iter()
            .find(|x| x.kind == AllocKind::Array)
            .expect("array allocation");
        assert!(
            !array.outbound.is_frame_confined(),
            "referent of a reference held by an escaping container claimed confined: {:?}",
            array.outbound
        );
    }

    // ── Tripwire 3: inbound negative controls ────────────────────────────

    #[test]
    fn tripwire_inbound_parameter_stored_into_local_object_fails_exemption() {
        let (facts, _) = facts_of("fn f(p: Array<int>) { let o = { held: p } }", "f");
        let obj = facts
            .allocations
            .iter()
            .find(|a| a.kind == AllocKind::Object)
            .unwrap_or_else(|| panic!("no object allocation; facts = {facts:#?}"));
        assert!(
            !obj.inbound.is_clean(),
            "an outside value stored into the object passed the inbound proof: {:?}",
            obj.inbound
        );
        assert_eq!(
            facts.region_exemption_candidates().count(),
            0,
            "exemption granted without the inbound proof"
        );
    }

    #[test]
    fn tripwire_inbound_call_result_stored_into_local_container_fails_exemption() {
        let (facts, _) = facts_of("fn f() { let v = make(); let a = [v] }", "f");
        let array = facts
            .allocations
            .iter()
            .find(|a| a.kind == AllocKind::Array)
            .unwrap_or_else(|| panic!("no array allocation; facts = {facts:#?}"));
        assert!(
            !array.inbound.is_clean(),
            "a call result stored into the array passed the inbound proof: {:?}",
            array.inbound
        );
    }

    #[test]
    fn tripwire_inbound_module_read_stored_into_local_container_fails_exemption() {
        // A module-level identifier read lowers to `MirConstant::Function(name)`
        // (lowering/expr.rs:2014) — indistinguishable from a function ref, so
        // it must not be classified as an inert literal.
        let (facts, _) = facts_of("fn f() { let a = [some_module_binding] }", "f");
        let array = facts
            .allocations
            .iter()
            .find(|a| a.kind == AllocKind::Array)
            .unwrap_or_else(|| panic!("no array allocation; facts = {facts:#?}"));
        assert!(
            matches!(
                array.inbound,
                InboundProof::ForeignStore(ForeignSource::OpaqueConstant, _)
                    | InboundProof::ForeignStore(ForeignSource::ForeignSlot, _)
            ),
            "module-resident value stored into the array passed the inbound proof: {:?}",
            array.inbound
        );
    }

    // ── Charter precision measurement (R24: no measurement, no close) ────

    /// Analysis precision on the committed comparison suite.
    ///
    /// Prints a per-workload table (`cargo test -p shape-vm --lib
    /// precision_report_on_charter_workloads -- --nocapture`) and asserts only
    /// non-vacuity: every allocation the suite contains receives a verdict, and
    /// the suite does contain allocations. No precision *bar* is asserted —
    /// a ratchet on these numbers needs its own ratification, and the point of
    /// the measurement is that the number reported to #195 is the honest one.
    #[test]
    fn precision_report_on_charter_workloads() {
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/charter/shape"
        );
        let workloads = [
            "alloc_tree.shape",
            "alloc_object_graph.shape",
            "collections_pipeline.shape",
            "collections_hashmap.shape",
            "closures_dispatch.shape",
            "strings_transform.shape",
            "json_roundtrip.shape",
            "numeric_matmul.shape",
            "numeric_spline.shape",
            "numeric_mandelbrot.shape",
            "startup_hello.shape",
        ];

        let mut suite_total = 0usize;
        let mut suite_confined = 0usize;
        let mut suite_exempt = 0usize;
        println!(
            "\n{:<28} {:>6} {:>9} {:>9}",
            "workload", "allocs", "confined", "exempt"
        );
        for name in workloads {
            let path = format!("{root}/{name}");
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("charter workload {path} unreadable: {e}"));
            let program = shape_ast::parser::parse_program(&source)
                .unwrap_or_else(|e| panic!("charter workload {name} failed to parse: {e}"));

            let (mut total, mut confined, mut exempt) = (0usize, 0usize, 0usize);
            for item in &program.items {
                let Item::Function(def, _) = item else {
                    continue;
                };
                let lowering = crate::mir::lowering::lower_function_detailed(
                    &def.name,
                    &def.params,
                    &def.body,
                    def.name_span,
                );
                let facts = analyze_escapes(&EscapeInput {
                    mir: &lowering.mir,
                    analysis: None,
                    had_fallbacks: lowering.had_fallbacks,
                    callee_summaries: None,
                });
                let (c, e, t) = facts.precision();
                total += t;
                confined += c;
                exempt += e;
                // Every allocation carries exactly one verdict from each
                // product — no site is left unclassified.
                assert_eq!(
                    facts.allocations.len(),
                    t,
                    "{name}::{} left allocations unclassified",
                    def.name
                );
            }
            let pct = |x: usize| {
                if total == 0 {
                    0.0
                } else {
                    100.0 * x as f64 / total as f64
                }
            };
            println!(
                "{name:<28} {total:>6} {confined:>6} {:>2.0}% {exempt:>6} {:>2.0}%",
                pct(confined),
                pct(exempt)
            );
            suite_total += total;
            suite_confined += confined;
            suite_exempt += exempt;
        }
        println!(
            "{:<28} {suite_total:>6} {suite_confined:>6} {:>2.0}% {suite_exempt:>6} {:>2.0}%\n",
            "SUITE",
            100.0 * suite_confined as f64 / suite_total as f64,
            100.0 * suite_exempt as f64 / suite_total as f64
        );
        assert!(
            suite_total > 0,
            "the charter suite produced no allocation sites — the measurement is vacuous"
        );
    }

    #[test]
    fn inbound_scalar_stores_stay_clean() {
        let (facts, _) = facts_of("fn f() { let n = 7; let a = [n, 1, 2] }", "f");
        let array = facts
            .allocations
            .iter()
            .find(|a| a.kind == AllocKind::Array)
            .expect("array allocation");
        assert_eq!(array.inbound, InboundProof::NoForeignStores);
    }

    #[test]
    fn region_exemption_requires_both_products() {
        // Outbound-confined but inbound-dirty: eligible for neither the
        // exemption nor a silent downgrade to the outbound half.
        let (facts, _) = facts_of("fn f(p: Array<int>) { let o = { held: p } }", "f");
        let obj = facts
            .allocations
            .iter()
            .find(|a| a.kind == AllocKind::Object)
            .expect("object allocation");
        assert!(obj.outbound.is_frame_confined());
        assert!(!obj.inbound.is_clean());
        assert_eq!(facts.frame_confined().count(), 1);
        assert_eq!(facts.region_exemption_candidates().count(), 0);
    }

    // ── Determinism (#205) ───────────────────────────────────────────────

    #[test]
    fn facts_are_identical_across_repeated_analyses() {
        let code = r#"
            fn f(p: Array<int>) -> Array<int> {
                let a = [1, 2, 3]
                let b = { held: a, other: p }
                let c = [b]
                let d = [9, 8]
                sink(d)
                return a
            }
        "#;
        let (first, _) = facts_of(code, "f");
        assert!(first.allocations.len() >= 4, "facts = {first:#?}");
        for _ in 0..16 {
            let (again, _) = facts_of(code, "f");
            assert_eq!(first, again, "escape facts differ between runs");
        }
    }

    #[test]
    fn allocations_are_ordered_by_site_not_by_hash_iteration() {
        let code = r#"
            fn f() {
                let a = [1]
                let b = [2]
                let c = [3]
                let d = { x: 1 }
            }
        "#;
        let (facts, _) = facts_of(code, "f");
        let sites: Vec<AllocSite> = facts.allocations.iter().map(|a| a.site).collect();
        let mut sorted = sites.clone();
        sorted.sort();
        assert_eq!(sites, sorted, "allocation order is not the site order");
    }

    // ── Soundness precondition ───────────────────────────────────────────

    #[test]
    fn lowering_fallback_marks_every_allocation_not_proven() {
        let program =
            shape_ast::parser::parse_program("fn f() { let a = [1, 2, 3] }").expect("parse failed");
        let Item::Function(def, _) = &program.items[0] else {
            panic!("expected a function item");
        };
        let lowering = crate::mir::lowering::lower_function_detailed(
            &def.name,
            &def.params,
            &def.body,
            def.name_span,
        );
        let facts = analyze_escapes(&EscapeInput {
            mir: &lowering.mir,
            analysis: None,
            // The same MIR, analyzed as if lowering had fallen back.
            had_fallbacks: true,
            callee_summaries: None,
        });
        assert!(!facts.allocations.is_empty());
        for a in &facts.allocations {
            assert_eq!(
                a.outbound,
                OutboundEscape::NotProven(NotProvenReason::MirLoweringIncomplete)
            );
            assert_eq!(
                a.inbound,
                InboundProof::NotProven(NotProvenReason::MirLoweringIncomplete)
            );
        }
        assert_eq!(facts.frame_confined().count(), 0);
        assert_eq!(facts.region_exemption_candidates().count(), 0);
    }

    #[test]
    fn alloc_set_tracks_membership() {
        let mut set = AllocSet::with_capacity(130);
        assert!(set.is_empty());
        assert!(set.insert(0));
        assert!(set.insert(129));
        assert!(!set.insert(129));
        assert!(set.contains(0));
        assert!(set.contains(129));
        assert!(!set.contains(64));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![0, 129]);
    }
}
