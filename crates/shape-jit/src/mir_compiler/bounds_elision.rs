//! MIR-level bounds-check elision analysis (ADR-018 §5).
//!
//! Identifies indexed-access **sites** whose runtime bounds check inside
//! `inline_array_get`/`inline_array_set` (legacy carrier) or
//! `v2_array_get`/`v2_array_set` (v2 `TypedArray<T>` carrier) is provably
//! redundant, and records them in a [`BoundsElisionPlan`] keyed by the exact
//! MIR statement position plus the structural shape of the access.
//!
//! # Why the plan is site-keyed
//!
//! The pre-widening plan was a set of `(arr_slot, iv_slot)` pairs consulted
//! at *every* `arr[iv]` in the function. That key is unsound the moment the
//! analyzer actually admits anything, because the loop's range fact does not
//! hold outside the loop:
//!
//! ```text
//!   while i < arr.length { acc = acc + arr[i]; i = i + 1 }
//!   acc = acc + arr[i]      // i == arr.length here — MUST stay checked
//! ```
//!
//! A pair-keyed plan trusts both accesses. The plan therefore keys on
//! `(BasicBlockId, statement index, base shape, index shape)`; the consumer in
//! `places.rs` reconstructs that key from `MirToIR::current_stmt_position`
//! plus the `Place`/`Operand` it is lowering. `current_stmt_position` is
//! `None` while terminators are lowered, so an index access embedded in a
//! terminator operand is never trusted.
//!
//! # Soundness argument
//!
//! An access `base[idx]` may skip the check only if BOTH
//!   1. `idx >= 0` — the unchecked path also skips `normalize_index`, so
//!      non-negativity is proven independently of the upper bound
//!      (`places.rs:694`); and
//!   2. `idx < base.length`.
//!
//! For a loop header `H` ending in `SwitchBool(cond, B, _)` where `cond` is
//! defined in `H` as `iv < bnd` (or `iv <= bnd`), the analyzer proves:
//!
//! - **`bnd <= base.length - slack`** for a non-negative `slack`. The bound
//!   is resolved through singly-assigned copy chains, accepting
//!   `bnd = base.length` (slack 0) and `bnd = base.length - k` (slack `k`,
//!   `k >= 0`), or the comparison reading `base.length` inline. `Le` headers
//!   contribute `slack - 1`, since `iv <= bnd` admits `iv == bnd`.
//! - **`iv >= s`** for a non-negative constant `s`: `iv` has exactly one
//!   assignment outside the loop, resolving through copy chains to
//!   `Constant(Int(s))` with `s >= 0`, and every assignment inside the loop
//!   is `iv = iv + c` with `c >= 0` (directly, or through a singly-assigned
//!   temporary — the shape MIR lowering actually emits).
//! - **`base` is stable**: not reassigned, not borrowed, not escaped into a
//!   container/closure/task/module binding, and not reachable by any call
//!   inside the loop. Length-mutating operations would otherwise invalidate
//!   the bound between its capture and the access.
//!
//! Inside `B` (the header's true successor, which the header test dominates)
//! `s <= iv <= bnd - 1 <= base.length - slack - 1`, hence
//! `base.length >= s + slack + 1`. That yields the three admitted index
//! shapes:
//!
//! | shape | admitted when | proof |
//! |---|---|---|
//! | `base[iv]` | `slack >= 0` | `0 <= s <= iv < length` |
//! | `base[iv ± c]` | `s + o >= 0` and `o <= slack` (`o` = signed offset) | `iv + o >= s + o >= 0`; `iv + o <= length - 1 + (o - slack) < length` |
//! | `base[c]` | `0 <= c <= s + slack` | `length >= s + slack + 1 > c` |
//!
//! Index temporaries carrying an `iv`-dependent value must be defined in `B`
//! before the access and before the first assignment to `iv` in `B`, so the
//! value they hold is the `iv` the header test constrained. Pure-constant
//! index temporaries carry no such requirement.
//!
//! # Deliberate non-admissions
//!
//! - **`len(x)` call-sourced bounds.** `for x in arr` lowers its bound to a
//!   `Call` on `MirConstant::Method("len")`. Trusting it would make a method
//!   *spelling* select a memory-safety proof, which §Forbidden Patterns
//!   refuses; it needs a resolved intrinsic identity (ADR-011) instead.
//! - **Runtime preconditions / loop versioning.** A loop bounded by an
//!   unrelated parameter (`while k < n { a[k] }`) carries no static relation
//!   between `n` and `a.length`, so nothing is admitted. ADR-018 §5 requires
//!   every elision to be a static proof.

