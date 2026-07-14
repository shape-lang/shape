# Wave 34 Typed Field Mutation Proof Close

Date: 2026-07-10

Scope: Wave-34A implementation worker. I added focused semantic probes for the
typed-object field mutation proof gap identified in
`docs/cluster-audits/wave33-global-proof-gap-refresh.md`. I did not run cargo,
just, nextest, rustc, build, test, Miri, benchmarks, or book-truth commands;
the supervisor owns those lanes.

## Covered Gap

This closes the narrow proof gap for ordinary typed-object field mutation:

- Storage/Miri: `TypedObjectStorage::write_slot_in_place` now has a Miri-only
  companion that updates the pointer-provenance sidecar after the same raw slot
  write used by production. The new probe overwrites a
  `Ptr(HeapKind::TypedObject)` field, verifies the schema-stamped field kind and
  heap mask remain invariant, releases the overwritten share, clones the new
  field, and drops the outer object.
- VM: `SetFieldTyped` now moves owned `KindedSlot`s through the write path so
  Miri provenance survives stack pop, canonical Option validation, field write,
  and drop. New VM probes cover scalar overwrite metadata, canonical
  `Option<T>` carrier overwrite/readback, and rejection of a non-Option typed
  object carrier.
- JIT FFI: `jit_typed_object_set_field` has focused coverage for scalar slot
  overwrite and, with `gc`, for threading the field's stamped TypedObject kind
  into `jit_write_barrier`.
- ShapeTest: the public Option-field mutation fixture now includes Some-to-Some
  payload overwrite, not only Some/None/self-cycle smoke.

The standing Miri gate now names the storage overwrite and VM Option
`SetFieldTyped` probes explicitly in `scripts/check-miri-provenance.sh`.

## Still Outside Scope

This is not a global UB proof. It does not cover every heap field kind,
snapshot/wire restore, raw snapshot mutation helpers, all property-assignment
paths, trait-object/container fields beyond the focused probes, full JIT codegen
coverage, or book-truth completeness. The JIT proof is FFI-level; native
codegen selection and all benchmark variants remain supervisor verification.

## Supervisor Commands

Run under the single global cargo lane.

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

Focused VM runtime probes:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-vm --lib set_field_typed_scalar_overwrite_preserves_metadata'

systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-vm --lib set_field_typed_option_overwrite_preserves_canonical_carrier_metadata'

systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-vm --lib set_field_typed_option_rejects_non_option_typed_object_carrier'
```

Focused JIT FFI/barrier probes:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-jit --lib jit_typed_object_set_field_overwrites_scalar_slot'

systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-jit --features gc --lib jit_typed_object_set_field_threads_field_kind_to_barrier'
```

Focused ShapeTest public fixture:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 cargo test -p shape-test --test structs_types typed_object_option_field_overwrites_some_payload'
```

Cheap static closeout:

```bash
git diff --check -- crates/shape-value/src/heap_value.rs crates/shape-vm/src/executor/typed_object_ops.rs crates/shape-jit/src/ffi/typed_object/field_access.rs tools/shape-test/tests/structs_types/option_field_mutation.rs scripts/check-miri-provenance.sh docs/cluster-audits/wave34-typed-field-mutation-proof-close.md
```

## File-Size Debt

Localized edits touched pre-existing oversized files:

- `crates/shape-value/src/heap_value.rs` is above 800 lines.
- `crates/shape-vm/src/executor/typed_object_ops.rs` is above 800 lines.

No broad extraction was attempted in this proof lane.
