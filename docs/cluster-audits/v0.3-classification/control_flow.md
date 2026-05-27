# control_flow classification

**HEAD:** 82f049dd
**Total tests in binary:** 480
**Passed:** 468 / Failed: 12 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test control_flow --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 5 |
| FN-REG-DIAGNOSTIC  | 5 |
| SCOPE-RECLAIM      | 2 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### blocks::cf_03_trailing_semicolon

Class: **FN-REG-DIAGNOSTIC**

```
Expected: ()
Actual:   null
```

- Old expected text: `()` (unit literal printed).
- New actual text: `null`.
- Language change: unit value rendering changed `()` → `null`. Print is consistent; fixture stale.

### blocks::cf_03b_trailing_semicolon_detail

Class: **FN-REG-DIAGNOSTIC**

```
Expected: a=42\nb=()\nc=3\nd=()
Actual:   a=42\nb=null\nc=3\nd=null
```

- Same `()` → `null` rendering shift; fixture text stale.

### if_else::cf_37_if_no_else_false

Class: **FN-REG-DIAGNOSTIC**

```
Expected: ()
Actual:   null
```

- Same `()` → `null` rendering; fixture stale.

### loops::for_loop_building_result_array

Class: **FN-REG-CORRECTNESS**

```
Semantic error: empty array `result` has an un-resolvable element type. It is created empty (`[]`) with no `Array<T>` annotation and is never pushed to, so the compiler cannot prove what element type it holds.
```

- Minimal repro: `let result = [];` then a `for ... { result.push(x) }` loop. Compiler claims the array is never pushed-to despite the `for`-loop push. This is the named 2026-05-26 trigger pattern (`result.push()` inside a conditional not propagating element type) — element-type inference from inside loop body isn't reaching the binding. Plausibly-correct user code; release-blocker.

### loops::for_loop_empty_array

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. ... REFUSED ON SIGHT: TypedArrayData resurrection under any rename
```

- Dated user pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade.
- SURFACE explicitly self-anchors at "ckpt-5 consumer-cascade tier 3" / "rebuild lands at ckpt-6 STRICT close" — exactly the work named in the 2026-05-18 pull-in row.
- No v0.4 anchor cited; routes here.
- Test asserts on `for x in []` running ok; semantics test stays the same after fix.

### loops::for_loop_over_string_array

Class: **FN-REG-CORRECTNESS**

```
Runtime error: TypeError: expected string, got string (line 4)
```

- Minimal repro: iterating an `Array<string>` with `for s in arr`. The "expected X got X" tautology is a real bug — internal type-tag (probably StringV2 vs String) mismatch surfacing as a runtime TypeError. Possibly Array<string> 2026-05-21 user pull-in territory but the failure shape is non-SURFACE — it's an actual TypeError on plausibly-correct code. Routed CORRECTNESS rather than SCOPE-RECLAIM because the SURFACE-and-stop discipline is not honored (no structured "not implemented" message, just an unhelpful runtime TypeError that misleads diagnosis).

### loops::cf_20_for_var_mutation

Class: **FN-REG-CORRECTNESS**

```
Expected: 10\n11\n12\n13\n14
Actual:   10
```

- Minimal repro: `for x in 10..15 { print(x) }`-shape, expects 10..=14 printed. Only 10 emitted — for-loop terminates after first iteration. Plausibly-correct code; range / for-loop fundamental regression.

### loops_nested::cf_17b_for_loop_value

Class: **FN-REG-DIAGNOSTIC**

```
Expected: while false result: None
Actual:   while false result: null
```

- Same `None` → `null` print rendering shift; fixture stale.

### loops_nested::cf_21_while_loop_expression

Class: **FN-REG-DIAGNOSTIC**

```
Expected: None
Actual:   null
```

- Same `None` → `null`; fixture stale.

### stress_for_in::test_for_in_empty_array

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. ...
```

- Same as `for_loop_empty_array`: 2026-05-18 dated pull-in, op_new_array(0) SURFACE; SCOPE-RECLAIM.

### stress_match_basic::test_match_array_pattern_basic

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

- Minimal repro: match with `[a, b]` array pattern where arm-body uses `a + b`. Array pattern binding loses element type. Plausibly-correct code; pattern-binding inference regression.

### stress_match_basic::test_match_array_pattern_three_elements

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

- Same root cause: array-pattern binding doesn't expose element type to arm body; bindings collapse to `unknown`. Same disposition as `test_match_array_pattern_basic`.
