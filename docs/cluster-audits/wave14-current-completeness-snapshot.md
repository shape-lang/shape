# Wave 18 Current Completeness Snapshot

Date: 2026-07-09
Supervisor: book-truth completeness campaign

## Current Book Truth

Authoritative current manifest:
`/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`

Current release-binary gate:

| Metric | Count |
|---|---:|
| Total snippets | 756 |
| Runnable snippets | 511 |
| Disabled snippets | 245 |
| Gate pass | 511 / 511 |
| VM/JIT failures | 0 |
| VM/JIT output divergences | 0 |
| Timeouts | 0 |

Evidence:

- Failed over-flip gate: `run-p2567603-i28950569.service`, 506 / 521 passed,
  15 both-fail.
- Corrected Wave-13 release gate: `run-p2643441-i29027122.service`, 507 / 507
  passed, report `/tmp/shape-wave13-book-truth-report.json`.
- Wave-14 release gate after typed-field and distributed-composition changes:
  `run-p2795815-i29182090.service`, 507 / 507 passed, report
  `/tmp/shape-wave14-book-truth-report.json`.
- Wave-15 release rebuild after async/comptime/Miri changes:
  `run-p2936760-i29327417.service`, release build passed.
- Wave-15 release-binary book gate: `run-p2941815-i29332891.service`,
  507 / 507 passed, report `/tmp/shape-wave15-book-truth-report.json`.
- Wave-16 release rebuild after state-caller, distributed-matrix, JIT perf
  probe, and sibling shape-app GC work: `run-p3101831-i29498078.service`,
  release build passed.
- Wave-16 release-binary book gate: `run-p3106903-i29503586.service`,
  507 / 507 passed, report `/tmp/shape-wave16-book-truth-report.json`.
- Wave-17 release rebuild after state args/locals, distributed dynamic-runtime
  refusal rows, JIT scalar mutation deopt work, and content/container parity
  scope: `run-p3328173-i29730226.service`, release build passed.
- Wave-17 release-binary book gate: `run-p3330649-i29732840.service`,
  507 / 507 passed, report `/tmp/shape-wave17-book-truth-report.json`.
- Wave-18 release rebuild after content typed-array parity:
  `run-p3441201-i29846808.service`, release build passed.
- Wave-18 extraction after book flips:
  `run-p3446590-i29852637.service`, 756 total / 511 runnable / 245 disabled.
- Wave-18 release-binary book gate: `run-p3446781-i29852843.service`,
  511 / 511 passed, report `/tmp/shape-wave18-book-truth-report.json`.
- Post-Wave-18 static guards passed: main `diff --check`, sibling
  `../shape-web` `diff --check`, ignored-test classifier, `check-no-dynamic`,
  typed-opcode proof checker with `unproven_gap=0`, and
  `bash -n scripts/check-miri-provenance.sh`.

Interpretation: every `runnable=true` book snippet currently works on both VM
and JIT according to the gate. Disabled snippets are not part of the default
truth set and need either implementation, a deterministic harness, or an
explicit manual/external lane.

## Largest Disabled Clusters

Top pages by disabled snippets in the corrected 245-disabled manifest:

| Page | Disabled | Current read |
|---|---:|---|
| `stdlib/native/io.mdx` | 25 | Mostly external filesystem/network/process/stdin; `read_file_async` remains active async/native work. |
| `stdlib/core/state.mdx` | 16 | Capture/resume/diff/introspection remain core implementation gaps. |
| `advanced/ownership-deep-dive.mdx` | 10 | Some stale candidates remain; explicit storage classes and reference edges are not complete. |
| `advanced/content-addressed-bytecode.mdx` | 9 | Needs state capture/resume/diff plus transport/store composition proof. |
| `advanced/security-permissions.mdx` | 9 | Mostly host/security proof sketches and policy examples. |
| `stdlib/native/http.mdx` | 9 | Live network examples and async shape need a controlled integration lane. |
| `fundamentals/content.mdx` | 8 | Content array fragments now work; remaining disabled snippets are old syntax, preview content traits, and table/container APIs. |
| `advanced/transport-layer.mdx` | 7 | Needs loopback transport proof; QUIC remains gated. |
| `stdlib/core/remote.mdx` | 7 | Serve fixtures and stale internal remote-call examples remain. |
| `fundamentals/traits.mdx` | 6 | Named impl dispatch, conversions, generic trait args, associated types. |
| `fundamentals/datetime.mdx` | 4 | Import/literal smokes are now runnable; DateTime method examples still need JIT parity. |

