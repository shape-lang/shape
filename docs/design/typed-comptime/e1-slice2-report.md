# E1 #17 slice-2 report — ParamId selection + fail-closed C0930

ADR-009 E1-D4 (user-ratified). Converts the headline defect — the silent
param-miss skips in the analysis signature-directive pre-pass — into a named hard
error. Branch `adr009/e1`, on top of the closed slice-1 (`bcb2db38`).

## C0930 allocation — verified free

Workspace-wide sweep (`crates/ tools/ bin/ docs/`): allocated capture/comptime
codes are `C0901`-`C0913`, `C0921`-`C0929`. `C0930` appears ONLY in decision docs
(E1-D4 designating it; the slice-1 report referencing it), never claimed by code.
`C0930` is used as ratified — no delta.

## What landed

- **`param_selection::resolve_param_id`** (`functions_annotations.rs`) — the SINGLE
  spelling→position resolution point against the frozen `FunctionDef`. Mints a
  **`ParamId`** whose field is private to the `param_selection` module, so a
  position can be obtained ONLY here; downstream indexes the resolved position and
  never re-resolves the spelling.
- **`apply_signature_directives_to_analysis_function`** — the two `set param`
  arms (`SetParamType`, `SetParamValue`) now call `resolve_param_id(...)` instead
  of `find(...) else { continue }`. A miss is **`[C0930]` `ShapeError::SemanticError`**
  (the slice-1 error-class precedent) carrying the **directive kind**
  (`set param type` / `set param value`), the **missing spelling**, and the
  **frozen callable's actual parameter list**. Return type changed
  `Result<(), String>` → `Result<(), ShapeError>`.
- **Caller** (`apply_function_comptime_signature_directives_for_analysis`) — passes
  the annotation name + handler location and propagates the `ShapeError` with `?`.
  The two NON-param-miss errors (override-conflict; scalar-default conversion)
  keep their exact prior message + `RuntimeError` class + handler-span location —
  no collateral reclassification.

The two silent-`continue` skip arms (pre-E1 `:396`/`:418`) are gone; both arms
fail closed.

## Error content (C0930)

```
[C0930] comptime `set param type` from @<annotation> on `<fn>` names parameter
`<spelling>`, which the frozen signature does not declare; its parameters are
[<p0>, <p1>, ...]
```

`ShapeError::SemanticError` (Display: `Semantic error: [C0930] …`). Destructuring
params (no simple name) render as `<destructured>` in the list.

## Tests (supervisor runs; filter `cargo test -p shape-vm --lib e1_param_selection`)

New file `functions_annotations/e1_param_selection_tests.rs`:

| Test | Covers |
|---|---|
| `set_param_type_on_a_declared_param_resolves_and_applies` | positive selection — declared param resolves + type applies |
| `set_param_type_on_an_undeclared_param_is_c0930_not_a_silent_skip` | negative — `[C0930]` w/ directive kind + spelling + param list; asserts NO mutation happened (fail-closed) |
| `set_param_value_on_an_undeclared_param_is_c0930` | negative — same single resolution point covers the value arm |
| `imported_handler_param_miss_surfaces_c0930_not_vanishes` | **the known hazard, end to end**: an IMPORTED (compiled) handler's `set param ghost` surfaces `[C0930]` through the full pass, not vanish |

The imported-handler pin uses the `imported_handler_resolution_tests` harness
pattern (compiled-annotation install + `install_semantic_freeze` +
`apply_function_comptime_signature_directives_for_analysis`).

## Differential — 2 currently-green tests flip (both rebaselined, reported, not silent)

Both are the ONLY `set param` sites in the workspace naming a nonexistent
parameter; every other site names a real param and is unaffected (`value`/`uri`/`x`
all resolve; the `x` override test is `#[ignore]`'d and its override path is
preserved byte-for-byte).

| Test | Classification | Action |
|---|---|---|
| `comptime/annotations.rs::ct_45c_set_param_typed` (`set param extra` on `greet(name)`) | **TRUE-POSITIVE — relied on the silent skip.** Its SURFACE doc explicitly documented "does not add a new parameter … body still rejects `extra` as undefined" — the exact silent-skip behavior E1-D4 converts. | Expectation `"Undefined variable: 'extra'"` → `"[C0930]"`; doc comment rewritten to state the fail-closed behavior. |
| `annotations_comptime/directives.rs::set_param_value_unknown_param_is_compile_error` (`set param missing` on `add(x)`) | **COLLATERAL — message change, still a compile error.** It already expected a hard error; the analysis pre-pass `[C0930]` now preempts pass-2's `unknown parameter` message. | Expectation `"unknown parameter 'missing'"` → `"[C0930]"`; comment added. |

No other test asserts the affected messages. (`referenced unknown parameter` —
the pass-2 message — has zero test assertions workspace-wide.)

## Out-of-scope observations (reported per E1-D4 scope discipline, NOT fixed here)

1. **Upstream handler-execution swallow** (`functions_annotations.rs:441`, in
   `apply_function_comptime_signature_directives_for_analysis`): a handler whose
   EXECUTION errors WITHOUT a `[comptime error]` marker is silently `continue`d.
   This is the C2 finding-3 "imported-annotation-handler failures swallowed"
   family — OUTSIDE the param-selection seam (it eats handler-execution failures,
   not directive param-misses). Per scope item 4 I fixed ONLY the param-miss
   visibility; this broader swallow is named main-side debt for a separate lane.
   (My param-miss fix is unaffected by it: a param-miss handler SUCCEEDS execution
   and reaches the seam, where `[C0930]` now fires — proven by the imported-handler
   pin.)

2. **Pass-2 re-resolves with a divergent message** (`functions_annotations.rs:3394-3437`,
   the install-phase directive application): its `set param` arms do their OWN
   `find(...)` and error `comptime directive referenced unknown parameter` — a
   SECOND resolution point with a different (untested) message. It already
   hard-errors (not a silent skip), so it is outside the ratified `:396/:418`
   scope; and the analysis pre-pass `[C0930]` now preempts it on the standard
   compile path. **Recommendation:** a small follow-up (or an append-only delta if
   wanted now) routes these two arms through `resolve_param_id` too, giving one
   `[C0930]` everywhere and making `resolve_param_id` the single resolution helper
   in the fullest sense of E1-D4. Low risk (no test asserts the pass-2 message).

## API fit (slices 3-5)

`resolve_param_id` slots at the same seam the emit-set consumers already feed;
`ParamId` is the position carrier they will hold instead of a spelling. The
slice-1 `CheckedBody` surface is UNTOUCHED (selection did not need it), honoring
the review's binding note. Any consumer-forced API change lands append-only with
re-review.

## Forbidden-patterns check

No dynamic fallback, no silent skip retained, no reparse. The fail-closed flip is
the opposite of the forbidden "silent skip / soft-fail counter for now" pattern.
`ParamId`'s private field enforces single-resolution structurally (the ProofGap
discipline), not by convention.
