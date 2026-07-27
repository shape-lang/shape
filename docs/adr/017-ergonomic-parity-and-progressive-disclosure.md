# ADR-017: Ergonomic Parity and Progressive Disclosure

## Status

Proposed 2026-07-27 (pending ratification).

Composes with ADR-011 through ADR-016. ADR-016 §10 owns the gate and manifest
representation of the script tier and ceremony budgets; this ADR owns the
language rules they measure. Sugar defined here is presentation over the
mechanisms of ADR-011/012/014/015 and creates no separate semantic authority.

## Context

Shape's goals are not only strict typing, verified lifecycle, and truthful
distribution. The language was explicitly designed to feel like a scripting
language at the entry level: script-mode top-level execution, `var` with
smart-default storage classes, full inference inside function bodies, and
object literals with a dynamic feel. ADR-011 through ADR-015 add ceremony that
is truthful and necessary at boundaries — declared contracts for annotated
callables, effect rows, linear remote outcomes, finite retry budgets — and
every one of those mechanisms was designed assuming a convenience layer above
it.

The risk is structural, and this repository has already lived it once: the
Book gate was historically deferred until a late documentation phase, and
ADR-016 exists because deferred obligations drift. Sugar has the same failure
shape. A mechanism ticket is acceptable without its sugar; the sugar is
"follow-up"; the follow-up never schedules; the shipped language is the
mechanism layer. The result would be a 9/10 architecture wearing 5/10
ergonomics — a defect against the language's stated goals, not a taste
disagreement.

This ADR makes ergonomic parity an acceptance rule rather than an intention.

## Decision

### 1. Sugar parity is an acceptance rule for public ceremony

A slice that adds required ceremony to a public surface — a mandatory
declaration, annotation, consumption obligation, or structured refusal that
user code must now write or handle — must name its ergonomic counterpart in
the same ticket: a sugar form, an inference rule, or a machine-applicable
fix. The slice's feature cannot reach `public` status (ADR-016 §2) until the
counterpart lands, or the user records a dated waiver naming what ships
without it and why.

The counterpart always lowers to the mechanism. Per ADR-011/012, sugar cannot
select semantics by spelling, cannot bypass resolution or type checking, and
cannot create a second authority. If a proposed convenience cannot be
expressed as a lowering to the existing mechanism, it is a design gap in the
mechanism and returns to design, not a special path.

### 2. The script tier is a defined, gated feature set

The script tier is the enumerated set of features with which entry-level
Shape is written: script-mode top-level execution, `let`/`var` bindings with
smart-default storage, full inference inside function bodies, unannotated
internal callables with inferred results (ADR-011) and inferred effect rows
(ADR-014 §8.2), literals, collections, control flow, pattern matching,
string interpolation, and ordinary error handling with `Result` and `?`.

A script-tier program requires zero effect-row declarations, zero ownership
or linearity annotations, and zero capability ceremony. Advanced concepts
appear only when their features are used: the first annotation requires a
declared contract; the first remote call introduces certainty; the first
uncertain outcome introduces obligations. Progressive disclosure is the
mechanism by which a strict language stays learnable, and it is gated
mechanically per ADR-016 §10 rather than asserted in prose.

The `var` smart-default story must become true before it is claimed: today a
flag forces every `var` binding to `SharedCow` regardless of aliasing or
mutation, and the LSP's storage-class inlay hint is a default-off local
heuristic that can disagree with the compiler. The script-tier feature entry
covers both: the refined storage rule (promote only when aliased and
mutated) replaces the forced default, and the inlay hint becomes a
projection of the compiler's own storage decision through the shared
semantic query, never an LSP-side guess.

### 3. Quasiquotation is v1 surface for typed construction

Comptime item construction gains quasiquote templates in the first version of
the construction surface, not as a later convenience. A template is ordinary
Shape syntax written inside the generator and parsed by the ordinary parser
at the generator's own compile time — it is the generator's source, never a
runtime string. The preserved `item_fn` parsing path is the seed for
template parsing; string reparse of rendered text remains deleted and
forbidden.

Template holes are typed:

- a name hole accepts only `GeneratedName`;
- a type hole accepts only `TypeRef` or an explicit `TypeParamRef`;
- an effect-row hole accepts only a closed row or an explicit
  `EffectParamRef` (ADR-014 §8.3), so effect-parametric generic signatures
  are quasiquotable;
- an expression or statement hole accepts only a checked typed fragment.

The ratified surface (2026-07-27) is `quote { ... }` with `${hole}`
splices — `${}` chosen because bare `{}` collides with blocks and with
f-string interpolation. Expansion lowers the filled template through the
same `ItemFn`/`ItemType` builders and canonical interner defined by
ADR-012 §3, with hygiene by default. Concatenating or interpolating strings never forms syntax, and a
template cannot smuggle spelling-based authority: whatever the template
produces enters ordinary resolution, discovery, and checking like any
builder-constructed item.

