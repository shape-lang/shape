# structs_types classification

**HEAD:** 82f049dd
**Total tests in binary:** 278
**Passed:** 229 / Failed: 48 / SIGABRT-OOM: 1 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test structs_types --no-fail-fast 2>&1`

> **Run-binding note.** The full parallel `cargo test` invocation crashes
> with `memory allocation of ~130 TB failed` → `signal: 6, SIGABRT` before
> the `test result:` line is printed (both observed runs). To capture
> per-test status + panic text I re-ran failing + indeterminate subsets
> with `--test-threads=1` (3 invocations: 43 known-failures, 30
> indeterminate, 1 isolation of `struct_nested_string_field`). The OOM
> classification (`structs::struct_nested_string_field`) was reproduced
> in isolation (single-test, single-threaded run aborted with the same
> ~130 TB allocation request). Final tally: 229 ok + 48 FAILED + 1
> SIGABRT-OOM = 278.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 3 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 46 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

Per the TAXONOMY 2026-05-22 dated user disposition ("W17.3-4 per-container
FieldType" pulled into v0.3), and the user's clarification at audit start
("TypedObject construction + W17.3-4 per-container FieldType are
SCOPE-RECLAIM triggers; the filter-on-User repro family extends to
construction failures here likely → FN-REG-CORRECTNESS"), the SURFACE
families observed below route as follows:

- **V3-S5 ckpt-3/ckpt-5 construction-cascade SURFACEs** (`op_new_array`,
  `range`) → SCOPE-RECLAIM under the 2026-05-18 user pull-in row.
- **`WrapTypeAnnotation` deleted-ValueWord SURFACE** → SCOPE-RECLAIM
  under the 2026-05-22 W17.3-4 per-container FieldType row + Q8 kinded
  redesign.
- **`MakeFieldRef base must reference a TypedObject; got Int64`,
  `expected object/array/string/... got scalar`, `TypedMergeObject got
  non-TypedObject kind`, `FieldType::Any` schema-error** — ALL on
  anonymous-object / typed-object construction fixtures → SCOPE-RECLAIM
  under the 2026-05-22 W17.3-4 per-container FieldType pull-in. These
  are the construction-cascade consumer breakage downstream of
  per-container FieldType monomorphization being incomplete; the work
  to land them is named in the 2026-05-22 W17.3-4 disposition.
- **`Cannot infer types for binary operation` on anon-object / typed
  field access** — same construction-cascade family: the typed-object
  schema for the anon-object literal is missing per-field FieldType
  metadata, which propagates to type-inference giving `unknown` for
  later field-arithmetic expressions. Route to SCOPE-RECLAIM under
  the 2026-05-22 W17.3-4 row.
- **`Undefined variable: Percent` / `unsupported constant variant in
  PushConst ... Discriminant(15)`** — comptime-field static access on
  generic-type symbols. The comptime-trait/comptime-field landing was
  pulled into v0.3 by the 2026-05-22 "Comptime trait into v0.3" row.
  SCOPE-RECLAIM.
- **FN-REG-CORRECTNESS (3 tests)**: (a) `struct_nested_string_field` —
  SIGABRT with ~130 TB allocation request on nested struct construction
  with a `string` field (real runtime crash, observed at HEAD); (b–c)
  `struct_field_mutation` + `struct_field_mutation_second_field` — VM
  hard assertion `DerefStore kind drift: popped Int64, place Float64 —
  ADR-006 §2.7.13 invariant violated` on `let mut p = Point { x: 1, y: 2
  }; p.x = 10` (where Point.x is `number` and literal is `int` — the
  compiler should widen, or the executor should reject at compile time,
  not assertion-panic at runtime). Both are runtime panics on
  plausibly-correct user code that any reasonable user would expect to
  work.

## Per-test classification

### structs::struct_nested_string_field

Class: **FN-REG-CORRECTNESS**

```
test structs::struct_nested_string_field ... memory allocation of
136617682127488 bytes failed
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
process didn't exit successfully: `.../structs_types-541995e4533e94c7
--test-threads=1 'structs::struct_nested_string_field'`
(signal: 6, SIGABRT: process abort signal)
```

Fixture (`tools/shape-test/tests/structs_types/structs.rs:113`):
```shape
type Server { host: string, port: int }
type Config { server: Server, debug: bool }
let cfg = Config { server: Server { host: "localhost", port: 8080 }, debug: false }
cfg.server.host
```

