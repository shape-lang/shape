# Shape Language Context

Canonical language for Shape's compiler/runtime semantics and user-visible
execution model.

Maintainability note: this remains one cross-cutting glossary while strict
typing and comptime terminology are still being resolved; splitting before the
domain boundaries stabilize would create competing canonical definitions.

## Project Policy

**Architecture-First Change Policy**:
Shape is pre-production. Breaking source, bytecode, wire, and embedding changes
are acceptable when they produce a cleaner semantic architecture. Do not add
compatibility aliases, parallel legacy protocols, or migration shims unless an
explicit external constraint is documented.
_Avoid_: preserving accidental APIs, compatibility by default

**Strictly Typed Comptime**:
Comptime reflection and generation operate only on typed descriptors, hygienic
symbols, checked AST/item fragments, and compiler-validated transformations.
Generated artifacts re-enter the ordinary type/effect/ownership checker. Source
strings, parser round-trips, JSON AST payloads, dynamic `Any`, and name-based
runtime reconstruction are not comptime interfaces.
_Avoid_: string macro, untyped AST escape hatch, Zig-style text generation

**Domain-General Comptime Bar**:
Comptime is general typed staged programming, not a database, serialization,
remote, derive, or other domain feature. Domain frameworks are Shape libraries
built from universal reflection, generation, staging, effect/capability,
dependency, hygiene, and LSP contracts. The bar is semantic access and
generation beyond token-oriented Rust macros, with stricter typing, hygiene,
determinism, diagnostics, and editor support than text/name-oriented comptime
systems. A feature is incomplete if it works only for one showcase domain.
_Avoid_: SQL compiler special case, derive-only macro system, editor-blind metaprogramming

**Comptime Engine**:
The compile-time evaluator and typed expansion engine invoked by `comptime {}`
and `comptime fn`. It may be used in expression, declaration, signature, body,
module, and annotation-hook compilation contexts. Each context supplies a
typed capability and accepts a typed value, fragment, or atomic expansion
delta; there is no ambient compiler mutation. A comptime block never executes
during runtime invocation.
_Avoid_: runtime comptime callback, context-free emit, invocation-time macro

**Expansion Sink**:
The context-indexed typed capability supplied to one `comptime {}` block. Its
position determines the legal input descriptors and exact output category:
closed value/expression, type or signature patch, body/hook fragment, checked
item/module delta, or annotation contribution. Emitting another category is a
compile-time type error; no sink grants ambient compiler mutation.
_Avoid_: universal emit, module item from expression sink, implicit stage escape

**Comptime LSP Contract**:
The requirement that editor services consume the same stage graph, descriptors,
typed builders, expansion sinks, generated-symbol identities, diagnostics, and
source maps as compilation. Completion and signature help are sink-sensitive;
hover exposes stage and exact types; navigation, references, and rename include
generated declarations; virtual expansion views retain bidirectional source
provenance. Unsupported editor behavior is a language-completeness gap, not an
optional tooling enhancement.
_Avoid_: LSP reimplementation, generated symbols without navigation, generic suggestions

**Declaration Discovery Fixed Point**:
The deterministic monotonic compiler stage in which module/item comptime sinks
reserve and define generated declaration headers before ordinary bodies are
checked. Each expansion identity runs once per dependency hash; generated
annotations may add further declarations until no unseen headers remain.
Previously discovered headers cannot be changed or removed in this stage, every
reserved symbol must be defined exactly once, and conflicts, cycles,
oscillation, or unbounded generation are compile errors with full provenance.
Compiler and LSP consume the same incremental fixed-point query.
_Avoid_: late name patching, source-order-dependent generation, speculative duplicate pass

**Generated Symbol**:
An ordinary compiler `SymbolId` created by a typed expansion and participating
in the same name resolution, typing, references, rename, workspace-symbol,
completion, and artifact graph as a source declaration. It is never represented
only by generated text, a plain name, or a dummy span.
_Avoid_: synthetic AST-only declaration, text-indexed generated reference

**Expansion Provenance**:
The stable identity and bidirectional source mapping carried by every generated
node: generator identity, application identity, exact target, comptime stage,
typed argument hash, dependency hash, generated node path, and source anchor.
Diagnostics and navigation expose both generated and originating locations.
_Avoid_: `Span::DUMMY`, untraceable expansion error, lossy source attribution

**Virtual Expansion View**:
A deterministic read-only rendering of checked generated IR, exposed to tooling
through an expansion URI. It supports inspection, hover, navigation, semantic
tokens, and diagnostics but is never reparsed, edited, renamed in place, or used
as compiler input. Rename edits an identity-bearing generator input when one
exists, otherwise reports the symbol as generator-controlled and points to its
configuration.
_Avoid_: editable fake source, generated text as semantic authority

**Typed Name Policy**:
A deterministic versioned and hash-covered mapping from a branded external
identifier domain and target namespace to a hygienic generated symbol. Public
generated names originate either from an explicit source binder or such a
policy; raw strings cannot become symbols. Collisions and invalid/reserved names
are compile errors with LSP previews and alias-insertion actions, never silent
numeric suffixes. Private generated temporaries may use unspellable fresh
identities.
_Avoid_: `symbol(string)`, silent `Name2`, unversioned casing convention

**Comptime Effect**:
A stage-specific effect describing external interaction by the comptime engine.
Comptime is pure by default; filesystem/package resources, selected environment
configuration, processes/toolchains, network/providers, target inspection,
clock, randomness, and secrets require explicit typed effects and host grants.
Runtime effects do not authorize comptime effects and vice versa.
_Avoid_: ambient build IO, runtime capability reused at compile time

**Comptime Capability**:
A host-issued, scoped, sandboxed authority consumed by typed comptime provider
APIs. Every operation records provider identity/version, normalized request,
tool/target configuration, content digest, provenance, limits, and dependency
edges. Providers are library-defined; domains such as SQL, protobuf, bindgen,
or GPU tooling are not compiler branches.
_Avoid_: unrestricted host handle, untracked subprocess, domain compiler plugin

**Tracked Build Input**:
A typed external value paired with canonical identity, content hash, provider
descriptor, provenance/source mapping, and freshness or offline-lock evidence.
It participates in expansion identity, incremental invalidation, compiler/LSP
query caching, and release reproducibility. Live data without a reproducible
snapshot cannot enter a release artifact.
_Avoid_: mtime-only dependency, invisible environment read, stale LSP schema

**Comptime Secret Grant**:
Opaque authority usable only by an authorized provider operation. Secret bytes
never become ordinary comptime values, expansion output, diagnostics, virtual
documents, logs, dependency hashes, or executable artifacts. Resulting public
resource content may be hashed without revealing the credential.
_Avoid_: `env("TOKEN")`, secret interpolation, credential in cache key

