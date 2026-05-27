# Cluster #3 — wire_conversion enum-discriminant panic

**HEAD:** 70507224 (audit-only; no source/fixture changes)
**Sub-cluster name:** `c3-wire-conversion-v2-raw-carrier-projection`
**Estimated size:** 11 tests (2 enums Group D wire panics + 9
type_inference `.type()` Discriminant(15)) + a likely larger cone of
silent-wrong-output (`print(Color::Red)` produces `{"Bool": false}`
at CLI — see §1.1) that the per-binary audits classified separately.

---

## 1. Minimal repros

### 1.1 Shape A — enum unit variant terminal expression (wire panic)

Test: `enums::basics_decl::test_enum_unit_variant_definition` (+
`enums::basics_programs::enum_unit_variants_declaration`).

```shape
enum Color { Red, Green, Blue }
Color::Red
```

Actual (verified `direnv exec /home/dev/dev/shape-lang cargo test -p
shape-test --test enums basics_decl::test_enum_unit_variant_definition
--no-fail-fast`):

```
thread '...' panicked at crates/shape-runtime/src/wire_conversion.rs:201:5:
assertion `left == right` failed: slot kind TypedObject does not match HeapValue::Decimal
  left: Decimal
 right: TypedObject
```

CLI run of the same program prints `{"Bool": false}` (no panic — release
debug_assert disabled) — silent-wrong-output is the lurking sibling.

### 1.2 Shape B — `.type()` static lowering (PushConst Discriminant(15))

Test family: `stress_generics::generic_runtime_type_via_type_method*` +
`stress_generics::generic_struct_type_name_with_{default,non_default}` +
`stress_inference::typeof_{int,number,array,bool,string,struct}_via_type_method` +
`stress_inference::typeof_struct_on_type_symbol` +
`stress_inference_complex::struct_type_name_via_{instance,symbol}` (9).

```shape
fn main() { let x = 42; print(x.type().to_string()) }
```

Actual:

```
Runtime error: unsupported constant variant in PushConst (Wave 6
follow-up): Discriminant(15) (line N)
```

`Discriminant(15)` = `Constant::TypeAnnotation` (16th variant in
`crates/shape-vm/src/bytecode/core_types.rs:656`, zero-indexed; verified
by counting `Int, UInt, Number, Decimal, String, Char, Bool, Null, Unit,
Function, Timeframe, Duration, TimeReference, DateTimeExpr,
DataDateTimeRef, TypeAnnotation`).

---

## 2. Root cause

**Both shapes are the same defect class: the slot-kind label is correct,
but the consumer (`slot_to_wire` / `op_push_const`) has no arm for the
producer's carrier — dispatch falls through to a fatal path.**

### 2.1 Shape A — `slot_to_wire` missing v2-raw `Ptr(HeapKind::TypedObject)` arm

Producer (`crates/shape-vm/src/executor/objects/object_creation.rs:214-221`):
`op_new_typed_object` allocates via `TypedObjectStorage::_new(...) -> *mut Self`
(raw, manually-allocated, `repr(C)` with `HeapHeader` at offset 0 carrying
`HEAP_KIND_V2_TYPED_OBJECT = 86`). It pushes `(ptr as u64, NativeKind::Ptr(HeapKind::TypedObject))`.
Enum unit variants flow through here: the compiler lowers `Color::Red` to
`PushConst(variant_id=0)` + `NewTypedObject(schema_id, field_count=1)`
(`crates/shape-vm/src/compiler/expressions/collections.rs:1305-1344`).

Consumer (`crates/shape-runtime/src/wire_conversion.rs:148-208`):
`slot_to_wire` → `heap_to_wire(bits, hk=HeapKind::TypedObject, ctx)`
takes the non-Char / non-Result / non-Option fall-through at line 198:
`let ptr = bits as *const HeapValue; let hv = unsafe { &*ptr };` — but
the bits are a `*mut TypedObjectStorage`, not an `Arc<HeapValue>`. The
first 8 bytes (`HeapHeader`) get reinterpreted as the Rust enum tag of
`HeapValue`. The `HeapValue::TypedObject(TypedObjectPtr)` arm at line
270 of `wire_conversion.rs` exists but is unreachable from this path —
it only fires when something elsewhere genuinely materializes
`Arc<HeapValue::TypedObject(...)>` and pushes its raw under
`Ptr(HeapKind::TypedObject)`.