use std::collections::{HashMap, HashSet};

use shape_vm::mir::types::{
    BasicBlock, BasicBlockId, BinOp, FieldIdx, MirConstant, MirFunction, Operand, Place, Rvalue,
    SlotId, StatementKind, TerminatorKind,
};

/// Maximum copy-chain / arithmetic-tree depth the resolvers will follow.
const MAX_RESOLVE_DEPTH: usize = 8;

/// The structural shape of an indexed access's receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElisionBase {
    /// `arr[...]`
    Local(SlotId),
    /// `obj.field[...]` — the field-projected receiver of ADR-018 §5.
    Field(SlotId, FieldIdx),
}

/// The structural shape of an indexed access's index operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElisionIndex {
    /// The index is read from a local slot (the induction variable itself, or
    /// a temporary holding `iv ± c` / a constant).
    Slot(SlotId),
    /// The index is an inline integer constant operand.
    Const(i64),
}

/// A single trusted access site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessSite {
    pub block: BasicBlockId,
    pub stmt: usize,
    pub base: ElisionBase,
    pub index: ElisionIndex,
}

/// Result of bounds-elision analysis: the set of access sites that may bypass
/// the inline bounds check. Empty by default — an empty plan keeps every
/// access on the checked path.
#[derive(Debug, Clone, Default)]
pub struct BoundsElisionPlan {
    trusted: HashSet<AccessSite>,
}

impl BoundsElisionPlan {
    pub fn is_trusted_site(&self, site: &AccessSite) -> bool {
        self.trusted.contains(site)
    }

    pub fn len(&self) -> usize {
        self.trusted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.trusted.is_empty()
    }

    /// Trusted sites in a deterministic order — the assertion surface for
    /// per-fixture elision-plan tests.
    pub fn sites_sorted(&self) -> Vec<AccessSite> {
        let mut v: Vec<AccessSite> = self.trusted.iter().copied().collect();
        v.sort_by_key(|s| {
            let (bk, bs, bf) = match s.base {
                ElisionBase::Local(l) => (0u8, l.0, 0u16),
                ElisionBase::Field(o, f) => (1u8, o.0, f.0),
            };
            let (ik, iv) = match s.index {
                ElisionIndex::Slot(sl) => (0u8, sl.0 as i64),
                ElisionIndex::Const(c) => (1u8, c),
            };
            (s.block.0, s.stmt, bk, bs, bf, ik, iv)
        });
        v
    }
}

/// Project a lowered `Place::Index(base, index)` onto the plan's key space.
///
/// Shared by the analyzer and by the `places.rs` consumer so the two cannot
/// disagree about what a site *is*. Returns `None` for receiver or index
/// shapes the plan cannot name (nested indexing, deref receivers, non-integer
/// constant indices).
pub fn classify_access(base: &Place, index: &Operand) -> Option<(ElisionBase, ElisionIndex)> {
    let b = match base {
        Place::Local(s) => ElisionBase::Local(*s),
        Place::Field(inner, f) => match inner.as_ref() {
            Place::Local(s) => ElisionBase::Field(*s, *f),
            _ => return None,
        },
        _ => return None,
    };
    let i = match index {
        Operand::Copy(Place::Local(s))
        | Operand::Move(Place::Local(s))
        | Operand::MoveExplicit(Place::Local(s)) => ElisionIndex::Slot(*s),
        Operand::Constant(MirConstant::Int(v)) => ElisionIndex::Const(*v),
        _ => return None,
    };
    Some((b, i))
}

// ── Function-wide index built once per analysis ───────────────────────────

