# literals classification

**HEAD:** 82f049dd
**Total tests in binary:** 184
**Passed:** 182 / Failed: 2 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test literals --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 1 |
| FN-REG-DIAGNOSTIC  | 1 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### stress_booleans_none::test_empty_string_is_truthy

Class: **FN-REG-CORRECTNESS**

```
assertion `left == right` failed: Expected false, got true
  left: true
 right: false
```

- Minimal repro: `if "" { ... }` — empty string is being treated as truthy. Test expected `false`, runtime says `true`.
- Affected subsystem: VM truthiness — empty-string truthiness inversion. Other truthiness tests (`test_null_is_not_truthy`, `test_int_zero_is_not_truthy`, `test_nonempty_string_is_truthy`) pass, so the regression is narrow to the empty-string case.
- Bisect: not run (audit-only).

### bool_none::none_literal

Class: **FN-REG-DIAGNOSTIC**

```
assertion `left == right` failed: Output mismatch.
Expected:
None
Actual:
null
  left: "null"
 right: "None"
```

- Old expected output: `None` (printed when a bare `None` / `null` literal is the program tail).
- New actual output: `null`.
- Language change: print/format of the `null` / Option-None scalar now emits `null` instead of `None`. Recurring fixture pattern — appears 3x in control_flow (`cf_03_trailing_semicolon`, `cf_37_if_no_else_false`, `cf_17b_for_loop_value`, `cf_21_while_loop_expression` also use `()` vs `null`). The runtime print format is consistent and arguably correct (`null` is the language's terminology, `None` is Rust's); fixtures need updating.
