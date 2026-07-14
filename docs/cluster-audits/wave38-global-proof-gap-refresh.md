# Wave 38 Global Proof-Gap Refresh

Date: 2026-07-10
Role: Wave-38E global proof-gap current refresh scout

## Scope

Static investigation only. I read current AGENTS evidence, the Wave-33 global
proof refresh, Wave-34 typed-field proof close, Wave-36/Wave-38 focused scouts,
proof scripts, and selected source/test surfaces. I did not edit production
code or `AGENTS.md`. I did not run cargo, just, nextest, rustc, build, tests,
Miri, benchmarks, extraction, or book-truth commands.

This is the only file written by this scout.

## Current Evidence Baseline

The proof story is stronger than Wave 33, but it remains a set of targeted
semantic proofs rather than a global proof of the VM/runtime/JIT/distributed
surface.

Source-only guards:

- `scripts/check-typed-opcode-proof-coverage.py` still intentionally scans
  compiler Rust sources only. Its expected `unproven_gap` count is zero, but the
  script states that it does not run cargo, rustc, nextest, or Miri.
- `scripts/check-ignored-test-classification.py` guards source-level
  `#[ignore]` reasons and counts. It classifies ignored tests; it does not
  execute them.
- `scripts/check-no-dynamic.sh` prevents forbidden dynamic-pattern count
  regressions against `docs/check-no-dynamic-baseline.txt`; it is a source grep
  guard over `crates`, `bin`, `tools`, and `extensions`.

Miri/provenance:

- `scripts/check-miri-provenance.sh` now covers provenance anchors, nested
  typed-object clone/drop, typed-object field overwrite, typed-array field
  clone/drop, trait-object raw carrier clone/drop, Result/Option carriers,
  SetFieldTyped Option overwrite, typed-object raw reads, and stack provenance.
- The script still explicitly says a passing run is not a full UB proof for the
  VM, runtime, JIT, FFI, snapshots, arbitrary Shape programs, all stack
  overwrite sites, all typed-object field kinds/producers, heap-element arrays,
  arbitrary trait dispatch, or snapshot/wire restore.

Runtime and book truth:

- Wave-37A/38A moved the current book baseline to 707 total snippets, 559
  runnable, 148 disabled, 8 expected-output, 6 expected-fail, and 8 fixture
  snippets. AGENTS records the full release-binary book gate passing 559/559.
- Book truth proves the current runnable surface only. Wave-38A still counts 68
  active missing-feature disabled rows, 41 external/manual/fixture-only rows,
  and 5 proof/design rows.
- Distributed CLI e2es now cover much more than the book surface, including
  selected receiver snapshot stores, TLS, extern-C transfer/resume, async
  fan-in, and required-extension Python/TypeScript lanes when enabled.

## Closed Or Materially Improved Since Wave 33

### Closed Narrow Proof Gap: Typed-Object Field Mutation

Wave 33 ranked typed-object field mutation first. Wave 34A closed that narrow
lane.

Evidence:

- `docs/cluster-audits/wave34-typed-field-mutation-proof-close.md` records the
  added storage/Miri, VM, JIT FFI, and ShapeTest probes.
- `scripts/check-miri-provenance.sh` now names
  `miri_write_slot_in_place_replaces_typed_object_field_and_preserves_metadata`
  and `set_field_typed_option_overwrite_preserves_canonical_carrier_metadata`.
- The storage path in `crates/shape-value/src/heap_value.rs` now includes the
  Miri sidecar companion
  `write_slot_in_place_with_miri_provenance`.
- The remaining boundary is explicit: this is not a global UB proof, not every
  heap field kind, not snapshot/wire restore, and not full JIT codegen.

Status: closed for the ordinary typed-object field mutation semantic bridge;
still targeted evidence, not global memory-safety proof.

### Improved: Async Snapshot-Safety And Distributed Cancellation

Wave-36B made live `Future(id)` handles fail closed during snapshot capture
instead of serializing a misleading resumable carrier. AGENTS records focused
future/snapshot tests, broader snapshot tests, and distributed async e2e
verification. Wave-35C made TLS `remote::call_async` cancellation parity
stronger and added ignored supervisor-lane cancellation coverage.

Remaining async proof gaps are still clear:

- actual pending-future snapshot/resume,
- remote callees returning `Future<T>`,
- streams and real `for await`,
- value-materializing `join settle`,
- JIT async lowering.

Wave-38C scoped the smallest remote-callee `Future<T>` lane as receiver-side
materialization before response serialization, not durable remote future
identity.

### Improved: Distributed/Snapshot/Polyglot Composition

Wave-37B closed the silent-skip risk for extension-backed Python/TypeScript
composition by adding a required-extension mode. AGENTS records default and
required-extension distributed snapshot/polyglot, dynamic snapshot, and
composition checks passing, plus a real TypeScript receiver fix.

