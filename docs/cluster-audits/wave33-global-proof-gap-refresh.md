# Wave 33 Global Proof-Gap Refresh

Date: 2026-07-10

Scope: Wave-33D read-only scout. I inspected current `AGENTS.md`, the prior
global proof map, JIT/GC barrier reports, proof guard scripts, and focused
source/tests for typed-object mutation, distributed/snapshot/polyglot
composition, async, comptime, and state carriers. I did not run cargo, just,
nextest, rustc, build, test, bench, Miri, or book-truth commands. This report is
the only file written.

## Current Baseline After Wave 32

The global proof story is stronger than the Wave-22 map, but still not a
semantic proof of the whole system.

Verified Wave-32 evidence:

- `state.capture_call(callable, args)` now returns a schema-backed
  `CallPayload { hash, args }` for bounded function/closure hash plus
  homogeneous scalar/string/bool typed-array args. Supervisor evidence:
  `state_capture_call` 4/0 in `run-p823572-i31464643.service`, state builtins
  37/0/1 ignored in `run-p826282-i31467527.service`, release build in
  `run-p827084-i31468401.service`, extraction 707 total / 549 runnable / 158
  disabled in `run-p863388-i31505053.service`, slice B 239/239 in
  `run-p863475-i31505148.service`, and full book gate 549/549 in
  `run-p906543-i31548396.service`.
- `fixture=serve` book-truth support exists for plain loopback `shape serve`
  rows. Supervisor evidence: Node helper 12/0 in
  `run-p826990-i31468298.service`, extraction 707 total / 549 runnable / 158
  disabled / 5 fixture rows in `run-p863388-i31505053.service`, affected slices
  B/D/E in `run-p863475-i31505148.service`, `run-p897091-i31538892.service`,
  and `run-p902334-i31544165.service`, and full book gate 549/549 in
  `run-p906543-i31548396.service`.
- Earlier state carrier work is verified: `capture_module` in
  `run-p114866-i30742753.service`, bounded scalar/string `diff` / `patch` in
  `run-p290748-i30923408.service` and `run-p291995-i30924703.service`,
  public caller/locals rows in `run-p419546-i31054638.service`, and those waves'
  full book gates through 536/536, 538/538, and 540/540.
- JIT heap write-barrier performance is no longer an open performance blocker.
  Wave-20 native heap rows made `17_jit_heap_field_overwrite` and
  `20_jit_heap_field_overwrite_function` native in both default and
  barrier-off artifacts, with final rebuild/classification/book/timing evidence
  `run-p3799627-i30218114.service`, `run-p3802488-i30221202.service`,
  `run-p3803693-i30222464.service`, and `run-p3804871-i30223682.service`.
- Distributed composition has targeted runtime evidence: TLS async remote
  snapshots land only in the receiver store in
  `bin/shape-cli/tests/distributed_proof_matrix_e2e.rs`; extern-C remote
  snapshot/resume from the receiver store is covered by
  `bin/shape-cli/tests/distributed_extern_c_snapshot_e2e.rs`.
- Remote async cancellation improved after Wave-24A. The ignored serialized
  proof suite passed 4/0 in `run-p87917-i30713580.service`, and adjacent
  scheduler/remote/active distributed async checks passed in
  `run-p89272-i30714995.service`.
- Targeted Miri/unsafe proof has expanded but remains intentionally bounded.
  The current `scripts/check-miri-provenance.sh` coverage names provenance
  anchors, nested typed-object clone/drop, typed-array clone/drop, trait-object
  clone/drop, Result/Option carriers, typed-object raw reads, and stack
  provenance. It explicitly says a passing run is not a full UB proof for VM,
  runtime, JIT, FFI, snapshots, arbitrary Shape programs, all field kinds,
  arbitrary trait dispatch, or snapshot/wire restore.

Wave-33 in-flight rows are not verified yet:

- Wave-33A owns the distributed book `serve-snapshot-resume` row for extern-C
  remote snapshot/resume.
- Wave-33B owns the next TypeRef typed-reflection implementation slice.
- Wave-33C owns the real-async current-gap scout.

## Evidence Tiers

Source-only guards:

- `scripts/check-typed-opcode-proof-coverage.py` scans compiler Rust sources
  for typed opcode mentions and expects zero unclassified typed-opcode proof
  gaps. It does not run cargo, rustc, nextest, Miri, or Shape programs.
- `scripts/check-ignored-test-classification.py` guards source-level
  `#[ignore]` taxonomy. It now expects zero active feature gaps in both
  `shape-vm` and `shape-jit`, but that means the source reasons are classified,
  not that ignored coverage has executed.

Targeted runtime tests:

- ShapeTest and CLI e2e tests prove selected user-visible paths: Option field
  mutation, remote calls, snapshot store ownership, async fan-in, and
  cancellation behavior. They are semantic evidence for those rows only.

Miri/provenance checks:

- The Miri gate is true unsafe/provenance evidence for the filters listed in
  the script. It is deliberately not a global UB proof and currently does not
  gate typed-object mutation, snapshot/wire restore, or JIT FFI return tags.

Book truth:

- The book gate proves current runnable snippets agree with the VM/JIT oracle
  and exact expected stdout when present. After Wave-32 that surface is 549/549
  runnable snippets. The remaining 158 disabled snippets are completeness risk,
  not failed evidence.

True semantic proof:

- A true proof claim needs source guard plus at least one runtime or Miri probe
  that forces the guarded path to execute, plus a negative/refusal row for
  unsupported cases. Most current claims are targeted evidence, not full proof.

## Ranked Remaining Proof Gaps

1. Typed-object field mutation semantic bridge.
   The source guard classifies `SetFieldTyped`, and runtime code routes VM
   writes through `write_field_at_idx`, `write_barrier_slot`, and
   `TypedObjectStorage::write_slot_in_place`
   (`crates/shape-vm/src/executor/typed_object_ops.rs`). ShapeTest covers
   `Option<T>` mutation and diagnostics
   (`tools/shape-test/tests/structs_types/option_field_mutation.rs`). JIT FFI
   `jit_typed_object_set_field` reads stamped `field_kinds[idx]`, writes via
   `write_slot_in_place`, and calls `jit_write_barrier`
   (`crates/shape-jit/src/ffi/typed_object/field_access.rs`). What is missing
   is a standing proof lane that wires mutation probes into
   `scripts/check-miri-provenance.sh` and separately forces the VM
   `SetFieldTyped` and JIT FFI write paths. There is a local
   `write_slot_in_place_on_shared_arc_no_write_via_shared_ref_provenance` test
   in `crates/shape-value/src/heap_value.rs`, but it is not in the Miri gate.

2. State resume and full state carriers.
   Wave-25 through Wave-32 made bounded carriers real: `capture_all`,
   `capture_module`, `diff` / `patch`, caller/args/locals, and `capture_call`.
   Public `state.resume(vm)` still depends on live callback wiring and a
   resumable `VmState`; `resume_frame` still refuses metadata-only `FrameState`.
   `resume.rs` documents the structural gaps: missing resume IP, non-empty
   frames that lack `return_ip`, `locals_base`, and `locals_count`, and bounded
   module/frame projection. This is a public API proof gap, but it is larger
   than a first semantic-proof lane.

3. Snapshot/wire restore provenance.
   Existing snapshot and distributed e2e tests prove selected hash/resume flows,
   including receiver-store ownership. The Miri gate still excludes snapshot
   and wire restore. Missing proof: restore typed objects, Result/Option
   normalization, heap fields, and then drop them under Miri without relying on
   printable output.

4. Distributed/snapshot/polyglot composition breadth.
   Non-skipping extern-C rows are strong; dynamic Python/TypeScript rows still
   skip when extension shared libraries are unavailable, and book truth only
   covers the five Wave-32 plain serve rows. Wave-33A is actively targeting a
   book fixture for extern-C remote snapshot/resume. TLS plus dynamic extension
   plus receiver-store resume remains a separate fixture/proof lane.

5. Real async beyond current fan-in and cancellation.
   Current evidence covers `remote::call_async`, join-all materialization,
   local scope/race cancellation, and now plain-TCP distributed cancellation.
   Remaining proof gaps are native async signatures, user continuations,
   remote callee futures, pending-future snapshot/resume, streams/for-await,
   join-settle values, TLS cancellation, and JIT async lowering. Wave-33C is
   the active scout here.

