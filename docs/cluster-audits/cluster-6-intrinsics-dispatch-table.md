# Cluster 6 audit — intrinsics-typed-CC (renamed from intrinsics-dispatch-table)

**Scope**: `crates/shape-runtime/src/intrinsics/mod.rs` (the
`IntrinsicFn` calling-convention type at line 32 + `IntrinsicsRegistry`
at line 39), residual `IntrinsicFn`-shaped function bodies in
`intrinsics/{math,rolling,array_transforms,recurrence}.rs` and
`crates/shape-runtime/src/multi_table/functions.rs`. Vector / FFT /
matrix / distributions / convolution / stochastic / random /
statistical intrinsics are **already migrated** to typed marshal
entries via `register_typed_fn_N` in the per-file `create_*_intrinsics_module`
factories.

**Predicted error drop**: -8 to -15 in shape-runtime --lib (the residual
`IntrinsicFn`-bodies cite ValueWord/ValueWordExt — Rule G primary +
corollary applies). Plus 0 errors for `IntrinsicsRegistry` deletion
(dead code; mechanical). Total cluster: -8 to -15. Range deliberately
wide; per-file size variability is high (math.rs has 5 deferred fns,
multi_table has 2, others have 1-3 each).

**Audit performed by**: scout-2026-05-07

## Audit 1 — consumer call shape

The 2026-05-07 dated entry at `docs/defections.md:3273-3554` (named
on-record) is the binding context. **Renamed**: previously
"intrinsics-dispatch-table"; the audit confirmed
`IntrinsicsRegistry` is dead code (zero external consumers — shape-vm
bypasses it entirely, dispatching via direct `match builtin` calls).
Correct cluster name going forward: **intrinsics-typed-CC**.

### Migrated consumers (already on `register_typed_fn_N` path)

Confirmed via `grep -n "create_.*_intrinsics_module\|register_typed_fn"`:

- **vector.rs** — 12 entry points (`__intrinsic_vec_abs/sqrt/ln/exp/add/sub/mul/div/max/min/select/add_i64`).
  Inputs: `Arc<AlignedTypedBuffer>` (f64) / `Arc<TypedBuffer<i64>>` (i64)
  zero-copy. Outputs: `ConcreteReturn::ArrayF64` / `ArrayI64`.
- **array_transforms.rs** (partial — 6 of 8 migrated; 2 deferred).
- **rolling.rs** (partial — 3 of 6 migrated; 3 deferred).
- **math.rs** (partial — 14 of 19 migrated; 5 deferred).
- **fft.rs / matrix.rs / convolution.rs / distributions.rs / random.rs
  / recurrence.rs / statistical.rs / stochastic.rs** — fully migrated
  per their `create_*_intrinsics_module` factories.

### Residual `IntrinsicFn`-shaped consumers (still on legacy CC)

Confirmed via `grep -rn "&\[ValueWord\]"`:

#### math.rs (5 deferred per mod.rs:122-137)

- `pub fn intrinsic_sum(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord>`
  (math.rs:188) — polymorphic-return (Vec<int> → int, Vec<number> → number).
  Pending **M1-split sub-decision** (per-element-type intrinsic split).
- `intrinsic_min` (math.rs:211) — same M1-split.
- `intrinsic_max` (math.rs:270) — same M1-split.
- `intrinsic_char_code` (math.rs:331) — multi-input-type dispatch
  (string vs char vs int). Pending **char_code multi-input dispatch
  sub-decision**.
- `intrinsic_bspline2_3d_batch` (math.rs:362) — fast-path/slow-path
  consumer audit pending.

#### rolling.rs (3 deferred per mod.rs:144-159)

- `intrinsic_rolling_sum` (rolling.rs:132) — polymorphic-input
  (Vec<int> fast path vs Vec<number>) plus validity-aware-return for i64
  fast path. Pending **M1-split + validity-aware-return sub-decision**.
- `intrinsic_rolling_min` (rolling.rs:185) — same.
- `intrinsic_rolling_max` (rolling.rs:238) — same.

#### array_transforms.rs (2 deferred per mod.rs:166-177)

- `intrinsic_diff` (array_transforms.rs:194) — polymorphic input/return
  + validity-aware-return for i64 fast path.
- `intrinsic_cumsum` (array_transforms.rs:232) — same.

#### recurrence.rs (1 deferred per mod.rs:180-185)

