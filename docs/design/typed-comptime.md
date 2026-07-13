# Strictly Typed Comptime

Status: architecture accepted through ADR-009; implementation in vertical slices

This document defines Shape's target comptime model. Every target example is
illustrative syntax until an ADR accepts the spelling and implementation proves
it. Current examples are labeled separately and must have repository evidence.

## Hard Constraints

1. Comptime is typed at every boundary. It does not generate or reparse Shape
   source strings, JSON AST payloads, dynamic `Any`, or magic object shapes.
2. Types, symbols, declarations, parameters, fields, callables, traits, and
   modules are compiler-issued identities rather than semantic strings.
3. Generated fragments are hygienic and use native contextual Shape syntax,
   exact role blocks where context is ambiguous, semantic cursors for edits,
   and typed builders for computed structure.
4. Installed output re-enters complete name, type, effect, ownership, borrow,
   exhaustiveness, and native-kind checking.
5. Annotation specialization happens after the callable signature freezes.
6. Runtime argument state remains compiler-internal. Comptime emits ordinary
   typed wrapper parameters, direct calls, and finite checked rewrites.
7. VM, JIT, transfer, cache, and snapshot paths consume the same hash-covered
   expansion and callable metadata.
8. Display strings are diagnostics only and never reconstruct semantic state.

## Example Labels

- **CURRENT / VM+JIT**: checked-in evidence covers both execution modes.
- **CURRENT / VM**: a focused VM example exists; JIT parity is not proved.
- **CURRENT / compiler**: parser/compiler path exists without an applied public
  behavior proof.
- **LEGACY CURRENT**: works through a text, JSON, `Any`, or name-based path that
  the target design removes.
- **TARGET**: proposed strictly typed comptime structure or syntax.

## Current Truth

### Proven Core

**CURRENT / VM+JIT - comptime expression value**

```shape
let answer: int = comptime { 40 + 2 }
print(answer)
```

The comptime block evaluates during compilation and materializes `42`.
Capturing an unavailable runtime local is rejected as an undefined variable.

**CURRENT / VM+JIT - type reflection**

```shape
type Point { x: int, y: int }

let field_name: string = comptime {
    type_info(Point).fields[0].name
}
print(field_name)
```

Current `TypeInfo`/`TypeRef` objects work for this bounded reflection example,
but parts of their representation still use strings and `Any` internally.

**CURRENT / VM+JIT - opaque typed category reflection**

```shape
let label = comptime {
    match type_category(type_ref(int)) {
        FrozenTypeCategory::Primitive => "primitive"
        FrozenTypeCategory::Never => "never"
        FrozenTypeCategory::Parameter => "parameter"
        FrozenTypeCategory::Nominal => "nominal"
        FrozenTypeCategory::Tuple => "tuple"
        FrozenTypeCategory::Record => "record"
        FrozenTypeCategory::Callable => "callable"
        FrozenTypeCategory::Reference => "reference"
        FrozenTypeCategory::Union => "union"
        FrozenTypeCategory::Erased => "erased"
    }
}
```

`type_ref(T)` now carries an unspellable compiler-issued semantic identity;
strings and unresolved names cannot construct it. `type_category` returns a
real exhaustive enum, and transparent aliases reuse the underlying identity.
Raw type refs and category values cannot cross into runtime code; they must be
consumed inside comptime. Completion, hover, signature help, enum-variant
completion, and compile diagnostics use the same catalog and compiler path.
This is the category layer; the payload-bearing `FrozenType<T>` sum is
CURRENT-partial via `reflect()` (ADR009-B1, below).

**CURRENT / VM+JIT - `Parameter` category from generic bodies (ADR009-A3)**

```shape
fn describe<T>(value: T) -> string {
    comptime {
        match type_category(type_ref(T)) {
            FrozenTypeCategory::Parameter => "parameter"
            FrozenTypeCategory::Primitive => "primitive"
            FrozenTypeCategory::Never => "never"
            FrozenTypeCategory::Nominal => "nominal"
            FrozenTypeCategory::Tuple => "tuple"
            FrozenTypeCategory::Record => "record"
            FrozenTypeCategory::Callable => "callable"
            FrozenTypeCategory::Reference => "reference"
            FrozenTypeCategory::Union => "union"
            FrozenTypeCategory::Erased => "erased"
        }
    }
}

print(describe(1))  // "parameter"
```

