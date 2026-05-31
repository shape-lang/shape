# v0.3.3 Reference Serialization — Round 2 FINAL (Broad Flip + Live Continuation)

> Finalizes `DESIGN-ROUND2-DRAFT.md` against three independently **HOLES-FOUND**
> adversarial reviews (`adversarial/REVIEW-container-double-free.md`,
> `adversarial/REVIEW-cycle-leak-and-drop-accounting.md`,
> `adversarial/REVIEW-live-continuation-loan-soundness.md`). Every load-bearing
> source claim — both the draft's and the reviews' — re-verified against source
> at workspace `HEAD` (`main`, `67768f17`). Round 1's `DESIGN.md` is the SOUND
> FLOOR (narrow ReturnSlot + ModuleBindingStore flip on a heap-owning
> `PromotedCell`/`SharedCell` carrier). This round designs the USER-CHOSEN
> BROADER scope: ClosureEnv flip + container (object/enum/array) B0004 flip +
> LIVE CONTINUATION resume.

---

## TOP-LINE VERDICT (read first)

| Sub-feature | Verdict | KL-4 double-free | Effort to make sound |
|---|---|---|---|
| **Broad container flip (object/enum)** | **DEFER** | **NOT RESOLVED** — C1 sibling-Local desync (UAF), C3 Any-field wild-free, both flip-armed | XL (sibling-ref reconciliation + Any-field tripwire + P2 points-to) |
| **Broad container flip (array)** | **DEFER** (was already correct: stays B0004) | RESOLVED by exclusion (sound) | XL (`RefCellElem` carrier) |
| **Broad closure-env flip** | **DEFER** | **NOT RESOLVED** — BREAK-5 carrier-rewrite-ordering UAF seam, C5 ClosureEnvMut storage-class incoherence | L (op_make_ref carrier rewrite + sink-disjoint storage isolation) |
| **Live continuation (resume + continue)** | **DEFER** | **NOT RESOLVED** — BREAK-1 multi-instance `&mut` exclusivity, BREAK-2 NLL-dead-but-physically-live, BREAK-3 unbuilt identity table | L–XL (consumed-snapshot tracking + is_mut read + two-pass restore) |
| **Reference-cycle leak policy** | **DEFER as stated** — the "leak-not-UAF floor" is FALSE under mutation (BREAK-C1) | mutation detonates cycle → UAF + double-free | XL (whole-program ownership/points-to for a real P2) |

**Is KL-4 (a real double-free) designed-through? NO — not for the broad scope.**
The in-session (no-snapshot) object/enum *drop* path is genuinely sound in
isolation (verified: `drop_fields` `HeapKind::Reference` arm `heap_value.rs:3852-3856`
+ `SharedCell` arm `:3893-3897`, allocator-symmetric with `Arc::into_raw`). But the
broad flip introduces **four new double-free/UAF paths the draft did not design
through**, each verified against source:

1. **C1 — sibling-Local-ref desync (silent-wrong → UAF).** Forcing a referent to
   `SharedCow` while a non-escaping sibling `&x` stays `RefTarget::Local` makes the
   sibling read a `*const SharedCell` as its frozen scalar kind. Verified:
   `read_ref_target`'s `Local` arm does `stack_read_kinded_raw` and returns
   `(bits, *kind)`, **explicitly discarding the live `_stored_kind`**
   (`variables/mod.rs:2997-2998`); `op_alloc_shared_local` rewrites the slot to
   `Arc::into_raw(Arc<SharedCell>)` bits (`variables/mod.rs:1530-1534`). Round-1's
   load-bearing invariant ("the slot kind never changes under `Local` refs",
   `DESIGN.md` §3.3) is **FALSE under the broad scope**.