**Typed Artifact Sink**:
A context-indexed comptime output capability for a declared artifact category:
checked Shape code, typed text or binary format, metadata schema, target-specific
link requirement, generated test set, compatibility report, or verified program
manifest. Outputs use logical identities; physical paths are host policy. Every
artifact carries format, target/ABI/schema, content hash, dependencies,
provenance, permissions, and source maps where applicable.
_Avoid_: `fs.write`, output path string, untyped build-script directive

**Atomic Artifact Set**:
A manifest-indexed collection of code and non-code comptime outputs committed
only when every member validates. Failure, cancellation, stale input, target
mismatch, or permission refusal publishes none of the set. Generated tests and
code enter ordinary compiler graphs as symbols; binaries and metadata remain
navigable artifacts in compiler/LSP queries.
_Avoid_: partially written codegen directory, host/target output confusion

**Annotation Mechanic**:
The independent target-attachment and hook-composition mechanism. An annotation
may be a marker, contribute compile-stage transforms, or define target-legal
runtime hook templates. Applying it causes the compiler to specialize and
lower those contributions; the annotation itself is not synonymous with
comptime, and runtime hook execution is not comptime execution.
_Avoid_: annotation equals macro, hook phase treated as comptime stage

**Hook Template**:
Ordinary typed runtime Shape code attached by an annotation to one legal target
join point and specialized during compilation. A nested `comptime {}` runs
while that template is specialized and emits checked runtime fragments; only
the emitted code remains for invocation. Templates cannot read runtime values
from comptime or defer compiler work until the hook runs.
_Avoid_: hook calling the comptime evaluator, runtime value captured by comptime

## Annotation Runtime

**ArgumentPack**:
A compiler-internal carrier parameterized by an annotated target function's
signature. It preserves parameter order, types, passing modes, ownership, and
authoritative kinds, and is not a homogeneous collection. It is not nameable,
inspectable, iterable, constructible, or serializable by ordinary Shape code.
Strictly typed comptime specialization generates ordinary wrapper parameters,
direct calls, and finite checked rewrites over it.
_Avoid_: args array, `Array<_>`, argument list

**HookDecision**:
A typed result from a specialized annotation before-hook. `Proceed` carries
same-layer state and selects the current arguments or one comptime-generated
`RewritePlan`; `Return` carries a target-compatible result and the same state
while explicitly skipping the target call. Effective arguments remain in the
compiler-internal call ledger; field presence in an object does not determine
annotation control flow.
_Avoid_: hook result object, magic `{args, result, state}` object

**HookTarget**:
A signature-indexed callable denoting the exact next-inner continuation in a
stacked annotation chain. It includes every inner annotation and eventually the
raw implementation; it is not a direct bypass to the unannotated body.
_Avoid_: raw implementation target, annotation-chain bypass

**Short-Circuit Unwind**:
When a before hook returns `HookDecision::Return`, its result skips every
deeper, unentered annotation layer and the body, then runs the returning
layer's `after` hook followed by already-entered outer `after` hooks. Skipped
layers run no hooks.
_Avoid_: bypassing outer after hooks, running hooks for unentered layers

**After Hook**:
A success-only result transformation. Suspension preserves the entered hook
chain for later resumption; `Failed`, `Cancelled`, and `Faulted` outcomes bypass
pending `after` hooks because no completion value exists. Failure transformation
belongs to `on_failure`; cleanup belongs to the evaluator-owned lifecycle scope.
_Avoid_: implicit finally hook, after hook as error handler

**Failure Hook**:
An explicit `on_failure` annotation phase that receives a structured
`RuntimeFailure` while unwinding a failed next-inner execution. It is distinct
from success-only `after`; it does not receive `EngineFault` and does not treat
`Cancellation` as failure. It may explicitly recover with a valid `R`, retry
the continuation, select a different remote placement, or propagate a
structured failure through a compiler-validated decision.
_Avoid_: overloading after, catching implementation faults

**Failure Recovery**:
The explicit conversion of `Failed(RuntimeFailure)` into `Completed(R)` by an
annotation failure hook. Recovery may obtain `R` directly, by retrying with a
new signature-compatible pack, or by choosing another remote host; it is not a
general source-level exception mechanism.
_Avoid_: implicit retry, untyped fallback value

**FailureDecision**:
The three-variant algebraic result of `on_failure`: `Propagate` carries a
structured failure outward, `Recover` supplies a signature-valid completion
value, and `Retry` submits a sealed `InvocationAttempt` using the current
argument state or a comptime-generated rewrite. Richer retry, fallback,
failover, and circuit policies are typed Shape stdlib abstractions over this
primitive, not additional compiler protocols.
_Avoid_: general evaluator access, compiler-only recovery DSL

**InvocationAttempt**:
A sealed, signature-indexed retry plan tied to the failed next-inner
continuation. It carries a validated internal argument state or checked rewrite,
placement, delay, budget, state, and duplicate-safety authority; user code
cannot invoke the continuation around those checks.
_Avoid_: direct retry call, retry-anyway boolean

## Typed Comptime Structures

**Comptime Value**:
An ordinary typed value evaluated during compilation. It is data, not syntax or
generated code, and has no implicit conversion to a checked fragment.
_Avoid_: treating string/bytes/object data as code

**ConstLift**:
The closed recursive capability permitting a comptime value to be embedded as a
runtime literal through an explicit typed builder such as `expr.literal(value)`.
Scalars and immutable closed aggregates may qualify; references, resources,
functions, provider capabilities, and arbitrary runtime handles do not.
_Avoid_: implicit quotation, lifting an affine capability

**TypeRef**:
A comptime-only opaque compiler-issued identity `TypeRef<T>` for one semantic
Shape type. It may originate only from type syntax, generic substitution, or
another typed compiler descriptor. Reflection proceeds through a typed
`FrozenType<T>`; there is no text-to-type constructor and no comparable
`.name`, `.source`, or display string. Diagnostic rendering passes the identity
directly to a diagnostic sink and cannot be used for semantic branching or
type reconstruction.
_Avoid_: `TypeRef.parse`, rendered-type equality, string-backed type identity

**TraitRef**:
A comptime-only opaque compiler-issued identity `TraitRef<Tr>` for one trait
contract, distinct from `TypeRef<T>` because a trait is not a value type. It
may originate only from trait syntax, generic substitution, or another typed
compiler descriptor; lookup never accepts a name or other text.
_Avoid_: treating a trait as a type, string-selected trait lookup

**ImplRef**:
Branch-scoped comptime evidence `ImplRef<T, Tr>` that the exact type `T`
implements the exact trait `Tr`. Trait lookup returns optional evidence rather
than a boolean, and checked builders requiring the implementation consume that
evidence directly. It cannot be forged, reconstructed by name, or used outside
the branch in which lookup proved it.
_Avoid_: boolean implementation test followed by unchecked trait generation