/// Where a slot is assigned, and with what.
struct SlotDef<'m> {
    block: BasicBlockId,
    stmt: usize,
    rvalue: Option<&'m Rvalue>,
}

struct Ctx<'m> {
    mir: &'m MirFunction,
    /// Every assignment to a slot: `Assign(Place::Local(s), rv)` statements
    /// plus `Call { destination: Place::Local(s) }` terminators (recorded with
    /// `rvalue: None` — an opaque producer we never resolve through).
    defs: HashMap<SlotId, Vec<SlotDef<'m>>>,
    /// Slots that are borrowed, escape into a container/closure/task/module
    /// binding, or are written through a projection. Never a stable base.
    unstable: HashSet<SlotId>,
    /// Field indices written anywhere via `Assign(Place::Field(..), _)`.
    written_fields: HashSet<FieldIdx>,
    /// `FieldIdx`es named `length` in this function's field-name table.
    length_fields: HashSet<FieldIdx>,
    preds: HashMap<BasicBlockId, Vec<BasicBlockId>>,
}

impl<'m> Ctx<'m> {
    fn build(mir: &'m MirFunction) -> Self {
        let mut defs: HashMap<SlotId, Vec<SlotDef<'m>>> = HashMap::new();
        let mut unstable: HashSet<SlotId> = HashSet::new();
        let mut written_fields: HashSet<FieldIdx> = HashSet::new();
        let mut preds: HashMap<BasicBlockId, Vec<BasicBlockId>> = HashMap::new();

        let mark_escape = |op: &Operand, set: &mut HashSet<SlotId>| {
            if let Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) = op {
                set.insert(p.root_local());
            }
        };

        for block in &mir.blocks {
            for succ in successors(&block.terminator.kind) {
                preds.entry(succ).or_default().push(block.id);
            }
            for (idx, stmt) in block.statements.iter().enumerate() {
                match &stmt.kind {
                    StatementKind::Assign(place, rv) => {
                        match place {
                            Place::Local(s) => defs.entry(*s).or_default().push(SlotDef {
                                block: block.id,
                                stmt: idx,
                                rvalue: Some(rv),
                            }),
                            Place::Field(_, f) => {
                                written_fields.insert(*f);
                                unstable.insert(place.root_local());
                            }
                            // `arr[i] = v` / `*r = v` do not change a length,
                            // but they do mean the root is mutated through a
                            // projection; treat the root as unstable only for
                            // Deref (an aliasing write we cannot track).
                            Place::Index(_, _) => {}
                            Place::Deref(_) => {
                                unstable.insert(place.root_local());
                            }
                        }
                        if let Rvalue::Borrow(_, p) = rv {
                            unstable.insert(p.root_local());
                        }
                    }
                    StatementKind::TaskBoundary(ops, _)
                    | StatementKind::ClosureCapture { operands: ops, .. }
                    | StatementKind::ArrayStore { operands: ops, .. }
                    | StatementKind::ObjectStore { operands: ops, .. }
                    | StatementKind::EnumStore { operands: ops, .. }
                    | StatementKind::ModuleBindingStore { operands: ops, .. } => {
                        for op in ops {
                            mark_escape(op, &mut unstable);
                        }
                    }
                    StatementKind::Drop(_) | StatementKind::Nop => {}
                }
            }
            if let TerminatorKind::Call { destination, .. } = &block.terminator.kind {
                match destination {
                    Place::Local(s) => defs.entry(*s).or_default().push(SlotDef {
                        block: block.id,
                        stmt: usize::MAX,
                        rvalue: None,
                    }),
                    other => {
                        unstable.insert(other.root_local());
                    }
                }
            }
        }

        let length_fields = mir
            .field_name_table
            .iter()
            .filter_map(|(idx, name)| if name == "length" { Some(*idx) } else { None })
            .collect();

