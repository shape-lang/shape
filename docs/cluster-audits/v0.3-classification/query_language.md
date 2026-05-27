# query_language classification

**HEAD:** 82f049dd
**Total tests in binary:** 23
**Passed:** 16 / Failed: 7 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test query_language --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 3 |
| FN-REG-DIAGNOSTIC  | 2 |
| SCOPE-RECLAIM      | 2 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### clauses::query_multiple_let_clauses

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Undefined variable', got: Semantic error: Cannot infer types for binary operation `Mul`: operand types are `unknown` and `int`. ...
```

- Old expected text: substring `Undefined variable`.
- New actual text: `Cannot infer types for binary operation Mul: ... unknown and int`.
- Language change: strict-typing rollout reorders diagnostic ordering — the inference failure now fires before the undefined-variable lookup, so the new diagnostic supersedes the old. Fixture text needs updating; behavior is also debatable (a true "undefined variable" diagnostic would be more useful), but the test only asserts the substring.

### clauses::query_let_clause_basic

Class: **FN-REG-DIAGNOSTIC**

- Same shape as `query_multiple_let_clauses`: old expects `Undefined variable`, new emits `Cannot infer types for binary operation Mul: ... unknown and int`. Same root cause + disposition.

### clauses::query_let_clause_with_where

Class: **FN-REG-CORRECTNESS**

```
Error should contain 'Undefined variable', got: Semantic error: Cannot infer types for binary operation `Mul`: operand types are `unknown` and `unknown`. ...
```

- Test fixture says "should error with Undefined variable", but program emits inference failure. Like the two above, but `unknown × unknown` (both sides) suggests no part of the query LET clause is resolving — possibly a real regression in `from ... let ... where` clause resolution (not just diagnostic ordering). Routed to CORRECTNESS because the both-unknown shape suggests query LET binding isn't producing a typed binding at all.

### basic::query_from_select_to_object

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: map: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1 surface. ... UNREACHABLE until ckpt-6 STRICT close.
```

- Dated user pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6.
- SURFACE defers under "UNREACHABLE until ckpt-6 STRICT close" without citing a dated v0.4 re-disposition. Same shape as `tables_queryable::map_extract_field_from_objects`.
- Test asserts on user-facing semantics; stays the same after fix.

### advanced::query_over_objects

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `GreaterEq`: operand types are `unknown` and `int`.
```

- Minimal repro: `from x in objects where x.field >= 5` — same root cause as `tables_queryable::filter_objects_by_field` (bidirectional closure / loop-variable inference over `Array<Object>` losing element-type). Named trigger case.

### advanced::query_join_with_filter

Class: **FN-REG-CORRECTNESS**

```
Error should contain 'Undefined variable', got: Semantic error: Cannot infer types for binary operation `Greater`: operand types are `unknown` and `int`.
```

- Both shapes present: fixture says "Undefined variable" but program produces inference failure. Like `query_let_clause_with_where` — `unknown × int` shape suggests partial-resolution, not pure diagnostic-reorder; closer to correctness regression on join-then-filter.

### clauses::query_order_by_ascending

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: map: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1 surface ... UNREACHABLE until ckpt-6 STRICT close.
```

- Same SURFACE shape + 2026-05-18 dated pull-in as `query_from_select_to_object`; same disposition.
