# Strictly Typed Comptime

Status: architecture accepted through ADR-009; implementation in vertical slices

Implementation program (remaining work, tickets, blocking edges):
[typed-comptime-implementation.md](typed-comptime-implementation.md)

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
descriptor payloads (`TypeParamDescriptor<T>`) landed with ADR009-B7 Slice 2
(see the B7 status entries below).

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
freeze facts; const-generic applications inside `type_ref(...)` stay a named
parse-time rejection, and B4/Dec-54 has landed the const carrier — the checked
const path is `const_arg(N)` applied through `type_constructor(Head).apply(...)`
(the parse-time rejection message now redirects there). LSP completion inside the
`type_ref(` type position routes to the type-annotation provider
(primitives, user types, in-scope generic parameters — never value
bindings); hover/signature stay generated from the shared catalog row. The
surface spelling remains `type_ref(T)` (not the Dec-48 turbofish
`type_ref<T>()`); the constructor-identity reclassification landed as ticket B4
(the B4 CURRENT block below reclassifies frozen nominal heads through
`type_constructor(Head)`, reusing these A2 applied-type identities unchanged).
Evidence: `docs/cluster-audits/wave46-typed-comptime-first-tracers.md`
(A2 addendum); e2e in `tools/shape-test/tests/comptime/frozen_type.rs`
(per-form VM+JIT matrix + rejection matrix) and
`tools/shape-test/tests/lsp/typed_comptime.rs`.
Book status: A2-enabled behaviors are gate-runnable in ShapeTest (VM+JIT);
book-chapter examples land in stage F1 per the program spec.

**CURRENT / VM+JIT - typed trait identity and implementation evidence
(ticket B2, 2026-07-13).** `trait_ref(Trait)` yields an opaque
compiler-issued `TraitRef` — a DISTINCT identity kind from `TypeRef` (Dec 49:
a trait is not a value type; trait identities are never interned as type
identities, and there is no `FrozenTypeCategory::Trait` variant per Dec 50
rule 5). `find_impl(type_ref, trait_ref) -> Option<ImplRef<T, Tr>>` answers
ONLY from implementation evidence frozen at the same registration-complete
barrier (freeze inputs 4/5 in `semantic_freeze.rs`, read once from the
analyzer env registry via a two-sub-pass trait-then-impl predeclare walk over
both compile entry points); an unimplemented pair is `None` — never an error,
never partial evidence. The canonical descriptors (`trait:{name}`,
`impl:{trait}:{type}:{impl_name_or_default}`) enter the same 128-bit SHA-256
identity scheme as type identities, so canonical trait and implementation
identities enter generated-artifact fingerprints. Evidence is consumed in the
`Some(proof)` match arm (Dec 49 positive form, proven VM+JIT); branch scoping
is enforced as stage-boundary lift rejection plus Some-arm-only issuance (the
schema-name-checked opaque decode blocks forged evidence structurally).
Rejection matrix R1-R9 named-diagnostic-asserted with LSP semantic-diagnostic
twins; blanket-impl satisfaction, legacy numeric widening, ambiguous
unqualified-impl attribution, and post-barrier (comptime-generated/derived)
implementations are named surface-and-stops, never silent `None`. Spelling:
the landed surface is positional `trait_ref(Serializable)` matching the
landed `type_ref(int)`; Dec 49's `trait_ref<Serializable>()` turbofish lands
with ticket A2 (deviation logged in `docs/defections.md`). Legacy
`implements(T, Trait)` remains untouched until E5 deletes it. Evidence:
`docs/cluster-audits/wave46-typed-comptime-first-tracers.md` (B2 addendum).
Book status: B2-enabled behaviors are gate-runnable in ShapeTest
(`tools/shape-test/tests/comptime/trait_evidence.rs`,
`tests/lsp/typed_comptime.rs`, VM+JIT); the gate-runnable book example lands
with F1 or earlier per spec §3.7.

**CURRENT / VM+JIT - uniform nominal application (ticket B4, 2026-07-13).**
`type_constructor(C)` yields a compiler-issued `TypeConstructorRef` — an opaque
constructor descriptor (`constructor:<head_hex>`) distinct from a bare nominal
leaf, minted ONLY for a head the freeze classifies `Nominal` (R5 non-nominal and
R6 unfrozen-head are named rejections from the one freeze query, not name-string
checks). `const_arg(N)` builds a checked const argument (`const:int:{value}`
identity). `type_constructor(Head).apply(...)` transports variadic CHECKED
`type_ref`/`const_arg` carriers (R4 untyped-argument-array is structurally
impossible — the site-rewrite only lowers checked carriers), checks arity and
type-vs-const kind against the head's parameter kinds read from the SINGLE
`param_kinds_of` projection in `semantic_freeze.rs` (never a second table), then
reproduces the A2 `applied:<head_hex><arg_hex,...>` descriptor byte-for-byte.
The identity equality is asserted both directions:
`identity(type_constructor(Option).apply(type_ref(int))) ==
identity(type_ref(Option<int>))`. `nominal.refine(constructor)` returns
`Some(applied)` iff the nominal is a genuine application of that constructor and
round-trips through `applied.type_argument(I)` (which reflects the I-th argument
back to its `TypeRef`, with a named out-of-range rejection), `None` otherwise.
One model spans zero-arg nominals, builtins (`Option`/`Result`/`Array`/
collections), user generics, and const-generic applications — no per-type
reflection variant. A generic ENUM head whose parameter kinds are unrecoverable
from the freeze is a named surface-and-stop, never a guessed kind. Rejection
matrix (each a named diagnostic, VM+JIT + LSP twins): R6 unfrozen head, R5
non-nominal head, wrong arity, wrong kind (`const_arg` into a Type slot), enum-
head arity-unrecoverable, `type_argument` out of range, forged/mismatched
carrier (schema-name forgery wall), and TypeConstructorRef/AppliedType/ParamKind
lift-into-runtime. Evidence:
`docs/cluster-audits/wave46-typed-comptime-first-tracers.md` (B4 addendum).
Book status: B4-enabled behaviors are gate-runnable green on VM and JIT in
ShapeTest (`tools/shape-test/tests/comptime/typed_constructor.rs`) and LSP
(`tools/shape-test/tests/lsp/typed_comptime.rs`); book-chapter examples land in
stage F1 per the program spec.

**CURRENT / C1 branch - generated captures and exact semantic evidence
(declared capture clause LANDED on `adr009/c1-rework`).** The generated-only
capture clause is carried by `captures: Option<CaptureClause>` on
`Expr::FunctionExpr` — one carrier, which C2 `CheckedBody` staging will populate
through the same field rather than a second capture mechanism. One selector,
`comptime_builtins::capture_plan`, maps the declared mode to the emitted
`CaptureKind`; DECLARED == EMITTED is enforced mechanically by emission-time
validation (a rejected declared mode reaching emission is a hard error) plus the
K1 sentinel — `CaptureKind` variants may be named in exactly one compiler file,
with `scripts/check-no-dynamic.sh` and a unit test failing the build on any
second producer. The strict four-mode mapping, the C0901-C0912 capture
diagnostic family (clause validation C0901-C0909; generated-capture tooling
query C0910/C0911; exact reference-flow conflict C0912), opaque semantic/cache
authority, native scalar and refcounted Shared lifecycle, and compiler-query
hover/definition/references/descriptor-authoritative rename are committed; the
whole capture surface is generated-code-only (an explicit clause in ordinary
source is `[C0903]`). String has authored exact zero-fallback proofs; the
complete `HeapKind::ALL` path includes direct Matrix/MatrixSlice lifecycle
proofs.