A generic function whose body reflects on its own declared type parameter now
compiles and runs on VM and JIT. `type_ref(T)` inside the generic body yields
the declared pre-substitution `Parameter` identity (Decision 52: declared
generic parameters are typed parameter identities, not inference holes), never
the substituted concrete category. The identity is scoped to the BASE generic
function name — stable across instantiations of one function (`identity(1)`
and `identity("s")` observe the same identity) and distinct across owning
functions. The specialization compiler carries the base definition's declared
type parameters into the reflection snapshot via an explicit scoped overlay
(spec §4.1: overlay, not rebuild — a single derivation, no second parameter
table).

Execution semantics: a generic body's comptime block executes once PER
instantiation (generic template bodies never compile at definition;
`identity(1)` + `identity("s")` = two comptime runs). Side-effectful comptime
(`warning()`, directives) in generic bodies therefore duplicates per
instantiation by design; it is not deduplicated.

Rejections re-fire on the specialized-compile path with their named
diagnostics: an undeclared name (`type_ref(U)` inside `fn f<T>`) fails the
freeze with "unknown semantic type identity" — hard specialized-body compile
errors now propagate out of monomorphization instead of being masked as
"cannot infer type argument(s)"; soft resolution failures (unresolved type
args, specialization cycles) keep their non-error fallback. String
construction, arity, comptime-only stage, runtime escape, and match
exhaustiveness rejections all hold inside generic bodies. `Parameter`
descriptor payloads (`TypeParamDescriptor<T>`) are B7, not yet enabled.

Book status: behavior is gate-runnable green on VM and JIT; the gate-runnable
book example lands with F1 or earlier per spec §3.7 (book examples only after
gate-runnable green — satisfied for this slice).

**CURRENT / VM+JIT - canonical semantic freeze (ticket A1, 2026-07-12).** The
reflection surface above is served by ONE `SemanticFreeze` built per
compilation unit at the registration-complete barrier
(`shape-vm/src/compiler/comptime_builtins/semantic_freeze.rs`), never rebuilt
per comptime site. Scoped generic parameters enter through `FreezeOverlay`
layers; every comptime site — inline blocks, comptime expressions, and
annotation handlers, speculative pre-pass included — consumes the same
`Arc<FreezeOverlay>` handle, and a site that cannot obtain one is a named
compile error (`NO_FREEZE_HANDLE_DIAGNOSTIC`), never an empty snapshot.
Freeze-boundary failures fire before any user comptime executes (Decision 52).
Aliases and enums declared after a comptime block are visible to it
(registration-complete freeze). The LSP `type_ref`/`type_category` metadata
rows are generated from the shared runtime-owned catalog
(`shape-runtime/src/comptime_reflection.rs`), not a parallel static table.
The legacy `type_info` path (`TypeKindLabel` string vocabulary,
`__ComptimeTypeInfo` carrier) consumes the same freeze handle and is confined
to the legacy intrinsic behind an `E5-deletes` marker + sentinel test until
ticket E5 deletes it. Evidence:
`docs/cluster-audits/wave46-typed-comptime-first-tracers.md` (A1 addendum).
Book status: A1-enabled behaviors are gate-runnable in ShapeTest
(`tools/shape-test/tests/comptime/frozen_type.rs`,
`tests/annotations_comptime/frozen_reflection.rs`, VM+JIT); book-chapter
examples land in stage F1 per the program spec.

**CURRENT-partial / VM+JIT - payload-bearing `FrozenType<T>` via `reflect()`
(ADR009-B1, 2026-07-13)**

```shape
let label = comptime {
    match reflect(type_ref(bigint)) {
        FrozenType::Primitive(p) => match p {
            FrozenPrimitive::SignedInteger(w) => match w {
                IntegerWidth::Arbitrary => "signed:arbitrary"
                _ => "signed:fixed"
            }
            _ => "other-primitive"
        }
        FrozenType::Never(n) => "never"
        FrozenType::Erased(e) => "erased"
    }
}
print(label)  // "signed:arbitrary"
```