### 4. Boundary ceremony has machine-applicable fixes

The structured diagnostic format carries optional machine-applicable edits,
and compiler and LSP project the same fix facts from the same semantic query
(ADR-011 §6). The canonical initial fix set:

- materialize an inferred signature on an annotated or exported callable
  (`NormalizedDeclaredContract` from the inferred facts);
- materialize an inferred effect row at a boundary (ADR-014 §8.2);
- insert the explicit numeric conversion the strict numeric rule requires;
- expand an exhaustive consumption skeleton for a linear outcome type;
- rewrite `join all` to `join settle` where uncertainty is possible
  (ADR-015 §10).

A fix is evidence-backed: it applies the semantic facts the compiler already
proved, never a guess. Fix-its are part of the slice that introduces the
diagnostic, per §1.

Fixes are single-sourced at the diagnostic emitter. The structured diagnostic
schema gains an appended structured-edit field — exact spans plus replacement
text — alongside the existing free-text diff; the LSP converts structured
edits to `TextEdit` mechanically and adds nothing. The current LSP-side fix
derivation (substring extraction from rendered message text) and the LSP's
parallel hand-written validators are the same dual-authority defect ADR-011 §6 names,
and they are migration debt: each compiler-emitted structured fix deletes its
scraping counterpart, and each validator's rule either moves into the shared
semantic query or is deleted, on a shrink-only baseline.

### 5. Supervisor scopes make the common remote call one character

`with supervisor <expr> { ... }` (spelling user-ratified 2026-07-27; the
`with` form establishes a general scoped-context pattern the language may
reuse) establishes a **lexically** scoped durable supervisor (ADR-015 §6). Inside the scope, the `?` operator is defined on
`RemoteCallOutcome<R>`, and every arm propagates through one mechanism —
ordinary `Result` propagation:

- `Completed(R)` unwraps to `R`;
- `Failed(RemoteError)` propagates as `Err`;
- `Uncertain(_, obligation)` causes the evaluator — not user code — to run
  the scope supervisor's sealed `accept`:
  - `Accepted(receipt)`: the obligation is transferred, which is precisely
    the condition under which ADR-012 §7 legalizes a `Result` projection.
    The call site propagates a typed `Err` whose payload carries the
    uncertainty evidence and the transfer receipt; the receipt is also
    retained in cleanup evidence. Callers catch settled failures and
    accepted-uncertain outcomes through the same mechanism, distinguished
    by the typed error payload — no second failure channel;
  - `Refused(obligation, _)`: the still-linear obligation escalates outward
    through enclosing supervisor scopes' `accept` surfaces. If every
    lexically enclosing scope refuses, the evaluator retains the obligation
    in the episode and the fail-closed recovery-pending behavior of ADR-015
    applies. Because that path can suspend the frame, `?` on
    `RemoteCallOutcome` contributes `Suspend` (in addition to the call's
    own `Remote` atom) to the enclosing contract row at contract-
    elaboration time — declared before freeze, never discovered after it;
  - `AcceptancePending(pending)`: **not** an escalation case. Per ADR-015
    §6 the pending handle is a different linear value bound to the
    supervisor whose storage outcome is ambiguous; it must resolve, be
    returned, or be handed to another durable recovery owner **at the scope
    that produced it**. The evaluator drives that resolution; handing the
    underlying obligation to an outer supervisor while the first
    acceptance may still land would risk two durable owners for one
    transfer and is forbidden.

Closures interact with the scope lexically, and no capture exemption is
needed: ADR-014 §4 classifies `DurableSupervisor` as unrestricted —
possession of a supervisor handle owns no obligation, so ordinary closure
capture applies. A closure written inside the scope captures the supervisor
like any unrestricted value; `?` inside that closure uses its lexically
captured supervisor wherever the closure later runs, so
`items.map(|i| fetch(i)?)` works, and an escaping closure remains
well-defined rather than becoming an error. "Outside a supervisor scope"
means lexically outside every scope, with no captured supervisor in reach —
only there is `?` on a linear outcome a compile error.

The sugar adds no authority and no new outcome: it automates exactly the
accepted-transfer path the ADRs define, through the sealed acceptance
surface. There is no ambient default supervisor; the scope is explicit, and
nesting forms an explicit escalation chain for refusal only. Outside a
supervisor scope, `?` on a structurally linear outcome is a compile error
whose diagnostic names both alternatives: exhaustive consumption, or an
enclosing supervisor scope.

### 6. Diagnostics teach at concept boundaries

