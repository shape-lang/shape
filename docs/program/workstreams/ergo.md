# ERGO workstream — ergonomic parity

Authority: ADR-017, ADR-016 §10, ADR-014 §8.2, R23; open rulings answered
2026-07-27 (grill Q1/Q4 — lane B = FIX-CHANNEL + VAR-TRUTH first;
`quote`/`${}` and `with supervisor` spellings ratified). Charter: the mechanisms
of ADR-011–015 ship with their convenience layer, gated, so the language's
script-feel goal survives the correctness program.

## Tickets

### ERGO-FIX-CHANNEL — single-sourced structured fixes

Scope: append a structured-edit field (exact spans + replacement text) to
`shape-diagnostics` (`SuggestedFix`, `crates/shape-diagnostics/src/lib.rs:185`;
schema is append-only). Make `tools/shape-lsp` a consumer of
`shape-diagnostics` (today: not even a dependency) converting structured
edits to `TextEdit` mechanically. Generate shrink-only baselines for the
LSP's message-scraping fix-extractors (`code_actions.rs:587-615`) and its
eleven parallel validators (`analysis.rs:70-81`); each migrated
fix/validator reduces them.
Blocked by: none. Blocks: ERGO-CONTRACT-FIXIT, ERGO-TEACHING-DIAG.
Tripwires: (1) a compiler-emitted fix applies byte-identically through CLI
`--json` and through LSP code action on the same source; (2) deleting one
regex extractor turns its baseline red if any code path still depends on it;
(3) a fix with stale spans (source edited since) is rejected, not misapplied.

### ERGO-CONTRACT-FIXIT — materialize signatures and effect rows

Scope: machine-applicable fixes for "annotated/exported callable needs a
declared contract" (materialize from inferred facts) and "boundary needs an
effect row" (materialize inferred row; `! {}` when pure). One keystroke in
LSP; `--fix`-style application in CLI.
Blocked by: ERGO-FIX-CHANNEL; #91 (callable facts); the row fix additionally
by the R21 row-in-type slice (PERF-independent; see EFFECT lanes).
Tripwires: (1) fix output re-checks clean with zero diagnostics on the
canonical tracer fns; (2) the materialized row equals the checker's inferred
row exactly (asserted against the semantic fact, not string comparison);
(3) applying the signature fix to a generic callable preserves explicit
`TypeParamRef` binders (no `unknown` leakage — regression vs the
`Type::to_annotation()` TypeVar-loss constraint).

### ERGO-VAR-TRUTH — real `var` smart defaults, truthfully surfaced

Scope: retire the force-`SharedCow` flag
(`crates/shape-vm/src/mir/storage_planning.rs:37-42`) so the refined
aliased-and-mutated rule (row 3) decides `var` storage; replace the LSP's
heuristic storage-class inlay (`inlay_hints.rs:446,1083`) with a projection
of the compiler's own storage decision (pattern:
`expansion_views.rs`); default the hint on for `var`.
Blocked by: none (joint with PERF-RC-ELISION, which needs the unpinning).
Tripwires: (1) blast-radius module diff vs main for the flag flip (per the
standing regression-scope ruling), with every storage-class change
classified; (2) LSP hint text equals the storage planner's decision on a
matrix of aliased/mutated/captured cases — a divergence fails the test, not
the reader; (3) `var` bindings that qualify now take the `LoadLocalMove`
path (asserted on emitted opcodes).

### ERGO-QUASIQUOTE — typed-hole templates over the construction API

Scope: quasiquote templates parsed as generator source with typed holes
(name→`GeneratedName`, type→`TypeRef`/`TypeParamRef`, expr/stmt→checked
fragments), lowering to the #106 typed builders. Migrate the
`item_fn`/`extend_method` string-typed-return arm and its ~19 dependent
tests to typed holes; delete `parse_type_annotation_payload`'s bare-string
arm (`comptime_builtins.rs:621-638`) — the deletion E5 CKPT-4 deferred for
lack of a typed alternative.
Blocked by: #106 (typed construction API). Blocks: the #110 deletion path
for the string arm.
Tripwires: (1) a template with a misspelled hole name is a compile error at
the generator, with the template-local span; (2) generated items from
templates carry the same `ExpansionIdentity` facts as builder-constructed
items (identity tests, not output comparison); (3)
`no_json_comptime_protocol.rs` sentinel extended: zero string→AST reparse
paths remain reachable from comptime builtins.

### ERGO-SUPERVISOR-SCOPE — `with supervisor` and `?` on remote outcomes

Scope: ADR-017 §5 exactly — lexically scoped supervisor, `?` on
`RemoteCallOutcome<R>` with uniform `Result` propagation (accepted transfer
projects to a typed `Err` carrying uncertainty evidence + receipt),
evaluator-run acceptance, refusal-only outward escalation
(`AcceptancePending` resolves at its producing scope, never escalates),
`Suspend` contributed to the enclosing row at contract elaboration, compile
error with teaching diagnostic outside a scope, and the non-escaping-
closure capture exemption.
Blocked by: DURABLE-SUPERVISOR (#157) and the ADR-012 §11 step-8 `@remote`
rebuild slice.
Tripwires: (1) fault-injected `Refused` and `AcceptancePending` cases prove
the obligation is never dropped and never double-owned (finalization
observers, per ADR-015 §9) — including the two-owner race: a pending
acceptance that later lands while escalation was attempted must be
impossible by construction; (2) the sugar's lowering is bit-identical in
semantic facts to the hand-written exhaustive consumption it abbreviates;
(3) `?` outside a scope — lexically outside every scope with no captured
supervisor in reach — names both alternatives in the structured payload;
(4) closure capture of the supervisor is ordinary (`DurableSupervisor` is
unrestricted per amended ADR-014 §4): a `map` closure inside the scope
compiles, and an escaping closure remains well-defined, with `?` using its
lexically captured supervisor wherever it runs.

### ERGO-TEACHING-DIAG — concept identities in diagnostics

Scope: stable concept identities in the diagnostic catalog; first-crossing
diagnostics for linearity, effect boundaries, uncertainty, and retry budgets
carry them; coverage-manifest mapping per ADR-016 §10.
Blocked by: ERGO-FIX-CHANNEL (shared catalog work); BOOK-CONTRACT (#113).
Tripwires: (1) an unmapped concept identity in gated evidence fails
BookTruthGate; (2) concept identities survive diagnostic rewording
(asserted on identity, not message).

### ERGO-CEREMONY-GATE — mechanical ceremony counts and budgets

Scope: compiler emits the ceremony-count structured fact per compiled fence
unit; harness enforces `ceremony: none` and flagship budgets per ADR-016
§10. shape-web side: fence metadata, flagship set, ratchet.
Blocked by: GATE-CLI (#118), GATE-MODES (#119).
Tripwires: (1) moving ceremony into hidden setup code still fails (fact
covers the compiled fence unit); (2) loosening a budget without a review
reason fails manifest validation; (3) the script-tier chapter compiles green
with zero ceremony end-to-end.

## Sequencing

ERGO-FIX-CHANNEL and ERGO-VAR-TRUTH are immediately startable after
ratification. ERGO-QUASIQUOTE tracks #106. ERGO-SUPERVISOR-SCOPE is the
latest (post-journal, post-remote-rebuild). Waiver rule: any mechanism slice
reaching `public` before its ERGO counterpart requires the dated user waiver
of ADR-017 §1 — recorded, not assumed.
