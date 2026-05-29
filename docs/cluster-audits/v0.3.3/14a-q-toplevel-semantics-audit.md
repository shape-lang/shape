# Wave-1-extension Phase A — `?` operator at script-toplevel semantics audit

**HEAD:** 4cfbf9d4
**Branch:** w1ext-q-toplevel-audit
**Audit-only.** No source changes. Team-lead commits the audit doc at close.
**Date:** 2026-05-29.

## Background

JOINT-FIX-1a (commit `b8a2940a` → merge `bbc2c272`) closed the `!!`
context-operator with WRAP semantics per the book contract: every `lhs !! rhs`
branch produces a fresh `Result` carrier (`Ptr(HeapKind::Result)`) and never
throws. The throw-of-an-Err is deferred to a downstream `?` operator
(`expr !! "context"?`).

After 1a, 6 `expect_run_err_contains` tests in Group A still fail:

- `context_op_err_surfaces_in_runtime_error`            (`context_operator.rs:250`)
- `context_op_err_preserves_cause`                      (`context_operator.rs:260`)
- `context_op_none_surfaces_in_runtime_error`           (`context_operator.rs:270`)
- `context_op_none_includes_none_cause`                 (`context_operator.rs:280`)
- `context_op_declared_result_return_type_propagates`   (`context_operator.rs:406`)
- `declared_result_return_with_err_context`             (`context_operator.rs:419`)

1a's close-relay: "they require `?` at script-toplevel to throw, which is
**JOINT-FIX-1b's `op_try_unwrap` territory (β)**." JOINT-FIX-1b
(`805a834a` → `608b1ee1`) closed Ok-payload `src_kind` preservation through
the typed `StoreLocal<Kind>` / `StoreModuleBinding<Kind>` family but
**explicitly deferred** "top-level Err propagation" as a distinct root cause
("Remainder of Group B failures are distinct root causes (top-level Err
propagation, type inference gaps, V2 typed-array verification warnings)
tracked separately." — 1b commit message).

Supervisor binder (verbatim): "The audit must resolve the language-design
contract for `?` at script-toplevel BEFORE any fix dispatches. `?` at
function-position propagates the Err carrier as the frame return (verified
at `exceptions/mod.rs:638-646` last checkpoint). At script-toplevel there
is no enclosing fallible frame — so the contract is genuinely undefined by
the function-position semantics. The 6 `expect_run_err` Group A tests assert
it THROWS (uncaught) at toplevel."

## Triangulation (a) — book reference

Book file:
`/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/fundamentals/error-handling.mdx`

The `?` operator is documented at §"## `?` Try Operator" (L105-119):

```
`expr?` unwraps and propagates failures.

| Input    | Behavior                                            |
|---|---|
| `Ok(v)`  | yields `v`                                          |
| `Err(e)` | early-return `Err(e)`                               |
| `Some(v)`| yields `v`                                          |
| `None`   | early-return `Err(AnyError)` (code `OPTION_NONE`)   |

This makes Option propagation Result-compatible.

`?` is compile-time restricted to `Result<...>` and `Option<...>` operands.
Using `?` on a plain non-fallible value is a semantic error.
```

The book also has §"## Uncaught Exception Display" (L274-295) which
describes the rendering of an uncaught exception with example output:

```
Uncaught exception:
Error [OPTION_NONE]: high level context
  at load_config (cfg.shape:7) [ip 29]
Caused by: Value was None
  at read_file (cfg.shape:3) [ip 11]
```

— but does NOT specify HOW an exception becomes "uncaught" at script
toplevel; the framing is `load_config` → `read_file` (both fn frames), and
the §"## End-to-End Example" (L306-322) wraps the toplevel call in
`fn main() { let cfg = load_config("config.shape")?; Ok(cfg) }` — that is,
the example is INSIDE a `main()` frame, not at bare script toplevel.

**(a) Outcome:** the book is **SILENT** on the dedicated semantics of
`?` at bare script-toplevel (no enclosing `fn` frame). The §`?` Try
Operator table describes "early-return `Err(e)`" — but "early-return"
presumes an enclosing returnable scope. At bare toplevel there is no
enclosing fallible-frame return target.

