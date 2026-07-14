# Wave 23 Semantic Proof-Bridge Plan

Date: 2026-07-09

Scout scope: read-only over proof scripts/docs, Miri/provenance tests, typed-opcode
checker, ignored-test classifier, typed-field mutation tests, snapshot/wire restore
code, JIT FFI return paths, and trait/object carrier tests. This report is the
only written artifact for Wave-23E.

Wave-22E's key warning is still correct: the source guards are useful drift
detectors, but they are not semantic proof. The next proof worker should keep
those guards and add a small number of probes that force the classified paths to
execute under runtime tests or Miri.

The first implementation lane should be typed-object field mutation, because it
is the narrowest bridge from source coverage to semantic evidence and it touches
the same primitive used by VM field writes, JIT typed-object field stores, and
snapshot typed-object materialization.

## Current Guard Boundary

`scripts/check-typed-opcode-proof-coverage.py` is explicitly source-only. It
classifies typed opcode mentions in compiler sources and expects zero unproven
gaps, but it does not run cargo, rustc, nextest, Miri, or Shape programs.

`scripts/check-ignored-test-classification.py` is also source-only. It checks
that ignored tests stay in known buckets; it does not prove that any ignored
surface is semantically implemented.

`scripts/check-miri-provenance.sh` is targeted semantic evidence. It currently
covers selected shape-value provenance anchors, nested typed-object clone/drop,
typed-array clone/drop, trait-object raw carrier clone/drop, result/option
carriers, typed-object `get_prop` raw reads, and stack provenance. It does not
yet cover typed-object mutation through the VM write opcode, snapshot/wire
restore materialization, JIT host-boundary return tags, or trait dispatch
semantics beyond carrier clone/drop.

## Lane 1: Typed-Object Field Mutation

Goal: prove the write path that source guards classify as covered actually
executes with the expected kind, option-carrier, barrier, and drop behavior.

Current static evidence:

- `crates/shape-vm/src/executor/typed_object_ops.rs` routes
  `SetFieldTyped` through schema/inline-cache/name resolution, validates
  canonical `__Option.Some/None` carriers for `Option<T>` fields, calls
  `write_barrier_slot`, and mutates storage through
  `TypedObjectStorage::write_slot_in_place`.
- `crates/shape-value/src/heap_value.rs` documents the unsafe contract for
  `TypedObjectStorage::write_slot_in_place`: callers must not keep a shared
  `&TypedObjectStorage` live across mutation, the VM is single-threaded, field
  kind is invariant, and the heap mask must remain consistent.
- `crates/shape-jit/src/ffi/typed_object/field_access.rs` uses the same
  storage primitive in `jit_typed_object_set_field`.
- `tools/shape-test/tests/structs_types/option_field_mutation.rs` already
  checks current user-facing `Option<T>` mutation behavior and diagnostics.

Missing semantic bridge:

- A Miri probe that mutates a typed-object field through the real
  `write_slot_in_place` path and then drops the replaced occupant.
- A VM probe that executes `SetFieldTyped` for at least one scalar field and one
  canonical `Option<T>` field.
- A JIT FFI wrapper probe that calls `jit_typed_object_set_field` without
  running a full JIT program, so the FFI boundary and write barrier are covered
  separately from codegen.

Owned files for the proof worker:

- `crates/shape-value/src/heap_value.rs`
- `crates/shape-vm/src/executor/typed_object_ops.rs`
- `crates/shape-jit/src/ffi/typed_object/field_access.rs`
- `tools/shape-test/tests/structs_types/option_field_mutation.rs`
- `scripts/check-miri-provenance.sh`
- `docs/cluster-audits/wave24-semantic-proof-bridge-close.md`

Smallest probes to add:

- `miri_typed_object_write_slot_in_place_replaces_ref_field`: construct a
  schema-backed typed object with one reference field, replace the field through
  `TypedObjectStorage::write_slot_in_place`, assert the field kind and heap mask
  are unchanged, and let both old and new occupants drop under Miri.
- `miri_set_field_typed_option_carrier_write_and_drop`: execute the VM
  `SetFieldTyped` path on an `Option<T>` field with a canonical
  `__Option.Some` payload and assert the stored carrier remains a typed object.
- `jit_typed_object_set_field_runtime_write_barrier_wrapper`: call
  `jit_typed_object_set_field` directly from a focused unit test, replacing a
  reference field and asserting the stored bits/kind are visible through the
  typed-object read helper.
- Keep the existing ShapeTest `option_field_mutation` rows as runtime
  user-facing coverage; do not replace Miri evidence with those tests.

