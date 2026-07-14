# Wave-39B Public State/Resume Scope

Role: lightweight public state/resume scope scout.

Scope honored: static reads only. I inspected the current state builtins,
`VmStateSnapshot`, public resume and snapshot restore code, focused state and
resume tests, the Wave-31 state-carrier audit, the Wave-25 book target note,
the Wave-33/Wave-38 gap notes, and the sibling state/resumability pages in
`shape-web`. I wrote only this report. No cargo, build, test, extraction, or
book-truth command was run.

## Recommendation

Do not wire the existing metadata-only `VmState` directly into public
`state.resume`. The smallest honest executable slice is a versioned,
explicitly executable `VmState` carrier for one constrained top-level
continuation boundary:

- the call stack is empty;
- the operand stack and local window are empty at the boundary;
- loop, timeframe, and exception-handler stacks are empty;
- module bindings are empty for the first slice, or are restored by name and
  exact kind if the implementation chooses to carry them;
- no closure, reference, channel, iterator, foreign runtime, or pending future
  is reachable;
- the boundary records the next instruction after the capture operation, so
  resume cannot re-enter the capture and loop forever; and
- the same content-addressed program and schema environment is required.

This is intentionally narrower than full `capture_all` continuation. It is
still executable because the carrier contains a validated continuation
identity and instruction position, rather than treating counts and
`instruction_count` as an instruction pointer. If the boundary does not meet
these preconditions, capture or resume must return a structured error. Missing
fields must not be interpreted as empty values.

The public API needs an explicit executable distinction. The preferred shape
is an additive, versioned executable envelope inside `VmState` (or a distinct
`ExecutableVmState` accepted by a deliberately revised `resume` signature),
while retaining the current metadata capture semantics. Do not make
`state.resume(vm)` appear complete by accepting the current metadata carrier
and starting a fresh VM at IP zero.

## Current Behavior

The current metadata path is real and useful:

- `create_state_module` registers `FrameState` with function name, blob hash,
  `ip`, and arg/local/upvalue counts, and registers `VmState` with frames,
  module bindings, and cumulative `instruction_count`
  (`crates/shape-vm/src/executor/state_builtins/core.rs:39-75`).
- `state.capture_all` reads a live `VmStateAccessor`, creates real typed
  `FrameState` objects and a schema-backed `VmState`, and preserves only the
  currently projectable homogeneous binding carriers
  (`state_builtins/introspection.rs:795-823`). The corresponding book row is
  already runnable as metadata at `stdlib/core/state.mdx:163`.
- `VmStateSnapshot` owns cloned `KindedSlot` values for current args, current
  locals, and module bindings. Its per-frame records still set `local_ip` to
  zero and leave per-frame args empty because the call-frame data is not
  available at that accessor boundary
  (`crates/shape-vm/src/executor/vm_state_snapshot.rs:83-131`).
- `state.resume` only queues a cloned payload when
  `ModuleContext.set_pending_resume` exists. Normal module dispatch currently
  supplies `None` for both resume callbacks
  (`crates/shape-vm/src/executor/state_builtins/introspection.rs:874-909`,
  `crates/shape-vm/src/executor/vm_impl/modules.rs:901-918`).
- The lower `apply_pending_resume` path can decode a typed-object payload and
  call `VirtualMachine::from_snapshot`, but the public decoder sets `ip: 0`.
  It restores bounded module bindings, while a non-empty frames array is
  rejected because the public `FrameState` cannot provide the structural
  fields required by `SerializableCallFrame`
  (`crates/shape-vm/src/executor/resume.rs:465-546` and `:617-710`).
- The internal `snapshot()` / CLI `--resume` path is a separate, more complete
  carrier. `VmSnapshot` already has stack, module bindings, IP, control state,
  and call frames; `SerializableCallFrame` has `return_ip`, `locals_base`,
  `locals_count`, function identity, upvalues, blob hash, and local IP
  (`crates/shape-vm/src/executor/snapshot.rs:153-269`,
  `crates/shape-runtime/src/snapshot.rs:512-587`). The two local resumability
  book rows are now fixture-backed and runnable; they do not prove public
  `state.resume`.

