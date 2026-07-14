# Wave 39R: Transferred Nested-Closure Layout Scope

Date: 2026-07-10

## Executive Finding

The current `FunctionBlob` schema is sufficient for the first supported lane:
transferred named functions may materialize nested closure literals whose
captures are all immutable by-value captures. No new serializable
`ClosureLayout` descriptor is required for that lane.

The receiver currently loses the compiler-only layout side table during
content-addressed reconstruction. `Program.closure_function_layouts_by_name`
and `LinkedProgram.closure_function_layouts` are `serde(skip)` by design, so
the linker produces no layout for an inbound nested closure. The named body
then reaches `op_make_closure`, which requires the side-table entry and emits:
`op_make_closure: no ClosureLayout registered for function 1`. This matches
the live `compute___impl` failure after Wave 39P: the call and body execute,
but the `data.map(|x| x * 2)` closure cannot be allocated.

The supported reconstruction boundary is deliberately narrower than “all
closures”:

* `blob.is_closure == true` identifies a closure body. `captures_count == 0`
  plus an empty `capture_kinds` vector is a valid zero-capture layout.
* `blob.capture_kinds` is the hash-covered, typed `NativeKind` track. It is
  sufficient to select scalar widths or pointer-sized fields and is preserved
  verbatim for drop/refcount dispatch.
* `blob.frame_descriptor.slots[..captures_count]` is the independent leading
  capture ABI. When present, it must agree with `capture_kinds`; for a
  non-zero capture count it is required. A zero-capture closure does not need a
  capture prefix and can use an empty layout even if no descriptor is present.
* `mutable_captures` is enough to reject mutable captures, but not enough to
  distinguish `OwnedMutable` from `Shared`. Therefore any transferred nested
  closure with a mutable capture must be refused rather than reconstructed.
* `NativeKind::Ptr(Closure)`, `Reference`, `SharedCell`, `IoHandle`, `Future`,
  and `TaskGroup` remain refused. The existing direct-closure refusal matrix
  must not be weakened or bypassed for nested closure materialization.

For the accepted set, the layout can be constructed as
`ClosureLayout::from_capture_types_with_native_kinds`: map scalar
`NativeKind`s to matching scalar `ConcreteType`s, map every accepted
pointer-bearing kind to a pointer-shaped representative `ConcreteType`, use
`CaptureKind::Immutable` for every slot, and pass the original native-kind
vector unchanged. This is the same representation already used by the direct
remote-closure fallback.

## Evidence Path

### 1. Closure literal compilation

`crates/shape-vm/src/compiler/expressions/closures.rs:3443-3481` creates one
`Function` per closure literal, marks it `is_closure`, records
`captures_count`, `ref_params`, `ref_mutates`, and `mutable_captures`, then
records its `ClosureTypeId`.

The capture storage classification is at
`crates/shape-vm/src/compiler/expressions/closures.rs:3495-3607`.
Read-only captures are `CaptureKind::Immutable`. Mutable `let mut` captures
can be `OwnedMutable`; mutable `var` captures can be `Shared`. The compiler
records these kinds in the in-memory `closure_capture_kinds` table and uses
them to rebuild the local layout at
`crates/shape-vm/src/compiler/compiler_impl_reference_model.rs:2891-2973`.

The compiler's local `ClosureLayout` contains more than the wire needs for
the immutable slice: `capture_types`, field offsets, `capture_kinds`, native
kinds, and the three storage masks. `ClosureLayout` documents the lockstep
invariants at `crates/shape-value/src/v2/closure_layout.rs:874-949`; its
constructor computes scalar/pointer widths and masks at
`crates/shape-value/src/v2/closure_layout.rs:1091-1199`.

### 2. Blob construction and hash coverage

`BlobBuilder::finalize` receives the proven per-capture native kinds from the
closure registry at
`crates/shape-vm/src/compiler/compiler_impl_initialization.rs:241-258` and
`275-313`. It writes them, together with `is_closure`, `captures_count`,
`mutable_captures`, `ref_params`, `ref_mutates`, and `frame_descriptor`, into
`FunctionBlob` at `crates/shape-vm/src/compiler/mod.rs:524-549`.

`FunctionBlobHashInput` includes all of those ABI-bearing fields at
`crates/shape-vm/src/bytecode/content_addressed.rs:119-152`, and
`compute_hash` feeds them into SHA-256 at
`crates/shape-vm/src/bytecode/content_addressed.rs:154-195`. In particular,
the capture native-kind vector and frame descriptor are already hash-covered.
Changing either changes the content hash; no new hash or protocol version is
needed for the proposed reconstruction helper.

