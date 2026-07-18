# E2 #18 — Supervisor decisions (2026-07-17, Fable supervisor)

Binding for all E2 implementers/reviewers. Phase-1 panel summaries live in the
AGENTS.md registry row (E2 program OPEN, 2026-07-17); the full phase-1 reports
were session-scratchpad artifacts lost to /tmp cleanup — load-bearing content
is restated here; anything deeper is re-derived from the repo directly.

## Decisions

- **E2-D1 (builder home).** E2 CONSUMES the internal C2 CheckedBody validator;
  the public builder + `finish()` surface is E1's (per the user-ratified
  C2 D1 amendment, tracked on #13/E1). E2 ships no public builder.
- **E2-D2 (probe ownership — ticket-inventory correction).** `__body_probe`
  is E2 scope (U03). `__type_probe`/`parse_type_annotation_payload` feed
  set-param/set-return (U02) and belong to E1/E5; the issue body's listing is
  an inventory error vs the U-class catalog. Do NOT delete `__type_probe` in
  E2. To be recorded in the #18 close report.
- **E2-D3 (splice form).** Dec 73 verbatim governs: checked splices at
  accepting positions with NO `$` sigil. `$label`-style spellings in older
  U03/U07 AFTER-examples are stale pre-Dec-73 text — corrected in E2's docs
  pass, never implemented.
- **E2-D4 (grammar scope, staged).** Slices 1–3 use the SINK-SUPPLIED typed
  path only (no pest/AST change). Standalone `item{}`/`module{}` blocks are a
  later slice AFTER parity, with the >10-file STOP applying; if that slice
  STOPs, the language-surface question escalates to the user.
- **E2-D5 (diagnostics).** E2 block = C0927+ (C0926 reserved for C3). Verify
  next-free empirically at implementation time.
- **E2-D6 (C0911 seam).** Slice 0 proves pre-analysis materialization
  feasibility with the existing InstallTransaction + GeneratedNodeIssuer
  machinery. If analyzer-ordering changes beyond that machinery are required:
  STOP → user. Pass-2 install + quarantine suppression is the named forbidden
  shape, refused in advance.
- **E2-D7 (parity gate).** The D6/D10 matrix rows + every currently-green
  smoke touching the legacy paths (measured at slice 0). The ignored D5
  replace-body-expr test stays rejected unless the matrix demands it. Book K3
  snippet included iff currently green.