**FrozenType**:
The comptime-only sealed indexed sum `FrozenType<T>` returned by reflecting a
`TypeRef<T>`. Its exhaustive variants are `Primitive`, `Never`, `Parameter`,
`Nominal`, `Tuple`, `Record`, `Callable`, `Reference`, `Union`, and `Erased`;
each carries a category-specific descriptor still indexed by `T`.
`FrozenPrimitive` is separately sealed over unit, bool, char, signed and
unsigned integer families, binary floating-point families, exact decimal,
string, null, and undefined. Applied builtins and user types are uniformly
nominal. Structural object intersections normalize to records, trait
intersections normalize into erased bounds, and aliases normalize away. It has
no string kind tag, syntax-preserving intersection, `Any` payload, nullable
category fields, or open record shape. Explicit `any`/`dyn` domains are erased;
internal `Any`, unknowns, dynamic-schema fallbacks, and inference variables
cannot freeze. A new semantic category changes the comptime ABI and must be
handled or explicitly rejected by consumers.
_Avoid_: universal type-info object, optional fields selected by a kind string

**Existential Descriptor Package**:
An ordinary typed package `exists<W...> Descriptor<W...>` that hides a
descriptor's heterogeneous witnesses without erasing them. Opening the package
introduces fresh lexical witness types or identities; they cannot escape that
scope unless explicitly repackaged into another existential value. Comptime
loop syntax over descriptor collections is sugar for a rank-2 generic callback,
not a dynamic compiler iterator.
_Avoid_: `Any` element collection, unscoped hidden witness, reflection-only magic

**Semantic Freeze Boundary**:
The all-or-nothing compiler boundary after type inference at which `TypeRef`,
`FrozenType`, `FrozenCallable`, and annotation specialization capabilities may
be issued. Declared generic parameters are resolved semantic identities and
receive typed parameter descriptors. Fresh inference variables, internal
`Any`, unknown storage kinds, and dynamic schema fallbacks fail the boundary
before user comptime code runs; no partial descriptor is observable.
_Avoid_: hook over an unresolved signature, `FrozenType::Unknown`, late field failure

**TypeParamDescriptor**:
A comptime-only compiler-issued descriptor for one declared generic type
parameter, retaining its stable identity, bounds, defaults, variance, and
ownership constraints without pretending that its later substitution is
known. It is the payload of the parameter category in `FrozenType<T>`.
_Avoid_: inference-variable descriptor, generic parameter selected by name

**Transparent Type Alias**:
A declaration synonym that introduces no semantic type identity. `TypeRef` and
`FrozenType` normalize it to its canonical underlying type. Documentation,
generic parameters, and declaration provenance remain available through a
separate typed `AliasDescriptor`; code requiring distinct identity must use an
explicit nominal declaration.
_Avoid_: `FrozenType::Alias`, alias that becomes nominal through metadata

**AliasDescriptor**:
A comptime-only declaration descriptor tying one hygienic alias-declaration
identity to its canonical underlying `TypeRef<T>` and declaration metadata. It
is obtained from declaration reflection, not type reflection, and never makes
the alias a distinct semantic type.
_Avoid_: recovering alias identity from `TypeRef<T>`, alias selected by name

**TypeConstructorRef**:
A comptime-only opaque compiler-issued identity for one nominal type
constructor together with its ordered type/const parameter kinds. Applying it
checks arity, argument kinds, constraints, and ownership before producing a
canonical `TypeRef<Applied>`; construction and comparison never use names.
_Avoid_: generic constructor string, untyped argument array

**AppliedType**:
A comptime-only typed descriptor tying an exact applied nominal type to its
`TypeConstructorRef` and ordered typed type/const arguments. Zero-argument
nominals use the same representation. `Option`, `Result`, arrays, collections,
user generics, and explicit nominal wrappers do not receive bespoke reflection
variants; runtime layout specialization remains compiler-internal.
_Avoid_: `FrozenType::Option`, argument recovery by position from `Any`

**NominalShape**:
The sealed comptime semantic representation algebra for an applied nominal
type: `Struct`, `Enum`, `Newtype`, or `Opaque`. Every payload remains indexed
by the exact applied owner type after generic substitution. Optional field,
variant, underlying-type, builtin, native-kind, and physical-layout metadata
are not part of the algebra; adding a shape changes the comptime ABI.
_Avoid_: nominal kind string, nullable shape members, runtime layout reflection

**RepresentationAccess**:
A compiler-issued, comptime-only, non-serializable capability authorizing
complete semantic representation reflection for one exact nominal type. It is
available under explicit type-author authority, such as a declaration-attached
transform or defining-module operation. Without it, reflection exposes identity
and public interface but no partial field/variant view. The capability does not
itself grant generated runtime code private-field access outside an authorized
installation scope.
_Avoid_: filtered public field list, privacy represented as `Opaque`, leaked access

**FieldDescriptor**:
A comptime-only compiler-issued descriptor `FieldDescriptor<Owner, FieldId,
Value>` for one named record field. `FieldId` is an opaque hygienic member
identity resolved in the owner's member scope, never a name or ordinal. Checked
read/write builders consume the descriptor to prove exact owner and value type;
source order controls deterministic iteration only. External codec labels are
explicit typed data and are not field identity.
_Avoid_: `field("name")`, positional named-field selection, runtime name lookup

**AssociatedConstDescriptor**:
A comptime-only compiler-issued descriptor
`AssociatedConstDescriptor<Owner, ConstId, T>` for a type-associated constant.
It carries a typed `ConstValue<T>` and belongs to the declaration interface,
not runtime representation fields. Configurable type-level values are const
generic parameters; zero-slot "comptime fields" and alias field-override syntax
do not exist.
_Avoid_: `StructField.is_comptime`, zero-slot field, alias metadata override

**Total Record**:
A runtime record in which every declared field is always present. Semantic
absence is represented by the field's explicit `Option<T>` type. Construction
may omit only fields with typed defaults, but the resulting value has no
optional-presence bitmap or missing-field access path.
_Avoid_: `field?: T`, `field.optional`, missing-field runtime error

**FieldInitialization**:
The sealed comptime construction policy for one total record field: `Required`
or `Defaulted(DefaultInitializer<Owner, FieldId, T, Effects>)`. It does not
alter the stored field type and is not a nullable property on the descriptor.
_Avoid_: optional default expression, default changing read type

**DefaultInitializer**:
A closed checked runtime thunk
`DefaultInitializer<Owner, FieldId, T, Effects>` used only when construction
omits that total field. It may use substituted const parameters and associated
constants but cannot observe partial `self` or sibling fields. Its full effect,
ownership, failure, suspension, and cleanup requirements flow into the
synthesized constructor, and omitted defaults run in declaration order.
_Avoid_: self-dependent default, untyped default AST, hidden constructor effect