- `intrinsic_linear_recurrence` (recurrence.rs:121) — passes
  `_ctx: &mut ExecutionContext`; ExecutionContext threading is part
  of the calling-convention question.

#### multi_table/functions.rs (2 — sibling cluster confirmed)

Per defections.md:3304, multi_table is on the same calling-convention
fate as intrinsics:

- `pub fn align_tables(ctx: &mut ExecutionContext, args: &[ValueWord]) -> Result<ValueWord>`
  (multi_table/functions.rs:53) — note the **arg order is reversed**
  (`ctx, args`) compared to IntrinsicFn (`args, ctx`); same shape,
  different convention surface.
- `pub fn correlation(_ctx: &mut ExecutionContext, args: &[ValueWord]) -> Result<ValueWord>`
  (multi_table/functions.rs:155).

multi_table consumer: `crates/shape-jit/src/ffi_symbols/data_access/mod.rs:95`
— JIT FFI shim that synthesizes `ValueWord` args, calls
`align_tables`, then bit-encodes the `ValueWord` return via
`nanboxed_to_jit_bits`. **Cross-crate consumer in shape-jit**;
migrating multi_table as part of this cluster forces a parallel
shape-jit FFI shim update.

### shape-vm dispatch sites (consumers of legacy CC)

`crates/shape-vm/src/executor/builtins/runtime_delegated.rs`:

- Lines 34, 36, 37, 51, 54, 82, 101 — dispatch arms for
  `BuiltinFunction::IntrinsicSum/Min/Max/Diff/Cumsum/RollingSum/CharCode`
  call shape-vm-side `vm_intrinsic_*` shims (in
  `executor/builtins/intrinsics/{math,signal,statistical}.rs`) which
  in turn call into the shape-runtime intrinsics via `delegate(args, fn)`.
- Lines 138, 141 — direct call into `intrinsics::recurrence::intrinsic_linear_recurrence`
  and `intrinsics::math::intrinsic_bspline2_3d_batch`.

**Q2-marshal-fold-light disposition** (per defections.md:3496-3554) is
binding: per-file commits, each commit covers shape-runtime body
migration **AND** shape-vm dispatcher routing (cross-crate atomically),
each dispatcher arm becomes ~5-10 lines looking up via
`module.typed_exports().functions.get("__intrinsic_*")`.

## Audit 2 — marshal-API readiness

### What's already in place

- **`register_typed_fn_N` family** (sync, arities 0-6) plus
  `_full` variants for optional-arg support — per
  `crates/shape-runtime/src/marshal.rs`. Vector / FFT / matrix etc.
  prove the path works end-to-end.
- **`Arc<AlignedTypedBuffer>` + `Arc<TypedBuffer<i64>>` + `Arc<TypedBuffer<u8>>` zero-copy FromSlot/ToSlot impls** —
  the perf-critical input shape for vector intrinsics is established
  and lands at zero data copy. **(Q1) is resolved** per defections.md:3315
  (zero-copy α + ε disposition).
- **`ConcreteReturn::ArrayI64` / `ArrayF64` / `ArrayString` / `ArrayHeapValue`**
  output variants cover the common intrinsic return shapes.
- **Q2-marshal-fold-light disposition signed off** per defections.md:3551 —
  cross-crate per-file commits (shape-runtime body + shape-vm dispatcher
  routing atomically); `BuiltinFunction::IntrinsicVec*` opcodes
  preserved; dispatcher arms reroute to `typed_exports.functions.get(...)`.
- **Migration ordering** suggested per defections.md:3424 (vector.rs
  unary first, math.rs, vector.rs binary, then complex; multi_table
  last).
- **Per-commit revert discipline** is binding: one commit per file,
  no bundling; bench-feasibility check after each commit.

### What's missing (residual sub-decisions)

- **M1-split sub-decision**: polymorphic-return / polymorphic-input
  intrinsics (`sum`/`min`/`max`/`rolling_*`/`diff`/`cumsum`) need a
  decision: split per-element-type (`__intrinsic_sum_i64`,
  `__intrinsic_sum_f64`) or thread through a single typed-input-typed-
  output variant via `Arc<TypedBuffer<T>>` generic? Decision affects
  shape-vm dispatcher arm count.
