# list_comprehension classification

**HEAD:** 82f049dd
**Total tests in binary:** 8
**Passed:** 7 / Failed: 1 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test list_comprehension --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 1 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### basic::comprehension_string_transform

Class: **SCOPE-RECLAIM**

```
let names = ["alice", "bob"]
let upper = [n.toUpperCase() for n in names]
upper[0]
```

Expected: `"ALICE"`. Actual:
```
Semantic error: list comprehension element type could not be determined
at compile time. Strict typing requires the element expression to have
a proven scalar type (int / number / bool / decimal / sized integer).
Annotate the comprehension's source so the element type resolves.
```

- **Dated user disposition:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade — explicitly includes W16.2-C empty-literal/spread/**comprehension**. ALSO covered by 2026-05-21 ("Array<string> must work").
- **SURFACE message text:** see above (`crates/shape-vm/src/compiler/loops.rs:1057-1067`).
- **Incorrect v0.4 anchor cited:** SURFACE does not cite v0.4 explicitly, but routes the user to "annotate the source" rather than supporting `string` element kind. The carrier-kind allowlist (int/number/bool/decimal/sized integer) excludes `string`, which is a 2026-05-21-gated type.
- **Why the cite is incorrect:** W16.2-C was pulled into v0.3 (2026-05-18 row) AND Array<string> must work (2026-05-21 row); a string-returning comprehension is squarely inside both. The compiler's `ScalarKind` allowlist needs a `String` arm (typed-array string carrier per W17.3-4 per-container FieldType, also v0.3-pulled-in 2026-05-22).
- **Test assertion shape:** asserts on **user-facing semantics** (`expect_string("ALICE")`); test stays the same after fix.

## UNKNOWN list

(none)
