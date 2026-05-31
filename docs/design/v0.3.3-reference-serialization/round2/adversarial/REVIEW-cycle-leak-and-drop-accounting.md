# Adversarial Review — lens: cycle-leak-and-drop-accounting

> Target: `docs/design/v0.3.3-reference-serialization/round2/DESIGN-ROUND2-DRAFT.md`
> Verdict: **HOLES-FOUND.** Every line:col verified against source at HEAD `67768f17`.

The design's headline cycle claim — "the only residual hazard is an Arc-cycle
**leak**, never a double-free or UAF" (§VERDICT, §6 row 5, §7) — does NOT survive
contact with the actual runtime mutation + drop paths. The "leak, not
memory-unsafety" framing assumes the cyclic structure is *built once and never
torn down or mutated*. But promoted referents are `SharedCell`s, and `SharedCell`
payloads are **mutable at runtime** through deref-store. Mutation is the move
that converts the cycle from a benign leak into a use-after-free / double-free.
The design never models the mutate path; it reasons only about the static graph.

---

## BREAK-C1 (UAF + double-free): mutating a `SharedCell` payload that holds the last share of a structure the cell transitively owns

### The drop-accounting fact the design omits

The mutate path for a promoted referent is `op_store_shared_local`
(`crates/shape-vm/src/executor/variables/mod.rs:1572-1617`) and, for the
deref-store case, `write_ref_target` (`variables/mod.rs:3025-3159`). Both end
the same way:

```rust
// op_store_shared_local — variables/mod.rs:1609-1615
let prev_bits = {
    let mut guard = cell_ref.lock();
    let prev = *guard;
    *guard = new_bits;
    prev
};
crate::executor::vm_impl::stack::drop_with_kind(prev_bits, cell_kind);   // :1615
```

When you write a new value into a `SharedCell`, the **prior occupant's share is
immediately released** via `drop_with_kind(prev_bits, cell_kind)`. This is the
correct refcount discipline for a *single* cell — but it is exactly the operation
the design's "a cycle is just a leak" argument never accounts for.

The cycle facet (§0.2, `_facet-kl2-cycle-policy.md:56-73`) states the cycle
mechanism as: "Neither last-share ever retires → neither `Drop` ever fires →
permanent leak." That is true **only while no one writes into the cell.** A
deref-store *forces* a `Drop` of the prior payload while the cycle is live.

### The breaking program

Under the broad flip, the ReturnSlot/ModuleBinding promotions are claimed
"leak-free by construction" (§1 bullet 1, cycle-facet §0.3 bullet 1). The
container flip is gated by P2. But the *mutate-into-a-cell* operation is reachable
in the **narrow, supposedly-safe** ReturnSlot/ModuleBinding scope the moment the
referent is a mutable `SharedCow` (which it must be — the promotion forces the
referent to `SharedCow`, §2.2 / closure-facet §3.2). Concretely:

```shape
let mut a = make_big_object()      // a: SharedCow cell A, holds Arc<TypedObjectStorage S>
module_ref = &a                    // ModuleBindingStore escape → PROMOTE.
                                   //   ref carrier = PromotedCell{cell: A}, A.refcount = 2
// ... later, in any frame ...
*module_ref = make_other_object()  // deref-store through the promoted ref
```

Trace the deref-store:
1. `write_ref_target` for the promoted cell → `cell.lock()`, read `prev_bits`
   (= `Arc::into_raw(Arc<TypedObjectStorage S>)`), write `new_bits`, then
   `drop_with_kind(prev_bits, Ptr(HeapKind::TypedObject))`
   (`stack.rs` SharedCell-style release; for TypedObject the arm calls
   `TypedObjectStorage::release_elem`, mirror of `heap_value.rs:3760`).
2. If `a`'s cell A held the **last** share of `S`, that release **frees `S`**.

This is fine in isolation. **But now make `S` transitively own cell A** — the
exact shape the design says only the *container* flip can build and only past a
B0004 reject:

```shape
type Node { mut peer: &Node }      // a struct field of reference type
let mut a = Node{ peer: ... }
module_ref = &a                    // PromotedCell{cell A}, A holds Arc<S_a>
a.peer = module_ref                // *** container B0004 — see BREAK-C2 for why P2 fails to stop this ***
```

Once `a.peer` (a field inside `S_a`) holds `PromotedCell{cell A}`, we have
`S_a -> A -> S_a`: a cycle. The design dispositions this as "leak." But:

```shape
*module_ref = Node{ peer: null }   // deref-store into cell A
```

Cell A's prior payload was `Arc<S_a>`. The store releases that share via
`drop_with_kind`. Because the *only other* strong share of `S_a` is the one the
cycle holds (via `a.peer -> A -> value=Arc<S_a>` — wait, that share lives inside
A's value, which we just overwrote), **the store drops `S_a`'s last external
share and then `S_a`'s own field (`a.peer`) drop walks `drop_fields`, decrementing
A** — and A is the very cell we are mid-`lock()` on. We free A's payload's owner
while A's value guard is held, and A's refcount can hit 0 inside its own
`drop_with_kind`, freeing the cell under the live `&module_ref`. The subsequent
`module_ref` access (or the closure that still holds `PromotedCell{A}`) is a
**use-after-free**, and the nested `drop_fields` over `S_a` re-enters the same
`HeapKind::SharedCell`/`Reference` release arm (`heap_value.rs:3852`/`:3893`),
producing a **double-decrement** on A.

**This is not "the cycle leaks." It is "mutating one edge of the cycle triggers a
cascading drop that re-enters a cell being held, => UAF + double-free."** The
design's entire soundness floor ("no UAF, no double-free; cycles only leak") is
predicated on the cyclic structure being immutable, which `SharedCow` referents
are categorically not.

### Why the acyclicity gate (P2) does not save this

The cycle is *assembled* by the `a.peer = module_ref` store (container flip) and
*detonated* by the `*module_ref = ...` deref-store (narrow flip). The design's P2
gate only inspects the **assembling store** at compile time. But:
- The deref-store is the narrow ReturnSlot/ModuleBinding mutate path, which the
  design explicitly does NOT gate ("leak-free by construction", no P2).
- P2 lives in the storage planner, which has **only intraprocedural per-slot
  dataflow** (`slot_flows_to_return`/`slot_holds_reference`/`slot_is_aliased`,
  `storage_planning.rs:1033-1075`). There is no transitive-ownership /
  aliasing-graph analysis. P2 cannot prove `S_a` is not a transitive owner of A
  because the necessary whole-heap reachability analysis does not exist in the
  planner and the design does not build it — it asserts "the storage planner
  proves acyclicity" (§1 P2) as though the machinery were present. It is not.

---

## BREAK-C2 (silent-wrong / leaked-unboundedly): the P2 acyclicity predicate is unspecifiable in the storage planner, so the "zero new leak surface" claim is false

The design's central scoping move (§7, cycle-facet §1.4) is: "the broad flip
promotes container/closure escapes **only where the storage planner proves
acyclicity**; where it cannot, the arm keeps rejecting." It then claims (§7) "the
broad flip ships with **zero new leak surface**."

This is verified-false at two levels:

1. **No acyclicity machinery exists, and the cited functions cannot compute it.**
   The planner's escape detection is `detect_escape_status` →
   `slot_flows_to_return` (`storage_planning.rs:1014-1061`), a per-slot DFS over
   *return-slot dataflow within one MIR function*. The acyclicity predicate P2
   needs "the stored ref's referent is not the container nor a transitive owner of
   it" — a whole-program points-to / ownership-reachability query. Nothing in
   `storage_planning.rs` computes points-to sets or transitive heap ownership. The
   design names no algorithm, cites no existing pass, and hand-waves the hardest
   part. An implementer handed this will either (a) build a points-to analysis
   (XL, undisclosed, not in the effort estimate of "M"), or (b) ship a stub
   predicate.

2. **The "safe floor" the design recommends (O8: "reject unless trivially
   acyclic — referent is a distinct, non-container scalar/object binding") does
   NOT in fact reject the BREAK-C1 shape.** In BREAK-C1, at the assembling store
   `a.peer = module_ref`, the referent `module_ref` points at `a`, and `a` is a
   distinct named binding from the field's container... no: the container IS `a`.
   But the planner sees `a.peer = <ref slot>`; the ref slot's `borrowed_place`
   root is `a`; the container root is also `a`. So a *self-store* (`a.peer = &a`)
   is catchable by a same-root check. **But the mutual case is not:**

   ```shape
   let mut a = Node{...}; let mut b = Node{...}
   module_ra = &a; module_rb = &b      // two ModuleBinding promotions (narrow, ungated)
   a.peer = module_rb                  // container store: referent root = b, container root = a → DISTINCT
   b.peer = module_ra                  // container store: referent root = a, container root = b → DISTINCT
   ```

   Both container stores pass any "referent is a distinct binding from the
   container" floor — the referent root (`b`, then `a`) is syntactically distinct
   from the container root (`a`, then `b`). Yet `A -> S_a -> b.peer -> B -> S_b ->
   a.peer -> A` is a textbook mutual Arc cycle. The "safe floor" predicate the
   design recommends as shipping "zero leak surface" admits this cycle. So the
   "zero new leak surface" claim is **false** even before considering mutation —
   and with BREAK-C1's mutation, this mutual cycle is detonatable into UAF the
   same way. The design's own O8 fallback predicate is unsound for the property
   it is supposed to guarantee.

The cycle facet concedes the residual exists ("a deliberately-constructed cycle
the conservative proof over-approximates as acyclic", §7 / cycle-facet
§1.4 last bullet) — but then asserts that residual is "rare, sound, documented."
BREAK-C1 shows the residual is **not sound** (it is UAF-reachable via mutation),
and BREAK-C2 shows it is **not rare** (any two-binding mutual reference, the most
natural way to write a doubly-linked structure, hits it under the recommended
floor).

---

## BREAK-C3 (double-free): serialization of a cyclic / aliased `PromotedCell` graph re-enters the typed-Arc recovery and double-counts

The snapshot serialize path is a **recursive tree walk with no cycle detection
and no depth guard** (`slot_to_serializable` / `slot_heap_to_serializable` /
`serializable_inner_kinded`, `snapshot.rs:843-1133`). Result/Option payloads
recurse via `serializable_inner_kinded` (`snapshot.rs:1080`, `:1093`); the
TypedObject field walk the design relies on in §2.3 ("the container field is
walked by the ordinary `TypedObject` serialize arm") **does not exist yet** — the
`HeapKind::TypedObject` case currently surface-and-stops in the `other =>` Err arm
(`snapshot.rs:1120-1127`). So the design's §2.3 single-source identity claim rests
on a serialize path that must be *newly built*, and the cycle hazard lands
squarely in that new code.

Two concrete failures:

1. **Infinite recursion / stack overflow on serialize.** Build the BREAK-C2
   mutual cycle (which passes the recommended P2 floor), call `snapshot()`. The
   new TypedObject-field serialize arm walks `a -> a.peer (PromotedCell) -> cell B
   -> b -> b.peer (PromotedCell) -> cell A -> a -> ...` with no `visited` set (none
   exists in `snapshot.rs`; grep for `visited`/`depth`/`cycle` returns only
   `call_depth` field and FilterExpr's bounded recursion). Result: stack overflow
   → abort, not the "structured Err" the design promises for cyclic shapes (§9
   N-cycle-1 claims "clean reject, never a silent leak"; but the cyclic graph
   already passed compile-time promotion, so there is no reject — it reaches the
   serializer and blows the stack).

2. **Refcount double-count across the dedup table.** The design's `heap_referents`
   dedup keys the `SharedCell` "by heap address" (§2.3, cycle-facet §4.1). The
   existing typed-Arc recovery pattern in every serialize arm is
   `Arc::from_raw(bits) … let _ = Arc::into_raw(arc)` (e.g. `snapshot.rs:917-919`,
   `:994-996`, `:1042-1044`) — reconstruct, read, *restore the share*. For a
   `PromotedCell` reference field this requires `Arc::from_raw::<RefTarget>(bits)`,
   matching on `PromotedCell{cell}`, then re-interning `cell` by address. If the
   same cell is reached via N reference holders, the dedup is supposed to write one
   token. But the *share-restoration* discipline (`Arc::into_raw` to put the share
   back) must run **once per recovery**, and the inner `Arc<SharedCell>` inside the
   `RefTarget` is borrowed, not owned-out, on each visit. The design never
   specifies the retain/release accounting for the **inner** `Arc<SharedCell>`
   during the multi-visit dedup walk — and the cluster-1.5 / W5 history (cited in
   the design itself, §2.1) is precisely the class of bug where a
   "reconstruct-read-restore" at the snapshot-clone boundary claimed a share
   without bumping the underlying refcount (see `MEMORY.md` / CLAUDE.md "v2-raw
   heap audit": `vm_state_snapshot.rs:295::clone_slot_kinded` drove a UAF at
   snapshot drop). The design asserts §2.3 is "the BREAK-4a fix… one identity
   source, deduped" but provides no per-share accounting for the nested
   `RefTarget -> SharedCell` Arc during the dedup walk. With a cycle, the cell is
   visited an unbounded number of times; any per-visit imbalance is multiplied.

---

## BREAK-C4 (broken borrow invariant after resume): live continuation re-enables a `&mut` deref on a cell whose exclusivity loan was discharged pre-snapshot

The design (§5.1) leans hard on "there is NO runtime borrow checker; resumed MIR
was already statically checked." True. But this lens is drop-accounting, and the
problem is that **the static check's premise no longer holds after live
continuation** for the mutate path.

The static B0001 exclusivity proof for `*module_ref = X` was discharged against
the *original* loan set. The proof guarantees: at this program point, no other
live loan aliases the referent. After `snapshot()` + restore + **continue**, the
design re-points `PromotedCell` references at restored cells and resumes execution
at the saved IP. But the design also flips the **container** flip, so a snapshot
can capture a state where:

- frame F1 holds `PromotedCell{cell A}` (a live `&mut a`, restored),
- and a *separately serialized* part of the heap (an object field, a second
  closure) also holds `PromotedCell{cell A}` via the SAME dedup token (§2.3 says
  N holders → one restored cell → "aliasing preserved").

Pre-snapshot, B0001 guaranteed these did not co-exist as two live `&mut`
(exclusivity). But the wire format **discards `is_mut`'s enforcement** — the
design carries `is_mut` "reserved, not read" (§5.2, round-1 §4.3). On restore,
both holders re-acquire a share of the SAME cell A. Now continuation resumes and
both can deref-store into A. The first store's `drop_with_kind(prev_bits, ...)`
releases the prior payload; the second holder still believes it observes the
pre-store value (its cached/derived state from before the snapshot point), and the
second store releases an *already-freed* payload → **double-free**, or reads a
stale pointer → **UAF**.

The design's defense (§5.2 G3) is the *same-program* resume guard — but G3 only
rejects cross-*program* resume (different content hash). It does NOT prevent
**within-the-same-program** restoration of an aliased `&mut` graph that the
original B0001 proof forbade from co-existing but the snapshot froze and the dedup
table faithfully reconstructed as two live holders. The design's §2.3 "aliasing
preserved" is in direct tension with B0001's "exclusivity": the snapshot can only
have been taken at a point where at most one `&mut` to A was live, BUT the broad
container flip means an *immutable* `&a` stored in an object field plus a live
`&mut a` in a frame is a legal, B0001-passing state (shared+exclusive is what
B0001 forbids — so this specific pair is caught). The genuinely unguarded case is
**two restored frames each carrying a live `&mut` to A via continuation of two
suspended tasks/coroutines** sharing the dedup'd cell — which the design's
KL-4/`ClosureEnvMut` exclusion is supposed to cover, but the *cell dedup* (§2.3)
silently re-establishes shared identity that the exclusion assumed impossible.

At minimum this is an **unproven invariant**: the design asserts "live continuation
re-establishes no loans" and is therefore sound, but never proves that the dedup
table's "N holders → one cell" reconstruction cannot produce a multi-`&mut`-to-one-cell
state that the original B0001 proof relied on being unreachable. The drop-accounting
on that reconstructed aliased graph is unspecified.

---

## BREAK-C5 (drop-accounting: `ClosureEnvMut` left un-flipped is incoherent with a promoted referent)

The design keeps `LoanSinkKind::ClosureEnvMut` un-flipped (§4.4,
`solver.rs:1192` `continue`) — "flipping it would re-open cross-mutation KL-4."
But the design ALSO forces the referent of an *immutable* `ClosureEnv` capture to
`SharedCow` (§4.3 Delta 1). A `SharedCow` cell is **mutable storage**. So consider
a single local `x` that is BOTH:

- captured by-`&mut` into a non-escaping closure (registers a `ClosureEnvMut`
  bookkeeping loan, `continue`, referent stays whatever storage), AND
- captured by-`&` into an *escaping* closure (triggers the `ClosureEnv` flip →
  forces `x`'s referent slot to `SharedCow`).

The §4.3-Delta-1 promotion forces `x` to `SharedCow` via the `explicit_storage`
override (`storage_planning.rs:931-933`). But the `&mut` capture's bookkeeping
(`ClosureEnvMut`) and B0001 conflict detection ran on the **pre-override** storage
model. The promotion mutates the storage class of a slot that another (un-flipped)
sink is also reasoning about, *after* B0001 already decided there was no conflict
(because the `&mut` was to a `Direct` local and the `&` to the same local was a
shared borrow — B0001 forbids exclusive+shared, so this exact pair is caught...
unless the `&mut`-capture is in a *disjoint NLL live range* from the `&`-capture,
which B0001 permits). In the disjoint-range case, B0001 passes, then the
`ClosureEnv` flip silently converts `x` to a shared mutable cell that the
`&mut`-closure now writes into via `op_store_shared_local` — but the
`&mut`-closure's capture layout was computed for a `Direct`/`UniqueHeap` capture
(Rule 2, `storage_planning.rs:945-947`), not `SharedCow`. The capture-kind track
and the cell-vs-direct dispatch now disagree for the same slot across two closures.
The design's Delta-1 override mutates storage *after* the planner already
classified the slot for the other sink. Result: one closure dispatches the slot as
`Ptr(HeapKind::SharedCell)` (cell), the other as a direct/box capture → wrong-carrier
`drop_with_kind` → wrong-type free at closure release. The design treats Delta 1 as
a local fix to one sink; it is a *whole-slot storage-class change* visible to every
sink touching that slot, and the interaction with the deliberately-un-flipped
`ClosureEnvMut` sink is unanalyzed.

---

## Summary

The design's load-bearing cycle claim — "cycles are only leaks, never UAF/double-free,
and the conservative gate ships zero new leak surface" — is broken on five counts,
all rooted in the same omission: **the design models the static reference graph but
never models mutation of `SharedCell` payloads or the drop cascade a mutation
triggers, and never specifies the acyclicity machinery it depends on (which does not
exist in the storage planner).** A `SharedCell` is mutable storage; the moment a
promoted referent is overwritten, the prior payload is force-dropped
(`variables/mod.rs:1615`, `:3147`), and in a cycle that drop cascade re-enters a
live cell — UAF + double-free, not a leak. The "M effort / construction-safe"
framing is an under-estimate: a sound version needs a whole-program ownership/points-to
analysis (undisclosed XL) just to make P2 real, plus cycle detection in the
not-yet-built TypedObject serialize path, plus a per-share accounting spec for the
nested `RefTarget -> SharedCell` Arc across the dedup walk.
