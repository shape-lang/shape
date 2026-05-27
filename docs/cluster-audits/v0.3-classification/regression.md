# regression classification

**HEAD:** 82f049dd
**Total tests in binary:** ~114 (truncated mid-run by SIGABRT)
**Passed:** ~71 / Failed: 42 / SIGABRT: 1 (cascade-aborted remaining tests after `qa::regression_crit_1_nested_property_access`)
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test regression --no-fail-fast 2>&1`
**Evidence log:** `/tmp/regression_out.txt` (canonical; `/tmp/audit_logs/regression.log` is the first ~76 lines)

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 6 |
| FN-REG-DIAGNOSTIC  | 36 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

**Classification driver.** 36 of 42 failures share one shape: the test helper
`jit_expect_number` (`tools/shape-test/tests/regression/jit.rs:26-38`)
requires `WireValue::Number(_)` but the source code under test uses
integer literals (`10 + 5`, `if 10 > 5 { 1 } else { 0 }`, `while i < 10 { ... }`)
which now produce `WireValue::Integer(_)` after the int/number split.
Both `jit_floating_point_precision` (uses `0.1 + 0.2`) and
`jit_float_loop_mixed_bound_comparison` (uses `0.0` / `1.0`) PASS — clean
diagnostic-only delta.

The remaining 6 are real correctness regressions: one SIGABRT
(134 TB malloc on `cfg.server.host`), one Result-trampoline divergence
(string trampoline twin passes; Result twin fails), and four
language-surface regressions (expression-target annotations, trait-bound
dispatch, Err-context Result inference, comptime-removed diagnostic
text).

## Per-test classification

### jit::jit_add, jit_sub, jit_mul, jit_div, jit_mod

Class: **FN-REG-DIAGNOSTIC**

```rust
fn jit_expect_number(source: &str, expected: f64) {
    match jit_eval(source) {
        WireValue::Number(n) => { ... }
        other => panic!("Expected Number({}), got {:?}", expected, other),
    }
}

// e.g. jit_add: jit_expect_number("10 + 5", 15.0);
```

- Old expected: source `"10 + 5"` returned `WireValue::Number(15.0)`.
- New actual: source `"10 + 5"` returns `WireValue::Integer(15)`.
- Language change: int/number split — integer-literal arithmetic stays in
  `int` (no implicit number coercion per CLAUDE.md type-system rule
  "int and number are separate"). Fix: helper relaxed to accept Integer,
  or fixtures use `10.0 + 5.0`.

### jit::jit_local_variables, jit_variable_reassignment

Class: **FN-REG-DIAGNOSTIC**

```rust
jit_expect_number("let x = 10\nlet y = 20\nx + y", 30.0);
jit_expect_number("let mut x = 1\nx = x + 1\nx = x + 1\nx", 3.0);
```

Same shape — `int` literals → `WireValue::Integer`, helper expects `Number`.

### jit::jit_comparison_gt, jit_comparison_lt, jit_comparison_eq, jit_comparison_neq, jit_comparison_gte_lte

Class: **FN-REG-DIAGNOSTIC**

```rust
jit_expect_number("if 10 > 5 { 1 } else { 0 }", 1.0);
```

`if`-branches return int-literal `1` / `0`. Same Integer-vs-Number shape.

### jit::jit_if_else

Class: **FN-REG-DIAGNOSTIC**

```rust
jit_expect_number("if true { 1 } else { 2 }", 1.0);
```

Same: branch values are int literals.

### jit::jit_while_loop, jit_while_sum_to_100, jit_nested_loop_comparison, jit_loop_comparison_fused

Class: **FN-REG-DIAGNOSTIC**

```rust
jit_expect_number("let mut x = 0\nlet mut i = 0\nwhile i < 10 { x = x + i\ni = i + 1 }\nx", 45.0);
```

Loop accumulators are int-typed; result `WireValue::Integer(45)`,
helper expects `Number`.

### jit::jit_function_call, jit_recursive_fibonacci

Class: **FN-REG-DIAGNOSTIC**

```rust
jit_expect_number("function double(n) { return n * 2 }\ndouble(21)", 42.0);
jit_expect_number("function fib(n) { if n < 2 { return n } ... }\nfib(20)", 6765.0);
```

Same int-vs-number split. `n * 2` stays in `int`.

### jit::jit_array_create_and_access, jit_array_length, jit_array_mutation_via_function

Class: **FN-REG-DIAGNOSTIC**

```rust
jit_expect_number("let arr = [10, 20, 30]\narr[1]", 20.0);
jit_expect_number("let arr = [1, 2, 3, 4, 5]\narr.length", 5.0);
```

`Array<int>` element access + `.length` return Integer; expected Number.

### jit::jit_array_push_via_function

Class: **FN-REG-DIAGNOSTIC** (negative-form variant)

Test is `#[should_panic(expected = "Expected 3")]` — was anchored to
the legacy panic message format. Helper now panics with
`Expected Number(3), got Integer(3)` (different substring), so the
should_panic match fails and the test reports FAILED. Fix is the same
helper relax as for the positive-form tests.

