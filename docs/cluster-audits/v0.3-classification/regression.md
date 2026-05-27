# regression classification

**HEAD:** 82f049dd
**Total tests in binary:** 114
**Passed:** 58 / Failed: 55 / Ignored: 0 / SIGABRT-skipped: 1
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test regression --no-fail-fast 2>&1`
**Evidence logs:** `/tmp/jit_out.txt`, `/tmp/tdd_out.txt`,
`/tmp/qa_out2.txt` (= `qa::` with `--skip
regression_crit_1_nested_property_access`), `/tmp/ls_out.txt`.

The binary is *named* `regression`: per TAXONOMY, every failure here
is suspect for FN-REG-CORRECTNESS unless explicitly bisected as
SCOPE-RECLAIM (or fits the narrower diagnostic-text-only rule).

Run discipline note: the default parallel run aborts mid-suite with
`memory allocation of 127522479781296 bytes failed` + SIGABRT after
~30 JIT tests. Re-running with `--test-threads=1` also SIGABRTs on
`qa::regression_crit_1_nested_property_access` (`memory allocation of
135242086536256 bytes failed`). Per-module serial re-runs (`jit::`,
`tdd::`, `qa:: --skip regression_crit_1_nested_property_access`,
`language_surface::`) complete and give the per-test panic text quoted
below. The 127/135 TB allocation is itself an FN-REG-CORRECTNESS
finding (a pointer is being dereferenced as a length) — see the
`regression_crit_1_nested_property_access` row.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 22 |
| FN-REG-DIAGNOSTIC  | 27 |
| SCOPE-RECLAIM      | 5 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 1 |
| **Total failures** | **55 (+ 1 SIGABRT-skipped = 56)** |

Three clusters drive the headline counts:

1. **JIT `Expected Number(X), got Integer(X)` cluster (24 tests in
   `jit.rs`)** — `jit_expect_number` (jit.rs:26-38) asserts on
   `WireValue::Number(_)` (f64), but integer-literal expressions now
   produce `WireValue::Integer(_)` (i64) per the post-strict-typing
   `int`/`number` split (CLAUDE.md §Type System Rules: "`int` and
   `number` are separate"). Values are correct, fixture's declared
   wire-tag is stale. Classified **FN-REG-DIAGNOSTIC**.
2. **SURFACE-bearing failures citing V3-S5 ckpt-5 (5 tests across
   `qa.rs`, `language_surface.rs`, `jit.rs`)** — `op_new_array`,
   `SetIndexRef`, `String.split` SURFACE messages. Classified
   **SCOPE-RECLAIM** per TAXONOMY 2026-05-18 row (V3-S5 ckpt-5/ckpt-6
   construction-cascade; "annotation cluster IS this work") +
   2026-05-21 row ("Array<string> must work") + 2026-05-22 row
   (W17.3-4 per-container FieldType).
3. **Real correctness regressions (22 tests)** — panics, Null returns,
   silent-wrong-output (pointer-as-float / pointer-as-int leaks),
   ADR-006 §2.7.13 kind-drift assertion firing in production VM,
   SIGABRT-on-127TB-alloc, inference-loss on plausibly-correct
   user-facing code (textbook fib, multi-arg lambda, named-fn-as-arg,
   intra-mod fn calls), `!!` error-context propagation broken,
   parser-rejects-documented-trait-method-syntax, enum equality
   refused by type-checker. Classified **FN-REG-CORRECTNESS**.

The previous draft of this doc routed 36 failures to FN-REG-DIAGNOSTIC
based on a truncated parallel-run log. With the full per-module
serial-run evidence the breakdown is 27 FN-REG-DIAGNOSTIC + 22
FN-REG-CORRECTNESS + 5 SCOPE-RECLAIM + 1 UNKNOWN: many JIT failures
that *look* like the Integer/Number cluster actually return `Null`
(no value, JIT regression) or a raw-pointer-shaped integer (silent-
wrong-output), and several non-JIT failures (annotation /
String.split / array-mutation) cite V3-S5 ckpt-5 SURFACE text that
binds to the 2026-05-18 dated user pull-in.

## Per-test classification

### jit::jit_ackermann

Class: **FN-REG-CORRECTNESS**

```
Expected Number(125), got Null
```

- JIT path returns `Null` from canonical recursive Ackermann. Distinct
  from the Integer/Number diagnostic cluster — no value returned at all.
- Subsystem: JIT call-convention / recursive-frame return path.

### jit::jit_add + jit_array_create_and_access + jit_array_length + jit_collatz + jit_comparison_eq + jit_comparison_gt + jit_comparison_gte_lte + jit_comparison_lt + jit_comparison_neq + jit_div + jit_fib_iterative + jit_if_else + jit_int_unboxing_fib_swap + jit_int_unboxing_nested_module_bindings + jit_int_unboxing_sum_module_binding + jit_large_number_arithmetic + jit_local_variables + jit_loop_comparison_fused + jit_mandelbrot_mixed_numeric_loop_regression + jit_mod + jit_mul + jit_nested_loop_comparison + jit_sub + jit_variable_reassignment + jit_while_loop + jit_while_sum_to_100

Class: **FN-REG-DIAGNOSTIC** (24 tests, one helper-function patch
retires all)

```
Expected Number(15), got Integer(15)
...
Expected Number(5050), got Integer(5050)
```

- Old expected: `WireValue::Number(f64)`.
- New actual: `WireValue::Integer(i64)` — correct value, correct
  typing per the int/number split.
- Language change: post-strict-typing, `int + int → int`; `WireValue`
  faithfully encodes that via `Integer(_)` vs `Number(_)`.
- Fix shape: widen `jit_expect_number` (jit.rs:26) to accept
  `Integer(n)` when `(n as f64 - expected).abs() < eps`, or split the
  helper. `jit_mandelbrot_mixed_numeric_loop_regression` is
  `#[should_panic]` and fails because the panic message text doesn't
  contain the literal `"Expected 5739"` anymore (reads `"Expected
  Number(5739)"`) — same family, fixture text-assertion stale.