**AnnotationDescriptor**:
A comptime-only compiler-issued descriptor for one applied annotation, indexed
by exact annotation identity, permitted target identity, typed argument pack,
and declared multiplicity. Arguments are comptime expressions selected through
hygienic parameter identities. Wrong targets and duplicate single-use
applications fail before hooks run; runtime hook state is not annotation
metadata.
_Avoid_: annotation name string, `Array<Any>` arguments, magic annotation object

**Annotation Target Contract**:
The complete finite set of typed target/phase handler clauses implemented by an
annotation. This set is the annotation's support declaration; there is no
parallel target list, universal `ComptimeTarget`, implicit default handler, or
silent no-op. An intentionally unchanged path returns typed `NoChange`, and an
application with no matching clause fails compilation.
_Avoid_: `targets: [...]` registry drift, target-kind branch, claimed support gap

**Exact Annotation Target**:
One concrete compiler-issued target descriptor accepted directly by a typed
annotation clause. Declaration targets include modules, nominals, traits,
aliases, callables, and their exact member descriptors. Expression targets are
typed value, block, or await descriptors. There is no umbrella target value;
the parser-only binding target is absent until a unique complete contract is
designed. Adding a target is a comptime-ABI change requiring its descriptor,
transform algebra, diagnostics, and execution proofs together.
_Avoid_: `AnnotationTarget`, target-kind enum dispatch, placeholder target

**FrozenCallable**:
A comptime-only sealed descriptor for one fully inferred callable signature,
including ordered parameters, stable identities, types, passing modes, kinds,
return type, effects, ownership constraints, and exact next-inner continuation.
_Avoid_: provisional signature as final descriptor, rendered type string

**ParamDescriptor**:
A comptime-only typed member of a `FrozenCallable`, indexed by stable parameter
position and carrying its exact type and passing mode. Position is semantic
because it defines positional calling and ABI order. A source name or named
argument resolves hygienically to the same position during compilation and is
not retained as a lookup string. Rewrites consume the descriptor directly.
_Avoid_: runtime parameter lookup, string-selected slot, homogeneous args array

**HygienicSymbol**:
A compiler-issued typed binding identity usable by checked fragments without
capturing or colliding through textual spelling.
_Avoid_: generated identifier string, caller-local accidental capture

**CaptureDescriptor**:
A comptime-only compiler-issued identity
`CaptureDescriptor<Sig, I, T, Mode>` for one runtime binding captured by one
exact generated callable owner. `Mode` is move, shared borrow, or exclusive
borrow. Descriptors are resolved from typed scope structure, never names, and
form a heterogeneous complete capture pack checked with the body.
_Avoid_: capture by string, homogeneous environment, inferred ambient generated capture

**Complete Capture Environment**:
The heterogeneous capture pack indexing `CheckedBody<Sig, Captures>` and
`CheckedTemplate<Sig, Captures>`. Generated closures declare every member and
mode explicitly; edits begin with the existing complete pack and change the
body, environment layout, ownership/drop plan, and references atomically.
Comptime values are not captures and enter runtime code only through
`ConstLift`. Hooks obtain runtime values only through exact typed hook inputs.
_Avoid_: partial environment update, implicit stage crossing, hook ambient capture

**Native Contextual Fragment**:
A checked generated fragment written with ordinary Shape grammar inside a
typed expansion sink. The sink or expected checked-fragment type determines
the expression, pattern, statement, body, item, or module role; a standalone
ambiguous value uses the corresponding explicit role block. Static structure
uses normal syntax, computed structure uses typed builders and name policies,
and edits use semantic cursors. Compatible fragments and descriptors are
inserted by type, while ordinary comptime data requires explicit `ConstLift`.
_Avoid_: quotation sublanguage, splice sigil, token tree, implicit capture

**CheckedExpr**:
A comptime-only expression fragment carrying its value type, effect row,
ownership/borrow facts, dependencies, and hygienic references. Native
contextual expression blocks and typed builders are its constructors, and
installed expressions re-enter whole-context checking. It is distinct from an
ordinary comptime value; a `ConstLift` value enters code only through explicit
literal construction.
_Avoid_: source expression string, unchecked AST node

**CheckedPattern**:
A comptime-only pattern fragment carrying its scrutinee type, hygienic binding
environment, sealed structural coverage, and ownership footprint. Native
pattern sinks and typed builders produce the same semantic pattern algebra.
_Avoid_: pattern source string, spelling-selected binder, unchecked match arm

**IrrefutablePattern**:
A compiler-proven `CheckedPattern` refinement whose structural coverage is all
values of its scrutinee type. Unconditional declarations, parameters,
assignments, and loop bindings accept only this proof; refutable mismatch is
never an implicit runtime exception.
_Avoid_: unchecked destructuring, runtime `Pattern match failed`

**CheckedArm**:
A checked pattern, optional guard, and result body with one lexical binding
scope. Guards do not establish structural exhaustiveness unless proven
constantly true. Binderful generation prefers whole arms so ordinary native
binding syntax remains the common surface.
_Avoid_: detached name-indexed binding environment, guarded coverage claim

**MatchPlan**:
A finite atomic comptime builder for generated match arms. It publishes a
checked expression only after result, effect, ownership, borrow, reachability,
and exhaustive-coverage validation; unknown coverage is a compile error.
_Avoid_: partial arm installation, exhaustiveness `NotApplicable`

**Diverging Let-Else**:
Statement-form `let PATTERN = VALUE else { NEVER }`, the only non-`match`
surface for a refutable binding. Matching probes without consuming and then
atomically commits every success binding; mismatch commits none. The `else`
block must have type `never`, and both edges run complete synchronous or
asynchronous drop obligations exactly once.
_Avoid_: completing else branch, partial binding, hidden mismatch panic

**Pattern Binder Mode**:
The stable ownership disposition of one pattern projection. A bare binder
copies a `Copy` value or moves another owned value, while binders projected
from `&T` or `&mut T` remain shared or exclusive borrows. Explicit `move`,
`&`, and `&mut` override the default. Body use cannot change the disposition;
NLL infers only borrow duration.
_Avoid_: body-inferred move versus borrow, clone pattern, display-only mode

**Guard View**:
A compiler-issued read-only view of a matched projection between structural
probe and final arm commit. Every final binder mode appears shared in a guard;
`true` ends the views and atomically commits final bindings, while `false` ends
them and commits nothing. Guard effects are tracked and never rolled back.
_Avoid_: early move, mutable guard binder, rollback after guard