Later proof-worker commands:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-vm --lib set_field_typed -- --nocapture'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-jit --lib jit_typed_object_set_field -- --nocapture'
```

Acceptance:

- The typed-opcode checker can continue to classify `SetFieldTyped`, but the
  closeout must cite the new Miri/runtime probes as the semantic proof.
- The Miri script output must name the new field-mutation filters in its
  coverage list.
- Failure modes must be diagnostic, not permissive. Unsupported carriers should
  assert the exact structured refusal rather than accepting any `Err`.

## Lane 2: Snapshot and Wire Restore

Goal: prove restore code preserves typed-object carrier semantics and is
drop-safe when reconstructing schema-backed `Result`, `Option`, typed objects,
and heap-element containers.

Current static and runtime evidence:

- `crates/shape-runtime/src/snapshot.rs` materializes heap nodes in two passes,
  builds typed-object shells, fills fields through
  `TypedObjectStorage::write_slot_in_place`, and normalizes legacy
  `ResultData`/`OptionData` into schema-backed typed objects.
- Existing snapshot tests cover schema-backed result/option round trips,
  legacy result/option normalization, typed-object fields containing legacy
  result data, typed-object arrays, and object identity/cycles.
- `crates/shape-runtime/src/wire_conversion.rs` converts using explicit
  `(bits, kind)` pairs, projects typed-object fields with known kinds, and
  restores wire `Result`/`Null` into schema-backed typed objects without tag-bit
  probing.
- Existing wire tests cover result/option wire projection and restore.

Missing semantic bridge:

- Miri coverage for restore-then-drop paths that allocate typed-object carriers
  through snapshot and wire restore.
- A focused runtime gate that keeps the legacy-to-schema-backed normalization
  tests tied to the source guard story.

Owned files for the proof worker:

- `crates/shape-runtime/src/snapshot.rs`
- `crates/shape-runtime/src/wire_conversion.rs`
- `scripts/check-miri-provenance.sh`
- `docs/cluster-audits/wave24-semantic-proof-bridge-close.md`

Smallest probes to add:

- `miri_snapshot_typed_object_restore_write_slot_and_drop`: build the smallest
  serializable typed object containing one scalar field and one reference field,
  restore it through `serializable_to_slot`, verify field kinds, then drop it
  under Miri.
- `miri_snapshot_legacy_option_result_restore_to_typed_object`: restore legacy
  `OptionData` and `ResultData` payloads and assert both normalize to
  `NativeKind::Ptr(HeapKind::TypedObject)`.
- `miri_wire_result_null_restore_to_schema_backed_typed_objects`: drive the
  existing wire result/null restore path under Miri and assert the restored
  carriers are schema-backed typed objects.

Later proof-worker commands:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-runtime --lib schema_backed_result -- --nocapture'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-runtime --lib wire_result_and_null_restore_to_schema_backed_typed_objects -- --nocapture'
```

Acceptance:

- Snapshot restore tests prove both positive carrier restoration and explicit
  refusal for unsupported serialized shapes.
- Wire restore continues to avoid tag-bit probing; tests must assert restored
  `NativeKind`, not just printable output.
- Miri coverage list names snapshot and wire restore probes separately.

## Lane 3: JIT FFI Returns

Goal: prove the JIT host boundary decodes return values using stamped return
type tags, not NaN-box guesses or default boolean/null fallbacks.

Current evidence:

- `crates/shape-jit/src/context.rs` defines `RETURN_TAG_NANBOXED`,
  `RETURN_TAG_F64`, `RETURN_TAG_I64`, `RETURN_TAG_I32`, `RETURN_TAG_BOOL`, and
  `RETURN_TAG_UNIT`.
- `crates/shape-jit/src/executor.rs` converts typed return tags into
  `WireValue::Number`, `WireValue::Integer`, `WireValue::Bool`, or
  `WireValue::Null` at the host boundary. The `RETURN_TAG_NANBOXED` path
  reports an error instead of decoding unknown bits.
- `crates/shape-jit/src/ffi/value_ffi.rs` documents the raw `u64` plus
  parallel `NativeKind` contract.
- `crates/shape-jit/src/mir_compiler/terminators.rs` and
  `crates/shape-jit/src/mir_compiler/types.rs` contain the call/return kind
  routing and the known unsupported trait-return surfaces.

Missing semantic bridge:

- Focused runtime tests that exercise the return-tag projection helper and fail
  if tag zero reaches the host boundary.
- A small VM/JIT parity fixture for scalar returns after the helper tests pass.

Owned files for the proof worker:

- `crates/shape-jit/src/context.rs`
- `crates/shape-jit/src/executor.rs`
- `crates/shape-jit/src/executor_return_tag_tests.rs`
- `crates/shape-jit/src/lib.rs`
- `crates/shape-jit/src/mir_compiler/terminators.rs`
- `crates/shape-jit/src/mir_compiler/types.rs`
- `tools/vmjit-diff/corpus/jit_return_tags.shape`
- `docs/cluster-audits/wave24-semantic-proof-bridge-close.md`