The existing focused tests correctly prove metadata construction, structured
errors, callback absence, and the empty module-binding carrier. The direct
resume tests prove the internal restore mechanics and rejection surfaces, but
not a Shape-level `state.capture_all` to `state.resume` continuation.

## Required Executable Carrier

For the constrained first slice, the carrier must have these fields or an
equivalent typed structure:

| Field | Required invariant |
|---|---|
| format/version | Exact supported version; unknown versions reject. |
| program identity | Content hash or code-manifest identity for the loaded program; never resolve code by an untrusted name alone. |
| schema/module identity | Schema-registry identity and module identity must match the restore environment. |
| resume function identity | Function blob hash, with function id only as a checked local fallback. |
| resume IP | Function-relative offset (or equivalent validated absolute IP) for the next instruction, not cumulative `instruction_count`; offset must be within the function body. |
| operand stack | Real serialized values plus authoritative `NativeKind` per slot. The first slice requires an empty stack and asserts that fact. |
| locals and args | Real serialized slots plus kinds and declared layout/count. The first slice requires zero locals and args; later frame work must carry them explicitly. |
| call frames | Explicit frame records with `return_ip`, `locals_base`, `locals_count`, function identity, and closure/upvalue data. The first slice requires an empty call stack. |
| module bindings | Name/index identity, serialized values, and kinds. The first slice may require an empty set; a non-empty set must not be silently dropped. |
| control state | Loop, timeframe, and exception-handler state, or an explicit empty-state invariant for the first slice. |
| suspension barriers | No pending futures, live references, channels, iterators, foreign frames, or opaque native state. |

For a later multi-frame slice, `FrameState` must grow beyond metadata counts.
Each frame needs actual args and locals, validated `local_ip`, `return_ip`,
`locals_base`, `locals_count`, function/blob identity, and upvalues with a
parallel kind vector. Closure captures must be restored through the registered
closure layout, with capture count and kind checks. A frame's `ip` must never
be inferred from `instruction_count` or accepted merely because it is within
the bytecode array.

The capture boundary also needs a dispatch contract. `capture_all` currently
observes the VM before a module builtin call; an executable capture must be
given the post-call continuation IP, matching the existing
`consume_snapshot_suspension` discipline in
`crates/shape-vm/src/executor/snapshot.rs:793-812`. Otherwise a state captured
before `resume` can restart at the capture site or at program entry rather
than continue after it.

## Security And Provenance Boundaries

The restore operation is a code and data admission boundary, not ordinary
object deserialization:

1. Resolve the program by content hash or a verified code manifest. Reject a
   hash/name mismatch and reject function ids that are out of range.
2. Resolve the schema registry and module identity before reading fields.
   Reject unknown schema ids, field-count drift, and duplicate or reordered
   binding identities unless the format explicitly records the order.
3. Decode every value through the kind-threaded snapshot codec. Never rebuild
   `NativeKind` from raw bits, use a Bool default, reinterpret a typed-object
   pointer as a different heap type, or accept an arbitrary raw IP.
4. Validate all vector lengths, frame windows, function-relative offsets,
   closure capture counts, and stack/kind lockstep lengths before installing
   anything into the VM.
5. Refuse unsupported heap and runtime state with a clear barrier. A pending
   future, foreign runtime frame, live borrow, channel, iterator, closure
   without a verified layout, or opaque native resource cannot be represented
   by this first carrier.
6. Preserve ownership through `KindedSlot` clone/transfer/drop discipline and
   the snapshot codec's identity context. No public carrier should expose or
   restore process-local pointers.

The first slice should therefore be same-program and scalar-only even if the
internal snapshot format can carry more. Cross-process or cross-node public
resume requires a persisted code manifest, schema hashes, blob availability,
and a store/permission policy. That is a dependency, not a free consequence
of adding the callback.

## File Ownership

One implementation worker should own the following closely related files:

- `crates/shape-vm/src/executor/state_builtins/core.rs`: executable carrier
  schema and registration, or the intentionally revised public type.