**Closed Pattern Algebra**:
The compiler-sealed structural operations from which every source or generated
`CheckedPattern` is built. Libraries may compose these operations in typed
comptime functions, but runtime parsing, extraction, and classification return
ordinary closed values for a later match and never become probe callbacks.
_Avoid_: active pattern, extractor protocol, user-asserted coverage

**Pattern Synonym**:
A typed comptime function that composes sealed pattern constructors and returns
a `CheckedPattern`. The compiler derives coverage, ownership, effects, hashes,
and provenance from the returned pattern; the function cannot assert them.
_Avoid_: runtime matcher function, source template, trusted purity claim

**FrozenPattern**:
The exhaustive, type- and binding-indexed comptime view of a `CheckedPattern`.
It exposes only the compiler's sealed semantic variants and typed children;
the accepted variant catalog changes with the comptime ABI.
_Avoid_: parser AST, untyped node enum, non-exhaustive reflection

**Pattern Constructor**:
One sealed structural variant of `FrozenPattern`, with complete typed children,
coverage, probe, ownership, hashing, backend, diagnostics, and LSP semantics.
Native syntax and typed builders create the same constructor; new constructors
change the language and comptime ABI.
_Avoid_: kind string, opaque pattern payload, partially implemented variant

**Constant Pattern**:
A compiler-canonical scalar equality singleton, or source sugar recursively
normalized into visible structural pattern constructors. It requires an exact
scrutinee type, never calls user equality, uses `const PATH` for named values,
and rejects NaN plus values carrying identity, authority, lifetime, or cleanup.
_Avoid_: generic equality pattern, numeric coercion, opaque structural const

**Range Pattern Domain**:
A compiler-sealed exact ordering whose intervals may participate in structural
pattern coverage. The initial domains are exact integers, Unicode-scalar
`char`, and canonical `decimal`; aliases normalize transparently, while nominal
wrappers remain explicit and user comparison traits never create a domain.
_Avoid_: user-ordered pattern, declaration-order enum range, runtime range test

**Range Bound**:
One side of a range pattern, semantically `Unbounded`, `Included`, or
`Excluded` over an exact `Range Pattern Domain`. Inclusion is represented
directly rather than by successor arithmetic; source and typed builders can
express every lower/upper combination, while two unbounded sides are `_`.
_Avoid_: endpoint increment, builder-only interval, fully unbounded range node

**Range Endpoint**:
An exact `ConstValue<T>` forming an included or excluded `Range Bound`. Source
creates one through a context-typed literal, explicit `const PATH`, or typed
`comptime {}` endpoint sink; ordinary runtime expressions and guessed names do
not cross this stage boundary.
_Avoid_: runtime endpoint, implicit const lookup, endpoint coercion

**Pattern Denotation**:
The canonical compiler-derived `CoverageSet<T>` of values matched by one
checked pattern, distinct from its reflectable constructor structure. It owns
exhaustiveness and reachability proofs plus reusable backend planning; users
may inspect typed results but cannot construct or assert coverage evidence.
_Avoid_: destructive pattern normalization, user-claimed coverage, source hash

**Decimal**:
The canonical arbitrary-precision exact number whose value has a finite
base-10 expansion. Trailing zeros and signed/scaled zero are representation
only; the semantic domain is unbounded and dense, while fixed precision,
scale, and rounding policy belong to explicit nominal library types.
_Avoid_: host decimal carrier, implicit rounding, built-in SQL decimal

**Fixed Decimal**:
A nominal typed-library or comptime-generated constraint over `Decimal` with
explicit precision, scale, construction, conversion, rounding, and adapter
policies. It does not transparently inherit decimal representation or matching.
_Avoid_: hidden decimal metadata, unchecked narrowing, core schema special case

**Sequence Pattern Domain**:
A compiler-issued structural capability for one exact homogeneous ordered
container and element type. Dynamic arrays, fixed arrays, and their first-class
segments share one sealed `Sequence` algebra; iterators and user indexing do
not create pattern eligibility.
_Avoid_: iterable pattern, callback-backed indexing, erased list pattern

**Sequence Rest**:
The optional single contiguous segment between a sequence pattern's ordered
prefix and suffix. It may be ignored or bound with a statically known segment
type and ownership mode; it never filters, iterates, or defers matching.
_Avoid_: multiple rest, lazy tail, backend-selected rest representation

**Slice**:
A first-class non-owning contiguous homogeneous sequence view. Shared or
exclusive references to a slice carry ordinary region and mutability proofs;
an owned dynamic sequence segment remains `Array<T>` rather than an owning
slice with hidden copying semantics.
_Avoid_: owning slice alias, iterator tail, copied rest

**Sequence Split**:
The atomic ownership partition committed after a sequence pattern and guard
succeed. It transfers disjoint elements and an optional contiguous rest while
retaining exact drop obligations and one backing-storage lifetime, without
allocating or giving any element multiple owners.
_Avoid_: eager tail materialization, early split, aliased element owner

**Symbolic Sequence Language**:
The canonical regular-language denotation of one homogeneous sequence pattern,
with transitions labeled by exact compiler-derived element coverage. It owns
length/content exhaustiveness, residual, reachability, witness, and denotation-
hash proofs without executing an iterator or runtime predicate.
_Avoid_: approximate slice coverage, runtime automaton contract, user matcher

**Coverage Budget**:
Deterministic compiler fuel for exact coverage construction, set operations,
and witness proofs. Exhaustion is a compile-time inability to prove required
semantics, never permission to assume exhaustive, emit fallback matching, or
accept unknown reachability.
_Avoid_: assume-exhaustive flag, runtime coverage check, silent approximation

**Whole-Place Alias**:
A hygienic pattern identity for the existing matched root place. It exposes the
root's inherited or narrowed capability and post-commit move/loan state without
creating another value, owner, copy, clone, or drop obligation.
_Avoid_: duplicate whole binding, hidden clone, detached alias value

**Reified Runtime Type**:
A complete semantic type with one compiler-issued portable execution identity
and checked erased-value carrier. Primitive, nominal, and fully instantiated
core types may qualify; carrier kinds, rendered names, open traits, structural
shapes, and unresolved generics do not establish reification.
_Avoid_: native-kind type test, vtable identity, erased generic arguments

**Exact Dynamic Type Pattern**:
A sealed refinement from an open erased domain to one `Reified Runtime Type`
whose static membership is already proven. It compares exact execution
identity without invoking conformance code and never makes a visible
implementor list exhaustive; a true catch-all covers the open remainder.
_Avoid_: trait pattern, structural downcast, known-implementors exhaustiveness

**Pattern Constraint**:
A binder-free checked pattern with compiler-proven empty ownership, loan, drop,
and commit footprint. It can be complemented or combined with one binding
pattern without creating binder unification or rollback semantics.
_Avoid_: name-free move pattern, user-asserted purity, hidden guard