The strongest indirect signal in the book is §"## Uncaught Exception
Display" — it describes an uncaught-exception RENDER path, implying that
some path turns an unhandled error into an uncaught exception with a
distinct user-facing message. The book does not name `?`-at-toplevel as
the trigger, but the uncaught-render path is the natural sink for it.

## Triangulation (b) — test corpus assertions

### Group A (`context_operator.rs`) — the 6 failing tests

All 6 share the structural shape `<Err-producer> !! "context"?` at bare
script-toplevel (no enclosing `fn`). Two of them put the producer behind a
function call (`test()?`).

| Test name | Code (verbatim) | `expect_run_err_contains` substring |
|---|---|---|
| `context_op_err_surfaces_in_runtime_error` (L250) | `Err("low level") !! "high level context"?` | `"high level context"` |
| `context_op_err_preserves_cause`           (L260) | `Err("original cause") !! "wrapper context"?` | `"original cause"` |
| `context_op_none_surfaces_in_runtime_error`(L270) | `None !! "missing value"?` | `"missing value"` |
| `context_op_none_includes_none_cause`      (L280) | `None !! "missing value"?` | `"None"` |
| `context_op_declared_result_return_type_propagates` (L406) | `fn test() -> Result<int> { return Err(...) !! "..." } \n test()?` | `"yes, something went wrong"` |
| `declared_result_return_with_err_context`  (L419) | `fn test() -> Result<int> { return Err(...) !! "..." } \n test()?` | `"yes, something went wrong"` |

Interpretation per assertion shape: the test framework's
`expect_run_err_contains` (`shape_test.rs:1272`) asserts that `eval()`
returns `Err(_)` (Rust-level) AND that the error string contains the given
substring. For shape-test's eval path, this means the host sees a
`VMError::RuntimeError(msg)` propagated past all in-language handlers, with
the user-facing context message embedded. This is exactly the shape that
`handle_exception` (`exceptions/mod.rs:164-179`) produces when there is no
matching `exception_handlers` entry: `Err(VMError::RuntimeError(format!
("Uncaught error: {}", message)))`.

**Group A assertion shape:** THROW (uncaught) at bare script-toplevel with
the `!!` context message as the user-visible error text.

### Group B (`try_operator.rs`) — `?` semantics

Group B has 27 tests. The relevant subset:

(i) `?` at **function-position** (most of Group B; e.g. `try_op_unwraps_ok_value`
L51, `try_op_propagates_err` L69, `try_op_on_err_skips_rest` L104, all
`try_op_*` and `chain_of_*` and `nested_*`). These wrap `?` in a fallible
function (`fn run() -> Result<number>`) and match the result downstream.
They verify the documented behavior at L112: `Err(e)` → early-return
`Err(e)` to the caller's frame. These are SOUND today per `op_try_unwrap`
(`exceptions/mod.rs:679-688`) calling `return_value_inner` which pops the
call frame and pushes the Err carrier to the caller.

(ii) `?` at **bare script-toplevel** — 4 tests, located under the dedicated
header "// ? at top level" (try_operator.rs:556):

| Test name | Code | Assertion |
|---|---|---|
| `try_op_at_top_level_ok` (L559) | `Ok(42)?` | `expect_number(42.0)` — unwrap to inner |
| `try_op_at_top_level_err_fails` (L570) | `Err("top level error")?` | `expect_run_err_contains("top level error")` |
| `try_op_at_top_level_none_fails` (L581) | `None?` | `expect_run_err_contains("None")` |
| `err_propagated_at_top_level_is_uncaught_exception` (L592) | `fn failing() -> Result<int> { Err("top level error") } \n failing()?` | `expect_run_err_contains("top level error")` |

The L592 test has an **explicit comment**: "If ? is used at top-level on an
Err, it should be an uncaught exception" — naming the throw-at-toplevel
contract directly in code-comment form.

