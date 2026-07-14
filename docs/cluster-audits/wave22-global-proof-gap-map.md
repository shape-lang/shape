# Wave 22 Global Proof Gap Map

Date: 2026-07-09

Scope: Wave-22E read-only scout. This report refreshes the global proof map
after Wave 21 using existing source, docs, tests, and cluster closeout evidence.
No cargo, just, nextest, rustc, build, test, bench, Miri, or book-truth command
was run for this scout. The only planned verification is a cheap static
`git diff --check` over this report.

## Current Baseline

Wave 21 changed the release-risk shape. The current book extraction baseline in
the active registry is 756 total snippets, 532 runnable, and 224 disabled, with
the last full book gate passing 532/532 (`AGENTS.md:44-51`, `AGENTS.md:98`).
That means the runnable surface is green, but the disabled surface remains a
separate completeness risk rather than test evidence.

The JIT field mutation story is materially stronger than it was in Wave 20.
Wave 21 reports direct JIT field reads no longer fall through to legacy
schema-less `get_prop`; resolved reads require byte offset plus projected
`NativeKind`, use schema-aware typed-object get-field FFI, and unresolved reads
deopt honestly (`AGENTS.md:49`). The current JIT lowering matches that: direct
field reads require a resolved typed-object layout before calling
`typed_object_get_field`, while unresolved/string property paths surface or
deopt (`crates/shape-jit/src/mir_compiler/places.rs:1063-1172`).

State introspection also moved from fabricated placeholders toward honest
blockers. `state.capture()` returns bounded real frame metadata, but
`capture_all()`, `resume()`, and `resume_frame()` still refuse to invent nested
`VmState`, dispatch, live slots, or resume IP (`AGENTS.md:51`,
`crates/shape-vm/src/executor/state_builtins/introspection.rs:52-70`,
`crates/shape-vm/src/executor/state_builtins/introspection.rs:398-474`).

## Proof Tiers

Source-only guards are valuable drift detectors, not semantic proofs. The typed
opcode coverage checker is explicitly source-only and scans compiler Rust source
without cargo, rustc, nextest, or Miri (`scripts/check-typed-opcode-proof-coverage.py:2-8`).
Its baseline expects zero unclassified typed-opcode proof gaps, but the buckets
are source mentions and helper classifications (`scripts/check-typed-opcode-proof-coverage.py:29-34`,
`docs/cluster-audits/w91a-typed-opcode-proof-coverage.md:17-35`). The ignored
test classifier is also source-only; it guards `#[ignore]` reason taxonomy and
count drift, but it does not prove the ignored tests are obsolete or runnable
(`scripts/check-ignored-test-classification.py:2-7`,
`docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md:81-101`).

Targeted semantic/runtime evidence exists, but it is intentionally narrow. The
Miri provenance script covers selected unsafe carrier paths and explicitly
excludes the full VM/runtime/JIT/FFI/snapshot space, arbitrary Shape programs,
all stack overwrite sites, all typed-object field kinds, heap arrays, arbitrary
trait dispatch, wire restore, and ignored tests (`scripts/check-miri-provenance.sh:4-9`,
`scripts/check-miri-provenance.sh:54-99`). The JIT/GC evidence includes focused
unit tests and benchmark probes around typed-object set-field and write
barriers (`crates/shape-jit/src/ffi/gc.rs:86-266`), but that is still targeted
runtime evidence, not a global memory-safety or optimizer proof.

Book truth is broad user-surface evidence only for runnable snippets. A passing
532/532 gate says the currently enabled book examples agree with the gate's
oracle; it says nothing about the remaining 224 disabled snippets except that
they were outside that run.

## Ranked Proof Gaps