2. **BREAK-C1 — mutation detonates a promoted cycle into UAF + double-free.** A
   `SharedCell` is **mutable storage**; a deref-store force-drops the prior payload
   (verified `op_store_shared_local:1609-1615`, `*guard = new_bits` then
   `drop_with_kind(prev_bits, cell_kind)`). In a cycle `S_a → cell A → Arc<S_a>`,
   overwriting cell A drops `S_a`'s last share, whose `drop_fields` re-enters cell
   A's release while A is held live → UAF + double-decrement. The "cycles only
   leak" floor assumes an immutability `SharedCow` referents categorically lack.

3. **C3 — Any-field scalar-overwrite wild-free.** `field_kinds`/`heap_mask` are
   immutable post-construction; `write_field_at_idx` drops the prior occupant using
   the schema-fixed `stored_kind = field_kinds[idx]` (`typed_object_ops.rs:889,964`),
   and the kind-invariance guard is **skipped for `FIELD_TAG_ANY`/`FIELD_TAG_UNKNOWN`**
   (`:897-899`). Overwriting a Reference-kinded `Any` field with a scalar leaves
   `field_kinds[idx]=Ptr(Reference)` + heap_mask bit set; the next `drop_fields`
   does `Arc::decrement_strong_count(scalar as *const RefTarget)` = wild free. The
   flip is what first makes a Reference-kinded `Any` slot constructible.

4. **BREAK-1 — live-continuation multi-instance `&mut` exclusivity.** `from_snapshot`
   builds a **fresh VM** (`executor/snapshot.rs:243`) and the G3 program-hash guard
   passes for two instances of the *same* content-hashed program (the distributed-
   execution use case). Two live exclusive `&mut` to logically-the-same cell, each
   in its own VM, each holding a distinct restored cell — B0001's "one MIR set / one
   VM" precondition violated. `is_mut` carried-reserved-not-read cannot gate this.

**Every one of these four paths is a real double-free/UAF the draft's KL-4 table
marks RESOLVED but is not.** Under the brief's hard constraint — *"under
no-known-incorrectness the broad flip CANNOT land until KL-4 (a real double-free)
is designed-through"* — the broad flip **cannot land in v0.3.3.**

**Compounding fact (decisive):** the round-1 carrier the entire broad flip
"rides" — `RefTarget::PromotedCell` and the `heap_referents` identity table —
**does not exist in the source tree.** Verified: `grep PromotedCell crates/` →
zero hits; `grep heap_referents crates/` → zero hits; `RefTarget` has exactly
three variants (`Local`/`ModuleBinding`/`TypedField`, `reference.rs:41-99`). The
broad flip is layered on infrastructure that is itself an un-ratified, un-built
round-1 proposal. The broad flip's incremental soundness obligations (C1, C3,
BREAK-1/2/3/5, BREAK-C1) are on top of round-1's own un-discharged O1.

---

## 1. What is genuinely SOUND (verified, kept)

These survive adversarial review and are the floor we build the recommendation on:

- **In-session object/enum drop symmetry.** `drop_fields` has live, allocator-
  symmetric `HeapKind::Reference` (`heap_value.rs:3852-3856`) and `SharedCell`
  (`:3893-3897`) arms; enum payloads are `TypedObjectStorage`-shaped
  (`collections.rs:1377` emits `NewTypedObject`). A fresh-literal `{ r: &x }`
  construction moves exactly one share into the field. The *runtime drop* of an
  object/enum holding a reference, with NO snapshot and NO mutation and NO sibling
  Local ref, balances refcount to zero. (REVIEW-container §"what holds";
  REVIEW-cycle §"in-session"; REVIEW-live §"what I could NOT break".)
- **Array exclusion (P1).** `HeapElement` structurally forbids `Arc<>`-wrapped
  element carriers (`v2/heap_element.rs:41-45`); `RefTarget`/`SharedCell` cannot be
  `HeapElement`; array reference-stores stay B0004-rejecting. SOUND by exclusion —
  the draft's array disposition was already correct. **Keep array rejecting.**