        Ctx {
            mir,
            defs,
            unstable,
            written_fields,
            length_fields,
            preds,
        }
    }

    fn def_count(&self, slot: SlotId) -> usize {
        self.defs.get(&slot).map_or(0, |v| v.len())
    }

    /// The single defining site of `slot`, or `None` when it is a parameter,
    /// undefined, or assigned more than once.
    fn single_def(&self, slot: SlotId) -> Option<&SlotDef<'m>> {
        let sites = self.defs.get(&slot)?;
        if sites.len() != 1 {
            return None;
        }
        sites.first()
    }

    fn is_param(&self, slot: SlotId) -> bool {
        self.mir.param_slots.contains(&slot)
    }

    fn block(&self, id: BasicBlockId) -> Option<&'m BasicBlock> {
        self.mir.blocks.iter().find(|b| b.id == id)
    }
}

fn successors(term: &TerminatorKind) -> Vec<BasicBlockId> {
    match term {
        TerminatorKind::Goto(b) => vec![*b],
        TerminatorKind::SwitchBool {
            true_bb, false_bb, ..
        } => vec![*true_bb, *false_bb],
        TerminatorKind::Call { next, .. } => vec![*next],
        TerminatorKind::Return | TerminatorKind::Unreachable => vec![],
    }
}

fn operand_local(op: &Operand) -> Option<SlotId> {
    match op {
        Operand::Copy(Place::Local(s))
        | Operand::Move(Place::Local(s))
        | Operand::MoveExplicit(Place::Local(s)) => Some(*s),
        _ => None,
    }
}

fn operand_place(op: &Operand) -> Option<&Place> {
    match op {
        Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => Some(p),
        Operand::Constant(_) => None,
    }
}

/// All blocks of the natural loops whose header is `header` — the union over
/// back edges `T -> header` of `{header, T} ∪ {blocks reaching T without
/// passing through header}`.
fn natural_loop_blocks(ctx: &Ctx<'_>, header: BasicBlockId) -> HashSet<BasicBlockId> {
    let mut body: HashSet<BasicBlockId> = HashSet::new();
    body.insert(header);
    let latches: Vec<BasicBlockId> = ctx
        .preds
        .get(&header)
        .map(|ps| ps.iter().copied().filter(|p| p.0 >= header.0).collect())
        .unwrap_or_default();
    let mut stack: Vec<BasicBlockId> = Vec::new();
    for latch in latches {
        if body.insert(latch) {
            stack.push(latch);
        }
    }
    while let Some(b) = stack.pop() {
        if let Some(ps) = ctx.preds.get(&b) {
            for p in ps {
                if *p != header && body.insert(*p) {
                    stack.push(*p);
                }
            }
        }
    }
    body
}

// ── Fact resolvers ────────────────────────────────────────────────────────

/// `bnd <= base.length - slack`, `slack >= 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoundFact {
    base: ElisionBase,
    slack: i64,
}

/// Resolve an operand to a compile-time integer constant, following
/// singly-assigned copy chains. Multiply-assigned slots (an induction
/// variable, say) never resolve.
fn resolve_const_int(ctx: &Ctx<'_>, op: &Operand, depth: usize) -> Option<i64> {
    if depth > MAX_RESOLVE_DEPTH {
        return None;
    }
    if let Operand::Constant(MirConstant::Int(v)) = op {
        return Some(*v);
    }
    let slot = operand_local(op)?;
    let def = ctx.single_def(slot)?;
    const_int_of_rvalue(ctx, def.rvalue?, depth + 1)
}

/// Constant-fold an rvalue over singly-assigned integer copy chains.
fn const_int_of_rvalue(ctx: &Ctx<'_>, rv: &Rvalue, depth: usize) -> Option<i64> {
    if depth > MAX_RESOLVE_DEPTH {
        return None;
    }
    match rv {
        Rvalue::Use(inner) | Rvalue::Clone(inner) => resolve_const_int(ctx, inner, depth + 1),
        Rvalue::BinaryOp(BinOp::Add, a, b) => {
            resolve_const_int(ctx, a, depth + 1)?.checked_add(resolve_const_int(ctx, b, depth + 1)?)
        }
        Rvalue::BinaryOp(BinOp::Sub, a, b) => {
            resolve_const_int(ctx, a, depth + 1)?.checked_sub(resolve_const_int(ctx, b, depth + 1)?)
        }
        _ => None,
    }
}