`reflect(TypeRef<T>) -> FrozenType<T>` returns the sealed payload-bearing
indexed sum (Decision 50/94) with the FIRST complete payload categories:

- **`Primitive(FrozenPrimitive)`** — the sealed 10-member sub-algebra (Unit,
  Bool, Char, SignedInteger, UnsignedInteger, BinaryFloat, Decimal, String,
  Null, Undefined) with exact width/domain payloads (`IntegerWidth` W8-W64 +
  `Arbitrary`, `FloatWidth` W32/W64). Synonym families coalesce to one
  payload (`int`/`i64`, `number`/`f64`/`float`, `string`/`str`,
  `unit`/`void`/`()`); `bigint` is `SignedInteger(IntegerWidth::Arbitrary)`
  by named decision. Payloads are typed descriptor data, never rendered
  type-name strings.
- **`Never(FrozenNever)`** and **`Erased(FrozenErased)`** — the Erased bound
  set is complete and provably empty for `any`, the only reachable erased
  spelling until A2 lands trait-bound `type_ref` syntax.

The three enabled variants carry the Dec 50/94 catalog ORDINALS (Primitive=0,
Never=1, Erased=9), not dense ids, so later B tickets add payload variants
without renumbering (comptime-ABI stability, spec §3.3). Reflecting a
category whose payload ticket has not landed is a NAMED compile-time
rejection ("reflect: the `<Category>` payload descriptor has not landed
(pending payload ticket); use type_category for the exhaustive category") —
never a partial descriptor — while `type_category` stays exhaustive over all
10 categories in the same program. Descriptors have no string `kind` field
and no nullable category fields; the sealed sum has no `Unknown`/`Any` arm
and match exhaustiveness is enforced. Descriptors (unspellable carriers AND
the spellable comptime model values) cannot cross into runtime code on any
lift channel — the comptime-result wall is a value-deep walk over nested
objects/arrays calling the shared `runtime_lift_rejection`. `reflect` is
comptime-only (the pre-existing runtime `reflect` surface stub is untouched
and unit-pinned); it works inside generic bodies per instantiation (A3
overlay) and inside annotation `@comptime` hooks. LSP hover, comptime-only
completion visibility, closed `FrozenType`/`FrozenPrimitive` variant
completion, and semantic diagnostics all derive from the shared runtime
catalog (`shape-runtime/src/comptime_reflection.rs`), no hand-written
parallel rows.

Evidence: `tools/shape-test/tests/comptime/reflect.rs` (VM+JIT, every
positive program on both engines), `tests/annotations_comptime/
frozen_reflection.rs`, `tests/lsp/typed_comptime.rs`, unit matrices in
`crates/shape-vm/src/compiler/comptime_builtins/type_reflection/tests.rs`
and `crates/shape-runtime/src/comptime_reflection.rs`; rejection-matrix
mapping in the wave46 audit B1 addendum
(`docs/cluster-audits/wave46-typed-comptime-first-tracers.md`).
Book status: B1-enabled behavior is gate-runnable green on VM and JIT in
ShapeTest; the book-chapter example lands in stage F1 per spec §3.7 (book
examples only after gate-runnable green — satisfied for this slice; the book
lives in shape-web, outside this worktree and this ticket).

**CURRENT / VM+JIT - checked type-expression syntax for `type_ref` (ticket A2, 2026-07-13)**

```shape
let label = comptime {
    match type_category(type_ref([int, string])) {
        FrozenTypeCategory::Tuple => "tuple"
        FrozenTypeCategory::Primitive => "primitive"
        FrozenTypeCategory::Never => "never"
        FrozenTypeCategory::Parameter => "parameter"
        FrozenTypeCategory::Nominal => "nominal"
        FrozenTypeCategory::Record => "record"
        FrozenTypeCategory::Callable => "callable"
        FrozenTypeCategory::Reference => "reference"
        FrozenTypeCategory::Union => "union"
        FrozenTypeCategory::Erased => "erased"
    }
}
```

`type_ref(...)` now accepts the full checked type grammar, not only bare
compiler-resolved names. Accepted spellings: tuples `[T, U]`, records
`{field: T}` / `{field?: T}`, callables `(T) -> R`, references `&T` /
`&mut T`, unions `T | U`, erased domains `any` / `dyn Trait` /
`dyn A + B`, and applied generics (`Option<int>`, `Array<User>`, user
generics, nested applications like `Option<Array<int>>`). One canonicalizer
(`shape-vm/src/compiler/comptime_builtins/type_reflection.rs`) produces a
deterministic, declaration-order-independent canonical descriptor per form
(record fields byte-sorted by name; union members deduped and byte-sorted;
`&T` vs `&mut T` significant; field optionality significant); its SHA-256
identity is the VM/JIT-shared ABI substrate for B4/B7. Normalization per
Decisions 50/94: transparent aliases normalize away through applied forms
(`type Ids = Array<UserId>` with `UserId = int` yields
`identity(Array<int>)`), structural object intersections canonicalize to
`Record`, trait intersections to `Erased` bound sets. Unresolved names at
any depth and inference holes are named freeze rejections at compile time,
before user comptime executes (Decision 52); applied arity is enforced from
freeze facts; const-generic applications are a named parse-time rejection
until B4/Dec-54 lands the const carrier. LSP completion inside the
`type_ref(` type position routes to the type-annotation provider
(primitives, user types, in-scope generic parameters — never value
bindings); hover/signature stay generated from the shared catalog row. The
surface spelling remains `type_ref(T)` (not the Dec-48 turbofish
`type_ref<T>()`) — the constructor-identity reclassification is ticket B4.
Evidence: `docs/cluster-audits/wave46-typed-comptime-first-tracers.md`
(A2 addendum); e2e in `tools/shape-test/tests/comptime/frozen_type.rs`
(per-form VM+JIT matrix + rejection matrix) and
`tools/shape-test/tests/lsp/typed_comptime.rs`.
Book status: A2-enabled behaviors are gate-runnable in ShapeTest (VM+JIT);
book-chapter examples land in stage F1 per the program spec.

**CURRENT / compiler - generated implicit capture rejection**

Annotation-generated functions are marked before body compilation. A closure
inside generated code that would implicitly capture a local fails compilation
instead of producing an incomplete environment and later `Null`. Ordinary
source closures retain their current capture inference. Explicit generated
capture packs are not yet implemented. Focused VM/JIT controls cover generated
free functions and methods with capture-free closures, closure parameters,
ordinary source captures, and deterministic rejection of local, parameter,
multiple, and `self` captures.

**CURRENT / VM+JIT - applied type annotation generation**

```shape
annotation summarize() {
    targets: [type]
    comptime post(target, ctx) {
        extend target {
            method summary() -> string { self.name }
        }
    }
}

@summarize
type User { name: string }

print(User { name: "Ada" }.summary())
```

Direct `extend target { ... }` has applied VM/JIT evidence. This does not prove
that computed source-string generation is acceptable.

### Implemented But Under-Proven

The compiler contains paths for `comptime pre`, `on_define`, `metadata`,
expression/await targets, comptime trait/impl context, variadic handler inputs,
and broad reflection. Several have only parser/compiler or VM-only evidence.
They are not treated as complete until focused positive and negative VM/JIT
examples exist.

### Legacy Semantic Paths To Remove

The migration inventory identifies fourteen classes:

1. Parsed AST serialized to JSON and parsed back into directives.
2. Source/JSON type reparsing.
3. Source/JSON body and module reparsing.
4. String-backed `TypeRef` and `Any` descriptor fields.
5. String-keyed type reflection and symbol rewriting.
6. Parameters selected by spelling.
7. String/sentinel `ItemFragment` encoding.
8. `string_lit` source escaping.
9. Helpers and annotations resolved by names after parsing.
10. Unhygienic synthetic names and magic handler roles.
11. `__original__`, wrapper, and target aliases encoded as names.
12. A parallel static comptime-extend collector.
13. Runtime hook `Any` carriers and shape inspection.
14. Stdlib source generation and template-name matching.

## Target Structure Index

Each structure receives a positive and rejected example as its design is
resolved. The decision documents below preserve the complete accepted language,
examples, rejection requirements, and implementation implications.

| Structure | Stage | Purpose | Status |
|---|---|---|---|
| `ConstValue<T>` / literal lifting | comptime | Move closed values into generated code | accepted |
| `TypeRef<T>` | comptime | Canonical type identity | accepted; CURRENT / VM+JIT — A1 canonical semantic freeze: one per-unit freeze barrier, shared query API, annotation-handler handle threading (wave46 A1 addendum); A2 checked type-expression surface CURRENT / VM+JIT — tuples `[T, U]`, records `{field: T}`, callables `(T) -> R`, references `&T`/`&mut T`, unions `T \| U`, erased `any`/`dyn Trait`, applied generics `Option<int>` incl. alias normalization through applied forms (wave46 A2 addendum) |
| `TraitRef<Trait>` | comptime | Canonical trait identity | accepted |
| `ImplRef<T, Trait>` | comptime | Branch-scoped implementation evidence | accepted |
| `exists<W...> Descriptor<W...>` | type system/comptime | Preserve heterogeneous descriptor witnesses | accepted |
| `FrozenType<T>` | comptime | Exhaustive indexed type-category sum | accepted final catalog through Decision 94; category layer CURRENT / VM+JIT via A1 (`type_category` + shared catalog); payload-bearing sum CURRENT-partial / VM+JIT via B1 — `reflect(TypeRef<T>) -> FrozenType<T>` with complete Primitive (sealed `FrozenPrimitive` + `IntegerWidth`/`FloatWidth` domains), Never, and Erased payloads at catalog-pinned ordinals 0/1/9; the 7 remaining categories reflect-reject by name (evidence: `tools/shape-test/tests/comptime/reflect.rs` VM+JIT, `tests/annotations_comptime/frozen_reflection.rs`, `tests/lsp/typed_comptime.rs`, unit `type_reflection/tests.rs` — wave46 B1 addendum); remaining payloads TARGET (B2/B4-B7) |
| `TypeParamDescriptor<T>` | comptime | Stable declared-generic identity and constraints | accepted; `Parameter` category identity CURRENT / VM+JIT (base-fn-scoped, pre-substitution, reachable from generic bodies — ADR009-A3; descriptor payloads pending B7) |
| `TypeConstructorRef<C, Params>` | comptime | Canonical nominal constructor and parameter kinds | accepted |
| `AppliedType<T, C, Args>` | comptime | Exact nominal application with typed arguments | accepted |
| `NamePolicy<Domain, Namespace>` | comptime generation | Deterministic external identifier to hygienic symbol mapping | accepted |
| `NominalShape<T>` | comptime | Exhaustive struct/enum/newtype/opaque semantic shape | accepted |
| `StructDescriptor<T>` | comptime | Applied nominal struct representation | accepted |
| `EnumDescriptor<T>` | comptime | Applied nominal enum representation | accepted |
| `NewtypeDescriptor<T, U>` | comptime | Nominal wrapper and underlying type | accepted |
| `OpaqueTypeDescriptor<T>` | comptime | Explicitly non-decomposable nominal representation | accepted |
| `RepresentationAccess<T>` | comptime authority | Complete nominal representation reflection | accepted |
| `FrozenCallable<Sig>` | comptime | Fully checked callable descriptor | accepted |
| `ParamDescriptor<Sig, I, T, Mode>` | comptime | Signature-indexed positional parameter identity | accepted |
| `FieldDescriptor<Owner, F, T>` | comptime | Owner-bound hygienic named-field identity | accepted |
| `AssociatedConstDescriptor<Owner, C, T>` | comptime declaration | Typed associated constant, separate from fields | accepted |
| `FieldInitialization<Owner, F, T>` | comptime | Required or typed-default construction policy | accepted algebra |
| `DefaultInitializer<Owner, F, T, Effects>` | comptime code | Closed checked runtime default initializer | accepted |
| `AnnotationDescriptor<A, Target, Args, Multiplicity>` | comptime | Typed applied annotation and target proof | accepted |
| `CaptureDescriptor<Sig, I, T, Mode>` | comptime | Typed closure capture identity | accepted through Decision 95 |
| `HygienicSymbol<T>` | comptime | Scope-safe generated binding identity | accepted |
| `PatternBinder<T, Mode>` | comptime code | Hygienic projected binding with stable ownership mode | accepted |
| `GuardView<T, FinalMode>` | comptime code/capability | Read-only pre-commit binder view for arm guards | accepted |
| `FrozenPattern<T, Bindings>` | comptime reflection | Exhaustive indexed view of a checked pattern | accepted catalog through Decision 93 |
| `PatternCursor<Root, Node, Bindings>` | comptime capability | Owner-bound semantic pattern location | accepted |
| `PatternEdit<Root, T, Bindings>` | comptime transform | Atomic exact-binding pattern rewrite | accepted |
| `CheckedExpr<T, Effects, Ownership>` | comptime code | Typed expression fragment | accepted |
| `CheckedPattern<T, Bindings>` | comptime code | Typed pattern with sealed coverage and ownership evidence | accepted |
| `IrrefutablePattern<T, Bindings>` | comptime code/proof | Pattern proven to cover every `T` | accepted |
| `CheckedArm<T, R, Effects>` | comptime code | Lexically scoped pattern, guard, and result body | accepted |
| `MatchPlan<T, R, Effects>` | comptime transform | Atomic exhaustive generated match | accepted |
| `CheckedStmt<Effects, Flow>` | comptime code | Typed statement fragment; binding scopes cannot leak from detached fragments | accepted |
| `CheckedBody<Sig, Captures>` | comptime code | Callable body matching one signature and complete capture set | accepted through Decision 95 |
| `CheckedItem<Decl>` | comptime code | Typed declaration/item fragment | accepted |
| `CheckedModule<Exports>` | comptime code | Typed module fragment | accepted |
| `CheckedTemplate<Sig, Captures>` | comptime code | Typed placeholder/template binding and complete capture set | accepted through Decision 95 |
| `RewritePlan<Target>` | comptime transform | Atomic identity-keyed target edits | accepted |
| `HookPlan<Sig, State>` | runtime plan | Fully typed annotation lifecycle | accepted model; documentation integration pending |

## Decision Documents

- [Values, Types, And Evidence](typed-comptime/values-types-and-evidence.md): Decisions 47-54 and 94.
- [Nominals And Members](typed-comptime/nominals-and-members.md): Decisions 55-60.
- [Annotations And Hooks](typed-comptime/annotations-and-hooks.md): Decisions 61-65.
- [Expansion And Tooling](typed-comptime/expansion-and-tooling.md): Decisions 66-69.
- [Resources And Fragments](typed-comptime/resources-and-fragments.md): Decisions 70-73 and 95.
- [Patterns And Control Flow](typed-comptime/patterns-and-control-flow.md): Decisions 74-76.
- [Guards And Exhaustiveness](typed-comptime/guards-and-exhaustiveness.md): Decisions 77-79.
- [Pattern Constructor Catalog](typed-comptime/pattern-constructor-catalog.md): Decisions 80-82.
- [Range Patterns](typed-comptime/range-patterns.md): Decisions 83-85.
- [Decimal Domain](typed-comptime/decimal-domain.md): Decision 86.
- [Sequence Patterns](typed-comptime/sequence-patterns.md): Decisions 87-88.
- [Sequence Coverage](typed-comptime/sequence-coverage.md): Decision 89.
- [Alias Patterns](typed-comptime/alias-patterns.md): Decision 90.
- [Dynamic Type Patterns](typed-comptime/dynamic-type-patterns.md): Decision 91.
- [Boolean Pattern Algebra](typed-comptime/boolean-patterns.md): Decisions 92-93.

## Decision Template

Every structure decision adds:

1. A target Shape example.
2. Its exact comptime type.
3. Legal contextual forms, builders, binders, and fragment insertions.
4. Stage, effect, ownership, and hygiene rules.
5. A rejected example with a required diagnostic.
6. Generated output or transform semantics.
7. Artifact/hash and VM/JIT implications.
8. Current implementation status and migration dependencies.

## Evidence Sources

- `docs/cluster-audits/wave41-comptime-current-surface.md`
- `docs/cluster-audits/wave41-comptime-untyped-paths.md`
- `docs/cluster-audits/wave41-comptime-example-matrix.md`
- `docs/cluster-audits/wave40-argument-pack-design-comptime-only.md`
- `docs/design/comptime-excellence.md`
- `docs/vision/rfc-comptime-transform-api-v1.md`
