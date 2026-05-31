# v0.3.3 Reference Serialization — Facet: Scope & Test

**Facet owner question:** What is IN and OUT of the v0.3.3 reference-serialization
feature, and what is the precise (VM **and** JIT) test matrix that gates the
release?

This facet defines the boundary and the gate. The producer-side mechanics
(escape→RC flip, identity-handle wire arm) are specified by the
escape-promotion and wire-format facets; this document pins what those facets
are *allowed to touch* and *must prove*.

All claims below cite `file:line` at workspace `HEAD` (branch `main`,
`67768f17`). Every design assertion was read from source, not re-derived.

---

## 0. One-paragraph thesis (the shape this facet bounds)

For v0.3.3, references that genuinely escape today (B0003 `ReferenceEscape` /
B0004 / B0006 / B0012 — `crates/shape-vm/src/mir/analysis.rs:186-201`) are
flipped from **reject** to **escape→RC-promote**: the referent is forced onto
the RC'd heap and its lifetime extended to cover the reference, reusing the
ADR-006 escape→RC machinery (`storage_planning.rs:956-959` Rule 3b +
`detect_escape_status` `:1014-1031`). The reference itself then serializes as an
**identity-handle** into a snapshot-local handle table, reusing the SharedCell
identity-map precedent (the ground-truth's "serialize-with-shared-identity,
identity-map on restore"). Because `snapshot()` / `from_snapshot()`
(`crates/shape-vm/src/executor/snapshot.rs:139`, `:235`) move the **whole VM as
one unit**, `&mut` exclusivity is automatically preserved across
serialize/restore — there is no second live mutator, so the
cross-node-coherence problem never arises. That is the entire reason the OUT
boundary below is sound.

---

## 1. SCOPE BOUNDARY

### 1.1 IN — snapshot→resume reference serialization

The feature is **exactly** the two named ADR-006 follow-ups, no more:

| ADR-006 follow-up | What it lands | Cite |
|---|---|---|
| **W17-snapshot-references** | Reference target identity across snapshot boundaries (entity-id stable handle table) | `docs/adr/006-value-and-memory-model.md:5975` |
| **W17-snapshot-sharedcell** | SharedCell per-kind cell payload + binding-identity table | `docs/adr/006-value-and-memory-model.md:5977` |

Concretely IN:

1. **Escape→RC promotion for the snapshot-reachable reference set.** The
   B0003/B0004/B0006/B0012 *reject* arms (`solver.rs:1146-1160` escaped-loans;
   `solver.rs:1162-1225` loan-sinks) are augmented so that, for the escape
   classes that snapshot needs to carry, the referent is promoted to RC'd heap
   instead of producing a `BorrowError`. The promotion reuses the existing
   lattice — `BindingStorageClass::SharedCow` / `UniqueHeap`
   (`type_tracking.rs:290-299`) via `storage_planning.rs:931-964` — and the
   existing `detect_escape_status` dataflow (`storage_planning.rs:1014-1031`).
   **No new storage class, no new escape kind.**

2. **Reference wire arm: discriminator → identity-handle.** Today
   `SerializableVMValue::ReferenceOpaque` (`snapshot.rs:507-512`) is a bare
   discriminator that surface-and-stops on restore
   (`snapshot.rs:5939` policy: opaque-stub arms return structured `Err`).
   IN-scope is replacing the opaque stub with an identity-handle payload
   (`Reference { target_handle: u32, kind: NativeKind }` shape) that the wire
   facet specifies, threaded through the existing
   `slot_to_serializable(bits, kind, store)` / `serializable_to_slot(sv,
   expected_kind, store)` API pair (`snapshot.rs:5945-5956`, called from
   `snapshot.rs:157` and `:175`).

3. **SharedCell identity round-trip.** `SharedCellOpaque`
   (`snapshot.rs:522-529`) gains the binding-identity table so two `var x`
   bindings that share a cell observe each other's mutations after restore
   (the exact concern noted at `snapshot.rs:526-528`). This reuses the
   SharedCell identity-map (ground-truth: identity-map on restore).

4. **Whole-VM exclusivity preservation.** The `snapshot()` /
   `from_snapshot()` pair (`snapshot.rs:139`, `:235`) already serializes the
   stack + module-bindings as one atomic unit via the per-slot kind-threaded
   projection (stack at `:154-163`, module bindings at `:172-181`). IN-scope is
   ensuring the identity-handle table is part of *that same unit* so a
   reference and its referent restore consistently — never two halves from
   different snapshots.

5. **JIT-produced reference slots round-trip identically.** References are
   strictly per-function in the JIT (`mir_compiler/rvalues.rs:283-286`: "References
   are strictly per-function — they never cross Cranelift call boundaries"). The
   JIT's β1 `RefTarget::TypedField` scope (`rvalues.rs:270-281`) and the
   `MakeRef`/`DerefLoad`/`DerefStore` round-trip (`shape-jit/src/core.rs:594-619`)
   must produce slots whose serialized form is **bit-for-bit indistinguishable**
   from interpreter-produced reference slots — the snapshot is taken at the VM
   level (`self.stack`/`self.kinds`, `snapshot.rs:154-156`), so a JIT frame that
   has deoptimized or whose values have flowed back to the interpreter stack must
   serialize through the same `slot_to_serializable` path.

### 1.2 OUT — live cross-node mutable sharing / move-on-send coherence

Explicitly **deferred to v0.4 (live-distributed-sharing)** and **not needed for
snapshot**:

1. **Live cross-node mutable aliasing.** Two nodes simultaneously holding live
   `&mut` to the same logical value, with mutations needing to propagate. This
   is the move-on-send coherence problem.

2. **Move-on-send semantics.** Sending a reference over the wire (QUIC,
   `shape-wire`) to a *different live VM* and keeping both sides coherent.

3. **Cross-VM `&mut` exclusivity enforcement.** Detecting and rejecting a second
   live `&mut` that originates in a *different* VM instance.

#### 1.2.1 Why snapshot does NOT need move-on-send coherence (the load-bearing justification)

The OUT boundary is sound because of **the unit of motion**. Three independent
facts make the cross-node coherence problem structurally impossible for
snapshot→resume:

- **Snapshot moves the whole VM, not a value.** `snapshot()`
  (`snapshot.rs:139-215`) serializes `self.stack[0..sp]`, `self.module_bindings`,
  `self.call_stack`, loop/timeframe/exception state — the *entire* VM image.
  `from_snapshot()` (`snapshot.rs:235`) rebuilds one VM from that image. There is
  never a moment where a reference and its referent live in *two* VMs.

- **Resume is sequential, not concurrent.** A snapshot is taken at one instant,
  serialized, then a *new* VM is built from it (`from_snapshot`,
  `snapshot.rs:235`). The source VM and the resumed VM do not run concurrently
  against shared memory — the resumed VM owns a *complete copy*. There is no
  second live mutator, so `&mut` exclusivity (the property B0001 enforces,
  `solver.rs:1073-1079`) is preserved by construction: the only `&mut` in the
  resumed image is the one that was exclusive in the source image.

- **Identity is intra-image, not inter-node.** The identity-handle table (IN
  §1.1.2) maps handles to targets *within a single snapshot image*. Two aliased
  references restore to the same target because they carry the same handle into
  the same table — exactly the SharedCell identity-map precedent
  (`snapshot.rs:526-528`). No cross-node identity negotiation is required because
  there is no second node holding the other end.

**Therefore:** the only coherence guarantee snapshot needs is *intra-image
identity consistency*, which the handle table provides. The inter-node live
coherence guarantee (move-on-send) is a strictly larger problem that
snapshot→resume never poses. Pulling it into v0.3.3 would be scope creep with no
snapshot-driver requirement — it belongs to v0.4 live-distributed-sharing, a
separate feature.

#### 1.2.2 OUT-boundary tripwire (must stay refused)

If implementation work starts reaching for any of these, it has crossed into
v0.4 territory and must surface to the user:

- A second VM instance referenced from `from_snapshot` or the wire layer.
- A "live handle" that resolves across VM instances rather than within one
  snapshot image.
- Any `&mut` exclusivity check that compares loans from *different* VMs.
- Re-introduction of a `ValueWord`-shaped reference carrier "to make wire
  sharing easier" (CLAUDE.md §Forbidden code #1 — refuse on sight).

---

## 2. THE TEST MATRIX

The matrix is split **POSITIVE** (the feature works) and **NEGATIVE** (the OUT
boundary and the soundness invariants still hold). Every row must run in
**both the VM (interpreter) tier and the JIT tier** unless the row notes
otherwise. JIT-tier rows that hit the β1 reference scope's surface-and-stop
(`rvalues.rs:270-281`) assert a *clean deopt to the interpreter*, not a failure.

Tests are **unit tests** (`#[cfg(test)]` modules) per CLAUDE.md §Testing
Conventions — never standalone files. The natural homes:

- Snapshot round-trip: `crates/shape-vm/src/executor/snapshot.rs::tests`
  (mirrors the existing six W17 smoke tests `test_w17_vm_snapshot_*`, ADR-006
  `:5986-5989`).
- Wire-arm projection: `crates/shape-runtime/src/snapshot.rs` tests
  (`slot_to_serializable` / `serializable_to_slot` round-trip).
- Borrow-checker promote/reject: `crates/shape-vm/src/mir/analysis.rs::tests`
  (mirrors the existing B-code assertions `:471-512`) +
  `crates/shape-vm/src/compiler/functions.rs` (existing B0001/B0003 tests at
  `:3353`, `:4096`).
- JIT reference round-trip: `crates/shape-jit/src/core.rs::tests` (mirrors the
  existing `MakeRef`/`DerefStore`/`DerefLoad` test at `:594-619`).

### 2.1 POSITIVE matrix

| # | Property | Test sketch | Tier(s) | Anchors / cites |
|---|---|---|---|---|
| P1 | **Reference survives snapshot→resume** | Build a VM with a slot holding a promoted reference; `snapshot()` → serialize → `from_snapshot()`; assert the restored slot is kind `Ptr(HeapKind::Reference)` and not an opaque-stub `Err`. | VM + JIT | `snapshot.rs:139`/`:235`; replaces `ReferenceOpaque` `snapshot.rs:507-512` |
| P2 | **Deref value correct after restore** | Referent = `42` (`Int64`); take `&x`; snapshot/resume; `DerefLoad` through the restored ref yields `42`. | VM + JIT | `RefTarget::Local{kind}` `reference.rs:54-58`; deref dispatch via carried `NativeKind` (`reference.rs:5-9`) |
| P3 | **Identity preserved across aliased refs** | Two refs `r1`, `r2` to the same referent; snapshot/resume; mutate through `r1`; read through `r2` sees the mutation (same handle → same target). | VM + JIT | identity-handle table; SharedCell identity precedent `snapshot.rs:526-528` |
| P4 | **`&mut` still exclusive after restore** | Take `&mut x`; snapshot/resume; assert the restored image has exactly one exclusive loan to the referent and a second `&mut` in the resumed program still trips B0001 at compile time. | VM (compile-time) | B0001 `solver.rs:1073-1079`, `analysis.rs:232-234` |
| P5 | **Referent mutation visible through ref** | Restore a promoted referent + ref; `DerefStore` a new value; subsequent `DerefLoad` and a direct read of the referent slot both observe it. | VM + JIT | `DerefStore`/`DerefLoad` `core.rs:598-619`; promoted referent = RC'd heap `storage_planning.rs:956-959` |
| P6 | **SharedCell identity round-trips** | Two `var x` bindings sharing a cell; snapshot/resume; mutation through one is observed by the other (binding-identity survives). | VM | `SharedCellOpaque` → identity table `snapshot.rs:522-529`; W17-snapshot-sharedcell `:5977` |
| P7 | **JIT-produced ref slot == interpreter-produced ref slot** | Compile a hot function that builds `&x` via JIT (`MakeRef`, `core.rs:594`); deopt / flow back to interpreter stack; `snapshot()`; assert the serialized arm is identical to the interpreter-built reference's arm. | JIT→VM | per-function ref invariant `rvalues.rs:283-286`; VM-level snapshot `snapshot.rs:154-156` |
| P8 | **Promoted referent refcount balances** | After snapshot/resume of a promoted ref + referent, drop both; assert no leak and no double-free (the `clone_with_kind`/`drop_with_kind` Reference arm balances). | VM | Reference arm retain/release `reference.rs:13-16`; ADR-006 §2.7.7 parallel-kind drop |
| P9 | **Whole-VM atomicity** | Reference in a module binding (`module_bindings[i]`, `snapshot.rs:172-181`) AND its referent on the stack restore from the *same* `VmSnapshot` — never mixed images. | VM | module-binding projection `snapshot.rs:172-181`; whole-VM unit §1.2.1 |

### 2.2 NEGATIVE matrix

| # | Property | Test sketch | Tier(s) | Anchors / cites |
|---|---|---|---|---|
| N1 | **Genuine dangling still rejected (if any remain)** | Any reference-escape class NOT in the snapshot-promote set (e.g. a ref returned past its referent's true lifetime where promotion is not applicable) still emits B0003. Pin the exact residual class the escape facet leaves as reject. | VM (compile-time) | B0003 `analysis.rs:238-240`; escaped-loans `solver.rs:1146-1160` |
| N2 | **Second `&mut` still B0001** | `let r1 = &mut x; let r2 = &mut x;` overlapping → `ConflictExclusiveExclusive` → B0001. Promotion must NOT relax exclusivity. | VM (compile-time) | `solver.rs:1073-1079`; `analysis.rs:232-234`; existing test `functions.rs:3353` |
| N3 | **Live-cross-node-mutable cleanly refused** | Attempting to resolve a reference handle against a *second* VM instance, or any move-on-send path, surfaces a structured error — NOT silent corruption, NOT a fabricated slot. | VM (resume) | OUT boundary §1.2.2; surface-and-stop policy `snapshot.rs:5939` |
| N4 | **Discriminator/kind mismatch surfaces** | A `serializable_to_slot` call where the wire arm's discriminator doesn't pair with `expected_kind` returns structured `Err` (no Bool-default). | VM | §2.7.5.1 policy `snapshot.rs:5938`, ADR-006 `:5968` |
| N5 | **Ref stored in container still B0004 (when not promote-eligible)** | A reference stored into an array/object/enum that escapes beyond snapshot reachability still emits B0004. | VM (compile-time) | B0004 `analysis.rs:242-244`; loan-sinks `solver.rs:1193-1202` |
| N6 | **Exclusive ref across task boundary still B0006** | `&mut` sent across a detached task boundary still emits B0006 — snapshot promotion does not touch the task-boundary sink. | VM (compile-time) | B0006 `analysis.rs:248`; `solver.rs:1206-1207` |
| N7 | **Module-binding ref escape (c6) unchanged** | The v0.3.3 c6 `ModuleBindingStore` sink (`solver.rs:1210-1214`) still emits B0003 `ReferenceEscapeIntoModuleBinding` when it is genuine escape, not snapshot-promote. | VM (compile-time) | c6 sink `solver.rs:1210-1214`, `analysis.rs:240` |
| N8 | **No ValueWord-shaped carrier reintroduced** | Sentinel/grep assertion: the reference wire arm carries `{ target_handle, kind }`, never a `ValueWord`/tag-bits payload. `just check-no-dynamic` must stay green. | build gate | CLAUDE.md §Forbidden code #1; `no_dynamic.rs` sentinel |

### 2.3 Coverage rationale

- **P1–P5** are the core "feature works" claims from the facet brief (survives,
  value correct, identity, `&mut` exclusive, mutation visible).
- **P6** covers the SharedCell half of the feature (W17-snapshot-sharedcell).
- **P7** is the JIT parity row — without it, a JIT-built reference could
  serialize differently and silently corrupt resume.
- **P8** guards the refcount discipline that promotion introduces (the most
  likely place a use-after-free hides — cf. the cluster-1.5 share-accounting
  anchors in CLAUDE.md §Known Constraints).
- **N1, N2, N5, N6, N7** prove promotion did NOT weaken any soundness check.
- **N3** is the OUT-boundary tripwire as an executable test.
- **N4, N8** are the §Forbidden-pattern guards (no Bool-default, no ValueWord).

---

## 3. HOW IT GATES v0.3.3

1. **All POSITIVE rows green in BOTH tiers** (JIT rows that hit the β1
   surface-and-stop assert clean deopt, `rvalues.rs:270-281`). The six existing
   W17 smoke tests (`test_w17_vm_snapshot_*`, ADR-006 `:5986-5989`) stay green —
   the new reference rows extend that suite, they do not replace it.

2. **All NEGATIVE rows green** — every pre-existing borrow-check rejection
   (`analysis.rs::tests:471-512`) plus the new N1/N3 rows. Promotion is additive;
   a regression in any B-code reject is a release blocker.

3. **`just check-clean` and `just check-no-dynamic` green** (CLAUDE.md §Build).
   N8 + the `no_dynamic.rs` sentinel must pass — no ValueWord/tag-bits carrier,
   no Bool-default.

4. **`bash scripts/verify-merge.sh` (11-check Phase 2d gate)** passes for the
   sub-cluster branch, including the 4-table HeapKind lockstep — the
   `Reference` HeapKind wire arm must extend `SerializableVMValue` in lockstep
   (ADR-006 `:5960`).

5. **`just test` (Tier 2: unit + deep) green before commit**; `just test-all`
   (Tier 3) green before the release tag. The 4 ignored
   `bin/shape-cli/tests/stdlib/simulation.rs` snapshot tests (CLAUDE.md §Known
   Constraints) are candidate un-ignores **only if** they exercise reference
   serialization — confirm with the simulation-test owner; do not silently
   un-ignore unrelated tests.

6. **Whole-release disposition:** per `MEMORY.md`
   (`project_v0_3_3_full_correctness_disposition.md`), v0.3.3 carries the FULL
   1220 release-blocking set. This feature's matrix is a *subset* of that gate;
   it does not move scope to v0.4. The OUT boundary (§1.2) is the only thing
   that legitimately defers, and it defers a *separate feature*, not a
   correctness item.

---

## 4. Boundary with sibling facets (what this facet does NOT specify)

- **Escape-promotion mechanics** (which exact escape classes flip, the
  `storage_planning` edit) — escape-promotion facet. This facet only pins that
  promotion reuses `storage_planning.rs:956-959` + `detect_escape_status` and
  adds no new storage class.
- **Wire-arm payload shape** (`Reference { target_handle, kind }` vs. table
  encoding) — wire-format facet. This facet pins that it replaces
  `ReferenceOpaque` (`snapshot.rs:507-512`) and threads through
  `slot_to_serializable` (`snapshot.rs:5945`).
- **Identity-handle table construction** (handle allocation, dedup via
  `Arc::ptr_eq`) — identity facet. This facet pins that it reuses the SharedCell
  identity-map precedent and lives inside the single `VmSnapshot` image (§1.2.1).