/// Recognise `place` as `<base>.length` and project the receiver.
fn length_of_place(ctx: &Ctx<'_>, place: &Place) -> Option<ElisionBase> {
    let Place::Field(inner, f) = place else {
        return None;
    };
    if !ctx.length_fields.contains(f) {
        return None;
    }
    match inner.as_ref() {
        Place::Local(s) => Some(ElisionBase::Local(*s)),
        Place::Field(inner2, f2) => match inner2.as_ref() {
            Place::Local(s) => Some(ElisionBase::Field(*s, *f2)),
            _ => None,
        },
        _ => None,
    }
}

/// Resolve the loop's bound operand to `base.length - slack`.
fn resolve_bound(ctx: &Ctx<'_>, op: &Operand, depth: usize) -> Option<BoundFact> {
    if depth > MAX_RESOLVE_DEPTH {
        return None;
    }
    if let Some(place) = operand_place(op) {
        if let Some(base) = length_of_place(ctx, place) {
            return Some(BoundFact { base, slack: 0 });
        }
    }
    let slot = operand_local(op)?;
    // A bound held in a local must be assigned exactly once — a recomputed or
    // conditionally-assigned bound carries no invariant.
    let def = ctx.single_def(slot)?;
    match def.rvalue? {
        Rvalue::Use(inner) | Rvalue::Clone(inner) => resolve_bound(ctx, inner, depth + 1),
        Rvalue::BinaryOp(BinOp::Sub, a, b) => {
            let inner = resolve_bound(ctx, a, depth + 1)?;
            let k = resolve_const_int(ctx, b, depth + 1)?;
            if k < 0 {
                return None;
            }
            Some(BoundFact {
                base: inner.base,
                slack: inner.slack.checked_add(k)?,
            })
        }
        _ => None,
    }
}

/// What an index operand denotes at an access site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexFact {
    Const(i64),
    /// `iv + offset`
    IvOffset(i64),
}

/// Positional constraints an `iv`-dependent temporary must satisfy: its
/// definition must be in the body block, strictly before the use, and not
/// after `iv` has been stepped in that block.
#[derive(Clone, Copy)]
struct PosLimit {
    body: BasicBlockId,
    before_stmt: usize,
    iv_first_assign: usize,
}

/// Resolve an operand to `iv + offset`. Every hop must be a singly-assigned
/// temporary defined in the body block ahead of the use and ahead of the
/// induction step, so its value is the `iv` the header test constrained.
fn resolve_iv_offset(
    ctx: &Ctx<'_>,
    op: &Operand,
    iv: SlotId,
    pos: PosLimit,
    depth: usize,
) -> Option<i64> {
    if depth > MAX_RESOLVE_DEPTH {
        return None;
    }
    let slot = operand_local(op)?;
    if slot == iv {
        // Reading `iv` itself: valid as long as the read is not after the
        // induction step in this block. A read inside the stepping statement
        // still observes the pre-step value, hence `<=`.
        return if pos.before_stmt <= pos.iv_first_assign {
            Some(0)
        } else {
            None
        };
    }
    let def = ctx.single_def(slot)?;
    if def.block != pos.body || def.stmt >= pos.before_stmt || def.stmt > pos.iv_first_assign {
        return None;
    }
    let inner_pos = PosLimit {
        before_stmt: def.stmt,
        ..pos
    };
    match def.rvalue? {
        Rvalue::Use(inner) | Rvalue::Clone(inner) => {
            resolve_iv_offset(ctx, inner, iv, inner_pos, depth + 1)
        }
        Rvalue::BinaryOp(BinOp::Add, a, b) => {
            if let Some(o) = resolve_iv_offset(ctx, a, iv, inner_pos, depth + 1) {
                return o.checked_add(resolve_const_int(ctx, b, depth + 1)?);
            }
            let o = resolve_iv_offset(ctx, b, iv, inner_pos, depth + 1)?;
            o.checked_add(resolve_const_int(ctx, a, depth + 1)?)
        }
        Rvalue::BinaryOp(BinOp::Sub, a, b) => {
            let o = resolve_iv_offset(ctx, a, iv, inner_pos, depth + 1)?;
            o.checked_sub(resolve_const_int(ctx, b, depth + 1)?)
        }
        _ => None,
    }
}