Wave-38B is active and has added
`bin/shape-cli/tests/distributed_content_addressed_e2e.rs`, which exercises a
real-socket missing dependency/resupply path by stripping helper blobs from the
first request and resupplying the receiver-reported missing blobs. Because
AGENTS still marks Wave-38B active, this report treats that file as promising
current evidence, not a verified closed lane.

Remaining distributed proof gaps:

- `@remote` value/capture matrix is still not a fully truth-gated cargo/book
  matrix.
- Content-addressed resupply/cache still needs closeout verification and likely
  cache-persistence/negotiation coverage beyond the first real-socket slice.
- Dynamic extension book rows remain external/manual rather than ordinary book
  truth.
- Async cancellation proofs remain ignored supervisor-lane tests, not default
  tests.

### Improved: Comptime Type Safety

Wave-37C added the first typed additive `ItemFragment` slice. `extend (expr)`
can now accept the typed fragment path for a zero-arg generated function
returning a literal scalar/string, while preserving the source-string
compatibility path.

This materially reduces the stringly authoring boundary, but it does not close
comptime as a typed macro system:

- `replace module (expr)` and broader generated bodies still use source/JSON
  payloads.
- Annotation metadata still has stringly surfaces.
- Wave-37C exposed a residual JIT-only generated-method failure.
- Wave-38D scoped that residual to generated extension-method parity:
  generated methods likely miss `Function.mir_data` and JIT fallback can miss
  extend-style `Type.method` names.

### Improved: State/Resume And Book Fixtures

State carriers are no longer just stubs. Prior waves landed `capture_call`,
`capture_all`, `capture_module`, bounded diff/patch, caller/args/locals, and
local snapshot/resume fixtures. Wave-37A flipped the two stale local
snapshot/resume book rows with `fixture=local-snapshot-resume`.

Public resume remains a completeness gap:

- `state.resume(vm)` depends on callback wiring and a `VmState` carrier that is
  genuinely executable.
- `state.resume_frame(frame)` still refuses metadata-only `FrameState`.
- `crates/shape-vm/src/executor/resume.rs` documents the missing resume IP and
  structural call-frame fields for non-empty public frames.

## Ranked Remaining Global Proof Gaps

1. Snapshot/wire restore provenance.

   This is now the highest-value proof-only lane. It is explicitly outside the
   current Miri gate, but the runtime has rich snapshot/wire restore machinery:
   `SNAPSHOT_VERSION = 7` uses `HeapNode` / `HeapRef` for cycle-capable heap
   identity, `serializable_to_slot` restores typed objects, heap-element typed
   arrays, typed-object-valued maps, Result/Option normalization, and scalar
   arrays, and VM tests cover several round trips. What is missing is standing
   Miri/provenance evidence that restored heap carriers can be cloned/read/dropped
   safely after `slot_to_serializable` -> wire shape -> `serializable_to_slot`.

2. Public state resume and full state carriers.

   This is a larger product/API completeness gap rather than a clean first
   proof lane. The core machinery has improved, but public `VmState` and
   `FrameState` do not yet carry all executable fields, and callback wiring is
   not uniformly available through native-module dispatch. The right work here
   is implementation plus focused proof, not a report-only proof bridge.

3. Distributed/snapshot/polyglot proof breadth.

   The coverage is strong and much less hand-wavy than Wave 33. Still, the
   evidence is not global: extension rows need explicit required-extension lanes
   or fixtures, `@remote` value/capture matrix coverage is incomplete, and the
   new real-socket content-addressed resupply slice is active/unclosed in AGENTS.
   Wave-38B owns the most immediate slice, so the global next lane should avoid
   overlapping it.

4. Real async beyond current fan-in/cancellation.

   Pending future snapshot refusal is proven targeted behavior, not pending
   future resume. Remote-callee future materialization is scoped by Wave-38C.
   Durable remote future identity, streams, `join settle` values, and JIT async
   lowering remain separate proof/implementation lanes.

5. Comptime generated-method/JIT parity and typed generation breadth.

   The typed `ItemFragment` first slice is real, but full typed fragments,
   generated methods, method MIR/JIT parity, and migration of real stdlib
   generators remain open. Wave-38D already scoped the next focused JIT
   generated-method lane.

6. JIT/GC/FFI proof breadth.

   Typed-object mutation FFI/barrier evidence exists, and prior JIT heap
   barrier performance blockers are closed. The remaining gap is breadth: every
   native object/trait/container write and return path should either use the
   stamped-kind discipline or deopt/surface explicitly. Current Miri cannot
   execute the whole native JIT surface, so this remains targeted wrapper and
   runtime proof, not global.

