# Facet: borrow-flip — escape→RC promotion of reference escapes

> Design section for v0.3.3 reference-serialization. FACET = borrow-flip.
> Every claim cites `file:line` at workspace HEAD (main @ 67768f17).

## 0. Thesis (restated, scoped)

Today the borrow solver **rejects** a reference that would outlive its owner via
`BorrowErrorKind::ReferenceEscape` (B0003) and the aggregate/module sink kinds.
The v0.3.3 flip: for the **return-slot** and **module-binding-store** escape
sinks, instead of emitting B0003, **promote the referent** to a reference-counted
heap binding (force its `BindingStorageClass` to a shared/RC class so it is never
dropped while the reference is live) and let the reference's lifetime be extended
to cover the escape. Serialization then encodes the reference as an
identity-handle into the referent (handled by the snapshot facet — out of scope
here except for the contract it imposes on this facet).

The whole VM moves as one unit at snapshot→resume, so `&mut` exclusivity is
preserved across the serialize/restore boundary by construction. The cross-node
live-coherence problem (move-on-send) is **not** in scope (that is the v0.4
live-distributed-sharing feature).

## 1. Where B0003 fires today

There are **two distinct emission paths** for reference-escape, both ultimately
mapping to `BorrowErrorCode::B0003` (`mir/analysis.rs:240`, the `ReferenceEscape |
… | ReferenceEscapeIntoModuleBinding => B0003` arm):

### 1a. The `escaped_loans` direct path

`escaped_loans: Vec<(u32, Span)>` (`mir/solver.rs:87`) is drained in
`solver.rs:1146-1160`:

```rust
let mut seen_escapes = std::collections::HashSet::new();
for (loan_id, span) in &facts.escaped_loans {
    if !seen_escapes.insert((*loan_id, span.start, span.end)) { continue; }
    let info = &facts.loan_info[loan_id];
    errors.push(BorrowError {
        kind: BorrowErrorKind::ReferenceEscape,   // <-- B0003
        span: *span,
        conflicting_loan: LoanId(*loan_id),
        loan_span: info.span,
        last_use_span: last_use_span_for_loan(facts, *loan_id),
        repairs: Vec::new(),
    });
}
```

`escaped_loans` is populated at exactly three sites, all in `extract_facts`:

| Site | `solver.rs` line | Condition |
|------|------------------|-----------|
| Return-slot via `Assign(dest, Borrow)` directly into `SlotId(0)` | `215` | `*slot == SlotId(0)` and `safe_reference_summary_for_borrow(...)` returned `None` (i.e. the borrow root is **not** a parameter) |
| Return-slot via later `Assign(SlotId(0), rvalue)` aliasing a local loan | `290` | `local_loans_from_rvalue(...)` yields a loan whose root is a **local**, not a param |
| Module-binding-store | `461` | any local loan flows into `StatementKind::ModuleBindingStore` |

Note the asymmetry already baked in: **parameter** borrows that flow to the
return slot do **not** escape — they become `return_reference_candidates`
(`solver.rs:211-213`, `285-288`) and are tracked as a `ReturnReferenceSummary`
(the function safely returns one of its reference params). Only **local-rooted**
loans hit `escaped_loans`. This is the existing "genuine escape" classifier; the
flip operates on precisely this set.

### 1b. The `loan_sinks` path (aggregate + closure + task + module)

`loan_sinks: Vec<LoanSink>` (`solver.rs:89`) is drained in `solver.rs:1162-1225`.
The relevant `match sink.kind` arms (`solver.rs:1181-1215`):