The first time a program crosses from the script tier into an advanced
concept — a linear value, an effect-row boundary, execution uncertainty, a
retry budget — the structured diagnostic carries the stable concept identity
of that concept. Concept identities are owned by the diagnostic catalog, are
permanent and tombstoned like feature identities, and are mapped to owning
Book sections through the coverage manifest (ADR-016 §10). A diagnostic that
introduces a concept without a resolvable concept identity fails the gate.

## Grounding (2026-07-27)

- The structured-fix pipe exists but is disconnected at both ends:
  `SuggestedFix { label, diff, confidence }` in
  `crates/shape-diagnostics/src/lib.rs:185` has exactly two emitters (both
  diff-less, `compiler/functions.rs:1276,:1280`) and zero LSP consumers —
  `shape-diagnostics` is not even a dependency of `tools/shape-lsp`. The LSP
  instead dispatches on diagnostic codes with substring-scraping of rendered
  message text as its fallback
  (`tools/shape-lsp/src/code_actions.rs:587-615`) and carries eleven
  hand-written parallel validators (`tools/shape-lsp/src/analysis.rs:70-81`).
  The schema is explicitly append-only, so the structured-edit field is a
  compatible addition.
- Machine-applicable `WorkspaceEdit` quick-fixes already work in the LSP
  (`code_actions.rs:101-548`), so §4 is a re-sourcing of fix authority, not a
  new capability.
- Quasiquote has no prior art in the grammar; the surviving string→AST
  reparse (`comptime_builtins.rs:547` `parse_type_annotation_payload`,
  reached from the bare-string arm at `:621-638`) is a documented BLOCKED
  item retained only because no typed alternative exists for `item_fn`'s
  string-typed returns. §3's typed-hole templates are that alternative: the
  string arm and its ~19 dependent tests migrate to typed holes, unblocking
  the deletion E5 deferred.
- The current generation surface is primitive — `item_fn` builds zero-param,
  literal-body functions only (`comptime_builtins.rs:1048-1083`) — so §3
  rides the construction-API lane rather than wrapping today's builtins.
- The `var` force-`SharedCow` flag is
  `crates/shape-vm/src/mir/storage_planning.rs:37-42` (row 1b); the refined
  aliased-and-mutated rule already exists behind it (row 3). The heuristic
  inlay hint is `tools/shape-lsp/src/inlay_hints.rs:446-468,1083-1152`; the
  read-only compiler-table projection pattern to copy is
  `tools/shape-lsp/src/expansion_views.rs`.
- Script mode, `var`/`let`/`let mut`, ownership modifiers, and object field
  hoisting are implemented
  (`shape.pest:848-850`, `crates/shape-ast/src/ast/program.rs:85` and
  `:104`, `crates/shape-runtime/src/type_system/inference/hoisting.rs`), so
  the script tier is an enumeration-and-gate task, not new machinery.

## Consequences

- Mechanism tickets grow a named ergonomic counterpart, and "sugar later"
  becomes a recorded user decision instead of a silent default.
- The script tier becomes falsifiable: a semantics change that leaks ceremony
  into entry-level code turns the Book gate red.
- Quasiquote ships with the construction surface, so annotation authors never
  pass through a builders-only era whose habits then have to be unlearned.
- The common remote call is `let r = fetch(x)?` inside a supervisor scope,
  while every certainty and ownership guarantee of ADR-012/015 holds
  unchanged underneath.
- The compiler must expose fix facts and ceremony counts as structured
  queries, slightly enlarging the semantic-fact surface of ADR-011 §6.

## Rejected alternatives

- **Make linearity or effect checking advisory to reduce ceremony.** That
  deletes the guarantees this program exists to build. Ergonomics is achieved
  over the verifier, never under it.
- **Give sugar its own semantics.** A second authority is exactly the
  spelling-based defect class ADR-011 removes; every convenience lowers.
- **String-based templating.** Rendered-text reparse is deleted; templates
  are parsed generator source with typed holes.
- **An ambient default supervisor.** Implicit transfer of a linear obligation
  is hidden authority — compiler magic in ADR-011 §3 terms. The scope is
  explicit.
- **Defer the sugar layer to a post-program cleanup wave.** The Book gate's
  own history (ADR-016 §Context) shows deferred obligations drift; parity is
  enforced per slice.
- **A general style linter as the ergonomics gate.** Budgets and ceremony
  counts apply to designated flagship documentation (ADR-016 §10), not to
  arbitrary user code; the language does not police style.

## Related decisions

- ADR-011: Resolved Semantic Identity and Typed Elaboration
- ADR-012: Verified Annotation Elaboration and Callable Transforms
- ADR-014: Closed Effects and Static Capability Ownership (§8)
- ADR-015: Recovery Episodes and Durable Obligation Journal
- ADR-016: Executable Public Feature Documentation (§10)
