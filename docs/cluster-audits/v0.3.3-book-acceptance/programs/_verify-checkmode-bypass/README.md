# CheckMode bidirectional Match constructor-pattern ownership BYPASS

Adversarial-verify finding (2026-06-22, read-only verifier).

## Headline
Survives FIX A/FIX B (commit 3eda1c8d). A constructor pattern (`Some(n)`/`Ok(...)`)
over a NON-enum scrutinee is silently accepted — and its binder laundered as a
raw heap pointer — WHEN the match is in **return/tail position of a fn with a
declared return type**.

CATASTROPHIC repro: `HOLE_match_return_position_struct_reinterpret.shape`
  rc=0, nondeterministic raw-pointer output (e.g. 110839871478836), VM==JIT both leak.

## Trigger (both required)
1. enclosing fn has an explicit `-> T` return type annotation, AND
2. the match is the return value (`return <match>` arm-expr OR match as annotated tail expr)

Statement-position matches, let-bound matches, match-as-arg, and unannotated-return
fns all REJECT correctly (control files `ctrl_*`).

## Root cause
`check_against` Match arm at
  crates/shape-runtime/src/type_system/inference/bidirectional.rs:141-154
infers the scrutinee (line 142) but DISCARDS it (`_scrutinee_type`) and calls the
scrutinee-LESS `bind_pattern_vars(&arm.pattern)` (line 148) =
`bind_pattern_vars_typed(pattern, None)`. With scrutinee=None,
`check_constructor_pattern_ownership(None, variant)` cannot prove non-enum
(surface-and-stop) and accepts the constructor pattern, binding the payload var to
a fresh unknown.

The `infer_expr` Match path (expressions.rs:1291-1302) applies substitutions to the
scrutinee and passes `Some(&scrutinee_type)` to `bind_pattern_vars_typed` — so the
ownership check fires there. The bidirectional value/return-position path does not.

## Suggested fix shape (team-lead)
In bidirectional.rs check_against Match: apply substitutions to the inferred
scrutinee and pass `Some(&scrutinee_type)` to `bind_pattern_vars_typed`, mirroring
expressions.rs:1291-1302. Do NOT relax check_constructor_pattern_ownership.
FP-clean counterpart `fp_legit_option_payload_OK.shape` (Result<Option<int>,string>)
must still compile+run (prints 107) after the fix.