6. Comptime typed generation.
   `__ComptimeTypeRef` exists and `set return (expr)` can consume TypeRef.
   Source still shows string/source-fragment surfaces for `extend (expr)`,
   `replace module`, connector/DuckDB generated return types, and source-level
   `set param name: (expr)`. Wave-33B is actively implementing the next
   TypeRef typed-reflection slice, but full typed fragments/quasiquote/hygiene
   remain outside that lane.

7. JIT/GC proof beyond heap mutation performance.
   Barrier performance and object-field overwrite runtime behavior are in good
   shape. Remaining proof is not "does the barrier run" for known heap field
   stores; it is whether every newly native object/trait/container write or
   return path either uses the same stamped-kind discipline or deopts
   explicitly. JIT FFI return tags are also still outside the Miri gate.

8. Disabled-book completeness.
   Wave-32 improves the count to 707 total / 549 runnable / 158 disabled.
   Wave-30 triage showed the disabled set is a mix of implementation gaps,
   fixtures, preview/prose, diagnostics, old syntax, and count-reduction
   candidates. Passing 549/549 proves the enabled book surface only.

## Recommended Next Proof Lane

Next lane: **typed-object field mutation semantic proof bridge**.

Why this lane: it is concrete, bounded, and crosses the central proof boundary:
source-only `SetFieldTyped` coverage becomes runtime/Miri evidence. It avoids
the larger public-resume design and does not overlap Wave-33A/B/C.

Owned files:

- `crates/shape-value/src/heap_value.rs`
- `crates/shape-vm/src/executor/typed_object_ops.rs`
- `crates/shape-jit/src/ffi/typed_object/field_access.rs`
- `crates/shape-jit/src/ffi/gc.rs`
- `tools/shape-test/tests/structs_types/option_field_mutation.rs`
- `scripts/check-miri-provenance.sh`
- closeout doc, for example
  `docs/cluster-audits/wave34-typed-field-mutation-proof-close.md`

Implementation target:

- Add or wire a Miri-gated `shape-value` probe for
  `write_slot_in_place` replacing a heap/reference field, not just scalar
  slots, and verify field kind / heap mask invariants before drop.
- Add a focused VM probe that executes `SetFieldTyped` on a scalar field and a
  canonical `Option<T>` field, asserting the stored carrier remains typed and
  unsupported carriers surface.
- Add or preserve a focused JIT FFI wrapper test for
  `jit_typed_object_set_field` replacing a reference field and proving the
  barrier uses the object's stamped runtime kind.
- Add the new filters to `scripts/check-miri-provenance.sh` coverage text so
  the standing gate names mutation explicitly.

Supervisor should run, under the serialized cgroup lane:

- `bash scripts/check-miri-provenance.sh`
- focused `shape-value` mutation Miri filters if they are not fully covered by
  the script during development
- focused `shape-vm --lib` filter for `SetFieldTyped` / typed object mutation
- focused `shape-jit --lib` filter for `jit_typed_object_set_field` / barrier
- focused ShapeTest filter for `structs_types::option_field_mutation`
- then the cheap source guards:
  `scripts/check-typed-opcode-proof-coverage.py`,
  `scripts/check-ignored-test-classification.py`, and `git diff --check`

Acceptance criteria:

- The report must not say "UB-free"; it should say the mutation path has
  targeted Miri and runtime evidence.
- `SetFieldTyped` source coverage should cite both the source guard and the new
  semantic probes.
- Unsupported carriers must still fail closed with structured diagnostics.
- No book rows need to change for this proof lane.

## Follow-On Proof Lanes

After the mutation bridge:

1. Snapshot/wire restore Miri lane for schema-backed typed-object restore and
   Result/Option normalization.
2. Public state resume lane for callback wiring plus an explicitly empty-frame
   `VmState` restore before attempting full frame resume.
3. Distributed composition lane after Wave-33A: TLS plus receiver snapshot store
   plus non-skipping extension artifact path.
4. JIT FFI return-tag lane for scalar return tags and fail-closed tag-zero
   behavior.
5. Comptime typed-fragment lane after Wave-33B TypeRef work lands.

