# book_doctests classification

**HEAD:** 82f049dd
**Total tests in binary:** 3
**Passed:** 1 / Failed: 2 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test book_doctests --no-fail-fast 2>&1`

## Run result

```
running 3 tests
test book_snippets_run_ok ... FAILED
test book_snippets_expected_output ... FAILED
test book_snippets_lsp_ok ... ok

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2627.80s
```

Both failures are aggregate tests that iterate over all book snippets and
fail when any individual snippet fails. The same TWO underlying snippet
files drive both failures:

- `shape-web/book/snippets/fundamentals/destructure_array.shape`
- `shape-web/book/snippets/fundamentals/destructure_object.shape`

The other 6 snippets (`hello`, `arithmetic_ops`, `comparison_ops`,
`function_add`, `function_double`, `variables_let`) all run-ok AND match
expected output. `book_snippets_lsp_ok` (semantic-tokens-only, no execute)
passes for all 8 snippets.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 2 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

The two FAILED rust tests roll up the same 2 snippet failures, so the
classification is per-snippet-failure (not per-rust-test). Both rust-test
failures route to the same 2 FN-REG-CORRECTNESS entries below.

## Per-test classification

### book_snippets_run_ok / book_snippets_expected_output — destructure_array.shape

Class: **FN-REG-CORRECTNESS**

```
thread 'book_snippets_run_ok' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Semantic error: Cannot infer types
for binary operation `Add`: operand types are `unknown` and `unknown`.
Strict typing requires both operands to have a known concrete type at
compile time. Add a type annotation to disambiguate.")
```

**Minimal repro** (`shape-web/book/snippets/fundamentals/destructure_array.shape`,
verbatim — 5 lines, book-documented as runnable with expected output `30`):

```shape
fn sum_pair([a, b]) {
    return a + b;
}

print(sum_pair([10, 20]))  // 30
```

**Affected subsystem:** parameter-pattern type-inference. The array-pattern
destructure binder `[a, b]` lands `a` and `b` as `unknown` typed locals
into the function body; `a + b` then trips the strict-typing
`Cannot infer types for binary operation Add` semantic check (the §Forbidden
strict-typing rule: no dynamic fallback, no `Any` escape hatch).

**Bisected regression commit:** not run (audit-only — no bisect). Sibling
commit `dcc2d104 Lower destructuring bindings into MIR` lowers
destructuring into MIR; W15.2-F shape-web book pass (commit `1254ac1a`
merge note) explicitly documents "bare destructure-param inference fail"
as a known v0.3 HEAD bug observed on the functions.mdx pass. This snippet
is the book-side manifestation of that documented bug.

Test asserts on the snippet executing successfully + producing `30`. Test
stays the same after fix (user-facing semantics, not a SURFACE assertion).

### book_snippets_run_ok / book_snippets_expected_output — destructure_object.shape

Class: **FN-REG-CORRECTNESS**

```
thread 'book_snippets_run_ok' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Semantic error: Cannot infer types
for binary operation `Mul`: operand types are `unknown` and `unknown`.
Strict typing requires both operands to have a known concrete type at
compile time. Add a type annotation to disambiguate.")
```

**Minimal repro** (`shape-web/book/snippets/fundamentals/destructure_object.shape`,
verbatim — 5 lines, book-documented as runnable with expected output `5.0`):

```shape
fn distance({x, y}) {
    return (x * x + y * y) ** 0.5;
}

print(distance({x: 3, y: 4}))  // 5
```

**Affected subsystem:** parameter-pattern type-inference (object variant).
Same root cause as `destructure_array` — the object-pattern destructure
binder `{x, y}` lands `x` and `y` as `unknown`; `x * x` trips strict-typing
`Cannot infer types for binary operation Mul`.

**Bisected regression commit:** not run (audit-only). Prior v0.3-targeted
fix commits `fa3d38cb fix(patterns): WS-4 — object destructuring fully
works in v0.3 (4a/4b/4c)` + `a67f2c79 Merge r6-ws4-destructure-fix`
landed object-destructuring work earlier in the v0.3 cycle. W15.2-D
shape-web pass (commit `6faf6f87` merge note) documents
"VM destructure {x,y}=obj Phase-2c TypedObject-exception (JIT works)"
divergence — a related but distinct destructure failure mode (let-binding
form, VM-only). The book-snippet failure here is the parameter-binding
form and fires under the bytecode/semantic-error path, not the VM-only
TypedObject path.

**Note on borderline SCOPE-RECLAIM disposition:** the 2026-05-21 dated
user disposition includes "Object destructuring must fully work" as
v0.3-gating, which would normally point at SCOPE-RECLAIM. However, the
taxonomy reserves SCOPE-RECLAIM for failures whose **SURFACE message
cites v0.4 / §5.16** (mis-cite reclassification). This failure emits a
plain `Semantic error: Cannot infer types ...` with NO v0.4 cite, so
it does not match the SCOPE-RECLAIM trigger. Routed FN-REG-CORRECTNESS
per the literal taxonomy rule; team-lead may consider whether to widen
SCOPE-RECLAIM at aggregation time given the 2026-05-21 named-scope
match.

Test asserts on the snippet executing successfully + producing `5.0`.
Test stays the same after fix.

## Notes on the 6 passing snippets

For completeness, the snippets that PASS `run_ok` + `expected_output`:

1. `getting-started/hello.shape` — `print("Hello, Shape!")` → `Hello, Shape!`.
2. `fundamentals/variables_let.shape` — `let` + f-string interpolation → `100.5 ES`.
3. `fundamentals/arithmetic_ops.shape` — `let a = 10; let b = 3;` then 5
   arithmetic operators. Expected output uses int-division `10/3 → 3` even
   though the source comment says `// 3.33...` — current runtime matches
   the expected file (int / int = int), not the comment.
4. `fundamentals/comparison_ops.shape` — 6 comparison operators, all `true`.
5. `fundamentals/function_add.shape` — `fn add_nums(a, b) { return a + b; }`
   called with `(10, 5)`. Passes because the callsite supplies concrete
   `int` arguments that propagate through to the unannotated params
   (`f09acf8e fix(inference): transitive callsite type propagation for
   nested unannotated fns`). The destructure failures above do NOT benefit
   from this path because the param-pattern binder loses the propagated
   element/field type before the body sees the binders.
6. `fundamentals/function_double.shape` — `fn double(x) { return x * 2; }`
   with `// ANCHOR:` comments. Same callsite-propagation path as #5.
