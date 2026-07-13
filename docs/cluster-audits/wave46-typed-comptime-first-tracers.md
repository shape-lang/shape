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
   categories. PARTIALLY CLOSED (2026-07-13, ADR009-B1): `reflect(TypeRef<T>)
   -> FrozenType<T>` is live with the first three complete payload
   categories — Primitive, Never, Erased — at catalog-pinned ordinals 0/1/9;
   the seven remaining categories are named compile-time reflect-rejections
   ("payload descriptor has not landed"), never partial descriptors, while
   `type_category` stays exhaustive at 10. Remaining payloads are B2/B4-B7
   territory. See the B1 addendum below.
2. The sealed `FrozenPrimitive<T>` sub-algebra and exact width/domain
   payloads. CLOSED (2026-07-13, ADR009-B1): the sealed 10-member
   `FrozenPrimitive` sub-algebra (Unit, Bool, Char, SignedInteger,
   UnsignedInteger, BinaryFloat, Decimal, String, Null, Undefined) with
   `IntegerWidth` (W8/W16/W32/W64/Arbitrary) and `FloatWidth` (W32/W64)
   domain payloads is catalog-generated, VM+JIT-proven across every synonym
   family, and lift-walled. `bigint` = `SignedInteger(Arbitrary)` by named
   decision. See the B1 addendum below.
