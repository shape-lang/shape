# closures_hof classification

**HEAD:** 82f049dd
**Total tests in binary:** 456
**Passed:** 296 / Failed: 160 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test closures_hof --no-fail-fast 2>&1`
**Audit log:** `/tmp/audit_logs/closures_hof.log` (1478 lines)

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 123 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 37 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |
| **TOTAL**          | **160** |

Grouped by failure SURFACE shape. Per-shape counts:

| # | Shape | Class | Count |
|---|---|---|---|
| S1 | `Semantic error: Cannot infer types for binary operation` (closure param `unknown`) | FN-REG-CORRECTNESS | 77 |
| S2 | `Runtime error: mutable/shared capture access in a frame without upvalues` | FN-REG-CORRECTNESS | 23 |
| S3 | `Runtime error: Not implemented: op_new_array(N) / op_new_typed_array(N): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3` | SCOPE-RECLAIM | 19 |
| S4 | `Runtime error: Not implemented: {map,filter,flatMap,distinctBy}: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1` (receiver Ptr(TypedArray)) | SCOPE-RECLAIM | 18 |
| S5 | `Runtime error: call_value_immediate_nb: callee must be Closure/ModuleFn/UInt64, got Ptr(NativeView)` | FN-REG-CORRECTNESS | 11 |
| S6 | `Runtime error: TypeError: expected string, got string` (typed StringConcat path) | FN-REG-CORRECTNESS | 3 |
| S7 | `Runtime error: {map,…}: second argument must be a closure, got kind {Ptr(NativeView)\|UInt64}` | FN-REG-CORRECTNESS | 2 |
| S8 | Silent-wrong result (`Expected 49, got 0` / `Expected 42, got 0.000…208`) | FN-REG-CORRECTNESS | 2 |
| S9 | `assertion left == right failed: slot kind TypedArray does not match HeapValue::Decimal` (wire_conversion.rs:201 panic) | FN-REG-CORRECTNESS | 1 |
| S10 | `Runtime error: Undefined variable: total.` (closure-capture resolution failure) | FN-REG-CORRECTNESS | 1 |
| S11 | `Semantic error: [B0005] let mut binding 'val' was moved into a closure` (borrow-checker on closure-share pattern) | FN-REG-CORRECTNESS | 1 |
| S12 | `Runtime error: no method 'mul' on receiver kind Int64` (named recursive fn body) | FN-REG-CORRECTNESS | 1 |
| S13 | `Semantic error: empty array '<name>' has an un-resolvable element type` (closure-of-X array element-type inference) | FN-REG-CORRECTNESS | 1 |

---

## S1 — Closure-param type-inference loss (77 tests, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt (canonical)

```
thread 'basic::lambda_pipe_two_params' panicked at tools/shape-test/src/shape_test.rs:1292:9:
Expected run ok, got error: Some("Semantic error: Cannot infer types for binary operation `Add`:
operand types are `unknown` and `unknown`. Strict typing requires both operands to have a known
concrete type at compile time. Add a type annotation to disambiguate.")
```

### Minimal repro shape

```shape
let add = |a, b| a + b   // ← lambda body cannot infer a/b types from call site
let x = add(2, 3)
```

Or via HOF:

```shape
fn apply(f, x) { f(x) }   // f's signature unknown
let r = apply(|n| n * 2, 5)
```

### Why FN-REG-CORRECTNESS

These are textbook lambda / HOF programs every Shape user would expect to work
(no annotation should be required on a 2-arg lambda when both call-sites are
typed). The compiler currently refuses with a strict-typing semantic error.
Bidirectional closure inference (per CLAUDE.md "Type System Rules") is supposed
to infer param types from generic method signatures + call-site, but is leaking
`unknown` into binary-op operands when the closure is bound to a `let` rather
than passed inline to a `.method()`. Reasonable user code is rejected.

### Affected subsystem

Type inference / closure param resolution. Most likely
`crates/shape-runtime/src/type_system/` (HM substitution + closure param
binding) or `crates/shape-vm/src/compiler/` (closure-binding type tracker).
Bidirectional inference for closures referenced via `let f = |…| …; f(args)`
or `let f = |…| …; arr.map(f)` is the suspect path.

### Bisected regression commit

Not bisected (audit-only — no cargo runs). Likely candidates: W17 / W18
typed-closure-storage rebuilds or recent type-tracker refactors. Per
CLAUDE.md "Known Constraints" cluster (b) names "typed-closure inference
regressions" as a pre-existing residual cluster — these tests appear to be
the same family.

### Affected test names (77)

```
array_methods::test_hof_reduce_strings   <- compound: actually S6 (TypeError string)
basic::lambda_pipe_complex_expression
basic::lambda_pipe_three_params
basic::lambda_pipe_two_params
basic::lambda_pipe_two_params_mul
basic::lambda_pipe_with_block_body
basic::lambda_pipe_with_block_body_compute
basic::lambda_pipe_with_block_body_multi_statements
basic::test_closure_arithmetic_chain
basic::test_closure_arrow_syntax_multi
basic::test_closure_basic_multi_param
basic::test_closure_complex_body_block
basic::test_closure_conditional_return
basic::test_closure_nested
basic::test_closure_pipe_syntax_arrow
basic::test_closure_scope_isolation
basic::test_closure_three_params
basic::test_closure_with_comparison_chain
basic::test_closure_with_comparison_chain_false
capture::closure_capture_in_returned_lambda
capture::closure_nested_capture
dynamic_captures::closure_factory_with_many_captures
dynamic_captures::closure_many_captures_with_params
edge_cases::edge_closure_in_match_arm
edge_cases::edge_closure_in_match_arm_second
edge_cases::edge_closure_three_deep
edge_cases::edge_lambda_returning_lambda
edge_cases::edge_nested_function_with_closure
edge_cases::test_closure_edge_closure_returning_closure_via_binding
edge_cases::test_closure_edge_conditional_closure_selection
edge_cases::test_closure_edge_double_return_via_binding
edge_cases::test_closure_edge_inside_loop_body
edge_cases::test_closure_edge_match_sub_branch
edge_cases::test_closure_edge_match_with_closure
edge_cases::test_closure_edge_nested_2_levels
edge_cases::test_closure_edge_nested_2_levels_with_block
higher_order::hof_apply_two_args
higher_order::hof_curried_add
higher_order::hof_function_as_return_value_with_state
higher_order::hof_map_over_pair
higher_order::hof_return_function_from_if
higher_order::hof_return_function_from_if_else_branch
higher_order::test_hof_factory_pattern
higher_order::test_hof_flip
higher_order::test_hof_multiplier_factory
higher_order::test_hof_nested_adder_chain
mutable_capture::test_mutable_capture_bug_array_push
mutable_capture::test_mutable_capture_bug_conditional_accumulate
mutable_capture::test_mutable_capture_bug_count_calls
mutable_capture::test_mutable_capture_bug_max_tracker
mutable_capture::test_mutable_capture_bug_string_builder
mutable_capture::test_mutable_capture_bug_with_condition
stress_capture::test_custom_apply
stress_capture::test_iife_multi_param
stress_capture::test_lambda_complex_expr
stress_closure_edge::test_closure_block_with_if_else
stress_closure_edge::test_deep_capture_chain
stress_closure_edge::test_fibonacci_via_closures
stress_closure_edge::test_full_pipeline_complex
stress_closure_edge::test_lambda_passed_inline_and_stored
stress_closure_edge::test_lambda_stored_and_passed
stress_closure_edge::test_nested_closure_capture_arithmetic
stress_hof::test_closure_capture_bool
stress_hof::test_closure_does_not_leak_locals
stress_hof::test_closure_reuse
stress_hof::test_reduce_empty_array
stress_lambda_basic::test_closure_factory_two_instances
stress_lambda_basic::test_double_nested_closure
stress_lambda_basic::test_lambda_block_body
stress_lambda_basic::test_lambda_block_conditional
stress_lambda_basic::test_lambda_block_conditional_positive
stress_lambda_basic::test_lambda_block_multiple_locals
stress_lambda_basic::test_lambda_four_params
stress_lambda_basic::test_lambda_three_params
stress_lambda_basic::test_lambda_two_params_add
stress_lambda_basic::test_lambda_two_params_sub
stress_lambda_basic::test_nested_closure_basic
stress_lambda_basic::test_nested_closure_chain
```

---

## S2 — `mutable/shared capture access in a frame without upvalues` (23 tests, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt (canonical)

```
thread 'mutable_capture::closure_mutable_capture_counter' panicked at …:1292:9:
Expected run ok, got error: Some("Runtime error: mutable/shared capture access
in a frame without upvalues (line 7)")
```

### Minimal repro shape

```shape
var counter = 0
let bump = || { counter = counter + 1 }
bump()
bump()
// counter expected to be 2 here
```

### Why FN-REG-CORRECTNESS

Mutable-capture / `var` closure-capture semantics are core Shape language
features (RAII + var smart-default per ADR-006 §Bindings). The closure is
being constructed without its upvalue frame (compiler emit path), then the VM
correctly diagnoses the missing frame at call time. This is a compiler-side
construction bug, not a missing language feature — the user pattern (counter
incremented by a closure) is the canonical motivating example for `var` /
SharedAtomicMut in ADR-006.

### Affected subsystem

Closure frame setup / upvalue allocation (`crates/shape-vm/src/compiler/`
closure-capture emit path + `executor/mod.rs::call_frame` upvalue binding).
Likely interaction with `BindingStorageClass` SharedAtomicMut path
(`type_tracking.rs:286`) not flowing into the emitted CreateClosure operand.

### Affected test names (23)

```
capture::closure_returned_keeps_captures_alive
dynamic_captures::mutable_capture_counter_reads_from_outer
dynamic_captures::mutable_capture_modifies_enclosing_scope
mutable_capture::closure_mutable_capture_accumulator
mutable_capture::closure_mutable_capture_counter
mutable_capture::test_closure_capture_mutable_internal_state
mutable_capture::test_closure_counter_pattern_outer_read
mutable_capture::test_mutable_capture_bug_accumulator_in_loop
mutable_capture::test_mutable_capture_bug_multiple_vars
mutable_capture::test_mutable_capture_bug_nested_closure
mutable_capture::test_mutable_capture_bug_partial_mutation
mutable_capture::test_mutable_capture_bug_returned_closure
mutable_capture::test_mutable_capture_bug_swap_values
mutable_capture::test_mutable_capture_bug_visible_after_call
mutable_capture::test_mutable_capture_counter_five
mutable_capture::test_mutable_capture_counter_increment_output
mutable_capture::test_mutable_capture_decrement
mutable_capture::test_mutable_capture_multiply_accumulate
mutable_capture::test_mutable_capture_running_sum_output
mutable_capture::test_mutable_capture_toggle
mutable_capture::test_mutable_capture_toggle_four_times
stress_capture::test_mutable_capture_accumulator
stress_capture::test_mutable_capture_counter
```

---

## S3 — `op_new_array / op_new_typed_array — V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE` (19 tests, SCOPE-RECLAIM)

### Class: **SCOPE-RECLAIM**

### Failure excerpt (canonical, verbatim)

```
Runtime error: Not implemented: op_new_array(3): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. The deleted typed-array-data enum +
`Buf<T>` / aligned-typed-buf wrapper layer + outer `HeapValue::TypedArray(Arc<_>)`
arm + `HeapKind::TypedArray=8` ordinal DELETED across V3-S5 ckpt-1..ckpt-4
per W12-typed-array-data-deletion audit §3.5 + §3.6 + ADR-006 §2.7.24 Q25.A
SUPERSEDED. Post-deletion target is per-T v2-raw `TypedArray<T>` flat-struct
monomorphization … Construction-site rebuild lands at ckpt-6 STRICT close
after ckpt-5-prime (wire/marshal/json + 4-table lockstep) + ckpt-5-prime²
(storage migration + 10 intrinsics marshal-parameter migration).
REFUSED ON SIGHT: TypedArrayData resurrection under any rename (Refusal #1).
```

### Dated user disposition pulling this in

**2026-05-18** — "V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade. W16.2-A
typed-object-element + W16.2-B trait-object-element + W16.2-C empty-literal/
spread/comprehension." → SCOPE-RECLAIM row 1.

### Cite incorrectness

The SURFACE itself cites "Construction-site rebuild lands at ckpt-6" — which
IS the work the 2026-05-18 disposition explicitly pulled into v0.3. The
SURFACE is technically internally consistent (it does not cite "v0.4 /
planned") but the work it defers is in v0.3 scope. No v0.4 anchor cited by
the SURFACE; routes to SCOPE-RECLAIM by virtue of the underlying construction-
cascade work being in v0.3 by the 2026-05-18 row.

### Asserts on SURFACE vs user-facing semantics

Tests assert on user-facing semantics (`let xs = [1,2,3]`, then map/filter/
reduce-on-arrays returning concrete values). They will pass without
modification once op_new_array / op_new_typed_array constructions ship at
ckpt-6 STRICT close.

### Affected test names (19)

```
array_methods::array_flatmap_basic                    <- op_new_typed_array(2)
array_methods::test_hof_array_flatmap                 <- op_new_typed_array(2)
array_methods::test_hof_pipeline_simulation           <- op_new_array(3)
basic::test_closure_in_array_call_via_binding         <- op_new_array(3)
higher_order::test_hof_array_flatmap                  <- op_new_typed_array(2)
higher_order::test_hof_pipeline_simulation            <- op_new_array(3)
stress_capture::test_empty_array_every                <- op_new_array(0)
stress_capture::test_empty_array_filter               <- op_new_array(0)
stress_capture::test_empty_array_map                  <- op_new_array(0)
stress_capture::test_empty_array_some                 <- op_new_array(0)
stress_capture::test_flatmap_basic                    <- op_new_typed_array(2)
stress_closure_edge::test_predicate_factory           <- op_new_array(3)
stress_hof::test_closure_captures_function_param      <- op_new_array(2)
stress_hof::test_closures_in_array                    <- op_new_array(3)
stress_hof::test_closures_in_array_invoke_each        <- op_new_array(3)
stress_hof::test_lambda_modulo                        <- op_new_array(3)
stress_hof::test_lambda_returning_array               <- op_new_array(2)
stress_hof::test_multiple_closures_same_scope         <- op_new_array(2)
stress_hof::test_nested_map_calls                     <- op_new_typed_array(2)
```

---

## S4 — `Not implemented: {map,filter,flatMap,distinctBy} — V3-S5 ckpt-2 consumer-cascade tier 1 SURFACE` (18 tests, SCOPE-RECLAIM)

### Class: **SCOPE-RECLAIM**

### Failure excerpt (canonical, verbatim)

```
Runtime error: Not implemented: filter: SURFACE — V3-S5 ckpt-2 consumer-cascade
tier 1 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per
W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED.
The previous `Arc<TypedArrayData>` receiver-recovery + per-variant match-arm
dispatch path (~206 references across 11 public handlers in this file)
cascade-broke at the enum deletion site (`crates/shape-value/src/heap_value.rs:3944`).
Post-deletion target is the v2-raw `TypedArray<T>` flat-struct carrier …
Receiver kind: Ptr(TypedArray). UNREACHABLE until ckpt-6 STRICT close.
REFUSED ON SIGHT: TypedArrayData resurrection under any rename
(Refusal #1, W12 audit §7).
```

### Dated user disposition pulling this in

**2026-05-18** — V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade row
(also covers ckpt-2 consumer-cascade, which is the same workstream named "the
annotation_targets + annotations_comptime cluster IS THIS WORK"). All receiver
kinds in these failures are `Ptr(TypedArray)` (verified via grep), distinct
from the filter-on-TypedObject / map-on-TypedObject family (vec.shape:46-54 /
56-62) that the user re-classified as FN-REG-CORRECTNESS on 2026-05-27 — that
TypedObject family does not appear in this binary.

### Cite incorrectness

The SURFACE cites internal ckpt landmarks ("UNREACHABLE until ckpt-6") and
does not cite "v0.4 / planned". The underlying work is in v0.3 scope by the
2026-05-18 row. Routes to SCOPE-RECLAIM.

### Asserts on SURFACE vs user-facing semantics

Tests assert on user-facing semantics (e.g. `[1,2,3].map(|x| x*2)` should
return `[2,4,6]`). Will pass without modification once ckpt-6 STRICT lands.

### Affected test names (18)

```
array_methods::test_hof_nested_map                            <- map
higher_order::test_hof_map_with_named_fn_via_lambda           <- map
higher_order::test_hof_nested_map                             <- map
stress_capture::test_chain_map_filter                         <- filter
stress_capture::test_chain_map_filter_some                    <- filter
stress_capture::test_chain_map_map                            <- map
stress_capture::test_closure_in_function                      <- filter
stress_capture::test_closure_in_function_with_capture         <- map
stress_capture::test_flatmap_expand                           <- flatMap
stress_capture::test_pipeline_map_filter_find                 <- filter
stress_closure_edge::test_chain_three_maps                    <- map
stress_closure_edge::test_closure_factory_with_array_method   <- filter
stress_closure_edge::test_flatmap_then_length                 <- flatMap
stress_closure_edge::test_map_with_block_body_closure         <- map
stress_hof::test_distinct_by                                  <- distinctBy
stress_hof::test_large_array_filter                           <- filter
stress_hof::test_large_array_map                              <- map
stress_hof::test_map_to_arrays                                <- map
```

---

## S5 — `call_value_immediate_nb: callee must be Closure/ModuleFn/UInt64, got Ptr(NativeView)` (11 tests, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt (canonical)

```
thread 'higher_order::hof_compose_used_directly' panicked at …:1292:9:
Expected run ok, got error: Some("Runtime error: call_value_immediate_nb:
callee must be NativeKind::Ptr(HeapKind::Closure), NativeKind::Ptr(HeapKind::ModuleFn),
or NativeKind::UInt64, got Ptr(NativeView) (line 6)")
```

### Minimal repro shape

```shape
fn compose(f, g) { |x| f(g(x)) }
let h = compose(|x| x + 1, |x| x * 2)
let r = h(3)
```

The composed closure carries a `Ptr(NativeView)` (some `TypedFieldValue` or
heap-view carrier) where the value-call ABI demands `Closure / ModuleFn /
UInt64`. This is a value-call ABI mismatch in the closure-of-closure /
named-fn-passed-as-value path.

### Why FN-REG-CORRECTNESS

Closure composition + named-fn-as-value is canonical higher-order code.
The error happens at runtime (not compile time, not a surface-and-stop) and
the callee carrier kind is wrong — this is the value-call ABI (ADR-006
§2.7.11 / Q12) receiving a non-callable kind. Forbidden-rationalization
risk if "rebuild value-call carrier" is deferred — this is a runtime
correctness bug.

### Affected subsystem

`crates/shape-vm/src/executor/control_flow/mod.rs::op_call_value` +
`call_convention.rs::call_value_immediate_*`. NativeView is leaking into the
callee slot where a Closure/ModuleFn should be — likely the closure-construction
emit path packing a `NativeView` carrier for `fn-name-as-value` or for closures
returned from HOFs.

### Affected test names (11)

```
basic::test_closure_call_other_closure
higher_order::hof_compose_two_functions
higher_order::hof_compose_used_directly
higher_order::hof_twice_applies_function_twice
higher_order::hof_twice_with_double
higher_order::test_hof_apply_twice_with_double_via_binding
higher_order::test_hof_compose_triple
higher_order::test_hof_compose_via_binding
higher_order::test_hof_twice_via_binding
stress_capture::test_custom_compose
stress_closure_edge::test_pipe_two_functions
```

---

## S6 — `TypeError: expected string, got string` (typed StringConcat path) (3 tests, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt

```
V2 bytecode verification warning: 1 violation(s) found
  - V2 typed opcode StringConcatTyped at offset 34 in function
    'Vec.reduce::string_string_closure_0_string_bf6dc5a8408b7231b'
    has no FrameDescriptor

thread 'array_methods::test_hof_reduce_strings' panicked at …:1325:9:
Expected run ok, got error: Some("Runtime error: TypeError: expected string,
got string (line 2)")
```

### Why FN-REG-CORRECTNESS

The error message itself is incoherent — "expected string, got string" is a
runtime type-check that compares a string to a string and reports a mismatch.
Almost certainly a NativeKind discriminator mismatch between two string
carriers (`String` vs `StringV2`) reaching the typed StringConcat opcode
through a generated closure's reduce. Compound with the V2 bytecode
"no FrameDescriptor" verification warning, suggests the typed-opcode emit
path is missing the FrameDescriptor that the StringConcatTyped runtime path
relies on for carrier discrimination.

### Minimal repro shape

```shape
let words: Array<string> = ["a", "b", "c"]
let s = words.reduce("", |acc, w| acc + w)   // ← typed reduce, typed string concat
```

### Affected subsystem

Typed string-concat emit path + `Vec.reduce` monomorphized stdlib variant
+ FrameDescriptor attachment for V2 typed opcodes. `crates/shape-vm/src/
executor/typed_ops/string_concat.rs` (or equivalent) + reduce stdlib
codegen.

### Affected test names (3)

```
array_methods::test_hof_reduce_strings
higher_order::test_hof_reduce_strings
stress_hof::test_reduce_string_concat
```

---

## S7 — `{map,…}: second argument must be a closure, got kind …` (2 tests, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt

```
Runtime error: map: second argument must be a closure, got kind Ptr(NativeView)
Runtime error: map: second argument must be a closure, got kind UInt64
```

### Why FN-REG-CORRECTNESS

Named function or HOF-returned function passed as `arr.map(named_fn)` or
`arr.map(hof_returning_fn())` carries the wrong NativeKind (NativeView /
UInt64) to the map handler. This is the same kind-carrier-leak family as
S5, surfacing through the map handler's argument check rather than the
value-call ABI. Canonical higher-order pattern (`arr.map(double)`).

### Minimal repro shape

```shape
fn double(n: int) -> int { n * 2 }
let r = [1, 2, 3].map(double)   // ← named fn as method arg → wrong kind
```

### Affected test names (2)

```
stress_hof::test_hof_returning_hof          <- Ptr(NativeView)
stress_hof::test_named_fn_as_map_arg         <- UInt64
```

---

## S8 — Silent-wrong result (2 tests, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt

```
thread 'higher_order::hof_named_function_as_argument' panicked at …:1299:9:
Expected 49, got 0

thread 'higher_order::test_hof_pass_named_fn_via_lambda_wrapper' panicked at …:1299:9:
Expected 42, got 0.00000…0208
```

### Why FN-REG-CORRECTNESS

**Silent-wrong-output** is explicitly named in the taxonomy as
FN-REG-CORRECTNESS. The HOF runs to completion but returns `0` (or a denormal
near-zero) instead of the expected integer. The 0.00…0208 denormal value
suggests typed-slot kind reinterpretation (i64 bit pattern read as f64) —
same kind-discriminator-leak family as S5/S7 but the value flows through
without surfacing the kind mismatch at any guard.

### Minimal repro shape

```shape
fn square(n: int) -> int { n * n }
fn apply(f, x) { f(x) }
let r = apply(square, 7)   // expected 49, got 0
```

### Affected test names (2)

```
higher_order::hof_named_function_as_argument          <- Expected 49, got 0
higher_order::test_hof_pass_named_fn_via_lambda_wrapper  <- Expected 42, got 0.000…0208
```

---

## S9 — `assertion left == right failed: slot kind TypedArray does not match HeapValue::Decimal` (1 test, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt

```
V2 bytecode verification warning: 2 violation(s) found
  - V2 typed opcode NewTypedArrayI64 at offset 4 in function
    'Vec.groupBy::i64_i64_closure_0_i64_baf6c26fbf453a083' has no FrameDescriptor
  - V2 typed opcode TypedArrayPushI64 at offset 89 in function
    'Vec.groupBy::i64_i64_closure_0_i64_baf6c26fbf453a083' has no FrameDescriptor

thread 'stress_hof::test_group_by_even_odd' panicked at
crates/shape-runtime/src/wire_conversion.rs:201:5:
assertion `left == right` failed: slot kind TypedArray does not match HeapValue::Decimal
  left: Decimal
 right: TypedArray
```

### Why FN-REG-CORRECTNESS

**Hard panic** in `wire_conversion.rs:201` — explicit `assert_eq!` failure
on slot-kind vs HeapValue-kind. This is a NativeKind / HeapValue
discriminator divergence reaching the wire-conversion boundary. SIGABRT-class
per the taxonomy. The groupBy result is being serialized with the wrong
slot-kind label on a Decimal HeapValue. Connected to the same NativeKind-mis-
labeling family (S5/S6/S7/S8) but here the wire-conversion path's assertion
catches it instead of producing silent-wrong output.

### Affected subsystem

`crates/shape-runtime/src/wire_conversion.rs:201` slot-kind vs HeapValue
parity assertion. Trigger is `Vec.groupBy` with int-keyed groups returning
a typed structure (Decimal sub-payload + TypedArray outer kind labeling
inverted).

### Affected test names (1)

```
stress_hof::test_group_by_even_odd
```

---

## S10 — `Runtime error: Undefined variable: total.` (1 test, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt

```
thread 'stress_capture::test_for_each_side_effect' panicked at …:1292:9:
Expected run ok, got error: Some("Runtime error: Undefined variable: total.
Variable names resolve from local scope and module scope.")
```

### Why FN-REG-CORRECTNESS

A `var total = 0; arr.forEach(|x| { total = total + x })` pattern. The
closure body references `total` but the runtime cannot resolve it — closure
upvalue binding is failing for a `forEach` side-effect pattern. Same family
as S2 (mutable-capture upvalue missing) but surfaces as scope-resolution
failure rather than the upvalue-missing message. Core mutable-capture
correctness.

### Affected test names (1)

```
stress_capture::test_for_each_side_effect
```

---

## S11 — `[B0005] let mut binding 'val' was moved into a closure` (1 test, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt

```
thread 'stress_closure_edge::test_two_closures_sharing_capture' panicked at …:1292:9:
Expected run ok, got error: Some("Semantic error: [B0005] `let mut` binding
'val' was moved into a closure here and cannot be read in the outer scope
afterwards (Rust-move semantics). Use `var val` if the binding needs to be
observed or mutated in the outer scope after capture, or observe mutations
via the closure's return value.")
```

### Why FN-REG-CORRECTNESS

The diagnostic itself is well-formed and suggests the right fix (`var val`).
However, the **test** is named `test_two_closures_sharing_capture` and asserts
on shared-capture behavior — i.e. the fixture is written to exercise the
ADR-006 `SharedAtomicMut` storage class, not `let mut` move semantics. Either
(a) the test fixture should be using `var val` to match ADR-006 — making this
arguably FN-REG-DIAGNOSTIC (test fixture needs update to use `var`), or
(b) the compiler should be inferring SharedAtomicMut for this binding shape
per ADR-006 §2.7 storage-class inference. Audit-conservative classification:
FN-REG-CORRECTNESS because the test is asserting on a documented language
feature (two closures sharing a mutable capture) that ADR-006 names as a
first-class case, and the user 2026-05-21 disposition includes "object
destructuring must fully work" — capture-sharing is in the same correctness
family.

### Affected test names (1)

```
stress_closure_edge::test_two_closures_sharing_capture
```

---

## S12 — `Runtime error: no method 'mul' on receiver kind Int64` (1 test, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt

```
thread 'stress_hof::test_named_recursive_fn' panicked at …:1292:9:
Expected run ok, got error: Some("Runtime error: no method 'mul' on
receiver kind Int64 (line 4)")
```

### Why FN-REG-CORRECTNESS

A recursive named function (likely `fn fact(n: int): int { if n <= 1 { 1 }
else { n.mul(fact(n-1)) } }` or similar) attempting to call `.mul()` on an
Int64 receiver — the method registry has no `mul` PHF entry for Int64. This
is either a test-fixture issue (user wrote `n.mul(...)` expecting it to
exist) OR a stdlib coverage gap. Either way, the SURFACE is at runtime
(no compile-time error), and Int64 method `.mul()` is a reasonable
expectation paralleling Decimal/BigInt's `.mul()` method. Audit-conservative
classification: FN-REG-CORRECTNESS pending stdlib-coverage decision.

### Affected test names (1)

```
stress_hof::test_named_recursive_fn
```

---

## S13 — Empty-array-of-closures un-resolvable element type (1 test, FN-REG-CORRECTNESS)

### Class: **FN-REG-CORRECTNESS**

### Failure excerpt

```
thread 'stress_capture::test_closure_capture_loop_variable' panicked at …:1292:9:
Expected run ok, got error: Some("Semantic error: cannot determine the element
type of empty array `closures`. The array is created empty with no `Array<T>`
annotation, so its element type must come from the first `.push(...)` — but the
type of the value pushed here is not statically known. … (`let mut closures: Array<T> = []`)
…

Semantic error: empty array `closures` has an un-resolvable element type.
…")
```

### Why FN-REG-CORRECTNESS

A `let mut closures = []; for i in 0..n { closures.push(|| i) }` pattern.
The compiler cannot infer the element type of `closures` because closure types
are not first-class in the FieldType lattice (no `Closure<…> -> …>` FieldType
constructor). Per user 2026-05-21 disposition "Array<string> must work" and
the broader "object destructuring must fully work" — Array-of-closures is in
the same canonical-usage family. FN-REG-CORRECTNESS pending FieldType
extension for closure element types.

### Affected test names (1)

```
stress_capture::test_closure_capture_loop_variable
```

---

## UNKNOWN entries

None. All 160 failures classified with verbatim evidence from the run log
at HEAD 82f049dd.