Known blocked cell: `share` over a module binding has no zero-fallback JIT
proof — it stays blocked on the W39 F1 module-binding function-body SURFACE
(ADR-006 §2.7.14), so module-binding `share` cannot yet claim native zero
fallback. Open debt: the presentation renderer is context-free — it cannot yet
spell trait-context members or existential witnesses and falls back to the
already-computed canonical identity (spelling is a diagnostic-only consistency
check, never semantic identity). Final supervisor execution gates and whole-C1
ratification remain pending. See the [detailed C1
status](typed-comptime/c1-generated-captures.md) and [rejected-design
record](typed-comptime/c1-capture-defections.md).

**CURRENT / VM+JIT - single installation chokepoint with atomic commit (C2
`CheckedBody` validator LANDED on `adr009/c2`, ticket ADR009-C2 #13).** Every
comptime-generated body (annotation `extend` methods, generated free functions,
and `replace body` edits) now publishes through ONE atomic install transaction
(`checked_body::InstallTransaction`): a watermark + a displaced-entry undo
journal bracket the whole `compile_in_place` driver, so a rejection at analysis
OR at pass-2 body compile rolls back every publication (function table, the
name-keyed side tables, the `analyze_function_body` fact bundle, the closure
index cluster, capture packs, generated-symbol reservations) — no partial
generated body is ever observable (proven by the `c2_slice0_preflight_tests`
atomicity pins + H1/H2/M3/M4 displaced-entry pins). The §4.2 ten-check battery
(type, effect, ownership, borrow-with-capture-loans, lifetime, suspension,
`Send`, cleanup, sync `Drop`, async-drop-context) provably executes over body +
complete environment inside that span (`checked_body::battery`), reusing the
shipped analyzer/solver codes (D4) and minting new codes only for the two
genuinely new install classes: `[C0922]` (a drop-obligated value across a
suspension point in a generated body, D6 conservative/fail-closed) and `[C0923]`
(async cleanup required in a sync context). Existing-body edits (`replace body`)
run inside that one transaction under one expansion identity (D7): the
async-drop gate is evaluated over the EFFECTIVE definition, so an edit is policed
on the REPLACEMENT it ships, and the D7 shape guards `[C0924]`
(split/two-identity) / `[C0925]` (incomplete environment) are wired as
defense-in-depth. VM+JIT install-success is proven by serialized-subprocess
zero-fallback proofs (`jit_c2_install_native`: a replace-body edit and a
generated `move`-capture method both run natively in both tiers). The shared C1
LSP capture/symbol query surface OBSERVES an edited body's captures (no second
surface) — a `replace body` edit's closure capture is provenance-chained on the
same surface — with the exact specialization identity of an EDITED closure's
capture quarantined `[C0911]` (see the deferred cells).

Findings ledger: (1) a GENERATED closure's capture/reference checks are enforced
at the C1 surface layer (`[C0902]` reference-escape, the C0003 "captures must be
explicit" envelope), NOT the solver — so the battery's row-5 lifetime rejection
is `[C0902]`, not solver `[B0003]`. (2) Async-`let` blocks containing a closure
fail Future unification before the borrow check (battery row 7 is
BLOCKED-BY-INFERENCE, a named production candidate). (3) The comptime-diagnostics
firewall (`sanitize_comptime_internal`) strips any `COMPTIME_JARGON_MARKERS`
token, so every install-rejection message is marker-free ("rejected", never
"refused"); the D6 message survives the firewall by that discipline. (4) The D6
headline drop-live-across-await case was a tripwire that FLIPPED: an
install-chokepoint flag + marker-free message made D6's end gate catch it ahead
of wave40. (5) An inherited RAII emission hole (a `MethodCall`-returned `Drop`
value bound to an unannotated local never ran `drop()`) was exposed and REPAIRED
via the `initializer_call_return_drop_type` MethodCall arm. (6) The
D6-effective-def move superseded the originally-surfaced `FunctionDef` AST field
(31 files / 78 sites) — the field would have left the gate scanning the pre-edit
body; see the defections record.

Deferred cells: D6 over-rejects on liveness (any drop-obligated local + any
suspension), and wave40's AsyncDrop/MustSettle program supplies the liveness
PRECISION that relaxes it — nothing installed under C2 can become retroactively
unsound. The `[C0924]`/`[C0925]` end-to-end NEGATIVES are not constructible from
a real program without production sabotage today (structurally: one transaction,
one identity, a C1-validated discovered environment), so their guards are wired
but their e2e pins are marked not-constructible; the constructors are exercised
directly for jargon-cleanliness. An EDITED (replace-body) closure's capture is
OBSERVED on the shared LSP query surface but quarantines `[C0911]
MissingInferenceFact` on specialization identity — the structural inference facts
are recorded at ANALYSIS time and a `replace body` replacement is materialized at
PASS-2, so the analyzer never publishes a fact for the replacement's closure;
codegen/install is unaffected (declared-mode lowering), and publishing the fact is
pre-analysis materialization of directive-edited bodies (E2, blocked by C2/D1),
not a bounded C2 patch. The public typed-builder surface
(`ctx.rewrite.replace_body(quote …)` producing a `CheckedBody`) is E-track
(E1/E2, blocked BY C2), not C2: C2's public surface is the shipped
annotation/`extend`/`replace body` pathway now routing through the validator. See
the [rejected-design record](../defections.md).

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

**CURRENT / compiler+LSP - generated symbol identities, expansion provenance,
source anchors, and identity-driven tooling (Decision 68, ticket ADR009-D1,
2026-07-13).** Scope: the EXISTING extend/materialization path; the
declaration-discovery fixed point (Decision 67) and `shape-expansion://`
virtual views are now CURRENT via ticket ADR009-D2 (see the dedicated D2
status entry below). Every declaration generated on that
path is an ordinary compiler symbol: the compiler issues a content-derived
`SymbolId` with full `ExpansionIdentity { generator, application, target,
stage, arguments_hash, dependencies_hash }` + `GeneratedOrigin { expansion,
node_path, source_anchor }` provenance
(`shape-vm/src/compiler/comptime_builtins/expansion_provenance.rs`, hashing
per the A1 canonical-descriptor SHA-256 scheme — never rendered text, never a
counter). The speculative pre-pass and the authoritative pass-2 compile agree
on ONE identity per application (idempotent re-issue); dedup is
identity-keyed — the name-string `materialized_comptime_fns` set is deleted,
and one generated name under two identities or one identity with conflicting
output is a named compile error carrying both expansions' provenance.
Generated declarations anchor at real source spans (`Span::DUMMY` is the
named row-1 rejection). Tooling consumes the ONE compiler query surface
(`BytecodeCompiler::generated_symbol_query()`), never a text scan: go-to-def
on a generated-method call site opens the checked declaration and links the
application + generator definition; references/workspace+document symbols
answer via `SymbolId`; diagnostics inside generated declarations carry
generated-node (with node path) + application + generator locations as
related information; rename on an explicit source binder edits ONLY the
source binder occurrences (the expansion recomputes; zero edits land in
generated ranges), and a wholly generator-controlled name is NEVER a text
edit — rename reports generator control and links the generator definition.
Evidence: `docs/cluster-audits/wave46-typed-comptime-first-tracers.md`
(D1 addendum, rejection matrix rows 1-10 + verification counts).
Book status: D1-enabled behaviors are gate-runnable in ShapeTest
(`tools/shape-test/tests/lsp/{generated_navigation.rs,
generated_provenance.rs, generated_rename.rs}` and
`tests/annotations_comptime/generated_method_runtime.rs`, VM+JIT);
book-chapter examples land in stage F1 per the program spec.

**CURRENT / compiler+VM+JIT - hygienic generated locals (ticket ADR009-E3
slice S1, legacy class U10, 2026-07-14).** The compiler no longer names its
generated locals / parameters / handler module bindings with synthetic,
user-guessable spellings (`argN`, `__ann_args`/`__ann_ctx`/`__ann_subject`/
`__ann_result`/`__ann_before_result`, `__target_arg__`, `__ctx_arg__`). Each
is a compiler-issued hygienic token `HygienicSymbol` (minted beside `SymbolId`
in `expansion_provenance.rs`, private-field content-derived ctor over a
`HygienicRole` descriptor + per-mint nonce — the same SHA-256 framing as the
D1 identities, never a counter that keys a table and never rendered text). The
role is bound by POSITION/TYPE, never a spelling: `declare_hygienic_local`
(`compiler/helpers.rs`) keys the slot by the token's UNSPELLABLE (`\u{1}`-
prefixed) descriptor, and the comptime mini-program renders the same
descriptor for forwarder params + target/ctx bindings so declaration and
reference agree. A user reference to a former spelling now resolves to nothing
(rejection-matrix row 2), and a user local/argument spelled identically cannot
collide with or shadow the generated slot (row 1). Delivery of target/ctx to a
handler is unchanged (bound by position). Evidence:
`compiler::comptime_builtins::expansion_provenance::tests::hygienic_symbol_mint_is_deterministic_distinct_and_unspellable`
(unit) + `tools/shape-test/tests/annotations_comptime/executed_extend_authority.rs`
(`u10_former_target_binding_spelling_does_not_leak_into_handler_body`,
`u10_target_delivered_by_position_after_hygienic_rename`, VM+JIT) + the
unchanged annotations_comptime (65) / comptime (263) regression suites. Scope:
generated-LOCALS leaf only — the `ctx.original`/`ctx.target` typed
capabilities (U11) and the handler/wrapper registry + chain names (U10
handlers, `functions_annotations.rs` wrapper names) are later E3 slices.