- **No runtime borrow checker.** `solver::analyze()` output lands on the Compiler,
  never on `VirtualMachine` (`executor/mod.rs:264` carries no loan table); the
  deref path carries no `is_mut`/liveness probe (`variables/mod.rs:2972-3019`).
  Resumed bytecode is statically-checked MIR. The "no runtime-loan-tracker"
  refusal (G1 sentinel) is correct and must be preserved.
- **`&mut` closure-env not flipped; B0001 runs before the sink drain.** B0001
  conflict detection (`solver.rs:1058-1144`) precedes the `loan_sinks` drain
  (`:1162-1225`); `ClosureEnvMut` is `continue` (`:1192`). Genuine `&mut`-conflicts
  are caught by B0001 first. SOUND as far as it goes — but see C5 below for the
  storage-class interaction the broad immutable flip introduces.
- **The narrow round-1 scope is cycle-free by construction.** ReturnSlot +
  ModuleBinding alone cannot assemble a reference cycle (a single promoted local
  reference, no container field to point back). The cycle hazard is exclusively a
  *broad-scope* (container/closure) phenomenon. (REVIEW-cycle confirms; this is
  why the narrow floor is safe and the broad flip is not.)

---

## 2. Disposition of every adversarial break (verified against source)

Eleven distinct breaks across three reviews. Each opened and confirmed at HEAD.
**No break is a forbidden-pattern violation by the reviewers** — they refuse the
design's *soundness*, not propose a dynamic shim. All confirmations cite the exact
lines I re-verified.

| Review | Break | Substance | Verified | Disposition |
|---|---|---|---|---|
| container | C1 | sibling non-escaping `Local` ref reads `SharedCell*` as frozen scalar kind → silent-wrong → UAF | `variables/mod.rs:2997-2998` discards `_stored_kind`; `:1530-1534` rewrites slot | **BLOCKER** — defers object/enum flip |
| container | C2 | `heap_referents` table + TypedObject deep-serialize + two-pass restore all UNBUILT; "Effort M, no new wire arm" false | `snapshot.rs:1104-1106` opaque; `:1120-1127` TypedObject Err; `from_snapshot:252-302` single-pass; `grep heap_referents` empty | **BLOCKER** — live-continuation leg rests on unbuilt machinery |
| container | C3 | Any-field scalar overwrite over a Reference-kinded slot → stale `field_kinds`/`heap_mask` → wild free | `typed_object_ops.rs:889,964` (stored_kind drop), `:897-899` (FIELD_TAG_ANY guard skip) | **BLOCKER** — flip-armed; needs Any-field tripwire |
| container | C4 | P2 acyclicity gate is vapor; mis-described as `sink_is_local` reuse | `storage_planning.rs:1014-1031` (Escaped/Captured/Local only); `solver.rs:1176-1179` (single-slot escape lookup) | **BLOCKER** — P2 is net-new XL points-to, not a one-liner |
| cycle | BREAK-C1 | mutating a `SharedCell` payload that owns its container → cascading re-entrant drop → UAF + double-free | `op_store_shared_local:1609-1615` force-drop; `stack.rs` recursive release chain | **BLOCKER** — the "cycles only leak" floor is FALSE |
| cycle | BREAK-C2 | the recommended O8 "safe floor" predicate admits the natural mutual cycle | `storage_planning` has no points-to; mutual `a.peer=&b; b.peer=&a` both pass a distinct-root floor | **BLOCKER** — "zero new leak surface" claim false |
| cycle | BREAK-C3 | snapshot serializer has no cycle/depth guard; cyclic graph → stack overflow; nested Arc per-share accounting unspecified | `grep visited\|cycle\|depth` in `snapshot.rs` empty; `:1120-1127` TypedObject Err | **BLOCKER** — folds into C2/BREAK-3 (unbuilt serialize path) |
| cycle | BREAK-C4 | live-continuation dedup re-establishes a multi-`&mut`-to-one-cell graph B0001 forbade | `from_snapshot:243` fresh VM; `is_mut` not read | **BLOCKER** — duplicate of BREAK-1 family |
| cycle | BREAK-C5 | `ClosureEnvMut` un-flipped + immutable-`ClosureEnv` Delta-1 forcing same slot to SharedCow → wrong-carrier drop | `storage_planning.rs:905` single-slot; `:931-959` whole-slot storage decision; Delta-1 is whole-slot | **BLOCKER** — defers closure flip |
| live | BREAK-1 | same-program multi-instance resume defeats G3 → two live exclusive `&mut` to one logical cell | `executor/snapshot.rs:243` fresh VM; G3 keys on program hash only | **BLOCKER** — defers unrestricted live continuation |
| live | BREAK-2 | resume mid-loop re-issues an NLL-dead-but-physically-live exclusive loan | `solver.rs:1254`/`:1067-1069` NLL liveness; `vm_impl/stack.rs:925-938` no slot-zero at NLL-last-use | **BLOCKER** — breaks even single-VM single-resume |
| live | BREAK-3 | `heap_referents` identity table unbuilt; per-slot `slot_to_serializable` API structurally cannot host it without redesign | `snapshot.rs:843` pure per-slot; `:1104-1106` opaque; `:1325-1327` restore Err | **BLOCKER** — duplicate of C2 from the live lens |
| live | BREAK-5 | `Local→PromotedCell` runtime carrier-rewrite ordering unspecified; kind-stamp alone makes a `Local` UAF *correctly refcounted* | `variables/mod.rs:2541` builds `Local` unconditionally; `reference.rs:41-99` no PromotedCell variant | **BLOCKER** — defers closure flip; carrier-rewrite is the un-closed seam |