fn resolve_index(ctx: &Ctx<'_>, op: &Operand, iv: SlotId, pos: PosLimit) -> Option<IndexFact> {
    if let Some(c) = resolve_const_int(ctx, op, 0) {
        return Some(IndexFact::Const(c));
    }
    resolve_iv_offset(ctx, op, iv, pos, 0).map(IndexFact::IvOffset)
}

// ── Stability and induction-variable proofs ───────────────────────────────

/// Does any `Call` terminator inside `loop_blocks` mention `slot` in its
/// callee or arguments?
fn call_in_loop_mentions(ctx: &Ctx<'_>, loop_blocks: &HashSet<BasicBlockId>, slot: SlotId) -> bool {
    ctx.mir.blocks.iter().any(|b| {
        loop_blocks.contains(&b.id)
            && match &b.terminator.kind {
                TerminatorKind::Call { func, args, .. } => std::iter::once(func)
                    .chain(args.iter())
                    .any(|op| operand_place(op).is_some_and(|p| p.root_local() == slot)),
                _ => false,
            }
    })
}

fn any_call_in_loop(ctx: &Ctx<'_>, loop_blocks: &HashSet<BasicBlockId>) -> bool {
    ctx.mir.blocks.iter().any(|b| {
        loop_blocks.contains(&b.id) && matches!(b.terminator.kind, TerminatorKind::Call { .. })
    })
}

/// The receiver's length must be invariant from the bound's capture through
/// every trusted access.
fn base_is_stable(ctx: &Ctx<'_>, base: ElisionBase, loop_blocks: &HashSet<BasicBlockId>) -> bool {
    let root = match base {
        ElisionBase::Local(s) => s,
        ElisionBase::Field(o, _) => o,
    };
    if ctx.unstable.contains(&root) {
        return false;
    }
    let max_defs = if ctx.is_param(root) { 0 } else { 1 };
    if ctx.def_count(root) > max_defs {
        return false;
    }
    if ctx
        .defs
        .get(&root)
        .is_some_and(|ds| ds.iter().any(|d| loop_blocks.contains(&d.block)))
    {
        return false;
    }
    if call_in_loop_mentions(ctx, loop_blocks, root) {
        return false;
    }
    match base {
        ElisionBase::Local(_) => true,
        ElisionBase::Field(_, f) => {
            // A field-projected receiver is reachable through any alias of the
            // owning object, so require the field to be write-free in the
            // whole function and the loop to be call-free.
            !ctx.written_fields.contains(&f) && !any_call_in_loop(ctx, loop_blocks)
        }
    }
}

/// Prove `iv >= s` for a non-negative constant `s` everywhere inside the loop.
///
/// Requires exactly one initializing assignment, outside the loop and in an
/// earlier block than the header, resolving to a non-negative constant; every
/// other assignment must be inside the loop and step by a non-negative
/// constant.
fn iv_lower_bound(
    ctx: &Ctx<'_>,
    iv: SlotId,
    header: BasicBlockId,
    loop_blocks: &HashSet<BasicBlockId>,
) -> Option<i64> {
    if ctx.is_param(iv) {
        // A parameter has no in-function initializer to inspect.
        return None;
    }
    let defs = ctx.defs.get(&iv)?;
    let mut init: Option<i64> = None;
    for def in defs {
        let rv = def.rvalue?; // an opaque call destination is never admissible
        if loop_blocks.contains(&def.block) {
            if !is_non_negative_step(ctx, rv, iv) {
                return None;
            }
        } else {
            if init.is_some() || def.block.0 >= header.0 {
                return None; // ambiguous or non-dominating initializer
            }
            let s = const_int_of_rvalue(ctx, rv, 0)?;
            if s < 0 {
                return None;
            }
            init = Some(s);
        }
    }
    init
}

