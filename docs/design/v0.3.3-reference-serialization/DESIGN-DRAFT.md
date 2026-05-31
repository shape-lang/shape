# v0.3.3 Reference Serialization — Design Draft

> Synthesis of the five facet notes (`_facet-borrow-flip.md`,
> `_facet-escape-rc-integration.md`, `_facet-snapshot-serialization.md`,
> `_facet-soundness.md`, `_facet-scope-and-test.md`). Every claim cites
> `file:line` at workspace `HEAD` (branch `main`, `67768f17`), verified against
> source during synthesis — not re-derived from the facet prose.
>
> **Status: DRAFT for supervisor/user review.** The facets agree on the
> direction but **contradict each other on the single most load-bearing
> mechanism** (what promotion does to a `RefTarget::Local` reference). That fork
> is unresolved here and is the headline open question (O1). Two facets describe
> mechanisms that **do not exist in code today** as if they were reuse targets;
> this draft corrects that framing (§3.0, §4.0).

---

## 1. Thesis

Today the borrow solver **rejects** a reference that would outlive its owner via
`BorrowErrorKind::ReferenceEscape` (B0003) — drained from `facts.escaped_loans`
at `crates/shape-vm/src/mir/solver.rs:1147-1160`, with the module-binding variant
`ReferenceEscapeIntoModuleBinding` from the `loan_sinks` drain at
`solver.rs:1212-1214`. Container-store, closure-env, and task-boundary escapes
reject via B0004 / B0003-closure / B0006 / B0012 (the other `loan_sinks` arms,
`solver.rs:1184-1207`).