### jit::jit_array_mutation_via_function

Class: **SCOPE-RECLAIM**

```
JIT execution failed: ... "Not implemented: SetIndexRef: SURFACE —
V3-S5 ckpt-5 consumer-cascade tier 3 surface. `RefTarget::TypedIndex`
variant + the deleted typed-array-data `write_index_in_place` API +
the deleted-enum's `Arc<...>` carrier all DELETED at ckpt-1..ckpt-4
... ckpt-6 STRICT close ... REFUSED ON SIGHT: TypedArrayData /
RefTarget::TypedIndex resurrection under any rename (Refusal #1)."
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6) + 2026-05-22
  (W17.3-4 per-container FieldType).
- SURFACE text quoted above.
- Incorrect v0.4 anchor cited: none; surface pins to ckpt-6.
- Why SCOPE-RECLAIM: array-index mutation is part of the typed-array
  rebuild explicitly named in the 2026-05-18 row.
- Test asserts on user-facing semantics. Stays the same after fix.

### jit::jit_array_push_via_function

Class: **FN-REG-CORRECTNESS** (empty-array inference loss across &-ref
boundary)

```
JIT execution failed: ... "Bytecode compilation failed: Semantic error:
empty array `arr` has an un-resolvable element type. It is created
empty (`[]`) ... never pushed to ..."
```

- Test source `fn add_item(&arr, item) { arr = arr.push(item) }; let
  arr = []; ...` — the compiler's "never pushed to" claim is wrong.
  Element-type inference doesn't propagate through the `&` ref-arg.
- Subsystem: compiler type-inference (`shape-runtime/src/type_system/`)
  ref-arg element-type propagation. Not in CLAUDE.md §Known Constraints.

### jit::jit_function_call

Class: **FN-REG-CORRECTNESS**

```
Expected Number(42), got Null
```

- Simple non-recursive function call returns `Null` instead of `42`.
- Subsystem: JIT call-convention / return-tag staging.

### jit::jit_int_unboxing_large_result + jit_int_unboxing_mixed_local_types + jit_int_unboxing_nested_loops + jit_int_unboxing_sum_local

Class: **FN-REG-CORRECTNESS** (4 tests, all returning Null on JIT path)

```
Expected Number(4999950000), got Null   # large_result
Expected Number(34), got Null           # mixed_local_types
Expected Number(100), got Null          # nested_loops
Expected Number(499500), got Null       # sum_local
```

- Subsystem: JIT int-unboxing optimization. Sum/loop reductions return
  `Null` instead of the accumulated value. Silent-wrong-output, not a
  tag mismatch.

### jit::jit_matrix_mul_small

Class: **FN-REG-CORRECTNESS** (same empty-array / ref-arg inference
loss family)

```
JIT execution failed: ... "Cannot infer types for binary operation
`Mul`: operand types are `unknown` and `unknown`. ... empty array `a`
has an un-resolvable element type ..."
```

- Matrix multiply allocates `let a = []` then pushes rows; element
  type is silently dropped, downstream `a[i][j] * b[j][k]` rejected.

### jit::jit_recursive_fibonacci

Class: **FN-REG-CORRECTNESS** (inference-loss on textbook fib)

```
JIT execution failed: ... "Cannot infer types for binary operation
`Less`: operand types are `unknown` and `int`. ..."
```

- Test source: `fn fib(n) { if n < 2 { n } else { fib(n-1) + fib(n-2) } }`.
- Recursive call's parameter `n` should infer as `int` from
  `n - 1` minus-int-literal arithmetic. Self-recursive function param
  inference regressed.

### jit::jit_sieve_small

Class: **FN-REG-CORRECTNESS** (empty-array inference-loss family)

```
JIT execution failed: ... "empty array `flags` has an un-resolvable
element type ..."
```

### jit::jit_trampoline_result_callvalue

Class: **FN-REG-CORRECTNESS** (silent-wrong-output, raw pointer leak)

```
Expected Number(42), got Integer(125243264144096)
```

- `125243264144096` is pointer-shaped. JIT call-value path returns a
  raw pointer where it should return the scalar `42`. Sibling
  `jit_trampoline_string_callvalue` fires the JIT SURFACE
  `RETURN_TAG_NANBOXED reached the host boundary without a stamped
  NativeKind (raw_bits=0x71e878d50130) ... executor.rs:267` and falls
  back to interpreter (PASSES). The Result-trampoline twin has no
  fallback and silently leaks the pointer as an integer.
- Subsystem: JIT FFI return-path stamping for Result-typed CallValue.

### tdd::bug3_mutable_capture_propagates

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: "Runtime error: mutable/shared capture
access in a frame without upvalues (line 4)"
```