Wave-13 corrected the earlier "21 deterministic stale flips" estimate. Only 7
net snippets were truthfully added. The over-flipped snippets exposed real
active gaps:

- pending statistical/stochastic intrinsic kinded-carrier migrations;
- public `Mat<number>` construction not available to user snippets;
- optimizer internals still hitting strict typed-array construction surfaces;
- physics stdlib module still rejected by strict inference;
- RK45 only became runnable after explicit closure and result-array typing.

## Working Surface

The following are currently backed by source and/or gate evidence:

- Strict default VM/JIT book corpus: 511 runnable snippets pass VM and JIT with
  no output divergence.
- Core strict proof guards: typed-opcode source inventory has
  `unproven_gap=0`; `check-no-dynamic` is clean.
- Option/Result and typed carriers are broadly exercised by prior differential
  gates and book examples.
- Typed `Option<T>` field mutation now works for `Some`, `None`, nested
  mutation, payload readback, invalid payload rejection, reference rejection,
  and a self-cycle smoke. Focused gate: `run-p2768919-i29153884.service`,
  8 / 8 passed.
- Distributed basics now work: remote call, explicit snapshot store selection,
  receiver-side snapshot/resume, extern-C receiver snapshot/resume, TLS
  `shape+tls://...` user remote calls, and dynamic Python/TypeScript receiver
  transfer/resume have focused evidence.
- Distributed composition now has a focused TLS + selected receiver snapshot
  store + dynamic Python/TypeScript transfer + local resume proof. Focused
  gate: `run-p2779628-i29164796.service`, 2 / 2 passed.
- Distributed proof matrix now includes non-ignored TLS missing-CA refusal,
  TLS server-name mismatch refusal, plaintext receiver-store isolation for
  remote `snapshot()`, and TLS receiver-store isolation for remote `snapshot()`.
  Focused gate: `run-p3027910-i29420591.service`, 4 / 4 passed; adjacent
  composition regression gate `run-p3030561-i29423319.service`, 2 / 2 passed.
- Async baseline works: `Future<T>` static handles, `await` unwrapping,
  `remote::call_async`, distributed async composition, and explicit
  snapshot-time rejection of unresolved `Future(id)` handles have focused
  evidence.
- Comptime baseline works for tested blocks/functions and annotation transform
  paths. `set param` default metadata, bounded scalar `set param value`, and
  `replace module` re-analysis are now verified, but typed fragments,
  reflection, and hygiene remain open.
- State introspection now has a real `state.caller()` surface for caller
  metadata with a `FunctionRef { name, hash }` shape, plus no-caller and
  missing-hash diagnostics. Focused gate: `run-p3025719-i29418276.service`,
  3 / 3 passed; broader state-builtin module filter
  `run-p3027391-i29420022.service`, 16 / 0 / 7 ignored.
- State `args` / `locals` now read real VM state for bounded scalar cases:
  `state.args()` returns homogeneous `Array<int>`, `Array<number>`, or
  `Array<string>`; `state.locals()` returns `HashMap<string,string>` for
  string-carrier locals. Unsupported `Any`, heterogeneous, empty, bool/null,
  and heap cases surface structured diagnostics instead of fabricated data.
  Focused gate: `run-p3213535-i29611638.service`, 20 / 0 / 5 ignored.
- Content array parity now works for direct, identifier-backed,
  chained-builder, mixed-node, and explicit `Array<content>`
  `Content.fragment([...])` inputs. Focused gate:
  `run-p3439111-i29844594.service`, 6 / 0 passed; the full book gate moved
  two content fragment snippets into the runnable set.
- Deterministic DateTime literal/duration snippets now run in both VM and JIT
  under the book gate. Method-heavy DateTime examples remain disabled until JIT
  parity is fixed.
- Many native modules are now deterministic-book-gated: crypto, compress,
  msgpack, yaml, toml, xml, csv/json partials, native math partials, set,
  collections, resource management, enums, strings, modules, annotations,
  content/datetime partials, and local snapshot/state scalar pieces.
- The sibling `shape-app` playground/notebook server now reaches a GC-enabled
  `shape-vm` through coherent local Shape crate paths. Verification:
  `run-p3096571-i29492431.service` passed `cargo tree -p shape-server -e
  features -i shape-vm` with `shape-vm/gc` active, and
  `run-p3096790-i29492670.service` passed `cargo check -p shape-server`.