- `crates/shape-vm/src/executor/state_builtins/introspection.rs`: capture
  preconditions, executable envelope construction, and strict resume request
  validation.
- `crates/shape-vm/src/executor/vm_state_snapshot.rs`: post-call continuation
  IP and real slot/frame capture; do not leave zero-IP placeholders on the
  executable path.
- `crates/shape-vm/src/executor/vm_impl/modules.rs`: wire the pending-resume
  callback with VM ownership and pass the correct continuation boundary.
- `crates/shape-vm/src/executor/resume.rs`: typed carrier decode, provenance
  checks, and conversion into the existing `VmSnapshot` restore path.
- `crates/shape-runtime/stdlib-src/core/state.shape`: public type and
  contract declaration, kept distinct from metadata-only capture.
- Focused tests in `crates/shape-vm/src/executor/state_builtins_tests.rs`,
  `crates/shape-vm/src/executor/resume.rs`, and a new narrow
  `tools/shape-test/tests/...` public-state-resume fixture.

Only after the same-process proof is stable should an owner add a focused
`bin/shape-cli/tests/...` e2e for a persisted executable carrier. Do not mix
this lane with `remote.rs`, polyglot extension setup, or the existing CLI
snapshot fixture helpers.

## Focused Proofs

The implementation should land with these proofs, in this order:

1. VM unit proof: construct the executable carrier at the supported empty
   top-level boundary, assert the program/schema/hash/IP fields and empty-state
   invariants, restore it, and assert the next instruction executes exactly
   once with the expected scalar result.
2. VM negative proofs: reject missing callback, metadata-only `VmState`, wrong
   program or schema identity, hash/id mismatch, IP out of range, malformed
   kind tracks, non-empty unsupported frame/control state, and each snapshot
   barrier without mutating the live VM.
3. ShapeTest proof: call the public capture and resume builtins through the
   real module dispatch path, not a hand-built `ModuleContext`; assert a
   post-resume marker/value and a no-loop instruction count. Keep this VM-only
   unless JIT entry semantics are separately specified.
4. E2E proof: run the bounded carrier through the actual selected store or
   same-process host boundary, restore with the same verified code/schema
   identity, and assert deterministic continuation. A CLI `snapshot()` test
   alone is insufficient because it bypasses the public state carrier.

## Book Rows And Classification

Exactly one currently disabled row is a possible flip from this slice:

- `stdlib/core/state.mdx:225`, manifest id
  `B__stdlib__core__state__12__L225.shape`, the `state.resume(vm)` example.
  It can flip only after the example is rewritten to the supported executable
  boundary and the ShapeTest/VM proof demonstrates continuation. The current
  `capture_all` plus `resume` text must not be flipped unchanged, because its
  metadata carrier is not executable.

The following remain disabled or are outside this lane:

- `stdlib/core/state.mdx:241`, manifest id
  `B__stdlib__core__state__13__L241.shape`, is a feature gap. It needs a real
  resumable frame carrier and is not made true by empty top-level resume.
- `advanced/resumability.mdx:21` and `:100` are no longer disabled. They are
  fixture-only CLI snapshot/resume rows and were correctly flipped by the
  local two-process fixture. They do not need this public state implementation.
- The disabled state cache, generic serialization, diff/patch, remote
  dispatch, and module-sync rows are independent feature gaps, not flips from
  callback wiring or an empty-frame resume.
- Distributed/polyglot snapshot rows are fixture and environment work. They
  need receiver setup, extension artifacts, store selection, and foreign-state
  policy; public `state.resume` does not remove those dependencies.

## Bottom Line

The next honest lane is an explicitly executable, versioned, same-program
empty-top-level carrier plus real callback wiring and a public dispatch proof.
It may reduce one disabled state row after a deliberate book rewrite. Full
public resume, `resume_frame`, cross-node transfer, closures, non-empty frames,
and arbitrary module/heap state remain separate implementation and proof
lanes. Starting from the current metadata-only `VmState` would produce a
restart or an IP-zero execution, not resume.
