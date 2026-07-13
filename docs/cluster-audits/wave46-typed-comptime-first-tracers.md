# Wave 46: Typed Comptime First Tracers

Date: 2026-07-11

## Scope

This wave moved two accepted ADR-009 invariants into executable compiler
behavior:

1. generated code cannot acquire an undeclared closure environment; and
2. comptime type categorization no longer requires a public string kind or a
   user-constructible type descriptor.

It is intentionally not a claim that the complete typed comptime algebra is
implemented.

## Landed

### Generated Capture Boundary

Annotation/comptime-generated functions and methods are marked before their
bodies compile. Closure lowering checks that marker after deterministic capture
discovery. A non-empty inferred capture set is a compile-time semantic error:

```text
generated closure implicitly captures 'value'; generated captures must be explicit
```

The previous public proof accepted the generated function and returned `Null`.
The new proof fails before runtime. A focused ordinary source-closure test still
passes, so this does not change current source closure inference.

### Typed Type Identity and Category

`type_ref(T)` accepts bare compiler-resolved type syntax. The comptime rewrite
maps that syntax to a canonical 128-bit SHA-256 descriptor fingerprint in the
immutable type-reflection snapshot. Its runtime carrier and native forwarding
entry have unspellable names and expose only the two fingerprint halves; source
code cannot forge it and strings are rejected. Adding unrelated declarations
cannot renumber existing identities.

`type_category(type_ref(T))` returns the schema-backed exhaustive enum:

```shape
pub enum FrozenTypeCategory {
    Primitive,
    Never,
    Parameter,
    Nominal,
    Tuple,
    Record,
    Callable,
    Reference,
    Union,
    Erased,
}
```

The enum has no `Unknown`, `Any`, or string arm. Unresolved names fail the
freeze boundary. Transparent aliases map to the underlying identity. Active
function generic parameters are added to the snapshot and receive scoped
`Parameter` identities. Primitive and user-nominal proofs are enabled, and
compiler/LSP metadata exposes both builtins with completion and hover
documentation.

The comptime-to-runtime materialization boundary rejects raw `TypeRef` and
`FrozenTypeCategory` values. They are compiler capabilities/reflection data,
not `ConstLift` values; callers must consume them inside comptime and lift an
ordinary closed result instead.

LSP completion now recognizes ordinary comptime blocks, comptime functions,
and annotation `comptime pre/post` hooks. Enum namespace completion is a
general `Enum::prefix` path for user enums, with `FrozenTypeCategory` variants
sourced from the same runtime-owned catalog used by the compiler and schema.

The reflection implementation moved into
`compiler/comptime_builtins/type_reflection.rs` (under 500 lines), reducing
growth pressure on the pre-existing oversized `comptime_builtins.rs`.

## Verification

- Generated implicit capture red proof: previously returned `Null`; now passes
  as an expected compile error.
- Public typed-reflection behavior and rejection matrix: 16 passed, covering
  VM/JIT exhaustive consumption, primitive/never/erased/nominal reachability,
  enums/builtin nominals, alias chains, arity/type/stage errors, legacy-carrier
  rejection, non-liftable capabilities, and exhaustiveness.
- Reflection identity unit matrix: 9 passed, covering active generic-function
  discovery, function-scoped parameters, synonym/alias normalization,
  declaration-order stability, distinct nominals, and unknown identities.
- Runtime catalog and opaque schema matrix: 3 passed.
- Generated capture matrix: 8 passed, covering generated free functions and
  methods, local/parameter/multiple/`self` captures, deterministic ordering,
  capture-free closures, closure parameters, VM/JIT consumers, and ordinary
  source closure control behavior.
- `annotations_comptime`: 60 passed, 0 failed with two test threads.
- `comptime`: 109 passed, 0 failed with two test threads.
- Typed comptime LSP completion/hover/signature/diagnostic/enum matrix: 14
  passed, 0 failed; the neighboring completion-context unit module passed
  58/58.
- The combined broad ShapeTest gate passed 568/568: 60 annotation-comptime,
  109 comptime, and 399 LSP tests. Peak memory was 3.1G with swap disabled.
- All gates ran under the single cgroup lane with swap disabled. The first
  unconstrained-harness run hit `TasksMax=256`; the deterministic rerun used
  `--test-threads=2` and passed.

## Still Missing

1. Payload-bearing and type-indexed `FrozenType<T>` descriptors for all ten
   categories.
2. The sealed `FrozenPrimitive<T>` sub-algebra and exact width/domain payloads.
3. Native type-expression syntax that can form `TypeRef` for tuples, records,
   callables, references, unions, erased domains, and applied generic types.
