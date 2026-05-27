# ranges classification

**HEAD:** 82f049dd
**Total tests in binary:** 16
**Passed:** 13 / Failed: 3 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test ranges --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 2 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 1 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### iteration::range_builtin_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-cascade
tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per
W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED.
... UNREACHABLE until ckpt-6 STRICT close. (line 3)
```

- Dated user disposition: 2026-05-18 (V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade — the V3-S5 family was pulled into v0.3).
- SURFACE message cites: "V3-S5 ckpt-3 consumer-cascade tier 2 surface ... UNREACHABLE until ckpt-6 STRICT close".
- Why mis-cite: per TAXONOMY 2026-05-18 row, SURFACE messages citing the V3-S5 ckpt-5/ckpt-6 cascade route to SCOPE-RECLAIM. ckpt-3 is upstream of ckpt-5/ckpt-6 in the same construction cascade; no later dated re-disposition has moved `range()` builtin to v0.4. `range()` is also a basic user-facing builtin a reasonable user expects.
- Test asserts on user-facing semantics (`.expect_number(10.0)`); test stays the same after fix.

### iteration::for_in_exclusive_range

Class: **FN-REG-CORRECTNESS**

```
Semantic error: empty array `items` has an un-resolvable element type. It is
created empty (`[]`) with no `Array<T>` annotation and is never pushed to,
so the compiler cannot prove what element type it holds.
```

- Minimal repro:
  ```shape
  let mut items = []
  for i in 0..5 { items = items.push(i) }
  items.length
  ```
- The binding IS reassigned via `items = items.push(i)` (int element). Compiler diagnostic claims "never pushed to" — it fails to see element-type evidence from reassignment with `.push(int)`. Plausibly-correct user-facing Shape.
- Affected subsystem: type inference for empty-literal `let mut` arrays under reassignment-style push (`items = items.push(x)`), in `crates/shape-runtime/src/type_system/` (empty-array element-type inference) interacting with `Array.push` PHF dispatch.
- Bisected commit: not run (audit-only).

### iteration::for_in_inclusive_range

Class: **FN-REG-CORRECTNESS**

```
Semantic error: empty array `items` has an un-resolvable element type. It is
created empty (`[]`) with no `Array<T>` annotation and is never pushed to,
so the compiler cannot prove what element type it holds.
```

- Same shape as `for_in_exclusive_range`; only the range operator differs (`0..=4` vs `0..5`). Same root cause: empty-literal `let mut items = []` + reassignment-form `items = items.push(i)` is not being credited as evidence of element type by the inference pass.
- Affected subsystem: same as above.
- Bisected commit: not run (audit-only).

## UNKNOWN list

(none)