/// `iv = iv + c` with `c >= 0`, directly or through a singly-assigned temp.
fn is_non_negative_step(ctx: &Ctx<'_>, rv: &Rvalue, iv: SlotId) -> bool {
    match rv {
        Rvalue::BinaryOp(BinOp::Add, a, b) => {
            (operand_local(a) == Some(iv) && resolve_const_int(ctx, b, 0).is_some_and(|c| c >= 0))
                || (operand_local(b) == Some(iv)
                    && resolve_const_int(ctx, a, 0).is_some_and(|c| c >= 0))
        }
        Rvalue::Use(op) | Rvalue::Clone(op) => {
            let Some(t) = operand_local(op) else {
                return false;
            };
            // `t = iv + c; iv = t` — the shape MIR lowering emits for
            // `i = i + 1`.
            ctx.single_def(t)
                .and_then(|d| d.rvalue)
                .is_some_and(|inner| match inner {
                    Rvalue::BinaryOp(BinOp::Add, a, b) => {
                        (operand_local(a) == Some(iv)
                            && resolve_const_int(ctx, b, 0).is_some_and(|c| c >= 0))
                            || (operand_local(b) == Some(iv)
                                && resolve_const_int(ctx, a, 0).is_some_and(|c| c >= 0))
                    }
                    _ => false,
                })
        }
        _ => false,
    }
}

/// The last `Assign(Local(cond), BinaryOp(Lt|Le, iv, bnd))` in `header`.
fn find_guard_definition(header: &BasicBlock, cond: SlotId) -> Option<(BinOp, SlotId, Operand)> {
    let mut found = None;
    for stmt in &header.statements {
        let StatementKind::Assign(Place::Local(lhs), rv) = &stmt.kind else {
            continue;
        };
        if *lhs != cond {
            continue;
        }
        found = match rv {
            Rvalue::BinaryOp(op @ (BinOp::Lt | BinOp::Le), l, r) => {
                operand_local(l).map(|iv| (*op, iv, r.clone()))
            }
            _ => None,
        };
    }
    found
}

// ── Driver ────────────────────────────────────────────────────────────────

/// Analyze a MIR function and return the set of trusted access sites.
pub fn analyze(mir: &MirFunction) -> BoundsElisionPlan {
    let mut plan = BoundsElisionPlan::default();
    let ctx = Ctx::build(mir);
    if ctx.length_fields.is_empty() {
        return plan;
    }

    for header in &mir.blocks {
        let TerminatorKind::SwitchBool {
            operand: pred_op,
            true_bb,
            ..
        } = &header.terminator.kind
        else {
            continue;
        };
        let Some(cond) = operand_local(pred_op) else {
            continue;
        };
        let Some((cmp, iv, bnd_op)) = find_guard_definition(header, cond) else {
            continue;
        };

        let loop_blocks = natural_loop_blocks(&ctx, header.id);
        if loop_blocks.len() < 2 {
            continue; // no back edge — a plain `if`, not a loop
        }
        let Some(body) = ctx.block(*true_bb) else {
            continue;
        };
        if !loop_blocks.contains(true_bb) {
            continue;
        }
        // The header must not perturb the induction variable between the test
        // and the branch.
        if block_assigns(header, iv) {
            continue;
        }

        let Some(bound) = resolve_bound(&ctx, &bnd_op, 0) else {
            continue;
        };
        // `iv <= bnd` admits `iv == bnd`, costing one element of headroom.
        let slack = match cmp {
            BinOp::Lt => bound.slack,
            _ => bound.slack - 1,
        };
        if slack < 0 {
            continue;
        }
        // The bound itself must not be recomputed inside the loop.
        if let Some(bnd_slot) = operand_local(&bnd_op) {
            if ctx
                .defs
                .get(&bnd_slot)
                .is_some_and(|ds| ds.iter().any(|d| loop_blocks.contains(&d.block)))
            {
                continue;
            }
        }
        if !base_is_stable(&ctx, bound.base, &loop_blocks) {
            continue;
        }
        let Some(iv_lower) = iv_lower_bound(&ctx, iv, header.id, &loop_blocks) else {
            continue;
        };

        let iv_first_assign = body
            .statements
            .iter()
            .position(|s| matches!(&s.kind, StatementKind::Assign(Place::Local(x), _) if *x == iv))
            .unwrap_or(usize::MAX);

        for (stmt_idx, stmt) in body.statements.iter().enumerate() {
            let pos = PosLimit {
                body: *true_bb,
                before_stmt: stmt_idx,
                iv_first_assign,
            };
            for (base_place, index_op) in index_accesses_in_statement(&stmt.kind) {
                let Some((base_key, index_key)) = classify_access(base_place, index_op) else {
                    continue;
                };
                if base_key != bound.base {
                    continue;
                }
                let Some(fact) = resolve_index(&ctx, index_op, iv, pos) else {
                    continue;
                };
                let admitted = match fact {
                    // `length >= iv_lower + slack + 1`, so every index in
                    // `0..=iv_lower + slack` is in range.
                    IndexFact::Const(c) => c >= 0 && c <= iv_lower.saturating_add(slack),
                    // `iv + o >= iv_lower + o >= 0` and `iv + o < length`.
                    IndexFact::IvOffset(o) => iv_lower.saturating_add(o) >= 0 && o <= slack,
                };
                if admitted {
                    plan.trusted.insert(AccessSite {
                        block: *true_bb,
                        stmt: stmt_idx,
                        base: base_key,
                        index: index_key,
                    });
                }
            }
        }
    }

    plan
}