**Net:** 13 breaks, **zero downgraded to FIX**, **zero downgraded to documented
KL-with-tripwire-and-ship.** Every break is a genuine soundness hole on the broad
scope; the honest disposition for each is **DEFER the affected sub-feature**.

---

## 3. Why no FIX is offered (and why offering one would be a defection)

The draft tried to FIX everything into v0.3.3. The temptation here is to write a
fix for each break and ship the broad flip anyway. That is exactly the walk-back
shape CLAUDE.md §Forbidden-rationalizations warns against. Walking through it
honestly:

- **C1 fix would require sibling-Local-ref reconciliation** — when a referent is
  SharedCow-promoted, every other live `RefTarget::Local` pointing at the same slot
  must be rewritten to read through the cell. That is a whole-frame ref-rewrite pass
  keyed on the storage-planner's promotion decision, touching `op_make_ref` ordering
  + a new deref arm + a slot-kind reconciliation. **XL, net-new, ADR-level.** It is
  not a v0.3.3 patch.
- **C4 / BREAK-C2 fix would require a real P2** — a whole-program points-to /
  transitive-heap-ownership analysis. The planner today is single-slot
  intraprocedural (`storage_planning.rs:905`, `detect_escape_status:1014-1031`).
  Building points-to is the **undisclosed XL** both reviews independently name.
  Any syntactic over-approximation (the "distinct-root" floor in O8) provably admits
  the natural mutual cycle (BREAK-C2), and via BREAK-C1 that cycle is detonatable
  into UAF — so the over-approximation is not a sound floor, it is a latent
  double-free. **There is no cheap sound P2.**
- **BREAK-C1 is unfixable without P2 OR a tracing collector.** Mutation of a
  promoted referent force-drops the prior payload (`op_store_shared_local:1615`);
  in a cycle that cascade is a UAF. Containing it needs either (a) proof the cycle
  cannot form (= P2, the XL above) or (b) a cycle collector that breaks strong-Arc
  cycles before the mutating drop frees a held cell (= the v0.4+ whole-VM Arc-cycle
  collector, explicitly out of scope and which must collect ALL Arc cycles, not
  references-only — parallel-implementation attractor, refuse on sight).