## Really Missing

These are the real blockers, not just stale docs:

1. Typed field mutation follow-through.
   Natural optional typed-object field mutation is now closed for the focused
   cases, including `a.peer = Some(a)` self-cycle smoke. Remaining follow-up is
   proof breadth: GC barrier/debug coverage for typed-object overwrites and
   snapshot/restore coverage for mutated typed objects.

2. State/capture/resume completeness.
   `state::caller()` is now real. `state::args()` and `state::locals()` are
   real for bounded scalar cases only. Public `state::capture*`,
   `state::resume`, full `Any`/heap-shaped args+locals, `resume_frame`, `diff`,
   `patch`, and content-addressed bytecode composition remain incomplete or
   proof-only. This is core to resumability and distributed computing.

3. Distributed/snapshot/polyglot deep composition.
   Wave-14 adds one composed e2e proof for TLS serve, selected receiver
   snapshot store, dynamic Python/TypeScript transfer, and local resume.
   Wave-16 adds TLS refusal rows and receiver-store isolation rows for
   plaintext and TLS remote `snapshot()`. Wave-17 adds non-skipping Python and
   TypeScript dynamic-runtime receiver opt-in refusal rows. The broader proof
   matrix still needs extern-C, async, permission failures, content-addressed
   payloads, and deterministic failure modes in one coherent matrix.

4. Real async.
   Current async is no longer just syntax, and live unresolved futures now fail
   snapshot capture with a clear diagnostic instead of silently persisting an
   unresumable handle. Remaining gaps include native async module signatures,
   join value materialization, cancellation/error propagation, snapshot/resume
   of pending async work, distributed future completion semantics, and real JIT
   async lowering. Wave-18 scoped the next bounded lane as ordered value
   materialization for `await join all { ... }`, including distributed
   `remote::call_async` fan-out/fan-in arrays.

5. Comptime typed ergonomics.
   Current comptime still leans on string payloads/directives in important
   authoring paths. The first ergonomics lane is closed for `set param` public
   metadata/defaults and `replace module` re-analysis. Typed
   metadata/reflection, typed fragments/quasiquote, richer default payloads,
   and hygiene still need a real design and implementation lane.

6. Typed-array, `Mat`, and intrinsic kinded-carrier migrations.
   These block current math/stochastic/statistical/optimizer/interpolation
   snippets and table/container examples. The first content-array slice is
   closed; public `Table<T>` and `Mat<number>` construction remain separate
   gaps.

7. Trait/conversion/testing/property gaps.
   Convert/TryFrom, named impl dispatch, generic trait args, associated types,
   testing Result assertions, and property-testing function-field schemas keep
   several language/book pages disabled.

8. JIT parity gaps.
   Book runnable snippets pass because JIT either compiles or falls back
   successfully, but DateTime and some array/typed-object paths still need real
   JIT support rather than interpreter fallback or disabled examples. Wave-16
   measured the compute-bound native-JIT GC/barrier control and found no
   meaningful delta. Wave-17 made scalar typed-object field mutation native-JIT
   eligible and measured no meaningful default-vs-barrier-off delta
   (`+0.02%`). Mutation-heavy heap typed-object field overwrite still deopts
   before it can measure a native-JIT write-barrier fast path. Wave-18 scoped
   that blocker to JIT MIR `FieldProjectionAssign` preflight after assignment
   lowering materializes `Copy(target_place)`, plus missing schema-backed JIT
   `__Option.Some/None` construction and barrier-correct dynamic option stores.

9. External/manual examples.
   Native IO, HTTP, file/env/time, extension packaging, frontmatter, and live
   remote-server examples need deterministic fixtures or opt-in integration
   lanes. They should not be flipped into the default book gate as-is.

10. Global proof gaps.
    Targeted Miri and source guards are useful but not global proofs. Wave-15
    expands targeted evidence for TypedArray carriers, TraitObject raw
    carriers, and typed-array stack provenance, but remaining proof work still
    includes snapshot/wire restore provenance, nested typed-array children,
    GC/JIT barrier performance and mutation evidence, semantic gates replacing
    source-only checks, and distributed/snapshot/polyglot proof boundaries.

## Wave 14/15 Findings

Async:

- Working: static `Future<T>`, `await Future<T>` unwrapping, native async module
  return futures for current modules, `remote::call_async`, current
  `join all` / `race` / `any` / `settle` handles, async-scope cancellation of
  local pending tasks, and explicit VM-only JIT fallback for async opcodes.
- Missing: context-aware native async signatures, argument-bearing user async
  continuations, shared-heap async returns, nonblocking continuation
  scheduling, value-materializing join results, distributed cancellation,
  remote functions returning futures, snapshot/resume of pending futures, and
  real async JIT lowering.
- Recommended first lane: make pending futures explicit and snapshot-safe.
  Add scheduler introspection and reject unresolved `Future(id)` values during
  snapshot capture with a clear diagnostic, then add distributed async
  composition tests.
- Wave-15 result: first lane closed. Focused cgroups passed distributed async
  5 / 0 in `run-p2880808-i29268412.service`, `task_scheduler` 9 / 0 in
  `run-p2884092-i29271759.service`, and snapshot 34 / 0 in
  `run-p2888496-i29276314.service`.

Comptime:

- Working: `comptime {}`, scalar/array values, conditionals, warnings/errors,
  build config, `comptime fn`, typed annotation `target` / `ctx`, `set return`,
  `replace body`, `extend target`, `extend (...)`, and basic `type_info`.
- Missing: `set param` public metadata refresh after directive mutation,
  non-int `set param value` support, `replace module` generated-code
  re-analysis, typed reflection instead of string/`Any` metadata, typed
  fragments/quasiquote, and hygiene.
- Recommended first lane: refresh function signature metadata after directive
  mutation (`function_defs`, `function_arity_bounds`, and related metadata),
  add strict module re-analysis after `replace module`, and keep string payload
  compatibility until a separate typed-fragment lane exists.
- Wave-15 result: first lane closed. Focused cgroups passed `comptime ct_45`
  3 / 0 in `run-p2908089-i29297230.service` and `annotations_comptime` 46 / 0
  in `run-p2909522-i29298706.service`.

Proof:

- Highest proof gap: targeted Miri is not a global no-UB proof. Next useful
  probes should cover TypedArray carriers, TraitObject internals,
  snapshot/wire restore, and JIT/FFI boundary values.
- Source-only guards are regression fences, not semantic proofs. The
  typed-opcode inventory, ignored-test classifier, and no-dynamic checker need
  runtime semantic companions.
- Distributed/snapshot/polyglot has good e2e evidence, but needs a complete
  non-skipping proof matrix with extensions built first.
- GC/JIT barrier proof and perf remain open for mutation-heavy native-JIT
  paths. A barrier-kind audit should map every JIT write lowering to a concrete
  barrier kind or prove exclusion by construction.
- Wave-15 result: targeted Miri expansion passed in
  `run-p2911284-i29300601.service` (13m52s, peak 2.8G/swap 0) for TypedArray
  field carrier clone/drop, TraitObject raw carrier clone/drop, typed-array
  stack provenance read/pop/drop, and the existing provenance probes. This is
  still not a global no-UB proof.
- Wave-16 result: default `jit + gc` and comparator
  `--no-default-features --features jit` release binaries both built in
  isolated target dirs. `06_ackermann` stayed native JIT and showed no
  meaningful compute-bound delta (`0.27s` median default vs `0.28s` comparator,
  `run-p3092390-i29487951.service`). The new
  `17_jit_heap_field_overwrite` and `18_jit_scalar_field_overwrite` probes both
  execute correctly but fall back in both variants on the existing JIT
  move-semantics surface, so native-JIT mutation barrier cost remains
  unmeasured. Detailed report:
  `docs/cluster-audits/wave16-jit-gc-barrier-perf-results.md`.

## Shape Completeness

Current status: Shape has a substantial verified strict core, but it is not
complete yet.

The current honest claim is:

- The shipped release binary can execute the 511 runnable book examples under
  VM and JIT with no failures or divergences.
- The disabled 245 examples are no longer mostly stale drift; they are a mix of
  implementation gaps, proof/harness gaps, external/manual examples, old syntax
  rewrites, and preview material.
- The biggest completeness blockers are now state/resume, broader distributed
  proof matrix, real async, typed comptime ergonomics, typed-array/Mat/intrinsic
  migrations, trait/conversion/testing gaps, JIT parity, external/manual
  fixtures, and the remaining global proof gaps.
