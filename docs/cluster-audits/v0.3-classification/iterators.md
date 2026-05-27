# iterators classification

**HEAD:** 82f049dd
**Total tests in binary:** 165
**Passed:** 44 / Failed: 121 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test iterators --no-fail-fast 2>&1`
**Source log:** `/tmp/audit_logs/iterators.log`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 120 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 1 |

All 120 SCOPE-RECLAIM failures route to the **2026-05-18 V3-S5 ckpt-5/ckpt-6
op_new_array construction-cascade** pull-in (TAXONOMY row 1). Every test
exercises `Array.iter()` / `[].iter()` / `range(...).iter()` / chained
`.map`/`.filter`/`.reduce`/`.take`/`.skip`/`.collect`/`.foreach`/`.enumerate`/`.chain`/`.find`/`.any`/`.all`/`.count`/`.flatten`/`.flatmap`
on user-facing Shape — semantics the user would expect to work. The work was
explicitly pulled-in by the 2026-05-18 disposition.

SURFACEs partition into two structural shapes:

**(A) Direct SURFACE-firing tests (77 tests, 7 sub-buckets)** — test
asserts `Expected run ok` and a stdlib receiver-recovery handler returns the
verbatim SURFACE message. Test would need updating only if it asserts on
the exact error text (it does not — `Expected run ok`); behavioral test
stays the same after fix.

| SURFACE shape | Count | Verbatim SURFACE prefix |
|---|---|---|
| `Array.iter`            | 45 | `Runtime error: Not implemented: Array.iter: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. TypedArrayData enum DELETED at ckpt-1 (2026-05-15) ...` |
| `op_new_array(0)`       | 18 | `Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade ...` (empty-literal construction; see KC §5.16) |
| `String.iter`           | 4  | `Runtime error: Not implemented: String.iter: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface.` |
| `range`                 | 4  | `Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface.` |
| `filter`                | 3  | `Runtime error: Not implemented: filter: SURFACE — V3-S5 ckpt-2 consumer-cascade ...` |
| `op_new_typed_array(2)` | 2  | `Runtime error: Not implemented: op_new_typed_array(2): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface.` |
| `map`                   | 1  | `Runtime error: Not implemented: map: SURFACE — V3-S5 ckpt-2 consumer-cascade tier ...` |

The SURFACEs cite `V3-S5 ckpt-2 / ckpt-3 / ckpt-5 consumer-cascade` and
`ADR-006 §2.7.24 Q25.A` — by the TAXONOMY 2026-05-18 row, these cites are
**not** a v0.4 escape hatch; they map directly to v0.3-pulled-in scope. The
2026-05-18 row reads: "SURFACE messages citing 'V3-S5 ckpt-5
consumer-cascade' or '§5.16 v0.4' are SCOPE-RECLAIM by default unless
audit shows otherwise." Audit confirms default here: every failing fixture
is user-facing iterator/array semantics, not a v0.4-exclusive feature.

**(B) Closure-inference cascade (43 tests, all `Cannot infer types for
binary operation Add|Mul`)** — `.iter()` returning Unknown post-deletion
strips the element-type signal that closure-body inference downstream of
`.map(|x| ...) / .filter(|x| ...) / .reduce(|acc, x| acc + x, 0)` relies
on. Fixtures are textbook iterator pipelines (e.g. `test_filter_and_sum`:
`[1..10].iter().filter(|x| x > 5).reduce(|acc, x| acc + x, 0)`). Diagnostic
text is unrelated to V3-S5 by message (no SURFACE prefix) but root cause is
the same receiver-recovery deletion — fixed by the same ckpt-3/ckpt-6
migration. SCOPE-RECLAIM by user-pull-in.

## Per-test classification

Every SCOPE-RECLAIM row below carries: (i) pulled-in date (2026-05-18 for
all), (ii) verbatim SURFACE excerpt OR `Cannot infer` diagnostic, (iii) the
ckpt cite the SURFACE itself names (incorrect re-cite per TAXONOMY row 1),
(iv) test-asserts-on-SURFACE? = NO for all (every test asserts
`Expected run ok` — they test behavior, not error text; tests stay the
same after fix).

### SURFACE-A — `Array.iter` (45 tests)

Class: **SCOPE-RECLAIM**

Failure excerpt (representative — `stress_chaining::test_any_negative`):

```
Expected run ok, got error: Some("Runtime error: Not implemented: Array.iter:
SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. `TypedArrayData` enum
DELETED at ckpt-1 (2026-05-15) per W12-typed-array-data-deletion audit §3.5 +
ADR-006 §2.7.24 Q25.A SUPERSEDED. ... UNREACHABLE until ckpt-6 STRICT close.
REFUSED ON SIGHT: TypedArrayData resurrection under any rename ...")
```

- **Pulled-in by:** 2026-05-18 (V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade).
- **SURFACE-cited anchor:** "V3-S5 ckpt-3 consumer-cascade tier 2 surface" / "UNREACHABLE until ckpt-6 STRICT close".
- **Why cite is incorrect:** the cite tries to defer the failure beyond v0.3, but TAXONOMY 2026-05-18 row pulls V3-S5 ckpt-3/-5/-6 cascade work into v0.3. There is no later dated re-disposition to v0.4.
- **Test asserts on SURFACE?** NO — fixtures assert `Expected run ok` on behavioral output (sums, counts, found-elements, booleans). Tests stay the same after fix.

Tests (45):

```
stress_chaining::test_all_positive
stress_chaining::test_any_negative
stress_chaining::test_chain_filter_map_any
stress_chaining::test_chain_map_filter_all
stress_chaining::test_count_evens_via_iter
stress_chaining::test_find_first_greater_than
stress_chaining::test_fn_iter_chain_in_expression
stress_chaining::test_fn_with_iter_pipeline
stress_chaining::test_iter_filter_then_count
stress_chaining::test_iter_filter_then_find
stress_chaining::test_iter_map_filter_take_count
stress_chaining::test_iter_map_then_all
stress_chaining::test_iter_map_then_any
stress_chaining::test_iter_skip_one
stress_chaining::test_iter_take_one
stress_map_filter::test_array_iter_to_array
stress_map_filter::test_iter_count
stress_map_filter::test_iter_filter_keep_all
stress_map_filter::test_iter_filter_keep_none
stress_map_filter::test_iter_map_identity
stress_map_filter::test_iter_skip_all
stress_map_filter::test_iter_skip_basic
stress_map_filter::test_iter_skip_more_than_available
stress_map_filter::test_iter_skip_then_take_then_count
stress_map_filter::test_iter_skip_zero
stress_map_filter::test_iter_take_all
stress_map_filter::test_iter_take_basic
stress_map_filter::test_iter_take_more_than_available
stress_map_filter::test_iter_take_zero
stress_map_filter::test_single_element_array_iter_collect
stress_reduce_collect::test_direct_filter_vs_iter_filter
stress_reduce_collect::test_direct_map_vs_iter_map
stress_reduce_collect::test_iter_all_false
stress_reduce_collect::test_iter_all_true
stress_reduce_collect::test_iter_any_false
stress_reduce_collect::test_iter_any_true
stress_reduce_collect::test_iter_chain_then_count
stress_reduce_collect::test_iter_chain_with_empty
stress_reduce_collect::test_iter_enumerate_count
stress_reduce_collect::test_iter_enumerate_take
stress_reduce_collect::test_iter_find_first_element
stress_reduce_collect::test_iter_find_found
stress_reduce_collect::test_iter_find_not_found
stress_reduce_collect::test_single_element_iter_all_operations
stress_reduce_collect::test_single_element_iter_any_check
```

### SURFACE-B — `op_new_array(0)` (18 tests)

Class: **SCOPE-RECLAIM**

Failure excerpt (representative — `stress_map_filter::test_array_filter_empty`):

```
Expected run ok, got error: Some("Runtime error: Not implemented: op_new_array(0):
SURFACE — V3-S5 ckpt-5 consumer-cascade ...")
```

- **Pulled-in by:** 2026-05-18 (explicitly names V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade — this is the named SURFACE).
- **SURFACE-cited anchor:** "V3-S5 ckpt-5 consumer-cascade" — the exact phrase the TAXONOMY 2026-05-18 row routes to SCOPE-RECLAIM.
- **Why cite is incorrect:** taxonomy default.
- **Test asserts on SURFACE?** NO — empty-array construction with downstream iter ops; tests assert behavioral output.

Tests (18):

```
stress_map_filter::test_array_filter_empty
stress_map_filter::test_array_map_empty
stress_map_filter::test_empty_array_iter_collect
stress_map_filter::test_empty_iter_count
stress_map_filter::test_iter_filter_empty_source
stress_map_filter::test_iter_map_empty
stress_reduce_collect::test_for_in_empty_array
stress_reduce_collect::test_iter_all_empty
stress_reduce_collect::test_iter_all_from_empty
stress_reduce_collect::test_iter_any_empty
stress_reduce_collect::test_iter_any_from_empty
stress_reduce_collect::test_iter_chain_empty_with_nonempty
stress_reduce_collect::test_iter_enumerate_empty
stress_reduce_collect::test_iter_enumerate_from_empty
stress_reduce_collect::test_iter_filter_from_empty
stress_reduce_collect::test_iter_find_from_empty
stress_reduce_collect::test_iter_skip_from_empty
stress_reduce_collect::test_iter_take_from_empty
```

### SURFACE-C — `String.iter` (4 tests)

Class: **SCOPE-RECLAIM**

Failure excerpt:

```
Expected run ok, got error: Some("Runtime error: Not implemented: String.iter:
SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface.")
```

- **Pulled-in by:** 2026-05-18 (V3-S5 ckpt-3/-5/-6 construction-cascade family — the receiver-shape migration covers String.iter as the sibling carrier).
- **SURFACE-cited anchor:** "V3-S5 ckpt-3 consumer-cascade tier 2 surface".
- **Why cite is incorrect:** TAXONOMY 2026-05-18 + 2026-05-21 ("Array<string> must work") together cover string-iter user semantics; no later re-disposition to v0.4.
- **Test asserts on SURFACE?** NO.

Tests:

```
stress_reduce_collect::test_empty_string_iter
stress_reduce_collect::test_string_iter_collect_via_source
stress_reduce_collect::test_string_iter_skip
stress_reduce_collect::test_string_iter_take
```

### SURFACE-D — `range` (4 tests)

Class: **SCOPE-RECLAIM**

Failure excerpt:

```
Expected run ok, got error: Some("Runtime error: Not implemented: range:
SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface.")
```

- **Pulled-in by:** 2026-05-18 (range iter is the same receiver-recovery shape).
- **SURFACE-cited anchor:** "V3-S5 ckpt-3 consumer-cascade tier 2 surface".
- **Why cite is incorrect:** taxonomy default.
- **Test asserts on SURFACE?** NO — `for-in` and large-array reduce ops.

Tests:

```
stress_reduce_collect::test_for_in_range
stress_reduce_collect::test_iter_large_array_count
stress_reduce_collect::test_iter_large_array_filter_count
stress_reduce_collect::test_iter_large_array_map_take_collect
```

### SURFACE-E — `filter` (3 tests)

Class: **SCOPE-RECLAIM**

Failure excerpt:

```
Expected run ok, got error: Some("Runtime error: Not implemented: filter:
SURFACE — V3-S5 ckpt-2 consumer-cascade ...")
```

- **Pulled-in by:** 2026-05-18.
- **SURFACE-cited anchor:** "V3-S5 ckpt-2 consumer-cascade tier".
- **Why cite is incorrect:** taxonomy default — ckpt-2/-3/-5/-6 are the same cascade family.
- **Test asserts on SURFACE?** NO.

Tests:

```
stress_map_filter::test_array_filter_basic
stress_map_filter::test_array_map_then_filter
stress_reduce_collect::test_for_in_filtered_array
```

### SURFACE-F — `op_new_typed_array(2)` (2 tests)

Class: **SCOPE-RECLAIM**

Failure excerpt:

```
Expected run ok, got error: Some("Runtime error: Not implemented:
op_new_typed_array(2): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3
surface. ... Construction-site rebuild lands at ckpt-6 STRICT close ...")
```

- **Pulled-in by:** 2026-05-18 (explicit V3-S5 ckpt-5/ckpt-6 op_new_typed_array).
- **SURFACE-cited anchor:** "V3-S5 ckpt-5 consumer-cascade tier 3 surface".
- **Why cite is incorrect:** the cited ckpt-5/-6 work IS pulled-in.
- **Test asserts on SURFACE?** NO.

Tests:

```
stress_chaining::test_array_flatten
stress_map_filter::test_array_flatmap_basic
```

### SURFACE-G — `map` (1 test)

Class: **SCOPE-RECLAIM**

Failure excerpt:

```
Expected run ok, got error: Some("Runtime error: Not implemented: map:
SURFACE — V3-S5 ckpt-2 consumer-cascade tier ...")
```

- **Pulled-in by:** 2026-05-18.
- **SURFACE-cited anchor:** "V3-S5 ckpt-2 consumer-cascade".
- **Why cite is incorrect:** taxonomy default.
- **Test asserts on SURFACE?** NO.

Tests:

```
stress_reduce_collect::test_reuse_array_for_multiple_operations
```

### INFER-cascade — `Cannot infer types for binary operation` (43 tests)

Class: **SCOPE-RECLAIM**

Failure excerpt (representative — `stress_chaining::test_filter_and_sum`):

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. Strict
typing requires both operands to have a known concrete type at compile time.
Add a type annotation to disambiguate.")
```

Fixture:

```shape
[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    .iter()
    .filter(|x| x > 5)
    .reduce(|acc, x| acc + x, 0)
```

- **Pulled-in by:** 2026-05-18 (root cause is same V3-S5 receiver-recovery migration — `.iter()` returns Unknown post-`TypedArrayData` deletion, so the bidirectional closure inference downstream of `.filter(|x| ...)` / `.reduce(|acc, x| ...)` loses the element-type signal needed for `acc + x`).
- **SURFACE-cited anchor:** none — diagnostic does not surface the V3-S5 anchor (the missing-anchor itself is a v0.3-pulled-in defect: ckpt-3 carrier deletion silently breaks element-type flow without the SURFACE telling the user about the upstream cascade). Not v0.4 territory.
- **Why cite is incorrect:** no cite to refute; defaults to SCOPE-RECLAIM under 2026-05-18 row because the underlying receiver-recovery work is pulled-in.
- **Test asserts on SURFACE?** NO — every fixture is a user-facing iterator pipeline with arithmetic on numeric element types. Tests stay the same after fix.

Tests (43):

```
stress_chaining::test_chain_map_filter_find
stress_chaining::test_complex_data_transformation
stress_chaining::test_complex_iter_pipeline
stress_chaining::test_complex_pipeline_pattern
stress_chaining::test_double_and_take_first_three
stress_chaining::test_filter_and_sum
stress_chaining::test_flatten_then_filter
stress_chaining::test_fn_returning_iter_result
stress_chaining::test_iter_filter_filter_collect
stress_chaining::test_iter_filter_map_reduce_chain
stress_chaining::test_iter_filter_map_take
stress_chaining::test_iter_filter_modulo
stress_chaining::test_iter_filter_skip_take_collect
stress_chaining::test_iter_filter_then_map
stress_chaining::test_iter_map_constant_value
stress_chaining::test_iter_map_map_collect
stress_chaining::test_iter_map_negative_values
stress_chaining::test_iter_map_then_filter
stress_chaining::test_iter_map_then_reduce
stress_chaining::test_iter_map_with_arithmetic
stress_chaining::test_iter_map_with_captured_in_iter_chain
stress_chaining::test_iter_map_with_captured_variable
stress_chaining::test_iter_reduce_with_captured_var
stress_chaining::test_iter_skip_filter_map_collect
stress_chaining::test_nested_array_map_inner
stress_chaining::test_nested_map_flatten
stress_chaining::test_sum_of_squares_via_iter
stress_map_filter::test_array_iter_collect_identity
stress_map_filter::test_array_reduce_empty
stress_map_filter::test_iter_filter_collect
stress_map_filter::test_iter_filter_even_numbers
stress_map_filter::test_iter_map_collect
stress_map_filter::test_iter_skip_then_take
stress_map_filter::test_iter_take_then_skip
stress_reduce_collect::test_array_take
stress_reduce_collect::test_direct_reduce_vs_iter_reduce
stress_reduce_collect::test_iter_chain_two_arrays
stress_reduce_collect::test_iter_enumerate_collect
stress_reduce_collect::test_iter_reduce_empty
stress_reduce_collect::test_iter_reduce_from_empty
stress_reduce_collect::test_iter_reduce_product
stress_reduce_collect::test_iter_reduce_sum
stress_reduce_collect::test_multiple_terminals_same_source
```

### UNKNOWN — `test_array_foreach` (1 test)

Class: **UNKNOWN**

Failure excerpt:

```
thread 'stress_reduce_collect::test_array_foreach' (...) panicked at
tools/shape-test/src/shape_test.rs:1292:9:
Expected run ok, got error: Some("Runtime error: Undefined variable: total.
Variable names resolve from local scope and module scope.")
```

Fixture:

```shape
fn test() -> int {
    let mut total = 0
    [1, 2, 3].forEach(|x| { total = total + x })
    total
}
test()
```

- **What blocks classification:** the diagnostic is `Undefined variable: total` at runtime, not a SURFACE message and not the closure-inference cascade. Closure mutating-capture on a stack-local `let mut` should resolve in the closure scope. This is plausibly FN-REG-CORRECTNESS (closure-capture analysis regression on `forEach`) **or** plausibly a downstream effect of the same V3-S5 receiver-recovery migration leaking into the closure-capture path (in which case SCOPE-RECLAIM). Diagnostic does not cite either anchor, and the failure shape (runtime undefined-var, not compile-time inference loss, not SURFACE) does not match either cluster cleanly.
- **Recommended next-step:** quick per-test bisect on `closure_analysis.rs` / `mir/storage_planning.rs` around `forEach` dispatch — owner: closure-capture/borrow-solver familiar agent; depth: ~30 minutes of investigation, no source change needed at audit time.

## Notes on `V2 bytecode verification warning` lines

A handful of failures (e.g. `test_array_flatten`, `test_fn_with_iter_pipeline`,
`test_fn_iter_chain_in_expression`) print extra `V2 bytecode verification
warning: N violation(s) found — V2 typed opcode NewTypedArrayI64 / ArrayLenTyped /
TypedArrayPushI64 has no FrameDescriptor` lines BEFORE the SURFACE panic.
These are pre-panic diagnostic warnings from the same V3-S5 construction
cascade (FrameDescriptor + typed-array op rebuild) and do NOT add new
classes — the actual failure is still the SURFACE / inference-cascade
already classified above.
