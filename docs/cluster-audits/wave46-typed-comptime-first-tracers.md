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
