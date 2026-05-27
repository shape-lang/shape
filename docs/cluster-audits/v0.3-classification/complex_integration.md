# complex_integration classification

**HEAD:** 82f049dd
**Total tests in binary:** 100
**Passed:** 38 / Failed: 60 / Crashed (TIMEOUT-class): 2 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test complex_integration --no-fail-fast 2>&1`

## Audit method note

The non-`--exact` `cargo test` invocation aborts mid-binary with SIGSEGV (signal 11) after ~5 tests complete (reproduced 2026-05-27 at HEAD). Per-test classifications come from per-test `--exact` runs of the prebuilt binary `target/debug/deps/complex_integration-4c4bc039c0a6f506` (driver script `/tmp/run-all-cint.sh`, results `/tmp/cint-results.txt`). The full-binary SIGSEGV is documented separately under SIGSEGV-aggregate.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 12 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 50 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 1 (SIGSEGV-aggregate, binary-level) |
| UNKNOWN            | 0 |

(38 tests pass — listed under "Passing tests" at end.)

## Per-test classification

### Class A: SCOPE-RECLAIM — closure-param strict-typing element-kind loss (×36)

**Shared SURFACE pattern:** `Semantic error: Cannot infer types for binary operation \`<Op>\`: operand types are \`unknown\` and \`<...>\`. Strict typing requires both operands to have a known concrete type at compile time.`

**Dated user disposition:** 2026-05-21 — "Array<string> must work" (TAXONOMY explicit row) + 2026-05-22 W17.3-4 per-container FieldType pull-in (TAXONOMY explicit row). Per-element kind must propagate from `Array<T>` / `HashMap<K,V>` / generic-fn-param into closure param binding (`|p| ...`, `|acc, x| ...`, `|x| ...`, `|a, b| ...`). The defect intersects the same root cause documented in `objects.md::object_destructuring_in_function` (closure/destructured-param kind not propagated from call-site).

**Incorrect anchor cited:** none (generic strict-typing error, no v0.4 / §5.16 cite).
**Test asserts on:** user-facing semantics (`expect_output(...)` / `expect_number(...)` / `expect_string(...)`); stays same after fix.

Tests (verbatim failures from `/tmp/cint-results.txt`):