```rust
LoanSinkKind::ReturnSlot => continue,                       // already non-erroring here (1a owns it)
LoanSinkKind::ClosureEnv if sink_is_local => continue,
LoanSinkKind::ClosureEnv => ReferenceEscapeIntoClosure,     // B0003
LoanSinkKind::ClosureEnvMut => continue,
LoanSinkKind::ArrayStore | ArrayAssignment if sink_is_local => continue,
LoanSinkKind::ArrayStore | ArrayAssignment => ReferenceStoredInArray,   // B0004
LoanSinkKind::ObjectStore | ObjectAssignment if sink_is_local => continue,
LoanSinkKind::ObjectStore | ObjectAssignment => ReferenceStoredInObject, // B0004
LoanSinkKind::EnumStore if sink_is_local => continue,
LoanSinkKind::EnumStore => ReferenceStoredInEnum,           // B0004
LoanSinkKind::StructuredTaskBoundary => ExclusiveRefAcrossTaskBoundary,   // B0006
LoanSinkKind::DetachedTaskBoundary if Exclusive => ExclusiveRefAcrossTaskBoundary,
LoanSinkKind::DetachedTaskBoundary => SharedRefAcrossDetachedTask,        // B0012
LoanSinkKind::ModuleBindingStore => ReferenceEscapeIntoModuleBinding,     // B0003
```

`LoanSinkKind` is defined at `mir/analysis.rs:38-64`; `BorrowErrorKind` at
`analysis.rs:131-159`; error→code map at `analysis.rs:235-250`.

The compiler turns each `BorrowError` into a hard compile error in
`compiler/functions.rs:613-636` (message table) — there is no soft-fail path; a
single B0003 in the function aborts compilation.

## 2. Which sinks flip to PROMOTE vs which stay hard-reject

The flip is **deliberately narrow** and tracks the thesis's scope (snapshot moves
the whole VM as one unit). Classify the seven escape sinks:

| Sink kind | Today | v0.3.3 disposition | Rationale |
|-----------|-------|--------------------|-----------|
| `ReturnSlot` (local-rooted) — paths 1a (`escaped_loans`) | B0003 | **PROMOTE** | The escaping reference outlives the frame; its referent is forced RC and the lifetime is extended. The returned value becomes an identity-handle into the RC'd referent. This is the headline case (`let r = &local; return r`). |
| `ModuleBindingStore` — paths 1a + 1b | B0003 | **PROMOTE** | Module bindings outlive every frame (the c6 comment at `solver.rs:452-459`). Promoting the referent to module-lifetime RC storage makes the store sound: `module_g = &local` extends `local`'s lifetime to the module binding. |
| `ClosureEnv` | B0003 (`ReferenceEscapeIntoClosure`) | **PROMOTE** | This is already escape→RC's home turf: closure capture is the canonical escape kind that **promotes** today (`storage_planning.rs:945-947`, mutable capture → `UniqueHeap`). The reference-into-closure case is the gap where capture-by-reference was rejected instead of promoting the referent. Flipping it unifies the two. *(See §2.1 caveat.)* |
| `ArrayStore` / `ArrayAssignment` | B0004 (`ReferenceStoredInArray`) | **HARD-REJECT (unchanged)** | A reference stored in a heterogeneous container is the `HeapKind::Reference`-in-array shape the snapshot facet flags as the hardest identity case (`snapshot.rs:507-512`, `ReferenceOpaque`). The container's element kind would have to be `NativeKind::Ptr(HeapKind::Reference)` and the snapshot serializer cannot yet round-trip a reference *element*. Out of scope for v0.3.3. |
| `ObjectStore` / `ObjectAssignment` | B0004 (`ReferenceStoredInObject`) | **HARD-REJECT (unchanged)** | Same — reference-in-struct-field requires per-field identity-handle serialization not in v0.3.3 scope. |
| `EnumStore` | B0004 (`ReferenceStoredInEnum`) | **HARD-REJECT (unchanged)** | Same — reference-in-enum-payload. |
| `StructuredTaskBoundary` / `DetachedTaskBoundary` | B0006 / B0012 | **HARD-REJECT (unchanged)** | This is the cross-task live-coherence problem. Promotion does **not** make it sound: two live tasks sharing an `&mut` is exactly the move-on-send problem the thesis defers to v0.4. Tasks are live concurrent actors, not a single VM frozen and moved. Keep rejecting. |

