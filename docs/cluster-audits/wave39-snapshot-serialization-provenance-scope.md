# Wave 39N Snapshot Serialization Provenance Scope

Date: 2026-07-10  
Role: read-only production-scope scout

## Decision

The first strict-provenance failure is an API-boundary defect, not a wire-format
defect. `slot_to_serializable_ctx` and `slot_to_serializable` accept only
`(u64, NativeKind)` (`crates/shape-runtime/src/snapshot.rs:1366-1377,
:1392-1396`). Their heap arms then reconstruct live pointers from the integer
at `:1621-1690`, `:1809-1834`, `:1852-1885`, `:1991-1995`, and
`:2145-2197`. The source `KindedSlot` already owns the required Miri sidecar;
the serializer discards it before the first dereference.

The smallest production-faithful fix is to make the canonical serializer
carrier-aware:

```text
slot_to_serializable_ctx(
    slot: &KindedSlot,
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
) -> Result<SerializableVMValue, String>

slot_to_serializable(
    slot: &KindedSlot,
    store: &SnapshotStore,
) -> Result<SerializableVMValue, String>
```

Keep one private parts/view helper for recursive fields. It receives
`bits`, `kind`, and, under `cfg(miri)`, a copied `MiriSlotProvenance`; it does
not own or drop that field share. `typed_object_storage_to_serializable` must
source that sidecar from `TypedObjectStorage::field_provenance` at
`:4029-4034`, alongside the existing bits/kind read at `:1580-1588`.

The old raw-pair entry point may remain temporarily as a clearly named
compatibility helper, but under `cfg(miri)` it must reject every non-null
pointer-bearing kind with a migration error. It must never call the new helper
with `MiriSlotProvenance::None`. Scalar projection remains valid through the
raw compatibility path.

## Pointer Recovery

For the carrier-aware path, each targeted branch must select its original typed
pointer under `cfg(miri)` and only use `bits as *const/*mut T` under
`cfg(not(miri))`:

* `NativeKind::String` and `Ptr(String)`: `MiriSlotProvenance::String` for
  `Arc<String>` reads.
* `Ptr(TypedObject)`: `MiriSlotProvenance::TypedObject` for the borrow at
  `:1827-1833`; nested field pointers are then read as pointers from the
  typed-object storage, never reconstructed from field bits.
* `Ptr(TypedArray)`: `MiriSlotProvenance::TypedArray` for the array base at
  `:1852-1872`. Typed-object, string, and decimal element values already live
  as typed pointers in the element buffer and must be read as such.
* `Ptr(HashMap)`: `MiriSlotProvenance::HashMap` for the outer `Arc` at
  `:1991-1995`. Map keys and typed-object values are then read through their
  typed pointer carriers at `:2005-2047`.
* `Ptr(Reference)` and `Ptr(SharedCell)`: use their existing sidecar variants
  for the outer `Arc` recovery in `serialize_reference` and
  `serialize_shared_cell` (`:2139-2197`). The shared-cell interior is a
  separate limitation: `SharedCell::lock()` returns only `(u64, NativeKind)`
  at `:2247-2256`, so its nested heap value is Stage 2.

`MiriSlotProvenance` currently has no `StringV2` or `DecimalV2` variants
(`crates/shape-value/src/heap_value.rs:533-542`). Those branches at
`snapshot.rs:1482-1509` must either gain matching variants and sidecar-aware
reads, or clean-refuse under Miri. Adding `expose_addr`, `from_exposed_addr`,
`transmute`, or another permissive integer-to-pointer reconstruction is not a
fix. Unsupported Arc heap arms must fail closed rather than fabricate a
provenance source.

## Migrate Now

1. Change the two canonical serializer signatures above and thread the carrier
   through `slot_heap_to_serializable` (`:1597-1602`) and all recursive typed
   object/array/map walks. Preserve the existing identity context and wire
   shapes.
2. Update `ExecutionContext::snapshot` at
   `crates/shape-runtime/src/context/mod.rs:452-459`; it already has a
   `&KindedSlot` and currently throws away the sidecar.
3. Convert the four focused probes in
   `snapshot.rs:5541-5854` to construct/use owning carriers:
   `miri_snapshot_wire_restore_provenance_heapnode_heapref_typed_object_identity`,
   `...typed_array_typed_object_elements`,
   `...hashmap_typed_object_shared_values`, and
   `...result_option_normalize_to_typed_objects`. The first three must call the
   carrier-aware serializer; their existing restore-side assertions remain.
   The Result/Option probe is restore-focused and needs no new serializer
   coverage, but must remain in the same gate.
4. Keep the existing `SerializeIdentityCtx` handle keys as raw pointers used
   only as identity keys (`:1232-1257`). The key role does not dereference a
   pointer and is not the failing cast.
5. Make the Miri failure explicit when a live pointer has no matching sidecar.
   `KindedSlot::new` already initializes `None` (`kinded_slot.rs:79-112`), so
   silently accepting it would recreate the current bug.

## Stage 2 Boundaries

Do not claim the VM snapshot or whole-program snapshot path closed in this
slice. `crates/shape-vm/src/executor/snapshot.rs:190-221` still serializes
raw `self.stack` and `self.module_bindings`; its stack provenance helpers
(`vm_impl/stack.rs:1217-1245`) are not used by this writer, and module-binding
storage has the same raw-array shape. Migrate those writers only when their
installation/restore paths consume owning carriers end to end.

Also defer:

* closure/shared-cell interiors and call-stack captures
  (`snapshot.rs:2247-2256`, `crates/shape-vm/src/executor/snapshot.rs:1058`;
  `shape-vm` closure capture restore);
* state/resume projection (`crates/shape-vm/src/executor/resume.rs:207-220,
  :584-605`);
* state-builtin capture/serialization
  (`crates/shape-vm/src/executor/state_builtins/core.rs:425-457`);
* remote argument/result and capture serialization
  (`crates/shape-vm/src/remote.rs:1417`,
  `crates/shape-vm/src/executor/builtins/remote_builtins.rs:169-302`);
* time-travel/raw module-binding carriers and any heap kinds without a
  `MiriSlotProvenance` variant.

The legacy raw serializer may remain at those compatibility boundaries only if
it rejects pointer-bearing values under Miri. It is not evidence of a strict
provenance closure.

## Proof Boundary

True closure for this slice means all four named runtime probes pass under
`MIRIFLAGS=-Zmiri-strict-provenance`, with no integer-to-pointer diagnostic in
the targeted serializer, and the `cfg(miri)` pointer branches visibly select
the sidecar. Passing default Miri or Tree Borrows alone, changing only probe
construction, or using exposed/permissive provenance proves only that a test
avoided the bad path; it does not close the production serialization API.

The existing gate wiring is `scripts/check-miri-provenance.sh:105-111` and
`:217-222`. Supervisor verification after implementation:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; \
    direnv exec "$PWD" env CARGO_BUILD_JOBS=2 \
    /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-runtime \
    --lib miri_snapshot_wire_restore_provenance'
```

Run the same filter with `MIRIFLAGS=-Zmiri-tree-borrows`, then run the full
`scripts/check-miri-provenance.sh` gate under its documented supervisor cgroup.
Also run `git diff --check --
docs/cluster-audits/wave39-snapshot-serialization-provenance-scope.md`.