**Boolean Pattern Algebra**:
The sealed `Not`, `AllOf`, and `Or` coverage operations. Negation accepts only
a `Pattern Constraint`; conjunction permits at most one binding producer; union
requires identical binding interfaces, so every successful commit remains one
ordinary checked ownership plan.
_Avoid_: general binder conjunction, predicate pattern, ownership rollback

**Residual Coverage**:
The exact denotation of one ordered match arm after subtracting prior arms that
contribute coverage. An empty residual is unreachable; a nonempty partial
residual remains valid and does not implicitly change binder types.
_Avoid_: approximate reachability, implicit refinement type, dead-arm allowance

**Pattern Cursor**:
An ephemeral compiler capability locating one typed node within one immutable
pattern root and revision. It cannot address another root, survive publication
of an edit, serialize, or contribute its address to an artifact hash.
_Avoid_: node index, source offset identity, cross-root cursor

**Scope-Owned Pattern Rewrite**:
An atomic rewrite whose owner matches the affected lexical contract. Exact-
binding subtree edits may be pattern-owned; binder-interface changes are
arm-owned; installed coverage and ordering changes are match-owned. No owner
publishes until all dependent scopes and proofs recheck together.
_Avoid_: binder remap by name, detached binding change, partial publication

**CheckedItem**:
A comptime-only declaration/item fragment carrying its declared callable or
type shape, effects, dependencies, and hygienic symbols. Installation uses the
ordinary registration and complete type/effect/ownership pipeline.
_Avoid_: parser-backed generated item, JSON AST item

**RewritePlan**:
A finite comptime-generated same-signature argument transformation. Every
replacement is selected through a typed parameter descriptor and checked for
type, passing mode, ownership, effects, and evaluation order before runtime.
Runtime policy may choose among emitted plans but cannot construct or inspect
one dynamically.
_Avoid_: runtime field mutation, string-selected rewrite

**Replay Authority**:
A sealed capability consumed by every retry. `NotExecutedProof<Sig>` is minted
only from evaluator evidence that user code did not run. An uncertain or
started attempt requires scoped `ReplayEvidence<Sig, Scope>` backed by an
explicit idempotency contract, provider deduplication lease, or future purity
proof. Evidence must cover every selected effect domain and argument change.
_Avoid_: idempotent boolean, call ID as retry authority

**Recovery Budget**:
The mandatory maximum-attempt and absolute-deadline bounds for one recovery
episode. Discovery, connection, backoff, execution, and reply waiting consume
the same parent-owned budget; hooks and providers may narrow but never extend
it.
_Avoid_: unbounded retry, per-provider deadline reset

**Retry Re-entry**:
`FailureDecision::Retry` authorizes exactly one new attempt. If that attempt
fails, the evaluator invokes the same layer's `on_failure` again with the
effective pack, updated hook state and attempt history, remaining recovery
budget, and evidence for the new failure.
_Avoid_: user-managed evaluator loop, multi-attempt Retry variant

**Hook State**:
Invocation-local state private to one annotation layer. It flows from that
layer's before hook to its later phases and is retained by a suspended
continuation. It is not implicitly shared across layers, calls, or remote
boundaries. Persistent annotation state is a separate store abstraction.
_Avoid_: global annotation context, implicit persistent hook state

**Total Hook Scope**:
The compiler-completed lifecycle for one entered annotation layer. It owns one
affine hook-state identity, synthesizes omitted identity/propagation/no-op
phases, transfers ownership through suspension and retry, and consumes the
state exactly once through the evaluator's shared VM/JIT unwind ledger. Hook
state may use synchronous `DropSafe` destruction or effect-visible `AsyncDrop`.
_Avoid_: optional cleanup path, partially implemented annotation lifecycle

**DropSafe**:
The synchronous destruction class: non-suspending, non-failing, non-reentrant
structural cleanup that the evaluator may invoke during contained terminal
unwind. Resources requiring awaited cleanup use `AsyncDrop` instead.
_Avoid_: async destructor, fallible lifecycle-state cleanup

**Terminal Observer**:
An optional outcome-neutral `observe_terminal(summary)` phase that runs after
cleanup on healthy contained terminal paths. It cannot access hook state,
arguments, results, credentials, or continuations, and cannot transform an
outcome, retry, suspend, or own cleanup obligations.
_Avoid_: user-facing finally, observer as resource cleanup

## Resource Lifecycle

**AsyncCleanup**:
A callable effect indicating that an owned cleanup obligation may suspend while
the evaluator settles a scope. A synchronous context cannot own an unresolved
`AsyncDrop` value and cannot block secretly to discharge one.
_Avoid_: invisible async destructor, block-on-drop

**AsyncDrop**:
An automatic affine destruction protocol executed by the total evaluator
unwind ledger on every contained completion, runtime failure, and cooperative
cancellation. Moving the value moves its one registered obligation; explicit
`await close(value)` consumes and disarms the same obligation early. Expected
incomplete close outcomes become typed cleanup evidence and do not replace the
primary completion, failure, or cancellation.
_Avoid_: optional async close, dropping a cleanup future

**Async Retirement**:
The mandatory first phase of `AsyncDrop`: synchronously consume source
ownership, revoke normal access, and install evaluator-owned retired state plus
an emergency guard. Retirement cannot suspend, fail, or re-enter Shape.
_Avoid_: suspending while the public owner remains usable

**Async Close**:
The bounded awaited phase of `AsyncDrop`. It attempts graceful cleanup under a
cancellation shield and returns host-validatable completion or incomplete
evidence. It may suspend, but cannot extend its inherited absolute deadline or
detach cleanup work.
_Avoid_: unbounded cleanup shield, close acknowledgement by convention

**Async Release**:
The mandatory final synchronous phase that consumes retired state and releases
local mechanics after either graceful close or explicit abandonment. It cannot
claim peer cleanup, rollback, or remote non-execution.
_Avoid_: local release as proof of external cleanup

**Cleanup Evidence**:
The typed terminal record produced for every automatic cleanup obligation:
truthful `Closed` evidence or `Abandoned` evidence describing timeout,
rejection, uncertainty, cancellation, or emergency release. Expected cleanup
evidence is not `RuntimeFailure` or `EngineFault`.
_Avoid_: close failure as panic, boolean closed flag

**MustSettle**:
An affine typestate wrapper `MustSettle<T, Goal>` for resources whose successful
close, commit, abort, acknowledgement, or protocol resolution is required for
normal program correctness. Its identity follows moves through fields,
aggregates, closures, tasks, annotation state, and returns. Every normal
control-flow edge, including `?` propagation, must settle, transfer, or return
the owner; automatic `AsyncDrop` remains the fallback for runtime failure and
cancellation.
_Avoid_: must-settle lint, best-effort transaction drop