### Group C (`edge_cases.rs`) — composition cases

4 additional `expect_run_err` tests use the same toplevel-`?` shape with
embedded error literals (L104-140):

- `edge_error_message_with_special_chars` (L104): `Err("error: '...' at /tmp/a.txt")?`
- `edge_error_message_with_quotes`        (L114): `Err("expected \"value\"...")?`
- `edge_error_message_with_newlines`      (L124): `Err("line1\nline2")?`
- `edge_very_long_error_message`          (L135): `Err("This is a very long error message...")?`

All four share the bare-toplevel `Err(...)?` shape and assert
`expect_run_err_contains` (THROW-uncaught with the error message in the
user-facing text).

**(b) Outcome:** the test corpus is **unambiguously** asserting
throw-at-toplevel as the contract. **13 tests** across 3 files
(6 Group A + 3 Group B toplevel + 4 Group C) all assert
`expect_run_err_contains` on the bare-toplevel `?`-on-Err/None shape, with
one test (`err_propagated_at_top_level_is_uncaught_exception`) carrying an
**explicit comment** naming the contract. The
`try_op_at_top_level_ok` companion test asserts the Ok-path symmetry
(toplevel `Ok(42)?` unwraps to `42` — the natural pair).

## Triangulation (c) — sibling operator semantics consistency

### `!!` (post-JOINT-FIX-1a, `exceptions/mod.rs:544-624`)

WRAP at all positions — function-position AND script-toplevel. Five
branches per the book contract table (L236-243); every branch produces a
fresh `Arc<ResultData>` carrier and pushes it. **No `handle_exception`
path.** The post-fix docstring is explicit: "if you find yourself reaching
for `handle_exception` in any branch below, STOP — that contradicts the
book + the 26 Group A WRAP-shaped tests. The throw shape was the deleted
pre-fix behavior; do not reintroduce."

`!!`'s position-independence is structurally important: `!!` is "always a
Result-producer" regardless of frame depth. The throw happens elsewhere
(downstream `?` or explicit `throw`).

### `?` at function-position (`exceptions/mod.rs:658-718`)

Verified sound today. On Err/None it calls `self.return_value_inner(bits,
kind)` (`exceptions/mod.rs:687, 702, 709`). `return_value_inner`
(`control_flow/mod.rs:763-812`) checks `if let Some(frame) =
self.call_stack.pop()` and, when the pop succeeds, restores the caller's
`ip` and `bp` and pushes the carrier onto the caller's operand stack. The
Err carrier surfaces as the fallible-fn's return value — exactly the book
contract.

### `?` at script-toplevel (`return_value_inner` else-branch, `control_flow/mod.rs:806-810`)

When `call_stack.pop()` returns `None` (no enclosing frame — bare toplevel
position), the else-branch executes:

```rust
} else {
    // Return from main
    self.push_kinded(return_bits, return_kind)?;
    self.ip = self.program.instructions.len();
}
```

The Err carrier is pushed to the operand stack and `ip` is jumped to
end-of-program. This is the **silent-push-and-halt** path — not throw, not
uncaught-exception, not user-visible runtime error. The shape-test eval
path then reads the toplevel result slot, sees an `Err(_)` Result carrier,
and returns `Ok(<Err carrier>)` to the Rust harness. From the host's
perspective there is no error — the program "succeeded" with an Err value
as its terminal value.

This is the silent-halt root cause for all 13 failing tests: they all
assert `result.is_err()` at the Rust harness boundary, but get
`result.is_ok()` carrying the Err carrier as a stringified Result value.

### Consistency analysis: is throw-at-toplevel-`?` consistent with the broader operator semantics?

The proposed throw-at-toplevel-`?` semantics is **internally consistent**
with the rest of the operator family:

1. `!!` is always-WRAP (position-independent producer of `Result`).
2. `?` at function-position is propagate-via-frame-return (the Err carrier
   surfaces as the enclosing fn's return value, available for downstream
   `?`/`!!` composition or `match`).