7. Disabled-book completeness.

   The current 559/559 runnable book gate is meaningful, but the remaining 148
   disabled rows still hide implementation, fixture, preview, old-syntax, and
   proof/design gaps. This is a release/completeness signal, not a direct proof
   that disabled behavior is correct.

8. Source-only guard semantic companions.

   Source guards are valuable because they prevent regression and force
   classification. They should not be promoted to semantic proof unless the
   guarded paths also have runtime, Miri, or book evidence. This is a standing
   interpretation rule more than a single file-ownership lane.

## Recommended Next Proof Lane

Next lane: snapshot/wire restore Miri provenance bridge.

Why this lane:

- It is explicitly excluded by the current Miri gate.
- It is central to state/resume, distributed transfer, content-addressed resume,
  and book fixture truth.
- It avoids active Wave-38B distributed ownership, Wave-38C async scoping, and
  Wave-38D comptime/JIT scoping.
- It can be bounded to proof probes without broad production changes.

Suggested ownership:

- Primary: `crates/shape-runtime/src/snapshot.rs`
- Primary: `scripts/check-miri-provenance.sh`
- Optional narrow VM-level probes: `crates/shape-vm/src/executor/snapshot.rs`
- Optional doc closeout:
  `docs/cluster-audits/wave39-snapshot-wire-restore-provenance.md`
- Avoid for this lane unless a Miri failure proves the need:
  `crates/shape-vm/src/remote.rs`, distributed CLI tests, broad resume public
  API files, and JIT codegen files.

First proof targets:

1. Add `cfg(miri)` restore probes in `crates/shape-runtime/src/snapshot.rs` for
   `SerializableVMValue::HeapNode { body: TypedObject, ... }` plus `HeapRef`
   back-reference. Restore through `serializable_to_slot`, clone/read a heap
   field, then release/drop both restored and source shares.
2. Add a typed-array restore probe for `TypedArray<*const TypedObjectStorage>`
   containing either shared or self-cyclic typed objects. The probe should prove
   restored element carriers can be read and dropped without relying on printed
   output.
3. Add a `HashMap<string, TypedObject>` restore probe with shared values so the
   `HeapNode` / `HeapRef` identity map is exercised outside direct object fields.
4. Add or wire a Result/Option normalization restore probe that carries a
   typed-object payload and then drops the normalized `__Result` / `__Option`
   typed object under Miri.
5. Add the new filters to `scripts/check-miri-provenance.sh` coverage text and
   run matrix, preserving the script boundary statement that this is still
   targeted evidence only.

Suggested test/filter names:

- `miri_snapshot_restore_typed_object_heapref_clone_drop`
- `miri_snapshot_restore_typed_array_typed_object_elements_clone_drop`
- `miri_snapshot_restore_hashmap_typed_object_values_clone_drop`
- `miri_snapshot_restore_result_option_typed_object_payload_clone_drop`

Supervisor-only verification commands:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

Focused development filters, if the full script is not yet wired:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 MIRIFLAGS=-Zmiri-strict-provenance /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-runtime --lib miri_snapshot_restore_typed_object_heapref_clone_drop'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 MIRIFLAGS=-Zmiri-strict-provenance /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-runtime --lib miri_snapshot_restore_typed_array_typed_object_elements_clone_drop'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 MIRIFLAGS=-Zmiri-strict-provenance /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-runtime --lib miri_snapshot_restore_hashmap_typed_object_values_clone_drop'
```

Adjacent non-Miri gates after implementation:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-runtime --lib snapshot'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-vm --lib snapshot'
```

Cheap source/static closeout:

```bash
scripts/check-typed-opcode-proof-coverage.py
scripts/check-ignored-test-classification.py
scripts/check-no-dynamic.sh
git diff --check -- crates/shape-runtime/src/snapshot.rs crates/shape-vm/src/executor/snapshot.rs scripts/check-miri-provenance.sh docs/cluster-audits/wave39-snapshot-wire-restore-provenance.md
```

Acceptance criteria:

- The closeout must say "targeted snapshot/wire restore provenance evidence",
  not "snapshot restore is UB-free".
- Restore probes must drive `serializable_to_slot` / identity-map paths, not
  hand-construct already-live typed objects after decode.
- HeapRef/back-reference behavior should be observed through restored carriers.
- Unsupported snapshot arms should continue to surface cleanly rather than
  fabricating placeholder heap values.
- No book rows need to change for this proof lane.

## Uncertainty

This report is static. I did not execute any gate and did not validate the
current dirty worktree. Wave-38B's new content-addressed real-socket file is
present locally, but AGENTS still marks that worker active, so I did not count
that proof gap as closed. Several reports and tests in this tree are untracked
or modified by other workers; I treated AGENTS closeout rows plus source files
as evidence and did not revert or normalize any of them.

Static check to run for this report:

```bash
git diff --check -- docs/cluster-audits/wave38-global-proof-gap-refresh.md
```