| Rank | Gap | Release risk | Current evidence | Evidence needed |
|---|---|---:|---|---|
| 1 | Public state carriers, resume, diff/patch | P0 | `FrameState` metadata is real, but public `VmState` and resume surfaces still stop at explicit blockers (`crates/shape-runtime/stdlib-src/core/state.shape:35-91`, `crates/shape-vm/src/executor/state_builtins_tests.rs:461-620`). | Schema-backed `VmState` with nested `Array<FrameState>` plus kinded module bindings, real args/locals/upvalues/resume IP, live dispatch callback, `resume_frame<T>`, diff/patch, snapshot roundtrip, and book/e2e examples that do not rely on fabricated metadata. |
| 2 | Global semantic proof bridge and unsafe provenance boundary | P0 | Static proof guards report no unclassified source gaps, and targeted Miri probes cover selected carriers, but the docs explicitly stop short of whole-runtime UB absence (`docs/cluster-audits/w94d-miri-unsafe-proof-expansion.md:7-47`). | Add targeted Miri probes for snapshot/wire restore, JIT FFI return paths, remaining typed-array and trait-object carriers, then connect source guards to runtime differentials so "covered" means executed or intentionally unreachable. |
| 3 | Trait/object native proof beyond honest deopt | P1 | Wave 21 closed direct field-read fallback for typed objects, but the registry still calls out trait dispatch native proof as a high-priority gap (`AGENTS.md:98`). Object/trait snippets currently deopt honestly rather than proving native parity. | VM/JIT differential corpus for trait dispatch, user impl calls, object field schemas, and typed method lookup; no-fallback probes for the promoted subset; explicit fallback assertions for unsupported dynamic trait/object cases. |
| 4 | Distributed, snapshot, and polyglot matrix breadth | P1 | There are non-ignored rows for TLS refusals, receiver snapshot-store ownership, dynamic runtime opt-in refusal, async remote calls, ordered `join all`, and live-future snapshot refusal (`bin/shape-cli/tests/distributed_matrix_e2e.rs:9-217`, `bin/shape-cli/tests/distributed_async_e2e.rs:6-223`). Positive Python/TypeScript transfer rows can still skip when extension artifacts are missing (`bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs:45-52`, `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs:302-320`). | Non-skipping extension fixture artifact or supervised build lane, combined TLS plus `remote::call_async` plus receiver-store snapshot plus resume/hash visibility, positive and refusal rows for Python/TypeScript, and security/permission negative rows. |
| 5 | Async beyond current value fan-in | P1 | `Future<T>`, `await`, `join all`, race/any cancellation, and `remote::call_async` have focused coverage. `join settle` is still a placeholder value surface, remote callees returning `Future<T>` are rejected, and live futures cannot be snapshotted (`tools/shape-test/tests/async_concurrency/join_strategies.rs:273-290`, `crates/shape-vm/src/executor/async_ops/mod.rs:698-720`, `crates/shape-vm/src/compiler/expressions/function_calls.rs:6282-6294`). | Materialized `join settle` values, native async signatures, user continuations, distributed cancellation, durable or explicitly refused remote future identity, pending-future snapshot/resume design, stream story, and JIT fallback invariants. |
| 6 | Comptime typed reflection, fragments, quasiquote, and hygiene | P2 | `set param` defaults and `replace module` re-analysis have tests, but directive payloads still frequently pass source text or lossy strings (`tools/shape-test/tests/annotations_comptime/directives.rs:8-240`, `crates/shape-vm/src/compiler/comptime_builtins.rs:233-287`, `crates/shape-vm/src/compiler/comptime_target.rs:414-440`). | Typed fragment/quasiquote API, typed annotation args, hygienic generated names/imports, structured type references instead of display strings, compatibility tests for legacy string directives, and VM/JIT flagship examples using the typed path. |
| 7 | Typed field mutation and JIT/GC barrier completeness | P2 | Option-field mutation has strict semantic tests, JIT typed-object set-field reads live field kinds and calls the write barrier, and barrier unit tests exercise typed-object overwrite/cycle collection (`tools/shape-test/tests/structs_types/option_field_mutation.rs:5-151`, `crates/shape-jit/src/ffi/typed_object/field_access.rs:68-103`, `crates/shape-jit/src/ffi/gc.rs:86-266`). Current risk is breadth, not the known heap overwrite path. | Current Wave-21 no-fallback benchmark artifact evidence for 17-20 attached to the report set, Miri probes around typed-object set-field/Option payload mutation, all field-kind schema tests, trait/container field variants, and repeated default-vs-barrier-off perf with artifact IDs. |
| 8 | Disabled-book completeness | P2 | The runnable book gate is green, but 224 snippets remain disabled after Wave 21. The prior triage grouped 245 disabled snippets into active implementation, external/manual, preview, proof/design, stale-green, old syntax, and diagnostic buckets (`docs/cluster-audits/wave19-disabled-current-triage.md:32-93`). Wave-22A owns the fresh 224-snippet reclassification. | Current per-snippet classification for all 224 disabled entries, stale-green flip candidates separated from manual/fixture/intentional diagnostics, exact expected-output strategy for nondeterministic hashes and host-state examples, then full gate after each count-reduction lane. |

