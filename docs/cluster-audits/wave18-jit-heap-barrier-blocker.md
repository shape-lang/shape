# Wave 18 JIT heap mutation/barrier blocker

Date: 2026-07-09

Scope: static inspection only. No cargo, rustc, tests, builds, benchmarks, or
book-truth commands were run.

## Question

`benchmarks/shape/17_jit_heap_field_overwrite.shape` is intended to measure the
default-GC JIT write barrier on repeated heap typed-object field overwrites:

```shape
type BarrierNode {
    peer: Option<BarrierNode>,
    payload: int,
}

let mut a = BarrierNode { peer: None, payload: 1 }
let mut b = BarrierNode { peer: None, payload: 2 }
let mut c = BarrierNode { peer: None, payload: 3 }

b.peer = Some(b)
c.peer = Some(c)

while i < 5000000 {
    if i % 2 == 0 {
        a.peer = Some(b)
    } else {
        a.peer = Some(c)
    }
}
```

Wave 17 made the scalar control fixture,
`benchmarks/shape/18_jit_scalar_field_overwrite.shape`, native-JIT in both the
default and `--no-default-features --features jit` artifacts. The heap fixture
still falls back before write-barrier timing.

## Current first fallback surface

The first/current fallback is a MIR preflight blocker named
`FieldProjectionAssign`, not the actual GC write barrier.

`crates/shape-jit/src/compiler/strategy.rs:59-69` and
`crates/shape-jit/src/compiler/strategy.rs:235-244` run top-level MIR preflight
before lowering. A failed preflight returns:

```text
MirToIR: top-level preflight failed: ...
```

The specific guard is in `crates/shape-jit/src/mir_compiler/mod.rs`.

`crates/shape-jit/src/mir_compiler/mod.rs:585-588` initializes
`field_projection_assigns`.

`crates/shape-jit/src/mir_compiler/mod.rs:722-728` increments it for direct
field-projection assignments into locals:

```rust
Rvalue::Use(
    Operand::Copy(Place::Field(_, _))
        | Operand::Move(Place::Field(_, _))
        | Operand::MoveExplicit(Place::Field(_, _)),
) if matches!(place, Place::Local(_))
```

`crates/shape-jit/src/mir_compiler/mod.rs:780-789` rejects two or more such
assignments with:

```text
FieldProjectionAssign SURFACE (Wave-17 scalar-move-lift): MIR contains ...
direct field-projection local assignments, the cluster shape produced by object
destructuring. This path is not independently proven on the JIT path and was
previously masked by the conservative move-then-read fallback; whole-program
deopt preserves VM == JIT until projection assignment lowering is proven.
```

That is the exact remaining blocker reported by the Wave 16/17 notes: the heap
benchmark falls back before the write barrier can be timed because top-level
MIR preflight rejects field projection assignment patterns.

## Why a write benchmark trips a field-projection read guard

The source benchmark mostly writes fields. The MIR shape still contains direct
field-projection reads because expression-level assignment lowering reads the
assigned target back into a temporary.

`crates/shape-ast/src/parser/expressions/binary_ops.rs:367-388` parses property
assignment into `Expr::Assign { target, value }`.

`crates/shape-vm/src/mir/lowering/expr.rs:264-271` lowers property/index
assignment targets through `lower_assign_target_place`.

`crates/shape-vm/src/mir/lowering/expr.rs:2105-2121` lowers `Expr::Assign` by:

1. assigning the RHS value into the target place, and
2. assigning `Copy(target_place)` into a temp as the expression result.

For `a.peer = Some(b)`, step 2 becomes a direct field-projection local
assignment even when the assignment expression result is unused by the source
statement.

Classification: this is a field-projection assignment / move-copy-liveness
preflight issue. It is not primarily heap pointer kind support, user-op
dispatch, or the GC barrier call itself.

## The next blockers after preflight

Clearing `FieldProjectionAssign` is necessary but not sufficient. The heap
fixture exercises two additional surfaces that a worker should handle in the
same implementation lane, otherwise the benchmark can become native-JIT but
measure the wrong thing or fall back at the next guard.

### 1. `Some(...)` construction is currently fail-closed in MIR JIT

Bare `Some(x)` lowers through MIR enum store:

`crates/shape-vm/src/mir/lowering/helpers.rs:430-463` recognizes bare
`Ok`/`Err`/`Some`.

`crates/shape-vm/src/mir/lowering/expr.rs:2291-2353` lowers the bare constructor
call into an aggregate and `ContainerStoreKind::Enum`.

The JIT rejects Result/Option enum stores before reaching the stale old
allocation path.