- **C2 / BREAK-3 fix would require building the `heap_referents` identity table,
  a recursive TypedObject deep-serialize with cycle detection, and a two-pass
  allocate-then-link `from_snapshot`** — none of which exist. The per-slot
  `slot_to_serializable(bits, kind, store)` API (`snapshot.rs:843`, `store` is a
  blob `SnapshotStore` not an identity map) is structurally hostile to threading
  shared dedup state. This is the round-1 W17-snapshot-references/-sharedcell
  follow-up, **still open**, plus the new TypedObject-deep-serialize work. **L–XL.**
- **BREAK-1 fix would require reading `is_mut` at restore + consumed-snapshot
  identity tracking** — the exact "runtime loan obligation" the design claims is
  absent, re-entering through the multi-instance door. It contradicts the
  "is_mut reserved-not-read, Effort S" disposition. **L.**
- **BREAK-5 fix (closure carrier-rewrite ordering)** is the smallest — emit a
  promotion-aware opcode so `op_make_ref` produces the heap-owning carrier before
  `MakeClosure` captures it. But it is moot until `PromotedCell` exists (round-1 O1)
  AND the C1 sibling-reconciliation lands (a closure-escaping `&x` with a sibling
  non-escaping `&x` hits C1 too).

Each "fix" is a multi-week ADR-level workstream. Bundling all of them into v0.3.3
is not a design, it is a re-scope. **The sound move is to DEFER the broad flip and
ship the narrow floor.**

---

## 4. RECOMMENDED SEQUENCING (what lands first, what is soundness-gated)

The release-blocking set for v0.3.3 is FULL (per the v0.3.3 full-correctness
disposition, 2026-05-27). Reference-serialization's contribution to that set is the
**narrow round-1 floor**, not the broad flip. Sequencing:

### Stage 0 — Round-1 narrow floor (the only reference-serialization work for v0.3.3)
**Gated on round-1 O1 (PromotedCell carrier ratification) + O2 (c6 binop reject).**
1. Build `RefTarget::PromotedCell { cell: Arc<SharedCell> }` (round-1 §3) — the
   carrier that does not exist yet. ADR-006 §2.7.30.
2. Flip **ReturnSlot** (`escaped_loans` drain) + **ModuleBindingStore**
   (`solver.rs:1212-1214`) only. Loan generation untouched; B0001 byte-for-byte
   unchanged (round-1 §5).
3. Build the single-source `heap_referents` `SharedCell` identity side-table +
   the allocate-then-link restore (round-1 §3.5). This is the W17-snapshot-references
   / -sharedcell follow-up — **net-new, not "ridden".**
4. c6 binop reference-typed reject co-lands (round-1 O2). Hard co-dependency.
5. Cycle policy: **documentation-only** (round-1 narrow scope cannot form a cycle;
   nothing to gate). XS.
6. Live continuation: **replay-only** for v0.3.3 (round-1 O3) — bit-identical replay
   of the same MIR, single VM, single resume. NOT the broad live-continuation that
   BREAK-1/2 break. `is_mut` carried reserved-not-read (round-1 §4.2).

Stage 0 is itself **NOT YET RATIFIED** (round-1 O1 is open). It is the floor; it is
not free.

### Stage 1 (v0.3.4+) — soundness-gated broad flip, one sub-feature per gate
Each item below is blocked on the named net-new analysis. None ships until its gate
is built and adversarially re-reviewed:
- **Object/enum container flip** — GATED on (a) C1 sibling-Local-ref reconciliation
  pass, (b) C3 Any-field-Reference tripwire (reject/re-tag scalar store over a
  Reference-kinded Any slot), (c) a real P2 points-to acyclicity analysis, (d) the
  TypedObject deep-serialize + cycle-guarded two-pass restore (C2). Effort XL.
- **Closure-env flip** — GATED on (a) the `Local→PromotedCell` runtime carrier-rewrite
  opcode + ordering (BREAK-5), (b) sink-disjoint storage-class isolation so Delta-1's
  whole-slot SharedCow change does not corrupt a co-resident `ClosureEnvMut` capture
  (C5), (c) the object/enum gates above (a closure-escaping `&x` shares C1). Effort L.
