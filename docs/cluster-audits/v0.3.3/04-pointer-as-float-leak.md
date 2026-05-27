# Cluster #4 — pointer-as-float / pointer-as-int silent-wrong-output

**HEAD:** workspace tip (post-`70507224`).
**Audit scope:** read-only root-cause classification. No source/fixture changes.
**Discipline binding:** post-strict-typing zero-tag-runtime invariant is BINDING
(CLAUDE.md §Forbidden Patterns + `docs/runtime-v2-spec.md`). Any "reintroduce
tag-bits / `ValueWord` / `synthesize_value_word_from_raw`" framing is refused
on sight. This is a kind-stamping bug (the kind is fabricated wrong at a
known site), not a kind-decoding bug (which would require the deleted
dynamic dispatch path).

---

## Cluster sources (3 tests, all FN-REG-CORRECTNESS, all silent-wrong-output)

### 1. `regression::tdd::bug5_named_fn_as_argument`

**Repro** (`tools/shape-test/tests/regression/tdd.rs:55`):
```shape
fn double(x) { x * 2 }
fn apply(f, x) { f(x) }
apply(double, 21)
```
Expected: `42`. Actual (workspace HEAD):
`0.000…208` = `f64::from_bits(208)` denormal — **the raw fn-id of
`double` materialized as `WireValue::Number`.**

### 2. `regression::jit::jit_trampoline_result_callvalue`

**Repro** (`tools/shape-test/tests/regression/jit.rs:662`):
```shape
fn make_ok() -> Result<int, string> { return Ok(42) }
fn call_it(f) -> int { let val = f()?; return val }
call_it(make_ok)
```
Expected: `Number(42)`. Actual:
`Integer(137900062693984)` — pointer-shaped (~140 trillion). The
**string twin (`jit_trampoline_string_callvalue:682`) PASSES** via the
`executor.rs:267` `RETURN_TAG_NANBOXED` SURFACE-and-deopt to the
interpreter. The Result-twin silently stamps `RETURN_TAG_I64` on the
pointer bits — no SURFACE fires, no fallback runs.

### 3. `regression::qa::regression_crit_1_nested_property_access`

**Repro** (`tools/shape-test/tests/regression/qa.rs:124`):
```shape
type Server { host: string, port: int }
type Config { server: Server, debug: bool }
let cfg = Config { server: Server { host: "localhost", port: 8080 }, debug: false }
print(cfg.server.host)
```
Expected: prints `localhost`. Actual:
`memory allocation of 135242086536256 bytes failed` + SIGABRT — the
nested-`TypedObject` field read leaks a `*const TypedObjectStorage` into
a `usize`-shaped allocator-length consumer. The 3-level sibling
`regression_crit_1_deep_nested_access` is `#[should_panic]` and passes by
panicking `Expected 42, got 123976115954656` — same pointer-as-i64 shape,
non-SIGABRT path.

---

## Root cause hypothesis (per-test, named-and-cited)

This is **not** a NaN-box residual in the runtime — the §2.7 model has
no NaN-box. It is **three distinct kind-stamping defects** at three
producer sites. The pointer-as-float test is *purely VM-side* (bug5
runs through `BytecodeExecutor`, not JIT).

### Defect A — VM op_return_value typed variants drop source kind

`crates/shape-vm/src/executor/control_flow/mod.rs:827-868`
(`op_return_value_i64` / `_u64` / `_f64` / `_i32` / …) all discard the
popped `_src_kind` and re-stamp the value with the opcode-suffix kind:

```
let (bits, _src_kind) = self.pop_kinded()?;
self.return_value_inner(bits, shape_value::NativeKind::Float64)
```

Combined with the producer-side decision in
`crates/shape-vm/src/compiler/helpers_binding.rs:369::emit_return_value_with_ownership`,
which picks the `ReturnValue<Kind>` opcode from
`last_expr_numeric_type` rather than the proven runtime kind of the
return slot, this stamps a UInt64 fn-id (or Ptr bits) as Float64 / I64
whenever the inferred numeric type of the return expression disagrees
with the value-call's actual produced kind. **Bug5 hits this.**
`Constant::Function(id)` push at
`crates/shape-vm/src/executor/stack_ops/mod.rs:123-126` deposits
`(fn_id as u64, NativeKind::UInt64)`. `apply(f, x) { f(x) }` returns the
call-value result; the inferred numeric type of `apply(...)` flows to
the typed-return helper. If the gate at `helpers_binding.rs:382-403`
accepts `Float64`/`Int64`, the bits (which are actually the call's
return — but in some lowering paths reach the typed-return at the
top-level shell as the *callee fn-id slot still on the stack*) get
re-stamped under the typed-return opcode kind. The 208 denormal is
the bare fn-id of `double` projected by `slot_to_wire`'s `Float64` arm
(`crates/shape-runtime/src/wire_conversion.rs:46`).

