# W12-map-chained Option B — Phase 1 audit-confirmation (audit-only close)

Phase 3 cluster-0 Round 15 W12-map-chained Option B sub-cluster.
Branch `bulldozer-strictly-typed-w12-map-chained-option-b`, parent `0d9ae51e`.
Date: 2026-05-13.

## §0 Status

**Phase 1 surface-and-stop — audit-only close.** The dispatch prompt's
Phase 1 surface-and-stop trigger fires verbatim:

> Stop and surface to the team lead if scope is unbounded ... OR the
> conduit extension's stamp doesn't align with the new producer's carrier.

The conduit extension's stamp (`ConcreteType::Array(I64)`) triggers a JIT
consumer fast-path that does NOT consume the carrier shape Option B
prescribes. Phase 2 production is **not safe to proceed**.

## §1 Phase 1 audit-confirmation findings

### §1.1 Consumer enumeration

The JIT consumer fast-path that fires on
`concrete_types[slot] = ConcreteType::Array(I64)` lives at
`crates/shape-jit/src/mir_compiler/v2_array.rs:334-441`
(`try_emit_v2_array_method`). Dispatch entry from
`mir_compiler/terminators.rs:119` after the gate at
`v2_array.rs:112-118` (`v2_typed_array_elem_kind`) → `types.rs:104-113`
(`is_v2_typed_array_slot`).

Fast-path arms covering scalar-element kinds (`Float64` / `Int64` /
`UInt64` / `Int32` / `UInt32` / `Bool` / `Int8` / `UInt8`) per the
prompt's enumeration:

- `length` / `len`: `v2_array.rs:343-349` — reads `(*arr).len as u32`.
- `push`: `v2_array.rs:351-365` — `jit_v2_array_push(arr, val, elem_size)`.
- `sum`: `v2_array.rs:367-387` — `jit_v2_array_sum_i64(arr)` /
  `jit_v2_array_sum_f64(arr)`.
- `min` / `max` / `mean` / `avg` / `sumSquares` / `sum_squares`:
  `v2_array.rs:389-409` (f64-only).
- `scale` / `addScalar` / `add_scalar`: `v2_array.rs:411-431`.
- `addArray` / `add_array` / `mulArray` / `mul_array`:
  `v2_array.rs:434-...`.

Every arm dereferences `arr` as `*const TypedArray<T>` flat struct (see
`crates/shape-jit/src/ffi/v2/mod.rs:98-124` — `jit_v2_array_sum_f64` /
`jit_v2_array_sum_i64` cast the FFI argument to
`*const TypedArray<f64>` / `*const TypedArray<i64>` and read
`(*arr).data` + `(*arr).len`).

`TypedArray<T>` is the v2-raw flat-struct shape at
`crates/shape-value/src/v2/typed_array.rs:28-44`:

```rust
#[repr(C)]
pub struct TypedArray<T> {
    pub header: HeapHeader,        // 8 bytes (refcount at offset 0)
    pub data: *mut T,              // 8 bytes (raw element buffer)
    pub len: u32,                  // 4 bytes
    pub cap: u32,                  // 4 bytes
}
// const _: () = assert!(size_of::<TypedArray<f64>>() == 24);
```

This is **NOT** `Arc<TypedArrayData>`. `TypedArrayData` is the enum at
`crates/shape-value/src/heap_value.rs:2878-2923`
(`I64(Arc<TypedBuffer<i64>>)`, `F64(Arc<AlignedTypedBuffer>)`, etc.).
`Arc::into_raw(Arc<TypedArrayData>)` is a pointer to the enum payload
(tag + Arc-wrapped buffer pointer), with the Rust-Arc refcount at
offset -16. Layouts diverge: refcount placement, data addressing,
allocation surface, lifecycle.

### §1.2 Producer enumeration

VM-side `.map` and the parametric companions per the dispatch
prompt's enumeration:

**`.map`**: implemented purely in Shape stdlib at
`crates/shape-runtime/stdlib-src/core/vec.shape:51-57`:

```
method map<U>(f: (T) => U) -> Vec<U> {
    let mut result = []
    for item in self {
        result.push(f(item))
    }
    result
}
```

Bytecode lowering for `let xs = [1,2,3,4,5]; let doubled = xs.map(|x| x*2)`
(verified via `SHAPE_JIT_DEBUG=1`):