- **Unrestricted live continuation** — GATED on (a) consumed-snapshot identity
  tracking + reading `is_mut` at restore to forbid multi-instance live exclusive
  resume (BREAK-1), (b) a snapshot-point liveness model that refuses snapshotting at
  an NLL-dead-but-physically-live exclusive-loan point, or zeroes the slot at
  NLL-last-use (BREAK-2), (c) the identity table from C2. Effort L–XL.
- **Array container flip** — GATED on the `RefCellElem` `HeapElement` carrier (round-1
  §3 / draft §3). Effort XL. (Disposition unchanged from the draft and correct.)
- **Reference-cycle collection** — v0.4+ whole-VM Arc-cycle collector that collects
  ALL Arc cycles (not references-only). Until then, a *real* P2 is the only thing
  that keeps the broad flip from shipping a UAF-reachable cycle.

### What must NOT happen (defection tripwires, refuse on sight)
- A "conservative acyclicity gate" that is actually a syntactic distinct-root check
  (admits the BREAK-C2 mutual cycle while staying *named* "rejects cycles").
- Dedup-by-raw-heap-address in the restore path (the forbidden BREAK-4 token → the
  cluster-1.5/W5 double-free).
- A reference-only cycle collector (parallel-implementation attractor).
- Carrying `is_mut` "reserved-not-read" and then quietly enabling multi-instance
  live continuation (BREAK-1 needs it READ).
- Any array-element reference carrier that stamps a new `ELEM_TYPE_*` onto the
  existing `drop_array_heap` without a real `HeapElement`-with-HeapHeader carrier.

---

## 5. ADR-006 amendment

**No new amendment for the broad flip in v0.3.3.** The round-1 §2.7.30 amendment
(PromotedCell carrier + single-source `heap_referents` identity-handle) stands as
the floor and must be ratified (round-1 O1) before any code. Round 2's
contribution to the ADR is a **negative clause** recording the deferred-scope
soundness boundary:

> **§2.7.30 addendum (round-2 scope boundary, DEFERRED).** Broad reference-escape
> promotion — object/enum/array container stores, closure-env captures, and
> unrestricted (multi-instance / mid-continuation) live resume — stays REJECTED
> (B0003/B0004) in v0.3.3. The broad flip introduces double-free/UAF paths not
> resolvable without net-new analysis: (i) sibling non-escaping `Local`-ref desync
> when a referent is SharedCow-promoted (`variables/mod.rs:2997-2998` discards the
> live slot kind); (ii) mutation of a promoted `SharedCell` referent that owns its
> container detonates an Arc cycle into a re-entrant drop cascade (UAF +
> double-free; `op_store_shared_local:1615`); (iii) Any-field scalar overwrite over
> a Reference-kinded slot wild-frees (`typed_object_ops.rs:964` drops by immutable
> `field_kinds[idx]`); (iv) same-program multi-instance live resume produces two
> live exclusive `&mut` to one logical cell that B0001 forbade
> (`from_snapshot:243`). The acyclicity predicate P2 these depend on is a
> whole-program points-to analysis the storage planner does not have
> (`storage_planning.rs:905` is single-slot; `detect_escape_status:1014-1031` knows
> only Escaped/Captured/Local). DEFERRED to v0.3.4+, each sub-feature behind its own
> built-and-re-reviewed gate (this DESIGN-ROUND2 §4 Stage 1). The reference-cycle
> "leak-not-UAF" floor is FALSE under mutation and must NOT be relied on.

This discharges the §Parallel-implementation attractor by NOT introducing a second
carrier or a second identity table for the broad scope — there is exactly one
carrier (`PromotedCell`, round-1) and one table (`heap_referents`, round-1), both
gated to the narrow floor.

---

## 6. Test matrix delta (round-2 scope)

