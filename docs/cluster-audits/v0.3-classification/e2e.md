# e2e classification

**HEAD:** 82f049dd
**Total tests in binary:** 4
**Passed:** 3 / Failed: 1 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test e2e --no-fail-fast 2>&1`

Binary `tools/shape-test/tests/e2e/main.rs` wires in only the
`lifetime_borrow_drop` submodule. The sibling files
`network_io.rs` and `process_spawn.rs` exist on disk but are NOT
declared as `mod` in `main.rs`, so they do not contribute tests at
HEAD. The 4 tests run are all from `lifetime_borrow_drop.rs`.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 1 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### lifetime_borrow_drop::inferred_shared_reference_accepts_explicit_ampersand_on_named_function

Class: **FN-REG-CORRECTNESS**

Failure excerpt (verbatim from the run at HEAD 82f049dd):

```
thread 'lifetime_borrow_drop::inferred_shared_reference_accepts_explicit_ampersand_on_named_function'
  panicked at tools/shape-test/src/shape_test.rs:1292:9:
Expected run ok, got error:
  Some("Semantic error: [B0004] unexpected `&` argument:
        target parameter is not a reference parameter")
```

Minimal repro (verbatim from the test fixture; 3 lines of user-facing Shape):

```shape
fn head(arr) { arr[0] }
let xs = [9]
head(&xs)
```

Test expectation: `expect_number(9.0)` — asserts on user-facing
runtime semantics, NOT on a SURFACE message. The test stays the
same after fix; only the compiler needs to learn to accept an
explicit `&` at a call site whose target parameter has an
*inferred* shared-reference pass-mode.

**Why FN-REG-CORRECTNESS (not SCOPE-RECLAIM, not V0.4-DEFER):**

- Plausibly-correct user-facing Shape: `head(arr)` with an
  unannotated `arr` parameter is the most common borrow-inference
  shape; passing `&xs` to it is the most common call-site form. A
  reasonable user would expect this to work.
- Not in any dated SCOPE-RECLAIM table row (the 2026-05-18 /
  2026-05-21 / 2026-05-22 / 2026-05-26 user pull-ins all cover
  other territory — V3-S5 op_new_array construction-cascade,
  Array<string>, Len trait, object destructuring, W16.2-J PHF
  retirement, W17.3-4 per-container FieldType, comptime trait,
  KC #2 format_*, W18 content rendering, LSP-parity). Lifetime /
  borrow inference on named-function call sites is not named.
- Not V0.4-DEFER: failure is a hard compile-time reject of valid
  code, not a surface-and-stop on a v0.4-anchored gap.

**Affected compiler subsystem:** `crates/shape-vm/src/compiler/helpers.rs:2956`
emits the `[B0004] unexpected '&' argument: target parameter is not
a reference parameter` diagnostic; the call-site reference-model
analysis in `crates/shape-vm/src/compiler/compiler_impl_reference_model.rs`
(see comment at line 425: "Being too conservative here causes false
B0004 errors when passing...") is the relevant decision site. The
diagnostic code itself is defined in
`crates/shape-vm/src/mir/analysis.rs:175` /
`BorrowErrorKind::ReferenceStoredInEnum` family at line 229.

**Bisected regression commit:** not bisected within audit scope
(audit-only). `git log --oneline -- crates/shape-vm/src/compiler/helpers.rs`
shows the most-recent compiler-side change touching this file is
`50910757 fix(v0.3): R8 W9 B3 Drop runtime fix (VM MakeRef SURFACE
+ JIT surface-and-stop)`; the B0004 emission predates that commit.
Bisect candidate range: the W14.2-G6 e2e-features triage commit
`eb317757` (Phase 4b Round 5b W14.2-G6) authored the sibling test
`closure_can_capture_inferred_reference_parameter` with an explicit
SURFACE-AND-STOP comment, suggesting the surrounding lifetime /
borrow inference work landed around then; the specific
`inferred_shared_reference_accepts_explicit_ampersand_on_named_function`
test, however, expects success (`expect_number(9.0)`), so it was
written as a pin against a *fix*, not a SURFACE pin. The fix
appears not to have landed.

**Sibling tests in the same file pass** at HEAD:
- `closure_can_capture_explicit_reference_parameter` — PASS
  (explicit `&x` parameter declaration + `&value` argument).
- `closure_can_capture_inferred_reference_parameter` — PASS
  (pinned via `expect_run_err_contains("Ptr(NativeView)")` against
  a different, *deeper* gap per W14.2-G6 comment; documented v0.4
  anchor cited there is
  `v0.4-w17-typed-carrier-monomorphization-getprop-nativeview`).
- `callable_value_rejects_explicit_reference_without_declared_contract`
  — PASS (this test *expects* a B0004; it asserts the rejection
  path works for the *callable-value* case, where it is the
  intended semantics).

The failing test isolates the *named-function inferred-shared-ref*
case: the compiler currently routes a named-function call with an
inferred-shared-ref parameter through the same conservative
rejection path as the callable-value case, when it should accept
the `&` because the inferred reference contract is statically
known at the call site.

## UNKNOWN list

None.