**Decision rule:** a sink flips to PROMOTE iff the referent's extended lifetime
is **strictly nested in or equal to a single VM's lifetime that the snapshot
moves atomically**, AND the snapshot serializer can encode the escaping reference
as a top-level binding identity-handle (return value or module binding) rather
than as a buried container element. That is true for return-slot, module-binding,
and (with §2.1) closure-env; false for aggregates and task boundaries.

### 2.1 Closure-env caveat — recommend deferring within v0.3.3

Closure-env promotion is *conceptually* in the flip family, but it has a sharper
edge: the escaping reference lives inside the closure's captured environment, and
the snapshot serializer would have to encode the captured-cell slot as an
identity-handle (the `OwnedClosureBlock` capture cells, `executor/mod.rs:188`
`closure_heap_bits` + the `§2.7.8/Q10` parallel-kind track). That is structurally
the same as the aggregate case (handle buried in a sub-structure), not the
top-level binding case. **Recommendation: v0.3.3 promotes ReturnSlot +
ModuleBindingStore only; ClosureEnv stays B0003 and is a v0.3.4/v0.4 follow-up.**
This keeps the flip to the two sinks where the escaping reference is a *named
top-level binding* the snapshot serializer can address directly. (Flagged as an
open question; the snapshot facet owns the final call on whether closure-cell
handles are serializable in v0.3.3.)

## 3. Does the "dangling" case dissolve? Residual genuine-dangling shapes

**Claim: with the referent RC-promoted, the classic "owner dropped while
reference live" dangling case dissolves — but two residual genuine-dangling
shapes remain and MUST still hard-reject.**

The B0003 escape error today conflates *two* properties: (a) the referent's
storage lifetime is shorter than the reference's, and (b) the referent's value
identity could be invalidated. Escape→RC fixes (a) by forcing the referent onto
RC'd heap with its refcount held by the reference (referent never drops while a
reference exists — exactly the `SharedCow`/`UniqueHeap` promotion already used at
`storage_planning.rs:944-959`). It does **not** automatically fix (b). The two
residual shapes:

### Residual R1 — reference to a value that is later MOVED out

Move-out is tracked **independently** of the loan/escape machinery, in
`compute_use_after_move_errors` (`solver.rs:1696`). The borrow-after-move check at
`solver.rs:1769-1784`:

```rust
if let Some(borrowed_place) = statement_borrow_place(&stmt.kind)
    && let Some((moved_place, move_span)) =
        find_moved_place_conflict(&moved_places, borrowed_place)
{
    errors.push(BorrowError { kind: BorrowErrorKind::UseAfterMove, ... });   // B-move
}
```

This fires on `&x` *after* `x` was moved. RC-promotion does **not** save this: a
move transfers ownership of the heap value to a new binding and the old slot is
dead — the reference would point at a slot whose value identity has been
relocated. The snapshot identity-map keys on the *referent binding slot*; a moved
referent has no stable slot identity to hand-encode.

**Disposition: R1 stays a hard error.** It is genuinely dangling and orthogonal
to the escape flip — `UseAfterMove` is produced by a different solver pass
(`solver.rs:1622`, `move_errors`) and is *not* in the `escaped_loans` /
`loan_sinks` set this facet touches. No change needed; it remains correct.

There is a subtler sub-case: a referent that is RC-promoted *and then moved*. The
flip must ensure promotion does **not** silently suppress R1. Because R1 is
detected in a separate pass that does not consult `escaped_loans`, promotion in
the escape pass leaves R1 detection intact by construction. Verified: the
move-conflict pass (`solver.rs:1696-1809`) reads only `ownership_decisions` and
MIR move-transfer facts, never `escaped_loans` or `loan_sinks`.

### Residual R2 — reference whose referent RC could still reach 0 before the reference dies