1. `cross_feature::test_complex_array_methods_with_closures_and_match` — `Add unknown/unknown` + `Equal unknown/Priority` (closure-param `p` over `[Priority::Low, ...]` not kinded).
2. `cross_feature::test_complex_closure_mutable_capture_loop_array` — `reduce(|acc, x| acc + x, 0)` over pushed array — closure-params `acc`/`x` unknown.
3. `cross_feature::test_complex_closure_returning_closure` — `|x| base + offset + x` returned closure body — params `base`/`offset`/`x` unknown.
4. `cross_feature::test_complex_enum_in_loop_with_match` — `result + match t {...}` — match-arm string-typed result combined with string `result` but combined-type `unknown`.
5. `cross_feature::test_complex_for_map_filter_fold` — `.reduce(|acc, x| acc + x, 0)` after `.filter().map()` chain.
6. `cross_feature::test_complex_hashmap_with_loop_aggregation` — `existing + score` where `existing = scores.get(name)` is HashMap-value-kind unknown.
7. `cross_feature::test_complex_higher_order_with_enum_result` — `if result < 0` where `result = f(val)` — closure-call-return kind unknown.
8. `cross_feature::test_complex_higher_order_with_enum_result_ok_path` — identical to 7.
9. `cross_feature::test_complex_multi_return_function_with_option` — `safe_divide(a, b)` body `a / b` and `Ok(v) => if v > 10` — function-param + match-bind both unknown.
10. `cross_feature::test_complex_for_map_filter_fold` — (already in 5).
11. `data_structures::test_complex_frequency_counter` — `current + 1` where `current = map.get(key)` HashMap-value unknown.
12. `data_structures::test_complex_struct_with_methods` — `self.x + other.x` on Vec2 — `other` param kind unknown.
13. `multi_function::test_complex_array_flatten` — `.reduce(|acc, x| acc + x, 0)` over `flatMap` result.
14. `multi_function::test_complex_array_unique` — `if item == val` in contains() loop, plus empty-array (combined).
15. `multi_function::test_complex_calculator_four_ops` — `div(z, 6)` body `a / b` — function-params unknown.
16. `multi_function::test_complex_is_palindrome` — `result + s.substring(i, i+1)` — `s` param kind unknown.
17. `multi_function::test_complex_recursive_gcd` — `gcd(b, a % b)` — function-params unknown.
18. `multi_function::test_complex_recursive_power` — `base * power(base, exp - 1)` — function-param `base` unknown; recursive result kind `int | number` union.
19. `multi_function::test_complex_string_reverse` — `result + s.substring(...)` — `s` param unknown.
20. `pattern_based::test_complex_dispatch_table` — `add_fn(3, 4)` where `add_fn = ops.get("add")` HashMap-value-kind unknown.
21. `pattern_based::test_complex_option_chain_safe_operations` — `a / b` in `safe_div`; `if x < 0` in `safe_sqrt`; `"ok: " + r` — three same-class errors.
22. `pattern_based::test_complex_pipeline_transform_filter_reduce` — `.reduce(|acc, x| acc + x, 0)`.
23. `pattern_based::test_complex_recursive_flatten` — `.reduce(|acc, x| acc + x, 0)` on flatMap result.
24. `pattern_based::test_complex_result_chain` — `if n > 0` where `n` is `Ok(n)` match-bind unknown.
25. `real_world::test_program_matrix_operations` — `a[0][0] + b[0][0]` — nested array element kind unknown.
26. `real_world::test_program_number_formatter` — `(n / 1000) + "K"` — `n` param kind unknown.
27. `real_world::test_program_running_statistics` — `.reduce(|acc, x| acc + x, 0) / values.length` — closure body unknown.
28. `real_world::test_program_score_tracker` — same `reduce / length` pattern.
29. `real_world::test_program_simple_calculator_repl` — `"Error: " + e` — `e` from `Err(e)` match-bind unknown.
30. `real_world::test_program_simple_interpreter` — `a + b` after `let a = stack[stack.length - 1]` — array-index kind unknown; combined with empty-array.
31. `real_world::test_program_validator` — `if age < 0` — `age` param kind unknown.
32. `real_world::test_program_word_counter` — `counts.set(word, existing + 1)` — HashMap-value-kind unknown.
33. `stress_edge_cases::test_complex_deeply_nested_closures` — `|c| a + b + c` — closure-capture kinds unknown.
34. `stress_edge_cases::test_complex_long_method_chain` — `.reduce(|acc, x| acc + x, 0)`.
35. `stress_edge_cases::test_complex_recursive_descent_evaluator` — `a + b` from stack pops, plus empty-array.
36. `stress_edge_cases::test_complex_reduce_with_initial_and_transform` — `.reduce(|acc, x| acc + x, 0)` after `.map(|x| square(x))`.

### Class B: SCOPE-RECLAIM — empty-array un-resolvable element type (×5)

**Shared SURFACE pattern:** `Semantic error: empty array \`<name>\` has an un-resolvable element type. It is created empty (\`[]\`) with no \`Array<T>\` annotation and is never pushed to, so the compiler cannot prove what element type it holds.`

**Dated user disposition:** 2026-05-22 W17.3-4 per-container FieldType pull-in. The diagnostic is mis-firing on the canonical `let mut x = [] / x = x.push(v)` pattern — the assignment form is not pattern-matched by the "never pushed to" check despite `x = x.push(v)` semantically being a push. Plausibly-correct user-facing Shape — empty-collection-then-grow is a canonical mutable-collection pattern.

**Incorrect anchor cited:** none. **Test asserts on:** user-facing semantics; stays same after fix.

1. `cross_feature::test_complex_mutable_closure_as_iterator` — `let mut result = [] / result = result.push(i)`.
2. `data_structures::test_complex_queue_via_array` — `let mut queue = [] / queue = queue.push(val)`.
3. `data_structures::test_complex_stack_push_pop` — `let mut stack = [] / stack = stack.push(val)`.
4. `multi_function::test_complex_array_zip` — `let mut result = [] / result = result.push(a[i] + b[i])`.
5. `real_world::test_program_task_list_manager` — `let mut tasks = [] / tasks = tasks.push(...)`.

### Class C: SCOPE-RECLAIM — V3-S5 ckpt-5 op_new_array(N) SURFACE (×4)

**Shared SURFACE pattern:** `Runtime error: Not implemented: op_new_array(N): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. ... REFUSED ON SIGHT: TypedArrayData resurrection under any rename (Refusal #1).`

**Dated user disposition:** 2026-05-18 — V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade (TAXONOMY explicit row; "The annotation_targets + annotations_comptime cluster IS THIS WORK"). SURFACE self-routes to ckpt-6 STRICT close (in-v0.3). **No v0.4 anchor cited.**

**Test asserts on:** user-facing semantics; stays same after fix.