### Defect B — JIT terminators.rs Return stamp falls through I64 → `RETURN_TAG_I64` for non-int Ptr return kinds

`crates/shape-jit/src/mir_compiler/terminators.rs:1796-1824`
(`TerminatorKind::Return`, I64-wide arm). When Cranelift's return value
is `types::I64` and the slot kind is heap-bearing (e.g. `Ptr(_)` /
`StringV2` / `String`), the `raw_int` predicate is `false` AND
`return_kind.is_some()`, so the tag stamped is `RETURN_TAG_NANBOXED`
(the surface-and-stop path). **The string twin
`jit_trampoline_string_callvalue` correctly takes this path** and
deopts to the interpreter via `executor.rs:802-812`.

**The Result-twin (`jit_trampoline_result_callvalue`) breaks because
the value-call return passes through `dispatch_call_via_trampoline_vm`
(`ffi/control/mod.rs:830`) whose returned `u64` then flows into the
caller's MIR slot.** When the callee returns an `int` via `Result<int,
string>` `?` unwrap, the bytecode return slot's `NativeKind` (per
W11-jit-new-array stamp at `terminators.rs:1797-1812`) ends up as
`NativeKind::Int64` from the bytecode compiler's view (the bytecode
return slot's static type), so `raw_int = true` → stamp
`RETURN_TAG_I64`. But the value actually living in `stack[0]` at
return time is the trampoline's `u64` which is a NaN-boxed
`Arc<HeapValue::ResultData>` pointer (the trampoline-VM-to-JIT
result-conversion gap the test's docstring names). Result:
`WireValue::Integer(137_900_062_693_984)` — pointer bits stamped as i64.

The Result conversion that should run lives on the JIT-trampoline
boundary (a sibling to `arc_string_retain` and the `format_kinded`
String carrier-shape unification at
`crates/shape-jit/src/ffi/conversion.rs:928`). The string twin's deopt
masks the missing conversion; the Result twin has no equivalent SURFACE
because `RETURN_TAG_I64` is a typed variant — the executor accepts it
silently.

### Defect C — `op_get_field_typed` field-kind drift for nested TypedObject

`crates/shape-vm/src/executor/typed_object_ops.rs:338-471`
(`op_get_field_typed`) reads `field_type_tag` from the operand,
dispatches through `push_field_value` (`typed_object_ops.rs:237-283`).
For heap-backed fields, `field_tag_to_heap_native_kind(field_type_tag)`
maps the operand tag → `NativeKind`. The operand tag for the OUTER
field (`Config.server`) is `FIELD_TAG_TYPED_OBJECT` → kind
`Ptr(HeapKind::TypedObject)` — correct.

The bug surfaces on the SECOND field-read (`.host`): the inner
TypedObject (`Server`) is now on the stack as
`(bits=*const TypedObjectStorage, kind=Ptr(TypedObject))`. The next
`op_get_field_typed` for `.host` reads the receiver via the
`ReceiverGuard` raw-pointer cast at
`typed_object_ops.rs:436-437` — sound for the kind. But the
`field_type_tag` operand encoded at compile time for the OUTER schema
points at `Server.host` with tag `FIELD_TAG_STRING` and the field-kind
mapping returns `NativeKind::String` (Arc-wrapped). The pushed bits are
the slot's `raw()` (`typed_object_ops.rs:280`) WITHOUT validating that
the slot's stored carrier matches `String` vs `StringV2` vs the
`*const StringObj` v2-raw variant. If `cfg.server` was constructed via
the v2-raw `TypedObjectStorage::_new` path with a `StringObj`-shaped
inner field but the schema tag reports `FIELD_TAG_STRING` (Arc<String>),
the slot bits are pushed as `NativeKind::String` and `clone_with_kind`
runs `Arc::increment_strong_count::<String>` against a `StringObj`
header — reading the length field of `StringObj` as the `ArcInner`
`strong` counter, then a downstream consumer (`print(...)`) reads the
opposite field as a length → 134 TB allocation request → SIGABRT.

The 3-level twin (`deep_nested_access`) hits the same shape at a
different depth, surfacing as `Expected 42, got 123976115954656`
(pointer bits printed via the integer fallback because the third level
is `int` not `string`).

**This is a v2-raw / Arc-wrapped carrier disambiguation gap, sister-class
to `c2825f93` "fix(vm): disambiguate u64-scalar from v2-typed-array
carrier (CKPT-C)".** Not a NaN-box decode — there are no tag bits to
decode. The kind is *wrong at the producer*.

---

## Bisect anchors

`git log --oneline -- crates/shape-jit/src/ffi/ crates/shape-vm/src/executor/typed_object_ops.rs`:

- **`abec57d0`** (most recent) — `feat(vm): W17.3-4.3 runtime dispatch +
  snapshot/wire for per-container FieldType variants (criterion B close)`
  — extends FieldType variants; touches `field_tag_to_heap_native_kind`.
- **`1aadf767`** — `refactor(jit): W16.2-J.3 macro-generate per-kind FFI
  symbols for 14 TypedArrayKind variants` — JIT-FFI per-kind rebuild.
- **`a4b38c76`** — `feat(type-schema): W17.3-4.1 per-container FieldType
  variants for HashMap + Set` — schema-side per-container rebuild.
- **`d929148e` / `c2825f93`** — `fix(vm): disambiguate u64-scalar from
  v2-typed-array carrier (CKPT-C)` — the **named sister-class fix** for
  Defect C; same shape (carrier-disambiguation gap), different op.
- **`aefe77e5`** — R5b-2-bool-null-sentinel-cluster — added
  `NativeKind::Null` because the §2.7.7/Q9 "kind IS the discriminator"
  invariant was being violated by Bool sentinel collision. Defects A
  and B are the same invariant-violation family at different sites
  (typed-return stamp / I64 fall-through stamp).

`git log --oneline -- crates/shape-vm/src/executor/stack_ops/mod.rs
crates/shape-vm/src/executor/call_convention.rs
crates/shape-vm/src/executor/control_flow/mod.rs
crates/shape-vm/src/compiler/helpers_binding.rs`:

- **`16fa2f8a`** — `feat(vm): W17-typed-module-exports-followup-constant-
  pool — Constant::Value kinded-pool extension for TableView slot
  injection` — most recent stack_ops touch.
- **`17117577`** — `Phase 3 cluster-1.5 v2-raw-empirical-isolation-and-
  fix: share-accounting double-release at closure-args / kinded-captures
  stack-write boundary` — relevant for Defect B (trampoline boundary).
- **`8b95290c`** — `W17-vm-call-value-closure-kind-mismatch: producer-
  side share-accounting fix at call_value_immediate_nb Closure arm` —
  named precedent for producer-side kind fixes.

No single commit is "the regression": the three defects are the
residual shape of incomplete carrier-disambiguation rebuilds (W16.2-J /
W17.3-4.x / W17-vm-call-value-closure) at three different surfaces that
share one root invariant.

---

## Affected subsystem

1. **Defect A** — VM bytecode-compiler typed-return picker
   (`compiler/helpers_binding.rs::emit_return_value_with_ownership`) +
   VM typed-`op_return_value_*` re-stamp
   (`executor/control_flow/mod.rs:827-872`). Producer-stamp + consumer-
   re-stamp pair both ignore the value-call's actual produced kind.
2. **Defect B** — JIT FFI return-tag handling
   (`mir_compiler/terminators.rs:1715-1830` + `executor.rs:740-814`).
   Specifically the `dispatch_call_via_trampoline_vm` boundary
   (`ffi/control/mod.rs:830`) returns raw `u64` without a kind
   companion → caller's slot kind is the static `NativeKind::Int64`
   stamped at compile time → pointer bits stamped `RETURN_TAG_I64`.
3. **Defect C** — VM `op_get_field_typed`
   (`executor/typed_object_ops.rs:338-471`) +
   `push_field_value`/`push_field_value_with_kind` (lines 237-317) +
   the `field_tag_to_heap_native_kind` mapping for String /
   TypedObject. v2-raw carrier (`StringObj` / `TypedObjectStorage`
   from `_new`) vs Arc-wrapped carrier (`Arc<String>` /
   `Arc<TypedObjectStorage>`) disambiguation gap on the field-read
   side, sister to `c2825f93`'s typed-array-side fix.

---

## Sub-cluster names + sizes

| Sub-cluster | Sites | Tests | Audit-class |
|---|---|---|---|
| 4A — VM typed-return kind re-stamp drift | `control_flow/mod.rs:827-872` + `helpers_binding.rs:369` + `stack_ops/mod.rs:123` | 1 (`tdd::bug5_named_fn_as_argument`) | FN-REG-CORRECTNESS |
| 4B — JIT trampoline-VM result carrier conversion missing for Result/Option/heap returns | `ffi/control/mod.rs:830` + `mir_compiler/terminators.rs:1814-1820` | 1 (`jit::jit_trampoline_result_callvalue`); string twin passes via existing deopt | FN-REG-CORRECTNESS |
| 4C — VM nested TypedObject field-read carrier-disambiguation | `typed_object_ops.rs:237-317` + `field_tag_to_heap_native_kind` mapping | 2 (`qa::regression_crit_1_nested_property_access` SIGABRT + `regression_crit_1_deep_nested_access` should_panic-passes) | FN-REG-CORRECTNESS, release-blocking (SIGABRT) |

**Total tests in cluster #4: 3 named + 1 `#[should_panic]`-sibling = 4.**