- **Minimal repro:** as above — 4 lines, nested-struct construction with
  a `string` inner field, then `.server.host` access.
- **Bisected regression commit:** not bisected during this audit (audit-
  only). Pre-existing source-comment header above the test reads `// BUG:
  nested typed struct field access (cfg.server.host) returns the inner
  object instead of the field` (so the in-source comment confirms the
  underlying bug has been latent — the OOM is a current manifestation).
- **Affected compiler subsystem:** nested TypedObject construction +
  field-path resolution. The peer fixtures `struct_nested_two_levels`
  (Point inner, no string) and `struct_nested_three_levels` (3-level int)
  use `.expect_run_ok()` and pass at HEAD; the string-field variant is
  the one that OOMs.

### structs::struct_field_mutation

Class: **FN-REG-CORRECTNESS**

```
thread 'structs::struct_field_mutation' panicked at
crates/shape-vm/src/executor/variables/mod.rs:2718:9:
assertion `left == right` failed: DerefStore kind drift:
popped Int64, place Float64 — ADR-006 §2.7.13 invariant violated
  left: Int64
 right: Float64
```

Fixture (`structs.rs:127`):
```shape
type Point { x: number, y: number }
let mut p = Point { x: 1, y: 2 }
p.x = 10
p.x
```

- **Minimal repro:** as above (4 lines). `Point.x: number`; constructor
  arg `1` is `int`; mutation `p.x = 10` pushes `Int64`; runtime asserts
  on the kind mismatch.
- **Bisected regression commit:** not bisected (audit-only). The
  assertion text names ADR-006 §2.7.13 (DerefStore kind invariant), so
  the source of truth is `crates/shape-vm/src/executor/variables/mod.rs:2718`
  + the constructor-side widening logic that should have widened the
  `1` literal to `Float64` at construction time.
- **Affected subsystem:** TypedObject mutation kind-widening at
  construction OR DerefStore opcode emission; either the constructor
  should have widened (so the slot is `Float64` post-construct) or the
  compiler should reject the `int`-literal-to-`number`-field assignment
  at compile time. Runtime assertion-panic is wrong: should be either
  silently-correct or compile-error.

### structs::struct_field_mutation_second_field

Class: **FN-REG-CORRECTNESS**

```
thread 'structs::struct_field_mutation_second_field' panicked at
crates/shape-vm/src/executor/variables/mod.rs:2718:9:
assertion `left == right` failed: DerefStore kind drift:
popped Int64, place Float64 — ADR-006 §2.7.13 invariant violated
```

Fixture (`structs.rs:140`):
```shape
type Point { x: number, y: number }
let mut p = Point { x: 1, y: 2 }
p.y = 99
p.y
```

- **Minimal repro / bisect / subsystem:** identical to
  `struct_field_mutation` above; second-field variant (`p.y = 99`).
  Same ADR-006 §2.7.13 invariant violation; same root cause; same fix
  surface.