4. Complete semantic normalization for applied nominals, object intersections,
   and trait intersections on every call path. The declared-generic-parameter
   half of this gap is CLOSED (2026-07-12, ADR009-A3): the generic-call
   specialization gap is fixed — a scoped compiler overlay carries the base
   definition's declared type parameters into the reflection snapshot when the
   monomorphized body compiles, and hard specialized-body compile errors now
   propagate instead of being masked as "cannot infer type argument(s)".
   Public e2es (VM+JIT) in `tools/shape-test/tests/comptime/frozen_type.rs`:
   `generic_body_observes_parameter_category_for_its_own_type_param`
   (positive `Parameter` proof),
   `undeclared_name_in_generic_body_still_fails_the_freeze` (negative:
   `type_ref(U)` in `fn f<T>` → "unknown semantic type identity"),
   `parameter_category_is_stable_across_instantiations_of_one_generic_fn`,
   `distinct_generic_fns_each_observe_parameter_for_their_own_type_param`,
   plus the full rejection matrix re-fired inside generic bodies
   (`*_inside_generic_bodies` / `*_generic_bodies_*` tests). Specialization
   unit pins in
   `crates/shape-vm/src/compiler/comptime_builtins/type_reflection/tests.rs`
   and `crates/shape-vm/src/compiler/monomorphization/cache.rs`; LSP
   generic-body matrix in `tools/shape-test/tests/lsp/typed_comptime.rs`.
5. Public `CaptureDescriptor<Sig, I, T, Mode>`, heterogeneous capture packs,
   `CheckedBody<Sig, Captures>`, and `CheckedTemplate<Sig, Captures>`.
6. Explicit generated closure capture syntax/builders and full ownership,
   lifetime, suspension, `Send`, cleanup, `Drop`, and `AsyncDrop` proofs.
7. Migration of legacy `type_info`, string/JSON/source generation directives,
   string-backed older descriptor rows, and runtime-hook `Any` carriers.
8. Generated-symbol/source-map integration beyond metadata-backed builtin
   completion and hover.

These gaps remain compile-time capability gaps. This wave does not add runtime
fallbacks or claim partially populated descriptors.

## ADR009-A1 Addendum (2026-07-12, ticket #2, branch adr009/a1)

Ticket A1 (spec §4.1, slices S1-S5) supersedes parts of this audit's
"Typed Type Identity and Category" and "Verification" sections:

- **Per-site snapshot is gone.** "the immutable type-reflection snapshot"
  above now reads as ONE `SemanticFreeze` per compilation unit, built at the
  registration-complete barrier in `compile()`
  (`comptime_builtins/semantic_freeze.rs`); `build_type_reflection_snapshot`
  and the by-value `TypeReflectionSnapshot` carrier are deleted. Active
  generic-function parameters are no longer "added to the snapshot": they
  enter through scoped `FreezeOverlay` layers over the shared `Arc` base.