### jit::jit_large_number_arithmetic, jit_floating_point_precision

`jit_floating_point_precision` PASSES (uses `0.1 + 0.2`).
`jit_large_number_arithmetic` FAILS:

Class: **FN-REG-DIAGNOSTIC**

```rust
jit_expect_number("1000000 * 1000000", 1e12);
```

Int literals; product stays in `int` (1e12 fits in i64). Same shape.

### jit::jit_ackermann, jit_fib_iterative, jit_collatz, jit_matrix_mul_small, jit_sieve_small

Class: **FN-REG-DIAGNOSTIC**

All use int-typed source bodies (recursion / iterative loops / array
indexing with int counters) and return ints. Helper expects Number.

### jit::jit_mandelbrot_mixed_numeric_loop_regression

Class: **FN-REG-DIAGNOSTIC** (negative-form variant)

Test is `#[should_panic(expected = "Expected 5739")]`. Body mixes
`2.0 * x / size` so the eventual `count` is int; helper panics with
`Expected Number(5739), got Integer(5739)`. should_panic substring
mismatch → FAILED. Same shape as `jit_array_push_via_function`.

### jit::jit_int_unboxing_sum_local, jit_int_unboxing_sum_module_binding, jit_int_unboxing_nested_loops, jit_int_unboxing_fib_swap, jit_int_unboxing_mixed_local_types, jit_int_unboxing_nested_module_bindings, jit_int_unboxing_large_result

Class: **FN-REG-DIAGNOSTIC**

Seven int-unboxing JIT regression tests — by their very nature these are
int-typed bodies (the whole point is to exercise the int unboxing path).
Source returns int; helper expects Number. Same shape.

### jit::jit_trampoline_result_callvalue

Class: **FN-REG-CORRECTNESS**

```rust
fn make_ok() -> Result<int, string> { return Ok(42) }
fn call_it(f) -> int { let val = f()?; return val }
call_it(make_ok)  // jit_expect_number(..., 42.0)
```

The string twin (`jit_trampoline_string_callvalue`) PASSES — so this is
not the diagnostic Integer/Number shape. The Result-trampoline conversion
from VM Arc<HeapValue> Result bits to JIT format is wrong (or `?` on the
trampoline path returns the wrong slot). Affected: JIT trampoline /
CallValue path for `Result<int, E>` returns. Subsystem: shape-jit
trampoline VM→JIT format conversion (see comment at `jit.rs:662-679`).

- Minimal repro: the 5-line snippet above.
- Affected: shape-jit CallValue trampoline + `?` lowering on Result int.
- Bisect: needs `git log --oneline -- crates/shape-jit/src/` since v0.3.2
  trampoline work — not bisected in this audit (audit-only, no cargo runs).

### language_surface::comptime_type_info_is_removed_and_not_suggested_by_lsp

Class: **FN-REG-CORRECTNESS**

```rust
ShapeTest::new(code)
    .expect_semantic_diagnostic_contains("type_info has been removed")
    .at(pos(2, 12))
    .expect_no_completion("type_info");
```

Test asserts diagnostic text **for an intentionally-removed comptime
builtin** (`type_info`). If the diagnostic substring no longer appears,
either (a) the removal regressed and the builtin is silently allowed
again (CORRECTNESS), or (b) the message text drifted (DIAGNOSTIC).
Without diagnostic-output capture in this log, the conservative routing
is CORRECTNESS — the regression test exists precisely to catch the
re-introduction case, and a passing-then-failing transition on this
specific test post-v0.3.0 needs source-level confirmation before
demoting to DIAGNOSTIC.

- Minimal repro: the 4-line `comptime { let info = type_info("Point") }`
  snippet at `language_surface.rs:33-37`.
- Affected: comptime builtin gating in `shape-runtime` + LSP completion
  filter; KC #2 (`format_*` deletion, 2026-05-22) is the analogous
  precedent.

### language_surface::trait_bound_method_dispatch_resolves_at_runtime

Class: **FN-REG-CORRECTNESS**