### stress_nested::anon_object_in_for_loop

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-
cascade tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1
(2026-05-15) per W12-typed-array-data-deletion audit §3.5 + ADR-006
§2.7.24 Q25.A SUPERSEDED. ... UNREACHABLE until ckpt-6 STRICT close.
REFUSED ON SIGHT: TypedArrayData resurrection under any rename
(Refusal #1, W12 audit §7). (line 4)
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 construction-
  cascade. The `range()` builtin is part of the ckpt-3 tier-2 surface
  cascade-blocked on the same workstream.
- **SURFACE text:** `range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2
  surface ... UNREACHABLE until ckpt-6 STRICT close`.
- **(Incorrect) v0.4 anchor cited:** none — SURFACE cites in-v0.3
  `ckpt-6 STRICT close`, not v0.4.
- **Why cite routes to SCOPE-RECLAIM:** ckpt-6 is in v0.3 scope per the
  2026-05-18 pull-in. No dated re-disposition to v0.4 exists.
- **Test asserts on:** user-facing semantics (anon-object created in
  `for i in range(4) { ... }` body, mutated total). Stays the same
  after fix.

### stress_nested::struct_in_for_loop_body

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-
cascade tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1 ...
UNREACHABLE until ckpt-6 STRICT close. (line 5)
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 construction-
  cascade (same `range()` ckpt-3 SURFACE as the previous entry).
- **SURFACE text + v0.4 cite:** as above (no v0.4 cite; in-v0.3 ckpt-6).
- **Test asserts on:** user-facing semantics (`Point` constructed in
  `for i in range(3) { ... }` body, sum field accumulated).

### stress_nested::struct_with_empty_array_field

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. The deleted typed-array-data enum +
`Buf<T>` / aligned-typed-buf wrapper layer + outer
`HeapValue::TypedArray(Arc<_>)` arm + `HeapKind::TypedArray=8` ordinal
DELETED across V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-deletion
audit §3.5 + §3.6 + ADR-006 §2.7.24 Q25.A SUPERSEDED. ... Construction-
site rebuild lands at ckpt-6 STRICT close ... REFUSED ON SIGHT:
TypedArrayData resurrection under any rename (Refusal #1). (line 4)
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array
  construction-cascade — empty-literal `[]` is W16.2-C scope, named in
  the row.
- **SURFACE text:** `op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-
  cascade tier 3 surface ... Construction-site rebuild lands at ckpt-6
  STRICT close`.
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (empty-array struct field
  `items: Array<int> = []` length read).

### stress_methods::typed_merge_decomposition

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: SURFACE: WrapTypeAnnotation depends on
the deleted ValueWord wrapper type. Annotation wrapping needs a kinded
redesign (ADR-006 §2.7.6 / Q8) — see playbook §8 cross-cluster cascade.
D-objects-mod scope does not include the compiler emit site. (line 8)
```

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType +
  the parallel Q8 kinded-API binding (ADR-006 §2.7.6) — the kinded
  redesign named in the SURFACE itself.
- **SURFACE text:** `WrapTypeAnnotation depends on the deleted
  ValueWord wrapper type. Annotation wrapping needs a kinded redesign
  (ADR-006 §2.7.6 / Q8)`.
- **(Incorrect) v0.4 anchor cited:** none — SURFACE cites cross-cluster
  playbook §8 (v0.3 in-flight Phase 2d work), not v0.4.
- **Test asserts on:** user-facing semantics (intersection-type
  decomposition + per-component field-sum).

### stress_fields::spread_typed_object_extra_field

Class: **SCOPE-RECLAIM**

```
Semantic error: [E0900] post-inference FieldType::Any in user-facing
schema `__merged_44_45` at field `z` (resolved type: any). User-
introduced FieldType::Any outside the named-exception classes is the
schema-side analogue of the deleted dynamic-slot-kind variants per
CLAUDE.md Forbidden Patterns (strict-typing plan). See ADR-006 §2.7.5
+ §2.7.26 + audit §5 for the binding rule.
```

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType
  (per-field FieldType on the merged-schema synthesizer is the named
  scope).
- **SURFACE text:** as above — `FieldType::Any` in merged-schema is
  precisely what W17.3-4 per-container FieldType eliminates.
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (`{ ...p, z: 15.0 }`
  spread + extra field access).

### stress_fields::object_merge_basic + stress_fields::object_merge_preserves_all_fields

Class: **SCOPE-RECLAIM** (both)

```
Runtime error: TypeError: expected two TypedObject operands for
TypedMergeObject, got non-TypedObject kind (line 5)
```

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType.
  Anon-object literal `let a = { x: 1, y: 2 }` is not constructed as a
  TypedObject at present (no per-container FieldType metadata) → the
  `+` merge opcode fails the receiver-kind check. W17.3-4 lands the
  per-container FieldType that gives anon-object literals proper
  TypedObject construction.
- **SURFACE text:** as above (TypedMergeObject receiver-kind check).
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (`{ x: 1, y: 2 } + { z: 3 }`
  merge, field access).

### stress_fields::anon_object_single_field + anon_object_multiple_fields + stress_methods::anon_object_field_access_diff + anon_object_five_fields + anon_object_single_int_field

Class: **SCOPE-RECLAIM** (5 tests)

```
Runtime error: MakeFieldRef base must reference a TypedObject;
got Int64 (line 4)
```

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType.
  Same construction-cascade family: anon-object literals don't become
  TypedObjects in current HEAD, so `obj.x` field-ref opcode fails. The
  W17.3-4 work lands per-container FieldType that gives anon-object
  literals the schema needed to construct as TypedObject.
- **SURFACE text:** `MakeFieldRef base must reference a TypedObject;
  got Int64`.
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (anon-object field access).

### stress_methods::anon_nested_two_levels + anon_object_bool_field + stress_nested::deep_nesting_anon_three_levels + nested_anon_objects

Class: **SCOPE-RECLAIM** (4 tests)

```
Runtime error: TypeError: expected object, array, string, or other
heap value, got scalar (line 4)
```

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType.
  Same anon-object-not-TypedObject construction issue surfacing as a
  heap-value-vs-scalar dispatch failure for nested anon-object field
  paths.
- **SURFACE text:** as above (heap-value-vs-scalar dispatch).
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (nested anon-object field
  access).

### stress_fields::anon_object_field_mutation_string

Class: **SCOPE-RECLAIM**

```
Semantic error: Assignment to 'obj.name' requires compile-time field
resolution. Generic runtime property lookup is disabled.
```

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType.
  Same family: without per-container FieldType the anon-object schema
  doesn't know `obj.name` is a `string` field at compile time → the
  field-resolution-required diagnostic fires. W17.3-4 fixes the
  schema-construction path.
- **SURFACE text:** `Assignment to 'obj.name' requires compile-time
  field resolution`.
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (anon-object string field
  mutation).

### stress_fields::destructure_anon_object_param + destructure_nested_object_param + lambda_destructure_object + stress_methods::closure_captures_struct + stress_methods::anon_object_multiple_string_fields + stress_methods::anon_object_with_expression_values + stress_nested::map_over_array_of_structs + reduce_array_of_structs + struct_field_in_loop + complex::complex_struct_with_function_pipeline + structs::object_in_array + structs::struct_in_array + structs::struct_returned_from_function + traits_extend::extend_multiple_methods

Class: **SCOPE-RECLAIM** (14 tests)

```
Semantic error: Cannot infer types for binary operation `Add`: operand
types are `unknown` and `unknown`. Strict typing requires both
operands to have a known concrete type at compile time. Add a type
annotation to disambiguate.
```

(Variant `anon_object_multiple_string_fields` has `unknown` and
`string`; same root cause — one operand still resolves to `unknown`.)
(`stress_methods::anon_object_with_expression_values` panicked at
`Expected number, got: String("Null")` — same root: unresolved-type
propagation through anon-object construction returns `Null`.)

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType.
  Same construction-cascade root: anon-object literal / nested-struct-
  field-path types don't resolve through compile-time inference
  because the per-container FieldType metadata is incomplete. W17.3-4
  lands the metadata.
- **SURFACE text:** `Cannot infer types for binary operation` (with
  varying operand pairs depending on the fixture).
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (object/struct field
  arithmetic, destructure-then-add, closure-capture-then-add,
  reduce-with-accumulator). All stay the same after W17.3-4 fix.

### stress_nested::build_array_of_structs_in_loop

Class: **SCOPE-RECLAIM**

```
Semantic error: empty array `arr` has an un-resolvable element type. It
is created empty (`[]`) with no `Array<T>` annotation and is never
pushed to, so the compiler cannot prove what element type it holds.
Strict typing requires a known concrete element type: add an
annotation (`let arr: Array<T> = []`) or remove the unused binding.
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5 W16.2-C empty-literal/
  spread/comprehension construction-cascade. The fixture is
  `let mut arr = []` then `arr = arr.concat([Wrapper { val: i }])` in
  a loop; the inference should propagate the `Wrapper` element type
  back from the concat call sites, but currently does not (W16.2-C
  scope).
- **SURFACE text:** as above (empty-array element-type unresolvable).
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (build array of
  Wrapper structs in a loop, index access).

### stress_nested::filter_array_of_structs

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Greater`:
operand types are `unknown` and `int`. Strict typing requires both
operands to have a known concrete type at compile time.
```

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType.
  Same construction-cascade family — closure `|i| i.value > 2` on
  `Array<Item>` doesn't propagate the Item.value field type to the
  closure param.
- **SURFACE text:** `Cannot infer types for binary operation Greater`.
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (filter-then-length on
  Array of structs).

### stress_nested::iterate_array_of_structs + complex::complex_array_of_structs_sum + complex::complex_multi_type_program_with_loop_and_trait + structs::object_in_for_loop + traits_extend::trait_impl_method_with_param

Class: **SCOPE-RECLAIM** (5 tests)

```
Runtime error: no method 'add' on receiver kind Int64 (line 7)
Runtime error: no method 'add' on receiver kind Float64 (line 7)
Runtime error: no method 'passed' on receiver kind Ptr(TypedObject)
```

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType +
  the related 2026-05-22 phase-2c host-tier marshal/snapshot rebuild
  scope. Method-dispatch on TypedObject / scalar receiver-kind for
  field-arithmetic methods (`p.x.add(p.y)` style) is the same
  construction-cascade consumer issue: the receiver kind is wrong
  because the source value didn't construct as a TypedObject (or did
  but lost its kind metadata at storage time).
- **SURFACE text:** `no method 'X' on receiver kind Y`.
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (struct method dispatch
  via extend block / impl block / field arithmetic).

### complex::complex_line_from_points + stress_nested::nested_typed_objects_field_sum

Class: **SCOPE-RECLAIM** (2 tests)

```
Expected run error, but got: Some(Object {"Number": Number(0.0)})
Expected run error, but got: Some(Object {"Number":
Number(2.720004750201124e-309)})
```

(These two are marked `// BUG: nested typed struct field access ...
returns the inner object instead of the field` in the source and use
`.expect_run_err()` to assert the bug surfaces. The error path no
longer surfaces — instead silent-wrong-output: 0 or denormalized
floating-point garbage. The fixtures still depend on the same nested-
struct-field-path resolution issue, but the test-fixture shape is
"assert-on-bug-surface", not "assert-on-correct-semantics".)

- **Dated user pull-in:** 2026-05-22 W17.3-4 per-container FieldType.
  The fixture asserts on the bug-surface; after W17.3-4 lands per-
  container FieldType + nested-field-path resolution, the fixture
  must be REWRITTEN to assert correct semantics (so the test fails in
  a different way today). Routes to SCOPE-RECLAIM (same scope as the
  underlying fix) with the additional note that the fixture
  assertion will change shape after fix.
- **SURFACE text:** `Expected run error, but got: Some(Object
  {"Number": Number(...)})`.
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** the SURFACE itself (bug-fixture shape; will
  need rewrite after fix).

### generics_comptime::comptime_field_static_access + comptime_field_numeric_default

Class: **SCOPE-RECLAIM** (2 tests)

```
Runtime error: Undefined variable: Currency. Variable names resolve
from local scope and module scope.
Runtime error: Undefined variable: Percent. Variable names resolve
from local scope and module scope.
```

- **Dated user pull-in:** 2026-05-22 "Comptime trait into v0.3" row.
  Comptime-field static access (`Currency.symbol`, `Percent.decimals`)
  via the type-symbol-as-value path is the comptime-trait/comptime-
  field landing surface.
- **SURFACE text:** `Undefined variable: <TypeName>` for the type-
  symbol-as-value lookup.
- **(Incorrect) v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (`Currency.symbol`
  returns `"$"`).

### stress_decl::generic_struct_default_type_name + generic_struct_inferred_type_name + type_method_on_struct_instance + type_method_on_type_symbol

Class: **SCOPE-RECLAIM** (4 tests)

```
Runtime error: unsupported constant variant in PushConst
(Wave 6 follow-up): Discriminant(15) (line 5)
Runtime error: unsupported constant variant in PushConst
(Wave 6 follow-up): Discriminant(15) (line 4)
```

- **Dated user pull-in:** 2026-05-22 "Comptime trait into v0.3" row +
  the parallel V3-S5 ckpt-5 construction-cascade (the type-symbol
  `.type()` method dispatches via the same construction path). The
  SURFACE message names `Wave 6 follow-up` which is part of the
  same in-flight v0.3 workstream (Wave-6 is the comptime/type-symbol
  rebuild named in the same disposition cluster).
- **SURFACE text:** `unsupported constant variant in PushConst (Wave 6
  follow-up): Discriminant(15)`.
- **(Incorrect) v0.4 anchor cited:** none — SURFACE cites `Wave 6
  follow-up` (in-v0.3 work), not v0.4.
- **Test asserts on:** user-facing semantics (`a.type().to_string()`,
  `Point.type().to_string()`, generic-struct default type name).

## UNKNOWN list

(none)