`capture_names` is intentionally not hash identity and is diagnostic-only.
It is not needed to build a layout. `ref_params` and `ref_mutates` describe
the callable's parameter ABI, not closure storage; they remain available for
the ordinary argument/refusal checks and must not be repurposed as capture
storage hints.

### 3. Transitive content-addressed transfer

Closure bodies are already included in the blob graph. During blob assembly,
`Operand::Function` and `Operand::ClosureAlloc` are added to the dependency
list at `crates/shape-vm/src/compiler/mod.rs:314-402`; therefore a named body
whose bytecode contains `MakeClosure` carries the nested closure blob as a
transitive dependency.

The sender walks `blob.dependencies` and module-binding value targets in
`crates/shape-vm/src/remote.rs:638-722`. Negotiation/resupply only removes or
restores those hashes; it does not remove nested closure metadata from the
blob itself. The serve path verifies received hashes before caching and
hydrates cached dependency blobs at
`bin/shape-cli/src/commands/serve_cmd.rs:1184-1227`, called from the inbound
Call path at `bin/shape-cli/src/commands/serve_cmd.rs:646-657`.

The receiver reconstructs a `Program` from the supplied `(hash, blob)` pairs
at `crates/shape-vm/src/remote.rs:748-805`. That reconstruction copies the
function store but only copies the source program's layout side table. The
side table is absent in a stub/inbound wire program because
`Program.closure_function_layouts_by_name` is `serde(skip)` at
`crates/shape-vm/src/bytecode/content_addressed.rs:328-340`.

### 4. Linker and VM failure

The linker copies blob metadata into `LinkedFunction` at
`crates/shape-vm/src/linker.rs:495-513` and `623-641`, but its current
`remap_closure_function_layouts` only looks up the absent name-keyed side
table at `crates/shape-vm/src/linker.rs:698-718`. It does not derive a layout
from the blob.

The linked-to-bytecode path does preserve the resulting side-table vector at
`crates/shape-vm/src/linker.rs:725-805`, but an inbound content-addressed
program has `None` entries because the remap source was empty. The VM producer
then unconditionally requires the entry at
`crates/shape-vm/src/executor/control_flow/mod.rs:541-559`. This is the exact
failure observed after the named `compute___impl` frame descriptor fix.

### 5. Existing direct-closure fallback

The direct remote-closure path validates the blob's capture count and shipped
kind track at `crates/shape-vm/src/remote.rs:1467-1542`, then refuses mutable,
nested-closure, reference, shared-cell, and resource captures at
`crates/shape-vm/src/remote.rs:1544-1597`.

If the compiler side table is absent, `finish_remote_closure_call` rebuilds an
all-immutable layout from the leading frame slots at
`crates/shape-vm/src/remote.rs:1600-1658` and selects that fallback at
`crates/shape-vm/src/remote.rs:1693-1737`. It never derives a kind from raw
bits. This is the correct template for nested closure layout reconstruction,
but it runs only when `request.upvalues` is present at
`crates/shape-vm/src/remote.rs:1232-1248`; a nested `MakeClosure` inside a
named body follows `op_make_closure` instead.

## Smallest Fix

Do not add a wire `ClosureLayout` field for the immutable nested-closure lane.
Add one shared, fallible helper for a transferred closure blob, preferably
next to the content-addressed types or in a small linker helper module. The
helper should:

1. Require `is_closure` when called for layout reconstruction.
2. Require `capture_kinds.len() == captures_count`. Treat a non-zero count
   with an empty/missing kind track as malformed, never as Bool or pointer
   default.
3. For a present `frame_descriptor`, cross-check its leading
   `captures_count` slots against `capture_kinds`; require enough slots. For a
   non-zero count, reject a missing descriptor. Permit the empty zero-capture
   case without a descriptor.
4. Reject any `mutable_captures[i]` that is true. Also reject the existing
   nested/reference/shared/resource `NativeKind` arms before constructing a
   layout. Do not infer `CaptureKind::OwnedMutable` or `Shared` from the
   native kind, frame bits, or serialized values.
5. Construct an all-`Immutable` layout with the exact native-kind vector. The
   pointer representative is only used for field width; the original
   `NativeKind` remains authoritative for clone/drop dispatch.

Use the helper as the fallback in `linker::remap_closure_function_layouts`
when the name-keyed compiler side table is empty. The function currently
returns a vector; make the fallback validation return a structured link error
or run an equivalent pre-link validation in `run_remote_call` so unsupported
nested captures fail closed with the existing `UnsupportedCapture` policy,
rather than reaching `op_make_closure` as an unexplained compiler bug. Keep
the compiler-produced side table first: local in-memory programs should use
their authoritative layouts, while transferred blobs use the validated
metadata fallback.

