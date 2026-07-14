# Wave 17 Disabled Action Map

Date: 2026-07-09
Source: `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`

## Current Counts

| Metric | Count |
|---|---:|
| Total snippets | 756 |
| Runnable snippets | 511 |
| Disabled snippets | 245 |

This map is intentionally operational. It does not replace per-snippet
root-cause triage, but it turns the current disabled corpus into dispatchable
waves.

## Dispatch Principle

Raw disabled count is not the same as product completeness.

The largest page by count is native IO, but the highest strategic gaps are
state/resume, distributed/snapshot/polyglot composition, real async,
typed comptime ergonomics, typed-array/container parity, JIT parity, and proof
coverage. Wave 17 therefore splits work across both:

- core completeness lanes that make the language/runtime story more true;
- count-reduction lanes where deterministic harnesses can safely flip snippets.

## Top Disabled Pages

| Page | Disabled | First honest lane |
|---|---:|---|
| `stdlib/native/io.mdx` | 25 | deterministic filesystem/process/stdin fixtures; async IO remains real async/native work |
| `stdlib/core/state.mdx` | 16 | state introspection, then capture/resume/diff/patch |
| `advanced/ownership-deep-dive.mdx` | 10 | storage-class/reference semantics and intentional diagnostic snippets |
| `advanced/content-addressed-bytecode.mdx` | 9 | state/resume plus content-addressed composition |
| `advanced/security-permissions.mdx` | 9 | security policy proof/harness snippets |
| `stdlib/native/http.mdx` | 9 | controlled HTTP fixture plus real async/native HTTP surface |
| `fundamentals/content.mdx` | 8 | remaining old syntax, preview content traits, and table/container APIs |
| `advanced/transport-layer.mdx` | 7 | distributed transport proof matrix |
| `stdlib/core/remote.mdx` | 7 | remote harness, stale annotation syntax, remote execute surface |
| `fundamentals/error-handling.mdx` | 6 | Convert/TryFrom/AnyError surface |
| `fundamentals/traits.mdx` | 6 | trait dispatch, generics, associated types, conversion traits |
| `tooling/python-extension.mdx` | 6 | extension packaging harness and async foreign calls |
| `fundamentals/datetime.mdx` | 4 | DateTime method JIT parity |
| `fundamentals/tables.mdx` | 5 | public `Table<T>` construction/query API |
| `stdlib/native/archive.mdx` | 5 | archive create/extract carrier migration or deterministic fixture |
| `stdlib/native/file.mdx` | 5 | deterministic filesystem fixture policy |
| `tooling/typescript-extension.mdx` | 5 | extension packaging harness and async foreign calls |

## Wave 17 Active Lanes

| Lane | Why now | Expected movement |
|---|---|---|
| State `args` / `locals` | `state.caller()` closed the first introspection slice; `args` and `locals` are the next visible disabled APIs. | Retire two more state placeholders if current return marshaling can support scalar data, or identify the exact `Any` container boundary. |
| JIT mutation deopt | Native-JIT write-barrier perf cannot be honestly measured while field-overwrite probes deopt before mutation code. | Either make the Wave-16 mutation probes native-JIT eligible or produce the exact implementation map for per-point move/copy/clone liveness. |
| Distributed dynamic-runtime refusal | Distributed/snapshot/polyglot is a core feature; Wave-16 added TLS/store rows but left dynamic-runtime refusal rows. | Add non-skipping proof rows or identify the extension setup gap that prevents them. |
| Content/container parity | Content, tables, `Mat<number>`, optimizer, and interpolation are a visible cluster of typed-array/container gaps. | Produce the first implementation wave with exact file ownership and safe flip candidates. |

Wave 17 outcomes so far:

- `state.args()` and `state.locals()` now read real VM state for bounded scalar
  cases; unsupported `Any`, heterogeneous, heap, bool/null, and empty cases
  surface structured diagnostics instead of fabricated data. Verification:
  `run-p3213535-i29611638.service`, 20 / 0 / 5 ignored.
- Distributed dynamic-runtime refusal now has non-ignored Python and TypeScript
  receiver opt-in refusal rows in `distributed_matrix_e2e`. Verification:
  `run-p3210328-i29608325.service`, 6 / 0.
- JIT scalar typed-object field mutation now reaches native JIT in both the
  default and JIT-barrier-off artifacts. Verification:
  `run-p3403181-i29805881.service`; timing:
  `run-p3403599-i29806316.service`.
- JIT heap typed-object field mutation still falls back on the ADR-006 section
  2.7.14 field-projection/move-semantics surface before the write barrier can
  be timed. That is now the exact remaining barrier-perf blocker.
- Content/container parity's first implementation lane is now closed for
  `Array<content>`: direct, identifier-backed, chained-builder, mixed-node,
  and explicit `Array<content>` `Content.fragment([...])` inputs work.
  Verification: `run-p3439111-i29844594.service`, 6 / 0; release book gate:
  `run-p3446781-i29852843.service`, 511 / 511.

Wave 18 outcomes:

- Four additional snippets moved into the truthful runnable set: two
  `Content.fragment([...])` examples and two deterministic DateTime
  literal/duration examples.
- Current extracted manifest is 756 total / 511 runnable / 245 disabled.
- JIT heap typed-object mutation barrier timing is still blocked before the
  barrier by `FieldProjectionAssign` preflight; see
  `wave18-jit-heap-barrier-blocker.md`.
- The next real-async/distributed lane is ordered value materialization for
  `await join all { ... }`, including distributed `remote::call_async`
  fan-out/fan-in arrays; see
  `wave18-real-async-distributed-next-lane.md`.