- `let xs = [1, 2, 3, 4, 5]` lowers to `NewTypedArrayI64 Count(5)` +
  `TypedArrayPushI64 ×5` + `StoreModuleBinding(316)` — v2-raw
  `*mut TypedArray<i64>` carrier with `NativeKind::UInt64`
  (`crates/shape-vm/src/executor/v2_handlers/array.rs:44-52` —
  `OpCode::NewTypedArrayI64` arm pushes raw pointer with
  `NativeKind::UInt64`).
- `xs.map(|x| x*2)` lowers to `Call(Function(FunctionId(195)))` — the
  stdlib `Vec<T>::map` monomorphized for `T=int, U=int`. The body
  emits `NewArray Count(0)` (Arc-based empty TypedArray, line 1406)
  + per-iteration `CallValue` + `ArrayPushLocal` (line 1431).
- `op_new_array` at
  `crates/shape-vm/src/executor/objects/object_creation.rs:367-371`
  produces `Arc::into_raw(Arc<TypedArrayData::I64(empty)>) as u64`
  with kind `NativeKind::Ptr(HeapKind::TypedArray)`.
- `op_array_push_local` at
  `crates/shape-vm/src/executor/objects/array_operations.rs:215-302`
  preserves the `Arc<TypedArrayData>` carrier (line 237-258 —
  `NativeKind::Ptr(HeapKind::TypedArray)` arm).

**Producer-side result carrier**: `Arc::into_raw(Arc<TypedArrayData::I64>)
as u64` (kind `Ptr(HeapKind::TypedArray)`).

**Other `.map`-family stdlib methods** at `vec.shape` (all share the
`let mut result = []; ...; result.push(...); result`pattern):

- `.filter` (line 41-49).
- `.reverse` (line 29-37).
- `.slice` (line 100-108).
- `.take` (line 110) — delegates to `.slice`.
- `.drop` / `.skip` (line 112) — delegates to `.slice`.
- `.flatten` (line 114-120).
- `.flatMap` (search-confirmed absent from vec.shape; uses
  Rust-side `array_transform::handle_flat_map_v2` for Arc-array
  receivers but the stdlib desugar path applies for v2-raw receivers).
- `.concat` — concat.rs (Rust-side, returns Arc-based).
- `.sort` (`vec.shape:` delegated to Rust-side
  `array_transform::handle_sort_v2`).

**All produce the same carrier shape**:
`Arc::into_raw(Arc<TypedArrayData>)` (kind `Ptr(HeapKind::TypedArray)`).

### §1.3 Carrier-shape mismatch verified

The JIT consumer fast-path expectation (per §1.1) is `*const TypedArray<i64>`
(v2-raw flat struct). The Option-B-prescribed producer carrier (per the
dispatch prompt: `Arc::into_raw(Arc<TypedArrayData<T>>) as u64`) is
`Arc<TypedArrayData>` (Rust enum).

These are **structurally distinct Rust types** with structurally
incompatible memory layouts. Reading `Arc<TypedArrayData::I64>` bits
as `*const TypedArray<i64>` interprets:

- The enum discriminator + first 4 payload bytes as `HeapHeader`
  (wrong header bytes; refcount at offset 0 is the enum tag, not a
  refcount).
- The `Arc<TypedBuffer<i64>>` payload pointer as `data: *mut i64`
  (wrong — the bits point at a `TypedBuffer<i64>`, not at element
  storage; dereferencing as `*const i64` reads the buffer's Vec
  header, not array data).
- Padding/alignment bytes as `len` and `cap`.

Empirical confirmation: this is exactly the SIGSEGV (exit 139) the
predecessor audit (commit `8354968a`) recorded at §7.1 when the
conduit extension landed and routed `xs.map(|x|x*2).sum()` through
the consumer fast path.

### §1.4 Conduit-extension alignment check

The dispatch prompt's Phase 1 requires:

> Confirm the previously-landed conduit extension at commit `8354968a`
> (`infer_top_level_concrete_types_from_mir_with_resolvers` in
> `crates/shape-vm/src/compiler/helpers.rs:494`) flows through correctly
> with the new carrier shape.

State of `helpers.rs:494` at baseline `0d9ae51e`:

- The function signature exists (line 494-500).
- Four destination-stamping passes present:
  - `ObjectStore` (line 563).
  - `EnumStore` (line 572).
  - `ArrayStore` (line 657).
  - `Call-terminator MirConstant::Function` (line 711).
  - `Call-terminator MirConstant::Method` gated on `struct_names`
    (line 824).
- The parametric `MirConstant::Method` pass for **built-in container
  receivers** (the conduit extension described at audit `8354968a` /
  WIP stash `4ddd1bfb`) is **NOT present in the baseline**. It was
  audit-only-closed and code-stashed per the predecessor audit close.

