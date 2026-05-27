# stdlib_math classification

**HEAD:** 82f049dd (running at workspace HEAD 70507224 post-tag; tests were
run on the live tree per direnv-toolchain rule — same source state as 82f049dd
for these test files; no source mutations during audit)
**Total tests in binary:** 43
**Passed:** 41 / Failed: 2 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test stdlib_math --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 2 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

Closing total: 43 passed-or-classified (41 pass + 2 FN-REG-CORRECTNESS).

## Per-test classification

### statistical::manual_variance_calculation

Class: **FN-REG-CORRECTNESS**

```
thread 'statistical::manual_variance_calculation' (2919478) panicked at
tools/shape-test/src/shape_test.rs:1292:9:
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. Strict
typing requires both operands to have a known concrete type at compile
time. Add a type annotation to disambiguate.")
```

Minimal repro (3 lines, from the fixture):

```shape
let m = 5.0
let total = pow(2.0 - m, 2.0) + pow(4.0 - m, 2.0) + pow(6.0 - m, 2.0) + pow(8.0 - m, 2.0)
total / 4.0
```

Even shorter (2 lines, isolates the symptom):

```shape
let a = pow(2.0, 2.0) + pow(3.0, 2.0)
a
```

Affected stdlib symbol / compiler subsystem: the `pow` builtin call
expression carries `Type::unknown` out to the surrounding `Add` operand-
inference site. `pow` metadata declares `return_type: "number"` at
`crates/shape-runtime/src/builtin_metadata.rs:168` but the bytecode
compiler's call-expression typing path (`BuiltinFunction::Pow` arm) is not
publishing the proven `number` return into the type tracker visible to
binary-operator inference. Plausibly-correct user Shape; both operands of
`Add` are literally `pow(<f64>, <f64>)` — return type is statically
provable as `number`. This is an inference-loss regression, not a v0.4-
gated feature.

Bisect (file-level): `crates/shape-vm/src/compiler/expressions/function_calls.rs`
+ `crates/shape-vm/src/compiler/helpers.rs` were last touched by
`cb5683bb`, `50910757`, `77546a3b`, `3db17392`, `19de5ef2`, `1762758f`
(merge cluster of R8 W9 + W18 + W17). The W17.3-4 per-container FieldType
work (`d748b5b1`, `a4b38c76`) and WS-9c anonymous-object-factory
inference fix (`0abd1c2b`) are the most-likely-suspect bisect anchors —
both reshape the call-expression return-type publish path. Same root
cause as `trig::sin_squared_plus_cos_squared` below (single bug, two
fixtures).

### trig::sin_squared_plus_cos_squared

Class: **FN-REG-CORRECTNESS**

```
thread 'trig::sin_squared_plus_cos_squared' (2927396) panicked at
tools/shape-test/src/shape_test.rs:1292:9:
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. Strict
typing requires both operands to have a known concrete type at compile
time. Add a type annotation to disambiguate.")
```

Minimal repro (from the fixture):

```shape
let x = 1.23
pow(sin(x), 2.0) + pow(cos(x), 2.0)
```

Affected stdlib symbol / compiler subsystem: identical root cause to
`manual_variance_calculation`. `pow(...)` return type does not flow into
the enclosing `Add` operand-type inference, so both `pow(sin(x), 2.0)`
and `pow(cos(x), 2.0)` arrive at the `Add` as `unknown`. `sin` / `cos`
return-type publish is intact (the standalone `sin(x)` / `cos(x)` calls
inside `pow` succeed — the failure is on the outer `Add`, not on the
argument coercion). Plausibly-correct user Shape; a trig identity that
any reasonable user expects to typecheck.

Bisect: same suspect range as above (single underlying regression).

## Notes for team-lead aggregation

- Both failures collapse to **one** root cause (pow-return-type-loss into
  surrounding binary-op inference). Aggregator may want to dedupe to a
  single FN-REG-CORRECTNESS entry with two affected fixtures.
- Not SCOPE-RECLAIM: no SURFACE message fires; the compiler emits a
  user-facing semantic error that the user could not act on (no missing
  feature cited; type annotation on a `number`-returning builtin call
  shouldn't be required).
- Not V0.4-DEFER: `pow`/`sin`/`cos`/`+` are core v0.1 builtins; nothing
  about the failure surfaces as a feature-gap with a "v0.4 / planned"
  annotation.
- No `#[ignore]` tests in this binary; full corpus (43) was executed.
- Run wall-clock: 524.44s on the shared workspace (cold compile +
  per-test VM bootstrap dominates; not flaky — both failures are
  deterministic-by-source on the live tree).
- KC #2 format_* deletion (pulled-in 2026-05-22) is not implicated —
  no fixture in this binary calls `format()` / `format_*`.
- B5 stdlib distributions_advanced/ode annotations (R8 W9 close) is not
  implicated — no fixture in this binary imports those modules.
- UNKNOWN list: empty.