- **E2-D8 (deletion discipline).** The deletion slice removes the WHOLE
  closed-under-callers U03/U07 inventory (as corrected by E2-D2) in ONE
  commit, only after parity + review. Walk-back phrases (CLAUDE.md §Forbidden
  rationalizations; esp. "keep parse_module_items_payload for stdlib until
  E6", any bridge/probe/helper rename, "keep JSON for serialization only")
  are refused on sight.
- **E2-Q1 (stdlib migration): USER-RATIFIED 2026-07-17 — OPTION A.** E2
  migrates the shipped stdlib consumers (serde/derive.shape,
  serde/serialize.shape, llm/tools.shape) off the `extend(source_string)` arm
  onto the typed fragment path within its own scope (slice 4). The U03
  deletion is TOTAL — no surviving source-reparse arm, no deferral to E6.
- **E2-D9 (module pre-analysis materialization): DEFERRED with a FLIP
  CONDITION.** Module-target handlers do NOT flow through the slice-0
  `materialize_computed_comptime_extends` discovery loop —
  `declaration_discovery.rs` excludes module targets BY DESIGN (they mutate
  module topology through separate pass-2 APIs); they run in pass-2 via
  `execute_module_comptime_handlers`, and `recheck_replaced_module_items`
  already re-runs `analyze_program_full` over the replacement. The C0911
  closure-fact gap is UNCONSTRUCTIBLE for slice 1's typed producer: `item_fn`
  mints only a literal-returning, closure-free function, and the legacy
  string route (the only closure-carrying route) never stamps generated
  provenance, so [C0911] cannot fire either way. Module pre-analysis
  materialization is therefore deferred to the `quote module` producer slice.
  **FLIP CONDITION:** when `quote module` lands, the closure-bearing pin is
  written FIRST; its result decides between the discovery-worklist topology
  change (which `declaration_discovery` explicitly declined — escalates to
  supervisor/user) and extended pass-2 fact publication. No improvised
  topology change before then.
- **E2-D10 (item_fn survival — inventory ruling): SURFACE PERSISTS, SCHEMA
  DIES.** `item_fn`'s SURFACE — the comptime builtin name + call signature —
  SURVIVES E2 as the CheckedItem constructor (deleting it would orphan D10
  parity while `quote item` is E-track-future). The U07 slice-5 deletion
  covers its INTERNALS: the `__ComptimeItemFragment` schema
  (builtin_schemas.rs:849), the sentinel fields,
  `literal_fragment_fields_from_slot` / `build_function_item_fragment` /
  `function_item_from_fragment`, and the source-reparse machinery. In slice 2
  `item_fn` produces the typed `CheckedItem` carrier instead of the sentinel
  fragment; the legacy fragment machinery stays byte-unchanged (dead-but-
  present) beside it until slice 5 deletes it whole (E2-D8 staging).
  Documented at the builtin.
- **E2-Q2 (serialize producer): USER-RATIFIED 2026-07-18 — PULL-INTO-E2.**
  serialize.shape's `@to_json` emits `extend {Type} { method to_json() ->
  string { <body> } }` where `<body>` is a comptime-COMPUTED f-string that
  interpolates `self.{field}` at RUNTIME. The slice-4 typed surface CANNOT
  express it — TWO gaps: (1) `item_fn` + `parse_extend_items_slot` yield only
  `Item::Function` (free functions); there is no typed `extend`-method /
  `Item::Extend` producer; (2) `item_fn` bodies are literal-only
  (`literal_expr_from_slot`: string/int/number/bool), whereas this body is a
  runtime-self-interpolating f-string template. A "body-as-text" producer would
  be source-reparse under a new name — a refused rename (CLAUDE.md §Forbidden
  rationalizations). So a NEW minimal typed producer (a single extend-method
  with a computed body, in the Dec-73 checked-splice form) is PULLED INTO E2 as
  review-mandatory **slice 4.5** — NOT deferred to the E-track future — which
  keeps E2-Q1-A totality intact (no surviving reparse arm, no carve-out, no
  deferral). Constraints: Dec-73 splice form (NO `$` sigil, per E2-D3),
  hygienic, ConstLift only; it is NOT a public builder API (that surface stays
  E1's per the user-ratified C2 D1 amendment); if implementation STOPs on
  grammar (the >10-file rule), it returns to the user as a designed feature
  slice. **Slice-4 scope correction:** serde/serialize.shape stays UNTOUCHED in
  slice 4 (only serde/derive.shape + llm/tools.shape migrate there); its
  migration lands in slice 4.5. **Ordering (binding):** slice-5's TOTAL U03
  deletion is BLOCKED on slice 4.5 completing — the deletion must not orphan
  serialize.shape's source-string emit.

## Slice plan (phase-1, condensed)

0. Pre-analysis materialization spike (EXECUTABLE; the STOP-decision slice;
   also measures the currently-green legacy-path smoke set = parity
   denominator). Review-mandatory.
1. CheckedModule / replace-module through the typed sink (D6 parity + JIT).
2. CheckedItem / item_fn replacement (D10 parity + NEW JIT proof).
3. Replace-body via C2 CheckedBody — flips the C0911 quarantine tripwire.
   Review-mandatory.
4. Stdlib migration (Q1-A): serde/derive + llm/tools onto `item_fn`
   (serde/serialize deferred to 4.5 per E2-Q2).
4.5. Serialize producer (E2-Q2, USER-RATIFIED): the minimal Dec-73 checked
   extend-method / computed-body splice producer, then migrate serde/serialize
   onto it. Review-mandatory; design-first (proposed minimal surface →
   supervisor sign-off → implement); grammar STOP escalates to the user.
5. TOTAL deletion, one commit. Review-mandatory. BLOCKED on 4.5 completing.
6. Battery + final independent review → verify-merge → merge → close #18.

## Key territory anchors (from phase-1, verified at 52fc13f8)

- Deletion surface (comptime_builtins.rs, ~1966 lines): reparse parsers
  :319 (`parse_function_body_payload`, `__body_probe` :325), :337
  (`parse_module_items_payload`, `__module_probe__` :342), :592
  (`parse_extend_items_slot`); `item_fn` :864; `__ComptimeItemFragment`
  schema builtin_schemas.rs:849 (issue's :268 cite is stale); fragment
  builders/readers :356/:419/:456/:514/:540; callers `__emit_replace_body`
  :1049, `__emit_replace_module` :1065, `__emit_extend_items` :1091,
  comptime.rs:302; emit-side statements.rs:700-738; AST variants
  statements.rs:62-75 (~19-file exhaustive-match fan-out).
- The directive carrier is ALREADY typed (`ComptimeDirective::{ReplaceBody
  {Vec<Statement>}, ReplaceModule/ExtendItems{Vec<Item>}}`) — E2's real work
  is replacing the mini-VM string/JSON TRANSPORT with a typed handle and
  deleting the schema.
- E1 latent collision: E1 rewrites the same `__emit_*`/
  `serialize_directive_payload`/`ComptimeDirective` enum — E1 must not start
  until E2 merges or territory is partitioned.