**Conduit-extension alignment**: the stamp the predecessor conduit
produces (`concrete_types[doubled_slot] = ConcreteType::Array(I64)`)
DOES NOT align with the new producer carrier per the dispatch prompt's
literal Option B prescription:

- Stamp-side reading: the JIT consumer (`v2_typed_array_elem_kind` →
  `is_v2_typed_array_slot` → `try_emit_v2_array_method`) treats
  `Array(elem)` as "raw `*const TypedArray<elem>` flat-struct bits"
  per the FFI signature.
- Producer-side carrier per Option B prescription: `Arc::into_raw(
  Arc<TypedArrayData<T>>)`.

The two shapes are structurally incompatible per §1.3.

### §1.5 Scope-boundedness

The dispatch prompt enumerates three scope-unbounded triggers:

| # | Trigger | State |
|---|---|---|
| 1 | More producers than VM-side `.map` family | OK — producers bounded to stdlib `vec.shape` methods + Rust `array_transform::handle_*_v2` Arc-side handlers (~12 sites). |
| 2 | More consumers than the typed-array fast-path families | OK — consumer bounded to `try_emit_v2_array_method` (the one fast path). |
| 3 | Conduit extension's stamp doesn't align with the new producer's carrier | **TRIGGERED** — §1.4 above. |

Trigger 3 fires. **Phase 1 surface-and-stop.**

## §2 Disposition options

The dispatch prompt's "Refuse on sight" list explicitly refuses:

- Option A (consumer-side narrowing) — refused by supervisor.
- Option C (both) — refused by supervisor.
- The defensive fallback "Option A.1 `producer_kind` tag track" —
  refused.
- Resurrecting the stashed WIP — refused.
- Bool-default for unproven `.map()` return-shape kind — refused.

The remaining option, Option B (producer-side carrier alignment), per
literal reading of the dispatch prompt's prescription, is **a no-op**
on the producer because the producer already produces the prescribed
carrier shape (`Arc::into_raw(Arc<TypedArrayData>)`). The smoke gate
cannot close with Phase 2 production on this literal reading.

