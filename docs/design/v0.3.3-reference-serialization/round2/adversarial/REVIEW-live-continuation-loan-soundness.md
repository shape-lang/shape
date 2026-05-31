# Adversarial Review — Round 2 DESIGN-ROUND2-DRAFT.md

> Lens: **live-continuation-loan-soundness**. Goal: break the broad flip +
> live continuation — find a program/case producing double-free, UAF,
> silent-wrong, unbounded leak, or a broken borrow invariant after
> resume-continuation. Every claim re-verified at workspace HEAD (`main`,
> `67768f17`).

## VERDICT: HOLES-FOUND

The draft is directionally careful and several of its negative findings are
correct (no runtime borrow checker; container-array exclusion by P1; `&mut`
closure-env not flipped). But the **live-continuation** sub-feature it markets
as "Effort S, N/A KL-4, pure value-state" is NOT sound as written, and the
broad-flip P2 acyclicity predicate it leans on does not correspond to any
machinery that exists or is designed. Five breaks below; B1, B2, B3 are
live-continuation-loan-soundness holes; B4, B5 are broad-flip soundness holes
that the live-continuation claims depend on.

---

## BREAK 1 (live-continuation, SILENT-WRONG → exclusivity violation) — resume into a NEW VM with the SAME program hash defeats the G3 guard, and re-running the snapshot point DOUBLES the `&mut` share-set

**The draft's load-bearing claim** (facet §3.3, synthesis §5.2):
> "the resumed VM does not create a *second* aliasing path to the cell (it
> restores exactly the share-set the snapshot captured — one share per
> serialized ref)."

**This is false for the `apply_pending_resume` path the feature actually runs
on.** Verified: `apply_pending_resume` (`resume.rs:110`) calls
`Self::from_snapshot(program, &snap, &store)` (`resume.rs:103`, doc) which builds
a **fresh** VM (`executor/snapshot.rs:243` `VirtualMachine::new(...)`), restores
stack/module_bindings/call_stack, and returns it. The G3 guard the draft
proposes (§5.2, facet §3.4) keys on a **program-identity content hash**: it
refuses resume when the program differs. It does NOT — and structurally cannot —
refuse resume of program A's snapshot into a *second live VM instance also
running program A*.

Concrete break:

```shape
fn build() {
    let mut x = 0
    let r = &mut x        // exclusive borrow; promotes x → SharedCell, r → PromotedCell
    module_g = r          // ModuleBindingStore escape → flips to promotion (round-1 scope)
    snapshot()            // VmState captured: module_g holds one PromotedCell share of cell_x
}
```

Now the host does what live-continuation explicitly enables (facet §2 sub-case 1
+ 2): it takes the captured `VmState` value and resumes it into **two** VMs (or
the same program loaded twice — e.g. a worker pool, the distributed-execution use
case content-addressed bytecode is built for, CLAUDE.md §Content-Addressed).
Both VMs share the **same program content hash**, so G3 passes for both. Each
`from_snapshot` independently materializes its own `SharedCell` from the
`heap_referents` token and re-acquires one `Arc<SharedCell>` share. There are now
**two live exclusive `&mut` references to logically the same `x`**, in two VMs,
each believing it holds the unique exclusive borrow B0001 proved.