**CURRENT / compiler+VM+JIT - hygienic generated function/handler/wrapper
names (ticket ADR009-E3 slice S2, legacy class U10, 2026-07-14).** The four
synthetic FUNCTION-registry spellings the compiler minted for its generated
wrappers/handlers — `__comptime_block__` (`comptime { }` mini-program entry),
`__comptime_handler_fn__` (annotation-`comptime`-handler mini-program entry),
`__ann_{name}_{before|after}_wrapper_{n}` (specialized runtime handler in the
outer function table), and `{name}___ann_wrapper` (foreign-annotation wrapper
slot) — are now compiler-issued hygienic tokens minted through the same
`HygienicSymbol` surface as the S1 locals (new `HygienicRole` variants
`ComptimeBlockWrapper` / `ComptimeHandlerWrapper` /
`SpecializedAnnotationHandler` / `ForeignAnnotationWrapper`). The role is bound
by the token, never a spelling; the UNSPELLABLE (`\u{1}`-prefixed) rendering is
the registry key, so a user function of the former spelling can neither collide
with the generated slot in the function table (row 1) nor be resolved to it
(row 2), and a comptime block/handler body cannot reference its own wrapper.
Distinct outer-registry wrappers are disambiguated by the compiler nonce (a
shared name would make `find_function` return the wrong handler index); the
program-isolated mini-program wrappers mint deterministically at a fixed nonce.
`reserve_generated_decl`/`SymbolId` provenance is deliberately NOT used for
these compiler temporaries — they carry no real application anchor (row-1
`SourceAnchor` rejects their dummy spans) and are never LSP-navigation targets
(see `docs/defections.md` 2026-07-14 S2 entry). Evidence:
`compiler::comptime_builtins::expansion_provenance::tests::hygienic_function_roles_mint_distinct_unspellable_identities`
(unit) +
`compiler::functions_annotations::s3_source_anchor_tests::annotation_handler_wrapper_anchors_at_the_handler_definition`
(asserts NO `*_wrapper` spelling survives in the function table) + the unchanged
annotations_comptime (65) / comptime (263) / shape-lsp `--lib` (840) / lsp
ShapeTest (494) regression suites. Scope: generated-FUNCTION/HANDLER/WRAPPER
leaf only — the `ctx.original`/`ctx.target` typed capabilities (U11) and the
`{name}___impl`/`{name}___{annotation}` chain names are later E3 slices.

**CURRENT / compiler+VM+JIT - typed `ctx.original` capability replaces the
`__original__` name-encoded alias (ticket ADR009-E3 slice S3, legacy class U11,
2026-07-14).** A `replace body { … }` directive's replacement invokes the
PRE-ANNOTATION body through the TYPED `ctx.original` capability, not the deleted
global `__original__` spelling. The pre-annotation body is compiled into a
compiler-issued HYGIENIC shadow function whose registry name is an unspellable
[`HygienicSymbol`] descriptor (new `HygienicRole::OriginalBodyShadow`, nonce
derived from the annotated function's name so the shadow re-registers
idempotently); the deleted `__original__{fn}` shadow spelling, the
`function_aliases["__original__"]` map, and ALL six of its consumers
(`mod.rs` field + init, `functions.rs` alias track/cleanup, `helpers.rs`
`find_function` alias branch, `expressions/function_calls.rs` `remote::call`
lookup) are gone. `ctx.original` is a typed `FrozenCallable` (B6) — its callable
identity is canonicalized through THE single semantic-freeze handle
(`comptime_freeze_overlay`), so requesting the capability without the handle is
the named `NO_FREEZE_HANDLE_DIAGNOSTIC` compile error (rejection-matrix row 3),
never a string/Any/partial descriptor. `ctx.original(args)` in the replacement
is rewritten to a direct typed `FunctionCall` to the hygienic shadow
(`original_body_rewrite`, an exhaustive AST fold) BEFORE MIR lowering / the
MIR-derived type-inference pass, so `ctx.original(5) + 100` types as ordinary
`int` arithmetic in both execution modes. The role is bound by the `.original`
capability member, never a guessable global: a user local spelled
`__original__x` resolves to the USER binding (row 1, collision-proof), and the
former `__original__(...)` global spelling is an ordinary undefined-function
error (row 2, no leaked spelling). Evidence:
`compiler::functions_annotations::s3_freeze_gate_tests::{ctx_original_capability_without_freeze_handle_is_the_named_row3_compile_error, ctx_original_capability_with_freeze_is_a_typed_frozen_callable}`
+ `compiler::original_body_rewrite::tests::*` (unit) +
`tools/shape-test/tests/annotations_comptime/executed_extend_authority.rs::{s3_ctx_original_invokes_pre_annotation_body_typed_path, s3_user_original_spelling_does_not_collide_reaches_body_via_ctx_original, s3_former_original_spelling_does_not_resolve_the_shadow}`
(VM+JIT) + migrated `compiler::functions::tests::test_replace_body_original_*`
+ the held annotations_comptime (68) / comptime (263) / shape-lsp `--lib` (840)
/ lsp ShapeTest (494) regression suites. Scope: `ctx.original` leaf — the
comptime `extend <target>` owner resolution and the
`{name}___impl`/`{name}___{annotation}` chain-name hygiene land in the sibling
S4 slice (block below).

Book status: S3 changes the SPELLING of an already-book-documented behavior
(the `replace body` pre-annotation invocation) from the deleted `__original__`
global to the typed `ctx.original` capability; the behavior is gate-runnable
green on VM and JIT in ShapeTest
(`executed_extend_authority::s3_ctx_original_invokes_pre_annotation_body_typed_path`).
No newly-enabled surface (no new opcode / builtin / grammar production); the
external comptime book chapter's `__original__` example text is refreshed to
`ctx.original` in the F1 book-refresh stage.