```rust
trait Displayable { display(): string }
type User { name: string }
impl Displayable for User { method display() { "user:" + self.name } }
fn render<T: Displayable>(value: T) -> string { return value.display() }
print(render(User { name: "Ada" }))
// expect_output("user:Ada")
```

User-pull-in 2026-05-22 ("Comptime trait into v0.3"). This is the
canonical trait-bound dispatch through a generic function pattern —
exactly the user-facing shape the 2026-05-26 audit was triggered to
protect. The test asserts on user-facing semantics; if `render` no
longer resolves `T::display()` at the call boundary, this is a
correctness regression.

- Minimal repro: the 10-line snippet above.
- Affected: generic method dispatch + trait-bound resolution
  (`shape-runtime/src/type_system/` + dispatch).

### language_surface::expression_annotation_before_after_hooks_execute

Class: **FN-REG-CORRECTNESS**

```rust
annotation trace_expr() {
  targets: [expression]
  before(args, ctx) { print("before"); args }
  after(args, result, ctx) { print("after"); result }
}
let x = @trace_expr() (1 + 2)
print(x)
// expect_output("before\nafter\n3")
```

Expression-target annotations + before/after hooks. This is the
annotation_targets / annotations_comptime cluster called out in the
SCOPE-RECLAIM 2026-05-18 disposition row. But this test asserts on
user-facing semantics (output equals `"before\nafter\n3"`), the
annotation system is shipped, and the failure shape is functional
(not a SURFACE cite). Routes to CORRECTNESS, not SCOPE-RECLAIM.

- Minimal repro: the 10-line snippet above.
- Affected: annotation engine, expression-target hook execution.

### language_surface::declared_result_return_type_accepts_err_context_without_spurious_generic_mismatch

Class: **FN-REG-CORRECTNESS**

```rust
fn test() -> Result<int> {
  return Err("some error") !! "yes, something went wrong"
}
test()?
// expect_run_err_contains("yes, something went wrong")
```

`Result<int>` declared with `Err(...) !! "..."` context. Test asserts
no spurious generic mismatch + that the error context surfaces at
runtime. Failure means either the compiler rejected the program (the
"spurious generic mismatch" the test name guards against returned) or
the context string was lost. Either way, user-facing correctness.

- Minimal repro: the 4-line snippet above.
- Affected: Result generic inference + `!!` error-context lowering.

### qa::regression_crit_1_nested_property_access

Class: **FN-REG-CORRECTNESS** (SIGABRT — release-blocking)

```rust
type Server { host: string, port: int }
type Config { server: Server, debug: bool }
let cfg = Config { server: Server { host: "localhost", port: 8080 }, debug: false }
print(cfg.server.host)
// expect_output_contains("localhost")
```

Test output (raw from the log):
```
test qa::regression_crit_1_nested_property_access ... memory allocation of 134377994761120 bytes failed
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
Caused by:
  process didn't exit successfully: ... (signal: 6, SIGABRT: process abort signal)
```

The 134 TB allocation is a classic pointer-bits-as-length silent-wrong-
output bug — `cfg.server.host` (a nested TypedObject field access)
returns raw pointer bits where a length / size was expected, and the
allocator interprets the pointer as a `usize` count. Crashed the test
binary so all subsequent tests in the file did not run (cascade abort).

The companion `regression_crit_1_deep_nested_access` is
`#[should_panic]`'d and PASSES (3-level nesting panics as expected),
which confirms 2-level nested access has a distinct failure mode from
the 3-level case. CRIT-1 was already filed as a known nesting bug;
this is the 2-level surface of the same root cause.

- Minimal repro: the 4-line snippet above.
- Affected: nested TypedObject field-load codegen — `cfg.server` returns
  a value whose subsequent `.host` access decodes incorrectly; v2-raw-
  heap-audit / typed-field-access lowering.
- Bisect: candidates are post-W17 typed-carrier monomorphization
  commits in `crates/shape-vm/src/executor/` and the print-path
  marshal in `shape-runtime/src/marshal.rs`. Not bisected here.

---

## Notes

- No SCOPE-RECLAIM entries. The `expression_annotation_before_after_hooks_execute`
  candidate routes to CORRECTNESS per the §SCOPE-RECLAIM rule: the
  failure shape is functional output mismatch, not a SURFACE cite.
- No V0.4-DEFER, no INFRA-FLAKY, no UNKNOWN.
- The SIGABRT cascade-aborted everything after
  `qa::regression_crit_1_nested_property_access` in this binary; the
  ~71 "passed" + 42 "failed" tally is what completed before the abort.
  The 1065-test corpus number on the audit charter is workspace-wide,
  not this single binary.
