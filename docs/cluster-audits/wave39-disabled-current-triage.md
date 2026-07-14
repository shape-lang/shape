# Wave 39 Current Disabled-Book Triage

Date: 2026-07-10
Role: Wave-39V read-only completeness/book-state analyst

## Scope And Evidence

This is a static reconciliation of the fresh sibling manifest, current sibling
MDX, `AGENTS.md`, and the current Wave-38/Wave-39 audit reports. No cargo,
just, nextest, rustc, build, test, extraction, or book-truth command was run by
this analyst. The only file written is this report.

Authoritative evidence:

- Manifest: `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`, generated `2026-07-10T07:10:04.546Z`.
- Extraction run `run-p2343094-i33024039.service`: 707 total, 565 runnable, 142 disabled, expected-output 11, expected-fail 6, fixture 11.
- Release-binary VM+JIT gate `run-p2343484-i33024441.service`: 565/565, with zero failures, divergences, or timeouts.
- Baseline: `docs/cluster-audits/wave38-disabled-current-triage.md` recorded 559 runnable / 148 disabled and the six-category taxonomy.

The manifest has exactly 142 `runnable=false` entries, and the current MDX has
exactly 142 ` ```shape runnable=false` fences. Their page/line sets agree.

## Book State

### Meaning Of 565/565

`565/565 = 100%` means every one of the 565 currently runnable extracted
snippets passed in the shipped release binary under both VM and JIT. It includes
the current fixture-backed rows and rows with expected output or expected-fail
metadata; those metadata counts are not extra snippets.

It does not mean all 707 book snippets pass. The other 142 are intentionally
excluded from execution because they are classified below. It also does not
prove disabled language features, external extension setup, preview APIs,
retired syntax, planned tooling, or arbitrary VM/runtime/JIT behavior. The
Wave-38 proof refresh states the same boundary: book truth proves the runnable
surface only (`docs/cluster-audits/wave38-global-proof-gap-refresh.md:45-55`).

### Exact Inventory

Counts reconcile from Wave 38 as follows: three active math rows became
runnable, and three external loopback `@remote` annotation rows became
runnable. Therefore `68 - 3 = 65` active rows and `41 - 3 = 38` external rows;
the other categories are unchanged.

| Category | Count | Share of 707 | Current interpretation |
|---|---:|---:|---|
| Active missing feature | 65 | 9.2% | Shipped language/runtime/stdlib behavior still fails or is incomplete. |
| External/manual/fixture-only | 38 | 5.4% | Behavior may exist, but the deterministic book harness lacks a required peer, extension, project, or host fixture. |
| Preview/out-of-scope | 22 | 3.1% | Explicitly preview, illustrative, or outside the current supported surface. |
| Old syntax/book rewrite | 8 | 1.1% | Documentation still uses retired syntax or an obsolete forwarding shape. |
| Proof/design gap | 5 | 0.7% | Planned debug/proof APIs rather than a missing ordinary runtime operation. |
| Intentional diagnostic, not yet expected-fail | 4 | 0.6% | Negative examples lack a stable one-diagnostic contract. |
| **Disabled total** | **142** | **20.1%** | Exact `runnable=false` manifest inventory. |

Area totals are `language surface 51`, `state/snapshot/distributed/proof 36`,
`comptime/extensions/tooling 28`, and `stdlib/math/domain 27`.

#### Active Missing Feature: 65

- **Language surface, 22:** `advanced/ownership-deep-dive.mdx:81`; `examples/comptime-codegen.mdx:22`; `fundamentals/datetime.mdx:364,404`; `fundamentals/error-handling.mdx:186,207,224,275,287`; `fundamentals/objects-arrays.mdx:366`; `fundamentals/operators.mdx:436,503`; `fundamentals/references-borrowing.mdx:73`; `fundamentals/strings.mdx:277,302`; `fundamentals/tables.mdx:109`; `fundamentals/traits.mdx:71,172,249,265,387`; `fundamentals/variables.mdx:82`.
- **State/distributed, 12:** `advanced/content-addressed-bytecode.mdx:154,168,226,264,541`; `stdlib/core/state.mdx:225,241,334,401,484,514,541`.
- **Comptime/tooling, 6:** `advanced/annotations.mdx:73,89`; `advanced/comptime-annotations-cookbook.mdx:31`; `advanced/comptime-llm-patterns.mdx:170`; `advanced/comptime.mdx:266`; `tooling/polyglot.mdx:96`.
- **Stdlib/math/domain, 25:** `stdlib/core/distributions.mdx:49`; `stdlib/core/monte_carlo.mdx:82`; `stdlib/core/property_testing.mdx:19,32,49,77`; `stdlib/core/stochastic.mdx:30,47,64,80`; `stdlib/core/testing.mdx:44,59,88,103`; `stdlib/domain/finance.mdx:16`; `stdlib/domain/physics.mdx:20,81`; `stdlib/domain/simulation.mdx:32,82,106`; `stdlib/math/interpolation.mdx:51`; `stdlib/math/optimize.mdx:58,78`; `stdlib/math/rotation.mdx:32,43`.

The three removed active rows are `stdlib/core/math.mdx:70,86,102`:
`correlation`, `covariance`, and `percentile`. Their current fences are
`runnable=true`, and the typed VM/JIT proof is recorded in
`AGENTS.md:50-51`.

#### External/Manual/Fixture-Only: 38

- **Language surface, 8:** `examples/web-request.mdx:22`; `fundamentals/datetime.mdx:19`; `fundamentals/functions.mdx:413`; `fundamentals/modules.mdx:80,191`; `fundamentals/resource-management.mdx:365,387`; `fundamentals/variables.mdx:168`.
- **State/distributed, 9:** `advanced/content-addressed-bytecode.mdx:515`; `advanced/module-distribution.mdx:563`; `advanced/polyglot-distributed.mdx:149`; `advanced/security-permissions.mdx:441`; `advanced/wire-protocol.mdx:90`; `stdlib/core/remote.mdx:113,226`; `stdlib/core/transport.mdx:61,95`.
- **Comptime/extensions/tooling, 21:** `advanced/annotations.mdx:508`; `advanced/comptime-annotations-cookbook.mdx:183,329`; `advanced/native-c-interop.mdx:139,155,286`; `tooling/extensions.mdx:120`; `tooling/polyglot.mdx:14,126,186`; `tooling/python-extension.mdx:68,117,142,163,184,197`; `tooling/typescript-extension.mdx:74,134,163,180,238`.
- **Stdlib/math/domain, 0.**

The three removed external rows were the pure loopback fixtures
`stdlib/core/remote.mdx:42,77` and `advanced/annotations.mdx:480`. The
Wave-39 fixture audit identified exactly these rows as the existing `serve`
fixture lane (`docs/cluster-audits/wave39-external-fixture-flips.md:23-61`),
and the current book gate proves the flip (`AGENTS.md:49,61`). The remaining
`remote.mdx:113` is a Python-extension example requiring a foreign receiver;
`:226` is a negative-endpoint diagnostic whose unused-port setup is not
deterministic (`remote.mdx:105-124,220-233`).

#### Preview/Out Of Scope: 22

- **Language surface, 13:** `advanced/ownership-deep-dive.mdx:45,54,459,470,483`; `fundamentals/async.mdx:123`; `fundamentals/content.mdx:51,61,107,453`; `fundamentals/tables.mdx:56,76`; `fundamentals/traits.mdx:330`.
- **State/security, 7:** `advanced/content-addressed-bytecode.mdx:321`; `advanced/security-permissions.mdx:329,346,360,383,466,498`.
- **Stdlib/domain, 2:** `stdlib/domain/iot.mdx:17,126`.

#### Old Syntax/Book Rewrite: 8

- **Language surface, 5:** `advanced/ownership-deep-dive.mdx:259`; `fundamentals/error-handling.mdx:90`; `fundamentals/references-borrowing.mdx:269`; `fundamentals/strings.mdx:397`; `fundamentals/tables.mdx:125`.
- **State/distributed, 2:** `advanced/content-addressed-bytecode.mdx:282,396`.
- **Comptime, 1:** `advanced/comptime-annotations-cookbook.mdx:308`.

The content-addressed rows still use retired `__original__(args)` forwarding;
they are documentation rewrites, not evidence that the current content-addressed
runtime is absent.

#### Proof/Design Gap: 5

All five are `advanced/developer-tools.mdx:86,137,238,320,462`. They describe
planned `std::debug` hot reload, time travel, prefetch, or proof APIs, not
ordinary shipped features that should be silently enabled.

#### Intentional Diagnostics: 4

- `advanced/ownership-deep-dive.mdx:399,425`
- `fundamentals/references-borrowing.mdx:253`
- `advanced/security-permissions.mdx:414`

These remain disabled because the examples are mixed/unstable borrow,
concurrency, or permission-context failures rather than stable single-error
contracts. The expected-fail harness is therefore not evidence that these four
should flip automatically.

## Working Feature Frontier

The current state is a collection of verified narrow slices, not a claim of
language-wide completeness.

| Surface | What is verified now | What remains outside the proof |
|---|---|---|
| Strict typed core and VM/JIT parity | Compiler typed-opcode source coverage has zero reported unproven gaps; the field-mutation semantic bridge has storage/Miri, VM, JIT-FFI, and ShapeTest evidence (`wave38-global-proof-gap-refresh.md:22-32,59-78`). The 565 runnable rows pass in both modes. | Source guards do not execute the compiler, and book parity is only the runnable corpus. The Wave-38 report explicitly rejects a global VM/runtime/JIT proof (`:18-20,40-43`). |
| GC and typed mutation | Ordinary typed-object field overwrite preserves carrier metadata and has targeted Miri/barrier evidence (`wave38-global-proof-gap-refresh.md:61-78`). | Breadth across every native object, trait/container write, return path, heap kind, and full JIT codegen remains open (`:202-209`). |
| Content-addressed distributed execution | Inbound blobs are hash-checked before caching; same-connection negotiation, resupply, transitive nested zero-capture closure transfer, and zero-blob reuse pass real-socket proofs (`AGENTS.md:44,58-60`). | This does not close the complete `@remote` value/capture matrix, all cache persistence modes, remote snapshot consumers, or every closure/capture kind. |
| Snapshot/wire provenance | Owning serialization and restore carriers preserve `HeapNode`/`HeapRef` identity, typed-object arrays/maps, and `Result`/`Option` normalization. Normal 4/4 and Stacked/Tree/Strict Miri 4/4 passed (`wave39-snapshot-wire-restore-provenance.md:6-47`; `AGENTS.md:52,57,74`). | This is targeted runtime evidence only. VM stack/module writers, closure/shared-cell interiors, state/resume, remote serialization, remaining wire arms, and arbitrary programs are Stage 2 (`wave39-snapshot-wire-restore-provenance.md:49-56`). |
| State and resume | Metadata capture, bounded state operations, internal CLI snapshot/resume, and two local resumability book fixtures work (`wave39-public-state-resume-scope.md:44-85`). | Public `state.resume` must not consume the metadata-only carrier. The smallest honest next slice requires a versioned executable carrier, validated IP, program/schema identity, callback wiring, and a constrained empty top-level continuation (`wave39-public-state-resume-scope.md:12-42,87-110`; `AGENTS.md:45`). |
| Async | Pending futures fail closed at snapshot boundaries; remote-callee `Future<T>` typing/materialization and direct/async/`join all` distributed cases pass 9/9 (`AGENTS.md:48,69,73`). | Durable remote future identity, polling/cancellation of remote callee futures, pending-future resume, streams/`for await`, value-producing `join settle`, and JIT async remain open (`wave38-global-proof-gap-refresh.md:80-98,188-193`). |
| Comptime and annotations | The typed `ItemFragment` zero-arg literal slice, nested annotation-array carrier selection, positional nested-array serialization, and generated extension-method JIT parity are verified (`wave38-global-proof-gap-refresh.md:125-141`; `AGENTS.md:50,53-54,72,77`). | Comptime is not yet a typed macro system: broad generated bodies and metadata still use source/JSON payloads. The general annotation `ArgumentPack` redesign remains open; current nested-array support is a bounded carrier fix, not that redesign. |
| Stdlib slice | `correlation`, `covariance`, and `percentile` now have canonical typed VM handlers with VM/JIT fallback parity and book rows enabled (`AGENTS.md:46,50-51`; `wave39-stdlib-next-slice.md:32-76`). | The remaining 25 active stdlib/domain rows include distributions, stochastic, testing/property helpers, Monte Carlo, finance/physics/simulation, interpolation, optimization, and rotation; these need separate carriers and dispatch proofs. |
| Polyglot and fixtures | Required-extension distributed Python/TypeScript composition and selected snapshot/resume/extern-C paths are covered by explicit tests; local snapshot/resume is fixture-backed (`AGENTS.md:75-76`; `wave38-global-proof-gap-refresh.md:100-123`). | Dynamic extension book rows still require pinned artifacts and fixture contracts. No external setup should be inferred from a passing ordinary book gate. |
| Book fixtures | `serve` now truthfully covers three pure loopback annotation rows; `local-snapshot-resume` covers two local rows; the full shipped gate is zero-failure (`AGENTS.md:49,61,75`). | Extension, project-module, live transport, HTTP, permission, native-library, and foreign-frame fixtures remain separate harness work. |

## Missing Frontier And Ranked Next Lanes

The ranking separates user-facing implementation from proof and harness work.
Dependencies are stated so a lane is not mistaken for an isolated row flip.

1. **Public executable `VmState` / `state.resume` slice.** Implement the
   constrained empty top-level continuation first, with version, program/schema
   identity, validated resume IP, callback wiring, and structured rejection.
   This is the highest-value active state row and depends on the carrier contract
   in `wave39-public-state-resume-scope.md:12-42,87-110`.
2. **VM stack/module snapshot provenance.** Extend the now-closed runtime
   serializer/restore carrier into VM stack and module-binding writers, then
   prove ownership under all three Miri modes. This is the prerequisite for
   honest full-program snapshot claims; current Stage-2 boundaries are listed at
   `wave39-snapshot-wire-restore-provenance.md:49-56`.
3. **Closure/shared-cell and remote provenance consumers.** Carry owning slots
   through call-stack captures, closure captures, remote argument/result paths,
   and nested shared-cell values. This depends on lane 2 and is distinct from
   the already-closed immutable transferred-closure layout proof
   (`AGENTS.md:58-60`).
4. **General annotation `ArgumentPack` redesign.** Keep the current nested-array
   carrier and positional-pack fixes, but design a typed, explicit pack for
   heterogeneous/nested annotation arguments without relying on one structural
   array encoding. This is the open annotation architecture boundary, not a
   stale book flip.
5. **Complete the `@remote` typed value/capture matrix.** Add focused scalar,
   array, map, closure, typed-object, error, and annotation cases over real
   sockets, then consume provenance-bearing carriers. This raises distributed
   confidence beyond the current narrow content-addressed and loopback slices.
6. **Finish the next typed stdlib intrinsic cluster.** After the proven three
   statistical rows, take distributions/stochastic/Monte Carlo as one bounded
   dispatch family, preserving typed-array semantics and VM/JIT parity. The
   current audit identifies the remaining active family and its dependency on
   typed handlers (`wave39-stdlib-next-slice.md:161-180`).
7. **Trait, conversion, error-context, and collection ergonomics.** Address the
   central language rows around `From`/`TryFrom`, optional conversion, generic
   trait dispatch/associated types, Result methods, collection accessors, and
   imported generic assertions. These have higher everyday user value than
   preview/debug rows but need compiler/type-system carriers rather than fixture
   changes.
8. **Broaden typed comptime generation.** Extend typed fragments beyond literal
   zero-arg functions, migrate generated bodies and annotation metadata away from
   source/JSON payloads, and add JIT/VM parity for generated methods and real
   stdlib generators. The first slice is explicitly narrow
   (`wave38-global-proof-gap-refresh.md:125-141`).
9. **Real async beyond materialized callee futures.** Implement and prove
   durable remote future identity, remote polling/cancellation, streams and
   `for await`, value-producing `join settle`, and JIT lowering in dependency
   order (`wave38-global-proof-gap-refresh.md:88-98,188-193`).
10. **Deterministic external fixture expansion.** Add pinned Python/TypeScript
    extension fixtures, extension-aware local snapshot/resume, isolated
    two-file modules, and controlled transport/permission/native fixtures. The
    first three ranked lanes are concrete and conditional, not current runtime
    failures (`wave39-external-fixture-flips.md:63-180`).

## Completeness Metrics And Caveat

- Runnable truth coverage: `565 / 707 = 79.9%`.
- Disabled inventory: `142 / 707 = 20.1%`.
- Enabled release gate: `565 / 565 = 100%`.
- Active missing-feature rows: `65 / 707 = 9.2%`.
- External/manual/fixture-only rows: `38 / 707 = 5.4%`.

These are book-state metrics, not a language-spec completeness metric. A row is
an example at one source location with one expected behavior and one harness
contract. It is not a denominator over the grammar, type system, runtime,
stdlib API, memory-safety surface, JIT, FFI, distributed protocol, or all
possible programs. The right reading is: 79.9% of extracted examples are
currently truth-gated, and every one of those passed the shipped two-mode gate;
the remaining 20.1% require the categorized work above.

## Stale/Flip Candidates

**Remaining stale/flip candidates: 0.** The fresh manifest and current MDX
agree on all 142 disabled rows. The six rows that current code/proofs covered
were already flipped in this wave: three typed statistical rows and three
loopback `@remote` rows. The local snapshot/resume rows were correctly flipped
in the earlier fixture wave. Every remaining row has either an explicit current
external/preview/old-syntax/diagnostic/design explanation or an active missing
feature reference above. In particular, metadata-only `state.capture_all`,
targeted snapshot provenance, nested annotation carrier support, and selected
distributed closure proofs do not imply that their broader disabled rows are
runnable.

## Open Boundaries

The just-closed `@remote` chain proves frame-prefix derivation, immutable nested
closure reconstruction, same-socket resupply/cache reuse, and the three pure
book fixtures. It does not prove arbitrary mutable/reference/shared captures,
remote provenance consumers, or the general `ArgumentPack` design.

The strict snapshot/wire chain proves carrier-aware serialization and restore
for the named HeapNode/HeapRef, typed-object array/map, and Result/Option cases
under normal, Stacked, Tree, and Strict Miri. It does not close VM stack/module,
closure/shared-cell, public state/resume, remote, JIT/FFI, or arbitrary wire
value provenance. Those remain Stage 2 by design.
