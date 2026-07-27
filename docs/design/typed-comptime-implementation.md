# ADR-009 Remaining Implementation Program

Status: program spec (2026-07-12). Baseline: `4d70508b` (Wave-46 typed-comptime
first tracers). Canonical decision: `docs/adr/009-strictly-typed-comptime-and-annotations.md`.
Architecture corrections:
`docs/adr/011-resolved-semantic-identity-and-typed-elaboration.md` and
`docs/adr/012-verified-annotation-elaboration-and-callable-transforms.md`.
Canonical design: `docs/design/typed-comptime.md` + `docs/design/typed-comptime/`
(Decisions 47-95). Implemented truth and gap list:
`docs/cluster-audits/wave46-typed-comptime-first-tracers.md`. Legacy-path
inventory: `docs/cluster-audits/wave41-comptime-untyped-paths.md` (U01-U14).

This spec covers only the remaining work. It does not reopen accepted design
questions. Tickets are tracked as GitHub issues labeled `adr-009` (map:
shape-lang/shape#1, tickets #2-#23), with native blocked-by dependencies
mirroring §6.

Architecture correction (2026-07-25): E4's shipped behavior remains evidence,
but its spelling-recognized `HookDecision`, pseudo-tuple, marker substitution,
and annotation-specific JIT path are replacement debt. E6 must not deepen
`__ComptimeTarget`; stdlib migration starts from exact descriptors and the one
two-stage annotation-elaboration seam. Contract elaboration freezes annotation
effects/outcomes before dependent checking; body/plan elaboration follows the
effective-contract freeze.

## 1. Scope

In scope (the seven program stages, mapping the Wave-46 "Still Missing" list):

1. Canonical semantic-freeze/query boundary (S1 seam close).
2. Payload-bearing `FrozenType<T>` for all ten categories, plus `TraitRef`/
   `ImplRef`, `TypeConstructorRef`/`AppliedType`, `NominalShape`, field/param
   descriptors, existential descriptor packages.
3. Typed capture descriptors and heterogeneous capture environments (S2 seam).
4. `CheckedBody`/`CheckedTemplate` installation with complete lifecycle
   validation.
5. Generated symbol identities, expansion provenance, source maps, declaration
   discovery fixed point, shared compiler/LSP expansion queries, virtual
   expansion views.
6. Deletion of the fourteen legacy string/source/JSON/`Any` path classes
   (U01-U14) after their typed replacements are complete.
7. Book/example enablement per proven behavior.

Explicitly out of scope for this program (accepted design, separate follow-on
programs; do not partially implement here):

- Pattern algebra implementation (Decisions 74-85, 87-93: `CheckedPattern`,
  `MatchPlan`, guard views, sequence/range/boolean pattern surfaces).
- The canonical arbitrary-precision decimal carrier (Decision 86 — Execution
  ABI change).
- Comptime effect capabilities and typed artifact sinks (Decisions 71-72).
- The affine `AsyncDrop`/`MustSettle` language protocol (wave40 designs). §4.2
  defines how CheckedBody installation stays complete without it.

## 2. Implemented baseline (verified 2026-07-12)

| Capability | Anchor |
|---|---|
| `type_ref(T)` → 128-bit SHA-256 `FrozenTypeIdentity`; alias-transparent; synonym-coalesced | `shape-vm/src/compiler/comptime_builtins/type_reflection.rs:16-226` |
| `TypeReflectionSnapshot` built per comptime site from BytecodeCompiler tables | `type_reflection.rs:228-296`; sites `statements.rs:1889,5635`, `expressions/mod.rs:1883` |
| `FrozenTypeCategory` (10 variants, no Unknown/Any), runtime-owned, shared with LSP | `shape-runtime/src/comptime_reflection.rs:9-70`; `shape-lsp/src/completion/mod.rs:1071` |
| Comptime→runtime lift rejection for TypeRef/category | `comptime_reflection.rs:25-40` |
| Generated implicit-capture rejection (reject-only gate) | `compiler/expressions/closures.rs:3113-3133`; marker `materialized_comptime_fns` |
| Three-mode typed closure environment (VM+JIT): `CaptureKind { Immutable, OwnedMutable, Shared }`, `ClosureLayout`, mode masks, retain/release | `shape-value/src/v2/closure_layout.rs:807-949`, `closure_raw.rs:449-492` |
| Captures as MIR loans; task-boundary sendability check | `shape-vm/src/mir/solver.rs:112,392-421` |
| Sync `Drop` dispatch + RAII scopes + `drop_async` method variant (`DropKind`) | `executor/objects/trait_object_ops.rs:692-810`; `compiler/mod.rs:196-200` |
| `extend target { method }` applied VM+JIT; two-phase materialization | `functions_annotations.rs:1384-1422,1463-1654` |

Known baseline defects this program must fix, not preserve: annotation-handler
comptime runs with an empty snapshot (`comptime.rs:1340-1344`); two coexisting
kind vocabularies (`TypeKindLabel` strings vs `FrozenTypeCategory`); LSP
reflection metadata is a parallel static table (`builtin_metadata.rs:702-742`);
generic-call specialization fails when a generic body contains comptime
reflection (blocks the public `Parameter` e2e).

## 3. Binding invariants (every ticket)

1. **Strictly typed public interfaces.** No source strings, JSON AST, token
   streams, name-selected mutation, dynamic `Any`, partially populated
   descriptors (ADR-009). Unprovable cases are compile errors, never runtime
   fallbacks. Surface-and-stop, per CLAUDE.md §Forbidden Patterns.
2. **Vertical slices only.** Each ticket lands its compiler model, public Shape
   surface, hard rejection matrix, VM entry proof, JIT entry proof, LSP
   behavior, positive+negative tests, and a documentation-status line. No
   parser-only, runtime-only, or "all descriptors at once" tickets.
3. **Comptime ABI discipline.** New categories/shapes/targets change the
   comptime ABI; canonical descriptors (never rendered text) enter expansion
   and artifact hashes shared by VM and JIT (Decisions 48-50, 54-55, 61-64).
4. **Ordering guarantees.** Freeze failures fire before user comptime executes
   (Dec 52); target/multiplicity checks fail before hooks (Dec 61); declaration
   discovery reaches its fixed point before body checking (Dec 67).
5. **LSP completeness is part of done** (Dec 66/68): completion is sink-legal;
   hover shows stage + exact descriptor type; navigation/rename cover generated
   symbols via `SymbolId`. Missing editor support = language completeness
   defect.
6. **Legacy deletion is per-class and total** — a class is deleted only when
   its typed replacement covers the working behavior recorded in the wave41
   example matrix; renamed survivals are refused (CLAUDE.md renames-to-refuse).
7. **Tests only for behavior actually enabled.** No tests pretending
   target-only structures exist. Book examples land only after the behavior is
   gate-runnable green on VM and JIT.
8. **Resource policy per `AGENTS.md`**: single global build/test lane,
   cgroup-wrapped commands; implementation sessions follow the sub-cluster
   dispatch discipline.

## 4. Seam resolutions

### 4.1 S1 — Canonical semantic freeze/query boundary

Evidence (scratchpad seam report; anchors in §2) shows three parallel type
projections: the analyzer (`TypeInferenceEngine`/env), the BytecodeCompiler
tables that feed `TypeReflectionSnapshot`, and the LSP's static
`builtin_metadata` table. Resolutions binding ticket A1:

- **One freeze barrier.** A single `SemanticFreeze` snapshot is constructed
  per compilation unit after inference/registration completes, not per
  comptime site. Scoped generic parameters enter via explicit overlay, not
  rebuild. The annotation-handler empty-snapshot gap is closed by threading
  the same freeze handle (a comptime site that cannot obtain one is a compile
  error, not an empty snapshot).
- **Canonical inputs.** The freeze reads the analyzer's resolved environment
  and the schema registry — not re-derived compiler-local tables. Where the
  compiler tables are today the only source (e.g. `struct_generic_info`
  runtime field types), A1 either promotes that data into the shared surface
  or documents it as freeze input; it does not keep two derivations of the
  same fact.
- **One query API.** Comptime evaluator intrinsics, the bytecode compiler, and
  the LSP consume the same query functions over the frozen snapshot
  (identity, category, and — as later tickets land — payload descriptors).
  The LSP's hand-written metadata rows for `type_ref`/`type_category` are
  replaced by metadata generated from the same catalog. Full incremental/
  memoized query-graph infrastructure is deferred until D2 (fixed point)
  requires it; deferral is of *infrastructure*, never of descriptor
  completeness.
- **One kind vocabulary.** `FrozenTypeCategory` (plus payload descriptors) is
  the only classification; `TypeKindLabel` strings survive only inside the
  legacy `type_info` path until E5 deletes it.
- Identity scheme (SHA-256 canonical-descriptor fingerprints, alias
  transparency, synonym coalescing, collision assertion) is kept as-is.

### 4.2 S2 — Capture packs, environment layout, lifecycle installation

Evidence: the environment-layout half exists (three-mode `CaptureKind`,
`ClosureLayout`, VM+JIT lowering, mask-driven teardown); the validation half is
greenfield (`CaptureDescriptor`/`CheckedBody` are doc-only; the Wave-46 gate is
reject-only; no AsyncDrop obligation ledger; `BindingStorageClass` and
`CaptureKind` unreconciled). Resolutions binding tickets C1-C3:

- **Mode mapping is total or rejected.** `CaptureDescriptor<Sig, I, T, Mode>`
  declares the ADR mode axis (`Move | SharedBorrow | ExclusiveBorrow`). C1
  defines the lowering onto the existing carrier: `Move` lowers to
  `CaptureKind::Immutable` or `OwnedMutable` by binding storage;
  shared-ownership captures (today's `var`/SharedCell) surface under their
  true semantics. True borrow captures are currently rejected by the borrow
  solver (`ReferenceEscapeIntoClosure`); generated code keeps that rejection
  with an explicit diagnostic until a region story exists. Every accepted mode
  has complete VM+JIT+drop semantics; every unaccepted mode is a compile-time
  rejection with a named diagnostic. No mode is partially supported.
- **Declare-and-pass replaces reject-only.** The Wave-46 gate stays as the
  default (inferred captures in generated code are errors); C1 adds the only
  legal alternative: an explicit typed capture pack resolved from scope
  structure (descriptors, never names), which the same gate validates against
  the deterministic discovery result — declared-but-unused and
  used-but-undeclared are both errors.
- **One installation chokepoint.** C2 introduces the single validator through
  which every generated body/template publishes: it re-runs type, effect,
  ownership, borrow (MIR solver with capture loans), lifetime, suspension,
  `Send` (task-boundary facts), cleanup, `Drop`, and async-drop checking for
  body + complete environment, and commits atomically or not at all.
- **Lifecycle completeness without the AsyncDrop protocol.** Validation is
  complete over *shipped* lifecycle semantics: sync `Drop`, the `drop_async`
  method variant, `DropKind` context legality, solver Send facts. Anything the
  shipped semantics cannot prove — e.g. a drop-obligated value live across a
  suspension point in a generated body, or async cleanup required in a sync
  context — is rejected at installation with a named diagnostic. When the
  wave40 AsyncDrop/MustSettle program lands, those rejections relax; nothing
  installed under this spec can become retroactively unsound.
- **Descriptor reconciliation.** C1 records the mapping between
  `BindingStorageClass` (MIR storage plan) and `CaptureKind` (closure mode) as
  part of the capture-pack model; the two axes remain distinct but the
  correspondence is explicit and asserted, not implicit.

## 5. Program stages

Stage numbers match the handoff. Ticket IDs group by track: A (freeze/query),
B (descriptors), C (captures/bodies), D (expansion/tooling), E (legacy
deletion), F (book).

**Stage 1 — freeze/query.** A1 canonical freeze + shared query API. A2 checked
type-expression syntax so `type_ref` covers tuples, records, callables,
references, unions, erased domains, applied generics (today: bare identifiers
only). A3 generic-call specialization fix so a generic body containing comptime
reflection compiles and the public `Parameter` e2e lands.

**Stage 2 — descriptors.** B1 `reflect()` returning payload-bearing
`FrozenType<T>` with the first complete categories (Primitive with the sealed
`FrozenPrimitive` sub-algebra, Never, Erased). B2 `TraitRef`/`ImplRef`/
`find_impl` evidence. B3 existential descriptor packages (`exists`,
`comptime for some`) — the iteration substrate for every heterogeneous
descriptor family. B4 `TypeConstructorRef`/`AppliedType` uniform nominal
application. B5 `NominalShape` (Struct/Enum/Newtype/Opaque) +
`FieldDescriptor`/`VariantDescriptor`/`AssociatedConstDescriptor` +
`RepresentationAccess` authority. B6 `FrozenCallable<Sig>` +
`ParamDescriptor`. B7 remaining categories (Tuple, Record, Reference, Union,
Parameter payloads) completing the ten-category catalog with exhaustiveness
proofs.

**Stage 3-4 — captures and installation.** C1 `CaptureDescriptor` + explicit
generated-capture declaration (§4.2). C2 `CheckedBody<Sig, Captures>`
installation validator (§4.2). C3 `CheckedTemplate<Sig, Captures>` + exact
typed hook inputs (hooks receive invocation values only through
signature-indexed inputs; no ambient capture).

**Stage 5 — expansion/tooling.** D1 generated `SymbolId`s, `ExpansionIdentity`
provenance, bidirectional source maps for the existing extend/materialization
path; LSP navigation/references/rename over generated symbols. D2 declaration
discovery fixed point + the shared compiler/LSP expansion query + read-only
`shape-expansion://` virtual views.

**Stage 6 — legacy deletion** (classes per U01-U14; ordered per the wave41
migration sequence). E1 typed `RewritePlan`-backed directives replace the JSON
directive protocol and spelling-selected parameters (U01, U06, plus the typed
`set return`/`set param` carriers of U02). E2 `CheckedItem`/`CheckedModule`
replace ItemFragment sentinels and body/module source reparsing (U03, U07).
E3 hygienic symbols replace synthetic names, `__original__`/wrapper/target
aliases; delete the parallel static extend collector (U10, U11, U12). E4
lands resolved ordered applied annotations, the contract/body operations of the
one `AnnotationElaboration` module, `PreparedAnnotationContract`,
`CheckedAnnotationPlan`, real `ArgumentPack<Sig>`/`Next<Sig>` Callable
Transforms, and annotation-free typed Core/MIR; then deletes the E4
spelling/AST/pseudo-tuple/marker/backend paths together with U13. E5 deletes
legacy `type_info`, string-keyed rewriting, `ComptimeTarget`, and the
string-backed descriptor schemas (U02, U04, U05), replacing consumers with
exact typed descriptors. E6 migrates `serde/derive`, `serde/serialize`,
`llm/tools`, and `@prompt` directly over those exact descriptors and the
checked elaboration seam, then deletes `string_lit` (U08, U14). E6 may not add
fields or accessors to `ComptimeTarget`.

**Stage 7 — book.** F1 full comptime/annotation book-chapter refresh with
gate-runnable VM+JIT examples for every enabled behavior, run against the full
book truth-gate universe (not the curated subset).

## 6. Tickets and blocking edges

| ID | Title | Blocked by |
|---|---|---|
| A1 | Canonical semantic-freeze snapshot and shared reflection query API | — |
| A2 | Checked type-expression syntax for `type_ref` (all type forms) | A1 |
| A3 | Generic-call specialization with comptime reflection in generic bodies | — |
| B1 | Payload-bearing `FrozenType<T>`: `reflect()`, Primitive/Never/Erased | A1 |
| B2 | `TraitRef`, `ImplRef`, `find_impl` evidence | A1 |
| B3 | Existential descriptor packages: `exists` + `comptime for some` | B1 |
| B4 | `TypeConstructorRef` + `AppliedType` uniform nominal application | A2, B1 |
| B5 | `NominalShape` + field/variant/const descriptors + `RepresentationAccess` | B3, B4 |
| B6 | `FrozenCallable<Sig>` + `ParamDescriptor` | B1, B3 |
| B7 | Remaining categories: Tuple/Record/Reference/Union/Parameter payloads | A3, B4 |
| C1 | `CaptureDescriptor` + explicit generated capture declaration | B6 |
| C2 | `CheckedBody<Sig, Captures>` installation validator | C1 |
| C3 | `CheckedTemplate<Sig, Captures>` + exact typed hook inputs | C2 |
| D1 | Generated symbol identities, expansion provenance, source maps | A1 |
| D2 | Declaration-discovery fixed point + shared expansion query + virtual views | D1 |
| E1 | Typed rewrite-plan directives; delete JSON directive protocol (U01/U06/U02-carriers) | B6, C2 |
| E2 | `CheckedItem`/`CheckedModule`; delete ItemFragment + source reparsing (U03/U07) | C2, D1 |
| E3 | Hygienic symbols; delete synthetic names/aliases + parallel extend collector (U10-U12) | D1 |
| E4 | Resolved ordered applied annotations + two-stage `AnnotationElaboration` + Callable Transforms; freeze effective contracts before callers, then delete spelling/AST/pseudo-tuple/marker/backend hook paths and `Any` hook shapes (U13) | B6, C3, ADR-011, ADR-012 |
| E5 | Delete legacy reflection: `type_info`, string-keyed rewriting, `ComptimeTarget`/string schemas; replace with exact descriptors (U02/U04/U05) | B2, B5, B7, E1, ADR-011 |
| E6 | Migrate stdlib directly to exact descriptors and checked elaboration; delete `string_lit` without extending `ComptimeTarget` (U08/U14) | E1, E2, E4, E5, ADR-012 |
| F1 | Book and example enablement over the full truth-gate universe | E6 |

Initially unblocked: A1, A3.

## 7. Per-ticket definition of done

Every ticket's issue carries this checklist, instantiated for its slice:

1. Compiler model: types/passes named, with the freeze/install ordering rules
   of §3.4 upheld.
2. Public Shape surface: exact syntax/builders, documented in the design index
   status column (TARGET → CURRENT with evidence label).
3. Hard rejection matrix: each forbidden form from the relevant decisions
   produces its named diagnostic; tests assert the diagnostics.
4. VM entry and JIT entry proofs (focused ShapeTest, both execution modes).
5. LSP: completion/hover/signature/diagnostics (and navigation/rename where
   symbols are generated) driven by the shared query surface, with tests.
6. Tests: positive + negative + schema/identity; no tests for structures the
   slice does not enable.
7. Docs: `docs/design/typed-comptime.md` status row updated; book status line
   (enabled behaviors get gate-runnable examples in F1 or earlier).
8. Considered-but-rejected compromises logged in `docs/defections.md`.
