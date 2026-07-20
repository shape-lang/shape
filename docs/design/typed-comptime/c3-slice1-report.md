# C3 #14 slice-1 report — CheckedTemplate carrier + per-specialization checking core

Landed on branch `adr009/c3` atop `c2297946` as four gated stage commits —
`aa7d7f88` (S1a carrier runtime pin) → `7aa0acff` (S1b carrier +
pseudo-tuple classifier) → `59020f45` (S1c concrete core + attribution) →
`8adf8033` (S1c polymorphic arms + G9 resolution) — plus the verify-round-1
fix commit `25f196d1` (append-only; the gated stage hashes are never
amended). Authority: `c3-decisions.md` (C3-G0..G13 + the slice plan),
`c3-slice0-report.md` (§7 feasibility + §8 deltas), CLAUDE.md Forbidden
Patterns + ADR-006 at maximum binding.

Slice-1 charter delivered: (S1a) the G9 carrier runtime-pinned VM==JIT
zero-fallback BEFORE anything stacked on it (the S0 mandate); (S1b) the
`CheckedTemplate` carrier module (typestate construction chokepoint,
discharging the `checked_body.rs:52-56` Sig/Captures deferral); (S1c) the
per-specialization checking core riding
`ensure_monomorphic_function_for_callsite` (C3-G10 emission tier + MIR
battery) with two-signature application-site attribution, the concrete
match-or-error rule, and the G9 pseudo-tuple resolution. NOT in S1 (by
plan): the public comptime API (S2), ConstLift (S3), sugar/grammar (S4),
any deletion (S6). The new seam has NO production caller until S2.

## S1a — the G9 carrier runtime pin (`aa7d7f88`)

Fixtures + test file only; no compiler code touched.

- `tests/smokes-jit-closure/c3-carrier-per-param.shape` — 1-ary bare-value
  mutation carrier, both hook kinds, 200-call hot loop crossing T1@100.
  Measured VM==JIT==40800, zero `[jit-fallback]` both modes.