Promotion holds *one* refcount via the reference. But the reference itself can be
**reassigned or dropped early** while the snapshot encodes the *binding* as live.
Concretely: the snapshot serializes the reference binding as an identity-handle
into the referent's RC'd heap cell. On restore, the identity-map (the same
mechanism as `SharedCell`, `snapshot.rs:522-529` `SharedCellOpaque`, identity
preserved on restore) must re-establish the share. If the serializer encodes the
handle but the referent cell was *not* actually promoted (because the flip's
storage-planning change missed a sink), restore would dangle.

**Disposition: R2 is dissolved by correct co-design, not by a residual reject.**
The contract this facet must guarantee to the snapshot facet: *every reference
that the snapshot encodes as an identity-handle has a referent whose
`BindingStorageClass` is an RC class (`SharedCow` or `UniqueHeap`), so its heap
cell's refcount is `>= 1` independent of the reference.* If storage-planning
promotion (§4.2) is correct, R2 cannot occur. This is the invariant the §4.3
assertion enforces.

### Summary

- Classic "owner scope ends, reference live" → **dissolved** by promotion.
- R1 (ref to moved value) → **residual genuine dangling, stays hard-error**,
  handled by the independent move pass, untouched by the flip.
- R2 (ref outlives an RC that hit 0) → **dissolved by the promotion invariant**;
  enforced by a debug assertion, not a residual reject.

## 4. Concrete change recipe

### 4.1 Solver: reclassify the two flipping sinks (don't emit, record a promotion)

The solver should **stop pushing `BorrowError` for the two flipping sinks** and
instead emit a **promotion directive** keyed by the referent's root slot, so
storage-planning can consume it.

**New fact on `BorrowFacts`** (`solver.rs:65-111`):

```rust
/// Referent root slots that must be RC-promoted because a reference to them
/// escapes via a flip-eligible sink (return-slot / module-binding). v0.3.3
/// reference-serialization. Each entry pairs the referent slot with the
/// escaping sink so storage-planning + the snapshot facet agree on the
/// identity-handle owner.
pub reference_escape_promotions: Vec<ReferenceEscapePromotion>,
```

where

```rust
pub struct ReferenceEscapePromotion {
    /// Root slot of the borrowed place (the referent to promote).
    pub referent_slot: SlotId,
    /// The escaping sink that triggered promotion.
    pub sink: LoanSinkKind,        // ReturnSlot | ModuleBindingStore only
    pub span: Span,
}
```

The referent slot is recoverable: at every push site we have `info.borrowed_place`
(`solver.rs:1099`, `1126`, `1151`) → `.root_local()` (`mir/types.rs:86`) gives the
referent's root slot. Note `reference_origin_for_place` (`solver.rs:821-832`)
already computes exactly this normalization (`ReferenceOriginRoot::Local(slot)` is
the case we promote; `Param` never reaches `escaped_loans`).

**Change at the `escaped_loans` drain (`solver.rs:1146-1160`):** split the loop.
For a loan whose sink is `ReturnSlot` or `ModuleBindingStore`, push a
`ReferenceEscapePromotion { referent_slot: info.borrowed_place.root_local(), … }`
instead of a `BorrowError`. Because `escaped_loans` currently mixes only those two
sink kinds (the three population sites at `solver.rs:215`, `290`, `461` are all
ReturnSlot or ModuleBindingStore), the entire `escaped_loans` drain flips to
promotion. Concretely:

```rust
for (loan_id, span) in &facts.escaped_loans {
    if !seen_escapes.insert((*loan_id, span.start, span.end)) { continue; }
    let info = &facts.loan_info[loan_id];
    // v0.3.3 flip: escape→RC-promote the referent instead of B0003.
    facts.reference_escape_promotions.push(ReferenceEscapePromotion {
        referent_slot: info.borrowed_place.root_local(),
        sink: /* ReturnSlot or ModuleBindingStore, see below */,
        span: *span,
    });
}
```