---

## Dependencies

- **Overlap with cluster #1 (SIGABRT family)** — **YES, partial.**
  Defect C drives the `qa::regression_crit_1_nested_property_access`
  SIGABRT, which cluster #1 also catalogs. Cluster #1 covers the
  process-killing surface; cluster #4 owns the root-cause carrier-
  disambiguation gap. The fix for 4C retires the cluster #1 entry
  for this test (and the `deep_nested_access` sibling). Cluster #1's
  other SIGABRTs (e.g. `executor/variables/mod.rs:3046` kind-drift
  assertion at `bug10_nested_field_mutation`) are a **sibling defect at
  the field-WRITE side**, same family (`RefTarget` captured kind
  drift vs `field_kinds` track) — distinct site, same v2-raw vs
  Arc-wrapped carrier-disambiguation root invariant.

- **Overlap with cluster #3 (wire_conversion)** — **NO, but adjacent.**
  `wire_conversion.rs::slot_to_wire` is the *consumer* that materializes
  the silent-wrong-output (`Float64 → WireValue::Number(208)`,
  `Int64 → WireValue::Integer(pointer-bits)`). The wire conversion code
  itself is correct: it does what the kind tells it to do. The defect
  is upstream — the kind is fabricated wrong at the producer (Defects
  A, B, C). Cluster #3 should NOT touch `slot_to_wire`'s Float64 / Int64
  arms; the fix lives at the producer sites named above.

