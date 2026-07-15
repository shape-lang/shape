# ADR-009 C1 — Generated Captures and Exact Semantic Evidence

[Typed-comptime status index](../typed-comptime.md) ·
[Rejected designs and boundary decisions](c1-capture-defections.md)

## Status

The complete C1 implementation and its focused proof code are committed on the
`adr009/c1-rework` branch. The final supervisor cargo, ShapeTest, LSP, CLI, and
broad execution gates have not run against the current head, so this document
does not claim an overall green gate, issue closure, or merge ratification.

## Generated-only capture surface

Generated closures declare their complete capture set after a semicolon in the
closure pipe:

```shape
|item: int; move scale, share total| item * scale + total
```

The clause is generated-code-only. An explicit clause in ordinary source is
the named `[C0903]` rejection, while an ordinary closure without a clause keeps
capture inference. A generated closure that uses an undeclared binding retains
the deterministic implicit-capture rejection; declared-but-unused and
duplicate entries are also compile errors.

`captures: Option<CaptureClause>` on `Expr::FunctionExpr` is the canonical
carrier. Directly generated source is its C1 producer. C2 `CheckedBody` staging
will populate the same field: two producers of one carrier, never two capture
mechanisms.

The final mapping is closed:

| Declaration | Binding | Result |
|---|---|---|
| `move x` | local `let` | `CaptureKind::Immutable` |
| `move x` | local `let mut` | `CaptureKind::OwnedMutable` |
| `move x` | module binding | `[C0906]` rejection |
| `share x` | `var`, existing `SharedCell`, or module binding | `CaptureKind::Shared` |
| `share x` | plain local `let` / `let mut` | `[C0908]` rejection |
| `&x` / `&mut x` | any binding | `[C0902] ReferenceEscapeIntoClosure` until regions exist |

Declaration and discovery meet through compiler-issued slot and binding-lineage
identity, never source name or span. The declared plan is the sole authority for
generated closure layout and access opcodes. Inference supplies storage and
ownership facts only to validate the declaration; it cannot fill an absent
entry, change its mode, or become a second producer.

## Exact semantic authority

Generated callable and capture facts retain an explicit `Exact` /
`Unavailable` / `Conflict` classification. Exact capture types close through
`SemanticFreeze`, including parameter passing modes, optionality, and return
semantics for callable-valued captures. ABI pointer kinds, registry ids,
rendered names, and spans never become semantic type evidence.

Declared generic parameters are opaque compiler-issued `TypeVar` capabilities.
Their semantic identity is owner + ordinal; source spelling is presentation and
an independent active-declaration consistency check, not an identity. The
transient annotation carrier is authenticated and fails closed.

Before every exact cache lookup, the exact call-site fact must match the active,
inference-issued `SemanticCalleeDeclaration` by opaque parameter token, ordinal,
and current spelling. An asserted exact fact with a missing, foreign, stale,
renamed, or malformed callee capability is `[C0911]`; it cannot downgrade into
an ABI-only result.

Exact-semantic and legacy ABI specializations occupy disjoint cache maps,
progress keys, and symbols. Exact keys add ordered frozen semantic arguments
and every active lexical parameter identity. Legacy keys may carry the lexical
context but cannot borrow an exact entry. Only a call site classified by
inference as absent, unavailable, or conflicting evidence enters the legacy
domain.

Ordinary recursive compilation installs an isolated callee scope. Lexically
spliced closure bodies are the only path that composes caller and callee scopes.
If both owners declare a generic parameter with the same spelling, the current
name-indexed `type_ref(T)` syntax cannot prove which owner authored the
reference. Closure-aware inlining therefore refuses before cache, progress,
symbol, counter, or function publication. The already-compiled ordinary
closure and exact type-only callee route remain available. This is a bounded
optimization refusal, not semantic fallback; name coincidence is only the
refusal signal.

Current imported-interface replay does not publish executable specialization
facts into local monomorphization. A future serialized, remote, or cross-module
specialization must persist or reissue the opaque callee capability and ordered
semantic arguments. Reconstructing authority from parameter names is not a
valid extension.