## Empirical Disabled Probe

Supervisor probe:

- service: `run-p3185744-i29583417.service`;
- command shape: release binary over all currently disabled snippets in both
  `--mode vm` and `--mode jit`;
- report: `/tmp/shape-wave17-disabled-probe.json`.

This probe predates the four Wave-18 flips. The DateTime L98/L106 candidates
and the two content-fragment candidates have since been verified and moved into
the runnable corpus; the probe should be regenerated before using its exact
class counts for the next dispatch.

| Probe class | Count | Read |
|---|---:|---|
| `both-fail` | 230 | Real implementation, syntax, external/manual, or pseudo-code blockers. |
| `disabled-but-green` | 15 | Review candidates only; several are comments/type declarations/external examples that happen to exit 0. |
| `jit-only-fail` | 3 | Current DateTime JIT-parity blockers. |
| `output-divergence` | 1 | Snapshot hash output is intentionally content/run dependent and needs an expected-output strategy before becoming a default runnable snippet. |

Disabled-but-green snippets that need page-level review:

| Snippet | Current read |
|---|---|
| `advanced/resumability.mdx:L105` | Function definition using `snapshot()` but never invoked; green because it defines only. Keep disabled unless rewritten as a deterministic runnable example. |
| `advanced/security-permissions.mdx:L162` | Comment-only permission mapping. Keep disabled or convert to prose. |
| `fundamentals/content.mdx:L47` | Trait declaration only. Needs content trait semantics decision; not a runtime example. |
| `fundamentals/content.mdx:L55` | `impl Content for Score`; green as definition-only. Needs trait/content behavior tests before flip. |
| `fundamentals/content.mdx:L484` | Type declaration only. Keep disabled or convert to prose. |
| `fundamentals/datetime.mdx:L12` | Import-only. Safe only if the book wants import smokes. |
| `fundamentals/datetime.mdx:L98` | Real deterministic DateTime method example; strong stale-flip candidate. |
| `fundamentals/datetime.mdx:L106` | Expression-only date arithmetic; needs visible assertion/print before flip. |
| `fundamentals/functions.mdx:L185` | Type-illustration with trait bounds; not a useful runtime example without `Display` impls/call. |
| `fundamentals/functions.mdx:L421` | Python extension example; green without invocation. Keep disabled until extension fixture exists. |
| `fundamentals/traits.mdx:L387` | Trait associated-type definition; green as definition-only but associated type behavior remains a trait gap. |
| `getting-started/basic-concepts.mdx:L149` | Type declaration plus comments. Keep disabled or convert to prose. |
| `stdlib/native/io.mdx:L449` | Real `io::exec("git", ...)` example, but host/tooling dependent. Keep out of default gate unless rewritten to a deterministic fixture command. |
| `tooling/python-extension.mdx:L163` | Async Python extension definition only. Keep disabled until extension fixture exists. |
| `tooling/typescript-extension.mdx:L180` | Async TypeScript extension definition only. Keep disabled until extension fixture exists. |

Current non-green parity blockers from the disabled set:

| Snippet | Probe class | Current read |
|---|---|---|
| `advanced/resumability.mdx:L21` | `output-divergence` | VM and JIT both save a snapshot but print different hashes. Needs expected-pattern support or a deterministic rewrite. |
| `fundamentals/datetime.mdx:L23` | `jit-only-fail` | VM prints `true true`; JIT path is not truthfully runnable. |
| `fundamentals/datetime.mdx:L367` | `jit-only-fail` | VM prints `true true`; JIT fails. |
| `fundamentals/datetime.mdx:L407` | `jit-only-fail` | VM prints six true lines; JIT fails. |

## Next Count-Reduction Waves

These are useful after the active Wave-17 core lanes are integrated:

1. Native file/env/IO deterministic harness lane.
   Convert pure filesystem/env examples to isolated temp fixtures and keep live
   stdin/process/network snippets disabled or manual.

2. Archive lane.
   Determine whether `archive::zip_create`, `zip_extract`, `tar_create`, and
   `tar_extract` fail from carrier migration, missing public constructors, or
   stale book syntax; then either implement the carrier arm or rewrite to a
   deterministic current API.

3. Testing/property lane.
   `assert_eq` / `assert_ne` are blocked by imported generic call-site
   inference; `assert_ok` / `assert_err` are blocked by Result method dispatch
   in the book smoke path. Property testing needs function-field schemas.

4. Traits/conversion lane.
   `From`, `TryFrom`, `Into`, `TryInto`, named impl dispatch, generic trait
   args, and associated types block both `traits.mdx` and
   `error-handling.mdx`.

5. DateTime JIT parity lane.
   Several DateTime examples are VM-safe but disabled because the page cannot
   truthfully promise VM/JIT parity yet.

## What Not To Flip Blindly

- Live HTTP examples against `api.example.com`.
- Extension examples that require Python/TypeScript toolchains without a
  packaged extension fixture.
- Intentional diagnostics such as use-after-move and escaping-reference
  examples; those belong in negative tests or explicitly disabled book fences.
- Pseudo-code with `...`, non-existent application modules, external services,
  or host-specific paths.
- Content-addressed examples that depend on unimplemented state capture/resume,
  diff, or patch APIs.

## Supervisor Read

The next honest progress should not be a single large "flip stale docs" wave.
The campaign needs alternating waves:

- implementation/proof waves for state, distributed, async, comptime, JIT, and
  typed containers;
- deterministic harness waves for native IO, archive, file/env, HTTP, and
  extension docs;
- book correction waves for intentional diagnostics and pseudo-code that should
  stay disabled with precise explanations.