Smallest probes to add:

- Extract a tiny `pub(crate)` host-boundary helper from `execute_with_jit` if
  needed, then test `I64`, `F64`, `I32`, `BOOL`, and `UNIT` tags directly.
- `jit_return_tag_nanboxed_surfaces_without_decode`: assert tag zero returns the
  existing diagnostic instead of converting raw bits.
- `jit_scalar_return_tags_vmjit_diff`: a minimal corpus file with functions
  returning int, number, bool, and unit, run through the existing VM/JIT diff
  harness after the unit tests pin the helper.

Later proof-worker commands:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-jit --lib jit_return_tag -- --nocapture'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 node tools/vmjit-diff/run-diff.mjs --filter jit_return_tags'
```

Acceptance:

- The negative tag-zero test must fail if `RETURN_TAG_NANBOXED` silently decodes
  raw bits.
- Positive tests must assert exact `WireValue` variants and payloads.
- Trait/object JIT return limitations remain explicit diagnostics until the
  missing trait return side table is implemented.

## Lane 4: Trait and Object Carriers

Goal: split carrier provenance from dispatch semantics and prove both the
supported positive rows and the currently unsupported structured-refusal rows.

Current evidence:

- `crates/shape-value/src/heap_value.rs` has Miri coverage for raw trait-object
  carrier clone/drop.
- `crates/shape-vm/src/compiler/trait_object_emission.rs` builds vtables for
  direct and boxed-return paths and documents unsupported `SelfArg`, `Generic`,
  `Compound`, and nested-Self surfaces.
- `crates/shape-vm/src/executor/objects/mod.rs` routes trait-object receivers
  into dynamic method dispatch.
- `crates/shape-vm/src/executor/tests/trait_object_thunks.rs` covers direct
  dispatch and boxed top-level `Self` rewraps, but several tests currently allow
  either success or a structured surface.
- `tools/shape-test/tests/traits/dispatch.rs` covers concrete trait dispatch
  from user-facing Shape tests.

Missing semantic bridge:

- Runtime tests that no longer accept both success and refusal for the same
  trait-object path.
- Separate positive tests for direct dispatch and top-level boxed `Self`
  return, and separate diagnostic tests for unsupported `SelfArg`, compound,
  generic, and nested-Self surfaces.
- If arrays of dynamic trait carriers are supported, one focused
  `Array<dyn Trait>` dispatch probe; otherwise an exact unsupported diagnostic.

Owned files for the proof worker:

- `crates/shape-value/src/heap_value.rs`
- `crates/shape-vm/src/compiler/trait_object_emission.rs`
- `crates/shape-vm/src/executor/objects/mod.rs`
- `crates/shape-vm/src/executor/tests/trait_object_thunks.rs`
- `crates/shape-vm/src/executor/tests/trait_object_carriers.rs`
- `tools/shape-test/tests/traits/dispatch.rs`
- `scripts/check-miri-provenance.sh`
- `docs/cluster-audits/wave24-semantic-proof-bridge-close.md`

Smallest probes to add:

- Keep `miri_trait_object_raw_carrier_clone_and_drop` in the Miri gate as the
  storage-provenance anchor.
- `trait_object_direct_dispatch_returns_exact_value`: assert exact output and
  receiver carrier kind for the direct vtable row.
- `trait_object_boxed_return_self_rewraps_and_dispatches`: return top-level
  boxed `Self`, rewrap as the trait object, and dispatch a second method.
- Convert permissive unsupported rows into exact diagnostic assertions, one row
  each for `SelfArg`, nested `Option<Self>`, nested `Result<Self, E>`, and
  generic/compound paths.

Later proof-worker commands:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-vm --lib trait_object_thunks -- --nocapture'
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-test traits -- --nocapture'
```

Acceptance:

- Positive trait-object tests assert exact values and carrier kinds.
- Unsupported trait-object tests assert exact structured diagnostics.
- No test in this area may accept both `Ok(_)` and `Err(_)` as passing unless
  it is deliberately being deleted in the same patch.

## Proof Closeout Rules

For each lane, the later proof worker should close with:

- The source guard result, labeled as source-only.
- The Miri or runtime command result, labeled as semantic evidence.
- The exact test filters added to `scripts/check-miri-provenance.sh`, if any.
- A statement of what remains unproved. For example, typed-object field mutation
  Miri does not prove arbitrary JIT codegen; JIT return-tag helper tests do not
  prove trait-object native JIT returns.

Do not broaden these lanes into full proof of the VM, serializer, or JIT. The
point is to bridge the specific Wave-22E source-guard gap with minimal semantic
evidence that will fail when the implementation silently stops executing the
claimed path.