fn block_assigns(block: &BasicBlock, slot: SlotId) -> bool {
    block
        .statements
        .iter()
        .any(|s| matches!(&s.kind, StatementKind::Assign(Place::Local(x), _) if *x == slot))
}

/// Every `Place::Index` reachable from a statement, as `(receiver, index)`.
fn index_accesses_in_statement(kind: &StatementKind) -> Vec<(&Place, &Operand)> {
    let mut out = Vec::new();
    match kind {
        StatementKind::Assign(place, rv) => {
            collect_place(place, &mut out);
            collect_rvalue(rv, &mut out);
        }
        StatementKind::Drop(p) => collect_place(p, &mut out),
        StatementKind::TaskBoundary(ops, _)
        | StatementKind::ClosureCapture { operands: ops, .. }
        | StatementKind::ArrayStore { operands: ops, .. }
        | StatementKind::ObjectStore { operands: ops, .. }
        | StatementKind::EnumStore { operands: ops, .. }
        | StatementKind::ModuleBindingStore { operands: ops, .. } => {
            for op in ops {
                collect_operand(op, &mut out);
            }
        }
        StatementKind::Nop => {}
    }
    out
}

fn collect_place<'a>(place: &'a Place, out: &mut Vec<(&'a Place, &'a Operand)>) {
    match place {
        Place::Local(_) => {}
        Place::Field(inner, _) | Place::Deref(inner) => collect_place(inner, out),
        Place::Index(base, index) => {
            out.push((base.as_ref(), index.as_ref()));
            collect_place(base, out);
            collect_operand(index, out);
        }
    }
}

fn collect_operand<'a>(op: &'a Operand, out: &mut Vec<(&'a Place, &'a Operand)>) {
    if let Some(p) = operand_place(op) {
        collect_place(p, out);
    }
}

fn collect_rvalue<'a>(rv: &'a Rvalue, out: &mut Vec<(&'a Place, &'a Operand)>) {
    match rv {
        Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => collect_operand(op, out),
        Rvalue::BinaryOp(_, a, b) => {
            collect_operand(a, out);
            collect_operand(b, out);
        }
        Rvalue::FuzzyComparison { lhs, rhs, .. } => {
            collect_operand(lhs, out);
            collect_operand(rhs, out);
        }
        Rvalue::Borrow(_, p) => collect_place(p, out),
        Rvalue::Aggregate(ops) => {
            for op in ops {
                collect_operand(op, out);
            }
        }
        Rvalue::EnumTest { operand, .. }
        | Rvalue::EnumPayload { operand, .. }
        | Rvalue::TypePatternTest { operand, .. }
        | Rvalue::EnumDiscriminantTest { operand, .. }
        | Rvalue::PrimitiveCast { operand, .. }
        | Rvalue::FormatValue { operand, .. } => collect_operand(operand, out),
    }
}