- `tests/smokes-jit-closure/c3-carrier-aggregate.shape` — heterogeneous
  `(int, number)` G9 mutation-return aggregate (user-declared type as the
  fixture-expressible proxy for the compiler-internal aggregate; same
  NewTypedObject-with-fully-typed-fields runtime shape). VM==JIT==40800
  zero-fallback. DISCLOSED ADAPTATION: the impl consumes the number via a
  `b > 3.0` comparison instead of the sketched `b as int` cast — a no-hook
  control proved the cast alone whole-program-deopts ("direct call to
  `calc_impl` … has no JIT FuncRef"), a PRE-EXISTING JIT gap unrelated to
  the carrier; the threshold also makes the before-hook's number mutation
  value-distinguishing (skip ⇒ 600).
- `tests/smokes-jit-closure/c3-carrier-aggregate-string.shape` —
  `(int, string)` sibling, zero-fallback directly (the
  named-expected-fallback contingency was not needed).
- `bin/shape-cli/tests/cli/jit_c3_carrier_native.rs` — one cell per
  fixture: exact stdout, VM==JIT equality, zero fallback lines in BOTH
  modes, each gated by the parse-based top-level-comptime vacuity guard
  (lifted to `pub(super)` in `jit_test_support.rs`, byte-identical;
  `jit_c2_install_native.rs` imports the lifted copy — the only
  pre-existing test file touched in the slice, import-only).

The stdout arithmetic is value-distinguishing per skipped hook (40800
green vs 40400 / 40200 / 600 per cell). **Named pin limit (verify-1 lens
residual, not a defect):** the pin guards both KNOWN silent-deopt classes
(top-level comptime via the vacuity guard; silent hook-skip via the
distinguishing values), but a hypothetical MARKERLESS silent whole-program
interpreter run would still pass — the inherent limit of any
stdout-based subprocess pin, identical in strength to the of-record
`jit_c2_install_native` pattern the S0 mandate prescribed. Doc-comment in
the test file names the hand-written-proxy status and the S7 obligation to
re-prove the COMPILER-GENERATED specialization path.

## S1b — the CheckedTemplate carrier (`7aa0acff`)

`crates/shape-vm/src/compiler/comptime_fragments/checked_template.rs`
(all `pub(in crate::compiler)`), discharging the `checked_body.rs:52-56`
deferral: the Sig/Captures semantic face lands as CARRIED SEMANTIC DATA on
a non-generic `CheckedTemplate` (`TemplateSig` + the shipped C1
`CaptureClause`); the Shape-face `CheckedTemplate<Sig, Captures>` type
parameters become S2's comptime-type projection of this data.

- `TemplateHookKind {Before, After}`; `TemplateSig {PolymorphicArgs,
  PolymorphicResult, Concrete(BodySignature)}`; `CheckedTemplate` (private
  fields + accessors, NO public constructor, NO serde — compiler-session-
  local, never serialized); `CheckedTemplateBuilder<SigState,
  CapturesState>` typestate (`finish()` ONLY on `<Present, Present>`, NO
  string/JSON constructor per G3). The wrong-variant offer
  (PolymorphicArgs to After etc.) is UNREPRESENTABLE: the variant derives
  from `hook_kind` and no constructor bypasses classification.
- Classification rule: no/empty type params → Concrete; exactly one plain
  type param + one plain by-value bare-`T` param + bare-`T` return →
  PolymorphicArgs (Before) / PolymorphicResult (After); everything else →
  named uncoded rejection with a positive twin.
- `finish()` reuses checked_body's `validate_capture_clause` VERBATIM
  (visibility-only widening at `checked_body.rs:270`; no third
  [C0902]/[C0907] sentence producer — the one-word C0907 divergence stays
  an S5 item) and runs the G9 pseudo-tuple walker for PolymorphicArgs
  bodies only.

`template_specialization/pseudo_tuple.rs` is the SINGLE traversal core
(one walker, two faces — binding invariant, do not fork): exhaustive
Statement/Expr matches mirroring `monomorphization/substitution.rs`, no
catch-all arms. Legal `args` uses EXACTLY: constant-Int `IndexAccess`
read, the same shape as assignment target, plain `args.length`,
`return args` (+ the parser's `Expr::Return` twin), and a FINAL top-level
bare-`args` tail. Everything else is a named uncoded rejection with a
positive twin (non-constant index, slicing, other/optional property,
bare-value position, closure-boundary occurrences, type param in
body-internal annotations, the reserved `__c3_` prefix, rebinding either
name, and — verify-1 fix — either name in CALL-NAME position). Named
boundaries: out-of-range constant indices are rewrite-face territory
(arity is target-side); f-string interpolation is a documented non-scanned
boundary (caught downstream by ordinary resolution after the name resolves
away, never silently honored).

## S1c — specialize_template: the per-specialization checking core (`59020f45`, `8adf8033`)

`template_specialization/mod.rs` — the INSTALL-side twin of the
construction chokepoint.

- **Target glue** (`specialization_target_from_def`): per-param types via
  the EXISTING `annotation_param_type_annotation` (declared annotation
  under the `annotation_type_is_unknown` guard, else the inference-facts
  fallback under the same guard) — the slice-0 §7.4 binding: AST-side
  types, NEVER the freeze round-trip; the frozen `CallableDescriptor` is
  carried for IDENTITY/equality ONLY. A type-less param is a named
  rejection at the `@application` site naming the USER-SPELLED fn +
  parameter (the S0 g3 mangled-name failure mode is inverted).
- **Transaction composition** (E1-D6b): the first statement of
  `specialize_template` REQUIRES the already-open C2 `InstallTransaction`;
  never a second transaction.
- **Concrete match-or-error** (C3-G4 degenerate case): Before = params
  positionally equal to the target's; return = the mutation carrier
  (`Single(T0)` iff arity 1; arity > 1 is a named rejection with the
  polymorphic positive twin — the G9 aggregate is compiler-internal BY
  RULING; arity 0 named-rejects). After = `(R) -> R`; void/absent return
  named-rejects (surface-and-stop; S2 revisits). Comparison: fast-path
  frozen-identity equality (derived `PartialEq` over `FrozenTypeIdentity`,
  arity-consistency-gated) when the descriptor is present AND the template
  side canonicalizes; otherwise per-position
  `declared_annotation_concrete_type` on BOTH sides → `ConcreteType`
  equality; a `None` resolution on either side is a named rejection naming
  side + position, never a guess.
- **Attribution** (`template_application_error`, C3-G10's genuinely-new
  piece): the SINGLE producer every user-facing rejection routes through —
  `SemanticError` anchored at the `@application` span, naming BOTH
  signatures (template's declared form + the required specialization
  signature; the multi-param Before aggregate renders as tuple NOTATION in
  message text only). Precedent `directive_signature_type_error`,
  re-anchored.
- **Polymorphic arms** (stage 4): `PolymorphicArgs` (Before) builds the
  `TemplateSpecializationPlan` (per-param carrier + `Single`/`Aggregate`)
  and rides `ensure_monomorphic_template_specialization`;
  `PolymorphicResult` (After) rides the plain UNCHANGED
  `ensure_monomorphic_function_for_callsite` at `[R]` — no plan, no
  suffix (sharing a cache entry/symbol with an ordinary generic
  instantiation at `R` is correct; pinned). EVERY `SpecializationFailure`
  (Soft AND Hard — the G10 hard-fail posture, no generic fallback for
  templates) wraps through the attribution producer preserving the inner
  detail text verbatim.

### The monomorphization ride (`cache.rs`)

`ensure_monomorphic_function_for_callsite`'s body moved VERBATIM into the
private `ensure_monomorphic_function_impl(..., template_plan:
Option<&TemplateSpecializationPlan>)`; the `pub(crate)` fn delegates with
`None` (existing callers byte-equivalent); the new
`pub(in crate::compiler)` template entry delegates with `Some(plan)`.
Exactly three plan-conditional seams: (a) the KEY/SYMBOL suffix (post
verify-1: `template_specialization_key_suffix`, see below) appended to the
mono key BEFORE `prepare_semantic_specialization` and to the specialized
symbol at the rename seam; (b) the CONST-PARAM GUARD (a plan reaching the
const-generic reroute is a named internal error — S3 ConstLift territory;
the classifier rejects const-generic template bodies at construction);
(c) the G9 RESOLUTION (`pseudo_tuple::resolve_pseudo_tuple` immediately
after substitution + rename; errors → Hard). Everything downstream
(register, cache-insert-before-compile, in-progress guard, save/restore,
overlay guard, `compile_function` = the G10 emission-tier + MIR battery,
`cache_remove`-on-Err, Hard classification) runs UNCHANGED.

The G9 resolution rewrites the substituted def to a plain concrete typed
AST function: params → `__c3_p{i}` typed from the target's AST side;
prologue `let mut __c3_arg_{i} = __c3_p{i}`; constant `args[i]`
read/assign-target → the minted local (out-of-range = named rejection
quoting index + arity + signature); `args.length` → the constant N;
mutation-return/tail → the `Single` local or the compiler-internal
`Aggregate` object literal `{a0..aN-1}`; return annotation → the `Single`
annotation or the fully-typed inline `TypeAnnotation::Object`. The
transient post-substitution `Tuple` annotation NEVER reaches checking or
emission. The aggregate is ADR-006-ordinary: the inline-schema
TypedObject the ordinary pipeline emits (`HeapValue::TypedObject
(Arc<TypedObjectStorage>)`, fully-typed fields, never `FieldType::Any`, no
`Box<HeapValue>`, no new HeapKind/discriminator, no `KindedSlot` in the
typed VM↔JIT slot ABI), reachable only via the suffixed unspellable
symbol.

### Rollback fold-in (disclosed, probe-tested)

The stage-mandated rollback probe CONFIRMED stale-index reuse against the
unmodified C2 rollback: `rollback_checked_body_install` truncated
`program.functions` to the watermark while the monomorphization cache
entry survived, so an identical re-specialization cache-hit a dangling
index. Fold-in per the disclosed protocol: rollback now evicts every
cache entry at/above the functions watermark
(`store.rs::evict_at_or_above_function_index`, BOTH cache domains, both
retain modes — `rollback_indexed_publications` truncates
`program.functions` in both modes, so at/above-watermark entries are
dangling in both; a cache entry is an executable resolution, never a
query reservation). Strictly-necessary, same Wave-class, disclosed in the
stage commit and the close-relay; pinned by
`rollback_evicts_the_specialization_cache_and_a_rerun_reregisters_fresh`
(eviction → `legacy_len` 0 → fresh re-registration at the freed index →
re-execution).

## Verify round 1 — findings and fixes (`25f196d1`)

Two review lenses ran over `c2297946..8adf8033`. Fixes are append-only
atop the gated hashes.

1. **BLOCKER — the Sig identity was non-injective (fixed).**
   `specialize_polymorphic_before` keyed the cache/symbol on
   `ConcreteType::Tuple(param_concretes)` rendered through
   `Tuple::mono_key()` — a FLAT non-delimited join. A target parameter may
   itself be a bracket-tuple type (homogeneous bracket types are
   supported), so parameter types redistribute across the arity boundary:
   `fn a(x: [int,int,int], y: number)` and `fn b(x: [int,int], y: int,
   z: number)` BOTH rendered `…::tuple_tuple_i64_i64_i64_f64::
   c3_before_hook`. Applying one Before template to A then B cache-hit A's
   handler for B — B's body never checked against B's Sig, a 2-param
   handler for a 3-param target, `args.length` frozen at the wrong
   constant (the C3-G10 violation). Latent (no production caller until
   S2) but the slice's core committed contract. FIX:
   `template_specialization_key_suffix` — the salt + per type argument the
   Sig arity (`::a{n}`) + the DELIMITED `Display` rendering (injective by
   parenthesization), applied identically to key and symbol, template
   seam ONLY. REFUTER
   (`colliding_flat_tuple_renderings_specialize_separately`) pins the
   exact pair with a non-vacuity control asserting the flat keys still
   collide; proven biting — with the suffix neutered to the bare salt the
   test fails exactly as derived (both targets shared one function
   index). **Named follow-up:** the root fix is a delimited
   `ConcreteType::Tuple::mono_key` globally; that shifts every ordinary
   tuple cache key in the program and needs its own regression pass — not
   taken as a verify-fix.
2. **SHOULD-FIX — call-name positions escaped the rejection contract
   (fixed).** `FunctionCall` names, `QualifiedFunctionCall`
   namespace/function, and `MethodCall` method names checked only the
   reserved `__c3_` prefix, so `args(1)` passed both faces — post-rewrite
   a confusing wrapped downstream error, or a SILENT meaning shift onto a
   module fn spelled `args`. FIX: `check_call_name` at all four name
   spellings; ClosureInterior rejects both template names with the
   closure-occurrence sentence, TemplateBody rejects `args_param` with
   the bare-value sentence and `type_param` with its call-name twin (new
   named rejection, positive twin carried). 5 new pseudo_tuple tests
   (4 validate-face + 1 rewrite-face shared-core fail-closed).
3. The zero-param-before rejection test now asserts the positive twin
   sentence its producer already carried.
4. **Count correction:** the stage-4 close-relay claimed "8 new
   rewrite-face unit tests"; the actual count is 7 (pseudo_tuple.rs
   15 → 22 at `8adf8033`; the commit message enumerates without counting
   and is not affected).

## Observations pending supervisor disposition (surfaced, not S1 defects)

- **The effective per-specialization checking surface is the emission
  tier's strict-proof classes** (the ruled slice-0 §7.3 posture; battery
  row 1 — the whole-program analyzer — does not re-run per specialization
  BY RULING). Measured near-misses recorded at the stage-4 test doc
  (`template_specialization/mod.rs`, `body_type_error_…` doc-comment):
  `args[0].trim()` on an int slot and `args[0] = "boom"` into an
  int-inferred local both COMPILE clean at the seam (unknown-method
  dispatch defers to runtime; a local re-assignment re-stamps the slot
  kind at emission) — an aggregate field's runtime kind can diverge from
  its declared carrier annotation. Charter-compliant, but SHOULD BE
  RE-DISPOSITIONED before S2 stacks the weave on the aggregate.