**CURRENT / compiler+VM+JIT - typed target-OWNER resolution replaces the magic
`"target"` string substitution + chain-name hygiene (ticket ADR009-E3 slice S4,
legacy class U11, 2026-07-14).** A comptime annotation handler's
`extend <target> { … }` directive now resolves its OWNER head against the
handler's POSITION-0 target parameter and a TYPED owner descriptor
(`shape_runtime::annotation_context::TargetOwner` = canonical nominal name +
`NominalShape`, ADR-009 B5), NOT the deleted hardcoded literal
`TypeName == "target"` match. `resolve_extend_owner_placeholder` rewrites the
`extend` head at the handler-execution site — the executed declaration-discovery
pre-pass AND the authoritative pass-2 compile call it with the SAME handler
(hence the same position-0 binding) and the same owner, so both phases reserve
ONE expansion identity per generated method. The role is bound by POSITION, not
a spelling: a handler that names its target parameter `t` and writes `extend t`
materializes on the annotated type, and a user type LITERALLY named `target`
resolves NOMINALLY through the ordinary type-name table when the handler's first
parameter is spelled differently (rejection-matrix row 2/3 — no `"target"`
string ever enters a symbol table by magic). The before/after runtime-hook
wrapper CHAIN names (`{name}___impl` / `{name}___{annotation}`) are now
unspellable [`HygienicSymbol`] descriptors (new
`HygienicRole::{AnnotationHookImplBody, AnnotationHookWrapper}`, stable digest
nonce so they re-register idempotently); a user function of the former spelling
neither collides with nor resolves to the generated chain slot (rows 1/2).
Scope note: the SEPARATE runtime-hook `ctx.target` field (the annotated function
as a typed callable, `§4.1.5`, load-bearing for `@remote`) is already a typed
capability and is untouched — it is not the deleted comptime `"target"`
placeholder. Evidence:
`compiler::functions_annotations::s4_target_owner_tests::{extend_head_matching_position0_binding_resolves_to_owner, literal_target_type_is_nominal_when_binding_differs, extend_head_binds_by_position_not_by_the_word_target, no_binding_leaves_extend_head_untouched, target_owner_is_a_typed_nominal_descriptor}`
(unit) +
`tools/shape-test/tests/annotations_comptime/executed_extend_authority.rs::{s4_extend_owner_binds_by_position_not_the_word_target, s4_user_type_named_target_resolves_nominally}`
(VM+JIT) + the migrated chain-hygiene assertions
`compiler::statements::tests::{test_annotated_function_generates_wrapper, test_annotation_chaining_generates_chain}`
+ the held annotations_comptime (70) / comptime (263) / shape-lsp `--lib` (840)
/ lsp ShapeTest (494) regression suites + `executed_extend_authority` 14/14
(the D7/D8/D12/R6 parity matrix through the typed path).

