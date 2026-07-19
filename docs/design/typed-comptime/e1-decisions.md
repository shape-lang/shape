# E1 #17 — Supervisor decisions (2026-07-19, Fable supervisor; USER-RATIFIED)

Binding for all E1 implementers/reviewers. All four decisions below were
presented as drafts at the phase-1 pause (user ruled "pause after the scout",
2026-07-18) and **ratified by the user 2026-07-19**. The full phase-1 scout
report was a session artifact; its load-bearing anchors are restated here and
in the AGENTS.md E1 registry row — re-verify every anchor at implementation
time, the repo is the authority.

## Scout territory (verified 2026-07-18; re-verify at impl)

- Emit side `statements.rs`: 3 surviving `serialize_directive_payload`
  callers (`:618` extend, `:652` set-param-type, `:684` set-return-type) +
  string-name emitters `:630/:663/:692`.
- Consumer side `comptime_builtins.rs`: `__emit_extend` :1247 (serde_json
  reparse), `__emit_set_param_type` :1277, `__emit_set_param_value` :1319,
  `__emit_set_return_type` :1357, `parse_type_annotation_payload` :286
  (+ `__type_probe` fallback :292),
  `type_annotation_from_string_or_type_ref_slot` :338 (reads
  `__ComptimeTypeRef.source`, reparses :373).
- Headline defect: **silent param-miss skips** at
  `functions_annotations.rs:396/:418`.
- C2-D1 user-ratified amendment (tracked on #13): the PUBLIC
  `CheckedBody<Sig,Captures>` builder + `finish()` does NOT exist yet — E1
  ships it greenfield beside `checked_body/`.
- U02 expr forms (`set return (…type_ref)`) resolve today by REPARSING
  `__ComptimeTypeRef.source`. B7 #11 IS merged (descriptor completeness
  available) but the FrozenTypeIdentity→TypeAnnotation reconstruction path's
  EXISTENCE is unproven. `.source` field deletion itself is E5's
  (U02/U04/U05); E1 needs the transport off reparse.

## Decisions

- **E1-D1 (slice 0 — reconstruction spike).** NEW slice 0: an executable
  spike proving or refuting FrozenTypeIdentity→TypeAnnotation resolution for
  exactly the expr-form corpus cases
  (`tools/shape-test/tests/annotations_comptime/type_mutation.rs:297/:323`).
  The spike's verdict decides slice-5's shape: full-E1 U02 vs a ruled E1↔E5
  split. Escalate to the user only if the verdict is "impossible without new
  machinery".
- **E1-D2 (slice plan).** The scout's 6-slice plan is adopted behind the
  spike:
  1. Public `CheckedBody` builder + `finish()` (C2-D1 amendment discharge) —
     review-mandatory.
  2. ParamId selection + fail-closed misses (per E1-D4).
  3. U01 literals → store+index.
  4. extend → typed carrier.
  5. U02 expr per the slice-0 spike verdict.
  6. TOTAL deletion, one commit, review-mandatory (E2-D8-style discipline:
     per-commit-green A → pure-deletion → B sequencing, fresh-context
     capstone handoff, workspace-wide both-spelling closure sweeps).
- **E1-D3 (no rename).** NO `RewritePlan` enum rename — `ComptimeDirective`
  already realizes the plan. Refactor-for-naming is out of scope.
- **E1-D4 (param selection semantics).** Resolve spelling→position ONCE
  against the frozen callable; a miss is a named hard error **C0930**
  (verify next-free empirically at implementation time), tested against the
  known swallowed-imported-annotation-handler-failure hazard. This converts
  the `functions_annotations.rs:396/:418` silent skips into hard errors.

## Supervisor rulings (within ratified bounds)

- **E1-D5 (slice-5 shape — probe-first; supervisor 2026-07-19, within
  E1-D1's ratified frame).** Slice-0 verdict: **PROVEN for the corpus**
  (gated GREEN at `7a4e9809`, 4/4 pins). The leaf/composite boundary (pin 4:
  `identity_of("Array<int>")` = None) means slice 5's shape is decided by a
  READ-ONLY probe at its top: is the `FreezeOverlay` available at
  `comptime_target` build time? If yes → **Shape A** (producer stamps the
  FrozenTypeIdentity onto the type_ref via the already-declared
  `identity_high/low` fields, `builtin_schemas.rs:423`) — full-E1 U02 off
  reparse. If no → **Shape B** (leaf now, composite = the ruled E1↔E5 split
  E1-D1 contemplated), surfaced to the user at the slice-5 gate before
  execution. No pre-commitment; the probe result decides. Slice-5 build-outs
  either way: thread `Arc<FreezeOverlay>` into `comptime_builtins_module_base`
  consumers, and write the total `FrozenPayloadDescriptor -> TypeAnnotation`
  reconstruction fn (primitive spellings invert the ONE
  `PRIMITIVE_SYNONYM_FAMILIES` table — no second name table).

## Operating rules

The E2-proven pipeline carries over wholesale (recorded in the AGENTS.md E1
registry row): supervisor-only build lane; one writer per worktree,
lifecycle-fenced; agents never build; per-slice gates judged by FAILED-name
sets vs the recorded post-E2 baselines (st-annotations 10-name, vmlib
7-name + `nested_exact` flapper, st-comptime 3-name at `-j1`, st-lsp 502,
lsp-lib 882); Forbidden Patterns at maximum binding — walk-back phrases and
bridge/probe/helper renames refused on sight.