`crates/shape-jit/src/mir_compiler/statements.rs:320-337` returns:

```text
EnumStore: SURFACE -- JIT Result/Option variant 'Some' construction is disabled
by W88A before FFI allocation. The old jit_v2_make_result_* /
jit_v2_make_option_* imports would allocate Arc<ResultData>/Arc<OptionData);
the replacement must build schema-backed __Result / __Option TypedObjectStorage
from a statically known helper ABI. Whole-function deopt preserves VM == JIT.
```

The runtime VM already builds canonical schema-backed carriers:

`crates/shape-vm/src/executor/vm_impl/builtins.rs:792-808` routes `SomeCtor`
through `result_option_carrier::build_some`.

`crates/shape-vm/src/executor/result_option_carrier.rs:59-70` builds
`Some`/`None`.

`crates/shape-vm/src/executor/result_option_carrier.rs:108-139` constructs a
`TypedObjectStorage` with `field_kinds` and `heap_mask`, then returns a
`KindedSlot::from_typed_object_raw(ptr)`.

Do not use the old JIT result/option producers. `crates/shape-jit/src/ffi/result.rs:16-22`
marks them as retired, and `crates/shape-jit/src/ffi/result.rs:249-256` routes
`jit_v2_make_option_some` / `jit_v2_make_option_none` to the retired producer
surface.

### 2. `Option<T>` field stores must not silently skip the barrier

The inline typed field write path can call the GC barrier when it knows the
field kind.

`crates/shape-jit/src/mir_compiler/places.rs:781-831` implements
`inline_typed_field_set`. With the `gc` feature enabled it:

1. maps a known field kind through `shape_value::gc::gc_jit_kind_tag`,
2. loads the old slot,
3. stores the new slot, and
4. calls `self.ffi.write_barrier` when the tag is nonzero.

`crates/shape-jit/src/mir_compiler/places.rs:833-843` resolves the field kind
from `field_native_kinds`.

`crates/shape-jit/src/mir_compiler/places.rs:1338-1362` uses the inline write
when a field byte offset is known.

However, static field-kind projection intentionally does not handle
`FieldType::Option`.

`crates/shape-jit/src/mir_compiler/mod.rs:1321-1475` populates field offsets and
static field native kinds. `FieldType::Option(_)`, `FieldType::Object(_)`,
`FieldType::Array(_)`, `Any`, maps, sets, and other dynamic surfaces currently
return `None`.

`crates/shape-runtime/src/type_schema/field_types.rs:258-285` also refuses to
convert `FieldType::Option(_)` to a single `NativeKind`, because the actual slot
kind lives in object storage.

If a worker simply relaxes preflight and reaches
`inline_typed_field_set(..., field_kind = None)`, the inline path will use tag
`0` and skip the write barrier. That would make the benchmark native-JIT but
invalidate the default-GC barrier measurement.

There is already a safer dynamic field-set FFI path:

`crates/shape-jit/src/ffi/typed_object/field_access.rs:68-103` implements
`jit_typed_object_set_field`. Under `gc` it reads
`(*ptr).field_kinds[idx]`, maps that runtime kind through
`gc_jit_kind_tag`, writes the slot, and calls `jit_write_barrier` with the
dynamic tag.

That path is slower than a fully inline store, but it is barrier-correct for
`Option<T>` / dynamic fields and keeps the program native-JIT eligible.

## Smallest safe implementation lane

Recommended write scope:

- `crates/shape-jit/src/mir_compiler/mod.rs`
- `crates/shape-jit/src/mir_compiler/statements.rs`
- `crates/shape-jit/src/mir_compiler/places.rs`
- `crates/shape-jit/src/ffi_refs.rs`
- a new or existing focused JIT FFI helper module for schema-backed
  `__Option` construction
- focused fixtures/tests around the two benchmark shapes

Avoid broad VM compiler rewrites unless the worker chooses the alternate
statement-context lane below.

### Step 1: narrow the field-projection preflight

Do not delete the fail-closed surface wholesale. It may still be protecting
object-destructuring and unproven projected move semantics.

Smallest safe approach:

- distinguish dead assignment-expression readbacks from real object
  destructuring / projected-value reuse; and
- allow simple typed field-projection local assignments only when the projected
  place can be read through the existing JIT place machinery.

An alternate implementation is to change VM MIR lowering so
`Statement::Expression(Expr::Assign { .. })` emits only the side-effecting write
and does not materialize the assignment expression result. That likely removes
the benchmark's generated `Copy(target_place)` temps at the source, but it is a
compiler-lowering change and should be verified against assignment expressions
whose values are intentionally used.