1. `cross_feature::test_complex_array_of_closures` — `[|x| x+1, |x| x*2, |x| x-3]` literal.
2. `pattern_based::test_complex_enum_with_loop_accumulation` — `[Action::Add(10), ...]` enum-payload array literal (5 elements).
3. `real_world::test_program_event_emitter` — `[|d| ..., |d| ...]` closure array literal (2 elements).
4. `stress_edge_cases::test_complex_enum_dispatch_with_closures_and_loop` — `[Task::Log(...), Task::Compute(10), ...]` enum-payload array (7 elements).

### Class D: SCOPE-RECLAIM — V3-S5 ckpt-5 String.split SURFACE (×2)

**SURFACE excerpt:** `Runtime error: Not implemented: String.split: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. The deleted typed-array-data String \`Arc<Buf<Arc<String>>>\` result carrier DELETED at V3-S5 ckpt-1..ckpt-4 ...`

**Dated user disposition:** 2026-05-18 V3-S5 + 2026-05-21 "Array<string> must work" (String.split returns `Array<string>`).

1. `cross_feature::test_complex_string_split_and_process` — `"Alice,30,NYC".split(",")`.
2. `real_world::test_program_simple_tokenizer` — `input.split(delimiter)`.

### Class E: SCOPE-RECLAIM — V3-S5 ckpt-2 filter SURFACE (×2)

**SURFACE excerpt:** `Runtime error: Not implemented: filter: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1 surface. \`TypedArrayData\` enum DELETED at ckpt-1 ... UNREACHABLE until ckpt-6 STRICT close.`

**Dated user disposition:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 cascade (ckpt-2 is upstream of ckpt-6 same cascade — same row as `jit.md::jit_loop_accumulator`).

1. `pattern_based::test_complex_map_filter_chain_with_closures` — `data.map(|x| x * 2).filter(|x| x > threshold)`.
2. `pattern_based::test_complex_pipeline_with_named_functions` — same shape.

### Class F: SCOPE-RECLAIM — Status-enum Equal type mismatch (×1)