## Requested Area Notes

Typed field mutation is no longer a first-order blocker for ordinary
`Option<T>` field assignment. The remaining proof work is to broaden beyond the
covered field kinds and connect JIT/GC runtime behavior to unsafe provenance
evidence. The important distinction is that static typed-opcode coverage already
classifies `Option<T>` field mutation as source-covered, while runtime proof
comes from focused Shape tests and JIT FFI/unit tests.

JIT/GC barriers have credible targeted evidence. The hot-path field overwrite
benchmarks were promoted in later waves, `jit_typed_object_set_field` uses the
live field kind to pick the old-value barrier tag, and barrier tests exercise
typed-object cycles. Release confidence still needs archived current Wave-21
artifact IDs plus a broader corpus proving that newly native object/trait
surfaces either use the same barrier discipline or deopt explicitly.

Distributed/snapshot/polyglot coverage is useful but still matrix-shaped. The
receiver-store snapshot tests are strong because they prove the hash is written
where the receiver runs, not where the caller runs. The weak point is positive
polyglot coverage that depends on extension artifacts and manual rows such as
SIGINT snapshot/resume.

Async proof is real for value fan-in, first-class futures, cancellation of local
race/any losers, and remote async composition. It is not yet proof of native
async function signatures, distributed cancellation, pending-future resume, or
remote future identity. The current live-future snapshot refusal is an honest
semantic boundary, not an implementation of resumable futures.

Comptime proof has moved past some Wave-12 gaps: `set param` scalar defaults and
`replace module` re-analysis now have focused tests. The remaining gap is
authoring and proof ergonomics. Too much of the public surface still serializes
or parses source text, stringified types, or lossy annotation arguments.

State carriers are the highest release risk because the public API names are
already aspirational: `VmState`, `resume`, and `resume_frame<T>` exist in the
stdlib surface, while the runtime correctly refuses to fabricate the missing
carrier data. This needs implementation proof, not just better diagnostics.

Disabled-book completeness remains a completeness signal, not a failing-test
signal. The current green book gate only covers the 532 runnable snippets. Until
Wave-22A's 224-disabled triage lands, the safest global map should treat the
disabled surface as unclassified current risk and avoid inferring implementation
truth from historical 245-snippet buckets except as a planning prior.

## Next Evidence Lanes

1. Implement and prove schema-backed state carriers before widening state book
   examples. This is the only P0 gap with public API names that currently refuse
   at runtime.
2. Turn source-only proof buckets into semantic evidence by adding targeted
   runtime/Miri probes for the riskiest covered helpers: typed-object field
   mutation, snapshot/wire restore, JIT FFI returns, and trait-object carriers.
3. Add a non-skipping distributed/polyglot proof lane with built extension
   artifacts and one combined TLS + async + receiver-store snapshot + resume
   path.
4. Promote object/trait JIT surfaces only behind VM/JIT differential tests and
   explicit fallback assertions.
5. Treat disabled-book count reduction as a separate completeness project:
   first reclassify the current 224, then flip only deterministic stale-green or
   newly implemented snippets through the full book gate.