The `debug_assert_eq!` at line 201 catches the type confusion in debug
builds; release builds proceed to a misaligned-pointer abort or, more
commonly, project the bits as the wrong `HeapValue` arm and produce
`{"Bool": false}` (the `print(Color::Red)` shape).

This is **directly analogous** to the existing carve-outs already in
`heap_to_wire` for `HeapKind::Char` (line 161 — "casting a codepoint to
a HeapValue pointer ... is a misaligned-pointer abort") and for
`HeapKind::Result` / `HeapKind::Option` (lines 167-197 — typed-`Arc<T>`
dispatch labels whose bits are NOT `Arc<HeapValue>`). The
`HeapKind::TypedObject` v2-raw carrier needs its own arm using the same
shape as the `HeapValue::TypedObject(storage)` arm at line 270 (schema
lookup → per-field `slot_to_wire`), but reading directly from `&*(bits
as *const TypedObjectStorage)` (the producer's actual carrier).

### 2.2 Shape B — `op_push_const` missing `Constant::TypeAnnotation` arm

Producer (`crates/shape-vm/src/compiler/expressions/function_calls.rs:
2503-2519`): when `.type()`'s receiver has a statically-resolved type,
the compiler emits `add_constant(Constant::TypeAnnotation(type_ann)) +
PushConst`. This is the dominant path for non-generic `.type()` calls
(int / number / string / bool / typed-struct).

Consumer (`crates/shape-vm/src/executor/stack_ops/mod.rs:91-225`):
`op_push_const` enumerates `Number / Int / UInt / Bool / Null / Unit /
Function / String / Char / Decimal / Duration / DateTimeExpr / Value`
plus a comment block at lines 212-220 explicitly acknowledging
"Remaining complex constants (Timeframe, TimeReference, DataDateTimeRef,
TypeAnnotation): these are deferred to a follow-up wave that aligns the
constant table with the kinded heap encoding". The catch-all at 221-224
errors with `Discriminant(15)`.

Result: every `.type()` call whose receiver has a statically-resolved
type runtime-crashes. The runtime-fallback `BuiltinFunction::TypeOf`
branch at line 2520-2526 is only reached when `static_type_annotation_for_expr`
returns `Ok(_) if self.should_runtime_type_query(...)` — generic-param
receivers and a narrow set; the static path is dominant and broken.

The "Wave 6 follow-up" message text matches the TAXONOMY defection-attractor
pattern ("mark as a follow-up for a later phase"). No dated user
re-disposition pulls `.type()` out of v0.3 — `.type()` is a documented
language builtin used across `stress_inference` / `stress_generics` /
`stress_inference_complex` test families.

---

## 3. Bisect anchors

`git log --oneline -- crates/shape-runtime/src/wire_conversion.rs`:
- `a26e82f5 feat(vm): W18.2 wire slot_extract_content` (most recent edit)
- `ca32cebc fix(v0.3): WS-3 — Result / AnyError` (added Result / Option arms — same gap-pattern this audit names; precedent for the fix shape)
- `aefe77e5 R5b-2-bool-null-sentinel-cluster` (added `NativeKind::Null` arm)
- `dcc01005 ... StringV2 + DecimalV2 heap-pointer variants`
- `4529c279 V3-S5 ckpt-5-prime ... HeapKind::TypedArray lockstep dispatch arm retirement` (parallel-shape retirement at the same site)

`git log --oneline -- crates/shape-vm/src/executor/objects/object_creation.rs`:
- `47b55a63 ... ckpt-final-prime² STRICT CLOSE: Path B atomic single-commit landing — TypedObjectPtr/TraitObjectPtr newtype-as-variant-payload canonical pattern`
- `0214f107 ... ckpt-1 INTERMEDIATE CLOSE: 11 TypedObjectStorage producer-site migrations Arc::new → _new`

`git log --oneline -- crates/shape-vm/src/executor/stack_ops/mod.rs`:
- `16fa2f8a feat(vm): W17-typed-module-exports-followup-constant-pool — Constant::Value kinded-pool extension` (most recent — added the `Constant::Value` arm but explicitly left `TypeAnnotation` / `Timeframe` etc. deferred per the line 212-220 comment).

**Bisect summary.** Shape A regressed at `47b55a63` (TypedObjectPtr
landing; producers flipped to `_new` raw carrier; consumer wire-conversion
arm for the new carrier was never written). Shape B is older — the
`PushConst(TypeAnnotation)` site was emitted by the compiler before the
runtime arm was written; the explicit "Wave 6 follow-up" comment dates
to `16fa2f8a` (2026-05-24).

---

## 4. Affected subsystems

- `crates/shape-runtime/src/wire_conversion.rs:148-208` —
  `heap_to_wire(bits, hk, ctx)` needs a `HeapKind::TypedObject` arm
  matching the v2-raw `*const TypedObjectStorage` carrier (mirrors the
  existing `HeapValue::TypedObject(storage)` arm body at line 270-301
  but sources `storage` from `&*(bits as *const TypedObjectStorage)`,
  not from `&*(bits as *const HeapValue)`).
- `crates/shape-vm/src/executor/stack_ops/mod.rs:91-225` —
  `op_push_const` needs a `Constant::TypeAnnotation(ta)` arm. Companion
  arms for `Timeframe / TimeReference / DataDateTimeRef` are deferred-
  with-it per the same comment block; the audit recommends the same
  shape (push a runtime payload representing the type-annotation string;
  `BuiltinFunction::TypeOf` already produces the wire shape consumers
  expect).
- (Downstream-likely, same root pattern) every v2-raw `HeapKind::*`
  whose producer is `_new`-style raw rather than `Arc<HeapValue>` —
  audit recommends a sentinel grep `Ptr(HeapKind::` arms in
  `wire_conversion.rs` vs `HeapValue::*` arms in `heap_value_to_wire`
  for symmetry. The pattern that Result / Option / Char already
  carve out is the template.

---

## 5. Dependencies

- **Sibling root-cause family with cluster #4 (pointer-as-float-leak,
  `regression::tdd::bug5_named_fn_as_argument`).** Both are "wire /
  marshal-tier dispatch reads raw slot bits as if they were a different
  carrier", but the distinct sites:
  - Cluster #4: PRODUCER mislabel — a function-pointer slot is stamped
    `NativeKind::Float64`; consumer correctly reads as `f64::from_bits`.
    Fix is upstream (kind-stamping site).
  - Cluster #3 (this audit): PRODUCER label is correct (`Ptr(HeapKind::
    TypedObject)`); CONSUMER lacks the v2-raw carrier arm and falls
    through to `*const HeapValue` reinterpretation. Fix is downstream
    (consumer dispatch arm).
- **Independent of cluster #2 (ADR-006 §2.7.13 kind-drift).** That
  cluster's assertion fires when `field_kinds[i]` drifts from the
  RefTarget-captured kind across a `DerefStore` — orthogonal site
  (`executor/variables/mod.rs:3046`).
- **Independent of SCOPE-RECLAIM (V3-S5 ckpt-5/ckpt-6 op_new_array
  construction-cascade).** SCOPE-RECLAIM territory is `op_new_array(N)`
  / `op_new_typed_array(N)` / `range` / `map` / `filter` /
  `String.split` SURFACEs — distinct sites and distinct surface shape
  (structured `Not implemented` SURFACE message, not silent panic
  or `Discriminant(15)`).
- **Cluster boundary for the `.type()` family.** The 9 type_inference
  tests classified `FN-REG-CORRECTNESS` for `.type()` in
  `type_inference.md` belong to this cluster, NOT to a separate
  `.type()` cluster. Root cause is `op_push_const` constant-table arm
  gap; no `.type()`-specific compiler bug.