- Test source (tdd.rs:29):
  `let mut count = 0; let inc = || { count = count + 1; count };
   inc(); inc(); count`
- Plausibly-correct closure-mutates-outer-let pattern. Capture-slot
  setup regressed.
- Subsystem: closure capture-slot ABI / `OwnedClosureBlock` per
  ADR-006 §2.7.8 Q10.

### tdd::bug4_module_member_access

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: "Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. ..."
```

- Test source: `mod math { pub fn add(a, b) { a + b } }; math::add(1, 2)`.
- Unannotated `pub fn` params should infer from call-site arg types.
  Intra-mod-block param-type inference regressed.

### tdd::bug5_named_fn_as_argument

Class: **FN-REG-CORRECTNESS** (silent-wrong-output, pointer-as-float
leak)

```
Expected 42, got 0.000...208     (2.08e-322 — pointer reinterpreted as f64)
```

- Test source: `fn double(x) { x * 2 }; fn apply(f, x) { f(x) };
  apply(double, 21)`.
- Returns a denormalized f64 whose low bits = 208 — raw function
  pointer interpreted as a number. HOF-passing a named fn corrupts
  the return path.
- Subsystem: named-fn-as-value lowering + CallValue path.

### tdd::bug10_nested_field_mutation

Class: **FN-REG-CORRECTNESS** (ADR-006 §2.7.13 kind-drift assertion
fires in production VM)

```
panicked at crates/shape-vm/src/executor/variables/mod.rs:3046:17:
assertion `left == right` failed: DerefStore: TypedField field_kinds[0]
= Ptr(TypedObject) drift vs RefTarget captured kind Int64 — ADR-006
§2.7.13 / Q14
  left: Ptr(TypedObject)
 right: Int64