Book status: S4 changes the RESOLUTION MECHANISM of an already-book-documented
behavior (`extend target { … }` inside a comptime handler) from a magic
literal-`"target"` substitution to a position-bound typed owner resolution; the
observable behavior for the documented `extend target` spelling is unchanged
(the handler's first parameter is `target` in every book example), and the
newly-reachable position-bound form is gate-runnable green on VM and JIT in
ShapeTest
(`executed_extend_authority::s4_extend_owner_binds_by_position_not_the_word_target`).
No newly-enabled opcode / builtin / grammar production; the external comptime
book chapter's note that `target` is a reserved keyword is refreshed to "the
handler's first (target) parameter, bound by position" in the F1 book-refresh
stage.

**CURRENT / compiler+LSP+VM+JIT - declaration-discovery fixed point, shared
expansion query, and `shape-expansion://` read-only virtual views
(Decision 66/67, ticket ADR009-D2, 2026-07-13).** Declaration-producing
comptime (v1: annotation handlers emitting free functions and `extend`
methods) reaches a deterministic, monotone fixed point BEFORE ordinary body
checking (Decision 67 ordering): `DeclarationDiscoveryFixedPoint`
(`shape-vm/src/compiler/comptime_builtins/expansion_provenance.rs`, extending
the D1 provenance surface in place — never a fork) drives a bounded monotone
worklist (`functions_annotations.rs` discovery driver) that applies each
expansion exactly once per application identity + dependency hash (the
`claim()` run-once memo keyed on `ApplicationId` + deps-hash), records each
discovered header as immutable (`record_header` — re-derivation with a
different definition is the named `DISCOVERY_HEADER_MUTATED` rejection), and
requires every reserved generated identity to be defined exactly once at
convergence (`converge` — a reserved-but-undefined identity is
`RESERVED_IDENTITY_UNDEFINED`). Duplicates, cross-application conflicts,
trigger cycles, frontier oscillation, and unbounded generation are all
compile errors carrying expansion provenance via `build_discovery_failure`
(the nine named diagnostics `GENERATED_NODE_WITHOUT_PROVENANCE` /
`GENERATED_SYMBOL_DUPLICATE_IDENTITY` / `GENERATED_SYMBOL_CONFLICT` /
`UNKNOWN_GENERATED_SYMBOL` / `DISCOVERY_CYCLE` / `DISCOVERY_OSCILLATION` /
`DISCOVERY_UNBOUNDED` / `DISCOVERY_HEADER_MUTATED` /
`RESERVED_IDENTITY_UNDEFINED`, each with an asserting test). Compiler and LSP
consume the SAME fixed-point query surface
(`BytecodeCompiler::generated_symbol_query() -> &GeneratedSymbolTable`,
Decision 66 closing rule): there is no speculative second pass and no LSP
re-evaluator — completion, navigation, references, rename, and virtual views
all read the one converged table. `shape-expansion://` URIs
(`tools/shape-lsp/src/expansion_views.rs`) render the checked generated IR
read-only with a bidirectional source map (`virtual_to_source` /
`source_to_virtual`); the render is inspection-only and is NEVER reparsed as
compiler input (pinned by `the_module_never_reparses_its_own_render`).
Identity everywhere is the A1 canonical-descriptor SHA-256 scheme (no strings,
no JSON, no partial descriptors, no counters).
Book status: D2-enabled behaviors are gate-runnable in ShapeTest —
`tools/shape-test/tests/lsp/typed_comptime.rs`
(`completion_sees_generated_free_function_after_discovery`,
`generated_free_function_visible_to_later_source_runs_identically_in_vm_and_jit`
VM+JIT, `generated_call_site_renders_read_only_virtual_view_with_source_map`)
plus the `expansion_views.rs` in-file view/source-map suite and the
`expansion_provenance.rs` rejection-matrix + deterministic-identity units;
book-chapter examples land in stage F1 per the program spec.

**CURRENT / VM+JIT - fully checked callable descriptors (Decisions 52 + 63,
ticket ADR009-B6, 2026-07-13).** `reflect(TypeRef<callable>)` yields
`FrozenType::Callable(FrozenCallable)` (catalog ordinal 6, pinned; no ordinal
renumber) carrying the callable's ordered structural descriptor: one
`ParamDescriptor` per parameter — value-type frozen identity, `optional` flag,
and the sealed `PassingMode` axis (`Move` / `SharedBorrow` / `ExclusiveBorrow`,
derived from the parameter's borrow annotation, since mode is not encoded in the
one-way SHA-256 identity string) — plus the return-type identity. The descriptor
is reconstructed from the freeze's WIDENED composite memo
(`semantic_freeze.rs` `CompositeMemoEntry { category, callable }`) rather than a
parallel identity-keyed side-table, so category and structure are one write, one
read, one lock (see `docs/defections.md`). Per Decision 52 (R3), a descriptor is
issued only AFTER the signature freezes: a signature carrying an unresolved
inference variable at any depth is the named freeze-boundary rejection — fired by
the single `annotation_has_unresolved_inference_variable` predicate BEFORE any
user comptime hook runs, never a partial descriptor. Public surface:
`callable.param(I)` (signature-indexed positional, computed indices accepted) and
`callable.parameters` (ordered collection, iterated via the B3 `comptime for some`
existential machinery — no second protocol), both desugaring at comptime-prep to
the single S1 descriptor carrier (one producer, no second freeze query). Rejection
matrix (each a named diagnostic): R1 string-keyed selection
(`callable.param("name")` → `PARAM_STRING_SELECTOR_DIAGNOSTIC`; names are
identity-insignificant hygiene facts, never a runtime string key), R2 params typed
`Array<Any>` / bare capital `Any`
(`CALLABLE_PARAM_ERASED_TO_ANY_DIAGNOSTIC`, reusing the single
`annotation_erases_witness_to_any` walk; lowercase `any` still reaches the Erased
leaf), R3 unresolved-signature issuance (above). LSP completion and `reflect(`
signature help for the new payload types flow through the ONE shared
`reflection_enum_variant_names` catalog lookup (auto-extended by the enabled-payload
macro): `PassingMode` completes its sealed axis from `PassingMode::ALL`, while the
`FrozenCallable` / `ParamDescriptor` payload STRUCTS return `None` and fall through
cleanly (no fabricated variants); the LSP `ParamReferenceMode { Shared, Exclusive }`
axis is a machine-checked projection of the shared ADR `PassingMode` axis
(`from_passing_mode` / `to_passing_mode`, exhaustive over `PassingMode::ALL`,
`Move -> None`) — no hand-written LSP table, no parallel mode enum. The
hygienic-token `param(#name)` and turbofish `param<I>()` SPELLINGS depend on
grammar the parser does not yet carry (`#ident` token / method-call const-arg
slot); they are TARGET, surfaced-and-stopped, with the same positional selection
already CURRENT as `param(I)` and the names preserved in the freeze structure ready
for them. Considered-but-rejected compromises (composite-memo widening vs parallel
side-table, string `param()` selector, `Array<Any>` param modeling, faked hygienic
spellings, hand-written LSP variant list) are logged in `docs/defections.md`.
Book status: B6-enabled behaviors are gate-runnable green on VM and JIT in
ShapeTest (`tools/shape-test/tests/comptime/{reflect.rs,callable.rs,annotations.rs}`)
and LSP (`tools/shape-test/tests/lsp/typed_comptime.rs` plus `shape-lsp`
`completion/mod.rs` + `type_inference.rs` units); book-chapter examples land in
stage F1 per the program spec.

Book status: B5 S1-enabled behavior (`FrozenNominal.shape()` discrimination of
the sealed `NominalShape` sum — Struct/Enum/Newtype/Opaque — on VM and JIT) is
gate-runnable green in ShapeTest (`tools/shape-test/tests/comptime/nominal.rs`
+ `reflect.rs`) and LSP (`tools/shape-test/tests/lsp/typed_comptime.rs`
`nominal_shape_completion`); generic substitution, field/variant iteration,
representation authority (`reflect_repr`), and hygienic `#name` selection are
later B5 slices; book-chapter examples land in stage F1 per the program spec.

Book status: B5 S2-enabled behavior (generic substitution before descriptor
issuance — reflecting an applied user struct `Pair<User>` substitutes field
type `T` → `User` before issuing `FieldDescriptor`s — plus derive-style
`record.fields` / `enum.variants` iteration over the typed descriptor rows) is
gate-runnable green in ShapeTest on VM and JIT
(`tools/shape-test/tests/comptime/nominal.rs`) and unit-proven at the freeze
layer (`type_reflection/tests.rs::applied_user_struct_substitutes_field_types_before_issuance`);
hygienic `#name` explicit selection remains a later B5 slice; book-chapter
examples land in stage F1 per the program spec.

Book status: B5 S3-enabled behavior (authority-gated `reflect_repr(type_ref,
access)` exposing the complete nominal representation under a compiler-issued
`RepresentationAccess<T>` delivered to a declaration-attached annotation
type-target expand hook, plus the hard member-selection rejection matrix R1-R11
each with a named diagnostic) is gate-runnable green in ShapeTest on VM and JIT
(`tools/shape-test/tests/comptime/nominal.rs`) and unit-proven at the freeze/
carrier layer (`type_reflection/tests.rs::carriers::representation_access_*`) +
LSP (`completion/mod.rs` reflect_repr row); the public-vs-complete field-set
distinction (field visibility) and hygienic `#name` selection remain
grammar-pending (see `docs/defections.md`); book-chapter examples land in stage
F1 per the program spec.

Book status: B5 S4-enabled behavior (the LSP nominal-descriptor surface —
hover over `reflect_repr` renders the descriptor TYPES it exposes:
`FieldDescriptor<Owner, #f, T>` with its owner nominal + hygienic member token
`#f`, the sibling `VariantDescriptor` / `AssociatedConstDescriptor`, and the
`FrozenNominal<T>.shape()` entry into the sealed `NominalShape`; `reflect_repr`
completes as a comptime-only builtin; the `NominalShape` result axis +
`FieldInitialization` disposition complete through the ONE shared
`reflection_enum_variant_names` lookup, and the `FieldDescriptor` /
`VariantDescriptor` / `AssociatedConstDescriptor` / `FrozenNominal` payload
STRUCTS fall through it with no fabricated variant list) is gate-runnable green
in ShapeTest LSP (`tools/shape-test/tests/lsp/typed_comptime.rs::{reflect_repr_hover_renders_descriptor_types_and_owner,
comptime_completion_offers_the_reflect_repr_reflection_entry,
nominal_shape_result_axis_completes_through_the_shared_reflection_lookup}`) and
`shape-lsp` units (`completion/mod.rs::test_nominal_member_descriptor_structs_have_no_enum_variant_completions`);
all hover/completion behavior is driven by the shared `comptime_reflection`
catalog (`REFLECT_REPR_BUILTIN_ROW` spliced verbatim into
`builtin_metadata::CORE_BUILTINS`), never a hand-written LSP table. Hygienic
`#name` explicit member selection stays grammar-blocked (B7; the descriptor
TYPE carries the member position as the `#f` token, and the hover surfaces it
there, but no `record.field(#name)` call spelling exists — see
`docs/defections.md`); book-chapter examples land in stage F1 per the program
spec.

**CURRENT / VM+JIT - the four composite `FrozenType` payloads (ticket
ADR009-B7, 2026-07-14).** `reflect(TypeRef<T>)` now answers the four composite
categories with complete structural payloads, joining B6 Callable + B5 Nominal
to make NINE enabled payloads:

- **`Tuple(FrozenTuple)`** (catalog ordinal 4) — the ordered `elements` array,
  each a `TupleElement` carrying its positional `index` + the element type's
  frozen identity halves. Position IS the index.
- **`Record(FrozenRecord)`** (ordinal 5) — the normalized structural record's
  `fields` array (byte-sorted by member), each a `RecordField` carrying an
  owner-bound HYGIENIC member identity (minted from the record's own identity +
  field name, never a source string — Dec 57), the field's value-type identity,
  and its `optional` flag. Object intersections normalize to the same Record.
- **`Reference(FrozenReference)`** (ordinal 7) — the `mutable` flag (`&T` vs
  `&mut T`) + the referent type's frozen identity halves.
- **`Union(FrozenUnion)`** (ordinal 8) — the deduped, byte-sorted `members`
  array (each a `UnionMember` identity); a singleton union coalesces to its
  member upstream, so a `FrozenUnion` always carries ≥2 members.

Every payload is reconstructed WITHOUT inverting the one-way SHA-256 identity:
the structural descriptors are threaded on `CanonicalType`, carried through the
freeze's WIDENED composite memo (site-interned composites) and per-category
BASE-index maps (composite aliases like `type Pair = [int, string]`), through
the ONE query surface (`FreezeOverlay::payload_of` /
`FrozenTypeIndex::payload_for_identity`) — never a partial descriptor, never a
string `.kind` field. Reference mutability is a typed `bool` (Shape has no
declared variance/ownership on references — see `docs/defections.md`); the
`Parameter` payload (`TypeParamDescriptor<T>`) landed with B7 Slice 2 (below),
leaving the `Existential` witness payload (B3-S3) as the sole named per-category
R1 rejection. Evidence:
`tools/shape-test/tests/comptime/reflect.rs::{reflect_on_composite_type_expressions_selects_the_matching_payload_arm,
tuple_reflects_ordered_element_identities, record_reflects_normalized_fields_with_optionality,
reference_reflects_mutability_and_referent, union_reflects_normalized_member_identities,
reflect_on_composite_alias_answers_the_same_composite_payload}` VM+JIT, unit
`type_reflection/tests.rs::payload_query::site_interned_composites_answer_full_structural_payloads`,
`comptime.rs::reflect_composite_payloads_decode_through_the_injected_model`,
`builtin_schemas` ordinal round-trip, and
`tests/annotations_comptime/frozen_reflection.rs::annotation_handler_reflect_composite_payload_reaches_generated_fn`;
LSP `FrozenType` completion auto-derives the four variants from the shared
catalog (`tests/lsp/typed_comptime.rs`, `completion/mod.rs`). Book-chapter
examples land in stage F1 per the program spec.

**CURRENT / VM+JIT - the `Parameter` payload completes the ten-category catalog
(ticket ADR009-B7 Slice 2, 2026-07-14).** `reflect(TypeRef<T>)` inside a generic
body now answers `Parameter(FrozenParameter)` (catalog ordinal 2) — the type
parameter's stable base-fn-scoped `parameter:{owner}:{name}` frozen identity
halves + a bound-set array. This is the public A3 path: the identity is
declaration-stable across instantiations (Decision 52, never the mono key,
never an inference hole), issued only off a scoped overlay identity through the
ONE query surface (`FreezeOverlay::payload_of`). The bound set is provably empty
today — `FrozenParameterBound` is uninhabited until ticket B2 lands the
trait-reference bound descriptors (the same B2 element that inhabits
`FrozenErased.bounds`), which is the honest "bounds where representable" form,
never a partial descriptor (Shape has no declared variance/ownership syntax on
type parameters, so the identity is the complete representable payload today —
see `docs/defections.md`). With `Parameter` enabled, **all ten Dec 50/94
categories are enabled**; only `Existential`'s witness-iteration payload (B3-S3)
remains the sole named per-category R1 rejection. Evidence:
`tools/shape-test/tests/comptime/reflect.rs::{reflect_on_generic_parameter_yields_the_parameter_payload,
parameter_payload_carries_a_provably_empty_bound_set,
reflect_on_undeclared_name_in_generic_body_is_the_named_rejection}` VM+JIT,
`tools/shape-test/tests/comptime/frozen_type.rs::parameter_payload_is_reachable_from_a_generic_body`
VM+JIT, unit
`type_reflection/tests.rs::scoped_parameter_answers_its_stable_identity_payload`
(identity stability across same-owner overlays + distinctness across owners),
`semantic_freeze.rs::payload_query_grows_the_shared_query_api`,
`comptime_reflection.rs::{parameter_descriptor_schema_has_its_own_named_lift_rejection,
enabled_payload_descriptor_type_names_derive_from_the_catalog,
frozen_type_payload_variant_ordinals_are_catalog_pinned}`, and the mini-VM
ten-way exhaustive match in `comptime.rs`; LSP `FrozenType` completion
auto-derives `Parameter` from the shared catalog
(`completion/mod.rs::test_frozen_type_variant_completion_is_closed_to_enabled_payloads`).
Book-chapter examples land in stage F1 per the program spec.

**CURRENT / LSP - the ten-category payload surface is catalog-closed in the LSP
(ticket ADR009-B7 Slice 3, 2026-07-14).** Every LSP behavior over the five new
B7 payload descriptor type names (`FrozenTuple` / `FrozenRecord` /
`FrozenReference` / `FrozenUnion` / `FrozenParameter`) and their element structs
(`TupleElement` / `RecordField` / `UnionMember` / `TypeParamDescriptor`) is
DRIVEN FROM the shared reflection catalog — there is no hand-written LSP variant
row (spec §4.1: the LSP consumes the same query surface). `FrozenType::`
completion offers exactly the ten catalog-enabled payload variants (auto-derived
from `FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES`, `Existential` excluded); the
`reflect` hover enumerates exactly those ten (the row description embeds
`FROZEN_TYPE_ENABLED_PAYLOADS_DOC`); and every enabled payload descriptor that
is a struct (all but the `FrozenPrimitive` enum sub-algebra) plus every element
struct completes with NO fabricated `::` variants (`reflection_enum_variant_names`
returns `None`), exactly as B5's `FieldDescriptor` family and B6's
`FrozenCallable`. Because the assertions iterate the shared catalog, a future
ticket enabling another category auto-extends every lock — no per-name LSP edit.
Evidence: `tools/shape-test/tests/lsp/typed_comptime.rs::{frozen_type_completion_is_closed_to_enabled_payload_variants,
reflect_hover_lists_the_b7_composite_and_parameter_payloads}`,
`tools/shape-lsp/src/completion/mod.rs::{test_frozen_type_variant_completion_is_closed_to_enabled_payloads,
test_enabled_payload_struct_type_names_have_no_fabricated_variant_completions,
test_b7_element_descriptor_structs_have_no_enum_variant_completions}`,
`tools/shape-lsp/src/hover_tests.rs::{test_comptime_builtin_hover_reflect_uses_shared_catalog,
test_comptime_builtin_hover_reflect_excludes_non_enabled_categories}`, and unit
`crates/shape-runtime/src/comptime_reflection.rs::enabled_payload_struct_type_names_have_no_variant_arm`.

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
| `TraitRef<Trait>` | comptime | Canonical trait identity | accepted; CURRENT / VM+JIT — B2 distinct frozen trait identity (`trait:{name}` SHA-256 descriptors, never interned as type identities; positional `trait_ref(Trait)` surface, turbofish pending A2) (wave46 B2 addendum) |
| `ImplRef<T, Trait>` | comptime | Branch-scoped implementation evidence | accepted; CURRENT / VM+JIT — B2 `find_impl(type_ref, trait_ref) -> Option<ImplRef<T, Tr>>` over barrier-frozen evidence, Some-arm consumption + None arm proven VM+JIT; branch scoping = stage-boundary lift rejection + Some-arm-only issuance (wave46 B2 addendum) |
| `exists<W...> Descriptor<W...>` | type system/comptime | Preserve heterogeneous descriptor witnesses | accepted; CURRENT-partial / VM+JIT via B3 — `exists<W...> Descriptor<W...>` package type (positional de-Bruijn `exists:{arity}:{inner}` SHA-256 identity), existential introduction/subsumption in the unifier, and `comptime for some<W...> x in coll {}` iteration sugar over the single reflect()/payload surface (no second protocol): fresh per-iteration hidden witnesses scoped by the freeze overlay, witness-typed loop binding, escape + non-existential + no-freeze + erased-to-Any rejections. Proven over the B1 `Array<exists<T> FrozenType<T>>` reflect-payload substrate (evidence: `tools/shape-test/tests/comptime/existential.rs`, `tests/lsp/typed_comptime.rs`, unit `constraints.rs`/`comptime_builtins/existential.rs` — ADR009-B3 S1). Engine scope: the `comptime for some` iteration runs at compile time on the comptime VM interpreter (comptime never tiers up to the JIT); the VM/JIT dual-run proves the enclosing program lowers and runs identically under both tiers (JIT-tier-clean at the comptime→runtime boundary), NOT that the JIT iterates the collection. LSP hover/inlay over a `some`-bound witness binding renders the opened descriptor (`FrozenType<T>`) + stage + escape rule via the shared `open_comptime_some_descriptor` surface, and hovering a `some`-clause witness name renders the opened hidden witness (evidence: `tests/lsp/typed_comptime.rs::{hover_on_some_bound_loop_var_shows_opened_descriptor_and_stage, hover_on_some_witness_name_shows_opened_hidden_witness, inlay_on_some_bound_iteration_shows_opened_descriptor}` — ADR009-B3 S3). Descriptor families beyond B1 (fields/params/variants) TARGET pending B5-B7 |
| `FrozenType<T>` | comptime | Exhaustive indexed type-category sum | accepted final catalog through Decision 94; category layer CURRENT / VM+JIT via A1 (`type_category` + shared catalog); payload-bearing sum CURRENT-partial / VM+JIT via B1+B6+B5+B7 — `reflect(TypeRef<T>) -> FrozenType<T>` with complete Primitive (sealed `FrozenPrimitive` + `IntegerWidth`/`FloatWidth` domains), Never, Erased, Callable (B6 `FrozenCallable`), Nominal (B5 `FrozenNominal`), the four composites Tuple (`FrozenTuple`), Record (`FrozenRecord`), Reference (`FrozenReference`), Union (`FrozenUnion`) (B7 Slice 1), and Parameter (`FrozenParameter`, B7 Slice 2, the public A3 base-fn-scoped-identity path) payloads at catalog-pinned ordinals 0/1/9/6/3/4/5/7/8/2 — TEN enabled payloads = the COMPLETE Dec 50/94 catalog; only Existential (B3-S3 witness payload) reflect-rejects by name (evidence: `tools/shape-test/tests/comptime/reflect.rs` + `nominal.rs` + `frozen_type.rs` VM+JIT, `tests/annotations_comptime/frozen_reflection.rs`, `tests/lsp/typed_comptime.rs`, unit `type_reflection/tests.rs` — wave46 B1 + ADR009-B6/B5/B7 addenda); LSP `FrozenType` completion + reflect hover auto-derive all ten from the shared catalog (`completion/mod.rs::{test_frozen_type_variant_completion_is_closed_to_enabled_payloads, test_enabled_payload_struct_type_names_have_no_fabricated_variant_completions}`, `hover_tests.rs::test_comptime_builtin_hover_reflect_excludes_non_enabled_categories`), no hand-written rows |
| `TypeParamDescriptor<T>` | comptime | Stable declared-generic identity and constraints | accepted; `Parameter` category identity CURRENT / VM+JIT (base-fn-scoped, pre-substitution, reachable from generic bodies — ADR009-A3; descriptor payloads pending B7) |
| `TypeConstructorRef<C, Params>` | comptime | Canonical nominal constructor and parameter kinds | accepted; CURRENT / VM+JIT — B4 `type_constructor(C)` yields a compiler-issued constructor descriptor (`constructor:<head_hex>`, distinct from a bare nominal leaf), minted only for a frozen nominal head; R5 non-nominal / R6 unfrozen-head named rejections; parameter kinds projected from the single `param_kinds_of` freeze source, never a second table (wave46 B4 addendum) |
| `AppliedType<T, C, Args>` | comptime | Exact nominal application with typed arguments | accepted; CURRENT / VM+JIT — B4 `.apply(...)` checks arity + type-vs-const kind then reproduces the A2 `applied:` descriptor byte-for-byte, so `identity(type_constructor(Option).apply(type_ref(int))) == identity(type_ref(Option<int>))` both directions; `const_arg(N)`, `nominal.refine(constructor)` round-trip, `applied.type_argument(I)` (wave46 B4 addendum) |
| `NamePolicy<Domain, Namespace>` | comptime generation | Deterministic external identifier to hygienic symbol mapping | accepted |
| `NominalShape<T>` | comptime | Exhaustive struct/enum/newtype/opaque semantic shape | accepted; CURRENT / VM+JIT via B5 S1 — `reflect(TypeRef<nominal>) -> FrozenType::Nominal(FrozenNominal)` (catalog ordinal 3); `FrozenNominal.shape()` projects the sealed `NominalShape` sum, discriminated exhaustively by a typed `match` (never a `.kind` string, R4). Un-applied generic head + applied-generic (pre-substitution) forms are named rejections (R10/R11). S1 CURRENT classification absent dedicated newtype/opaque syntax = struct field count (0=Opaque, 1=Newtype, ≥2=Struct — see `docs/defections.md`). Evidence: `tools/shape-test/tests/comptime/nominal.rs` + `reflect.rs` VM+JIT, unit `type_reflection/tests.rs::nominal_shape_classification_and_member_identities`, `builtin_schemas` round-trip, `tests/lsp/typed_comptime.rs::nominal_shape_completion` — ADR009-B5 S1. Generic substitution (`Page<User>` → substituted field types) + derive-style field/variant iteration CURRENT / VM+JIT via B5 S2 (`nominal.rs::{struct_field_iteration_counts_fields, enum_variant_iteration_reads_arities, generic_substitution_precedes_field_issuance}`, unit `type_reflection/tests.rs::applied_user_struct_substitutes_field_types_before_issuance`). Authority-gated `reflect_repr` + the hard member-selection rejection matrix R1-R11 (R1 string-field, R2 ordinal, R4 `.kind` string, R5 runtime-repr-class, R6 no-authority, R7 non-filtered ordinary reflect, R9 comptime-field excluded, R10 substitution, R11 freeze-boundary — each a named diagnostic) CURRENT / VM+JIT via B5 S3 (`nominal.rs::{r1_string_field_selection, r2_ordinal_field_selection, r4_kind_string_read, r5_r9_runtime_representation_class_read, r9_comptime_field_is_not_a_struct_field, reflect_repr_*}`). LSP surface CURRENT via B5 S4 — hover over `reflect_repr` renders the exposed descriptor TYPES (`FieldDescriptor<Owner, #f, T>` with owner + hygienic member token `#f`, `VariantDescriptor`, `AssociatedConstDescriptor`) and the `FrozenNominal<T>.shape()` entry into the sealed `NominalShape`; the `NominalShape` result axis + `FieldInitialization` disposition complete through the ONE shared `reflection_enum_variant_names` lookup (the descriptor payload STRUCTS fall through it, no fabricated variants), all catalog-driven, no hand-written LSP table (evidence: `tools/shape-test/tests/lsp/typed_comptime.rs::{reflect_repr_hover_renders_descriptor_types_and_owner, comptime_completion_offers_the_reflect_repr_reflection_entry, nominal_shape_result_axis_completes_through_the_shared_reflection_lookup}`, unit `completion/mod.rs::test_nominal_member_descriptor_structs_have_no_enum_variant_completions`). `#name` explicit selection remains grammar-blocked (B7; the descriptor TYPE carries the member position as `#f` and the hover surfaces it there, but no `record.field(#name)` call spelling exists — see `docs/defections.md`) |
| `StructDescriptor<T>` | comptime | Applied nominal struct representation | accepted; CURRENT / VM+JIT via B5 S1+S2 — owner identity + field count + ordered `FieldDescriptor` array (typed member/value identities); derive-style field iteration over substituted generics (`Pair<User>` field `T` → `User`) proven dual-engine (B5 S2) |
| `EnumDescriptor<T>` | comptime | Applied nominal enum representation | accepted; CURRENT / VM+JIT via B5 S1+S2 — owner identity + variant count + ordered `VariantDescriptor` array (member identity + payload arity), derive-style variant iteration proven dual-engine (B5 S2). Full per-variant payload field TYPES are a later slice |
| `NewtypeDescriptor<T, U>` | comptime | Nominal wrapper and underlying type | accepted; CURRENT / VM+JIT via B5 S1 — owner identity + the wrapped inner type's frozen identity |
| `OpaqueTypeDescriptor<T>` | comptime | Explicitly non-decomposable nominal representation | accepted; CURRENT / VM+JIT via B5 S1 — owner identity only (no decomposable representation) |
| `RepresentationAccess<T>` | comptime authority | Complete nominal representation reflection | accepted; CURRENT / VM+JIT via B5 S3 — `reflect_repr(type_ref, access) -> FrozenType<T>` exposes the complete representation ONLY under a compiler-issued `RepresentationAccess<T>` authority (unspellable SOH-prefixed carrier, identity-bound), delivered to a declaration-attached annotation type-target expand hook as the third positional `access` parameter (author consent, Dec 56). The mint intrinsic is unspellable and injected only into the handler mini-VM, so user code can never obtain a capability. Ordinary `reflect()` exposes identity + public interface, never a filtered/partial view (R7); `reflect_repr` without a genuine authority is the named R6 rejection; a `RepresentationAccess<T>` cannot decompose another type (identity-bound authority). Evidence: `tools/shape-test/tests/comptime/nominal.rs::{reflect_repr_with_authority_exposes_complete_shape, ordinary_reflect_is_not_a_filtered_representation, reflect_repr_without_authority_is_the_named_r6_rejection}` VM+JIT, unit `type_reflection/tests.rs::carriers::{representation_access_carrier_gates_reflect_repr, representation_access_is_bound_to_its_own_type, representation_access_mint_rejects_unknown_identity}`, `builtin_schemas` round-trip — ADR009-B5 S3. The public-vs-complete field-set distinction collapses until field-visibility (`public`/`private`) syntax lands (grammar-pending; see `docs/defections.md`) |
| `FrozenCallable<Sig>` | comptime | Fully checked callable descriptor | accepted; CURRENT / VM+JIT via B6 (S1–S3) — `reflect(TypeRef<callable>) -> FrozenType::Callable(FrozenCallable)` (catalog ordinal 6) carrying the ordered `params` array + return type identity, reconstructed from the freeze's WIDENED composite memo (the one-way SHA-256 identity drops names/modes by design; memo widened rather than a parallel side-table — see `docs/defections.md`). Issued only after the signature freezes: an unresolved-signature signature is the Dec-52 freeze-boundary rejection BEFORE any hook (R3). Evidence: `tools/shape-test/tests/comptime/reflect.rs::{callable_reflects_to_a_frozen_callable_with_param_count, callable_param_passing_modes_reflect_on_vm_and_jit}` VM+JIT, unit `type_reflection/tests.rs::payload_query` + `builtin_schemas.rs` — ADR009-B6 S1. Param accessor surface CURRENT / VM+JIT via B6 S2 — signature-indexed positional `callable.param(I)` (computed index accepted) + `callable.parameters` (ordered collection, comptime-iterable) desugar to the S1 descriptor carrier; R1 string-keyed selection (`param("name")`) + R2 `Array<Any>` params are named rejections; VM+JIT annotation entry proof reads param modes/arity at compile time (`tools/shape-test/tests/comptime/callable.rs`, `tests/comptime/annotations.rs::b6_*`); LSP `FrozenType::Callable`/`PassingMode` completion + `reflect(` signature help auto-derived from the shared catalog (`tests/lsp/typed_comptime.rs`). LSP surface CURRENT / via B6 S3 — enum-variant completion for the new payload types flows through the ONE shared `reflection_enum_variant_names` lookup (`PassingMode` completes its sealed axis; `FrozenCallable`/`ParamDescriptor` are structs → fall through, no fabricated variants), and the LSP `ParamReferenceMode { Shared, Exclusive }` axis is reconciled as a machine-checked projection of the shared ADR `PassingMode { Move, SharedBorrow, ExclusiveBorrow }` axis (`Move -> None`; `from_passing_mode`/`to_passing_mode`, exhaustive over `PassingMode::ALL`) — no hand-written LSP variant table, no parallel mode enum (`tools/shape-lsp/src/{completion/mod.rs,type_inference.rs}` unit tests). The hygienic-token `param(#name)` spelling depends on a `#ident` token the grammar does not yet carry (TARGET B6 S2+; names are preserved in the freeze structure ready for it) |
| `ParamDescriptor<Sig, I, T, Mode>` | comptime | Signature-indexed positional parameter identity | accepted; CURRENT / VM+JIT via B6 (S1–S3) — one positional descriptor per param: value-type frozen identity, `optional` flag, and the sealed `PassingMode` axis (`Move`/`SharedBorrow`/`ExclusiveBorrow`, derived from the parameter's borrow annotation — mode is NOT in the identity string). Names identity-insignificant, preserved in the freeze structure for hygienic `param(#name)` (grammar-pending; positional `param(I)` CURRENT via B6 S2). Evidence as `FrozenCallable` above + `tools/shape-test/tests/comptime/callable.rs` — ADR009-B6 S1–S3 |
| `FieldDescriptor<Owner, F, T>` | comptime | Owner-bound hygienic named-field identity | accepted; CURRENT / VM+JIT via B5 S1+S2 — owner + owner-bound HYGIENIC member identity (`member:{owner}:{field}` hash, NEVER a source-name string, R1/R3) + value-type frozen identity (SUBSTITUTED for an applied generic before issuance, R10) + `FieldInitialization`; derive-style iteration over `record.fields` proven dual-engine (B5 S2). LSP CURRENT via B5 S4 — the `reflect_repr` hover renders the descriptor type `FieldDescriptor<Owner, #f, T>` with its owner nominal and the hygienic member token `#f`, catalog-driven (`tools/shape-test/tests/lsp/typed_comptime.rs::reflect_repr_hover_renders_descriptor_types_and_owner`). `#name` explicit selection is grammar-blocked (B7; the `#f` token is the descriptor TYPE's hygienic member position, not a spellable call form — see `docs/defections.md`) |
| `AssociatedConstDescriptor<Owner, C, T>` | comptime declaration | Typed associated constant, separate from fields | accepted; schema+model CURRENT via B5 S1 (registered + lift-walled); population depends on const generics (later slice) |
| `FieldInitialization<Owner, F, T>` | comptime | Required or typed-default construction policy | accepted algebra; CURRENT sealed enum (`Required`/`Defaulted`) via B5 S1 — the freeze struct input carries no per-field default flag yet, so all fields are `Required` today (Dec 59 total records; `Defaulted` population is a later slice, see `docs/defections.md`) |
| `DefaultInitializer<Owner, F, T, Effects>` | comptime code | Closed checked runtime default initializer | accepted |
| `AnnotationDescriptor<A, Target, Args, Multiplicity>` | comptime | Typed applied annotation and target proof | accepted |
| `CaptureDescriptor<Sig, I, T, Mode>` | comptime | Typed closure capture identity | accepted through Decision 95; compiler model + generated-only `FunctionExpr.captures` clause CURRENT on the C1 branch (supervisor gates pending) with four declared modes (`Move`, `Share`, `SharedBorrow`, `ExclusiveBorrow`), structural slot/lineage identity, exact `SemanticFreeze` capture types, and one declared plan driving layout/opcodes; borrow modes are named rejections until regions. Compiler-query hover/definition/references and descriptor-authoritative rename are committed; unavailable/conflicting evidence never falls through to name-based rename. Public `CheckedBody` construction remains TARGET C2 |
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
| `CheckedBody<Sig, Captures>` | comptime code | Callable body matching one signature and complete capture set | accepted through Decision 95; TARGET C2 staging populates the canonical `Expr::FunctionExpr.captures` carrier introduced by C1 — no parallel capture representation |
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
  Decision 68 is CURRENT on the existing extend/materialization path
  (ADR009-D1, 2026-07-13); Decision 66/67 — the declaration-discovery fixed
  point, the shared `generated_symbol_query()` surface, and `shape-expansion://`
  read-only virtual views — are CURRENT / compiler+LSP+VM+JIT (ADR009-D2,
  2026-07-13, evidence `tools/shape-test/tests/lsp/typed_comptime.rs` +
  `expansion_views.rs`). Decision 69 (name policies) remains TARGET.
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