The v0.3.3 flip: for the **narrow** set of escape sinks that the snapshot can
carry — **ReturnSlot and ModuleBindingStore only** — replace the B0003 reject
with an **escape→RC promotion** of the *referent* (force its
`BindingStorageClass` onto an RC'd heap class so it is never dropped while the
reference is live, extending the referent's lifetime to cover the reference).
The reference then serializes as an **identity-handle** into the referent; on
restore an identity-map re-points every reference at the one restored referent.

`snapshot()` / `from_snapshot()` (`crates/shape-vm/src/executor/snapshot.rs:139`,
`:235`) move the **whole VM as one unit**, so `&mut` exclusivity is preserved
across serialize/restore *by construction* — there is one writer timeline, one
VM. The cross-node live-coherence problem (move-on-send) is **out of scope**
(v0.4 live-distributed-sharing).

**The narrow-flip thesis is directionally sound but roughly 3–4× the work the
brief implies**, because (a) the dominant reference shape is not a heap pointer,
(b) the "SharedCell identity-map to reuse" does not exist yet, and (c) the flip
must be surgically scoped or it re-legalizes a known SEGFAULT as silent
wrong-memory aliasing. Each of these is itemized below.

### 1.1 The one fact that reshapes everything: what a reference IS at runtime

`RefTarget` (`crates/shape-value/src/reference.rs:41-99`) has exactly **three**
live variants post-V3-S5-ckpt-4 (`TypedIndex` was deleted, `reference.rs:90-98`):

```rust
pub enum RefTarget {
    Local        { frame_index: u32, slot_index: u32, kind: NativeKind },          // :54 — coordinate, OWNS NOTHING
    ModuleBinding{ binding_idx: u32,                  kind: NativeKind },           // :66 — index,      OWNS NOTHING
    TypedField   { receiver: TypedObjectPtr, field_offset: u32, kind: NativeKind }, // :84 — owns ONE RC share of the object
}
```

A reference slot's bits are `Arc::into_raw(Arc<RefTarget>) as u64`, kind
`NativeKind::Ptr(HeapKind::Reference)` (`reference.rs:11-16`). The `Arc<RefTarget>`
keeps the **descriptor** alive — it does **not** own the referent for the first
two variants. Deref resolution for `Local` re-resolves a *coordinate* against the
live stack on every access (`read_ref_target`,
`executor/variables/mod.rs:2982-2998`): `base_pointer + slot_index`, reading
`stack_read_kinded_raw(slot)`. Only `TypedField` carries an owning share
(`receiver: TypedObjectPtr`, retained via `v2_retain`).

**Consequence:** the thesis silently assumes "a reference points at a heap value
and RC keeps that value alive." That is **type-correct only for `TypedField`**.
For `Local`/`ModuleBinding`, "promote the referent and serialize the reference as
a handle into it" requires *first converting the coordinate into a heap-owning
handle* — which is the central design fork (§5, O1). This synthesis adopts the
soundness facet's read of the carrier as ground truth; the borrow-flip and
scope-test facets implicitly assumed the heap-pointer model and must be read
through this correction.

---

## 2. Borrow-solver change (flip reject → promote)

### 2.1 Where B0003 fires today (verified)

`escaped_loans: Vec<(u32, Span)>` (`solver.rs:87`) is populated at exactly three
sites, all in `extract_facts`, and drained into B0003 at `solver.rs:1147-1160`:

| Site | `solver.rs` line | Sink |
|------|------------------|------|
| `Assign(SlotId(0), Borrow)` — local borrow directly into return slot | `215` | ReturnSlot |
| later `Assign(SlotId(0), rvalue)` aliasing a local loan | `290` | ReturnSlot |
| `ModuleBindingStore` receiving a local loan | `461` | ModuleBindingStore |

Parameter borrows that flow to the return slot are *already* classified as safe
(`return_reference_candidates`, `solver.rs:211-213`, `285-288`) — only
**local-rooted** loans hit `escaped_loans`. The flip operates on precisely this
set, which contains **only ReturnSlot and ModuleBindingStore** sinks.

The `loan_sinks` drain (`solver.rs:1162-1225`) handles the rest:
`ClosureEnv` → B0003-closure (`:1184`); `ArrayStore`/`ObjectStore`/`EnumStore` →
B0004 (`:1195-1202`); `StructuredTaskBoundary`/`DetachedTaskBoundary` → B0006/B0012;
`ModuleBindingStore` → `ReferenceEscapeIntoModuleBinding` (`:1212-1214`).

### 2.2 Which sinks flip, which stay rejecting (resolved consensus)

The facets superficially disagree on scope: the scope-test facet's §0/§1.1 prose
says "B0003/B0004/B0006/B0012 are flipped from reject to promote," while its own
test matrix (N5 ref-in-container still B0004, N6 task-boundary still B0006)
*keeps them rejecting*. The borrow-flip and escape-rc facets, and the soundness
facet's "minimal sound flip," all converge on the **narrow** set. **Resolution:
the narrow set is binding; the scope-test §0 prose is an overclaim corrected
here.**

| Sink | Today | v0.3.3 | Rationale |
|------|-------|--------|-----------|
| **ReturnSlot** (local-rooted) | B0003 | **PROMOTE** | Headline case (`let r = &local; return r`). Referent forced RC, lifetime extended. |
| **ModuleBindingStore** | B0003 | **PROMOTE** | Module bindings outlive every frame; promoting makes `module_g = &local` sound (the c6 case). |
| `ClosureEnv` | B0003 | **DEFER — stays reject** | Conceptually in-family, but the escaping ref lives inside a captured cell (`closure_heap_bits`, `executor/mod.rs:188`) — structurally the *buried-handle* case, not a top-level binding. v0.3.4/v0.4 follow-up (borrow-flip §2.1). |
| `ArrayStore`/`ObjectStore`/`EnumStore` | B0004 | **STAYS REJECT** | Ref-in-container is the `TypedField`-class **Arc cycle leak** (soundness §2.2): a field holding `Arc<RefTarget::TypedField{receiver = own storage}>` is a strong-count cycle; `Arc` is non-tracing (`shape-gc` no-op). Also requires per-element identity serialization not in scope. |
| `StructuredTaskBoundary`/`DetachedTaskBoundary` | B0006/B0012 | **STAYS REJECT** | This *is* the cross-task live-coherence (move-on-send) problem deferred to v0.4. Two live tasks sharing `&mut` is not a single frozen VM. |

**Decision rule:** a sink flips iff (1) the escaping reference is a *named
top-level binding* (return value or module binding) the snapshot can address
directly, and (2) the referent's extended lifetime nests within the single VM
the snapshot moves atomically. True for ReturnSlot + ModuleBindingStore; false
for everything else.

### 2.3 The surgical implementation shape (B0001 must survive — soundness §3)

The flip MUST be a change to the **escape-sink → diagnostic** mapping *only*,
never to loan/conflict generation. This is the same surgical shape the c6 fix
used (commit `60baf4fd` added the `ModuleBindingStore` sink + a `BorrowErrorKind`
arm without touching conflict detection).

- Conflict detection (`solver.rs:1058-1144`, B0001 `ConflictExclusiveExclusive`
  at `:1076`) is derived from `MakeRef`/`Borrow` rvalues *independent of storage
  class* and runs *before* any promotion decision. Leave it **byte-for-byte
  untouched.**
- At the `escaped_loans` drain (`solver.rs:1147-1160`): instead of pushing a
  `BorrowError`, push a promotion directive keyed by the referent root slot
  (`info.borrowed_place.root_local()`, `mir/types.rs:86`; the same normalization
  `reference_origin_for_place` already computes at `solver.rs:821-832`). Since
  `escaped_loans` contains only ReturnSlot + ModuleBindingStore, the entire drain
  flips.
- At the `loan_sinks` drain: the `ModuleBindingStore` arm (`:1212-1214`) changes
  to `continue` (the `escaped_loans` path now owns its promotion); every other
  arm unchanged.

**Walk-back hazard (soundness §3.2):** an implementer who promotes the *binding*
to a COW cell and then *suppresses the loan entirely* ("it's RC'd, the borrow is
safe") kills B0001 — two `&mut` to a `SharedCow` cell would be modeled as two
shares, not two exclusive loans. This is the exclusivity analog of "keep
ValueWord for one edge case." **Promotion must be invisible to loan generation:
the loan is still issued, the conflict is still detected, only the terminal
escape-sink diagnostic is replaced by a promotion marker.** Enforced by the N2
sentinel test (§7).

### 2.4 New solver fact

```rust
// BorrowFacts (solver.rs:65-111) — new field
/// Referent root slots that must be RC-promoted because a reference to them
/// escapes via ReturnSlot or ModuleBindingStore. v0.3.3 reference-serialization.
pub reference_escape_promotions: Vec<ReferenceEscapePromotion>;

pub struct ReferenceEscapePromotion {
    pub referent_slot: SlotId,
    pub sink: LoanSinkKind,   // ReturnSlot | ModuleBindingStore only
    pub span: Span,
}
```

The two flipping sinks reach this from the same loan info already present at the
push sites. Carry the sink kind by widening `escaped_loans` to
`Vec<(u32, Span, LoanSinkKind)>` (option a from borrow-flip §4.1; local + clean).

---

## 3. Escape→RC integration

### 3.0 Correction: this reuses the *machinery*, not a finished feature

The escape-rc facet frames the SharedCell promotion path as a reuse target. The
**runtime promotion machinery genuinely exists** and is verified:
`op_alloc_shared_local` (`executor/variables/mod.rs:1459-1535`) promotes a local
slot to `Arc<SharedCell>` (`SharedCell::new(value_bits, value_kind)` at `:1511`,
`Arc::into_raw` at `:1512`, slot rewritten to `Ptr(HeapKind::SharedCell)` at
`:1530-1534`); the closure-capture compiler path (`compiler/expressions/closures.rs`)
sets `BindingStorageClass::SharedCow` and adds the binding to `shared_locals` so
every outer read/write goes through `Load/StoreSharedLocal`. The `SharedCell`
carries its own §2.7.8/Q10 parallel-`kind` companion
(`crates/shape-value/src/v2/closure_layout.rs`). **This is the reuse target and
it is real.**

What does **not** exist is the *snapshot* side (§4.0).

### 3.1 Storage-planning rule (the consensus part)

`decide_slot_storage` (`storage_planning.rs:905-1006`) is the single decision
point. The escape-aware rules already present (`:931-964`): Rule 2 mutable-capture
→ `UniqueHeap` (`:945-947`); Rule 3b `escaped && aliased && mutated` → `SharedCow`
(`:956-959`); else `Direct`. `detect_escape_status` (`:1014-1031`) already returns
`Escaped` for slots flowing to the return slot.

Add a rule, threading `reference_escape_promotions` (the referent slot set) into
`StoragePlannerInput`, that forces the **referent** onto an RC class — reusing
`UniqueHeap`/`SharedCow` (`type_tracking.rs:293-294`), **no new class**.

**Facet contradiction on the class — resolved.** The borrow-flip facet (§4.2)
picks `UniqueHeap` when the referent is mutated, `SharedCow` otherwise. The
escape-rc facet (§3.1) argues `SharedCow` *always*, because an escaped reference
makes the referent intrinsically *aliased* (the reference and the binding are two
observers of the same cell), and `UniqueHeap` is reserved for the
*uniquely-owned* box case. **Resolution: `SharedCow` for the reference-escape
referent.** The escape-rc reasoning is correct — a reference escape is by
definition shared identity (two live observers must see the same storage), which
is the textbook "aliased" condition (`storage_planning.rs:900-901`). `UniqueHeap`'s
single-owner-through-a-box semantics do not model the reference + binding pair.
This also matches Rule 3b's existing `SharedCow` routing. The referent is promoted
`Direct → SharedCow`; the reference *binding* itself stays
`BindingStorageClass::Reference`.

### 3.2 Single-owner-of-record discipline (soundness §5.4 — double-drop trap)

The promoted referent must have **exactly one owner-of-record** (the single
`Arc<SharedCell>` cell). The reference must remain a non-owning handle/coordinate
into that cell. If the implementer instead gives the reference its **own owning
`Arc`** of the cell AND the original binding also drops it AND the coordinate
resolution participates in drop accounting → **double-drop**, the exact
cluster-1.5 / W5 share-accounting bug class
(`executor/vm_state_snapshot.rs:295`, CLAUDE.md Known Constraints) that produced
the W17 SIGABRTs. The current `Local` path is double-drop-free *precisely because
the ref owns nothing* (§1.1). Promotion must preserve that property: the cell is
the sole owner-of-record; deref reads `clone_with_kind` the projected value as
today, not the cell.

### 3.3 Drop/RAII semantics change (soundness §5 — needs a ruling)

Lifetime extension *is* a change to when `Drop::drop` runs. For the module-binding
case (`module_g = &local`), the referent's `Drop` defers from function-scope-end
to module-binding/program-lifetime-end. This is **safe** (single RC cell, single
Drop on last release — the §2.7.7 parallel-kind drop discipline handles it) but
**observable**: an `impl Drop` type (file handle, lock) escaping by reference is
held longer than its lexical scope suggests. Rust accepts this for owner-moves;
Shape's model is novel because the *referent* is promoted, not moved. **This needs
a documented RAII ruling** (O4) and a one-line ADR/CLAUDE.md note.

---

## 4. Snapshot serialization (identity-map)

### 4.0 Correction: the SharedCell identity-map does NOT exist today

Both the brief and three facets describe "reuse the SharedCell identity-map." The
soundness facet (§6) and the snapshot facet (§2) both correct this, and source
confirms: a grep for `identity_map`/`cell_id`/`by_token`/`heap_referents` across
`snapshot.rs` + `executor/snapshot.rs` returns **nothing**. Today:

- `HeapKind::Reference` → `SerializableVMValue::ReferenceOpaque`
  (`shape-runtime/src/snapshot.rs:1104`), a content-free stub
  (`:507-512`, explicitly the "W17-snapshot-references follow-up").
- `HeapKind::SharedCell` → `SharedCellOpaque` (`:1106`, `:522-529`), which states
  the identity requirement as an **open problem**, not a solved one.
- The inverse arms `(SV::ReferenceOpaque, HeapKind::Reference)` and
  `(SV::SharedCellOpaque, HeapKind::SharedCell)` **fail-stop** on restore
  (`snapshot.rs:1325-1335`).
- Whole-VM deep restore lands **empty** today: `resume.rs:503-515` returns empty
  `call_stack` / `module_bindings`.

**So the identity-handle serialization is NET-NEW work** that *establishes* the
pattern both `Reference` and the future `SharedCell` follow-up will share — not a
reuse. This is the largest underestimate in the brief.

### 4.1 The two-mechanism split (snapshot facet §0 — the cleanest framing)

The classic "serialize a pointer as a handle, dedupe on restore" problem applies
to **exactly one** `RefTarget` arm. The other two are *already* symbolic:

| RefTarget arm | Identity carrier | Restore mechanism | Needs ptr identity-map? |
|---|---|---|---|
| `Local` | `(frame_index, slot_index)` | re-index into restored `call_stack` | **No** — symbolic |
| `ModuleBinding` | `binding_idx` | re-index into restored `module_bindings` | **No** — symbolic |
| `TypedField` | `receiver: *const TypedObjectStorage` | dedupe via ptr identity-map | **Yes** |

`Local`/`ModuleBinding` serialize as their integer fields verbatim and re-index
into the W17-reconstructed frame stack / module bindings. The identity-map is
needed only for `TypedField`.

### 4.2 Wire format

Replace `ReferenceOpaque` (`snapshot.rs:507-512`) with a structured arm:

```rust
Reference { is_mut: bool, target: SerializableRefTarget }

enum SerializableRefTarget {
    Local        { frame_index: u32, slot_index: u32, projected_kind: <wire-NativeKind> },
    ModuleBinding{ binding_idx: u32,                  projected_kind: <wire-NativeKind> },
    TypedField   { referent_token: u64, field_offset: u32, projected_kind: <wire-NativeKind> },
}
```

`VmSnapshot` (`snapshot.rs:238-261`) gains a `#[serde(default)]` side-table
`heap_referents: Vec<SerializableHeapReferent>` (kind-tagged, today always
`TypedObject`; reserves a `SharedCell` kind for the follow-up). The token is the
index into this table. The reference arm carries **no inline referent payload** —
inlining would duplicate shared objects and break the aliasing contract; this is
why `ReferenceOpaque` was payload-free and we keep that property.

`projected_kind` must serialize via the wire-stable `NativeKind` ordinal the
§2.7.7 parallel-kind track already uses (the W17 deep-frame-stack facet must
serialize kinds for the stack regardless — O2/O5). It must **not** serialize as
raw discriminant bits (`NativeKind::Ptr(HeapKind)` carries a nested ordinal).

Bump `SNAPSHOT_VERSION` (`snapshot.rs:37`); `#[serde(default)]` keeps old
snapshots readable. The new `Reference` HeapKind wire arm must extend
`SerializableVMValue` in the 4-table HeapKind lockstep that
`scripts/verify-merge.sh` enforces (ADR-006 §2.7.5.1).

### 4.3 Serialize path — dedupe-on-write

One `IdentityWriter { seen: HashMap<u64,u64>, referents: Vec<...> }` is constructed
per `VmSnapshot` and threaded through every slot walk (stack / locals /
module_bindings / frame upvalues in `capture_vm_state`,
`vm_state_snapshot.rs:64`). For a `TypedField` ref, `intern_typed_object(receiver.0)`
returns the existing token if seen, else **reserves the token before recursing**
(so cycles terminate) and projects the object body via the *existing kind-threaded*
`TypedObject` arm. Same pointer → same token regardless of which slot referenced
it: this is where **multiple-refs-to-one-referent** dedupe and where a `TypedField`
ref and the binding holding that object share one side-table entry.

**Cycles / self-reference** (object A → ref → B → ref → A): handled by
reserve-token-before-recurse; the back-edge finds the token already present and
returns without re-recursing (standard serde-object-table cycle break).

The serialize arm recovers the share without consuming it:
`Arc::<RefTarget>::from_raw(bits)` → project → `Arc::into_raw` to restore the
original share (`reference.rs:11-12` provenance).

### 4.4 Restore path — three phases (W17 integration)

Ordering is mandatory:

1. **Phase A — materialize `heap_referents`** into a `RestoreIdentityMap
   { by_token: Vec<u64> }` (`token → restored *const TypedObjectStorage`), using
   the existing `TypedObject` reconstruction. Two-phase even within A:
   allocate-all-then-link, because a ref field inside `referent[i]` may point at
   `referent[j]`, `j > i`.
2. **Phase B — W17 deep-frame-stack + module-binding restore** (`resume.rs:503-515`
   empty stub, filled by the W17 facet). This gives `Local`/`ModuleBinding` refs
   their referents.
3. **Phase C — reconstruct `Reference` slots**: `Local`/`ModuleBinding` re-index
   into the now-populated frame stack / module bindings; `TypedField` re-points at
   `by_token[token]` and does **one** `v2_retain` (matching the per-ref share the
   original held). N refs with the same token → same restored pointer → aliasing
   preserved.

### 4.5 `&mut` vs `&` mode (snapshot facet O1)

`RefTarget` does **not** carry mutability today (it's erased at the carrier; B0001
enforces exclusivity statically before any runtime ref exists,
`solver.rs:1073-1079`). **Recommendation: drop the mode from the wire format**
(`is_mut` reserved, always `false`). The whole VM moves as one unit and exclusivity
was proven at compile time; the resumed VM re-executes the same MIR with the same
proof. Deref correctness uses `projected_kind`, not mode. Owned by the borrow-solver
facet's ruling (O1).

---

## 5. Soundness argument

The soundness facet's verdict: **the thesis is directionally salvageable but
as-stated unsound**, for three reasons, all corrected in the narrow flip above.
Per-question:

| # | Question | Verdict | Resolution in this draft |
|---|---|---|---|
| 1 | Whole-VM snapshot preserves `&mut` exclusivity? | **Coherence SAFE** — exclusivity is compile-time-only, intra-function (`solver.rs:1073-1144`); no runtime borrow tracking exists (`grep RwLock\|RefCell\|try_borrow` over the ref path returns nothing). Restore replays the same bytecode → no *new* violation. **But** the real hazard is wrong-frame coordinate resolution: a `Local{frame_index, slot_index}` re-resolved against a *differently-framed* restored stack silently aliases the wrong memory (no diagnostic). | The §4.4 Phase-A-before/Phase-C-after ordering + the W17 deep-frame-stack faithfully reconstructing `base_pointer`/`locals_base` (`executor/snapshot.rs:435`) is the mitigation. Hard dependency on W17 (O6). |
| 2 | Cyclic / self-reference under RC | `Local`/`ModuleBinding` **leak-free** (own nothing — §1.1). `TypedField` **cycle possible**, `Arc` leaks it (non-tracing). | Keep B0004/B0011 **rejecting** container-stored refs (§2.2). The flip is binding-escape + module-binding **only**, never container-store. |
| 3 | Second `&mut` still B0001 after promotion? | **SAFE iff** the flip changes escape-sink→diagnostic mapping only, never loan/conflict generation. | §2.3 surgical shape + N2 sentinel test. |
| 4 | c6 binop-ref gap (`f(&a) + &a`) interaction | **DANGEROUS.** Today `f(&a) + &a` **segfaults** (`06-borrow-check-bypass.md:37-58`): the `&a` operand has no `LoanSink`, flows into `op_add_*`, which cannot add `Ptr(HeapKind::Reference)` to `Int64`. The flip must NOT touch this path — "first-class refs everywhere" would convert the segfault into silent wrong-memory aliasing, OR tempt a forbidden runtime auto-deref coercion. | The flip touches escape-**sink** diagnostics only. The binop-operand reject is **c6 recipe (c)** — *add* `Expr::Binary{lhs\|rhs: Expr::Ref(_)}` rejection at semantic-check (`06-borrow-check-bypass.md:163-166`), landed **independently** of the flip. N3 covers it. |
| 5 | Drop/RAII ordering change | **Safe (no UAF)** iff single-RC-cell owner-of-record + ref-stays-non-owning (§3.2); **semantics CHANGE** (deferred Drop, §3.3). | Single-owner discipline (§3.2) + documented RAII ruling (O4). |
| 6 | Reuse SharedCell identity-map? | **FALSE premise** — no identity-map exists; both Reference and SharedCell are opaque-stub fail-stop today (`snapshot.rs:1104,1106,1325-1335`). | §4.0 corrects the framing: net-new work establishing the shared pattern, ~the entire identity-table subsystem. |

**The genuinely sound minimal flip** = escape-sink→diagnostic remapping
(ReturnSlot + ModuleBindingStore only) + referent-only `SharedCow` promotion
(single owner-of-record) + net-new identity-handle serialization + an
*independent* binop-operand reject (c6 recipe c) + a B0001-survives-promotion
sentinel.

---

## 6. v0.3.3 scope boundary vs v0.4

### IN (v0.3.3)
1. Escape→RC promotion for **ReturnSlot + ModuleBindingStore** referents (the two
   ADR-006 follow-ups: `W17-snapshot-references`,
   `docs/adr/006-value-and-memory-model.md:5975`; `W17-snapshot-sharedcell`,
   `:5977`). Reuses `SharedCow`/`UniqueHeap` + `detect_escape_status`; no new class.
2. Reference wire arm: `ReferenceOpaque` → structured `Reference { is_mut, target }`
   + identity-handle side-table.
3. SharedCell identity round-trip (the binding-identity table the same side-table
   serves).
4. Whole-VM atomicity: reference + referent restore from the *same* `VmSnapshot`.
5. JIT-produced reference slots round-trip **bit-identically** to interpreter-produced
   slots (refs are strictly per-function in JIT,
   `mir_compiler/rvalues.rs:283-286`; the snapshot is VM-level, so a deopted JIT
   frame serializes through the same `slot_to_serializable`).

### OUT (v0.4 live-distributed-sharing)
1. Live cross-node mutable aliasing (two nodes holding live `&mut` to one value).
2. Move-on-send: sending a reference to a *different live VM* and keeping both
   coherent.
3. Cross-VM `&mut` exclusivity enforcement.

**Why OUT is sound (the load-bearing justification):** the *unit of motion* is the
whole VM, not a value. `snapshot()` serializes the entire image; `from_snapshot()`
rebuilds one VM that owns a complete copy. Source and resumed VM never run
concurrently against shared memory — there is never a moment where a reference and
its referent live in two VMs. So the only coherence guarantee snapshot needs is
*intra-image identity consistency* (the handle table). Inter-node live coherence is
a strictly larger, separate feature.

**OUT-boundary tripwire (must stay refused):** a second VM instance referenced from
`from_snapshot`/wire; a "live handle" resolving across VM instances; a `&mut` check
comparing loans from *different* VMs; a `ValueWord`-shaped reference carrier "to
make wire sharing easier" (CLAUDE.md §Forbidden #1 — refuse on sight).

### Also OUT of v0.3.3 (deferred *within* the reference feature, not to v0.4)
- `ClosureEnv` reference escape (§2.2; buried-cell handle, v0.3.4/v0.4).
- `TypedIndex` / typed-array-element refs (the variant is deleted,
  `reference.rs:90-98`; cascade-broken until per-element-kind receiver rebuild
  lands downstream).
- Container-stored refs (B0004/B0011) and task-boundary refs (B0006/B0012) — stay
  **rejecting** (correctness, not deferral).

---

## 7. Test matrix (VM + JIT)

Unit tests only (`#[cfg(test)]`), per CLAUDE.md. Homes: snapshot round-trip in
`executor/snapshot.rs::tests` (mirroring `test_w17_vm_snapshot_*`); wire-arm in
`shape-runtime/src/snapshot.rs::tests`; borrow promote/reject in
`mir/analysis.rs::tests` + `compiler/functions.rs`; JIT round-trip in
`shape-jit/src/core.rs::tests`. Every row runs in **both VM and JIT tiers** unless
noted; JIT rows that hit the β1 ref surface-and-stop (`rvalues.rs:270-281`) assert
**clean deopt to interpreter**, not failure.

### POSITIVE
| # | Property | Tier |
|---|---|---|
| P1 | Reference survives snapshot→resume (restored slot kind `Ptr(Reference)`, not opaque `Err`) | VM + JIT |
| P2 | Deref value correct after restore (referent `42` Int64; deref yields `42`) | VM + JIT |
| P3 | Identity preserved across aliased refs (`r1`,`r2` → same referent; mutate via `r1`, read via `r2`) | VM + JIT |
| P4 | `&mut` still exclusive after restore (one exclusive loan; second `&mut` in resumed program still B0001) | VM (compile-time) |
| P5 | Referent mutation visible through ref (`DerefStore` then `DerefLoad` + direct slot read agree) | VM + JIT |
| P6 | SharedCell identity round-trips (two `var x` share a cell; mutation via one seen by other) | VM |
| P7 | JIT-produced ref slot **bit-identical** to interpreter-produced (build via JIT `MakeRef`, deopt, `snapshot()`, compare arm) | JIT→VM |
| P8 | Promoted referent refcount balances (drop both ref + referent → no leak, no double-free — guards the §3.2 trap) | VM |
| P9 | Whole-VM atomicity (ref in module binding + referent on stack restore from the *same* `VmSnapshot`) | VM |

### NEGATIVE
| # | Property | Tier |
|---|---|---|
| N1 | Genuine dangling still rejected — `UseAfterMove` (ref to a moved value, `solver.rs:1696-1809`, separate pass) stays a hard error | VM (compile-time) |
| N2 | **Second `&mut` still B0001** (`let r1 = &mut x; let r2 = &mut x;` → `ConflictExclusiveExclusive`) — the soundness §3.3 sentinel; **close-gate** | VM (compile-time) |
| N3 | **Binop-ref reject** — `f(&a) + &a` (and any `Expr::Binary{Ref}`) is a clean compile error, not a segfault and not silent aliasing (c6 recipe c, **independent of the flip**) | VM (compile-time) |
| N4 | Discriminator/kind mismatch on restore surfaces structured `Err` (no Bool-default) | VM |
| N5 | Ref stored in container still B0004 | VM (compile-time) |
| N6 | Exclusive ref across task boundary still B0006 | VM (compile-time) |
| N7 | ClosureEnv ref escape still B0003 (deferred, §2.2) | VM (compile-time) |
| N8 | No `ValueWord`-shaped carrier reintroduced; `just check-no-dynamic` green; `no_dynamic.rs` sentinel passes | build gate |
| N9 | Cycle terminates: `TypedField` A→ref→B→ref→A round-trips (reserve-before-recurse) | VM |
| N10 | Back-compat: a pre-v6 snapshot (no `heap_referents`) restores with an empty table | VM |

### Gate
All POSITIVE green in both tiers (JIT deopt rows assert clean deopt); all NEGATIVE
green (promotion is **additive** — any pre-existing B-code reject regression is a
release blocker); `just check-clean` + `just check-no-dynamic` + the 11-check
`scripts/verify-merge.sh` (4-table HeapKind lockstep for the new `Reference` wire
arm) green; `just test` before commit, `just test-all` before tag. The six existing
`test_w17_vm_snapshot_*` smoke tests stay green (extended, not replaced). Per
`MEMORY.md` v0.3.3 carries the FULL 1220 release-blocking set — this matrix is a
subset, not a scope move.

---

## 8. ADR-006 amendment recommendation

**Yes — a new amendment is needed.** This is not covered by an existing §2.7.x.
The soundness facet recommends two amendments; this synthesis folds them into one
new sub-section plus two cross-references:

**New `§2.7.26` (proposed) — reference-escape promotion + snapshot identity-handle:**

1. **Reference-escape promotion rule.** ReturnSlot + ModuleBindingStore reference
   escapes promote the *referent* to `SharedCow` (single owner-of-record); the
   reference binding stays `BindingStorageClass::Reference` as a non-owning handle.
   B0001/B0004/B0006/B0012 and the loan/conflict generation are untouched —
   promotion is a terminal escape-sink→diagnostic remapping only. (Cross-ref:
   ADR-006 escape→RC §2.7.8 + the `type_tracking.rs:286` storage lattice.)
2. **Snapshot identity-handle contract.** The `Reference { is_mut, target }` wire
   arm + the `heap_referents` identity side-table + the three-phase restore
   *supersede* the `ReferenceOpaque` / `SharedCellOpaque` opaque-stub fail-stop
   (`snapshot.rs:507-512`, `:522-529`, `:1325-1335`). The new `Reference` and (later)
   `SharedCell` wire arms enter the §2.7.5.1 snapshot 4-table HeapKind lockstep.
   If a `RefTarget::PromotedCell` variant is introduced (see O1), the
   `as_heap_value()`-is-unsound-on-Reference invariant (`reference.rs:15-16`) and the
   §2.7.13 kinded-carrier rules must be restated for the promoted case, and the new
   variant added to the lockstep table.
3. **Deferred-Drop-on-reference-escape RAII ruling** (one line): is deferring an
   `impl Drop` referent's drop from lexical scope to the escaping reference's
   lifetime the intended RAII semantics? (Rust says yes for owner-moves; Shape's
   case promotes the *referent*.) Needs an explicit ratification + a CLAUDE.md note,
   because it changes observable Drop ordering.

The §2.7.26 sub-section also discharges the CLAUDE.md §Parallel-implementation
attractor: there is **one** identity-table, shared by `Reference` now and
`SharedCell` later — not two carriers meeting at a structural-equivalence layer.

---

## 9. Open questions for supervisor/user

- **O1 — THE central design fork (blocking): does promotion convert a
  `RefTarget::Local` coordinate into a heap-owning handle, or does the `Local` ref
  stay a coordinate that re-indexes into the W17-restored stack?** The facets
  contradict each other. The escape-rc facet (§3.2) proposes a **new
  `RefTarget::SharedCell { cell: Arc<SharedCell>, kind }`** variant (Local refs to
  promoted slots become heap-owning). The snapshot facet (§4.1) serializes `Local`
  **symbolically** (no new variant, re-index on restore). The soundness facet (§6.1)
  proves *a `Local` ref to a non-promoted slot has no serializable identity* — which
  argues the escape-rc heap-ification is **mandatory for serialization**, yet (§2.3,
  §5.4) warns a heap-owning ref reintroduces the cycle-leak + double-drop traps.
  **These cannot all be true simultaneously.** Resolution determines the entire
  carrier shape, the `RefTarget` variant count, the §2.7.13 lockstep, and whether
  the snapshot facet's "symbolic Local" path is even reachable. **Recommend the user
  rule on this before any code.** (My lean: heap-ify only the *referent slot's
  storage* via `SharedCow` so the existing `Local` coordinate now points at a stable
  promoted slot — keeping the ref a non-owning coordinate per §3.2 — and serialize
  the promoted referent through `heap_referents` while the `Local` ref serializes
  symbolically. This reconciles all three facets but needs verification that a
  `SharedCow`-promoted local slot has a stable `(frame_index, slot_index)` that
  survives W17 restore.)

- **O2 — Is there an existing wire-stable `NativeKind` serde** used by the §2.7.7
  parallel-kind track that the frame stack already serializes? If yes, reuse it and
  drop `SerializableNativeKind`. Owned by the W17 deep-frame-stack facet (it must
  serialize kinds for the stack regardless).

- **O3 — Referent body for non-`TypedObject`.** Escape→RC promotion forces the
  referent to a heap object — is it *always* `TypedObjectStorage`, or can a promoted
  scalar / array referent land a different `HeapKind` (and need more
  `SerializableHeapReferentKind` arms)? Note O1's `SharedCow` route makes the
  promoted referent a **`SharedCell`**, not a `TypedObject` — so the `heap_referents`
  side-table likely needs the reserved `SharedCell` kind *in v0.3.3*, not as a
  follow-up. Owned by borrow-solver + storage-planning.

- **O4 — RAII ruling.** Is deferred-Drop-on-reference-escape (§3.3) the intended
  semantics? Needs explicit user/strategic-owner ratification + a CLAUDE.md note.

- **O5 — `&mut` vs `&` mode on the wire.** Confirm option (a): drop the mode
  (`is_mut` reserved, always `false`), relying on the static B0001 proof +
  whole-VM-atomic-move. (Recommended — §4.5.)

- **O6 — W17 deep-frame-stack dependency (hard blocker for `Local`/`ModuleBinding`
  refs).** This feature *cannot* round-trip `Local`/`ModuleBinding` references until
  the W17 deep-frame-stack restore fills `resume.rs:503-515` (empty today). Confirm
  W17 lands first or co-lands, and that it (i) faithfully reconstructs
  `base_pointer`/`locals_base` so `Local` coordinates resolve to the same slots
  (soundness §1.3), and (ii) exposes an allocate-then-link hook so referent-internal
  ref fields patch after all referents exist (§4.4 Phase A).

- **O7 — Scope-prose correction ratification.** The scope-test facet §0/§1.1 prose
  says B0004/B0006/B0012 are flipped; its own matrix (N5/N6) keeps them rejecting.
  This draft resolves to the **narrow** set (ReturnSlot + ModuleBindingStore only).
  Confirm the narrow scope is the binding intent.