```

- Test source (tdd.rs:124): `type Inner { val: int }; type Outer { data:
  Inner }; let mut o = Outer { data: Inner { val: 1 } }; o.data.val = 42`.
- Plausibly-correct nested field mutation. Field-kind tracking at
  deep-store time mismatched: RefTarget captured `Int64` (the leaf
  `val`), but the field-kinds slot at index 0 reads `Ptr(TypedObject)`.
- Subsystem: VM `executor/variables/mod.rs` DerefStore + RefTarget
  kind-capture (ADR-006 §2.7.13 / Q14).

### tdd::bug11_push_through_ref

Class: **FN-REG-CORRECTNESS** (same empty-array inference family as
`jit_array_push_via_function`)

```
Expected run ok, got error: "Semantic error: empty array `items` has
an un-resolvable element type. ..."
```

- Test source: `fn add_item(&arr, item) { arr = arr.push(item) }; let
  mut items = []; add_item(&items, 1); add_item(&items, 2); items.length`.
- Inter-procedural element-type inference dropped across the `&`
  ref-arg + callee push. Subsystem identical to `jit_array_push_via_function`.

### language_surface::comptime_type_info_is_removed_and_not_suggested_by_lsp

Class: **FN-REG-DIAGNOSTIC**

```
Expected semantic diagnostic containing 'type_info has been removed',
found: []
```

- Old expected: `'type_info has been removed'`.
- New actual: no diagnostic at all — `type_info` is silently absent.
- Language change driver: comptime-builtin `type_info` removal. The
  per-test contract is "removal is announced via a diagnostic"; the
  removal happened but the friendly diagnostic was not emitted.
- Fix shape: emit the diagnostic (test passes verbatim) OR drop the
  expectation. The prior draft of this doc flagged this as
  CORRECTNESS (concerned that the builtin might be silently re-allowed).
  Re-routing to DIAGNOSTIC: the diagnostic-array is `[]` (empty), not
  `[]` plus a passing inner expression — the builtin call IS rejected
  by the compiler (no semantic error means inference inferred away the
  reference entirely OR the comptime call doesn't surface), but the
  symbol gating works (the LSP completion-block half of the assertion
  isn't where the test fails). This is the diagnostic-text contract,
  not the gating contract.

### language_surface::declared_result_return_type_accepts_err_context_without_spurious_generic_mismatch

Class: **FN-REG-CORRECTNESS**

```
Error should contain 'yes, something went wrong', got: Runtime error:
Uncaught error: some error (line 3)
```

- Test source (language_surface.rs:111):
  `fn test() -> Result<int> { return Err("some error") !! "yes,
   something went wrong" }; test()?`
- The `!!` error-context is a documented Shape feature (CLAUDE.md
  §Language Features: "Error handling ... `!!` error context"). The
  context string is silently dropped — only the inner `"some error"`
  reaches the uncaught surface.
- Subsystem: Result `?` + `!!` error-context interaction. Silent-
  data-loss on a user-facing language feature.

### language_surface::expression_annotation_before_after_hooks_execute

Class: **SCOPE-RECLAIM**

```
Expected run ok, got error: "Runtime error: Not implemented:
op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3
surface. ... Construction-site rebuild lands at ckpt-6 STRICT close
... REFUSED ON SIGHT: TypedArrayData resurrection under any rename
(Refusal #1). (line 14)"
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6; "annotation cluster
  IS this work") + 2026-05-22 (Comptime trait into v0.3).
- SURFACE text quoted above (full op_new_array V3-S5 ckpt-5 verbatim
  per `annotations_runtime.md` precedent).
- Incorrect v0.4 anchor cited: none; surface pins to ckpt-6.
- Why SCOPE-RECLAIM: annotation `before/after` hook plumbing builds
  arg arrays via `op_new_array` — exact root cause shape called out
  in TAXONOMY 2026-05-18 row.
- Test asserts on user-facing semantics (`before\nafter\n3` output).
- Note on re-routing from the prior draft: this is the row where the
  prior draft explicitly declined SCOPE-RECLAIM ("failure shape is
  functional output mismatch, not a SURFACE cite"). The full
  per-module log shows the failure IS a verbatim ckpt-5 SURFACE cite
  ("Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5
  ckpt-5 consumer-cascade tier 3 surface ..."), identical to the
  `annotations_runtime.md` rows. Routes to SCOPE-RECLAIM.

### language_surface::trait_bound_method_dispatch_resolves_at_runtime

Class: **FN-REG-CORRECTNESS** (parse-level rejection of documented
trait method-decl syntax)

```
Expected no semantic diagnostics, found: [("expected something else,
found identifier `display`", Range { ... line: 2, character: 2 ... })]
```

- Test source: `trait Displayable { display(): string } ... fn
  render<T: Displayable>(value: T) -> string { return value.display() }
  print(render(User { name: "Ada" }))`.
- Parser rejects the trait-method declaration `display(): string` —
  but the documented in-trait method shape per CLAUDE.md §Language
  Features ("Traits: `trait Name { fn method(self) -> ReturnType; }`")
  IS this shape minus a `fn` keyword. The test uses the
  no-`fn`-keyword form; whether that's the canonical shape vs. the
  `fn`-prefixed shape is the regression question. Either way the
  trait-bound dispatch end-to-end test never gets past parse.
- Subsystem: pest grammar `crates/shape-ast/src/shape.pest` trait body.
- 2026-05-22 user disposition pulled "Comptime trait into v0.3", but
  this failure is parse-level for a long-shipped trait method-decl
  shape — not part of the comptime-trait pull-in.

### qa::regression_crit_1_nested_property_access  *(SIGABRT-skipped from serial run)*

Class: **FN-REG-CORRECTNESS** (process-killing memory-allocation bug —
release-blocking)

```
memory allocation of 135242086536256 bytes failed
process didn't exit successfully: ... (signal: 6, SIGABRT: process
abort signal)
```

- Test source (qa.rs:124): two-level nested-TypedObject property
  access, `cfg.server.host` where `cfg: Config { server: Server,
  debug: bool }`.
- 135 TB allocation = a pointer is being decoded as a `usize`-shaped
  length somewhere in the nested-field read path. SIGABRTs the entire
  test binary. Sibling `regression_crit_1_deep_nested_access` is
  `#[should_panic]` and *passes* by panicking with `Expected 42, got
  126593764023984` (also a raw pointer interpreted as int) — confirms
  the same bug-shape on 3-level nesting.
- Subsystem: VM nested-TypedObject field-read. Per the original BUG
  comment "NaN-boxing bug with nested TypedObject" — but
  post-strict-typing there's no NaN-boxing, so the failure is in the
  new typed-slot TypedObject-field-of-TypedObject access path.

### qa::regression_high_2_multi_arg_lambda

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: "Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. ..."
```

- Test source: `let add = |a, b| a + b; add(3, 4)`.
- Bidirectional closure inference should infer `a, b: int` from the
  call site. Single-arg sibling `regression_high_2_lambda_variable_callable`
  (`let inc = |x| x + 1; inc(10)`) PASSES — multi-arg lambda
  call-site inference regressed.
- Subsystem: closure-param inference from call-site arg types.

### qa::regression_high_9_annotation_after_void

Class: **SCOPE-RECLAIM**

```
Expected run ok, got error: "Runtime error: Not implemented:
op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3
surface. ... (line 9)"
```

- Dated pull-in: 2026-05-18 (annotation cluster).
- SURFACE: `op_new_array(0)` ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor: none; ckpt-6 close.
- Why SCOPE-RECLAIM: annotation hook plumbing → array-literal
  construction. Same family as `expression_annotation_before_after_hooks_execute`.
- Test asserts on user-facing semantics.

### qa::regression_med_13_mutable_params  *(`#[should_panic]` — FAILED because it did NOT panic)*

Class: **UNKNOWN**

```
test qa::regression_med_13_mutable_params - should panic ... FAILED
```

- Test source (qa.rs:317, `#[should_panic]`):
  `fn reset(s) { let mut local = s; local = ""; local }; reset("hello")`
  with `.expect_string("")`.
- The test was anchored to a documented bug (`let mut local = s`
  treated as shared ref). It now runs to completion without panicking.
  Without captured run output it's unclear whether the original bug is
  fixed cleanly (→ FN-REG-DIAGNOSTIC, drop `#[should_panic]`) or the
  behavior shifted in a different way that masks the bug
  (→ FN-REG-CORRECTNESS).
- Blocks classification: missing actual-run output (Cargo suppresses
  it under `#[should_panic]` until the panic-shape is asserted).
- Recommended next-step: drop `#[should_panic]` temporarily, re-run,
  compare to `expect_string("")`. If matches → FN-REG-DIAGNOSTIC
  (fix-landed, fixture stale). Else → FN-REG-CORRECTNESS.

### qa::regression_med_1_string_split

Class: **SCOPE-RECLAIM**

```
Expected run ok, got error: "Runtime error: Not implemented:
String.split: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface.
The deleted typed-array-data String `Arc<Buf<Arc<String>>>` result
carrier DELETED at V3-S5 ckpt-1..ckpt-4 ... ckpt-6 STRICT close per
the v2-raw `TypedArray<*const StringObj>` carrier shape. REFUSED ON
SIGHT: TypedArrayData resurrection under any rename (Refusal #1)."
```

- Dated pull-in: 2026-05-21 ("Array<string> must work") + 2026-05-22
  (W17.3-4 per-container FieldType).
- SURFACE text quoted above.
- Incorrect v0.4 anchor: none; ckpt-6 close.
- Why SCOPE-RECLAIM: `String.split` returns `Array<string>` which the
  2026-05-21 user disposition explicitly named as v0.3-gating.
- Test asserts on user-facing semantics (split-result length 3).

### qa::regression_med_1_string_substring

Class: **FN-REG-CORRECTNESS** (off-by-one / inclusive-vs-exclusive end)

```
assertion `left == right` failed: Expected 'el', got 'ello'
  left: "ello"
 right: "el"
```

- Test source: `"hello".substring(1, 3)`, expecting `"el"` (exclusive
  end, JS-style — positions 1 and 2 → "el"). Returns `"ello"` — stdlib
  `String.substring` semantics shifted from `(start, end_exclusive)`
  to something taking either a length or an inclusive end.
- Subsystem: stdlib `String.substring`.

### qa::regression_med_4_enum_equality_different_variants + qa::regression_med_4_enum_equality_same_variant

Class: **FN-REG-CORRECTNESS** (2 tests, enum `==`/`!=` rejected by
type-checker)

```
Expected run ok, got error: "Semantic error: Cannot infer types for
binary operation `NotEqual`: operand types are
`Concrete(Reference(TypePath { segments: [\"Color\"], qualified:
\"Color\" }))` and `Concrete(Reference(TypePath { segments: [\"Color\"],
qualified: \"Color\" }))`. Strict typing requires both operands to have
a known concrete type at compile time."
```

- Test source: `enum Color { Red, Green, Blue }; Color::Red !=
  Color::Green` (and the `==` sibling).
- Both operands ARE the same concrete enum type (`Color`). The
  type-checker mis-rejects because `Concrete(Reference(...))` enum
  references are treated as un-unified for Eq.
- Subsystem: enum `==`/`!=` operator type-check / trait `Eq`
  resolution for enums. Plausibly-correct user-facing canonical enum
  equality rejected.

### qa::regression_med_9_comptime_fields

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: "Runtime error: Undefined variable:
Currency. Variable names resolve from local scope and module scope."
```

- Test source: `type Currency { comptime symbol: string = "$",
  amount: number }; Currency.symbol`.
- `Type.field` static-path access on a comptime field is a documented
  Shape feature (CLAUDE.md §Language Features). Fails with generic
  "Undefined variable" — no SURFACE message, no surface-and-stop.
- Subsystem: type-as-value reference + comptime static-field access.
- SCOPE-RECLAIM rule-out: 2026-05-22 "Comptime trait into v0.3" covers
  comptime traits, not "type-name as static-field-access expression".
  Routes to FN-REG-CORRECTNESS pending user re-disposition.

## Cross-cluster notes

- The SIGABRT default-parallel run is itself a release-blocking signal
  — a `regression` test binary that cannot run to completion masks
  every classification below it.
- The 24-test Number-vs-Integer FN-REG-DIAGNOSTIC cluster is *one*
  helper-function patch (jit.rs:26 `jit_expect_number`), not 24
  independent fixture updates.
- The 4 empty-array-inference-loss tests
  (`jit_array_push_via_function`, `jit_matrix_mul_small`,
  `jit_sieve_small`, `tdd::bug11_push_through_ref`) are all the same
  root cause: inference doesn't propagate element type through `&`
  ref-arg + callee push. One compiler fix retires all four — that fix
  is FN-REG-CORRECTNESS scope, not SCOPE-RECLAIM (no dated user
  pull-in named "ref-arg element-type inference"; baseline language
  correctness).
- Three silent-wrong-output / raw-pointer-leak tests
  (`tdd::bug5_named_fn_as_argument` → pointer-as-f64,
  `jit::jit_trampoline_result_callvalue` → pointer-as-int,
  `qa::regression_crit_1_nested_property_access` → pointer-as-malloc-
  length) are the most severe shape per TAXONOMY ("silent-wrong-output;
  SIGABRT / SEGFAULT").
- 5 SCOPE-RECLAIM tests all SURFACE on the V3-S5 ckpt-5/ckpt-6
  construction-cascade family — consistent with the
  `annotations_runtime.md` / `annotation_targets.md` cluster pattern
  from the rest of the audit. None mis-cite v0.4.
- Zero V0.4-DEFER findings: no test surfaces with a clean "v0.4 /
  planned" annotation per ADR-006 §2.7.14 cleanly outside dated
  pull-in scope. The "v0.4 / planned" string appears in the V2
  verifier fallback message for `jit_array_mutation_via_function` but
  that test is also reachable through the V3-S5 SURFACE — it routes to
  SCOPE-RECLAIM per the dated 2026-05-18 row.
- Zero INFRA-FLAKY findings (the SIGABRT is deterministic, not flaky).

## UNKNOWN list

1. `qa::regression_med_13_mutable_params` — `#[should_panic]` test
   that no longer panics. Need a re-run with `#[should_panic]`
   removed to capture actual output and route to either
   FN-REG-DIAGNOSTIC (clean fix) or FN-REG-CORRECTNESS (masking shift).
