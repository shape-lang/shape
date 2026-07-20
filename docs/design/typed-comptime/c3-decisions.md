# C3 #14 — Program of record (2026-07-20, grill session; ALL rulings USER-RATIFIED)

Binding for all C3 implementers/reviewers. Produced by a grill-with-docs
session over the phase-1 scout findings (scout workflow `wf_861a4911-cd7`,
5 scouts + synthesis, findings restated in the AGENTS.md C3 row). The grill
rulings SUPERSEDE the phase-1 draft decisions C3-D1..D8 where they conflict;
non-conflicting draft content is carried forward below.

## The reshaped charter

C3 builds the **metaprogramming primitive for annotation runtime hooks** — a
PUBLIC comptime API producing `CheckedTemplate<Sig, Captures>` — and every
other surface rests on it. This supersedes the IMPLEMENTATION reading of
Dec 65 ("annotations and comptime are independent"): the annotation feature
now rests on the comptime machinery; user-facing independence is preserved
via sugar (annotation users never write comptime). The Dec-95 staging
spellings (`hook.emit {}`, `body(captures){}`, `#ident`) are the E-track
SECOND PRODUCER of the same carrier, not C3's surface — dated user
disposition 2026-07-20 (satisfies the scope-reclaim rule).

## Grill rulings (C3-G0..G8, user-ratified 2026-07-20)

- **C3-G0 (design premise — load-bearing everywhere).** There are NO users;
  Shape's surface is greenfield. Compatibility carries ZERO weight (the 48
  green annotation pins are tests, not users). Criterion: ergonomics + LSP
  richness + compile-time checkability + architectural soundness.
- **C3-G1 (architecture).** Metaprogramming-first: the complete-for-hooks
  comptime API is the foundation; stdlib abstracts complexity; every other
  surface is a producer/consumer of the same carrier.
- **C3-G2 (layering).** The declarative `annotation name(config) {
  before/after }` block SURVIVES as sugar that LOWERS onto the public API.
  ZERO private side-channels: if desugaring ever needs a capability the API
  does not expose, the API is incomplete and must grow (the sugar test =
  the API-completeness gate).
- **C3-G3 (body construction).** Hook bodies are ORDINARY TYPED SHAPE
  FUNCTIONS referenced as templates (module-scope or comptime-local); the
  API wraps fn + Sig-binding + declared captures into `CheckedTemplate`.
  Quote/splice staged blocks stay E-track (later producer, same carrier).
  Code is written as code — never strings, never hand-built AST.
- **C3-G4 (signature polymorphism — language-semantics commitment).**
  Unified Sig-binding rule: at specialization the template's body fn is
  type-checked against the bound Sig (the frozen target CallableDescriptor).
  A GENERIC body instantiates per target (typed tuple `args`, tuple
  indexing/destructuring typed per specialization); a CONCRETE body is the
  degenerate case — match-or-error at the `@application` site naming both
  signatures. **Per-specialization checking** (errors at the application
  site, still compile-time) — NOT once-generic checking. `before` returns
  the typed `Args` tuple (typed args-mutation); `after` takes/returns the
  typed result. Config enters the body ONLY as ConstLift'd DECLARED captures
  (C1 CaptureClause) — no ambient scope; this is what makes [C0926] total.
- **C3-G5 (ConstLift domain — compositional).** Liftable = primitives
  (`int`, `number`, `bool`, `string`, unit) + tuples/arrays/`Option` of
  liftables, RECURSIVELY. In scope: heap-constant baking at specialization;
  Dec-95 rule 6 (lifted-constant identity → specialization hash) with
  STRUCTURAL equality for composite config values. Never-liftable per
  Dec 95: references, resources, capabilities, functions, provider grants,
  compiler descriptors, secrets, runtime handles — declaration-site named
  rejection listing the domain.
- **C3-G6 (JIT proof strength — B+).** Slice-0 spike MEASURES wrapped-MIR
  lowering (hook-bearing wrappers are `mir_data=None` by design at HEAD).
  Small → C3 lowers MIR from the wrapped definition (full native).
  Deep → parts-plus-pinned-seam: zero-fallback CLI proofs for the
  specialized handler + impl fn, whole-program VM==JIT value equality, and
  the wrapper cell pinned as an EXPLICIT named-fallback with loud-flip
  semantics + a named follow-up ticket carrying the measurement. Never a
  vacuous green in either direction.
