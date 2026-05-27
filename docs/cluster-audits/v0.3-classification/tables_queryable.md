# tables_queryable classification

**HEAD:** 82f049dd
**Total tests in binary:** 11
**Passed:** 9 / Failed: 2 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test tables_queryable --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 1 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 1 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### table_methods::filter_objects_by_field

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Semantic error: Cannot infer types for binary operation `Greater`: operand types are `unknown` and `int`. Strict typing requires both operands to have a known concrete type at compile time. Add a type annotation to disambiguate.
```

- Minimal repro: filtering a `Array<{ x: int, ... }>` by `obj.x > 5` — closure parameter type not being bidirectionally inferred from the array element type. This is the exact pattern named in the 2026-05-26 user trigger (`filter on Array<User>`).
- Bisect: not run.
- Affected subsystem: bidirectional closure-parameter inference for `.filter` on `Array<Object>`. Plausibly-correct user-facing code that previously worked; this is the named v0.3.2 trigger surface.

### table_methods::map_extract_field_from_objects

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: map: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) ... UNREACHABLE until ckpt-6 STRICT close.
```

- Dated user pull-in: 2026-05-18 — V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade.
- SURFACE text: "V3-S5 ckpt-2 consumer-cascade tier 1 surface ... UNREACHABLE until ckpt-6 STRICT close".
- Incorrect anchor: SURFACE does not cite v0.4 by name, but defers under "UNREACHABLE until ckpt-6 STRICT close"; ckpt-6 is in 2026-05-18 v0.3 pull-in.
- Test asserts on user-facing semantics (`.map` returning a transformed array); stays the same after fix.