### Step 2: add schema-backed `Some` / `None` construction for MIR JIT

Implement a JIT ABI that mirrors
`shape-vm/src/executor/result_option_carrier.rs` rather than using retired
`Arc<OptionData>` producers.

For this benchmark the required variant is `Some`, but object construction also
contains `None` initializers. The worker should handle both `Some` and `None`
for `__Option` in the same small lane.

The produced carrier must be stamped as
`NativeKind::Ptr(HeapKind::TypedObject)` and must preserve correct
`field_kinds` / `heap_mask` for the variant and payload fields.

Keep unsupported `Result` or richer enum operations fail-closed unless they are
implemented in the same schema-backed style. `EnumTest` / `EnumPayload` still
need their own proof before broad Result/Option matching is enabled.

### Step 3: route dynamic `Option<T>` field stores through a barrier-correct path

For fields with a known byte offset but no static field kind, do not call the
inline setter in a way that skips the barrier. Prefer:

- known static field kind: keep `inline_typed_field_set`;
- known byte offset with unknown/dynamic field kind: call
  `jit_typed_object_set_field`, which reads `field_kinds[idx]` and barriers
  dynamically;
- unknown byte offset: keep existing fallback/deopt behavior.

Do not globally map `FieldType::Option(_)` to
`Ptr(HeapKind::TypedObject)` unless the worker also proves that all public
`Option<T>` field carriers are always schema-backed `__Option` typed objects on
the JIT path. The current `FieldType::Option` refusal is an intentional signal
that the runtime slot kind is value-dependent.

## Verification probes for the worker/supervisor

The scout did not run these commands. They are the smallest probes to ask the
supervisor or implementation worker to run with the global cargo/build lane.

- Focused compile/native probe for
  `benchmarks/shape/17_jit_heap_field_overwrite.shape` in the default artifact:
  assert no JIT fallback and output `7500000`.
- The same heap fixture with `--no-default-features --features jit`: assert no
  fallback and output `7500000`.
- Re-run `benchmarks/shape/18_jit_scalar_field_overwrite.shape` in both
  artifacts to ensure the scalar control stays native-JIT.
- Add a focused JIT regression for multiple assignment-expression field writes,
  e.g. `a.peer = Some(b); a.peer = Some(c);`, proving the old
  `FieldProjectionAssign` blocker no longer fires for the safe pattern.
- Add a focused schema-backed `Some` / `None` JIT regression that asserts the
  carrier kind is typed-object-compatible and does not reach the retired
  `jit_v2_make_option_*` surface.
- Add a GC-enabled integration or IR/assertion probe showing `Option<T>` field
  overwrite either calls `jit_write_barrier` with a nonzero dynamic tag or goes
  through `jit_typed_object_set_field`.

## Risks

GC correctness: the main risk is allowing native field stores for
`Option<T>` while using tag `0`. That would skip the write barrier on
schema-backed `__Option` / typed-object edges and make the benchmark result
meaningless.

Ownership and move semantics: `Some(b)` must preserve the VM carrier semantics
exactly. The helper must transfer or retain payload slots consistently with
`result_option_carrier::build_some`; otherwise cycles like `b.peer = Some(b)`
can leak, double-free, or corrupt `field_kinds`.

Fallback safety: the existing `FieldProjectionAssign` preflight is broad, but
it was installed to preserve VM == JIT while projected assignments were
unproven. Narrow it to the benchmark-safe shape or to proven typed projections,
and keep unproven destructuring/projected-move surfaces deopted.

Measurement quality: routing dynamic `Option<T>` field writes through
`jit_typed_object_set_field` includes an FFI field-store cost in the hot loop.
That is still a native-JIT/default-GC barrier measurement, but it is not a pure
inline write-barrier microbenchmark. A later lane can add a proven inline
static-kind path if `Option<T>` carrier kind invariants are documented and
tested.

## Conclusion

The exact remaining blocker is the JIT MIR preflight
`FieldProjectionAssign` surface. It is triggered by generated assignment-result
field reads from property assignment lowering, not by the GC barrier itself.

The smallest safe implementation lane is:

1. narrow the field-projection preflight for safe/dead assignment readbacks or
   proven typed projections;
2. add schema-backed JIT `__Option.Some` / `__Option.None` construction; and
3. make `Option<T>` typed-object field overwrites barrier-correct by using a
   dynamic field-set path when the static field kind is unknown.

Only after all three pieces are in place will
`17_jit_heap_field_overwrite.shape` be a trustworthy native-JIT fixture for
default-GC write-barrier timing.