3. Native type-expression syntax that can form `TypeRef` for tuples, records,
   callables, references, unions, erased domains, and applied generic types.
   CLOSED (2026-07-13, ADR009-A2, ticket #4) — see the A2 addendum below;
   const-generic applications remain a named parse-time rejection until
   B4/Dec-54 lands the const carrier.
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

## ADR009-B1 Addendum (2026-07-13, ticket #5, branch adr009/b1)

Ticket B1 (spec §5 Stage 2, Dec 50/94) lands `reflect(TypeRef<T>) ->
FrozenType<T>` — the sealed payload-bearing indexed sum — with the FIRST
complete payload categories: Primitive (the sealed 10-member
`FrozenPrimitive` sub-algebra with exact `IntegerWidth`/`FloatWidth`
width/domain payloads), Never, and Erased (bound sets, provably empty for
`any` until A2). It closes Still-Missing item 2 and partially closes item 1
above (3 of 10 payload categories; the rest are B2/B4-B7).

- **The ONE query API grew.** `SemanticFreeze::payload_of` +
  `FreezeOverlay::payload_of` sit beside `identity_of`/`category_of`
  (`semantic_freeze.rs`); no parallel projection. Payloads derive from the
  same single `PRIMITIVE_SYNONYM_FAMILIES` table that produces identities
  (`type_reflection.rs`) — one construction point per spec §4.1. Descriptor
  builders live in the new `type_reflection/payloads.rs` (file-size policy;
  `type_reflection.rs` stays under 500 lines).
- **Catalog-owned data model.** The sealed `FrozenPrimitive` sub-algebra,
  the enabled-payload list, the `REFLECT_BUILTIN_ROW`, the width-domain
  enums (`IntegerWidth` W8-W64 + `Arbitrary`, `FloatWidth` W32/W64), and
  the LSP variant-completion lookup
  (`reflection_enum_variant_names`) are ALL generated from the shared
  runtime catalog (`shape-runtime/src/comptime_reflection.rs`) — no second
  hand-written list anywhere (the exact defect A1 deleted). `bigint` is
  `SignedInteger(IntegerWidth::Arbitrary)` by named decision.
- **Ordinal-pinned comptime ABI.** The three constructable `FrozenType`
  payload variants carry the Dec 50/94 catalog ORDINALS (Primitive=0,
  Never=1, Erased=9), not dense ids, so later B tickets add variants
  without renumbering (spec §3.3). The mini-VM's injected spellable model
  enum is pinned to the same ordinals at `register_enum` time
  (`frozen_type_payload_variant_ordinal`), so the spellable model and the
  unspellable value carrier can never disagree
  (`comptime/reflect.rs::never_and_erased_payload_arms_execute_on_vm_and_jit`
  proves the Erased=9 arm end-to-end).
- **Lift wall extended, same-commit discipline.** Every new descriptor
  schema ("\u{1}comptime:FrozenType" / …FrozenPrimitive / …FrozenNever /
  …FrozenErased, plus the IntegerWidth/FloatWidth carriers and the
  spellable model names) gained its own named `runtime_lift_rejection` arm
  in the SAME commit as its registration — no schema ever existed without
  a lift wall. The comptime-result wall became a VALUE-DEEP walk
  (`comptime_result_lift_rejection`: typed-object fields + typed-array
  elements, executed under the mini-VM program's schema registry) after a
  red run proved nested descriptors were silently swallowed to `Null`; the
  fix extends the CHANNEL to call the shared wall, never the reverse.
- **Runtime-name-collision fence.** The pre-existing runtime `reflect`
  surface stub (`helpers.rs` name mapping → `BuiltinFunction::Reflect`,
  executor `NotImplemented` arm) is UNTOUCHED and unit-pinned in both
  directions; comptime `reflect` resolves to the freeze-consuming forwarder
  by resolution order, and runtime-position `reflect` is the named
  comptime-only rejection.
- **Tracer discipline.** Reflecting any of the 7 non-enabled categories is
  a NAMED per-category compile-time rejection ("reflect: the `<Category>`
  payload descriptor has not landed (pending payload ticket); use
  type_category for the exhaustive category") — never a partial
  descriptor. The 10-variant `FrozenTypeCategory` catalog is untouched;
  its completeness test (`comptime_reflection.rs`) stays the canary, and
  `type_category` is proven exhaustive at 10 alongside `reflect` in one
  program.

### B1 rejection matrix (rows R1-R8, all named-diagnostic-asserted, green 2026-07-13)

| Row | Forbidden form | Asserting tests |
|---|---|---|
| R1 | `reflect()` on a non-enabled category (7 categories) | e2e (both reachable forms): `comptime/reflect.rs::{reflect_on_generic_parameter_is_the_named_r1_rejection, reflect_on_nominal_types_is_the_named_r1_rejection, r1_rejection_points_at_the_exhaustive_category_layer}`; hooks: `annotations_comptime/frozen_reflection.rs::annotation_handler_reflect_r1_rejection_fires_in_hooks`; all 7 per-category diagnostics at unit level: `type_reflection/tests.rs::non_enabled_categories_reject_with_named_per_category_diagnostics`, `builder_rejects_non_enabled_categories_with_the_named_diagnostic`; compiler pin `comptime.rs::reflect_non_enabled_category_is_the_named_r1_rejection`; LSP `lsp/typed_comptime.rs::reflect_non_enabled_category_has_semantic_diagnostic`. (Tuple/Record/Callable/Reference/Union have no `type_ref` spelling until A2 — unit-pinned only, per invariant §3.7; documented in the reflect.rs suite header.) |
| R2 | String kind access (`info.kind == "record"`, `.fields ?? []`) | `comptime/reflect.rs::{reflect_result_has_no_string_kind_field, reflect_result_has_no_nullable_fields_field}`; unit `comptime.rs::reflect_result_has_no_string_kind_field`; LSP `reflect_string_kind_access_has_semantic_diagnostic`. No descriptor schema has a `"kind"` string field or a nullable category field. |
| R3 | Descriptor lifts to runtime (any channel) | `comptime/reflect.rs::{frozen_type_descriptor_cannot_escape_to_runtime_code, frozen_primitive_descriptor_cannot_escape_to_runtime_code, frozen_never_descriptor_cannot_escape_to_runtime_code, frozen_erased_descriptor_cannot_escape_to_runtime_code, width_domain_payloads_cannot_escape_to_runtime_code, descriptor_nested_in_an_object_cannot_escape_to_runtime_code, descriptor_nested_in_an_array_cannot_escape_to_runtime_code, descriptor_cannot_lift_through_the_set_param_value_directive}`; deep-walk unit pins `comptime.rs::{deep_lift_wall_names_a_descriptor_nested_in_an_object_result, deep_lift_wall_ignores_ordinary_comptime_results}`; per-schema arms `comptime_reflection.rs::{each_new_descriptor_schema_has_its_own_named_lift_rejection, width_domain_schemas_have_their_own_named_lift_rejection, spellable_payload_model_names_have_their_own_named_lift_rejection, lift_rejection_still_fires_for_type_ref_and_frozen_type_category, lift_rejection_ignores_ordinary_values}` |
| R4 | Wrong arity / non-`TypeRef` argument (string, int, legacy `__ComptimeTypeRef` descriptor) | `comptime/reflect.rs::{reflect_requires_exactly_one_type_ref_argument, reflect_rejects_non_type_ref_arguments}`; unit `comptime.rs::reflect_arg_forms_are_rejected_with_named_diagnostics`; LSP `reflect_string_argument_has_semantic_diagnostic` |
| R5 | `reflect()` at runtime position (incl. generic bodies) | `comptime/reflect.rs::{reflect_is_comptime_only, reflect_is_comptime_only_inside_generic_bodies}`; unit `comptime.rs::reflect_is_comptime_only_at_runtime_position`; collision fence (both directions) `comptime.rs::runtime_reflect_name_mapping_and_stub_arm_are_untouched`; LSP `runtime_position_reflect_has_semantic_diagnostic` + visibility rows `{runtime_completion_hides_reflect, generic_body_runtime_position_after_comptime_block_hides_reflect}` |
| R6 | Non-exhaustive match / `Unknown`-`Any` arm on the sealed sum | `comptime/reflect.rs::{reflect_matches_are_checked_for_exhaustiveness, no_escape_arm_is_nameable_on_the_sealed_sum}`; unit `comptime.rs::reflect_match_exhaustiveness_is_enforced_over_the_injected_model`; LSP closed completion `{frozen_type_completion_is_closed_to_enabled_payload_variants, frozen_primitive_completion_is_closed_and_has_no_unknown_arm}` |
| R7 | User code forges a descriptor | Unspellable schema names + no public constructor (structural); forged SPELLABLE model values are lift-walled: `comptime/reflect.rs::comptime_constructed_model_values_are_still_lift_walled`; unit `comptime.rs::deep_lift_wall_names_a_forged_spellable_model_value` |
| R8 | Partially populated descriptor / `Default`-empty constructor | Structural: `FrozenPayloadDescriptor` has no `Default`; `FrozenErasedBound` is a deliberately UNINHABITED enum (a non-empty bound set is unrepresentable until A2/B2); grep at slice close over `payloads.rs`, `type_reflection.rs`, `semantic_freeze.rs`, `comptime_reflection.rs` finds no `derive(Default)`/`impl Default` on any descriptor type (wave46 row-9 discipline) |

### B1 verification recipe and counts (2026-07-13, all green)

Focused invocations (single cgroup lane, `direnv exec`, per AGENTS.md):

- `cargo check -p shape-vm -p shape-runtime -p shape-lsp -p shape-test
  --all-targets` — green.
- `bash scripts/check-no-dynamic.sh` — exit 0, every per-symbol count
  exactly at baseline (monotonic gate; no forbidden-family identifiers).
- shape-vm `--lib`: `compiler::comptime_builtins` 33 passed (incl. the
  9-test identity matrix, the S2 payload matrices, the E5 confinement
  sentinel, semantic_freeze); `compiler::comptime` 109 passed / 4
  pre-existing ignores (incl. the S3/S4 reflect pins);
  `compiler::monomorphization` 175; `feature_tests::module_tests` 10.
- shape-runtime `--lib`: `comptime_reflection` 30;
  `type_schema::builtin_schemas` 12; `builtin_metadata` 7 (incl.
  `is_comptime_builtin_function("reflect")` metadata pin).
- shape-lsp `--lib`: 779 passed (completion catalog-lookup drift pins +
  reflect hover pin included).
- ShapeTest `comptime`: 152 passed with two threads (122 at the B1 branch
  base incl. the A3-grown 30-test `frozen_type.rs`, + the 30-test
  `reflect.rs` suite; `frozen_type.rs` and the legacy `type_info_chained`
  suite untouched and green).
- ShapeTest `annotations_comptime`: 50 passed at `--test-threads=1` (the
  justfile-documented stable mode; 48 at A1/A3 close + the 2 S4
  payload/R1 hook rows in `frozen_reflection.rs`).
- ShapeTest `lsp` `typed_comptime`: 38 passed (24 at S3 close + 14 S5
  rows); `structs_types` `generics_comptime`: 20 passed (A3 regression
  surface, untouched).
- `just check-clean`: green at slice close (canonical workspace gate).

Every positive payload behavior is proven on BOTH engines via
`expect_vm_and_jit_output`; no test exercises a non-enabled category's
payload structure (invariant §3.7). Rejected-compromise log: the
2026-07-13 ADR009-B1 close-out entry in `docs/defections.md`.

## ADR009-A2 Addendum (2026-07-13, ticket #4, branch adr009/a2)

Ticket A2 (spec Stage 1, gap #3 above) closes the native type-expression
syntax gap: `type_ref(...)` accepts the full checked type grammar, forming
canonical `TypeRef` identities through ONE canonicalizer.

### Accepted spellings (exact, per the S3 grammar)

| Form | Spelling | Category |
|---|---|---|
| Bare names | `type_ref(int)`, `type_ref(Point)`, `type_ref(T)`, `type_ref(any)` | Primitive / Nominal / Parameter / Erased |
| Tuples | `type_ref([int, string])` | Tuple |
| Records | `type_ref({x: int})`, `type_ref({x?: int})` (optionality significant) | Record |
| Callables | `type_ref((int) -> bool)` | Callable |
| References | `type_ref(&T)`, `type_ref(&mut T)` (mutability significant) | Reference |
| Unions | `type_ref(int \| string)` (deduped, byte-sorted, singleton collapses) | Union |
| Erased | `type_ref(any)`, `type_ref(dyn Speak)`, `type_ref(dyn A + B)` (bounds sorted) | Erased |
| Applied generics | `type_ref(Option<int>)`, `type_ref(Array<User>)`, user generics, nested `Option<Array<int>>` | Nominal (with typed args) |

Deliberate surface choices: the spelling stays `type_ref(T)` — NOT the
Dec-48 turbofish `type_ref<T>()`; constructor-identity reclassification is
ticket B4. Bare generic heads (`type_ref(Option)`) stay `Nominal`, pinned by
`frozen_type.rs::enum_and_builtin_container_types_are_nominal`, in recorded
tension with the 2026-05-31 bare-unparameterized-generic ruling — B4's call.
Record-field identity is fixed as byte-sorted-by-field-name (Dec 50/94 are
silent on ordering; sorting delivers R11 declaration-order independence).
Const-generic applications (`type_ref(Array<3>)`) are a named parse-time
rejection ("const-generic type applications are not yet supported in
type_ref") — no `TypeAnnotation` const carrier, no descriptor bytes minted
(see the S3 defections entry). Applied ENUM heads are arity-UNCHECKED
(enum type-params are not recoverable from the schema registry at freeze
time — documented in the freeze module; wiring a compiler-side store is a
deferred decision, not silently taken).

### Architecture (extends A1, no parallel surfaces)

- ONE canonicalizer `canonicalize_type_annotation` in
  `shape-vm/src/compiler/comptime_builtins/type_reflection.rs`: resolved
  `TypeAnnotation` -> (canonical descriptor, `FrozenTypeCategory`,
  `FrozenTypeIdentity`). Leaves resolve ONLY through the overlay query API
  (alias/synonym/parameter normalization inherited, never re-implemented).
  The descriptor grammar comment is the B4/B7 ABI substrate — identities are
  SHA-256 over descriptor bytes; changing the grammar re-hashes (ABI break).
- `FreezeOverlay` grew an interior-mutable composite memo FOLDED INTO the
  existing `category_of` query (spec §4.1 one-query-API; resolution order:
  scoped parameters -> site-interned composites -> base). No new lookup
  entry point, no per-site rebuild, nothing interned on error.
- Parser: dedicated `type_ref_call` pest rule, type-annotation-first ordered
  choice with expression fallback (all A1 named rejections preserved); new
  carrier `Expr::TypeSyntax(TypeAnnotation, Span)` walked through the full
  exhaustive-match cascade with named surface-and-stop errors outside
  `type_ref` position. Bare identifiers keep the A1 `Expr::Identifier`
  lowering byte-identically.
- Rewrite + checker in lockstep: `rewrite_comptime_type_symbol_args`
  (Result-ified) and `access.rs` accept exactly
  `[Expr::Identifier] | [Expr::TypeSyntax]`; canonicalization failures are
  named compile errors BEFORE user comptime executes (Dec 52).
- Alias fixpoint interns composite alias targets (`type Pair = [int,
  string]`) through the same canonicalizer, so bare alias names agree with
  spelled composites (R7/Dec-53 — no identity split).
- LSP: textual `type_ref(` type-position detection
  (`context.rs::is_in_type_ref_type_position`, wins over the ComptimeBlock
  context, works in generic bodies and nested comptime blocks) routes to the
  EXISTING type-annotation completion provider, which now also offers
  primitive spellings, user-declared type names, and in-scope generic type
  parameters — never value bindings. `type_ref` hover/signature rows stay
  generated from the shared runtime catalog (`comptime_reflection.rs`).

### A2 rejection matrix (all named-diagnostic-asserted, green 2026-07-13)

| Row | Forbidden form | Asserting tests (frozen_type.rs unless noted) |
|---|---|---|
| R1 | String spells a composite type | `composite_string_type_ref_construction_is_still_a_string_rejection`; lsp `composite_string_type_ref_has_semantic_diagnostic` |
| R2 | Unresolved leaf at depth (incl. `dyn NoSuchTrait`) | `unresolved_leaf_*` family naming the leaf; lsp `nested_unresolved_type_ref_has_semantic_diagnostic` |
| R3 | Inference holes | `_` parses as an ordinary never-frozen NAME (unknown-identity naming `_`); Dec-52 hole family pinned at unit level (no source spelling can smuggle an analyzer tyvar) |
| R5 | Applied arity mismatch | builtin + user-struct arity from freeze facts (`type_ref applied type 'H' expects N type argument(s)...`); enum heads unchecked (documented) |
| R6 | Const-generic application | S3 named parse rejection re-fired e2e |
| R7 | Alias minted as distinct identity | positive proof: `type_ref(Ids) == type_ref(Array<int>) == type_ref(Array<UserId>)` bit-identical |
| R8 | Intersection survives as variant | object∩ == directly-spelled record identity; trait∩ == `dyn` erased identity; mixed = named rejection |
| R9 | Non-type expressions | row-7 re-fired over the S3 expression fallback |
| R11 | Order-dependent identity | unrelated-decl insertion + field/member source reorder = bit-identical identity literals |
| R12 | Composite TypeRef escapes to runtime | both comptime-only guards re-fired |
| R14 | Legacy path learns composite forms | `type_info([int, string])` parses as a VALUE array; confinement sentinel unchanged |

### A2 final verification counts (2026-07-13, all strictly additive vs the A1/A3 close baselines)

- shape-vm `compiler::comptime_builtins`: 45 passed (A1 close: 19);
  `compiler::comptime`: 115 passed / 4 pre-existing ignores (A1 close: 82);
  `functions_annotations` s3 freeze gate: 3; `compiler::monomorphization`:
  175 — all passed.
- shape-runtime: `comptime_reflection` 7 (6 + the A2 row-surface test),
  `type_schema::builtin_schemas` 6, `builtin_metadata` 4 — all passed.
- shape-ast lib: 550 passed.
- ShapeTest `comptime`: 151 passed (A1 close: 109). ShapeTest
  `annotations_comptime`: 49 passed (A1 close: 48).
- ShapeTest `lsp`: FULL target 414 passed (A1 close: 399; +12 S3-S5 module
  additions, +3 S6 completion/hover); shape-lsp lib: 787 passed
  (A1 close: 772; +9 context detection, +6 type-param/primitive provider).
- `cargo check --all-targets` across shape-ast/-vm/-runtime/-lsp/-jit/-test:
  clean. `scripts/check-no-dynamic.sh`: exit 0.


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

## ADR009-D1 Addendum (2026-07-13, ticket #15, branch adr009/d1)

Ticket D1 (spec stage 5, Decision 68; slices S1-S6) closes Still-Missing
item 8 for the EXISTING extend/materialization path: generated declarations
are ordinary compiler symbols with stable expansion provenance, and the LSP
answers navigation/references/symbols/rename/diagnostics for them from the
compiler's query surface. The declaration-discovery fixed point (Decision
67) and `shape-expansion://` virtual documents remain ticket D2 and were NOT
built (structural grep sentinels pin the exclusion).

- **Identity core** (`shape-vm/src/compiler/comptime_builtins/expansion_provenance.rs`):
  `ExpansionIdentity { generator, application, target, stage, arguments_hash,
  dependencies_hash }` with 128-bit SHA-256 canonical-descriptor hashing
  (A1 `FrozenTypeIdentity` scheme, length-prefix framed, domain-separated);
  opaque content-derived `SymbolId` (constructor private to the issuing
  module — ProofGap pattern; never a counter); `GeneratedOrigin { expansion,
  node_path, source_anchor }` with a required real `SourceAnchor`
  (`SourceMap` file id + span; `Span::DUMMY` refused at construction).
- **Stamping + identity-keyed dedup**: every directive-producing site builds
  an `ExpansionSite`; the speculative pre-pass and authoritative pass-2 build
  it from the SAME AST inputs, so one application yields ONE identity across
  both phases (idempotent `Fresh`/`Reissued` reservation). The name-string
  `materialized_comptime_fns` set is DELETED; `GeneratedSymbolTable` is the
  single source of truth (name membership is a derived view).
- **Real source anchors**: generated decl-level spans (name span, type-param
  spans) re-anchor to the application span at every directive-consumption
  point; handler wrappers anchor at the generator. SCOPE LINE (S3): the
  re-anchoring is DECL-LEVEL — body node spans keep handler-emitted offsets
  until D2 virtual documents give generated bodies addressable text
  (`GeneratedOrigin.node_path` covers attribution meanwhile), and
  `function_item_from_fragment`'s `Span::default()` sites are mini-VM
  scaffolding that never survives onto a registered declaration.
- **Query surface + diagnostics**: `BytecodeCompiler::generated_symbol_query()`
  answers provenance by `SymbolId`/name (unknown identity = named error, no
  Option-shrug) plus position/name/listing FILTERS; generated-decl compile
  failures raise C0003 anchored at the application with THREE related
  locations (generated node incl. node path, application, generator), mapped
  to LSP relatedInformation.
- **LSP navigation + rename**: goto-def on a generated-method call site opens
  the checked decl (anchored at the application until D2) and links
  application + generator; references include AST call sites + application;
  workspace/document symbols list qualified generated names that never occur
  as text; rename classifies from provenance — an explicit source binder
  (name token inside the generator/application anchors) renames by
  RECOMPUTATION (binder token + call sites only; zero edits inside generated
  ranges), a wholly generator-controlled (computed) name is NEVER a text
  edit: prepare-rename declines and rename answers the named report
  (`GENERATOR_CONTROLLED_NAME_RENAME_REPORT`) linking the generator
  definition.

### D1 rejection matrix (rows 1-10, all asserted, green 2026-07-13)

| Row | Forbidden form | Enforcement + asserting tests |
|---|---|---|
| 1 | Generated node without identity/provenance (Span::DUMMY anchor) | `GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC` at `SourceAnchor::new` + every directive-consumption point; `expansion_provenance::tests::source_anchor_rejects_dummy_span_but_accepts_offset_zero`; `functions_annotations` s2 stamping tests |
| 2 | One generated name under two expansion identities (silent first-wins) | `GENERATED_SYMBOL_CONFLICT_DIAGNOSTIC` carrying BOTH origins; `expansion_provenance::tests::one_name_under_two_identities_is_the_named_row2_error` |
| 3 | One identity expanded twice with conflicting output | `GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC` via content fingerprint; `expansion_provenance::tests::conflicting_content_for_one_reserved_identity_is_the_named_row3_error` |
| 4 | Rename on a wholly generator-controlled name produces a text edit | never an edit: `lsp/generated_rename.rs::{prepare_rename_declines_on_generator_controlled_name, rename_on_generator_controlled_name_is_a_report_not_a_text_edit}`; unit `rename::tests::generator_controlled_rename_is_the_named_report_and_never_an_edit` |
| 5 | Source-binder rename edits generated/virtual ranges | edits = source binder occurrences only; `lsp/generated_rename.rs::{rename_on_generated_method_edits_source_binder_and_call_sites_only, rename_on_generated_method_never_edits_generated_ranges, rename_on_extend_target_name_edits_source_only_and_recomputes}` |
| 6 | LSP serves generated symbols from text scans | decoy comment/string exclusions: `lsp/generated_navigation.rs::{references_on_generated_method_exclude_comment_and_string_decoys, goto_definition_on_generated_method_excludes_decoy_lines}`; qualified-name-never-in-text: `workspace_symbols_include_generated_symbol_under_its_qualified_name`; rename decoys row 5 above |
| 7 | Diagnostic on a generated node reports only the generated location | three related locations: `lsp/generated_provenance.rs` (application/generator/node-path notes at real lines) |
| 8 | Hashes over rendered source text | canonical descriptors only: `expansion_provenance::tests::{argument_and_dependency_hashes_are_formatting_insensitive, descriptor_framing_prevents_concatenation_collisions, expansion_identity_fingerprint_is_sensitive_to_every_component}` |
| 9 | D2 scope grab (fixed point, `shape-expansion://`, query graph) | grep sentinel `expansion_provenance::tests::row9_d2_scope_vocabulary_has_not_entered_the_source_tree` (URI scheme comment-only; no fixed-point/query-graph identifiers in compiler/LSP surfaces) |
| 10 | Forbidden carrier shapes (provenance bridge/shim/adapter renames, `Option<ExpansionIdentity>`, public name-string SymbolId ctor, counter identity) | grep sentinels `expansion_provenance::tests::{row10_forbidden_provenance_carrier_shapes_are_absent, identity_core_row9_structural_grep_note}` |

### D1 final verification counts (2026-07-13, all 13 gates green)

Counts at branch head `118e278b` (S6 close + review round 1 — the round-1
fix added 5 generated-symbol classification units and 4 collision
integration tests over the S6-close numbers). Strictly additive vs base
`adr009/base@fbff1b5d` (A1 close baselines in
parentheses): shape-vm `compiler::comptime_builtins` 38 (19);
`compiler::functions_annotations` 17 (3-test s3 gate at A1);
`compiler::comptime` 101 passed / 4 pre-existing ignores (82+4);
`compiler::monomorphization` 175; `no_dynamic` 1; shape-runtime
`comptime_reflection` 6 (6); shape-lsp `--lib` 788 (incl. new rename +
generated-symbol classification units); ShapeTest `lsp` 431 (399);
`comptime` 122 (109); `annotations_comptime` 52 with two threads (48);
`extend_blocks` 17. `cargo check -p shape-ast -p shape-vm -p shape-runtime
-p shape-lsp -p shape-test --all-targets` clean;
`scripts/check-no-dynamic.sh` exit 0.

Pre-existing limitation carried (S3, not a D1 regression): for the
`print(user_fn_call())` shape the JIT path writes through raw stdout instead
of the harness CaptureAdapter — reproduced byte-for-byte with a hand-written
control; the VM+JIT behavior-unchanged proofs in
`annotations_comptime/generated_method_runtime.rs` therefore assert result
values where affected.