1. `stress_edge_cases::test_complex_large_program_with_everything`:
```
Semantic error: Cannot infer types for binary operation `Equal`: operand types
are `Status` and `Concrete(Reference(TypePath { segments: [\"Status\"] ...
```
Compares `self.status == Status::Active` — same surface as objects.md object_destructuring (2026-05-21 row): two `Status` paths typed differently by the inference engine (one resolved to `Status`, the other to `Concrete(Reference(TypePath{...}))`). 2026-05-21 + 2026-05-22 per-container FieldType pull-in row applies (enum-variant kind not unified to the named-type carrier).

### Class G: FN-REG-CORRECTNESS — closure-returned-from-function upvalue-frame missing (×2)

**Shared:** `Runtime error: mutable/shared capture access in a frame without upvalues (line N)`. Plausibly-correct canonical Shape (counter / accumulator pattern). Returned closure executes in frame without upvalue slot for the captured `let mut`.

Affected subsystem: closure-escape / frame-upvalue setup in `crates/shape-vm/src/executor/call_convention.rs` + §2.7.8/Q10 closure-cell path (`OwnedClosureBlock::read_capture_kinded`).

1. `data_structures::test_complex_counter_accumulator`:
```shape
fn make_counter(start) {
    let mut val = start
    let inc = || { val = val + 1; val }
    inc
}
let inc = make_counter(0); inc(); inc(); print(inc())
```
2. `cross_feature::test_complex_nested_closures_with_capture` — same shape with `let mut total = start`.

### Class H: FN-REG-CORRECTNESS — method-chain receiver-kind dispatch loss (×3)

Method dispatch fails after method-chain or recursion — receiver-kind not propagated.

1. `cross_feature::test_complex_struct_method_chain`: `Vec2 { x: 1, y: 2 }.scale(3).translate(10, 20)` → `Runtime error: no method 'translate' on receiver kind Ptr(TypedObject) (line 13)`. `.scale(...)` returns Vec2; chained `.translate(...)` dispatch can't find the `extend Vec2 { method translate(...) }` registration. Affected subsystem: `crates/shape-vm/src/executor/objects/method_registry.rs` + return-kind propagation in method-call lowering.

2. `multi_function::test_complex_recursive_factorial`: `Runtime error: no method 'mul' on receiver kind Int64 (line 4)`. Source has `n * factorial(n-1)` — the binary `*` is being dispatched as a `mul` method on Int64 instead of as a typed `MulInt` opcode. Likely a desugar/op-lowering regression where binary `*` falls through to method-dispatch for Int64. Affected subsystem: binary-op lowering for `int` operands.

3. `real_world::test_program_retry_logic`: `Runtime error: no method 'cmp' on receiver kind Int64 (line 10)`. Source: `while i < max_retries` — binary `<` dispatching as `cmp` method on Int64 instead of `LtInt` opcode. Same shape as 2.

### Class I: FN-REG-CORRECTNESS — HashMap homogeneous-value rejection (×1)

1. `stress_edge_cases::test_complex_all_features_together`:
```
Runtime error: HashMap.set(): value kind Float64 incompatible with HashMap<string, int> (line 33)
```
Source:
```shape
let receipt = HashMap()
    .set("items", items.length)   // int
    .set("total", total)           // Float64 (from price * (100 - p) / 100)
```
HashMap value-type inferred as `int` from first set, rejects `Float64` on second. Either should unify to `number` (numeric tower) OR support heterogeneous typed values (`HashMap<string, int | number>`). Plausibly-correct user-facing — config/receipt HashMaps with mixed numeric values are a canonical pattern.

### Class J: FN-REG-CORRECTNESS — silent-wrong-output (×1)

1. `multi_function::test_complex_calculator_chained_operations`: `Expected 10, got 0.0000…0005` (Float64 underflow / wrong-arithmetic). Source: `add(mul(3, 4), negate(mul(2, 1)))` = `add(12, -2)` = `10`. Returns a denormal Float64 near zero instead — silent-wrong-output is RELEASE-BLOCKING per FN-REG-CORRECTNESS taxonomy. Likely int↔number coercion path on function-return wrongly emitting a Float64 with mangled bits. Affected subsystem: function-return value kind-propagation for `negate(x) { 0 - x }` (Int64 result wrongly promoted).

### Class K: FN-REG-CORRECTNESS — `?` operator does not propagate Err (×1)

1. `cross_feature::test_complex_result_try_operator_error_propagation`:
```
Expected run error, but got: Some(Object {"Result": Object {"ok": Bool(false),
"value": Object {"String": String("negative")}}})
```
Source: `pipeline(-1)?` at top level — `pipeline` returns `Err("negative")`, the trailing `?` should propagate (program exits with error) but instead the test returns the unwrapped `Result { ok: false, value: "negative" }` as the program's success value. **`?` at module-scope does not raise to the host** — silent-wrong-output for the canonical `?` semantics. Affected subsystem: top-level `?` lowering in `crates/shape-vm/src/compiler/` (TryOp lowering at non-fn-body context).

### Class L: FN-REG-CORRECTNESS — let-shadowing/reassign diagnostic on builder pattern (×1)

1. `real_world::test_program_config_merger`:
```
Semantic error: Cannot reassign immutable variable 'defaults'. Use `let mut` or `var` for mutable bindings
```
Source:
```shape
let defaults = HashMap()...
let overrides = HashMap()...
let merged = defaults.set("port", overrides.get("port")).set("debug", overrides.get("debug"))
```
The `merged = defaults.set(...)` is a NEW binding, not a reassign — `let merged = ...` introduces a fresh name. The compiler is mistakenly treating it as reassignment of `defaults` (likely because `defaults.set(...)` returns a builder result that flows into a `let` whose name-resolution wrongly intersects `defaults`). Plausibly-correct user-facing builder pattern. Affected subsystem: name-resolution / let-binding lowering for `let X = Y.method().method()` where `Y` is immutable.

### Class M: FN-REG-CORRECTNESS — SIGABRT-class crashes (×2)

These two tests CRASH (core dump / 137-TB allocation) — SIGSEGV / SIGABRT class. **Plausibly-correct user-facing nested-struct patterns.** Affected subsystem: TypedObject nested-field-access (CLAUDE.md "Known Constraints" already names `p.addr.city` as a bug — these are the same bug class manifesting as a memory corruption).

1. `data_structures::test_complex_deep_nested_struct_access`:
```
test data_structures::test_complex_deep_nested_struct_access ... timeout: the monitored command dumped core
```
Source: 3-level nested type access `o.mid.inner.val` + `o.mid.label`. Test fixture comment already flags `// BUG: nested typed struct field access returns the inner object instead of the field`. Current symptom: core dump (escalated from wrong-value to SEGFAULT).

2. `data_structures::test_complex_nested_typed_objects`:
```
test data_structures::test_complex_nested_typed_objects ... memory allocation of 137834098343184 bytes failed
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
timeout: the monitored command dumped core
```
Source: 2-level nested type access `p.addr.city` + `p.addr.zip` (Person { addr: Address {...} }). 137-TB allocation request = uninitialized size_t read from corrupted nested-field slot. SIGABRT-class memory corruption — RELEASE-BLOCKING.

### Class N: FN-REG-CORRECTNESS — trait method signature parse rejection (×1)

1. `cross_feature::test_complex_trait_dispatch_polymorphism`:
```
Expected run ok, got error: Some("expected something else, found identifier `describe`")
```
Source:
```shape
trait Describable {
    describe(): string
}
```
Current Pest grammar requires `method describe() -> string;`. The test uses TypeScript-style `name(): Type` form which the grammar rejects. The test was authored at the initial commit and never modified — fixture's claim that `describe(): string` is parseable is the audit-truth. Either grammar accepts both shapes (correctness fix) or fixture migrates (would shift to FN-REG-DIAGNOSTIC) — pending team-lead disposition; defaulting to FN-REG-CORRECTNESS since the alternate trait-method syntax is plausibly-correct user-facing Shape (TS-style) and the failure is a hard parse rejection (not a stale diagnostic-text assertion).
Affected subsystem: `crates/shape-ast/src/shape.pest` trait_item rule.

## SIGSEGV-aggregate (full-binary run)

Class: **INFRA-FLAKY** (binary-level, not per-test)

`cargo test -p shape-test --test complex_integration --no-fail-fast` exits with signal 11 (SIGSEGV) after ~5 tests in the cross_feature group (reproduced 2026-05-27 at HEAD). Per-`--exact` runs of the same prebuilt binary do not crash — suggests parallel-test-isolation issue OR stdlib JIT-cache contention specific to multi-thread invocation. Same-shape symptom as the JIT heavy-execution-test SIGILL race noted in CLAUDE.md "Known Constraints" (`crates/shape-jit` deep-tests gating). Not blocking the per-test classifications above (per-test runs reliable).

## Passing tests (38)

```
cross_feature::test_complex_block_expressions_with_control_flow
cross_feature::test_complex_const_type_annotation_function
cross_feature::test_complex_destructuring_typed_struct
cross_feature::test_complex_enum_match_function_combo
cross_feature::test_complex_if_match_closure_return
cross_feature::test_complex_nested_match_with_guards
cross_feature::test_complex_recursive_tree_sum
cross_feature::test_complex_result_try_operator_chain
data_structures::test_complex_hashmap_key_value_store
data_structures::test_complex_hashmap_overwrite
data_structures::test_complex_linked_operations_on_typed_struct
data_structures::test_complex_set_difference_via_arrays
data_structures::test_complex_set_intersection_via_arrays
data_structures::test_complex_set_union_via_arrays
data_structures::test_complex_trait_impl_dispatch
multi_function::test_complex_binary_search
multi_function::test_complex_bubble_sort
multi_function::test_complex_collatz_steps
multi_function::test_complex_count_occurrences
multi_function::test_complex_iterative_fibonacci
multi_function::test_complex_math_library
multi_function::test_complex_multi_function_pipeline
multi_function::test_complex_recursive_sum_array
multi_function::test_complex_selection_sort
multi_function::test_complex_string_pad_left
pattern_based::test_complex_command_dispatcher
pattern_based::test_complex_enum_state_machine_with_payload
pattern_based::test_complex_fizzbuzz_match_guards
pattern_based::test_complex_option_find_in_array
pattern_based::test_complex_option_find_none
pattern_based::test_complex_state_machine_traffic_light
pattern_based::test_complex_visitor_pattern_with_match
real_world::test_program_string_builder
stress_edge_cases::test_complex_deeply_nested_expressions
stress_edge_cases::test_complex_large_string_operations
stress_edge_cases::test_complex_many_function_definitions
stress_edge_cases::test_complex_many_variables_in_scope
stress_edge_cases::test_complex_nested_control_flow
```

## UNKNOWN list

None. All 60 failing tests + 2 crashed tests classified per per-test evidence in `/tmp/cint-results.txt`.

## Note for team-lead

- **High SCOPE-RECLAIM density (50/62 = 81%)** dominated by one root-cause class: closure-param / function-param / match-bind / HashMap-value-kind not propagating element-kind through generic/method-dispatch boundaries (Class A). The defect intersects the same 2026-05-21 / 2026-05-22 per-container FieldType + Array<string> dispositions already cited in `objects.md::object_destructuring_in_function`.
- **12 FN-REG-CORRECTNESS** — two of which are SIGABRT-class memory corruption on nested TypedObject access (Class M tests, distinct from CLAUDE.md's already-noted nested-field bug in that they now CRASH rather than wrong-value).
- **Binary-level SIGSEGV** on the standard `cargo test` invocation is an INFRA finding that needs surfacing alongside the per-test classifications.