## Runtime, JIT, and Shared ownership

The authored C1 matrix covers generated move-let, move-let-mut, move-heap, and
nested-share closures; the ordinary inferred nested-Shared #53 reproduction;
exact-output, zero-fallback CLI fixtures under
`jit_generated_capture_native`; compiler proof
`nested_declared_share_preserves_the_outer_cell_descriptor`; and balanced VM
install/teardown proofs
`shared_scalar_capture_frame_local_has_one_balanced_carrier_share`,
`shared_heap_capture_frame_local_retains_the_cell_not_its_payload`, and
`nested_declared_share_runs_through_vm_install_and_teardown`.

Refcounted Shared payloads are now authored as a native path rather than an
intentional fallback. The JIT keeps the raw `Arc<SharedCell>` as the one cell
identity. Reads retain through the cell-owned `NativeKind`; replacements
transfer the incoming share and retire the displaced value after unlocking;
the final cell drop retires the current payload through the canonical typed,
GC-aware dispatch.

The focused proof inventory distinguishes public execution from catalog and
lifecycle completeness:

- N9 in `jit_closure_capture_native.rs` requires native String read, replace,
  outer observation, exact output, and zero fallback.
- `jit_generated_capture_native.rs` adds both generated and ordinary inferred
  nested-String recapture fixtures, each requiring exact VM/JIT output and zero
  fallback while preserving one cell identity.
- `HeapKind::ALL` is the canonical, compile-time-checked 36-variant ordinal
  catalog. The compact JIT kind decoder round-trips every entry, including
  `Matrix` and `MatrixSlice`, before any Shared allocation can be emitted.
- `shared_cell_tests.rs` uses zero payloads to prove compact-code routing and
  null-safe admission across the catalog, alongside focused String,
  Matrix/MatrixSlice, and nested-recapture ownership proofs.
- `shared_cell_ownership_matrix.rs` drives the generated `HeapKind::ALL`
  catalog through an exhaustive match and constructs every accepted nonzero
  scalar or heap carrier. It proves retained reads, read-share release,
  replacement retirement, and final cell drop; `Ptr(NativeScalar)` remains the
  sole typed pre-allocation refusal.

These tests and routes are authored and committed but await the supervisor
execution lane. They are not yet a verified capability claim. Module-binding
capture inside a JIT-compiled function remains the separate W39 F1 surface;
therefore `share` over a module binding cannot yet claim native zero fallback.

## Compiler-query tooling

`EnvironmentAnalyzer::analyze_function_captures` supplies structural facts and
real occurrences to the validated packs. `BytecodeCompiler::generated_capture_query`
projects `GeneratedCaptureQuery` only from those packs and structurally verified
source maps. `GeneratedCaptureBindingIdentity` joins sibling, nested, and
multi-application occurrences; `GeneratedCaptureOccurrenceIdentity` preserves
each exact declaration and body position. `GeneratedCaptureDescriptorView` and
`GeneratedCaptureSourceMap` are read-only projections under
`GeneratedCaptureStage::GeneratedOnly`.

Hover, go-to-definition, references, and rename consume the same compiler
query. Rename is descriptor-authoritative: the cursor selects a capture through
`capture_at`, the opaque binding identity selects the complete verified anchor
graph, and the edits equal the reference graph. A same-spelled binding under a
different owner is untouched. Missing source, nonlocal anchors, unavailable
query context, C0910, or C0911 is terminal; generated capture rename never falls
through to generic name-based rename. Generated capture and generated-symbol
providers reuse the same bounded request compiler. Frontmatter and
import-registered requests retain the same offsets and module context.

Effect rows remain outside C1 because no authoritative compiler effect carrier
exists yet. Tooling does not relabel ownership mode as an effect.

## Evidence state

Focused static/adversarial review has passed for the exact-specialization,
Shared-lifecycle, and descriptor-authoritative rename corrections. All named
execution proofs remain pending the supervisor lane and the whole-C1
fixed-point gate. Book prose and runnable examples remain TARGET stage F1.