3. `?` at bare script-toplevel has NO enclosing fallible fn-frame. There is
   no structural target for "propagate the Err carrier as the frame
   return". The natural extension is "the host is the frame" — the Err
   surfaces to the host as an uncaught exception, mirroring how
   `op_throw` + `handle_exception` surfaces an uncaught throw when no
   `exception_handlers` entry is present.

Composition consistency check: the canonical book pattern
`expr !! "context"?` should produce identical user-facing behavior whether
written at script-toplevel (the 6 Group A failing tests) or inside `fn
main() -> Result<...> { ... }` (the book's End-to-End Example at L306-322).
Today only the latter works. The throw-at-toplevel semantics restores the
symmetry: bare-toplevel `expr !! "ctx"?` surfaces "ctx" as an uncaught
exception, exactly as `fn main() -> Result<int> { let _ = expr !! "ctx"?;
Ok(0) } main()?` would today (the test's L406/L419 shape — which is also
failing because the inner `main()?` likewise has no enclosing frame).

This is **not** the dual-mode framing the supervisor binder flagged ("legit
context-distinction (frame-presence is a real structural difference)"). It
is the SAME semantics — early-return to the enclosing fallible scope —
applied to a different definition of "enclosing scope". The structural
distinction (frame-presence) is real, but it does not split `?` into two
operators with different rules; it splits the early-return TARGET (caller
fn-frame vs host process) with the same SHAPE (surface the Err carrier as
the enclosing scope's failure).

**(c) Outcome:** throw-at-toplevel-`?` is consistent with the broader
operator semantics. It is the natural extension of the function-position
contract to the no-enclosing-frame case, with the host as the implicit
caller. `!!` stays position-independent WRAP; `?` stays
propagate-to-enclosing-fallible-scope with the host as the toplevel
"enclosing scope".

## Canonical contract decision

**(A) Throw-at-toplevel** is the canonical contract.

Rationale:

1. **Test corpus is unambiguous.** 13 tests across 3 files all assert
   `expect_run_err_contains` on the bare-toplevel `?`-on-Err/None shape.
   One test (`err_propagated_at_top_level_is_uncaught_exception`) carries
   an explicit code-comment naming the contract: "If ? is used at top-level
   on an Err, it should be an uncaught exception".

2. **The Ok-path companion is already asserted** (`try_op_at_top_level_ok`:
   `Ok(42)?` → `42`). This is the natural Ok/Err symmetry for `?` — both
   work at toplevel, Ok unwraps to the inner, Err surfaces as
   uncaught-exception. Choosing a different Err-shape (e.g. silent-push or
   wrap-at-toplevel) would break this symmetry.

3. **Sibling operator semantics support it.** `!!` is always-WRAP; the
   downstream `?` is the throw point. At function-position, `?` early-
   returns the Err carrier to the enclosing fallible fn — at toplevel, the
   natural extension is to surface to the host (uncaught exception). Same
   shape, different scope.

4. **The book is silent on bare-toplevel-`?` specifically**, but the §"##
   Uncaught Exception Display" path (L274-295) describes exactly the
   rendering machinery (`Uncaught exception: ...`) that
   `handle_exception`'s no-handler branch produces, with the same
   "high-level context with cause chain" shape that `!! + ?` composition
   naturally yields. The book documents the SINK; the test corpus + sibling
   semantics document the SOURCE.

5. **JOINT-FIX-1a's `!!` rewrite + JOINT-FIX-1b's `op_try_unwrap` Ok-path
   fix already partition the work** such that toplevel-`?`-throws is the
   single remaining gap to make the documented composition pattern
   (`expr !! "context"?`) work at toplevel — the same way it works inside
   a `fn main() -> Result<...> { ... }` fallible-frame body.

The book SHOULD be updated to name the contract explicitly (the §"## `?`
Try Operator" table's "early-return `Err(e)`" wording should add a row or
note for the toplevel case: "at script-toplevel: surface as uncaught
exception (renders via §Uncaught Exception Display)"). The book update is
out of scope for Phase B-fix but should be tracked as a documentation
follow-up.

### Why this is NOT (C) dual-mode

The supervisor binder noted: "If the audit surfaces a dual-mode-`?` design
(propagate in fn, throw at toplevel), that's a legitimate context-
distinction (frame-presence is a real structural difference, not a
defection-attractor) — but it must be NAMED as the contract in the audit
doc + book, not left implicit."

This audit considered (C) dual-mode and rejected it as a framing issue.
The semantics are SINGLE-mode: `?` on Err/None surfaces the Err carrier as
the enclosing fallible-scope's failure. The "scope" is fn-frame inside an
`fn`, and host-process at bare toplevel. The structural difference
(frame-presence) is in the *target* of the early-return, not in the
*semantics* of `?` itself. Naming this as "dual-mode" would suggest two
separate operator behaviors with different rules — but the rule is one:
"early-return Err to enclosing fallible scope". The fn-vs-toplevel split is
mechanical, not semantic.

### Why this is NOT (B) wrap-at-toplevel

(B) would require the 13 tests to be MIS-WRITTEN and need migration. The
explicit code-comment at `try_operator.rs:592-594` ("If ? is used at
top-level on an Err, it should be an uncaught exception") rules this out
— the contract is named in test corpus comments + asserted across 3 files
with 13 tests. Choosing (B) would require user-level scope-disposition to
move 13 tests out of v0.3.3 release-blocking scope, which contradicts the
v0.3.3 full-correctness disposition (per `project_v0_3_3_full_correctness_disposition.md`).

### Why this is NOT (D) genuine new language-semantics decision needing user ratify

(D) would apply if the book were silent AND the test corpus did not
disambiguate AND sibling operators did not resolve. None of those holds:
the test corpus is unambiguous (13 tests + explicit comment), and the
sibling-operator consistency analysis (c) shows throw-at-toplevel is the
natural extension. (D) would be appropriate if the audit found genuine
ambiguity — but it found consensus across three orthogonal signal sources
(tests, sibling ops, book uncaught-render path).

The book documentation update IS a follow-up — but the contract itself is
not a "new" language-semantics decision; it is the disambiguation of an
implicit shape that the implementation got wrong (silent-push-and-halt) and
the tests already named.

## Phase B fix-target preview

**Primary target:** `crates/shape-vm/src/executor/exceptions/mod.rs:679-709`
(`op_try_unwrap`'s Err / None / null-coded-None branches).

**Proposed shape:** at each early-return site, check whether the call stack
is empty BEFORE calling `return_value_inner`. If empty (bare toplevel),
route through `handle_exception` with the Err carrier as the payload (per
the `op_throw` precedent at `exceptions/mod.rs:326+`). Otherwise call
`return_value_inner` as today.

Pseudocode:

```rust
// Err(e) branch
let result_kind = value.kind();
let result_bits = value.slot().raw();
std::mem::forget(value);
if self.call_stack.is_empty() {
    // Bare toplevel — no enclosing fallible fn-frame. Surface as
    // uncaught exception to the host per the canonical contract
    // (audit 14a §c sibling consistency).
    let payload = KindedSlot::new(ValueSlot::from_raw(result_bits), result_kind);
    // Per op_throw: normalize_err_payload so the payload is a canonical
    // AnyError TypedObject; handle_exception's no-handler branch
    // produces `Err(VMError::RuntimeError(format!("Uncaught error:
    // {}", message)))` with the AnyError.message field.
    let normalized = self.normalize_err_payload(payload)?;
    return self.handle_exception(normalized);
}
self.return_value_inner(result_bits, result_kind)
```

Same shape for the None branch (`exceptions/mod.rs:699-702`) and the
null-coded-None branch (`exceptions/mod.rs:704-709`) — though the None
paths need to first construct the AnyError-wrapped Err carrier with code
`OPTION_NONE` per the book (L114): "None ... early-return `Err(AnyError)`
(code `OPTION_NONE`)". The function-position None branches today push the
None carrier directly and rely on downstream type-inference to coerce —
the toplevel branch can't defer, so it must wrap-then-throw.

Open questions for Phase B (NOT for this audit to resolve):

1. **Where to construct the OPTION_NONE AnyError**: in `op_try_unwrap` at
   the toplevel branch, or extract a shared helper that the
   function-position branches could also call (with the wrapper deferred
   today, applied at toplevel)?
2. **Normalize via `normalize_err_payload`** the existing Err carrier
   (Result(Err(_))) into a TypedObject AnyError for the throw path? The
   `!!` post-fix already produces a properly-built AnyError as the Err
   payload (`exceptions/mod.rs:580` via `build_any_error`), so for the 6
   Group A tests the carrier is already canonical — but a bare `Err("top
   level error")?` (Group B/C) may have a raw String payload that needs
   normalization. The `op_throw` precedent already handles this via
   `normalize_err_payload` (`exceptions/mod.rs:326-329`).
3. **Trace_info shape**: the toplevel-throw should produce a trace consistent
   with the book's "## Uncaught Exception Display" example. Today's
   trace_info_* builders return empty strings (per audit 07 §F4 comment),
   so trace is minimal — Phase B inherits that as-is; richer trace is a
   v0.4 follow-up.

**Sibling site to audit during Phase B fix:** `op_unwrap_option`
(`exceptions/mod.rs:744-783`) — its None branch returns
`Err(VMError::RuntimeError("called UnwrapOption on None value"))` directly
(no `handle_exception` path). That's a different opcode with a different
contract (pattern-checking site, not `?` operator), but it's the only other
opcode in `exceptions/mod.rs` that needs to surface to-the-host on
no-handler-present. The Phase B fix should NOT change `op_unwrap_option` —
it's already surfacing correctly via the RuntimeError shape; the question
is only whether the message string ("called UnwrapOption on None value")
should be normalized through the AnyError "## Uncaught Exception Display"
path. Out of scope for this audit; flagged for Phase B reviewer awareness.

**Will NOT introduce:**
- No new opcode for the toplevel case (no `TryUnwrapAtToplevel` — would be
  the same defection-attractor as W4-δ `ConvertBoolToString`; the
  compile-time toplevel-position information is NOT needed, the runtime
  `call_stack.is_empty()` check is the natural test).
- No `if call_stack.is_empty() { … } else { … }` parallel implementation
  across handlers — the structural test is at the single early-return site
  in `op_try_unwrap`, no carrier-shape boundary involved.
- No `ValueWord` / decode bridge / kind-blind handler — the Phase B fix
  operates entirely in the kinded-slot ABI per ADR-006 §2.7.10 / Q11. The
  Err carrier flows through `normalize_err_payload` + `handle_exception`
  with its kind preserved end-to-end.
- No `?` semantic change at function-position — that path stays exactly as
  it is today (sound per JOINT-FIX-1b close).

## Discipline notes

- Audit-only. No source changes in this phase. No commits during audit;
  team-lead commits the audit doc at close.
- No defection-attractor framings used in the recommended Phase B shape:
  no "toplevel bridge", no "throw probe", no "host-boundary helper", no
  "ValueWord shim". The fix is a single `is_empty()` check at three sites
  in one function, routing through the existing `op_throw`-precedent
  machinery (`normalize_err_payload` + `handle_exception`).
- Phase B fix is a SEPARATE dispatch after supervisor + user ratify these
  findings. The 6 Group A + 3 Group B + 4 Group C tests (13 total) are the
  Phase B close-gate test set; book documentation update is a separate
  follow-up.

## Close gates (Phase A audit)

1. `just check-clean` exit 0 (verified pre-write).
2. Audit doc at `docs/cluster-audits/v0.3.3/14a-q-toplevel-semantics-audit.md`
   with (a)(b)(c) sections + canonical contract decision (A) + Phase B
   fix-target preview.
3. No source / fixture changes (`git diff --stat HEAD` shows only this
   audit doc; AGENTS.md row committed by team-lead).
4. `git stash list` empty.