The facet's own escape hatch (§3.4) names ONLY the *cross-program* case as the
hazard and gates exactly that. The **same-program-multi-instance** case — which
is the entire point of serializing a resumable snapshot for distributed execution
— is not gated by a program-hash check, because the hash is identical. B0001's
exclusivity proof is a *whole-program static fact over one MIR set executing in
one VM* (facet §3.1 says this verbatim); the moment two VMs restore the same
`&mut`-carrying snapshot, the "one MIR set, one VM" precondition is violated and
the proof no longer holds. Result: two writers to "the same" mutable cell with
no coherence — silent-wrong, and a data race if the VMs run concurrently
(`SharedCell` is `Arc`-shared but the two cells are now *distinct* allocations,
so the mutations silently diverge rather than even racing — arguably worse,
because the program's single-VM semantics promised they were the same `x`).

**Why the draft misses it:** the synthesis collapses "live continuation" to
"value-state reconstruction = round-1 work, S" (§5.1) and treats the only
exclusivity hole as cross-program (G3). But content-addressed resumable snapshots
are explicitly a multi-instance feature; "same program hash" is the *common*
case, not a rejected edge. The G3 guard as specified is necessary but not
sufficient. A sound gate would have to forbid resuming a snapshot that contains a
live exclusive (`is_mut`) `PromotedCell` more than once — which requires *reading*
`is_mut` at restore and tracking consumed-snapshot identity, contradicting the
"`is_mut` carried-reserved-not-read, Effort S" disposition (facet §3.3,
synthesis §5.2 obligation 1). This is the runtime-loan-obligation the draft
claims does not exist, re-entering through the multi-instance door.

---

## BREAK 2 (live-continuation, BROKEN-INVARIANT) — resume mid-loop re-executes a borrow region the NLL liveness proof already retired, doubling a live loan against the restored cell

The draft (§5.1, facet §2 sub-case 1) asserts: "continued instructions are the
*same opcodes* that passed the gate ... the proof already covered every program
point reachable forward. Sound by construction."

This conflates *static reachability* with *runtime liveness*. B0001 conflict
detection is **NLL liveness-based**: `compute_nll_live_points`
(`solver.rs:1254`) + the intersection test (`solver.rs:1067-1069`,
`has_overlap = pa.iter().any(|p| pb.contains(p))`). A loan is "live" only between
its issue point and its last *use* — `loans_with_reachable_uses`
(`solver.rs:1280-1315`). The proof that two exclusive loans don't conflict
depends on their live *ranges* not overlapping, which in a loop depends on a loan
being **dead** (last-used) before the next iteration re-issues it.

Consider:

```shape
fn drive() {
    let mut acc = 0
    for i in 0..10 {
        let r = &mut acc      // loan L issued each iteration; dead at iteration end
        *r = *r + i           // last use of L this iteration
        snapshot()            // <-- snapshot taken AFTER last use, loan L is NLL-dead here
    }
}
```

At the snapshot point, the NLL proof says loan L is **dead** (its last use was
the `*r = ...` write; the iteration boundary kills it). The compiler proved no
conflict because L's live range never overlaps the *next* iteration's L. But the
runtime carrier `r` is a live `RefTarget` value on the stack at the snapshot IP
(NLL deadness is a *compile-time* property; the slot still physically holds the
ref bits — there is no runtime "kill" that zeroes it; verified: nothing in
`truncate_stack`/the executor drops a slot at NLL-last-use, only at scope/frame
exit, `vm_impl/stack.rs:925-938`).

Now promote this to the broad scope where `r` escapes (e.g. `module_g = &mut
acc` inside the loop, or capture into an escaping closure once §4 lands). The
snapshot serializes the still-physically-live promoted reference. On resume +
continue, execution re-enters the loop body and **re-issues loan L for the next
iteration while a restored, still-physically-live exclusive ref to the same cell
exists**. The compiler's non-overlap proof assumed the prior iteration's L was
dead; resume re-materialized it as a live value. Two exclusive references to one
`acc` cell coexist at runtime. Silent-wrong.

The draft has no obligation covering "snapshot taken at a point where an
NLL-dead-but-physically-live exclusive ref exists." Its mental model ("same
opcodes ran, proof covers all forward points") is a *static-reachability* model;
B0001 is a *liveness-interval* model, and resume can resurrect a value the
liveness model retired. **The "sound by construction" claim in §5.1 is
unproven** — the construction it relies on (NLL deadness ⇒ no live carrier)
does not hold across a snapshot boundary, because the snapshot captures physical
slot bits, not NLL liveness.

(Note: this is distinct from B1. B1 is multi-instance same-program; B2 is
single-instance, single-VM, single resume — it breaks even under the strictest
replay-once interpretation, because the loop iteration itself is the second
aliasing path.)

---

## BREAK 3 (live-continuation, UNBOUNDED-LEAK + double-decrement risk) — the snapshot identity table the design rides DOES NOT EXIST and cannot be built as a per-slot `slot_to_serializable` call; the restore path independently re-acquires shares with no dedup

The synthesis treats `heap_referents` (the single-source identity table) as
round-1-delivered infrastructure it merely "rides" (§2.3, §4.1, §5.3). Verified
at HEAD: it does not exist, AND the serialize/restore API shape **cannot host it
without a redesign the draft does not acknowledge**:

- Serialize is `snapshot()` (`executor/snapshot.rs:139-213`), which walks each
  stack slot (`:154-163`) and each module binding (`:172-181`) by calling the
  **pure per-slot function** `slot_to_serializable(bits, kind, store)`
  (`shape-runtime/snapshot.rs:843`). For `Ptr(HeapKind::Reference)` it returns
  `SV::ReferenceOpaque` — a **unit variant, identity discarded** (`:1104`); for
  `Ptr(HeapKind::SharedCell)` it returns `SV::SharedCellOpaque` — also identity-
  free (`:1106`). The `store` parameter is a `SnapshotStore` (blob sidecar),
  **not** a mutable identity map. There is no shared dedup state threaded through
  these calls.
- Restore is the inverse per-slot `serializable_to_slot` (`:1174`), which for
  both opaque arms **hard-`Err`s** (`:1325-1327`). The stack-restore
  (`from_snapshot` `:252-261`), module-binding restore (`:268-276`), and closure
  capture restore (`restore_call_stack` `:383-396`) each call it *independently*,
  in sequence — there is **no allocate-all-then-link pass** between them. The
  draft's §5.3 reconstruction-order claim ("the round-1 `heap_referents`
  allocate-then-link pass materializes each `SharedCell`, then each `PromotedCell`
  reference acquires one share") describes machinery that is not present and is
  not introduced by the draft.

Why this is a live-continuation soundness hole, not just "round-1 unfinished":
the broad flip + live continuation **enlarges** what must round-trip. A
closure-buried promoted reference (§4) serializes through the SAME per-slot pipe
(`snapshot_closure_frame` → `read_capture_kinded` → `slot_to_serializable`,
`executor/snapshot.rs:579-580`). With no identity dedup, the obvious "fix" an
implementer reaches for under the broad scope is to make each
`ReferenceOpaque`/`SharedCellOpaque` carry the cell's *bits* (raw `*const
SharedCell`) so restore can rebuild it — which is precisely the BREAK-4 raw-
pointer token the draft itself forbids on sight (§2.1, KL-4-array tripwire (b)).
The draft asserts the single-source table makes that unnecessary, but **the
single-source table is exactly the unbuilt piece**, and the per-slot API it must
replace is structurally hostile to it (no shared identity state). Until that
table is designed (not "ridden"), two outcomes are reachable:
- **Leak (unbounded):** if restore allocates a fresh cell per opaque token with
  no dedup, N references that aliased one cell become N cells; the original
  cross-reference cycle the design *thought* it rejected (P2) can now never be
  collected because each ref owns a private cell — and worse, aliasing is silently
  broken (two object fields that observed each other's mutation no longer do —
  the draft's own P-obj-3 positive test, synthesis §9, fails silently).
- **Double-decrement:** if an implementer dedups by raw heap address (the
  forbidden token), two restored references reconstructed from the same address
  each call `Arc::decrement_strong_count::<SharedCell>` on a cell that was only
  `Arc::into_raw`'d once on restore → the cluster-1.5 / W5 SIGABRT double-free
  class the draft claims is "RESOLVED by construction" (§2.1, §6).

The draft's §2.1 double-free proof ("`Arc::into_raw` ⇄ `decrement_strong_count`
on identical T, allocator-symmetric, each holder one clone one decrement") is
sound **only if the restore path produces exactly one `Arc<SharedCell>` per
logical cell and hands each holder exactly one share via the identity table.**
That table is the unverified premise. The proof is conditional on infrastructure
the design does not deliver, so KL-4 is NOT "designed-through" for the snapshot/
live-continuation path — it is designed-through only for the *in-session* (no
snapshot) drop path (`drop_fields` arms `heap_value.rs:3852`/`:3893`, which I
confirmed exist). Live continuation specifically exercises the un-built path.

---

## BREAK 4 (broad-flip, the P2 predicate is vapor) — "storage planner proves acyclicity" names machinery that does not exist and is not designed; the conservative floor cannot be expressed in the single-slot planner

The entire container-flip soundness (§1 resolved rule, §7 cycle policy) rests on
predicate **P2: "the storage planner proves the stored ref's referent is not the
container nor a transitive owner of it (acyclicity)."** Verified: the storage
planner `decide_slot_storage` (`storage_planning.rs:898-1006`) reasons about a
**single slot at a time** via `slot_is_aliased` (`:914`/`:198`), `slot_is_mutated`
(`:913`/`:234`), `detect_escape_status` (`:928`/`:1014`). `detect_escape_status`
distinguishes exactly three states — `Escaped` (flows to return SlotId(0)),
`Captured` (in `closure_captures`), `Local` (`:1019-1030`) — and has **no notion
of a referent-ownership graph**. The only transitive machinery present is
`propagate_transitive_closure_escape` (`:750`, `:833-835`), which propagates
*escape* status across nested **closure captures** — not referent ownership, not
cycles.

The draft hand-waves P2 as "reuses the existing `sink_is_local` exemption
pattern" (§7). But `sink_is_local` (`solver.rs:1176-1179`) is
`slot_escape_status.get(&slot) == Some(EscapeStatus::Local)` — a single-slot
escape-status lookup. It says nothing about whether the *referent* is a
transitive owner of the *store target*. To answer "is `a.next = &a` a cycle" the
planner must relate the referent (`a`) to the store target (`a.next`'s container,
which is `a`) — a points-to / ownership-graph query the MIR has no representation
for at this layer (references-into-fields are `RefTarget::TypedField` carriers
resolved at runtime; the compile-time MIR sees `Assign(Place::Field(...),
Borrow(...))` with no aggregate-ownership model).

Consequence: P2 as the "safe floor" (§1 open point, §7, O8) is **not
implementable as described**. Either (a) the implementer builds a real points-to
ownership analysis (NOT S/M effort, not "folded into the gate" — this is the
XL the draft pretends it isn't), or (b) the implementer ships a syntactic
over-approximation (e.g. "reject only the literal `x.f = &x` self-store") that
misses `a.next = &b; b.next = &a` (mutual ownership) and **silently promotes a
cycle the design promised to reject**. Under (b) the documented "zero new leak
surface" claim (§7) is false — the broad flip ships a leak path the design
asserted it gated out. This is the classic walk-back the CLAUDE.md §Forbidden-
rationalizations warns about ("document it as out-of-scope" / "conservative
gate" that quietly under-approximates): the cycle stays *named* as rejected while
the actual predicate admits it.

This is load-bearing because the draft's KL-2 disposition (§7: "ACCEPT-
DOCUMENTED-LEAK with a conservative acyclicity gate that keeps cyclic shapes
rejecting") is the *only* thing standing between the broad flip and an
unbounded-leak feature. If P2 cannot be soundly cheaply expressed, the honest
fallback is the one the draft buries in O8's parenthetical: keep container/closure
sinks fully rejecting (reduce to round-1 narrow scope). The draft's headline
"broad flip soundly designable, Effort M" is contingent on a predicate it never
shows is buildable.

---

## BREAK 5 (broad-flip closure-env, UAF latent in the §4.3 Delta-2 stamp) — the kind-stamp fix is necessary but the design never proves the capture VALUE written is a `PromotedCell` and not still a frame-`Local` ref at `MakeClosure` emit time

§4.2 correctly identifies that a closure-buried `RefTarget::Local` is a
guaranteed UAF (resolves against live `call_stack` after the frame pops,
`read_ref_target` `:2986-2994`, confirmed) and that only `PromotedCell` is sound.
§4.3 Delta 2 fixes the *kind track* (stamp `Ptr(HeapKind::Reference)` not
`Ptr(HeapKind::NativeView)` — verified `native_kind_from_concrete_type:948` is
the wrong-carrier route, and `from_capture_types_with_native_kinds:1055` exists).

But the kind stamp is orthogonal to the **carrier-rewrite ordering** problem the
design does not address. `op_make_ref` (`variables/mod.rs:2541`) builds a
`RefTarget::Local` for `Operand::Local` — *unconditionally, at runtime*. The
compile-side promotion (§2.2, §4.3 Delta 1) rewrites the referent's *storage
class* to SharedCow and is supposed to rewrite the *ref's* `RefTarget` to
`PromotedCell`. But there is no `PromotedCell` variant yet (verified
`reference.rs:41-99` — three variants, no `PromotedCell`), and critically the
draft never specifies **at which opcode the `Local`→`PromotedCell` rewrite
physically happens at runtime.** The MIR promotion directive is a compile-time
marker; the runtime `op_make_ref` still emits `Local` unless a *different opcode*
is emitted for promoted refs. If the capture write path
(`control_flow/mod.rs:564-575`, `write_capture_raw_u64`) copies the *bits of a
`Local`-carrying ref* into the closure block (because `op_make_ref` ran first and
produced `Local`), then stamping the capture *kind* as `Ptr(HeapKind::Reference)`
makes it **worse, not better**: release now correctly decrements an
`Arc<RefTarget>` (`closure_layout.rs:544-546`), but the inner `RefTarget` is a
`Local{frame_index,...}` coordinate, so deref-after-frame-pop is still the UAF —
now with a *correctly-refcounted pointer to a UAF-prone coordinate*. The kind
stamp guards the Arc accounting; it does NOT guarantee the captured value is the
heap-owning carrier.

The N-closure-deref-after-frame-pop test (§9, "single most important test")
guards the *symptom* but the design's mechanism section never closes the
*ordering*: the promotion must rewrite the runtime carrier (emit a
`MakePromotedRef`-style opcode, or have `op_make_ref` consult the storage plan)
**before** `MakeClosure` captures it. The draft assumes "the promotion directive
rewrites the ref's RefTarget → PromotedCell" (§2.2) is a solved compile-step, but
the runtime opcode that builds `Local` (`op_make_ref`) is not in the delta list
(§4.3 lists only Delta 1 storage-class + Delta 2 kind-stamp). This is an
unclosed UAF seam, not merely a test obligation — and it is exactly the seam the
"sharpest UAF case" warning (§4.2) should have forced the mechanism to specify.

---

## What I could NOT break (genuinely sound findings)

- **No runtime borrow checker** (§5.1): verified. `solver::analyze()`
  (`solver.rs`) output lands on the Compiler, never on `VirtualMachine`
  (`executor/mod.rs:264` carries no loan table); `read_ref_target`/`write_ref_target`
  carry no `is_mut`/liveness probe (`variables/mod.rs:2972-3019`). The "no
  runtime-loan-tracker" refusal (G1 sentinel) is correct.
- **Array exclusion by P1** (§3): verified structurally — `HeapElement`
  (`heap_element.rs`) forbids `Arc<>`-wrapped carriers; `RefTarget`/`SharedCell`
  cannot be `HeapElement`; array stays B0004-rejecting. Sound.
- **`&mut` closure-env not flipped, B0001 runs before sink drain** (§4.4):
  verified — B0001 conflict detection (`solver.rs:1058-1090`) runs before the
  `loan_sinks` drain (`:1162-1225`); `ClosureEnvMut` is `continue`
  (`solver.rs:1192`). Sound as far as it goes (but see B1/B2 for the
  *serialized*-`&mut` hole that bypasses this entirely).
- **In-session (no-snapshot) object/enum drop symmetry** (§2.1): the
  `drop_fields` `HeapKind::Reference`/`SharedCell` arms exist and are allocator-
  symmetric (`heap_value.rs:3852`/`:3893`, `closure_layout.rs:544-546`). The
  double-free proof holds **for the in-session path only** — not the snapshot/
  restore path (BREAK 3).

---

## Bottom line

The draft's *negative* findings (what stays rejected) are well-verified and
mostly correct. Its *positive* claims for the broad flip + live continuation rest
on three pieces of infrastructure it treats as delivered-and-ridden but which are
either unbuilt (`heap_referents` identity table — BREAK 3), unbuildable-as-
described in the current planner (P2 acyclicity — BREAK 4), or under-specified at
the runtime carrier-rewrite seam (`Local`→`PromotedCell` ordering — BREAK 5). And
the live-continuation exclusivity story has two concrete silent-wrong holes the
G3 program-hash guard does not close: same-program multi-instance resume (BREAK 1)
and snapshot-at-NLL-dead-but-physically-live-loan resume (BREAK 2). Under
no-known-incorrectness, the broad flip + live continuation **cannot land as
designed** — the honest fallback the draft itself parenthesizes (O8: reduce to
round-1 narrow scope, cycle policy documentation-only) is the sound floor.