**SettleOutcome**:
A typed must-handle result of explicitly consuming a `MustSettle` owner.
`Settled(Proof)` carries sealed evidence satisfying its goal; `Incomplete`
carries typed reason/evidence. When external ownership or outcome remains
unresolved, `Incomplete` must carry a new affine recovery obligation.
_Avoid_: settlement exception, discardable close result

**Recovery Obligation**:
A `MustSettle<RecoveryHandle, ResolutionGoal>` created when settlement leaves
external state unresolved. It must be resolved, returned, or transferred to a
durable supervisor; merely inspecting or logging its evidence does not consume
the obligation.
_Avoid_: outcome-unknown as handled error, detached recovery task

**Obligation Transfer**:
The affine handoff of a settlement or recovery obligation to a supervisor. The
caller remains the owner until a typed acceptance receipt binds the obligation
identity, settlement goal, provider/effect domain, durability scope, and new
owner. Spawning work or emitting telemetry is not transfer.
_Avoid_: fire-and-forget recovery, best-effort queue as ownership handoff

**Cleanup Invariant Fault**:
An `EngineFault` reserved for impossible lifecycle states such as lost or
duplicate owners, skipped ledger entries, forged evidence, corrupt plans, or a
failure in supposedly total retirement/release. Expected cleanup incompleteness
never uses this channel.
_Avoid_: operational timeout as invariant fault

**Cancellation Settlement**:
The ordered transition `Requested -> Observed -> Terminated -> Joined` for
cooperative work. Resource cleanup begins only after tracked borrowers are
joined. Deadline expiry yields explicit non-termination or abandonment evidence,
not a successful-cancellation claim.
_Avoid_: cancellation requested means stopped, abort handle means joined

**Abandonment**:
A typed terminal cleanup record stating that local ownership was retired while
graceful or external cleanup remained incomplete or unconfirmed. Abandonment is
observable evidence, never an alias for closed, cancelled, rolled back, or
settled.
_Avoid_: best-effort close reported as success

**Cleanup Snapshot Barrier**:
A refusal to capture state containing a live cleanup or settlement obligation,
retired resource, active close, borrower, or recovery owner unless a versioned
provider-neutral contract proves durable identity, idempotent replay, and one
restorable owner. Live sessions, credentials, routes, tokens, and provider
handles are never serialized.
_Avoid_: snapshotting a socket, defaulting cleanup state on restore


**Transparent Placement**:
An execution-placement policy that preserves a function's declared parameter
and return types. `@remote` returns the target's declared `R`; transport,
protocol, permission, and receiver failures leave through the VM's non-returning
runtime failure channel rather than becoming an implicit `Result` value.
Recoverable remote failure is requested explicitly with `remote::call` or
an ordinary task spawned from it.
_Avoid_: implicit remote Result, typed remote exception

## Evaluation

**Evaluation**:
The canonical VM, JIT, and host outcome for one invocation:
`Completed(R)`, `Suspended(Suspension)`, `Failed(RuntimeFailure)`,
`Cancelled(Cancellation)`, or `Faulted(EngineFault)`. `Evaluation<R>` is an
evaluator/host type, not a Shape source type; a function's declared `R`
constrains only `Completed(R)`.
_Avoid_: treating every execution outcome as `R`, error result

**Completion Value**:
The `R` carried by `Evaluation::Completed`. When `R` is a Shape `Result<T, E>`,
both `Ok` and `Err` are ordinary completion values.
_Avoid_: promoting a Shape `Result::Err` to evaluator failure

**Runtime Failure**:
A structured, non-returning failure encountered while executing valid Shape
code. Its discriminant, stable code, source context, origin, and typed details
remain authoritative across VM, JIT, FFI, async, and remote boundaries. It is
not a Shape value and is not catchable by implication.
_Avoid_: runtime error string, typed exception

**Engine Fault**:
A structured evaluator outcome for an implementation invariant violation or a
contained panic in the VM, JIT, marshal layer, or extension adapter. An engine
fault is not presented as a user-program failure.
_Avoid_: converting an internal fault into a runtime failure

**Host Projection**:
The final mapping from `Evaluation<R>` to an external interface such as CLI
diagnostics and exit status, a server response, or an embedding API. Formatting
belongs here; internal adapters do not parse or reconstruct semantics from
strings.
_Avoid_: authoritative error side channel, semantic string parsing

## Distributed Execution

**Remote Failure**:
A failed remote-call outcome with two independent dimensions: a failure cause
and execution certainty. Cause explains what went wrong; certainty records
whether receiver user code definitely did not execute, may have executed, or
is known to have started. Retry policy is based on certainty, not inferred from
broad cause labels.
_Avoid_: treating transport cause as proof of non-execution

**Execution Certainty**:
`DefinitelyNotExecuted`, `OutcomeUnknown`, or `ExecutionStarted` for a failed
remote attempt. Only `DefinitelyNotExecuted` is generally safe to retry without
an idempotency policy or receiver deduplication.
_Avoid_: retry-safe timeout, cancelled means not executed

**RemoteError**:
The recoverable Shape value returned in the `Err` arm of an explicit remote
call. It is a record containing an orthogonal `RemoteErrorCause` and
`ExecutionCertainty`; it is not a flat enum whose variant name implies retry
safety.
_Avoid_: implicit certainty, cause-times-certainty variant matrix

**RemoteErrorCause**:
A typed enum describing why a remote call failed, including transport,
protocol, permission, receiver, resource, and timeout causes. Cause payloads
remain typed and do not encode execution certainty.
_Avoid_: unstructured remote message, retry policy from cause alone

**Remote Dispatch**:
The single internal execution pipeline shared by `@remote` and the async
`remote::call`. It owns permission checks, serialization, content-addressed
transfer, delivery classification, deadlines, cancellation, receiver execution,
and response validation, and produces a structured `RemoteOutcome<R>`.
_Avoid_: separate raising and Result dispatch implementations

**Remote Call Surface**:
The ordinary variadic typed syntax of the single async recoverable primitive
`remote::call`. The compiler validates arguments against the target signature
and lowers them to `ArgumentPack<Sig>`; `await remote::call(...)` produces
`Result<R, RemoteError>`, while ordinary `spawn`, scopes, races, and joins supply
concurrency and cancellation.
_Avoid_: user-built `_0` records, mandatory ArgumentPack at ordinary call sites

**Remote Projection**:
A thin boundary mapping from `RemoteOutcome<R>` to a public surface. The
transparent projection completes with `R` or produces evaluator
`Failed(RuntimeFailure::Remote)`; the recoverable projection completes with
`Result<R, RemoteError>`.
_Avoid_: duplicating dispatch semantics in each public API

