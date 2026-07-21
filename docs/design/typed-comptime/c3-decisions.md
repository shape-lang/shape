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

## Slice-0 rulings (supervisor 2026-07-20, presented at the user-ordered
## pause; user ruled "continue now" — recommendations adopted, G9 disclosed)

- **C3-G9 (the Args carrier — S1-blocking fork resolved).** S0 proved G4's
  typed tuple does NOT exist at the value level (no heterogeneous literal,
  no index surface). Ruling: **(b′) per-param native carrier + `args` as a
  SPECIALIZATION-RESOLVED PSEUDO-TUPLE.** `args[i]` / `args.length` are
  template-level constructs resolved at specialization to the i-th typed
  param slot / a constant (constant indices only; non-constant index = a
  named rejection); polymorphic bodies address params through the
  pseudo-tuple, concrete bodies through their own declared params;
  args-mutation returns specialize to a COMPILER-INTERNAL per-target
  aggregate at the weave boundary (never user-visible). Runtime shape = the
  per-param form S0 measured VM==JIT zero-fallback. DISCLOSED refinement of
  the pause-presented lean: bare per-param args fails G4 (no aggregate for
  polymorphism/mutation); a first-class tuple surface would satisfy G4
  verbatim but holds C3 hostage to a general language feature — filed as
  **#63** instead (G0-aligned, wanted independently). The carrier is
  RUNTIME-PINNED VM+JIT as S1's FIRST stage before anything stacks on it
  (the S0 mandate).
- **C3-G10 (per-spec checking tier authority).** Emission tier + MIR
  battery, riding `ensure_monomorphic_function_for_callsite` (substitution
  + battery + hard-fail + cache). Genuinely new pieces: application-site
  error attribution naming BOTH signatures; Sig→ConcreteType glue; the
  concrete-body match-or-error comparison. Sig TYPES bind from the
  AST/inference side; the frozen CallableDescriptor is for IDENTITY only
  (reconstruct_type_annotation rejects Nominal/Record until B4/B5).
  Built BESIDE `compile_specialized_annotation_handler` (a G7 deletion
  target, not a foundation).
- **C3-G11 (G8 withdrawal confirmed).** S0 proved generic-target hooks are
  NOT uniformly broken today (type-agnostic hooks on single-type-param
  generics work: g1/g2/g4/g5). The G8 surface-and-stop rejection is
  therefore a DELIBERATE CAPABILITY WITHDRAWAL — confirmed anyway (the
  working cases work by accident of the homogeneous-args representation C3
  deletes). Obligation: defections.md entry lands WITH the rejection (S5),
  naming the withdrawn cases and the #59 re-arm condition.
- **C3-G12 (nested-fn annotation drop).** Annotations on fn-local nested
  functions are SILENTLY DROPPED today (S0 a4/a4c). C3 adds a LOUD named
  rejection at the application site; support = follow-up **#62**.
- **C3-G13 (implicit-capture rejection coding).** Pin as message-text now
  with a #60 routing note (the E1 precedent for comptime-builtin-layer
  diagnostics); revisit when #60's coded path lands.
- **JIT soundness fence (from S0, binding on S7 and all JIT work):**
  un-suppressing `mir_data` on the LEGACY weave is MEASURED-FORBIDDEN
  (hooks silently skipped: VM 40600 vs JIT 20500). Full-native comes ONLY
  via the generated typed-AST wrapper through the ordinary pipeline (the
  measured-green C3 shape). Any fallback-if-ever is classifier
  trampolining; never compile-failure demotion; async hooks =
  named-expected-fallback; S7 cells MUST execute hook paths.

- **C3-G14 (the @remote cut — A′; USER-RATIFIED 2026-07-21).** The S6
  classification collapse surfaced that autoloaded `std::core/remote.shape`
  (`pub annotation remote(addr)`: untyped config + `ctx.target` + the
  `{result:}` short-circuit) rides capabilities with no typed C3 spelling —
  the short-circuit is the HookDecision protocol, E4's charter, and a hook
  API without it is genuinely incomplete for the flagship patterns
  (cache/retry/remote). User principle applied: legacy earns nothing by
  incumbency; CUT cleanly and re-imagine on the sound design. Ruling:
  **A′ — C3's capstone deletes EVERYTHING including remote.shape's current
  implementation; @remote goes DARK; the distributed e2e tests that ride it
  are #[ignore]'d pointing at #68 (they are E4's acceptance suite); E4 #20
  re-implements @remote on the properly-designed typed HookDecision
  protocol as its first acceptance consumer and closes #68.** Zero legacy
  survives C3; no rushed protocol design inside C3; the E4 fence stands.
  Consequence for the S2-F3 E4-blocked ctx pins: retired at the capstone
  with the same #68 pointer (they pinned legacy-surface E4 capabilities).

## Operating rules

The E1/E2-proven pipeline carries over wholesale (AGENTS.md C3 row):
supervisor-only memory-capped lane; one writer at a time; FAILED-name-set
gates vs the S0-measured baselines (`-j1` for shape-test); Forbidden
Patterns at maximum binding; ALL workflow agents FABLE (user model ruling
2026-07-20); fresh-context capstone for S6; per-slice supervisor
double-check (self-reports are not evidence).