- **Same-Sig cache-hit carrier spellings:** on a cache hit the returned
  `MutationCarrier`/plan annotations come from the CURRENT target's AST
  spellings while the compiled def was resolved under the FIRST target's.
  Semantically equivalent under `ConcreteType` equality (the injective
  suffix guarantees the Sigs match); alias spellings of the same concrete
  type can differ between carrier and def. Recorded for the S2 weave
  design (which consumes the carrier).
- **S7 ledger note:** the concrete happy-path pins are identity-proven
  (function index + carrier), not execution-proven — sound because the
  concrete handler IS the definition-compiled body fn and the negative
  twins defeat an always-Ok stub; S7's zero-fallback cells must EXECUTE
  the handler paths (the standing S0 §2 obligation).
- **S1a pin limit** as stated in the S1a section above.

## Test inventory at `25f196d1`

In-module unit tests: `checked_template.rs` 15;
`template_specialization/pseudo_tuple.rs` 27 (15 validate-face stage-2 +
7 rewrite-face stage-4 + 5 verify-1 call-name);
`template_specialization/mod.rs` 24 (15 stage-3 concrete/attribution +
8 stage-4 polymorphic execution-proven + 1 verify-1 refuter). CLI: 3
S1a cells in `jit_c3_carrier_native.rs`. Polymorphic pins are
EXECUTION-proven per the S0 §2 named uncertainty (mutated slots 3→4 /
3→9, `args.length` constant 3+2=5, After at two R types with distinct
unsalted indices, rollback re-execution) — never compile-proof alone.

