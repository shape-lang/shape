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

- **E1-D6 (slice-1 scope ratifications; supervisor 2026-07-19, anchors
  independently verified).** (a) Slice 1 = the COMPILER-INTERNAL Rust
  builder in `comptime_fragments/` (`pub(in crate::compiler)`), NOT the
  Decision-95 Shape `body(captures){}` staging surface (C3/E-track) — the
  C2-D1 "beside `checked_body/`" phrasing pins this. (b) `finish()` =
  construction chokepoint (`Result<CheckedBody, ShapeError>`, provenance-
  ready, never a silent partial); #13's atomic "checks and installs" is
  discharged BY COMPOSITION with the shipped C2 validator +
  InstallTransaction at the consumer (the CheckedItem "provenance-READY,
  not yet reserved" precedent) — slices 3-5 MUST route through BOTH, never
  either alone; stated in the carrier docs; the review checks the
  composition claim. (c) Boundary: API-foundation-only — no consumer wired
  in slice 1; negative tests per rejection class; reviewer assesses API fit
  against the slice-3/4/5 emit sites; wiring-time API changes are
  append-only + delta re-review. (d) Typestate builder (`finish()` only on
  `<Present,Present>`); `[C0902]`/`[C0907]` reused with verified-faithful
  semantics; empty-body rejection deliberately UN-NUMBERED pending E1-D4's
  C0930 next-free computation (C092x follow-up).

- **E1-D7 (slice-5 scope — A-FULL; USER-RATIFIED 2026-07-20).** The E1-D5
  probe (report `f6000a07`) found Shape A FEASIBLE (overlay reachable at all
  three producer sites) with three divergences: `__ComptimeTypeRef`
  (builtin_schemas.rs:408) does NOT yet carry identity fields (the :423
  identity_high/low belong to `COMPTIME_FROZEN_TYPE_REF_SCHEMA`, a different
  schema); `ComptimeTarget` stores types as rendered STRINGS (AST discarded
  at from_function/from_type); `declaration_discovery.rs:101` pre-pass calls
  `from_type` without overlay scope. The user ratified **A-FULL in one
  slice**: composites included — AST canonicalized at from_function/from_type,
  FrozenTypeIdentity stored in ComptimeTarget, overlay threaded into the
  pre-pass, identity fields ADDED to `__ComptimeTypeRef`. Binding
  implementation rules: (a) **stamped→identity-only**: a type_ref carrying a
  stamped identity resolves via identity or fails with a NAMED error — no
  silent fallback to `.source` reparse (that fallback is the canonical
  walk-back shape); unstamped/legacy refs fall to the existing arms
  unchanged (dead-but-present until slice 6). (b) **ONE identity
  computation**: producer-side identity for composites must reuse the
  freeze's canonical hashing path — a second independent hasher is a
  parallel-implementation defection. (c) Reconstruction inverts the ONE
  `PRIMITIVE_SYNONYM_FAMILIES` table; composites recurse the descriptor
  algebra totally. Execution: supervised multi-agent workflow (ultracode) —
  read-only parallel scouts/verify lenses, SEQUENTIAL single-writer
  implement stages through the memory-capped lane, supervisor integration
  verify after.

- **E1-D8 (applied-generic boundary — stamp-gate; USER-RATIFIED 2026-07-20).**
  Slice-5 stage-1 PROVED (pins at `389c1940`; supervisor-verified at
  `semantic_freeze.rs:1122-1125`) that applied generic nominals
  (`Array<int>`, `Option<T>`, `HashMap<K,V>`) canonicalize to a Nominal
  identity but `payload_of` yields the pre-existing NAMED
  `applied_nominal_pending_rejection` — `substituted_applied_nominal` is
  None for builtin/generic heads. Reconstruction for them requires B4/B5
  applied-nominal substitution, OUT of E1's footprint. Ruling: **stamp-gate**
  — producers stamp FrozenTypeIdentity iff `reconstruct_type_annotation`
  (the SAME predicate the consumer uses) succeeds; the reconstructable
  frontier = primitives, Never, base `any`, Tuple, Reference, Union,
  Callable. Applied generics / records / bare user-nominals fall through
  UNSTAMPED to the `.source` reparse arm, which therefore stays **LIVE for
  exactly that class** (not deletable in slice 6) until B4/B5 lands, then
  E5 deletes it. A named follow-up issue binds the residual at E1 close.
  Composite e2e coverage = Tuple (Array<int> would force a false failure or
  a forbidden silent fallback). This narrows E1-D7's "composites included"
  to the provable frontier — user-dispositioned, not a walk-back.

## Operating rules

The E2-proven pipeline carries over wholesale (recorded in the AGENTS.md E1
registry row): supervisor-only build lane; one writer per worktree,
lifecycle-fenced; agents never build; per-slice gates judged by FAILED-name
sets vs the recorded post-E2 baselines (st-annotations 10-name, vmlib
7-name + `nested_exact` flapper, st-comptime 3-name at `-j1`, st-lsp 502,
lsp-lib 882); Forbidden Patterns at maximum binding — walk-back phrases and
bridge/probe/helper renames refused on sight.
