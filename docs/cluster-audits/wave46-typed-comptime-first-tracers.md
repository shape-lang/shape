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
