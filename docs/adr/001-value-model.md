# ADR-001: Value Model -- NanBoxed Canonical, VMValue Boundary-Only

## Status

Accepted (2026-02-19)

## Context

Shape's runtime currently maintains two parallel value representations:

1. **NanBoxed** (`shape-value/src/nanboxed.rs`): A compact 8-byte NaN-boxed
   representation using IEEE 754 quiet NaN space with 3-bit tags. Inline types
   (f64, i48, bool, None, Unit, Function, ModuleFunction, Ref) are stored in a
   single `u64`. Complex types are heap-allocated behind a tagged pointer.

2. **VMValue** (`shape-value/src/value.rs`): A Rust enum with ~25 variants
   covering every Shape type (Int, Number, Decimal, String, Bool, Array,
   TypedObject, Closure, Duration, Time, etc.). This was the original runtime
   value type before NanBoxed was introduced.

The Feb 2026 codebase audit found:

- **1345 VMValue references across 69 files** (guard script: `cargo xtask vmvalue inventory`).
- Per-crate breakdown: shape-vm 647, shape-value 376, shape-runtime 320, shape-jit 2.
- Location categories: 36 hot-path files, 22 support/legacy files, 9 test/bench files, 2 boundary files.
- Conversion between NanBoxed and VMValue (`to_vmvalue` / `from_vmvalue`) occurs at
  multiple execution boundaries: call convention, snapshots, debugger, REPL, wire
  serialization.
- The dual representation causes measurable overhead: extra allocation pressure
  from materialization, wasted branch prediction budget from dual dispatch, and
  cognitive load maintaining two type systems that must stay in sync.

The JIT subsystem has a *third* tagging scheme (`shape-jit/src/nan_boxing.rs`)
using 16-bit tags in the NaN exponent space, further fragmenting the value ABI.

Continuing without a clear decision leaves the codebase in an unstable
"half-migrated" state where neither representation can be optimized in isolation.

## Decision

### 1. NanBoxed is the canonical runtime representation

All VM stack slots, registers, function arguments/returns, and hot-path data
structures use `NanBoxed` as their native value type. No new code paths should
introduce `VMValue` in execution-critical paths.

### 2. VMValue is deprecated to boundary-only use

VMValue remains valid **only** at explicitly declared subsystem boundaries:

| Boundary           | Module/File                                    | Direction         |
|--------------------|------------------------------------------------|-------------------|
| REPL display       | `shape-vm/src/repl/`                           | NanBoxed -> display |
| Debugger/inspector | `shape-vm/src/executor/debug*.rs`              | NanBoxed -> display |
| Snapshot/serialize | `shape-runtime/src/snapshot.rs`                | NanBoxed <-> wire |
| Wire conversion    | `shape-runtime/src/wire_conversion.rs`         | NanBoxed <-> external |
| Plugin host calls  | `shape-runtime/src/plugins/`                   | NanBoxed <-> host |

New VMValue usage outside these boundary modules must be rejected by the
`cargo xtask vmvalue check` CI gate.

### 3. Introduce ExternalValue for display, wire, and debug

A new `ExternalValue` enum will be introduced (see downstream task) to replace
VMValue at boundary points. ExternalValue is designed for human/machine
consumption (serialization, pretty-printing, debug inspection) and is explicitly
**not** for runtime execution:

```rust
pub enum ExternalValue {
    Int(i64),
    Float(f64),
    Decimal(String),      // String-serialized for lossless transport
    String(Arc<String>),
    Bool(bool),
    None,
    Array(Vec<ExternalValue>),
    Object {
        type_name: String,
        fields: Vec<(String, ExternalValue)>,
    },
    Enum {
        type_name: String,
        variant: String,
        payload: Option<Box<ExternalValue>>,
    },
    Function { name: String, arity: u16 },
    Opaque(String),       // Type name only, for non-displayable values
}
```

ExternalValue has no heap pointers, no interior mutability, and no runtime
semantics. It is `Serialize + Deserialize + Clone + Debug`.

### 4. Conversion chokepoint

All NanBoxed-to-external and external-to-NanBoxed conversions will be routed
through a single `conversion` module in `shape-value`. This creates:

- One place to audit for correctness.
- One place to instrument for performance monitoring.
- A clear API boundary: `NanBoxed::to_external()` and `ExternalValue::to_nanboxed()`.

### 5. Migration is incremental

VMValue will not be deleted in a single pass. The migration follows this order:

1. Define ExternalValue type and conversion chokepoint. (Phase 0)
2. Convert boundary modules one at a time (snapshot, wire, debugger, REPL). (Phase 1)
3. Convert call_convention and ExecutionResult to NanBoxed-native. (Phase 2)
4. Measure remaining VMValue references; target zero in non-test code. (Phase 3)
5. Delete VMValue. (Phase 4)

Each phase is independently shippable. The `cargo xtask vmvalue` CI script tracks
spread and the `check-trend` subcommand (to be added) will enforce monotonic
decrease.

## Consequences

### Positive

- **Single hot-path representation**: eliminates conversion overhead in call
  convention, stack operations, and function dispatch.
- **Smaller stack frames**: 8 bytes per slot (NanBoxed) vs 40+ bytes (VMValue enum).
- **JIT ABI alignment**: once the JIT adopts the same 3-bit tag scheme
  (ADR-002), VM and JIT share a single value ABI with zero-cost transitions.
- **Clear boundary contract**: ExternalValue makes the display/wire API explicit
  and auditable, rather than leaking runtime internals.
- **Measurable progress**: the vmvalue_guard gives CI-enforced monotonic
  reduction with per-crate and per-category granularity.

### Negative

- **Migration cost**: 1345 existing references must be triaged and converted
  incrementally, requiring coordination across VM, runtime, and JIT crates.
- **Temporary dual API**: during migration, some modules will accept both
  NanBoxed and VMValue, increasing surface area until conversion completes.
- **ExternalValue is a new type**: adds a third value enum (temporarily), though
  it is strictly non-runtime and has no execution semantics.

### Risks

- **Incomplete migration stall**: if migration velocity drops, the codebase
  could remain in a three-representation state. Mitigation: CI trend enforcement
  and per-sprint reference count targets.
- **Boundary misclassification**: a module incorrectly classified as "boundary"
  could retain VMValue in a hot path. Mitigation: the allowlist is
  category-annotated and reviewable in `vmvalue_allowlist_categories.tsv`.

## References

- Audit: `shape/docs/audits/2026-02-19-shape-state/06-vmvalue-gc-status.md`
- Audit: `shape/docs/audits/2026-02-19-shape-state/05-performance-bottlenecks.md`
- Guard script: `cargo xtask vmvalue`
- NanBoxed source: `shape/shape-value/src/nanboxed.rs`
- VMValue source: `shape/shape-value/src/value.rs`
- JIT NaN-boxing: `shape/shape-jit/src/nan_boxing.rs`
- Heap object plan: `shape/docs/audits/2026-02-19-shape-state/07-heap-backed-types-jit-plan.md`
