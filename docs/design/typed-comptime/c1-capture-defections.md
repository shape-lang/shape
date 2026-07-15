# ADR-009 C1 — Rejected Capture Designs and Boundary Decisions

[Defections index](../../defections.md) ·
[Current C1 status](c1-generated-captures.md)

This record preserves the rejected carriers, second-authority attempts, and
final boundary decisions from the C1 rework. The implementation and proof code
are committed, but final supervisor execution gates and issue closure remain
pending.

## 1. Span-keyed `capture(...)` head form and name identity

The first attempt (`34c57f5c`) stripped a leading `capture(...)` expression
before checking and recorded the declaration in a side table keyed by `Span`.
Generated snippets are independently parsed at byte offset zero and may carry
`Span::DUMMY`, so unrelated closures could collide and consume each other's
declarations. Its live descriptor matched source names rather than
compiler-issued scope slots and explicitly omitted the ruled `FunctionExpr`
field.

Worse, the declaration was validated and then discarded: the pre-existing
inference selector still chose the emitted `CaptureKind`, so changing or
deleting a declared mode could leave identical bytecode. The attempt could not
prove the ticket's central invariant and was not merged.

Chosen: `captures: Option<CaptureClause>` on `Expr::FunctionExpr`, backed by
node-borne generated origin and structural binding identity. Direct generation
is C1's producer; C2 `CheckedBody` staging will populate this same carrier.

## 2. Inference as fallback or second capture producer

REFUSED. The declared and discovered sets meet through one structural slot
resolver. The declared plan is the only authority for generated closure layout
and access opcodes. Binding analysis may validate ownership and storage, but it
cannot supply an undeclared capture, substitute another mode, or repair a
missing, length-mismatched, or kind-mismatched artifact. Every disagreement is
a compile error.

Ordinary source closures without a clause retain capture inference. Generated
and ordinary paths are mutually exclusive at the one selector.

## 3. Explicit capture clauses in ordinary source

REFUSED by Decision 95. The closure-pipe spelling
`|item: int; move scale, share total|` is generated-code-only. An ordinary
source closure carrying a clause receives `[C0903]`; an ordinary closure with no
clause keeps inference. Generated code with a missing declaration retains the
Wave-46 implicit-capture rejection. Declared-but-unused and duplicate entries
are also hard errors. Node-borne generated origin, not function name or span,
selects the rule.

## 4. A declared word lowering to a differently named ownership kind

REFUSED by the final user rulings. The closed mapping is:

- `move` over local `let` -> `Immutable`;
- `move` over local `let mut` -> `OwnedMutable`;
- `move` over a module binding -> `[C0906]`, never silently `Shared`;
- `share` over `var`, an existing `SharedCell`, or a module binding -> `Shared`;
- `share` over plain local `let` / `let mut` -> `[C0908]`;
- `&` and `&mut` -> `[C0902] ReferenceEscapeIntoClosure` until regions exist.

There is no `lowered != declared` escape hatch. Shared ownership has its own
source word; `move` never means `Shared`.

## 5. Treating a prerequisite or authored test as the C1 proof

REFUSED. The integrated native-closure prerequisite proves ordinary-source
immutable, owned-mutable, and Shared captures can reach native JIT; it does not
exercise a generated declaration. C1 carries a separate exact-output,
zero-fallback matrix over generated move and share fixtures plus the ordinary
inferred #53 controls. Those tests remain authored evidence until the
supervisor lane executes them against the final head.

Refcounted Shared is no longer a recorded fallback limitation. String has
ordinary and generated nested zero-fallback fixtures. The canonical
`HeapKind::ALL` decoder covers all 36 heap kinds, including Matrix and
MatrixSlice, and focused tests exercise typed zero/read/write/drop lifecycle
for those two formerly omitted variants. The remaining native limitation is
module-binding function-body lowering under W39 F1.

The tooling proof is likewise distinct. `CaptureAnalysis` provides structural
facts to validated packs; `GeneratedCaptureQuery` projects only those packs and
verified source maps. Hover, definition, references, and descriptor-authoritative
rename consume that query. C0910/C0911, missing maps, and nonlocal anchors stop
the operation rather than enabling a name/span guess. The committed proof code
still awaits supervisor execution.

## 6. ABI or presentation as exact specialization authority

REFUSED. ABI-equivalent nominal and callable types can be semantically distinct.
Declared parameters are opaque compiler-issued `TypeVar` capabilities whose
identity is owner + ordinal. Their transient annotation carrier is
authenticated and fails closed. An exact call-site fact must match the active,
inference-issued `SemanticCalleeDeclaration` before every cache lookup by opaque
parameter token, ordinal, and current declaration spelling. Spelling is a
consistency check, never identity.

An asserted exact fact whose callee capability is missing, foreign, stale,
renamed, or malformed is C0911 and cannot enter an ABI-only cache. Exact and
legacy specializations use disjoint cache maps, progress keys, and symbols.
Exact keys add ordered `SemanticFreeze` category/identity pairs and active
lexical parameter identities. A call site genuinely classified as absent,
unavailable, or conflicting evidence may use the legacy execution domain, but
neither domain can borrow the other's entry.

## 7. Same-spelled lexical generic rebinding

REFUSED. Ordinary recursive compilation installs an isolated callee scope.
Only the explicit lexical-inline route composes caller and callee parameter
identities. Current authored `type_ref(T)` syntax is name-indexed, so when both
owners spell a parameter `T`, the optimizer cannot prove which owner authored
the reference.

Closure-aware inlining refuses before cache, progress, symbol, counter, or
function publication and uses the already-compiled ordinary closure plus exact
type-only callee path. This `Ok(None)` is a bounded optimization limitation
after hard callee-capability validation, not a downgrade of invalid exact
evidence. Name coincidence is only a refusal signal.

## 8. Reconstructing imported or replayed authority from names

REFUSED. Current imported module calls do not enter local executable
monomorphization, and the interface-check replay helper does not publish facts
to the VM, so C1 needs no serialized capability format. A future cross-module,
remote, or persisted specialization must persist or reissue the opaque callee
capability and ordered semantic arguments. Rebuilding owner/ordinal authority
from public parameter names would reopen the cache-confusion defect.

## Current and target boundary

CURRENT on the C1 branch: canonical `FunctionExpr` carrier, four-mode catalog,
structural binding and lineage identity, generated-only gate, one declared plan
driving emission, opaque declared-parameter authority, disjoint exact/legacy
specialization, native scalar/refcounted Shared code, and compiler-query hover,
definition, references, and descriptor-authoritative rename.

TARGET before C1 ratification: whole-C1 Standards/Spec fixed-point acceptance
and the supervisor cargo, ShapeTest, LSP, CLI, and broad gates. The committed
generated zero-fallback, refcounted lifecycle, rename, and #53 proofs are not
capability claims until those gates pass. TARGET C2 constructs the same carrier
through `CheckedBody<Sig, Captures>`. TARGET W39 F1 enables native
module-binding `share`. Any future persisted specialization carries or reissues
the opaque callee capability rather than reconstructing it from names.