- **Overlap with cluster #2 (annotation/V3-S5 ckpt-5/op_new_array
  SURFACE)** — **NO.** Cluster #2 is SCOPE-RECLAIM (V3-S5 ckpt-6 close
  is the binding remediation per 2026-05-18 user pull-in). Cluster #4
  is FN-REG-CORRECTNESS — baseline language correctness regressions in
  call-value lowering and nested-field read. Distinct root causes,
  distinct fix paths.

- **Sister-class precedent (NAMED, not a defection)** — `c2825f93`
  "fix(vm): disambiguate u64-scalar from v2-typed-array carrier
  (CKPT-C)" + `aefe77e5` "R5b-2-bool-null-sentinel-cluster fix" are the
  binding template for the fix shape: producer-side kind stamp +
  consumer-side `kind IS the discriminator` enforcement, no Bool-default
  fallback, no transitional shim. The §2.7.7/Q9 invariant binding from
  `aefe77e5` applies verbatim to all three defects.

---

## Defection-attractor framings refused on sight

Per CLAUDE.md §Forbidden Patterns + the broader-family regex:

- "Decode the tag bits at the JIT trampoline boundary" — refused.
  No tag bits exist; the kind is missing because the trampoline returns
  raw `u64`.
- "Add a one-shot Convert{Ptr}To{Int} opcode" — refused (W4-δ pattern).
- "Keep `ValueWord` for the trampoline FFI return path" — refused.
- "Per-field-type unwrap-and-flatten at the field-read" — refused
  (cluster-0 parallel-impl pattern; cluster-close target audit doc
  `phase-3-cluster-0-status.md` is the binding remediation).
- "Mark the Result-trampoline conversion as v0.4 follow-up" — would
  require dated user disposition per TAXONOMY rules; none exists.
  Routes to FN-REG-CORRECTNESS.

The remediation shape is producer-side kind correctness at each of
the three named sites. The kind track is the discriminator; the bits
follow.