- **Annotation handlers consume the same freeze.** The empty-snapshot gap
  (`comptime.rs:1340-1344` at this audit's baseline) is deleted; the barrier
  runs AHEAD of the speculative annotation pre-passes, and a comptime site
  without a freeze handle is the named compile error
  `NO_FREEZE_HANDLE_DIAGNOSTIC`.
- **Registration-complete semantics.** Aliases and enums declared after a
  comptime block are visible to it (new VM+JIT proof
  `later_declared_alias_and_enum_are_visible_to_earlier_comptime_blocks`).
- **LSP rows are catalog-generated.** The hand-written `type_ref` /
  `type_category` rows in `builtin_metadata.rs` are replaced by descriptors
  owned by the shared runtime catalog (`comptime_reflection.rs`,
  `frozen_type_category_catalog!`).
- **Legacy vocabulary confined.** `TypeKindLabel` /
  `classify_legacy_type_info` / `build_type_info_heap_value` /
  `__ComptimeTypeInfo` survive ONLY on the legacy `type_info` intrinsic path
  (consuming the same freeze handle), marked `E5-deletes` and pinned by the
  sentinel test
  `type_reflection/tests.rs::legacy_type_info_vocabulary_is_confined_to_the_legacy_intrinsic_path`.
  Ticket E5 deletes them; nothing new may import them.

### A1 rejection matrix (rows 1-8, all named-diagnostic-asserted, green 2026-07-12)

| Row | Forbidden form | Asserting tests |
|---|---|---|
| 1 | String constructs a `TypeRef` | `comptime/frozen_type.rs::strings_cannot_construct_type_refs`; `lsp/typed_comptime.rs::string_type_ref_construction_has_semantic_diagnostic` |
| 2 | Unresolved/unknown type crosses the freeze boundary | `frozen_type.rs::unresolved_type_cannot_cross_freeze_boundary`; `lsp/typed_comptime.rs::unresolved_type_ref_has_semantic_diagnostic`; unit `type_reflection/tests.rs::unknown_identity_is_rejected_at_the_freeze_boundary` |
| 3 | Comptime site without a freeze handle (A1-new) | `semantic_freeze::tests::comptime_site_without_freeze_handle_is_a_named_compile_error`; `functions_annotations::s3_freeze_gate_tests::{extends,signature_directive}_prepass_without_freeze_handle_is_the_named_row3_compile_error` |
| 4 | Partial semantic state frozen / handler runs before freeze check (Dec 52, A1-new) | `semantic_freeze::tests::{unresolved_inference_variable_cannot_be_frozen, nested_unresolved_inference_variable_cannot_be_frozen}`; `functions_annotations::s3_freeze_gate_tests::freeze_rejection_fires_before_annotation_handler_body_executes` |
| 5 | Raw `TypeRef`/`FrozenTypeCategory` escapes to runtime | `frozen_type.rs::{type_ref_is_comptime_only, raw_type_refs_cannot_escape_to_runtime_code, raw_frozen_categories_cannot_escape_to_runtime_code}`; `lsp/typed_comptime.rs::raw_type_ref_escape_has_semantic_diagnostic` |
| 6 | Legacy descriptors / arbitrary values forge a `TypeRef` | `frozen_type.rs::{legacy_reflection_descriptors_cannot_forge_type_refs, arbitrary_values_cannot_be_used_as_type_refs}`; `lsp/typed_comptime.rs::legacy_type_descriptor_has_semantic_diagnostic` |
| 7 | Wrong arity / non-type argument forms | `frozen_type.rs::{type_ref_requires_exactly_one_type_argument, non_type_expressions_cannot_construct_type_refs}` |
| 8 | Non-exhaustive category consumption / `Unknown` arm | `frozen_type.rs::category_matches_are_checked_for_exhaustiveness`; `lsp/typed_comptime.rs::frozen_category_completion_is_closed_and_has_no_unknown_arm` |

Row 9 (no `Option<freeze>` / `default()` at any comptime entry) is enforced
structurally — `SemanticFreeze`/`FrozenTypeIndex` have no `Default` and no
empty constructor — and by diff-review grep at each slice close.

### A1 final verification counts (this addendum's date)

Suite counts changed since the wave46 baseline (all strictly additive vs
`main@4d70508b`; no deletions, no rebaselines):

- shape-vm `compiler::comptime_builtins`: 19 passed (semantic_freeze +
  9-test identity matrix + confinement sentinel); `compiler::comptime`
  filter: 82 passed / 4 pre-existing ignores; `compiler::functions_annotations`
  s3 freeze gate: 3 passed.
- shape-runtime: `comptime_reflection` 6, `type_schema::builtin_schemas` 6,
  `builtin_metadata` 4 — all passed.
- ShapeTest `comptime`: 109 passed (baseline 108 on main + the
  later-declared-visibility VM+JIT proof).
- ShapeTest `annotations_comptime`: 48 passed with two threads (44 on main +
  the 4-test `frozen_reflection.rs` VM+JIT matrix). NOTE: this audit's
  original "60 passed" verification line does not match `main@4d70508b`
  (static count 44); the addendum records the measured current truth.
- ShapeTest `lsp`: full 399 passed (not just the 14-test typed_comptime
  module); shape-lsp `completion::tests` 58 + `context::tests` 58 passed.
- `just check-clean` + `scripts/check-no-dynamic.sh`: green at slice close
  (see S5 close-out in `docs/defections.md`).

## ADR009-B2 Addendum (2026-07-13, ticket #6, branch adr009/b2)

Ticket B2 (spec Stage 2, Dec 49, slices S1-S6) adds typed trait identity and
implementation evidence on top of the A1 freeze surface. Nothing here
re-derives trait/impl truth from a parallel table; the legacy
`implements(T, Trait)` string path is byte-untouched (E5 deletes it).

- **Barrier-ordering: two-sub-pass predeclare walk (S1).** Trait defs and
  trait impls historically registered AFTER the freeze barrier
  (`register_item_functions` / pass-2 `Item::Impl`), which is why the legacy
  `implements` re-reads `env.trait_impl_keys()` live at every comptime site —
  the per-site pattern A1 deleted for types.
  `predeclare_item_semantic_freeze_inputs` (statements.rs) gained
  `Item::Trait` / `Item::Impl` / `ExportItem::Trait` arms run as TWO
  sub-passes (`SemanticFreezePredeclarePass::TypesAndTraits`, then `Impls`)
  because source/topological order does not guarantee trait-before-impl and
  `register_trait_impl` validation needs the trait present. Both entry
  points are wired: `compile()`'s predeclare loop and the graph entry's
  qualified dep-module walk (deps then root, twice) — the A1-review-round-1
  imported-module regression class is covered by barrier-truth tests for a
  root AND an imported dep module. The impl arm mirrors the analyzer's
  `register_impl` exactly, so the analyzer's later re-registration is an
  idempotent Ok (registry.rs:315-330).
- **Freeze inputs 4 and 5 (S2).** `SemanticFreeze::freeze` reads the
  barrier-complete env registry ONCE into: (4) trait identities — a separate
  `frozen_trait_ids` map, NEVER interned into `FrozenTypeIndex.
  frozen_type_ids` (so `type_ref(TraitName)` cannot resolve as a type and
  the cross-category collision assertion stays sound) and NO
  `FrozenTypeCategory::Trait` variant (Dec 50 rule 5, structural pin test);
  (5) impl evidence — `FrozenImplEvidence` facts keyed
  `(trait_identity, type_identity)`, default + named impls distinct.
- **Identity-descriptor scheme.** Canonical descriptors `trait:{name}` and
  `impl:{trait}:{type}:{impl_name_or___default__}` go through
  `FrozenTypeIdentity::from_canonical_descriptor` (128-bit SHA-256, never
  counter-allocated). The unit e2e
  `trait_evidence::tests::find_impl_evidence_artifact_carries_canonical_freeze_identities`
  proves the evidence artifact returned by `find_impl -> Some(proof)`
  carries exactly the canonical trait/type/impl identity hashes — Dec 49's
  "canonical trait and implementation identities enter generated-artifact
  fingerprints".
- **Public surface (S4).** `trait_ref(Trait)` (positional bare-ident form —
  Dec 49's turbofish spelling lands with ticket A2; deviation logged in
  `docs/defections.md`) lowers via the comptime rewrite to
  frozen-trait-identity int literals (identity-literal transport, no
  strings). `find_impl` queries ONLY `FreezeOverlay::impl_evidence_of` —
  proven by passing EMPTY legacy `trait_impl_keys` in the unit e2e — and
  returns `TypedReturn::Some(ImplRef)/None` (R9: unimplemented pair =
  `None`, never an error, never partial). Reserved unspellable carriers
  `"\u{1}comptime:TraitRef"` / `"\u{1}comptime:ImplRef"`; the ImplRef schema
  carries all three identity pairs (trait + type + impl). Positive Dec-49
  proof green under VM AND JIT
  (`find_impl_some_arm_consumes_evidence_under_vm_and_jit`).
- **Ruled stances (named surface-and-stops, never silent `None`):**
  blanket-impl satisfaction, legacy numeric widening, ambiguous
  unqualified-impl attribution, named-impls-only pairs, and post-barrier
  (comptime-generated/derived) implementations. All firewall-safe (pinned by
  `semantic_freeze::tests::b2_user_facing_diagnostics_are_firewall_safe`).
- **LSP rows are catalog-generated.** `TRAIT_REF_BUILTIN_ROW` /
  `FIND_IMPL_BUILTIN_ROW` live in the shared runtime catalog
  (`comptime_reflection.rs`) and are spliced verbatim into `CORE_BUILTINS`;
  completion/hover/signature help/diagnostics are driven by those rows
  (parity pins `hover_tests.rs::test_comptime_builtin_hover_{trait_ref,find_impl}_uses_shared_catalog`).

### B2 rejection matrix (rows R1-R9 + post-barrier, all named-diagnostic-asserted, green 2026-07-13)

Public e2e rows live in `tools/shape-test/tests/comptime/trait_evidence.rs`,
LSP twins in `tools/shape-test/tests/lsp/typed_comptime.rs`, unit pins in
`crates/shape-vm/src/compiler/comptime_builtins/trait_evidence.rs` and
`semantic_freeze.rs`:

| Row | Forbidden form | Asserting tests |
|---|---|---|
| R1 | Trait-as-type: `type_ref(TraitName)` | `trait_evidence.rs::r1_trait_as_type_is_the_named_traits_are_not_value_types_rejection`; A1-row-2 guard `r1_upgrade_keeps_the_generic_diagnostic_for_unknown_names`; unit `trait_evidence::tests::type_ref_carrier_rejects_a_frozen_trait_identity_with_the_named_r1_text`; LSP twins `typed_comptime.rs::{trait_as_type_ref_has_semantic_diagnostic, unknown_name_keeps_generic_diagnostic_with_traits_declared}` |
| R2 | Type-as-trait: `trait_ref(User)` / `trait_ref(int)` | `r2_type_as_trait_only_a_declared_trait_forms_a_trait_ref`; LSP twin `type_as_trait_ref_has_semantic_diagnostic` |
| R3 | Name-string lookup | `r3_strings_cannot_construct_trait_refs`; `r3_find_impl_rejects_name_string_lookup`; LSP twins `{string_trait_ref_construction, find_impl_string_lookup}_has_semantic_diagnostic` |
| R4 | Boolean-authorized generation (incl. `implements(...)` result) | `r4_boolean_cannot_authorize_implementation_evidence`; LSP twin `boolean_authorized_generation_has_semantic_diagnostic` |
| R5 | Evidence escaping its branch (minimal sound: stage boundary + Some-arm-only issuance) | `r5_impl_ref_evidence_cannot_escape_the_comptime_stage_boundary`; LSP twin `raw_impl_ref_escape_has_semantic_diagnostic`; Some-arm-only issuance is structural (only `find_impl` builds `ImplRef`; R7 blocks forgery) |
| R6 | TraitRef/ImplRef escaping to runtime | `r6_trait_ref_cannot_escape_to_runtime_code`; unit `comptime_reflection::tests::runtime_lift_rejection_names_trait_evidence_as_comptime_only`; LSP twin `raw_trait_ref_escape_has_semantic_diagnostic` |
| R7 | Forged evidence (arbitrary values / legacy descriptors / hand-built objects) | `r7_arbitrary_values_and_legacy_descriptors_cannot_forge_evidence`; unit `trait_evidence::tests::{trait_ref_carrier_rejects_identities_the_freeze_never_issued, impl_ref_decode_rejects_forged_or_lookalike_evidence, trait_ref_decode_is_schema_name_checked}` (structural: schema-name-checked opaque decode + freeze re-validation) |
| R8 | Wrong arity / wrong argument forms | `r8_trait_ref_arity_and_wrong_forms_are_named_rejections`; `r8_find_impl_arity_is_a_named_rejection`; LSP twins `{trait_ref_arity, find_impl_arity}_has_semantic_diagnostic` |
| R9 | Missing-impl fabrication | `find_impl_unimplemented_pair_is_none_never_an_error` (`None` — never an error, never a default/partial `ImplRef`); unit `find_impl_unimplemented_pair_executes_the_none_arm` |
| + | Post-barrier (comptime-generated/derived) impl queried as evidence | `post_barrier_comptime_generated_impls_are_a_named_stop_not_none`; unit `trait_evidence::tests::post_barrier_registered_pair_is_the_named_ordering_stop_never_a_silent_none` — a Dec 52 ordering stop, never masqueraded as `find_impl -> None` |

R10 (structural, diff-review): no extension of `implements()`/`TypeKindLabel`
(E5-confinement sentinel green; `comptime_builtins.rs` diff across B2 =
2 intrinsic-name constants + 1 registration call + the
`create_comptime_builtins_module` signature), no parallel trait table, no
`FrozenTypeCategory::Trait`, identities only via
`from_canonical_descriptor`, `check-no-dynamic` green at every slice close.

### B2 final verification counts (this addendum's date)

All strictly additive vs base `adr009/base@fbff1b5d` (A1+A3); no deletions,
no rebaselines:

- shape-vm `compiler::comptime_builtins`: 50 passed (A1's 19 + S1 barrier
  tests, S2 identity/evidence matrix, S3/S4 carrier + intrinsic unit e2es,
  S5 post-barrier pin, S6 firewall-safety pin).
- shape-runtime: `comptime_reflection` 11, `builtin_metadata` 5,
  `type_schema::builtin_schemas` 8 — all passed (TraitRef/ImplRef schema,
  catalog-row, and generated-row drift tests included).
- ShapeTest `comptime`: 138 passed with two threads (109 at A1 close, plus
  the A3 generic-body matrix merged in base and the 16-test
  `trait_evidence.rs` matrix: pre-flight, positive VM+JIT proof, None arm,
  pair-discrimination, R1-R8 rows, post-barrier stop).
- ShapeTest `lsp` `typed_comptime` module: 40 passed (A1/A3's 23 + the 17
  B2 completion/hover/signature/diagnostic-twin tests).
- shape-lsp `--lib hover`: 98 passed (incl. the two B2 shared-catalog
  parity pins).
- Full gate list (17 commands) green at S6 close — see the S6 close-out
  entry in `docs/defections.md`.