To carry the sink kind through, either (a) widen `escaped_loans` to
`Vec<(u32, Span, LoanSinkKind)>`, or (b) since each `escaped_loans` push is paired
1:1 with a `loan_sinks` push at the same site (`215`+`216`, `290`+`291`,
`461`+`462`), look the sink up from `loan_sinks` by `loan_id`. Option (a) is
cleaner and local.

**Change at the `loan_sinks` drain (`solver.rs:1181-1215`):** the `ReturnSlot =>
continue` arm (line 1182) stays `continue` (the `escaped_loans` path owns
return-slot promotion now). The `ModuleBindingStore` arm (lines 1212-1214)
currently emits `ReferenceEscapeIntoModuleBinding`; **change it to `continue`** —
the `escaped_loans` path (which also receives the module store at `solver.rs:461`)
now drives its promotion. Keep `ClosureEnv`, `ArrayStore`/`ArrayAssignment`,
`ObjectStore`/`ObjectAssignment`, `EnumStore`, and both task-boundary arms
**exactly as they are** (hard-reject — §2 table).

Net solver delta: `escaped_loans` → promotions; `ModuleBindingStore` sink arm →
`continue`; everything else byte-for-byte unchanged.

### 4.2 Storage-planning: consume promotions, force RC on the referent

`decide_slot_storage` (`storage_planning.rs:905-1006`) is the single decision
point. Today the storage class is chosen by the `if/else` chain at
`storage_planning.rs:931-964`. Reference-escape promotion adds a **new highest-but-
one priority rule** (above the `Direct` default, below the explicit-`Reference`
preservation so first-class reference *bindings* are unaffected):

Thread the promotion set into `StoragePlannerInput` (the struct passed as `input`)
as `reference_escape_promotions: &HashSet<SlotId>` (the set of `referent_slot`s
from §4.1). Then insert a rule between Rule 3b (`storage_planning.rs:956-959`) and
the `Direct` default (`storage_planning.rs:960-963`):

```rust
} else if input.reference_escape_promotions.contains(&slot) {
    // v0.3.3 reference-serialization flip: a reference to this slot escapes
    // via a return-slot or module-binding sink. Force the referent onto RC'd
    // heap so it is never dropped while the escaping reference is live, and
    // so the snapshot serializer can encode the reference as an
    // identity-handle into this cell. Reuses the existing escape→RC machinery
    // (same class as Rule 2 / Rule 3b) — NO new carrier.
    if is_mutated {
        // Exclusive-or-mutated referent: UniqueHeap (boxed, single owner +
        // the held reference) — matches the mutable-capture promotion at
        // Rule 2 (line 945-947).
        BindingStorageClass::UniqueHeap
    } else {
        // Shared referent: SharedCow — matches Rule 3b (line 956-959), the
        // existing escaped+aliased+mutated promotion. SharedCow's CoW
        // semantics keep `&mut` exclusivity sound under the whole-VM snapshot
        // move (the share is identity-preserved, not duplicated).
        BindingStorageClass::SharedCow
    }
}
```

**Why these two classes, not a new one.** The HARD CONSTRAINT forbids any new
`ValueWord`-shape carrier and mandates reuse of the ADR-006 escape→RC machinery.
`UniqueHeap` (`type_tracking.rs:293`) and `SharedCow` (`type_tracking.rs:294`) are
exactly the two RC heap classes the planner already emits for escape; both already
serialize via the `§2.7.7` parallel `Vec<u64>`+`Vec<NativeKind>` track and both
already have snapshot round-trip support (they are not in the `*Opaque` blocked
set, unlike `Reference`/`SharedCell`). The reference *binding* itself stays
`BindingStorageClass::Reference` — only the **referent** is promoted.

`detect_escape_status` (`storage_planning.rs:1014-1031`) already returns `Escaped`
for slots flowing to the return slot (`slot_flows_to_return`,
`storage_planning.rs:1033-1059`), so the promoted referent's `escape_status` will
correctly read `Escaped` after the flip — the existing escape plumbing recognizes
it without further change.