- **C3-G7 (legacy fate — delete-in-C3).** The legacy hook-input machinery
  (homogeneous `args` array + its heterogeneous-signature hard error,
  name-keyed magic params, `self`-as-f64, per-invocation config eval) is
  DELETED WITHIN C3 under per-commit-green sequencing: build new path →
  flip the sugar's lowering → rewrite the 48 annotation pins onto the typed
  surface → pure-deletion commit (fresh-context capstone) → absence pins.
  One implementation exists at C3's close. E4 keeps its own charter
  (HookDecision, failure/retry state, ctx.state Any-deletion) — C3 does not
  touch it. The collision pins (injection.rs args-mutation family, nested
  `Array<Array<int>>` carrier pin, helpers.rs:8447) are resolved IN C3 as
  rewrites onto typed spellings.
- **C3-G8 (generic targets — surface-and-stop).** Installing a
  CheckedTemplate on a GENERIC target is a named rejection firing at the
  `@application` site (citing the #59 monomorphization-origin re-arm), NOT
  at template definition — signature-polymorphic annotations stay definable
  and usable on every concrete target. Lifted when #59 lands.
  Defections-logged.

## Carried forward from the phase-1 drafts (confirm-by-exception, unopposed)

- **Carrier home**: `comptime_fragments/checked_template.rs`,
  `pub(in crate::compiler)`, typestate construction chokepoint (finish()
  only on complete states, no string constructor), discharging the
  checked_body.rs:52-56 Sig/Captures deferral. Specialization output routes
  through BOTH the construction chokepoint AND the already-open C2
  InstallTransaction (E1-D6b atomicity-by-composition; CheckedReplaceBody
  shadow_export journaling precedent; never a second transaction; origin
  threaded as a parameter). Templates non-serializable (`#[serde(skip)]`
  inherited): documented in-code + follow-up issue.
- **Diagnostics**: [C0926] = the headline rejection (invocation-scope/
  ambient value entering a template outside its exact inputs). Further
  codes from C0931+ after an empirical census. C0913–C0921 untouchable
  (C2's block). Reuse [C0902] (borrow-mode capture), [C0907] (duplicate),
  [C0930] (param-miss) where they own the class. Every rejection pinned as
  code + exact sentence with a positive twin.
- **E4 fence**: failures/retry-state/cleanup-obligations, the before-result
  HookDecision protocol, and ctx.state are E4's — C3 must not half-migrate
  them. `ctx.target` stays per the E3-S4 ruling.
- **Naming**: the ConstLift Rust type is module-scoped (e.g.
  `const_lift::LiftedConst`) — never bare `ConstValue` (collision with
  monomorphization `call_site_consts` machinery). Do NOT build on the dead
  `comptime_concrete::ConstantValue` (its `Opaque` variant is
  ValueWord-shaped — Forbidden-Patterns defection).

## Slice plan (9 slices; S1–S4 + S6 review-mandatory)

- **S0 — spikes + fresh baselines** (no product code): fresh `-j1`
  measurements of all six suites (baselines of record, exact FAILED-name
  lists); SPIKE-JIT (wrapped-MIR lowering depth → decides G6); SPIKE-GENERIC
  (what generic-target hooks do today → sharpens G8); SPIKE-AMBIENT (what
  fires today on application-site-local reference); SPIKE-VMRED
  (helpers.rs:8447 root-cause + pre-declared arithmetic); C09xx next-free
  census; per-specialization-checking feasibility probe (can the C2 battery
  re-run a body against a bound Sig).
- **S1 — CheckedTemplate carrier + per-specialization checking core.**
- **S2 — the public comptime API** (template/hook builtins, E2 item_fn
  pattern; the sugar test defines completeness).
- **S3 — ConstLift** (compositional domain, heap baking, spec-hash rule 6).
- **S4 — grammar (typed config params) + sugar lowering onto the API**
  (annotation_def_params grows type annotations; ~8-file exhaustive-match
  fan-out).
- **S5 — exact signature-indexed inputs + rejection matrix** ([C0926] et
  al.) + pin-rewrite wave 1.
- **S6 — legacy deletion** (fresh-context capstone, A→deletion→B, pin
  rewrite completes, absence pins).
- **S7 — VM+JIT proofs** per S0's G6 measurement (CLI subprocess
  zero-fallback pattern; no `.with_jit().expect_output()`; non-vacuity
  guards mandatory).
- **S8 — LSP hover via the shared query surface** (C1 slice-4 precedent;
  generic view at declaration, specialized types at application) + book
  gate-runnable example + design-index row + defections.md + close (final
  3-lens panel, full gate, verify-merge).

## Operating rules

The E1/E2-proven pipeline carries over wholesale (AGENTS.md C3 row):
supervisor-only memory-capped lane; one writer at a time; FAILED-name-set
gates vs the S0-measured baselines (`-j1` for shape-test); Forbidden
Patterns at maximum binding; ALL workflow agents FABLE (user model ruling
2026-07-20); fresh-context capstone for S6; per-slice supervisor
double-check (self-reports are not evidence).