- **Validity-aware-return sub-decision**: i64-fast-path returns from
  `rolling_sum` / `diff` etc. need a validity bitmap (positions
  before window-full have no value). Current `option_i64_vec_to_nb`
  helper at intrinsics/mod.rs:363 produces `IntArray` with validity
  bitmap. ConcreteReturn::ArrayI64 doesn't carry validity. Decision
  needed: extend ConcreteReturn::ArrayI64 to carry validity, add a new
  ConcreteReturn::NullableArrayI64 variant, or project to TypedArray
  via a different route?
- **char_code multi-input dispatch**: char_code accepts string-of-len-1
  / Char / int. The marshal layer pins one NativeKind per arg; multi-input
  requires either pre-dispatch (compiler-side specialization) or a
  union FromSlot type.
- **bspline2_3d_batch consumer audit**: defections.md:132 calls this
  out as pending; not architectural per se but blocking migration of
  this specific intrinsic.
- **multi_table CC convention difference**: align_tables / correlation
  use `(ctx, args)` not `(args, ctx)`. Migration needs to either
  flip the argument order (mechanical) or pick a different marshal
  entry that takes ctx (no current `register_typed_fn_N_with_ctx`
  arity exists; ModuleContext is already passed).
- **Q2-marshal-fold-heavy follow-on workstream**: deletes
  `BuiltinFunction::IntrinsicVec*` opcodes entirely; defers post-Q2-
  light validation. Out of cluster #6 scope.
- **`IntrinsicsRegistry` deletion** (Q3) — confirmed dead code; pure
  mechanical commit (~50 LoC removed). Predicted 0 errors. Lands
  before/during/after migration with no architectural risk.

## Architectural-shape options

The architectural-shape decision (**Q2-marshal-fold-light**) is signed
off. Three sub-decisions remain.

### Option ML1 — Per-element-type split (M1-split-α)

**Shape**: split each polymorphic intrinsic into per-element-type
typed-marshal entries. `__intrinsic_sum_i64` (input
`Arc<TypedBuffer<i64>>`, output `ConcreteReturn::I64`),
`__intrinsic_sum_f64` (input `Arc<AlignedTypedBuffer>`, output
`ConcreteReturn::F64`). Compiler emits the right variant based on
inferred input element type (already does this for vector.rs's
i64-vs-f64 split — `IntrinsicVecAdd` vs `IntrinsicVecAddI64`).

**Pros**:
- Strict-typed at the marshal layer; each entry pins its NativeKind.
- Mirrors vector.rs's existing precedent (`add` vs `add_i64`).
- shape-vm dispatcher arm count grows by N-per-intrinsic but each
  arm is trivial.

**Cons / risks**:
- BuiltinFunction enum grows by ~10-15 variants (`IntrinsicSum_i64`,
  `IntrinsicSum_f64`, `IntrinsicMin_i64`, ...) — within the
  Q2-marshal-fold-light scope (opcode discriminants preserved per
  defections.md:3514) but adds opcode count.
- Compiler-side type-inference must commit to per-element-type
  resolution before the call site (it already does this for vector
  intrinsics).

**Effort**: 2-3 days for the 5 math + 3 rolling + 2 array_transform
splits.

### Option ML2 — Generic `Arc<TypedBuffer<T>>` via element-type discriminator slot (M1-split-β)

**Shape**: single typed marshal entry per intrinsic name; element-type
read from a side-channel discriminator slot (extra `kind: i64` arg
encoding the element type). Body dispatches inner per-T arm.

**Pros**:
- One marshal entry per intrinsic; no opcode explosion.
- Body-side discrimination is internal; marshal contract is simple.

**Cons / risks**:
- **Watchlist match (close)**: re-introduces an inline kind-tag at the
  marshal-arg layer — same shape as the rejected "Convert<X>To<Y>
  opcode" pattern from CLAUDE.md ("Add a new opcode for this specific
  conversion"). The kind discriminator is a runtime tag-decode by
  another name. **Likely REJECT.**
- Loses the strict-typed input at the marshal layer.

**Effort**: 1-2 days but architecturally suspect.

### Option ML3 — Hybrid: split for hot paths, single-entry for cold paths

**Shape**: vector + math hot-path intrinsics use ML1 (per-element-type
split); cold-path intrinsics (e.g. char_code, bspline2_3d_batch) use
a `ConcreteReturn::Any`-style escape with body-side dispatch.

**Pros**:
- Pragmatic.

**Cons / risks**:
- **Watchlist match**: defections.md:3357 explicitly refuses
  "use β owned-clone for cold-path intrinsics, zero-copy for hot ones
  only — splits the calling convention into hot/cold buckets,
  defection-attractor. All intrinsics use one shape." ML3 is the
  same shape applied to per-element-type vs single-entry split.
  **REJECT.**