## Gates

At `8adf8033` (stage close): template_specialization/pseudo_tuple/
checked_template/monomorphization filters green; full shape-vm `--lib`
`-j1` FAILED set == the S0 7-name baseline EXACTLY; cli_tests
`--test-threads=1` 50/50 (47 pre-existing + 3 S1a); `cargo check -p
shape-vm --all-targets` clean; `just check-clean` and
`just check-no-dynamic` exit 0. At `25f196d1` (verify-1 fixes):
template_specialization 51/51 green; full shape-vm `--lib` `-j1` FAILED
set == the S0 7-name baseline EXACTLY (3234 passed); `cargo check -p
shape-vm --all-targets` clean; `just check-clean` exit 0;
`just check-no-dynamic` exit 0 (the pre-existing baseline-suggestion
informational line left untouched, out of S1 scope).

Standing S1 invariants, verified by both lenses: the legacy weave
(`compile_specialized_annotation_handler`,
`specialize_annotation_runtime_handlers`, `compile_annotation_wrapper`,
`annotation_arg_array_element_annotation`, the `FieldType::Any` ctx
schema) is byte-unchanged beside the new path — a C3-G7 deletion target,
not a foundation; the 48 green annotation pins (annotations_runtime 24 +
annotation_targets 24) are untouched; no grammar changes; no C09xx
minted (S5 owns minting from C0931+); no new HeapKind; no
`FieldType::Any`; forbidden-pattern and refused-regex greps over the full
diff are clean.