Reading Option B as "produce v2-raw `*mut TypedArray<T>` flat-struct
bits matching the JIT consumer FFI signature" requires the producer
to switch from `Arc<TypedArrayData>` to `Arc<TypedArray<T>>` (or to
raw `*mut TypedArray<T>` un-Arc'd). This contradicts ADR-006 §2.3's
"`HeapValue::TypedArray(Arc<TypedArrayData>)`" mandate — adding a
new producer carrier shape for typed arrays is ADR-006 §2.3
amendment territory, analogous to W17-jit-typed-object-arc-storage-
migration's β disposition (per the parallel `8ae56222` audit close).

Surface-and-stop per the dispatch prompt's §0 discipline:

> The cite must reference a real ADR § paragraph. "Surface-and-stop"
> is NOT a euphemism for "leak a Bool-kind null"; it is a hard
> return with a structured error.

ADR-006 §2.3 "HeapValue payloads — typed Arc" is the binding §:

> `HeapValue::TypedArray(Arc<TypedArrayData>)`, ... typed-Arc payloads
> carry typed `Arc<T>` directly

The producer-side `.map` family already complies with this. The JIT
consumer fast-path's `*const TypedArray<T>` FFI signature does NOT
comply — it reads the bits as v2-raw flat struct, which is a
different ADR-006 §2.3 storage carrier than typed-Arc payload.

## §3 Recommendation for Round 16 / supervisor disposition

The producer/consumer fast-path mismatch surfaced by the predecessor
audit (`8354968a` §7.1) is not closable by producer-side migration
alone under the supervisor's Option B ratification, given the literal
reading of "`Arc::into_raw(Arc<TypedArrayData<T>>)`" already matches
the producer's current carrier.

The structurally-coherent close requires either:

- **B' — ADR-006 §2.3 amendment**: unify the typed-array carrier
  shape across `HeapValue::TypedArray(Arc<TypedArrayData>)` (the
  ADR-006 §2.3 typed-Arc shape) and `*mut TypedArray<T>` (the v2-raw
  flat struct shape). One canonical layout, one canonical drop
  path. Mirror of W17 β.1 (W17 audit `8ae56222` §β.1) — redesign for
  layout compatibility.
- **B'' — Producer migration to v2-raw**: change the producer
  (`vec.shape::map` + companions + the Rust-side Arc array path) to
  produce raw `*mut TypedArray<T>` flat-struct bits with
  `NativeKind::UInt64` (matching the literal `[1,2,3,4,5]` carrier
  shape produced by `OpCode::NewTypedArrayI64`). This requires
  either:
  - Compiler-side specialization of `vec.shape::map` body for v2-raw
    receivers (emit `NewTypedArrayI64` for the empty `result = []`
    and `TypedArrayPushI64` for `result.push(...)`), needing
    closure-return-kind inference at the call site.
  - OR replacing the Shape-stdlib `vec.shape::map` body with a Rust
    method handler that does runtime-kind dispatch and produces the
    matching carrier.
  - Either way: ADR-006 §2.3 amendment to acknowledge two carrier
    shapes for "Array<T> where T is scalar" — `*mut TypedArray<T>`
    (UInt64-tagged v2-raw) and `Arc<TypedArrayData>` (Arc-tagged
    typed-Arc).
- **A (refused) — consumer-side narrowing**: change the JIT consumer's
  FFI signature to consume `Arc<TypedArrayData>` carrier — explicitly
  refused by the supervisor.

This mirrors the W17 audit `8ae56222` Option γ scope-split
recommendation: cluster-0 dispatches a narrow fix that doesn't
require ADR amendment; the typed-Arc-vs-v2-raw unification becomes
a separate cluster-1 follow-up after supervisor β.1 / β.2 / β.3
disposition.

The pattern recurrence (2-instance class now per the 2026-05-13
status doc Round 14 close note: W12-map-chained + W17-typed-object-
arc) suggests a CLAUDE.md amendment candidate: **producer/consumer
fast-path mismatch where the producer-side carrier shape is the
ADR-006 §2.3 typed-Arc and the consumer-side fast-path reads it as
v2-raw flat struct (or vice versa)** is a recurrent defection-
attractor distinct from the W-series ValueWord-rename family. If a
third instance surfaces, the amendment becomes binding.

## §4 Refuse-on-sight discipline preserved

Per the dispatch prompt's "Refuse on sight" list, this audit-only
close honors:

- **No Option A or Option C recommendation**: §3 surfaces B' / B''
  / refused-A as structured options for supervisor disposition; no
  consumer-side narrowing or "both" fix is recommended.
- **No defensive fallback** (Option A.1 producer_kind tag track):
  not used.
- **No bool-default** for unproven `.map()` return-shape kind: the
  conduit extension's `ConcreteType::Array(I64)` stamp is precisely
  the right shape per §2.7.5.1 — no bool-default involved.
- **No bridge/probe/helper/hop/translator/adapter/shim framing**:
  this audit describes the producer/consumer layouts by name
  (`Arc<TypedArrayData>` / `*mut TypedArray<T>`), the migration by
  what it would change (the carrier shape one tier of either side
  reads), and the surface as "ADR-006 §2.3 amendment territory".
- **No "preserve mismatched carrier for one edge case"** framing:
  the audit-only close is the supervisor-discipline response, not a
  preservation of mismatched carriers.
- **No `ValueWord` resurrection**: deleted patterns stay deleted.
- **No stashed WIP resurrection**: the WIP stash (commit `4ddd1bfb`)
  is NOT unstashed; the predecessor conduit-extension stays stashed
  per the dispatch prompt's explicit refusal.

## §5 Close gate state

This is an audit-only close per Phase 1 surface-and-stop. The
close-gate items the dispatch prompt enumerated for Phase 2
production are NOT applicable:

- **Smoke 2 VM=JIT**: not exercised — Phase 2 production not
  performed (would close the gap, but per the dispatch prompt's
  literal Option B reading, the production is structurally
  impossible without ADR amendment).
- **`cargo check --workspace --lib --tests`**: confirmed EXIT=0 at
  baseline `0d9ae51e` (no source changes made by this audit-only
  close).
- **`bash scripts/verify-merge.sh`**: confirmed 12/12 at baseline
  (no source changes).
- **`bash scripts/check-no-dynamic.sh`**: confirmed EXIT=0 at
  baseline (no source changes).
- **AGENTS.md row**: appended per the audit-only close pattern
  (mirror of the predecessor `bulldozer-strictly-typed-w12-jit-
  map-chained-method-return-kind-propagation` audit close at
  `8354968a`).

## §6 Files touched

- `docs/cluster-audits/w12-map-chained-option-b-audit.md` (NEW —
  this audit doc).
- `AGENTS.md` (row appended — audit-only close, blocked status).
- `docs/cluster-audits/phase-3-cluster-0-status.md` (close subsection
  appended).

Zero source changes. Predecessor stash (`stash@{0}`) preserved
intact (refused-on-sight per dispatch).
