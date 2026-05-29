# Wave-1-extension Phase A — Group B 13-distinct-root per-bisect audit

**HEAD:** `4cfbf9d4`
**Audit-only.** No source changes. Team-lead commits at close.
**Worktree:** `shape-w1ext-groupb-bisect-audit` (branch `w1ext-groupb-bisect-audit`)

## Background

`docs/cluster-audits/v0.3.3/07-result-bang-and-try-broken.md` framed Group B
(`tools/shape-test/tests/error_handling/try_operator.rs`) as 15 tests sharing
the `c7-result-fn-return-kind-clobber` root. JOINT-FIX-1 (`f067797`) +
JOINT-FIX-1a (`bbc2c27`) + JOINT-FIX-1b (`608b1ee1` merging `805a834a`) drove
that count to **2 passing** (basic `try_op_propagates_err`,
`try_operator_propagates_err`, `try_op_multiple_first_fails`,
`try_op_multiple_second_fails`, `try_op_on_inline_err`,
`try_op_on_inline_ok`, etc — 25 of 38 try_operator tests now pass).

**Residual 13 Group B failures at HEAD `4cfbf9d4`** were measured 2026-05-29
via `cargo test -p shape-test --test error_handling --release --no-fail-fast
-- try_operator::`. JOINT-FIX-1b's close-relay identified 4 candidate sub-
roots: (#1) top-level Err propagation; (#2) missing `__into_int` builtin;
(#3) V2 typed-array verification; (#4) inference-loss in nested arms. This
audit per-bisects all 13 and confirms (or refines) the sub-root assignment +
Wave class.

**Coordination:** A parallel audit is auditing `?`-at-script-toplevel
semantics (agent `w1ext-q-toplevel-audit`). Sub-root #1 is owned by that
audit. This audit confirms which Group B tests are subsumed by #1 vs need
distinct fix territory.

## Sub-root classification

| Sub-root | Count | Layer | Wave-class | Phase B fix-target shape |
|---|---:|---|---|---|
| #1 top-level Err/None propagation | 4 | runtime | Wave 1 | subsumed by `?`-toplevel audit (`?` is a no-op at script-top — wrong semantics) |
| #2 stale `__into_int` / `__try_into_int` in TEST FIXTURE | 2 | (n/a — fixture) | Wave 1 | **fixture-only edit** — replace with available primitive (e.g. plain `(self as int)` body or trait impl removed). Compiler/runtime correct per `intrinsics.shape:162`. |
| #3 inference-loss after `?` (nested-arm / closure / unannotated fn) | 4 | compile-time | Wave 1 | extend `stamp_unwrapped_success_type` (`compile_expr_try_operator`, `crates/shape-vm/src/compiler/expressions/advanced.rs:83`) to cover nested-fn-result + closure-body + bare-arg cases |
| #4 call-arg / loop-var KIND CLOBBER post-`?` (silent-wrong-output) | 2 | runtime | Wave 1 | same family as JOINT-FIX-1b but at **call-arg passing** + **for-iter-var binding** sites (not StoreLocal). Likely the typed `OpCode::CallTyped*` arg-pass + `op_iter_next_*` arms also re-stamp kind from the parameter's static kind, dropping the actual producer kind. |
| #5 V2 typed-array verifier warning (incidental — NOT the test's failure) | 0 separate | n/a | n/a | (the `NewTypedArrayI64 ... no FrameDescriptor` print is a verifier WARNING that does not block execution; the actual test failure is sub-root #4 silent-wrong-output) |
| #6 user-impl TryInto/Into dispatch missed (`as int?` falls through to built-in) | 1 | compile-time | Wave 1 | trait-dispatch routing — `compile_as_cast` ignores `impl TryInto<int> for string` in scope; emits `TryConvertToInt` opcode directly. NEEDS-INVESTIGATION whether this is the same as #3 or a distinct compile-emit gap. |
| **Total Group B failing at HEAD 4cfbf9d4** | **13** | | | |

**Notes on revisions from the 1b relay's 4-sub-root prediction:**
- The "missing `__into_int` builtin" prediction (#2 in relay) is a **FIXTURE
  STALENESS** issue, not a compiler-emit gap. `__into_*` / `__try_into_*`
  builtins were intentionally removed (`crates/shape-runtime/stdlib-src/
  core/intrinsics.shape:162`) when primitive conversions migrated to typed
  `ConvertTo*` / `TryConvertTo*` opcodes (per the comment at L162-163).
  Test fixtures `try_op_propagates_conversion_failure` +
  `infallible_type_assertion_uses_into_impl` still call the deleted
  builtins by name in the `impl TryInto/Into` body. This is NOT a compiler-
  emit gap.
- The "V2 typed-array verification" prediction (#3 in relay) was based on
  the visible `V2 bytecode verification failed: N violation(s)` print. The
  print is a WARNING (per `crates/shape-vm/src/executor/vm_impl/program.rs:
  92` — "warning" wording) that does NOT block execution. The actual test
  failure is silent-wrong-output from sub-root #4 (kind clobber on call-arg /
  iter-var). The V2-verifier missing-FrameDescriptor warning is incidental.
- The "inference-loss in nested arms" prediction (#4 in relay) is sub-root
  #3 above. Confirmed.
- Sub-root #6 (user-impl TryInto dispatch missed) emerged from the
  per-bisect; was not in 1b's relay prediction. NEEDS-INVESTIGATION
  whether 1b's enclosing classification absorbs this or it warrants its
  own bisect.

## Per-test classification

### 1. `try_operator::err_propagated_at_top_level_is_uncaught_exception`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:591-599`

Repro (file `/tmp/groupb_repros/01_err_propagated_at_top_level.shape`):

```shape
fn failing() -> Result<int> { Err("top level error") }
failing()?
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode (`./target/release/shape run --mode vm`): prints
  `{"Result": {"ok": false, "value": {"String": "top level error"}}}` — Err
  carrier returned as the script's expression value. Exit code 0.
- JIT mode: same (`?`-residual flag deopts to VM per c4-4B).
- Test asserts `expect_run_err_contains("top level error")` →
  `Expected run error, but got: Some(Object {"Result": Object {"ok":
  Bool(false), "value": Object {"String": String("top level error")}}})`.

**Classification: #1 top-level Err propagation**

Rationale: At script top-level, `?` should propagate `Err`/`None` as an
uncaught exception (host-visible error). Currently `?` evaluates the Result,
gets the Err, and (per `op_try_unwrap` at `crates/shape-vm/src/executor/
exceptions/mod.rs:687`) calls `return_value_inner` which for a script top-
level frame just stores the value as the script result. The `?`-at-script-
toplevel-throws-vs-pushes-as-script-value semantics question is the parallel
audit's territory.

**Wave-class: Wave 1** (memory-safety: NO; silent-wrong-output: YES — script
exits 0 with wrong shape; spurious-reject: NO). FN-REG-CORRECTNESS.

**Subsumed by `?`-toplevel audit.** No separate fix territory.

---

### 2. `try_operator::try_op_at_top_level_err_fails`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:569-578`

Repro:
```shape
Err("top level error")?
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: prints `{"Result": {"ok": false, "value": {"String": "top level
  error"}}}`. Exit code 0.
- JIT mode: same (deopts to VM).
- Test asserts `expect_run_err_contains("top level error")`.

**Classification: #1 top-level Err propagation**

Rationale: Identical to #1 above but without an enclosing fn. The `?` on a
direct top-level `Err(...)` literal exhibits the same wrong-semantics:
returns Err carrier as script value instead of throwing.

**Wave-class: Wave 1** FN-REG-CORRECTNESS. **Subsumed by `?`-toplevel
audit.**

---

### 3. `try_operator::try_op_at_top_level_none_fails`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:580-589`

Repro:
```shape
None?
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: no script output, exits with `Null` value (panic message:
  `Expected run error, but got: Some(String("Null"))`).
- JIT mode: same (deopts to VM).
- Test asserts `expect_run_err_contains("None")`.

**Classification: #1 top-level None propagation** (mirror of Err case).

Rationale: Top-level `None?` should throw `Value was None` per book contract.
Currently `?` propagates None silently to script-end where it renders as
`Null`. Same `?`-toplevel semantics question.

**Wave-class: Wave 1** FN-REG-CORRECTNESS. **Subsumed by `?`-toplevel
audit.**

---

### 4. `try_operator::none_try_propagation_returns_err`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:524-537`

Repro:
```shape
fn test() -> Result<int> {
  let x = None?
  Ok(x)
}
print(test())
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: prints `null`. (The `None?` propagates None to fn return; fn-
  return-as-None then renders as `null` rather than being lifted to
  `Err(AnyError{...OPTION_NONE...})`.)
- JIT mode: same.
- Test asserts `expect_output_contains("Err")`.

**Classification: #1 top-level None propagation — INSIDE fn** (variant)

Rationale: This is the same `?`-on-None semantics gap, but observed INSIDE a
fn that has `-> Result<int>` return type. Per book contract, `None?` inside a
fn returning `Result<T>` should early-return `Err(AnyError{OPTION_NONE,...})`,
NOT propagate the bare null. `op_try_unwrap` (`crates/shape-vm/src/executor/
exceptions/mod.rs:702`) currently does `self.return_value_inner(opt_bits,
opt_kind)` returning the raw None carrier — the AnyError-wrapping the
docstring claims ("AnyError-wrapped OPTION_NONE") is missing. This is
distinct from #1-#3 (which are script-top-level questions); this one is
**inside-fn None-to-Err lift gap**.

**Wave-class: Wave 1** FN-REG-CORRECTNESS. Likely **adjacent to `?`-toplevel
audit** but distinct enough to need its own fix (None-to-Err lift in
fallible-fn-return position).

---

### 5. `try_operator::try_op_on_none_propagates_err`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:491-505`

Repro (`/tmp/groupb_repros/05_try_op_on_none.shape`):
```shape
fn run() -> Result<number> {
    let v = None?
    Ok(v)
}
match run() {
    Ok(v) => 0
    Err(_) => -1
}
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: `Error: Runtime error: Uncaught error: No match arm matched the
  value (line 8)` — the `match run()` failed because `run()` returned the
  null sentinel and neither `Ok(v)` nor `Err(_)` arm matched.
- JIT mode: same.
- Test asserts `expect_number(-1.0)` (i.e. the `Err(_)` arm should fire).

**Classification: #1 (variant) — None-to-Err lift gap INSIDE fn**

Rationale: Identical root to test #4. `None?` inside `run()` returns raw
null instead of `Err(AnyError{OPTION_NONE,...})`; the caller's `match` then
fails the exhaustiveness check because `null` matches neither arm.

**Wave-class: Wave 1** FN-REG-CORRECTNESS.

---

### 6. `try_operator::try_op_on_err_skips_rest`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:103-118`

Repro (`/tmp/groupb_repros/06_try_op_on_err_skips.shape`):
```shape
fn run() -> Result<number> {
    let v = Err("stop")?
    Ok(v + 100)
}
match run() {
    Ok(v) => v
    Err(_) => -999
}
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: `error[SEMANTIC]: Cannot infer types for binary operation Add:
  operand types are unknown and int.` Compile-time reject at line 3 (`v +
  100`).
- JIT mode: same (compile-time error fires before execution).
- Test asserts `expect_number(-999.0)`.

**Classification: #3 inference-loss after `?`**

Rationale: `Err("stop")` infers as `Result<unknown, string>` (success-arm
type variable never resolved because there is no Ok constructor in this
position). `stamp_unwrapped_success_type` (`compile_expr_try_operator` at
`crates/shape-vm/src/compiler/expressions/advanced.rs:83`) tries to extract
the success type from the inner expression's inferred type, sees a
`Type::Variable(TypeVar)` for the success arm, and returns None — `v`'s slot
ends up with no kind. `v + 100` then fires the strict-typing reject. The fix
is to use the ENCLOSING function's declared return type's success arm
(`Result<number>` → `number`) when the inner expression's success arm is an
unresolved type variable.

**Wave-class: Wave 1** FN-REG-CORRECTNESS.

---

### 7. `try_operator::try_op_in_closure`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:625-640`

Repro (`/tmp/groupb_repros/08_in_closure.shape`):
```shape
fn run() -> Result<number> {
    let f = |x| Ok(x * 3)?
    Ok(f(5))
}
match run() {
    Ok(v) => v
    Err(_) => -1
}
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: `error[SEMANTIC]: Cannot infer types for binary operation Mul:
  operand types are unknown and int.` Compile-time reject at line 2 (`x * 3`).
- JIT mode: same.
- Test asserts `expect_number(15.0)`.

**Classification: #3 inference-loss after `?`** (closure-param-infer-loss
sub-variant)

Rationale: The closure `|x| Ok(x * 3)` has no parameter type annotation on
`x`. Bidirectional inference must propagate `f`'s eventual call-site type
(`f(5)` → `x: int`) backward into the closure body. The `?` on the closure
body (which yields the closure result) compounds the inference work. Likely
adjacent to cluster #08 (`closures-S1-closure-param-infer-loss`). The `?`
operator's `stamp_unwrapped_success_type` is downstream of the closure
inference and so cannot help. **Adjacent to cluster #08 — may be subsumed
by #08's fix; verify in Phase B.**

**Wave-class: Wave 1** FN-REG-CORRECTNESS.

---

### 8. `try_operator::try_op_on_nested_result`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:407-423`

Repro (`/tmp/groupb_repros/10_on_nested_result.shape`):
```shape
fn inner() -> Result<number> { Ok(5) }
fn outer() -> Result<number> {
    let r = inner()
    let v = r?
    Ok(v * 2)
}
match outer() {
    Ok(v) => v
    Err(_) => -1
}
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: `error[SEMANTIC]: Cannot infer types for binary operation Mul:
  operand types are unknown and int.` Compile-time reject at line 5 (`v * 2`).
- JIT mode: same.
- Test asserts `expect_number(10.0)`.

**Classification: #3 inference-loss after `?`**

Rationale: `let r = inner()` binds `r: Result<number>` correctly (inferred
from `inner`'s return type), but `let v = r?` then loses the `number`. The
`stamp_unwrapped_success_type` chain runs on `r?` — `r`'s inferred type is
`Result<number>`, success arm is `number`, so the stamp should succeed.
This case suggests the inference is correct but the **propagation from
`r?`'s last-expr stamp to `v`'s binding slot has a gap** (likely
`propagate_initializer_type_to_slot` not reading the stamp, or the stamp is
cleared between `r?` compilation and the `let v = ...` initializer-store
binding step). Distinct from test #6 (`Err("stop")?`) which has the inner-
type missing; this one has the inner-type PRESENT but the stamp is lost in
propagation.

**Wave-class: Wave 1** FN-REG-CORRECTNESS.

---

### 9. `try_operator::try_op_in_loop_all_ok`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:355-374`

Repro (`/tmp/groupb_repros/09c.shape` — minor: type annotations added):
```shape
fn check(n) -> Result<number> { Ok(n * 2) }
fn run() -> Result<number> {
    let mut sum = 0
    for i in [1, 2, 3] {
        let v = check(i)?
        sum = sum + v
    }
    Ok(sum)
}
match run() { Ok(v) => v; Err(_) => -1 }
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: stderr prints `V2 bytecode verification failed: 4 violation(s) -
  V2 typed opcode NewTypedArrayI64 at offset 13 in function 'run' has no
  FrameDescriptor` (warning), then test asserts and reports
  `Expected 12, got 4886673876534166000`.
- JIT mode: same.
- Test asserts `expect_number(12.0)`.

**Classification: #4 call-arg / loop-var kind clobber post-`?`**

Rationale: The V2 verifier warning is a **distinct surface but not
blocking** — a `print(sum)` after a simpler `for i in [1,2,3] { sum +=
i }` body also prints the verifier warning but produces correct `6`. The
actual silent-wrong-output (`4886673876534166000`, a denormal/bit-cast of an
i64) comes from the same `_src_kind` discard pattern JOINT-FIX-1b fixed at
the typed `op_store_local_*` family — but here the discard happens on
**call-arg passing into `check(i)`** (the param `n` is stamped from the
opcode suffix, dropping the producer's actual kind). The receiver `n: number`
sees the i64-bits of `i` (an int) as if they were f64 bits — `n * 2` then
produces a denormal that propagates through the rest of the loop.

The compiler-emit side appears to use a typed `Call*` opcode (or similar)
whose handler re-stamps the parameter slot from the opcode suffix, mirroring
1b's `op_store_local_*` clobber. **Adjacent root family to JOINT-FIX-1b.**

**Wave-class: Wave 1** FN-REG-CORRECTNESS. **Silent-wrong-output**
(memory-safety: NO; numeric: YES).

---

### 10. `try_operator::try_op_chained_function_calls`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:605-623`

Repro (`/tmp/groupb_repros/07_chained_function_calls.shape`):
```shape
fn step1() -> Result<number> { Ok(10) }
fn step2(x) -> Result<number> { Ok(x + 20) }
fn run() -> Result<number> {
    let a = step1()?
    let b = step2(a)?
    Ok(b)
}
match run() { Ok(v) => v; Err(_) => -1 }
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: `Expected 30, got 20`. Probe (`/tmp/groupb_repros/07e.shape`)
  showed `print(a)` inside `run` prints `10`, `print(x)` inside `step2`
  prints `5e-323` (denormal = `f64::from_bits(10)`). So `a = 10` is stored
  correctly but the call-arg passing into `step2(a)` reads as raw i64 bits.
- JIT mode: same.
- Test asserts `expect_number(30.0)`.

**Classification: #4 call-arg kind clobber post-`?`**

Rationale: Identical root to test 9 — call-arg passing site re-stamps the
parameter slot from the opcode suffix's static kind. JOINT-FIX-1b fixed
`op_store_local_*` but not the call-arg / parameter-receive path. Same fix
shape will apply: preserve `src_kind` from producer at the call-arg
parameter binding.

**Wave-class: Wave 1** FN-REG-CORRECTNESS. **Silent-wrong-output (numeric).**

---

### 11. `try_operator::fallible_type_assertion_uses_named_try_into_impl`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:182-206`

Repro (`/tmp/groupb_repros/11_uses_named_try_into.shape`):
```shape
impl TryInto<int> for string as int {
    method tryInto() {
        Ok(7)
    }
}

fn parse_price(raw: string) -> Result<int> {
    let n = (raw as int?)?
    Ok(n)
}

match parse_price("n/a") {
    Ok(v) => v
    Err(_) => -1
}
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: `Error: Runtime error: cannot convert string 'n/a' to int (line
  9)`. The user-supplied `impl TryInto<int> for string` returning `Ok(7)`
  is **NOT being dispatched** — the `as int?` lowers to the built-in
  `TryConvertToInt` opcode (`crates/shape-vm/src/compiler/expressions/
  type_ops.rs:676`) which fails on `'n/a'`.
- JIT mode: same.
- Test asserts `expect_number(7.0)` (i.e. the impl's `Ok(7)` should fire).

**Classification: #6 user-impl TryInto/Into dispatch missed**
(NEEDS-INVESTIGATION whether this is its own sub-root or a sub-variant of
#3 inference-loss)

Rationale: The `as int?` cast should dispatch to the user-registered
`TryInto<int> for string as int` impl when in scope, not fall through to the
built-in `TryConvertToInt` opcode. The compile-emit path at
`crates/shape-vm/src/compiler/expressions/type_ops.rs:248-262` does check
for a user-registered `TryInto` impl via `lookup_trait_impl_named`, but for
this fixture either the lookup misses or the dispatch path emits the wrong
opcode. This is a **compile-emit gap distinct from sub-root #3**, but the
1b relay grouped it under sub-root #2 (because the related diagnostics tests
also failed with similar `__try_into_int` issues). Per CLAUDE.md `__into_*` /
`__try_into_*` are NOT-gated for the compiler's `as` lowering — the compiler
WAS generating these for primitive conversions before they were removed in
favor of typed `TryConvertTo*` opcodes (`intrinsics.shape:162`). The user-
impl dispatch path is separate territory.

**Wave-class: Wave 1** FN-REG-CORRECTNESS.

---

### 12. `try_operator::fallible_type_assertion_propagates_conversion_failure`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:208-230`

Repro (`/tmp/groupb_repros/12_propagates_conversion_failure.shape`):
```shape
impl TryInto<int> for string as int {
    method tryInto() {
        __try_into_int(self)
    }
}

fn parse(raw: string) -> Result<int> {
    let n = (raw as int?)?
    Ok(n)
}

match parse("not-int") { Ok(v) => v; Err(_) => -1 }
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: `error[RUNTIME]: Undefined function: __try_into_int.`
- JIT mode: same.
- Test asserts `expect_number(-1.0)`.

**Classification: #2 stale `__try_into_int` reference in TEST FIXTURE**

Rationale: The TEST FIXTURE body explicitly calls `__try_into_int(self)`.
This builtin was removed per `crates/shape-runtime/stdlib-src/core/
intrinsics.shape:162` ("`__into_*`/`__try_into_*` builtin declarations
removed — primitive conversions now use typed ConvertTo*/TryConvertTo*
opcodes directly"). The COMPILER no longer emits this name; only the test
fixture's user-written impl body still does. **Per supervisor binder:**
"if the gap is a missing `__into_int` the compiler should be generating for
an `as int` assertion, that's a compiler-emit gap, not a gating issue" —
here the compiler is NOT generating `__try_into_int`; the test fixture is.
**FIXTURE-ONLY EDIT**, not a compiler-emit gap. Update the impl body to use
a non-removed primitive (e.g. `Err("...")` to force the propagates-failure
path, or rely on `TryConvertToInt` opcode dispatch via a different shape).

**Wave-class: Wave 1** FN-REG-CORRECTNESS (fixture-class).

---

### 13. `try_operator::infallible_type_assertion_uses_into_impl`

Location: `tools/shape-test/tests/error_handling/try_operator.rs:232-247`

Repro (`/tmp/groupb_repros/13_infallible_into_impl.shape`):
```shape
impl Into<int> for bool as int {
    method into() {
        __into_int(self)
    }
}

let x = true as int
x
```

Pre-fix at HEAD `4cfbf9d4`:
- VM mode: `error[RUNTIME]: Undefined function: __into_int.`
- JIT mode: same.
- Test asserts `expect_number(1.0)`.

**Classification: #2 stale `__into_int` reference in TEST FIXTURE**

Rationale: Same as test 12 — the FIXTURE body calls the removed
`__into_int` builtin. Per `intrinsics.shape:162` the builtin no longer
exists. Compiler-side `true as int` should dispatch to `ConvertToInt` opcode
(`crates/shape-vm/src/compiler/expressions/type_ops.rs:625`) — which would
work without the user-impl since `bool→int` is a built-in primitive
conversion. **FIXTURE-ONLY EDIT.** Either:
- (a) drop the `impl Into<int> for bool` block — the built-in
  `ConvertToInt` already handles `bool→int` (yields 1/0) per `executor/
  builtins/type_ops.rs`.
- (b) change the impl body to `if self { 1 } else { 0 }`.

**Wave-class: Wave 1** FN-REG-CORRECTNESS (fixture-class).

---

## SCOPE-RECLAIM boundary check (sub-root #3 / #4 / #5 specific)

Per supervisor binder: "audit names which of the 4 sub-roots are
compile-time vs runtime; Wave-class each (all expected Wave-1-class but
confirm none are latent SCOPE-RECLAIM — e.g. V2 typed-array verify could
touch V3-S5 territory)."

**Check 1: V2-verifier `NewTypedArrayI64 ... no FrameDescriptor` warning
(test 9).**

The verifier print fires on a function that uses array literals (`[1, 2,
3]`) inside its body when the function lacks a registered FrameDescriptor.
This is `crates/shape-vm/src/bytecode/verifier.rs:243-249` flagging
`func.frame_descriptor.is_none()` for V2-typed opcodes. **DISTINCT from
V3-S5 ckpt-5/6 Family-1 op_new_array construction-cascade** (`docs/
cluster-audits/v0.3.3/13-scope-reclaim-partition.md:31-94`):

- V3-S5 ckpt-5/6 SURFACE shape is: `Runtime error: Not implemented:
  op_new_array(N): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3` (a
  surface-and-stop in `executor/`). Architectural target: per-T v2-raw
  `TypedArray<T>` flat-struct monomorphization. Lands at ckpt-6 STRICT
  close.
- The Group B test 9 SURFACE is: `V2 bytecode verification failed: N
  violation(s) - V2 typed opcode NewTypedArrayI64 at offset N in function
  'run' has no FrameDescriptor`. The opcode is `NewTypedArrayI64`
  (working — the bytecode IS emitted and DOES execute). The verifier
  catches a `FrameDescriptor` attachment gap. Not SURFACE-and-stop; not
  op_new_array; not the TypedArrayData deletion family.

**Boundary call: NOT latent SCOPE-RECLAIM.** The verifier warning is
incidental — the actual test-failure shape (silent-wrong-output) is sub-root
#4 kind-clobber, which is JOINT-FIX-1b-adjacent and Wave-1-class.

**Check 2: any other test failure shape touch V3-S5 / W17.3-4 / W18 / KC#2
territory?**

Per per-bisect inspection: no. Sub-roots #1/#3/#4/#6 are all runtime kind-
tracking + compile-time inference + trait-dispatch. None touches:
- TypedArrayData construction-site rebuild (Family 1 V3-S5 ckpt-5/6)
- per-container `FieldType` rebuild (Family 3 W17.3-4)
- content-rendering (W18)
- aliased-CoW SEGFAULT (§5.16 Family 1)
- imported-const ident-eval (§5.16 Family 2)
- W17-marshal-return (§5.16 Family 3)
- Drop codegen (§5.16 Family 4)
- B2 EnumPayload (§5.16 Family 5)

**Outcome: NONE of the 13 Group B tests are latent SCOPE-RECLAIM.**
All 13 are Wave-1-class FN-REG-CORRECTNESS. No supervisor + user
re-disposition needed for sub-root #3 / #4 / #5.

## Phase B fix-target list

### Sub-root #1 — top-level Err/None propagation (4 tests)

**Tests:** 1, 2, 3, 4, 5 (test 4 + 5 are "inside-fn None-to-Err lift" — a
variant; tests 1/2/3 are pure top-level).

**Wait — recount:** 1 (err_propagated_at_top_level), 2 (top_level_err), 3
(top_level_none), 4 (none_try_propagation — fn-return-None-to-null), 5
(try_op_on_none_propagates_err — fn-return-None-to-null fed to match).

Tests 4 + 5 are technically the **None-to-Err lift gap INSIDE fn-return-
position**, distinct from tests 1-3 (top-level script-end semantics). The
parallel `?`-toplevel audit may or may not absorb tests 4 + 5; the
inside-fn variant is a `op_try_unwrap` op-body issue (the None arm at
`crates/shape-vm/src/executor/exceptions/mod.rs:702` currently returns the
raw None carrier instead of constructing
`Err(AnyError{OPTION_NONE,...})`).

| Test count | Likely fix layer | Sub-cluster size | Dependency |
|---:|---|---|---|
| 3 (tests 1, 2, 3) | `op_try_unwrap` + script-top-level frame return ABI | S | **subsumed by `?`-toplevel audit** |
| 2 (tests 4, 5) | `op_try_unwrap` None arm: wrap into `Err(AnyError{OPTION_NONE})` per docstring's stated intent at L671 | S | depends on `?`-toplevel audit's None-to-Err semantics decision |

### Sub-root #2 — stale `__into_int` / `__try_into_int` in TEST FIXTURE (2 tests)

**Tests:** 12, 13.

| Test count | Likely fix layer | Sub-cluster size | Dependency |
|---:|---|---|---|
| 2 | `tools/shape-test/tests/error_handling/try_operator.rs:211-216, 235-240` (fixture-only edits) | XS | none (fixture-only) |

**Important:** the COMPILER does not emit `__into_int` / `__try_into_int`
anywhere — these were intentionally removed in favor of typed
`ConvertTo*` / `TryConvertTo*` opcodes (`crates/shape-runtime/stdlib-src/
core/intrinsics.shape:162`). **No compiler-emit fix.** Only fixture
modernization. This matches the supervisor binder verbatim: "if the gap is
a missing `__into_int` the compiler should be generating for an `as int`
assertion, that's a compiler-emit gap, not a gating issue" — here the
compiler is NOT generating it.

### Sub-root #3 — inference-loss after `?` (4 tests)

**Tests:** 6, 7, 8 — and possibly #11 (NEEDS-INVESTIGATION).

| Test count | Likely fix layer | Sub-cluster size | Dependency |
|---:|---|---|---|
| 1 (test 6) | `compile_expr_try_operator` (`crates/shape-vm/src/compiler/expressions/advanced.rs:83`): extend `stamp_unwrapped_success_type` to fall back on enclosing fn-return-type's success arm when the inner expr's success arm is an unresolved `TypeVar` | S | none |
| 1 (test 7) | adjacent to cluster #08 closures-S1 closure-param-infer-loss (`docs/cluster-audits/v0.3.3/08-closures-S1-closure-param-infer-loss.md`) | S | **adjacent — may be subsumed by cluster #08 fix** |
| 1 (test 8) | `propagate_initializer_type_to_slot` not consuming `last_expr_type_info` stamp from `compile_expr_try_operator` — bisect needed | S | none |

### Sub-root #4 — call-arg / loop-var kind clobber post-`?` (2 tests)

**Tests:** 9, 10.

| Test count | Likely fix layer | Sub-cluster size | Dependency |
|---:|---|---|---|
| 2 | typed `CallTyped*` arg-pass handlers OR typed `op_iter_next_*` handlers preserve `src_kind` from producer parallel-kind track (mirror JOINT-FIX-1b's `op_store_local_*` fix shape) | S | shares fix family with JOINT-FIX-1b; **investigate at fix-time whether this is a single 2-line handler change or needs systematic call-arg-ABI walk** |

### Sub-root #6 — user-impl TryInto/Into dispatch missed (1 test)

**Tests:** 11.

| Test count | Likely fix layer | Sub-cluster size | Dependency |
|---:|---|---|---|
| 1 | `crates/shape-vm/src/compiler/expressions/type_ops.rs:248-262` (`lookup_trait_impl_named` for TryInto on `as int?` cast) — possibly returns the impl correctly but the dispatch path lowers to the wrong opcode | M (NEEDS-INVESTIGATION — could be S if simple lookup miss) | none |

### Per-sub-root summary

| Sub-root | Tests | Sub-cluster size | Dispatch | Coord |
|---|---:|---|---|---|
| #1 (top-level + inside-fn None-to-Err) | 5 (3 + 2) | S + S | Phase B candidate(s) | coord w/ `?`-toplevel audit |
| #2 (stale fixture) | 2 | XS | Phase B candidate | fixture-only edit |
| #3 (inference-loss) | 3 | S × 3 | Phase B candidate(s) | check overlap w/ #08 closures-S1 |
| #4 (kind clobber call-arg / iter-var) | 2 | S | Phase B candidate | mirror 1b fix shape |
| #6 (TryInto dispatch missed) | 1 | M? | Phase B candidate | NEEDS-INVESTIGATION |
| **Total** | **13** | | | |

## Discipline notes

- Audit-only. No source changes. No commits. No `git stash`.
- 13 minimal repros at `/tmp/groupb_repros/01_*.shape` through `13_*.shape`
  (transient; not committed).
- Run-verified at HEAD `4cfbf9d4` via `./target/release/shape run --mode
  {vm,jit} <repro>.shape` plus `cargo test -p shape-test --test
  error_handling --release --no-fail-fast -- try_operator::` (full
  panic-message capture at `/tmp/groupb_stderr.log`).
- No defection-attractor framings used. No "renames-to-refuse-on-sight"
  family. Sub-roots #4's fix is preserve-src_kind in the typed call-arg /
  iter-var ABI — same family as JOINT-FIX-1b's `op_store_local_*` fix,
  NOT a shim/bridge/decode helper.
- Per supervisor binders + CLAUDE.md `Forbidden Patterns`. No SCOPE-RECLAIM
  re-disposition risk surfaced (all 13 are Wave-1-class).