### 4.3 The promotion invariant (R2 enforcement)

Add a debug assertion in storage-planning (or in the snapshot facet's
reference-encoder, by agreement) that any slot the snapshot encodes as a
reference identity-handle has `storage_class ∈ {UniqueHeap, SharedCow}`. This is
the mechanical guarantee that R2 cannot occur:

```rust
debug_assert!(
    matches!(referent_class, BindingStorageClass::UniqueHeap | BindingStorageClass::SharedCow),
    "reference-escape referent slot {referent_slot:?} was not RC-promoted; \
     snapshot identity-handle would dangle (v0.3.3 R2 invariant)"
);
```

### 4.4 Snapshot handoff contract (what this facet hands the snapshot facet)

This facet does **not** change `snapshot.rs`. It hands the snapshot facet:

1. The `reference_escape_promotions` fact (referent root slots are RC-promoted;
   their heap cells have refcount `>= 1` independent of the reference).
2. The guarantee that the only `HeapKind::Reference` values the snapshot must
   serialize are (a) return values and (b) module bindings — never buried in
   array/object/enum/closure containers (those still hard-reject, §2). This lets
   the snapshot facet replace the `ReferenceOpaque` discriminator
   (`snapshot.rs:507-512`) with a top-level identity-handle keyed on the referent
   slot, reusing the `SharedCell` identity-map restore path (`snapshot.rs:522-529`,
   identity preserved on restore).

### 4.5 Compiler / diagnostics fallout

- `compiler/functions.rs:613-636`: the `ReferenceEscape` and
  `ReferenceEscapeIntoModuleBinding` message-table arms become **dead for the
  flipped sinks** but must NOT be deleted — `ClosureEnv` still produces
  `ReferenceEscapeIntoClosure` (a B0003 sibling) and the array/object/enum sinks
  still produce B0004. (If §2.1's closure deferral holds, `ReferenceEscape` itself
  becomes reachable only via no remaining sink and could be marked
  `#[allow(dead_code)]` or kept for a future re-tightening — recommend keeping.)
- `mir/repair.rs:88-110`: the `ReferenceEscape` / `ReferenceStoredIn*` repair
  suggestions ("return an owned value instead") are now wrong for the flipped
  cases (the code compiles, no repair needed). For flipped sinks no `BorrowError`
  is produced so no repair is generated — repair.rs is reached only for the
  surviving hard-reject sinks. No change needed, but verify no test asserts a
  repair on `let r = &local; return r`.

## 5. Test impact (search-only; no test authoring in this facet)

Tests asserting B0003 on `return &local` / `module_g = &local` will flip from
"expect error" to "expect success + correct snapshot round-trip". Candidate
suites: the borrow-solver unit tests in `mir/analysis.rs` (`analysis.rs:535`,
`:592` reference `ReferenceStoredInEnum` — those stay), and the c6 audit's
module-binding tests (`docs/cluster-audits/v0.3.3/06-borrow-check-bypass.md`,
referenced at `analysis.rs:55-63`). These must be re-dispositioned by the test
facet, not silently flipped.

## 6. Forbidden-pattern compliance

- No new carrier: reuses `UniqueHeap`/`SharedCow` (`type_tracking.rs:293-294`),
  the existing escape→RC classes. No `ValueWord`-shape struct, no `Bool`-default,
  no `SlotKind::Dynamic`.
- Parallel `Vec<u64>`+`Vec<NativeKind>` per §2.7.7 is inherited — both promotion
  classes already use it; the flip changes *which slots* get promoted, not the
  carrier.
- No "small fallback / one edge case" rationalization: the flip is a complete
  reclassification of two named sinks, and the two residual dangling shapes (R1
  move, R2 RC-zero) are handled by an existing independent pass and an invariant
  assertion respectively — not by a retained dynamic path.