The round-2 deliverable for v0.3.3 is **negative**: prove the broad scope stays
rejecting and the narrow floor is the boundary. Additive to round-1 P1–P9/N1–N11.

**Negative (clean compile error — NEVER segfault, NEVER leak, NEVER silent-wrong):**
- **N-obj-escape:** `fn f() { let x=5; return { r: &x } }` → clean B0004
  `ReferenceStoredInObject` (verified live today, `solver.rs:1199`). Stays rejecting.
- **N-enum-escape / N-arr-escape:** struct-payload / array-element ref escape → B0004.
- **N-closure-escape:** `&x` captured into an escaping closure → B0003
  `ReferenceEscapeIntoClosure` (`solver.rs:1184`). Stays rejecting.
- **N-cycle-self / N-cycle-mutual:** `a.next=&a` and `a.peer=&b; b.peer=&a` → B0004,
  never promoted. (Guards against a future O8 distinct-root floor that would admit
  the mutual case — BREAK-C2.)
- **N-multi-instance-resume:** a snapshot containing a live exclusive `PromotedCell`
  resumed twice (same program hash) → structured `Err`, never two live `&mut`
  (BREAK-1). For v0.3.3 this is moot because live continuation is replay-only; the
  test documents the boundary for Stage 1.
- **N3 (c6 binop, hard co-dependency from round 1):** reference-typed call/closure
  result fed to a typed binop → clean c6-widened reject, never the live segfault.
- **G1 sentinel:** grep over `executor/` for a new loan/borrow-state table stays
  empty (refuses the runtime-loan-tracker defection).
- **G-sentinel-promotedcell-scope:** grep that `PromotedCell` is constructed ONLY on
  the ReturnSlot/ModuleBinding promotion paths, never on a container/closure sink.

**Positive (narrow floor only — round-1 P-set):** unchanged; the broad P-obj-*/
P-closure-*/P-cycle-* positives from the draft §9 are **withdrawn** (they require the
deferred broad flip).

**Gate:** all NEGATIVE green (additive — any pre-existing B-code regression is a
release blocker); `just check-clean` + `just check-no-dynamic` +
`scripts/verify-merge.sh` green; the six `test_w17_vm_snapshot_*` smoke tests green.

---

## 7. Bottom line for dispatch

**Broad-container-flip: DEFER. Broad-closure-env-flip: DEFER. Live-continuation
(unrestricted): DEFER. KL-4 double-free: NOT RESOLVED for the broad scope.**

The brief's hard constraint is dispositive: *"under no-known-incorrectness the
broad flip CANNOT land until KL-4 (a real double-free) is designed-through."* The
broad flip introduces **four** verified double-free/UAF paths (C1 sibling-Local
desync, BREAK-C1 mutation-detonated cycle, C3 Any-field wild-free, BREAK-1
multi-instance `&mut`), none designed through, each requiring net-new XL analysis
(sibling-ref reconciliation, whole-program points-to P2, Any-field tripwire,
consumed-snapshot tracking) plus three pieces of infrastructure that **do not exist
in the tree** (`PromotedCell`, `heap_referents`, two-pass cycle-guarded restore).
The draft's "Effort M, KL-4 RESOLVED" is an under-estimate by an order of magnitude.

**v0.3.3 ships the narrow round-1 floor only** (ReturnSlot + ModuleBindingStore,
replay-only continuation, cycle policy documentation-only), itself gated on the
still-open round-1 O1 (PromotedCell ratification) + O2 (c6 binop reject). The broad
flip is correctly sequenced to v0.3.4+ behind per-sub-feature soundness gates (§4
Stage 1). This is not a scope-reclaim of the v0.3.3 release-blocking set — the
broad flip was never *in* it; the user chose the broader *design* scope, and the
honest design answer is that the broader scope is not soundly landable in v0.3.3.
**Surface C1, BREAK-C1, BREAK-1, and the unbuilt-infrastructure fact to the user
before any dispatch.**