This is not a schema migration. The helper consumes existing serialized and
hash-covered fields, so old valid immutable blobs remain identity-compatible.
If a later protocol wants mutable/shared nested captures, the current schema
is insufficient: it would need a hash-covered storage-kind descriptor plus
the inner kind needed by `OwnedMutable`/`Shared` allocation and drop glue,
along with an explicit wire/version compatibility decision. That is outside
this lane.

## Regression Matrix

Add focused unit coverage at the helper/linker boundary:

* A zero-capture `is_closure` blob with `captures_count == 0` and an empty
  `capture_kinds` vector produces `Some(ClosureLayout)` with zero masks and
  zero captures, and does not require a frame descriptor.
* An immutable scalar capture and an immutable pointer capture reconstruct
  exact native kinds, pointer/scalar field widths, and drop masks. The frame
  prefix must match the capture vector.
* Count mismatch, missing non-zero frame descriptor, and frame-prefix mismatch
  fail without constructing a layout.
* Mutable, `Ptr(Closure)`, `Ptr(Reference)`, `Ptr(SharedCell)`, and resource
  kinds fail with the established refusal classification. No test should make
  a mutable capture pass by treating it as immutable.
* Mutating any existing metadata field (`capture_kinds`, frame descriptor,
  `mutable_captures`, or closure identity) changes `FunctionBlob::compute_hash`;
  this guards the claim that no new hash field is required.

Add a real-socket regression under the distributed CLI tests, using the
existing `shape serve` and `WireClient` support:

1. Compile a named remote target whose body contains
   `data.map(|x| x * 2)` and confirm the sender's minimal blob set includes
   both the named body and the zero-capture nested closure blob.
2. Send an intentionally stripped first request, resupply the reported hashes
   on the same connection, and assert the receiver returns the mapped result.
   The receiver log must show inbound execution and no `no ClosureLayout`
   error.
3. Send a subsequent zero-blob request after negotiation/cache hydration and
   assert the same result, proving the nested closure blob is retained and
   reconstructed from the cache rather than from sender-side fallback.
4. Keep direct closure tests for immutable success and mutable/reference/
   resource/nested refusal unchanged; add a named-body negative only if the
   new link/preflight error surface is introduced.

## Ordered Implementation Steps

1. Add the shared fallible immutable-layout reconstruction helper and unit
   tests for zero captures, scalar/pointer captures, metadata disagreement,
   and refusal classes.
2. Add the linker fallback for missing `closure_function_layouts_by_name`,
   preserving compiler-side layouts when present and propagating structured
   unsupported-metadata errors.
3. Verify the existing direct-closure path calls the same helper or remains
   behaviorally identical; do not duplicate a weaker kind mapping.
4. Add the real-socket named-body nested-closure test with stripped,
   resupplied, negotiated, and zero-blob cache-reuse calls.
5. Run the focused unit and CLI test gates under the supervisor's single cargo
   lane, then run the release build and the broader distributed gate only from
   the supervisor worktree.

## Supervisor Verification Commands

The scout ran no cargo, build, rustc, or test command. The following are
commands for the supervisor after implementation; they are intentionally
shown with the required hard cgroup limits.

```bash
systemd-run --user --wait --collect --pipe \
  -p MemorySwapMax=0 -p MemoryMax=12G -p TasksMax=256 \
  env CARGO_BUILD_JOBS=2 cargo test -p shape-vm remote::tests::transferred_closure

systemd-run --user --wait --collect --pipe \
  -p MemorySwapMax=0 -p MemoryMax=12G -p TasksMax=256 \
  env CARGO_BUILD_JOBS=2 cargo test -p shape-cli --test distributed_content_addressed_e2e

systemd-run --user --wait --collect --pipe \
  -p MemorySwapMax=0 -p MemoryMax=12G -p TasksMax=256 \
  env CARGO_BUILD_JOBS=2 cargo build -p shape-cli --release

systemd-run --user --wait --collect --pipe \
  -p MemorySwapMax=0 -p MemoryMax=24G -p TasksMax=512 \
  env CARGO_BUILD_JOBS=2 cargo test -p shape-cli --test distributed_proof_matrix_e2e
```

Existing evidence remains: compiler assertion `run-p2211540-i32888535.service`,
release build `run-p2226733-i32904290.service`, and the live receiver trace
`run-p2233065-i32911060.service`. Those prove the frame metadata and inbound
body transfer; they do not prove nested closure layout reconstruction.

## Changed File

`docs/cluster-audits/wave39-transferred-nested-closure-layout-scope.md`