**Remoting Provider**:
A nominal execution-provider capability that owns discovery, routing,
destination encoding, transport, authentication, codec, negotiation, pooling,
and provider telemetry through coherent provider-owned sessions. Providers are
implemented as privileged Shape modules; the host exposes only granted
primitives for I/O, cryptography, clocks, credentials, and task control. Shape
retains authority over typed invocation semantics, permissions, deadlines,
cancellation, validation, and execution certainty.
_Avoid_: global transport singleton, public component stack, native provider plugin ABI

**RemoteDestination**:
An immutable opaque logical destination branded by its `RemotingProvider`.
Provider modules construct it from typed configuration; it exposes no generic
host, port, URI, socket, credential, or byte representation.
_Avoid_: remote address string, provider cast

**Placement**:
An opaque provider-branded authority derived from a `RemoteDestination` plus
host-neutral placement preferences. It may represent a service, pool, queue,
actor, region policy, or physical peer without exposing provider mechanics.
_Avoid_: hostname as placement, live route as source value

**Placement Suitability**:
The proof that a selected placement can satisfy a target's complete effect,
permission, extension/provider, ABI, and execution-capability requirements.
The compiler proves known requirements; dynamic peers provide authenticated
pre-submission admission. Refusal is `DefinitelyNotExecuted`.
_Avoid_: execute-then-discover capability mismatch, provider-asserted compatibility

**Provider Session**:
The provider-owned async exchange boundary used by `Remote Dispatch`. Sessions
may implement arbitrary remoting mechanics, but consume a host-owned canonical
invocation envelope and return observations that the host validates and uses
to derive outcomes and certainty.
_Avoid_: provider-authored `R`, provider-authored execution certainty

**Provider Grant**:
A host-issued capability exposing one validated remoting-provider configuration
generation and its permitted destination/network scope to Shape code. The host
loads the privileged Shape provider module, binds credentials and secrets, and
grants authority; application Shape programs cannot load providers, inject raw
configuration, or obtain secrets as values.
_Avoid_: source-loaded transport, global provider config, secret-bearing destination

**Shape Execution Peer**:
A remote destination that runs a Shape runtime implementing the negotiated
Shape execution ABI. Providers may customize every remoting mechanism, but they
do not replace the peer runtime or reinterpret Shape values outside that ABI.
_Avoid_: arbitrary non-Shape RPC target as remote execution peer

## Executable Artifacts

**Execution Binding**:
The hash-covered compatibility requirement carried by every executable
artifact: canonical artifact format, readable ABI epoch, exact semantic
execution-ABI ID, and sorted required-capability set. Session negotiation never
replaces independent artifact verification.
_Avoid_: Shape version as execution ABI, handshake-only compatibility

**Execution ABI ID**:
The authoritative exact semantic fingerprint for portable Shape execution.
Peers and every artifact in one linked transitive closure must use the same ID;
there is no semver-range inference, best-effort translation, or silent
downgrade. It is generated from canonical descriptors for opcodes, kinds,
frames, calls, values, schemas, annotations, effects, cleanup, artifacts,
hashing, and linking, plus reviewed markers for behavioral semantics not
captured structurally. Compiler git state and host Rust layout are excluded.
_Avoid_: integer ABI as compatibility authority, Rust-layout fingerprint

**ArtifactKey**:
The complete cache and negotiation identity
`{object_class, artifact_format, execution_abi_id, content_hash}`. Only
canonically decoded and hash-verified artifacts may enter a cache under this
key; a bare content hash is never sufficient.
_Avoid_: bare-hash blob cache, unchecked cache insertion

**Execution Capability**:
A canonical namespaced descriptor for a genuinely additive execution feature.
Artifacts carry direct requirements and the verifier recomputes their
transitive union. A capability cannot redefine an existing opcode, kind,
frame, cleanup, or link meaning; such a change mints a new execution-ABI ID.
_Avoid_: capability flag as ABI compatibility escape hatch

**Program Artifact Manifest**:
A hash-covered root binding the entry function, complete transitive function
closure, schemas, trait dispatch, foreign entries, cleanup/effect/layout
companions, permissions, and recomputed capability union under one execution
binding.
_Avoid_: serialized in-memory Program as canonical artifact, unbound side table

**Verified Program Artifacts**:
The only artifact state accepted by the linker. A single verifier checks format,
ABI, canonical hashes, structure, transitive consistency, companions,
capabilities, and permissions before producing this capability.
_Avoid_: linking ordinary unverified Program data

**Provider Artifact**:
A Shape-native remoting provider packaged as an ordinary verified
`ProgramArtifactManifest` under the core execution ABI. A separate host-grant
manifest authorizes its provider lifecycle role and privileged intrinsics; it
does not use a distinct loader, bytecode format, or semantic runtime.
_Avoid_: native provider binary, special provider bytecode

**Provider Runtime**:
A host-owned isolated Shape runtime/actor executing one provider generation
under immutable grants and exchanging canonical typed messages with application
runtimes. It shares no application VM stack, heap, or mutable registry. Unsafe
or non-cooperative host primitives require a killable process boundary.
_Avoid_: provider code inside application frames, shared mutable provider singleton

**Provider Generation**:
An immutable provider instance identity binding the verified provider artifact,
normalized non-secret configuration digest, grant set, and runtime generation.
Destinations, placements, sessions, and obligations pin this identity. Reload
publishes a new generation for new work while old generations drain or report
explicit abandonment.
_Avoid_: mutating provider config in place, reloading beneath an in-flight call

**Provider Rebinding**:
The explicit restore-time validation of a persisted sealed destination against
its provider artifact identity, configuration digest/reference, destination
schema, and current host grants. Success mints a new generation-bound
capability; no current provider instance is adopted implicitly.
_Avoid_: restoring a live provider pointer, silent provider substitution

**`@remote` Annotation**:
An ordinary typed Shape annotation defined in the stdlib. The compiler knows
the generic annotation protocol, signature-indexed hook types, and remote
intrinsics, but does not special-case the `remote` annotation name. Annotation
specialization happens at compile time; distributed execution happens at
runtime through `Remote Dispatch`. Its core configuration is exactly one
`Placement<P>` plus provider-neutral `RemoteCallOptions`; it has no string,
host-list, raw destination, or provider-selection overload. It preserves the
target's parameters and `R` while adding explicit `Remote<P>` and `Suspend`
effects to the frozen callable signature.
_Avoid_: compiler-builtin `@remote`, source macro that implements networking

**Remote Effect**:
The callable effect `Remote<P>` identifying distributed execution through
provider `P`. It composes with `Suspend`, cleanup, and other effect rows and
cannot be erased by an annotation or higher-order conversion.
_Avoid_: hidden network effect, remote call in a pure synchronous context