**Effort**: N/A (forbidden by intrinsics-typed-CC watchlist).

### Option VAR1 — Validity-aware-return as `ConcreteReturn::NullableArrayI64`

**Shape**: add `ConcreteReturn::NullableArrayI64(Vec<Option<i64>>)`
variant; ToSlot projects to TypedArrayData::I64 with validity bitmap
populated.

**Pros**:
- Strict-typed at the marshal layer.
- Mirrors the `Option<f64>` FromSlot precedent (NaN-sentinel, marshal.rs:118).

**Cons / risks**:
- New ConcreteReturn variant — small additive scope but adds to the
  variant count.
- Vec<Option<i64>> body-side representation is heap-allocating per
  position; perf-sensitive intrinsics may want a different shape
  (`(Vec<i64>, BitVec)`).

**Effort**: 1 day.

### Option VAR2 — Reuse `Arc<TypedBuffer<i64>>` ToSlot and let bodies write validity-bitmap-aware buffers directly

**Shape**: bodies produce `Arc<TypedBuffer<i64>>` with validity bitmap
already populated (matching `option_i64_vec_to_nb`'s current behavior
at intrinsics/mod.rs:363); `ToSlot for Arc<TypedBuffer<i64>>`
preserves the validity bitmap.

**Pros**:
- Zero-copy on the output side.
- Reuses the already-landed `Arc<TypedBuffer<i64>>` ToSlot.
- TypedBuffer already supports validity bitmap (per mod.rs:365 `push_null`).

**Cons / risks**:
- Body-side burden: each migrating body must construct TypedBuffer
  directly rather than via Vec<Option<i64>>. Mostly mechanical.

**Effort**: 1 day; recommended path.

## Recommendation

**Rank**: M1-split → ML1 (per-element-type split). Validity-aware →
VAR2 (reuse `Arc<TypedBuffer<i64>>` ToSlot). char_code → split-per-
input-type at the compiler emission layer (so `char_code(string)`
emits `IntrinsicCharCodeStr`, `char_code(int)` emits
`IntrinsicCharCodeInt`, etc.) — same shape as M1-split-α.
multi_table → fold into intrinsics-typed-CC migration (not a
separate cluster) per defections.md:3304 ("originally clustered with
intrinsics by handover-naming; audit-1 confirms the same architectural
fate").

**`IntrinsicsRegistry` deletion (Q3)** — land mechanically before
the per-file migrations to remove the dead-code surface upfront.

If I were the supervisor I'd execute:
1. C1 — defections.md cluster-execution entry (0 errors).
2. C2 — `IntrinsicsRegistry` deletion (0 errors, dead-code removal).
3. C3-C7 — math.rs migrations (5 fns, 5 commits per per-file revert
   discipline). Each commit covers shape-runtime body + shape-vm
   dispatcher routing atomically.
4. C8-C10 — rolling.rs migrations.
5. C11-C12 — array_transforms.rs migrations.
6. C13 — recurrence.rs migration.
7. C14-C15 — multi_table/functions.rs migrations (with shape-jit FFI
   shim update).
8. Bench-feasibility gate post-cluster.

## Open questions for supervisor

1. **(ML1/ML2)** M1-split shape: per-element-type marshal entries
   (ML1) — strict-typed but adds opcode count? or generic kind-discriminator
   arg (ML2 — refused-by-shape) ? Confirmation that ML1 is the binding
   answer.
2. **(VAR1/VAR2)** validity-aware-return shape: new ConcreteReturn
   variant (VAR1) or reuse Arc<TypedBuffer<i64>> ToSlot with
   validity-bitmap-aware body construction (VAR2)?
3. **(yes/no)** Does multi_table migration land in cluster #6, or as
   a separate "multi_table-typed-CC" sub-cluster? defections.md says
   "same architectural fate"; supervisor confirmation that they
   bundle together (or split for revert granularity).
4. **(yes/no)** Is `IntrinsicsRegistry` deletion (Q3) authorized to
   land C2 before any per-file migration starts, or sequenced after
   the bench-feasibility gate fires?
5. **(yes/no/A/B)** char_code's multi-input-type dispatch: handle at
   compiler emission layer (split-per-input-type, recommended) or
   add a body-side dispatch in a single `__intrinsic_char_code` entry
   taking `ConcreteReturn::Any`?
