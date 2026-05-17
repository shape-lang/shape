# W12-Option-B-reframed — Phase 1 audit-only close (architectural-gap surface)

Phase 3 cluster-0 Round 16 sub-cluster `W12-Option-B-reframed`
(supervisor-ratified after Round 15 W12-Option-B audit-only close
surfaced that the literal Round 14 prescription conflated
`TypedArrayData<T>` (Arc-wrapped Rust enum) with `TypedArray<T>`
(24-byte `#[repr(C)]` flat struct)).

Branch `bulldozer-strictly-typed-w12-option-b-reframed`,
parent `a7b93def` (post-Round-15-W17-narrow-merge + status doc HEAD).
Date: 2026-05-13.

## §0 Status

**Phase 1 surface-and-stop — audit-only close.** The kickoff prompt's
working hypothesis is:

> Option B''' (boundary conversion per §2.7.14-A). Per-variant unwrap-
> and-flatten from `Arc<TypedArrayData::T>` to `*const TypedArray<T>`
> at the JIT FFI boundary. **Other typed-array fast-paths already work
> for direct invocation (e.g. `[1,2,3].sum()` after Round 11A) — they
> must already perform this conversion somewhere. Find that site.**

The audit confirms **no such site exists**. The two carrier shapes
(`Arc<TypedArrayData>` and `*mut TypedArray<T>`) are structurally
unrelated Rust types with disjoint memory layouts, allocation
surfaces, and lifecycle contracts. `[1,2,3].sum()` works **not via
conversion** but because both ends of the literal-array → JIT-consumer
pipeline are natively the flat-struct shape; the Arc shape is produced
only by `op_new_array` (the bytecode-emitter path stdlib `vec.shape::map`
takes) and never enters the JIT typed-array fast path under direct
invocation.

The kickoff's working hypothesis rests on an incorrect premise.
Phase 2 production cannot proceed. Per the kickoff prompt's
surface-and-stop discipline:

> If Phase 1 surfaces a fresh architectural gap, STOP and surface
> for supervisor disposition (do NOT fold in production).

This is the architectural gap. Audit-only close; supervisor disposition
required for Round 17.

## §1 Phase 1 audit deliverables

### §1.1 Deliverable (a) — The existing working conversion site

**Finding: NO conversion site exists.** The two carriers travel
through bytecode-emitter-disjoint paths:

**Path 1 — literal `[1,2,3]` (the working JIT fast-path case).**
Bytecode emission at
`crates/shape-vm/src/compiler/expressions/collections.rs:214-228`:

- Annotated `let arr: Array<int> = [1,2,3]` or inferred-homogeneous-int
  literal → `OpCode::NewTypedArrayI64 Count(3)` +
  `OpCode::TypedArrayPushI64` ×3.

VM-side execution at
`crates/shape-vm/src/executor/v2_handlers/array.rs:44-53`
(`OpCode::NewTypedArrayI64` arm):

```rust
let ptr = TypedArray::<i64>::with_capacity(cap);  // 24-byte flat struct
unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I64) };
self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
```

Producer-side carrier: **`*mut TypedArray<i64>` raw pointer with
`NativeKind::UInt64`** (no Arc, no refcount accounting — the
`HeapHeader.refcount` field at offset 0 is manual per
`crates/shape-value/src/v2/typed_array.rs:28-44`).

JIT consumer at
`crates/shape-jit/src/mir_compiler/v2_array.rs:367-387` (`.sum()`
arm) reads `arr_ptr` as the same raw pointer and passes it to
`jit_v2_array_sum_i64(arr: *const TypedArray<i64>)` at
`crates/shape-jit/src/ffi/v2/mod.rs:115-124`. Both ends are
**natively** the flat-struct shape — there is no Arc to unwrap,
no enum tag to dispatch on.

Empirical confirmation at this commit:
```
$ ./target/release/shape run --mode jit /tmp/sum_literal_smoke.shape
# /tmp/sum_literal_smoke.shape: `let xs = [1,2,3,4,5]; print(xs.sum())`
15
JIT_EXIT=0
```

**Path 2 — stdlib `vec.shape::map`'s `let mut result = []` (the
failing case).** Bytecode emission at the same compiler file's
`else` branch (`collections.rs:229-264`): empty literal with no
spread/nested elements + no `pending_variable_typed_array_kind`
binding annotation falls through to `OpCode::NewArray Count(0)`.

VM-side execution at
`crates/shape-vm/src/executor/objects/object_creation.rs:336-388`
(`op_new_array`, empty-case at `:367-371`):

```rust
let buf: TypedBuffer<i64> = TypedBuffer::from_vec(Vec::new());
let arr = Arc::new(TypedArrayData::I64(Arc::new(buf)));
let bits = Arc::into_raw(arr) as u64;
return self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray));
```

Producer-side carrier: **`Arc<TypedArrayData::I64(Arc<TypedBuffer<i64>>)>`
with `NativeKind::Ptr(HeapKind::TypedArray)`** — a Rust enum whose
discriminator is the variant tag, whose payload is an
Arc-wrapped `TypedBuffer<i64>` (a separate heap allocation), and
whose lifecycle is Rust-Arc (refcount at offset -16 from
`Arc::into_raw`).

`vec.shape::map`'s body
(`crates/shape-runtime/stdlib-src/core/vec.shape:51-57`) iterates
the receiver and calls `result.push(f(item))` for each element.
`result.push(...)` lowers to `OpCode::ArrayPushLocal`, handled at
`crates/shape-vm/src/executor/objects/array_operations.rs:215-302`
— which dispatches on the receiver's `array_kind`:

```rust
match array_kind {
    NativeKind::Ptr(HeapKind::TypedArray) => { ... Arc<TypedArrayData> path ... }
    NativeKind::UInt64 => { ... v2-raw *mut TypedArray<T> path ... }
    _ => Err(NotImplemented(...))
}
```

The two carrier shapes are first-class peers in the VM — they are
**not** convertible at runtime; they are co-existent runtime carriers
selected at bytecode-emission time.

**Conclusion for deliverable (a)**: No conversion site exists. The
prompt's working hypothesis ("find the existing site that performs
per-variant unwrap-and-flatten from `Arc<TypedArrayData::T>` to
`*const TypedArray<T>`") rests on an incorrect premise. Direct
`[1,2,3].sum()` works because the literal path **natively produces
the flat-struct carrier shape**, not because a conversion happens
somewhere.

### §1.2 Deliverable (b) — Why `.map()` results bypass that site

There is no "site" to bypass — but the failure mechanism is well-
identifiable.

The JIT consumer fast-path gate at
`crates/shape-jit/src/mir_compiler/terminators.rs:116-118` reads:

```rust
if let Some(elem_kind) = self.v2_typed_array_elem_kind(&receiver_place) {
    if let Some(()) = self.try_emit_v2_array_method(...) { ... }
}
```

`v2_typed_array_elem_kind` (`v2_array.rs:112-118`) inspects
`concrete_types[slot]` (a **compile-time-stamped** ConcreteType
vector from `BytecodeProgram.top_level_local_concrete_types`):

```rust
pub(crate) fn v2_typed_array_elem_kind(&self, place: &Place) -> Option<NativeKind> {
    let slot = match place { Place::Local(s) => *s, _ => return None };
    is_v2_typed_array_slot(&self.concrete_types, slot.0)
}
```

`is_v2_typed_array_slot` (`types.rs:104-113`) returns `Some(elem)` when
`concrete_types[slot] = ConcreteType::Array(elem_ct)` and `elem_ct`
maps to a scalar Cranelift type.

**The gate is a compile-time type test, not a runtime carrier-shape
test.** It returns `Some(NativeKind::Int64)` for any slot whose
ConcreteType is `Array(I64)` — REGARDLESS of which of the two
runtime carriers the slot actually holds.

For Smoke 2's `doubled = xs.map(|x| x*2)`:

- Pre-Round-14 (without conduit extension): `concrete_types[doubled]
  = Void` (no MirConstant::Method built-in-receiver pass at
  `helpers.rs:494`); the gate returns `None` → routes through
  generic `jit_call_method` → result is whatever the method
  dispatcher returns. Downstream `doubled.sum()` reaches the same
  `None` gate → also generic dispatch. Eventual `print(...)` hits
  the §2.7.5 surface-fire because the operand kind track
  propagates `None` through the chain (`terminators.rs:572-612`).
  Empirical confirmation at this commit:
  ```
  $ ./target/release/shape run --mode jit /tmp/kickoff_smoke2_full.shape
  Error: Runtime error: JIT compilation failed: Route A surface-and-stop:
  NotImplemented(SURFACE) — `print` Call-terminator operand NativeKind is None
  JIT_EXIT=1
  ```

- Post-Round-14 (with conduit extension at predecessor stash
  `4ddd1bfb`, currently NOT applied at baseline `a7b93def`):
  `concrete_types[doubled] = Array(I64)` (correctly stamped per the
  Round 14 audit at
  `docs/cluster-audits/w12-jit-map-chained-method-return-kind-propagation-audit.md`
  §1.3). The gate now returns `Some(Int64)` → `try_emit_v2_array_method`
  fires for `.sum()` → emits a direct `jit_v2_array_sum_i64(arr_ptr)`
  call that expects `arr_ptr` to be a `*const TypedArray<i64>` flat
  struct. But the **runtime carrier** under the slot is
  `Arc<TypedArrayData::I64>` (produced by `op_new_array` inside
  `vec.shape::map`'s body). The FFI body reads `(*arr).data` at
  offset 8 (`ffi/v2/mod.rs:117`), which on the Arc carrier is the
  Arc-payload pointer interpreted as `*mut i64` — wrong-type
  dereference → SIGSEGV (predecessor audit `8354968a` §7.1
  recorded this exit code 139).

**Conclusion for deliverable (b)**: `.map()` results route to the
JIT consumer fast-path when (and only when) `concrete_types[slot]`
is stamped `Array(_)`. The fast-path was designed for the
literal-array carrier shape (flat struct), but the ConcreteType
stamp does not discriminate between the two runtime carriers. Per
ADR-006 §2.7.5 the producer's carrier shape is the authoritative
type — the JIT consumer's fast-path assumes a stricter runtime
invariant than the compile-time ConcreteType stamp guarantees.

### §1.3 Deliverable (c) — The minimal fix shape

**No minimal fix shape exists** that routes `.map()` results through
"the same conversion" (because no conversion site exists per §1.1).

The honest disposition options remain those enumerated in the
Round 15 W12-Option-B audit
(`docs/cluster-audits/w12-map-chained-option-b-audit.md` §3) and the
Round 14 predecessor audit
(`docs/cluster-audits/w12-jit-map-chained-method-return-kind-propagation-audit.md`
§7.2):

- **B' — ADR-006 §2.3 amendment**: unify the typed-array carrier
  shape across `HeapValue::TypedArray(Arc<TypedArrayData>)` (typed-Arc
  shape) and `*mut TypedArray<T>` (v2-raw flat struct). One canonical
  layout. Mirror of W17 β.1 redesign-for-layout-compatibility scope.
  Multi-week.
- **B'' — Producer migration to v2-raw**: change `vec.shape::map`'s
  bytecode lowering so the empty literal `let mut result = []`
  emits `NewTypedArrayI64` (selected by element-kind inference at
  the binding site) and `result.push(...)` emits `TypedArrayPushI64`
  (selected by receiver-kind inference at the call site). Requires
  either closure-return-kind threading into `let mut result = []`'s
  binding-annotation track, OR a Rust-side method handler that
  intercepts before stdlib's `Call(FunctionId)` dispatch and emits
  the v2-raw carrier directly. Either route requires ADR-006 §2.3
  amendment to acknowledge dual carrier shapes for `Array<T>` where
  T is scalar (the current §2.3 mandates `Arc<TypedArrayData>` as
  the typed-array HeapValue payload — silent on the `UInt64`-tagged
  `*mut TypedArray<T>` peer carrier).
- **A — Consumer-side narrowing (refused by supervisor at Round 15)**:
  restrict `try_emit_v2_array_method` to receivers whose **runtime
  carrier** is verifiably `*mut TypedArray<T>` — e.g. via a parallel
  `producer_kind` tag track that flows the bytecode-emission-time
  carrier choice into the JIT compile-time fast-path gate. Explicitly
  refused by supervisor at Round 15.
- **C — Both (refused by supervisor at Round 15)**.
- **B''' — Boundary conversion at JIT FFI dispatch shell (the kickoff
  prompt's working hypothesis)**: at the JIT consumer's fast-path
  entry, when the **runtime carrier** under the slot is the Arc
  shape, emit a per-variant unwrap-and-flatten that allocates a
  fresh `TypedArray<T>` (via `TypedArray::<T>::with_capacity(...)` +
  `from_slice` or equivalent), copies the `TypedBuffer<T>` payload
  into the new flat struct's `data` buffer, then dispatches the
  matching FFI entry. **Bounded-LoC fix** at first glance, but the
  conversion is O(n) per call site, not a constant-time pointer
  rewrap — and the new allocation has different lifecycle (raw
  pointer vs Arc), which would either leak the Arc payload (silent
  refcount imbalance) or require an additional drop arm in
  `release_func_for_place` that releases the original Arc share
  after the copy. This is the structural defect Round 14/15 both
  surfaced under different framings: the two carriers cannot be
  bridged by a "conversion" because they hold structurally distinct
  data (enum tag + Arc<TypedBuffer> indirection vs flat 24-byte
  repr(C) struct). A copy is the only way, and a copy is not a
  "conversion" — it's a per-call O(n) materialization. The
  defection-attractor framing here is to call the copy a "boundary
  conversion" when it is actually a synthesis of a new value with
  a different lifecycle.

**Conclusion for deliverable (c)**: the minimal fix shape the kickoff
prompt sought (per-variant unwrap-and-flatten "at the JIT FFI
boundary") does not exist as a bounded production scope. The
underlying structural mismatch the Round 15 W12-Option-B audit
surfaced has not been eliminated by the W12-Option-B-reframed
rescoping — it has been re-described.

### §1.4 Deliverable (d) — §2.7.14-A amendment posture

The supervisor-provided draft §2.7.14-A text frames the two shapes as
"a two-shape contract" with a "boundary conversion site at the JIT
FFI dispatch shell". My audit finds that **the draft text mis-
describes the runtime reality** in three load-bearing places:

1. **"Heap carrier (canonical per §2.3): `Arc<TypedArrayData::T>`"** —
   The carrier is `Arc<TypedArrayData>` (Rust enum, variant carries
   `Arc<TypedBuffer<T>>` per element kind). The `T` in
   `TypedArrayData::T` is an enum-variant tag (`I64` / `F64` / ...),
   not a type parameter on the carrier struct.

2. **"`Arc<TypedArrayData::T>` ... is dispatch-friendly (variant
   selects monomorphization)"** — The variant does select dispatch
   inside the VM (per `op_array_push_local`'s match arm). But at
   the JIT FFI boundary the variant is **not** the discriminator;
   the discriminator is `NativeKind::Ptr(HeapKind::TypedArray)` vs
   `NativeKind::UInt64`, which the bytecode emitter has already
   committed to at the producing call site. A "boundary conversion"
   that reads the runtime variant tag and dispatches accordingly
   is per-call branching, not per-call-site monomorphization —
   structurally distinct from the §2.7.14 Q15 Route A claim
   ("monomorphized per-element-kind FFI entry points").

3. **"The conversion site is the JIT FFI dispatch shell: per-variant
   unwrap-and-flatten before calling the monomorphized FFI entry."**
   — There is no extant "conversion site" the amendment can name. A
   newly-introduced conversion site at the JIT consumer would have
   to allocate a fresh `TypedArray<T>` and **copy the payload**;
   the original Arc share must then be released (or the new copy
   leaks the buffer). This is a synthesis, not a conversion. The
   draft's "unwrap-and-flatten" framing reads as if the data lives
   in a compatible layout that just needs different addressing; in
   reality the layouts are not compatible. The draft language hides
   this asymmetry under "conversion" terminology.

**Recommendation for deliverable (d)**: do NOT commit the draft
§2.7.14-A text as-is. The draft framing repeats the Round 14
audit's conflation in different vocabulary. The structurally-honest
amendment shape names the two carrier shapes as **runtime peers**
(not as "two-shape contract") and acknowledges that:

- Choice between them is made at bytecode-emission time, based on
  whether the binding site has a statically-resolvable scalar
  element kind (`NewTypedArrayI64` path) vs not (`NewArray` path).
- The `NativeKind` discriminator at every runtime site
  (`Ptr(HeapKind::TypedArray)` vs `UInt64`) is the authoritative
  dispatch source; there is no automatic conversion between them.
- The JIT consumer fast-path at `v2_array.rs::try_emit_v2_array_method`
  is reachable only via `concrete_types[slot] = Array(_)`, which is
  the **ConcreteType** stamp (compile-time), not the runtime
  `NativeKind`. The gap is that ConcreteType is too coarse a
  discriminator: both runtime carriers map to ConcreteType
  `Array(I64)`, but only one of them is sound to dispatch to the
  flat-struct FFI.

The structurally-coherent close requires one of:

- **B' / B''** (ADR-006 §2.3 amendment routes as enumerated in
  Round 15 W12-Option-B audit §3).
- **A new architectural ruling** that the JIT consumer fast-path
  gate must read the **runtime `NativeKind`** (not the compile-time
  `ConcreteType`) — equivalent to a constrained Option A (refused
  by supervisor at Round 15) but reframed as "use the authoritative
  per-slot kind track per §2.7.7/Q9 instead of the broader-grained
  ConcreteType stamp". This is the W17-narrow Round 15 pattern
  applied to the typed-array fast-path gate. It does not require
  ADR amendment — §2.7.7/Q9 already mandates the parallel-kind
  track as authoritative. The two-carrier ambiguity arises only
  because the gate uses ConcreteType instead.

The third option above merits separate disposition; it is structurally
different from Round 15's Option A (which was framed as a "defensive
guard" with a new `producer_kind` side-table). The §2.7.7/Q9-based
approach reads the kind from the existing authoritative track, no
new side-table. It is the same shape as W17-narrow's
`receiver_type_name` migration from tag-bit dispatch to NativeKind
dispatch.

## §2 Scope-boundedness check (Phase 1 gate)

The kickoff prompt's Phase 1 surface-and-stop trigger fires:

> If your Phase 1 audit finds the conversion site lives at
> [some specific location], you may overlap territory with
> follow-up-A — coordinate via AGENTS.md row append. **Surface to
> team lead if file-level territory overlap is unavoidable.**
>
> If Phase 1 surfaces an architectural gap requiring ADR amendment
> beyond §2.7.14-A or a fresh sub-cluster scope, STOP and surface
> to the team lead with a structured error report. Do NOT fabricate
> a fallback. Do NOT widen scope into Option A/C territory or
> carrier unification.

Both subtriggers fire:

1. The conversion site does not exist (§1.1).
2. The §2.7.14-A draft text mis-describes the runtime reality and
   committing it would lock in the Round 14 conflation in different
   vocabulary (§1.4).

**Audit-only close per Phase 1 surface-and-stop discipline.**

## §3 Disposition options for Round 17

Same three structural options enumerated at the Round 14 / Round 15
audits, now with a fourth (the §2.7.7/Q9-based gate-narrowing):

- **B' — ADR-006 §2.3 amendment**: unify the carrier shapes. Multi-week.
- **B'' — Producer migration to v2-raw + ADR amendment**: migrate
  `vec.shape::map` (and parametric companions) to emit
  `NewTypedArrayI64`-class opcodes when the element kind is
  statically resolvable; amend ADR-006 §2.3 to acknowledge dual
  carrier shapes for scalar `Array<T>`.
- **A-§2.7.7 — JIT-consumer-fast-path gate narrowing via NativeKind
  read from the §2.7.7/Q9 parallel-kind track** (new disposition,
  NOT the Option A refused at Round 15 — distinct framing): change
  `v2_typed_array_elem_kind` from a `concrete_types[slot]` lookup
  to a runtime-NativeKind-track lookup at the dispatch shell. The
  fast path fires only when `NativeKind` is `UInt64` AND the slot's
  element-kind tag (the `stamp_elem_type` byte at offset 6 in the
  heap header) matches the expected ConcreteType element. When
  `NativeKind` is `Ptr(HeapKind::TypedArray)` the fast path
  declines → generic `jit_call_method` dispatch path takes over →
  the Arc-carrier-shape method body returns through the §2.7.10/Q11
  ABI carrier. This is structurally the W17-narrow pattern applied
  to the typed-array gate, and aligns with the supervisor's
  Round 15 disposition for W17-narrow.
- **C — refused at Round 15**.
- **A — refused at Round 15** (the `producer_kind` side-track form).

Option **A-§2.7.7** is the smallest scope and does NOT require ADR
amendment — but it changes the gate's behavior such that
`vec.shape::map` results continue through the generic method-dispatch
ABI rather than the typed-array fast-path. The performance impact is
that `.map().sum()` runs through method-dispatch trampoline for the
`.sum()` step instead of the inline SIMD reduction, until B' or B''
land. That is consistent with the current state at baseline
`a7b93def` (which surfaces `Route A surface-and-stop:
NotImplemented(SURFACE) — print Call-terminator operand NativeKind is
None` because **no conduit extension is applied**, so the generic
dispatch path's print-operand kind ends up `None`).

## §4 Refuse-on-sight discipline preserved

Per the kickoff prompt's "Refuse on sight" list:

- **No bridge/probe/helper/hop/translator/adapter/shim framing** for
  the proposed conversion site. The audit describes the two shapes
  by name (`Arc<TypedArrayData>` / `*mut TypedArray<T>`), the
  bytecode-emission split by what determines it (annotated/inferred
  element kind at literal site), and the supervisor-options space
  by what each amendment would commit to.
- **No carrier unification via deletion** of either shape recommended.
  Both runtime carriers are first-class peers; the runtime supports
  them in lockstep (see `op_array_push_local` / `op_array_pop` /
  `op_slice_access` per-carrier arms at
  `crates/shape-vm/src/executor/objects/array_operations.rs`).
- **No Option A / Option C** recommended. **Option A-§2.7.7 is
  distinct from Option A** (different mechanism: reads the
  authoritative per-slot NativeKind track from §2.7.7/Q9, not a
  new `producer_kind` side-table). Surfaced explicitly for
  supervisor disposition.
- **No defensive fallback** recommended.
- **No Bool-default** for unproven `.map()` return-shape kind.
- **No `ValueWord` resurrection**: deleted patterns stay deleted.
- **No predecessor stash (`4ddd1bfb`) resurrection**: refused on
  sight per the Round 15 W12-Option-B audit's discipline.
- **§2.7.14-A draft NOT committed as-is**: the draft repeats the
  Round 14 conflation in different vocabulary; committing it would
  lock in the wrong-architecture framing the W-series defection-
  attractor family refuses. The draft is preserved verbatim in this
  audit doc (§5) so future agents see what was considered-and-not-
  committed.

## §5 §2.7.14-A draft text (supervisor-provided, NOT committed)

The supervisor's draft amendment text, recorded here for the audit
trail. Refused-on-sight commit per §1.4 / §4 above:

> §2.7.14-A — Route A realization at the JIT FFI boundary
>
> Q15's Route A decision (monomorphized per-element-kind FFI entry
> points) carries an implicit two-shape contract:
>
> - Heap carrier (canonical per §2.3): `Arc<TypedArrayData::T>` where
>   T is the per-element-kind enum variant carrying
>   `Arc<TypedBuffer<T>>`.
> - JIT FFI consumer shape: `*const TypedArray<T>` where
>   `TypedArray<T>` is the runtime-v2 native 24-byte `#[repr(C)]`
>   flat struct.
>
> These shapes are structurally distinct by design — the heap carrier
> is dispatch-friendly (variant selects monomorphization); the FFI
> consumer shape is codegen-friendly (direct pointer arithmetic on
> data offset). The conversion site is the JIT FFI dispatch shell:
> per-variant unwrap-and-flatten before calling the monomorphized
> FFI entry. The conversion is the §2.7.14 Q15 Route A boundary
> realization, not a "bridge" or "compatibility helper" — it is the
> canonical lowering from heap carrier to JIT consumer expectation.
>
> Forbidden: any framing that calls this conversion a "bridge"/
> "probe"/"helper"/"hop"/"translator"/"adapter"/"shim" (CLAUDE.md
> broader-family rule); any framing that proposes "carrier
> unification" via deletion of either shape (loses information at
> the side that's deleted).
>
> The conversion site MUST exist for every Route A consumer site
> that receives results from a heap-dispatched producer (`.map()`,
> `.filter()`, etc.). The Round 14 audit's literal "Option B"
> conflation merged the two shapes by accident; this amendment
> names the boundary explicitly to prevent future conflations.

**Reasons not committed**:

1. The "conversion site is the JIT FFI dispatch shell" framing
   asserts the site exists; the audit confirms it does not exist
   (§1.1).
2. The "per-variant unwrap-and-flatten" framing describes the two
   layouts as variants of a compatible representation; they are
   structurally distinct types with separate allocations (§1.4).
3. The "must exist for every Route A consumer site that receives
   results from a heap-dispatched producer" framing prescribes work
   without describing what the work is — the actual work is
   per-call O(n) data copy + lifecycle re-shape, which is a
   different architectural decision than a "site". Refusing to
   commit the draft preserves the supervisor's ability to make
   that decision in Round 17 with full information.

## §6 Close gate state

Audit-only close per Phase 1 surface-and-stop discipline.

**Pre-commit verification at baseline `a7b93def`** (this audit's
parent commit):

- `cargo check --workspace --lib --tests` EXIT=0 (verified in-shell).
- `bash scripts/verify-merge.sh` EXIT=0, 12/12 PASS (verified
  in-shell, last 20 lines preserved in close-report).
- `bash scripts/check-no-dynamic.sh` EXIT=0 (verified in-shell).

**Smoke 2 baseline state** (audit §0):
- `--mode vm` prints `30` (working).
- `--mode jit` errors `Route A surface-and-stop:
  NotImplemented(SURFACE) — print Call-terminator operand NativeKind
  is None`. This is the **pre-conduit-extension** surface (the
  Round 14 conduit-extension WIP was audit-only-closed with code
  stashed per supervisor's Round 14 disposition; the predecessor
  branch's `stash@{0}` is not applied at baseline `a7b93def`).
  SIGSEGV is NOT currently reachable on this baseline.

Zero source changes by this audit-only close. Only docs:

- `docs/cluster-audits/w12-option-b-reframed-audit.md` (this doc).
- `AGENTS.md` (row appended).
- `docs/cluster-audits/phase-3-cluster-0-status.md` (subsection
  appended).

## §7 Files touched

| File | Change |
|---|---|
| `docs/cluster-audits/w12-option-b-reframed-audit.md` | NEW — this audit. |
| `AGENTS.md` | Row appended (audit-only close, blocked status). |
| `docs/cluster-audits/phase-3-cluster-0-status.md` | Subsection appended. |

Zero source changes. Predecessor stash (`stash@{0}` `4ddd1bfb`)
NOT resurrected — refused-on-sight per kickoff prompt.

## §8 Recommendation for Round 17

Surface to supervisor for Round 17 disposition between four
structural options:

1. **B'** (ADR-006 §2.3 amendment to unify carrier shapes).
2. **B''** (producer migration to v2-raw + ADR amendment to
   acknowledge dual carrier shapes for scalar `Array<T>`).
3. **A-§2.7.7** (JIT-consumer-fast-path gate narrowing via
   NativeKind read from §2.7.7/Q9 parallel-kind track — new
   disposition, NOT the Round 15-refused Option A, see §1.4 / §3).
4. **Status quo + accept performance gap** (the typed-array
   SIMD fast-path simply does not fire for `vec.shape::map`
   results; generic method-dispatch ABI handles them via the
   Arc carrier; Smoke 2 closes by extending the conduit at
   `helpers.rs:494` for the **non-fast-path** generic dispatch
   route's kind propagation through `print`).

Producer/consumer fast-path mismatch is now a **3-instance
defection-attractor class** (per Round 15 W12-Option-B audit §3
recommendation):
- W12-map-chained (closed audit-only Round 14, conduit-stashed).
- W17-typed-object-arc-storage-migration (closed audit-only Round 14,
  ADR-006 §2.3 amendment territory; cluster-0 unblocked by
  W17-narrow Round 15).
- W12-map-chained-option-b (closed audit-only Round 15, same
  defection family).

This audit (W12-Option-B-reframed) is the **fourth instance** of
the same class, surfacing again at the same surface. CLAUDE.md
"Forbidden Patterns" amendment binding per the supervisor's
2026-05-13 Round-14-close recurrent-pattern note.

## §9 Original kickoff text preserved verbatim

The Round 16 W12-Option-B-reframed dispatch prompt's working
hypothesis, preserved here verbatim per audit-trail discipline:

> Working hypothesis: Option B''' (boundary conversion per §2.7.14-A)
>
> Neither B' (carrier unification) nor B'' (producer migration to
> flat-struct) fits cleanly. Working hypothesis: each layer keeps
> its natural shape, and the conversion happens at a well-named
> boundary site. Per-variant unwrap-and-flatten from
> `Arc<TypedArrayData::T>` to `*const TypedArray<T>` at the JIT
> FFI boundary. **Other typed-array fast-paths already work for
> direct invocation (e.g. `[1,2,3].sum()` after Round 11A) — they
> must already perform this conversion somewhere. Find that site.**

The audit's response: the working hypothesis rests on an incorrect
premise. The conversion site does not exist. `[1,2,3].sum()` works
because the literal-array bytecode-emission path natively produces
the flat-struct carrier shape, not because a conversion happens
somewhere. The two carriers are runtime peers selected at
bytecode-emission time, with bytecode-emitter-disjoint paths to
their respective JIT consumer surfaces. Phase 1 surface-and-stop.
