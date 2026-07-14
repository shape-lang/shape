# Wave 20 Current State

Date: 2026-07-09
Supervisor: book-truth completeness campaign

## Book Truth

The current book truth target is no longer the original 394-disabled state. The
current manifest has:

| Metric | Count |
|---|---:|
| Total snippets | 756 |
| Runnable snippets | 511 |
| Disabled snippets | 245 |
| Last full release-binary gate | 511 / 511 passed |

Evidence:

- Manifest:
  `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.
- Last full release-binary book gate:
  `run-p3804871-i30223682.service`, report
  `/tmp/shape-wave20-book-truth-report.json`.
- Current disabled reprobe:
  `run-p3528381-i29935582.service`, report
  `/tmp/shape-wave19-disabled-probe.json`.

All `runnable=true` examples are currently book-true: VM and JIT pass without
output divergence in the last full gate. Disabled examples are split between
real implementation gaps, fixture/manual examples, preview/proof material,
stale-green review candidates, old syntax rewrites, and intentional diagnostics.

## Disabled State

Current disabled primary buckets:

| Bucket | Count |
|---|---:|
| Active implementation gaps | 81 |
| External/manual/fixture-needed | 82 |
| Preview/out-of-scope | 27 |
| Proof/design gaps | 23 |
| Stale-green review candidates | 13 |
| Old syntax/book rewrites | 9 |
| Intentional diagnostics | 10 |

Top disabled pages:

| Page | Disabled |
|---|---:|
| `stdlib/native/io.mdx` | 25 |
| `stdlib/core/state.mdx` | 16 |
| `advanced/ownership-deep-dive.mdx` | 10 |
| `advanced/content-addressed-bytecode.mdx` | 9 |
| `advanced/security-permissions.mdx` | 9 |
| `stdlib/native/http.mdx` | 9 |
| `fundamentals/content.mdx` | 8 |
| `advanced/transport-layer.mdx` | 7 |
| `stdlib/core/remote.mdx` | 7 |
| `fundamentals/error-handling.mdx` | 6 |
| `fundamentals/traits.mdx` | 6 |
| `tooling/python-extension.mdx` | 6 |

Detailed disabled triage is in
`docs/cluster-audits/wave19-disabled-current-triage.md`.

## Working Surface

- Strict VM/JIT book corpus: 511 runnable snippets pass.
- Typed `Option<T>` field mutation: focused VM coverage passes for `Some`,
  `None`, nested mutation, payload readback, invalid payload rejection,
  reference rejection, and self-cycle smoke.
- Distributed basics: remote call, TLS `shape+tls://...` calls, explicit
  snapshot store selection, receiver-side snapshot/resume, extern-C receiver
  snapshot/resume, and dynamic Python/TypeScript receiver transfer/resume have
  focused evidence.
- Distributed composition: TLS serve plus selected receiver snapshot store,
  Python/TypeScript transfer, snapshot hash visibility, and local resume have
  focused e2e coverage.
- Distributed matrix: TLS missing-CA refusal, TLS server-name mismatch refusal,
  plaintext/TLS receiver-store isolation for remote `snapshot()`, and
  Python/TypeScript runtime opt-in refusal have non-ignored tests.
- Async: static `Future<T>`, `await`, native module futures, `remote::call_async`,
  live-future snapshot refusal, local cancellation, `join race` / `join any`,
  and now ordered value materialization for `await join all { ... }` are covered.
- Comptime: blocks, `comptime fn`, tested annotations, directive transforms,
  `set param` metadata/defaults, bounded scalar `set param value`, and
  `replace module` re-analysis are usable.
- State: `state.caller()` is real; `state.args()` and `state.locals()` are real
  for bounded scalar/string cases; scalar serialize/deserialize and
  `state.fn_hash` are wired. `state.capture()` now returns bounded real
  `FrameState` metadata (`function_name`, `blob_hash`, `ip`, and frame counts)
  instead of fabricated locals/args/upvalues.
- JIT heap-field mutation: `17_jit_heap_field_overwrite` and
  `20_jit_heap_field_overwrite_function` now run native-JIT in both shipped
  `jit+gc` and `jit`/GC-off artifacts, exercising `jit_typed_object_set_field`
  and the GC write-barrier path.
- Content arrays: `Content.fragment([...])` now works for direct,
  identifier-backed, chained-builder, mixed-node, and explicit `Array<content>`
  inputs.
- Sibling `shape-app`: the playground/notebook server now reaches a GC-enabled
  `shape-vm` through coherent local Shape crate paths.
- Proof guards: ignored-test classifier, typed-opcode proof coverage
  (`unproven_gap=0`), no-dynamic, and targeted Miri/provenance scripts are clean
  at the current supervised checkpoints.

## Really Missing

1. Direct field-read native-JIT path.
   VM typed-field mutation works and heap typed-field overwrite reaches native
   JIT, but direct `Place::Field` value reads still deopt to avoid known
   object/trait crashes and divergences. Top-level field writes remain native.

2. State/resume completeness.
   `state.capture()` is real bounded metadata, but `capture_all`, public
   `state.resume`, full heap/Any args+locals, `resume_frame`, `diff`, `patch`,
   and content-addressed bytecode composition are not complete.

3. Real async beyond first fan-in.
   `join all` values are now real for homogeneous carriers, but native async
   signatures, user coroutine suspension, remote cancellation, remote functions
   returning futures, pending-future snapshot/resume, streams, and JIT async
   lowering remain incomplete.

4. Distributed/snapshot/polyglot proof breadth.
   Core paths are real and deeply tested in slices, but the complete matrix
   still needs more combined extern-C/dynamic/runtime-refusal/snapshot/async/
   permission/content-addressed cases.

5. Comptime ergonomics/type-safety.
   The substrate is typed, but authoring still relies on directive/string
   payloads for important code-generation paths. Typed fragments/quasiquote,
   typed reflection metadata, and hygiene are the real missing pieces.

6. Containers and math carriers.
   Public `Table<T>`, public `Mat<number>` construction, optimizer typed-array
   migration, stochastic/statistical carriers, and some domain modules remain
   active gaps.

7. Traits/conversion/testing/property testing.
   User trait/impl source currently deopts under JIT to avoid observed
   VM/JIT divergence in ordinary trait method calls. Named impl dispatch,
   generic trait args, associated types, `From`/`TryFrom`/`Into`/`TryInto`,
   testing Result helpers, and property-testing schemas remain shared blockers.

8. Native/manual fixtures.
   IO/file/env/http/archive/native-C/Python/TypeScript examples need controlled
   fixtures or explicit manual lanes. They should not be flipped into the
   default book gate as-is.

9. Global proof gaps.
   Current proof is useful but not global: Miri is targeted, source guards are
   not semantic proof, and distributed/snapshot/polyglot plus GC/JIT mutation
   boundaries still need broader evidence.

## Active Next Step

Wave-20A and Wave-20B are verified. The next count-reduction lane should choose
between direct field-read JIT proof/fix, state `capture_all`/`resume` carriers,
or the next disabled-book implementation bucket. The strategic proof gap is now
direct field reads and trait dispatch, not GC write-barrier overhead.
