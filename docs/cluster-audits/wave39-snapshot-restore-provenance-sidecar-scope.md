# Wave-39I Snapshot Restore Provenance Sidecar Scope

Role: read-only architecture/proof scout. This report is based on the current
worktree source and the supplied supervisor-run evidence. It does not claim a
global UB proof.

## Finding

The smallest production-faithful fix is to make the restore result an owning
`KindedSlot`, whose normal-build representation remains the existing
`(ValueSlot bits, NativeKind)` carrier and whose `cfg(miri)` representation
also carries `MiriSlotProvenance`. `serializable_to_slot` and
`serializable_to_slot_ctx` should return that carrier, not a bare `(u64,
NativeKind)` pair. Existing pair consumers must explicitly convert with a
sidecar-preserving operation or be limited to scalar/no-owner cases.

This reuses the sidecar already used by `KindedSlot::new_with_miri_provenance`,
`TypedObjectStorage::field_provenance`, and the VM stack provenance table. It
avoids a second restore-only ownership model and makes dropping an incompletely
installed restore value release the correct allocation through the existing
`KindedSlot` `Clone`/`Drop` implementation.

The immediate failure is at `crates/shape-runtime/src/snapshot.rs:3523-3548`
and `:3569-3625`: the Result/Option compatibility arm creates a payload
`KindedSlot`, then `build_builtin_variant_typed_object_slot` extracts only
`payload.slot()` and `payload.kind()`, calls `TypedObjectStorage::_new`, and
returns raw bits. Under Miri the new outer TypedObject has a `None` field
sidecar. Its `Drop` reaches
`KindedSlot::drop: missing Miri provenance for TypedObject carrier`.

## Evidence and Restore Graph

Supervisor evidence:

* `run-p1811933-i32478795` passed the historical probes, then the first
  provenance service run stopped at Miri isolation while creating tempfile
  state.
* After isolation was removed, focused default-Miri service
  `run-p1859968-i32529330` passed the `HashMap<TypedObject>` shared-value and
  `HeapNode`/`HeapRef` identity probes.
* The same run aborted during legacy Result/Option normalization in
  `KindedSlot::drop`, with missing TypedObject provenance. It also emitted
  integer-to-pointer warnings throughout snapshot restore. Strict provenance
  did not run because default Miri failed first.

The current paths are:

1. **Serialization.** `slot_to_serializable_ctx` at
   `snapshot.rs:1389-1400` dispatches heap values to
   `slot_heap_to_serializable`. TypedObject identity is interned at
   `:1832-1857`; TypedObject-element arrays walk each element at
   `:1887-1916`; HashMap projection starts at `:2009-2040`. These paths explain
   why a restore must preserve allocation identity and not only the numeric
   address.
2. **Pass 1 identity materialization.** `materialize_cell_bodies` at
   `:2346-2408` delegates HeapNode bodies to `materialize_node_base`.
   `materialize_typed_object_node` at `:2462-2506` allocates a shell, records
   it before filling, and resolves every field through `resolve_child`.
   `materialize_typed_object_array_node` at `:2515-2549` records a stamped
   `TypedArray<*const TypedObjectStorage>` before pushing elements.
   `materialize_typed_object_hashmap_node` at `:2560-2617` resolves values as
   `TypedObjectPtr` and records the outer `Arc<HashMapKindedRef>` afterward.
3. **Child ownership.** `resolve_child` at `:2633-2667` either materializes
   or looks up a HeapNode/HeapRef, then retains one share. It currently returns
   `(u64, NativeKind)`. The array push at `:2542-2547` and map insertion at
   `:2603-2610` reconstruct typed pointers from those bits. This is the
   correct ownership transfer in normal builds, but loses Miri provenance.
4. **Identity maps and ledger.** `RestoreLinkCtx` at `:1286-1311` stores
   `identity_map: HashMap<u64,u64>`, `heap_node_map:
   HashMap<u64,(u64,NativeKind)>`, and `retained: Vec<(u64,NativeKind)>`.
   `release_base_shares` at `:1340-1377` dispatches raw integer addresses to
   the per-carrier release primitive. The map and ledger must carry the same
   provenance as the eventual slot; otherwise the success and abort paths are
   only numerically correct.
5. **Pass 2.** `serializable_to_slot_ctx` at `:2738-2776` resolves HeapNode
   and HeapRef entries, while `link_shared_cell` and
   `link_promoted_reference` at `:2787-2857` resolve the older cell path.
   Both families currently return raw pairs. The generalized carrier should
   preserve the sidecar for both; the focused HeapNode probes primarily need
   TypedObject, TypedArray, and HashMap variants.
6. **Direct object restore.** `sv_typed_object_to_ptr` at `:3022-3048`
   recursively restores fields with `serializable_to_slot`, stores only bits
   and kinds, and calls `_new`. It must collect one provenance entry per field
   and call `_new_with_miri_field_provenance` under Miri. This covers nested
   TypedObjects, strings, TypedArrays, and other heap-bearing fields, not only
   the Result/Option failure.
7. **Typed-object arrays.** The `SV::TypedObject` arm at `:3392-3435`
   currently receives raw bits from `sv_typed_object_to_ptr` and pushes them.
   Once the field/object helper returns an owning carrier, the array must
   transfer the typed pointer represented by its sidecar into the typed pointer
   element buffer. The buffer itself is `TypedArray<*const
   TypedObjectStorage>`, so a typed pointer pushed there retains provenance.
8. **Legacy Result/Option normalization.** The `ResultData` and `OptionData`
   arms at `:3124-3154` and `:3191-3211` should build a `KindedSlot` outer
   object using the payload's sidecar. `build_builtin_variant_typed_object_slot`
   should mirror the already-correct VM helper
   `crates/shape-vm/src/executor/result_option_carrier.rs:108-140`, which saves
   `payload.miri_provenance()` and passes it to
   `_new_with_miri_field_provenance`.

## Actual Consumers of `serializable_to_slot`

These are the production consumers, excluding the in-module tests:

* `crates/shape-runtime/src/snapshot.rs`: recursive field/leaf restore at
  `:2664`, ctx fallback at `:2775`, top-level implementation at `:2904`, and
  `sv_typed_object_to_ptr` at `:3034`.
* `crates/shape-runtime/src/context/mod.rs:562-580`: restores context values
  and wraps the pair with `KindedSlot::new`. This must use the returned
  sidecar-bearing carrier directly.
* `crates/shape-vm/src/executor/snapshot.rs:321-400`: Pass 1/Pass 2 restores
  stack and module bindings. Stack installation must call
  `push_kinded_slot_preserving_miri`; module bindings need the equivalent
  sidecar-preserving write, because `module_binding_write_kinded` at
  `crates/shape-vm/src/executor/mod.rs:933-945` currently stores only the bits
  and kind arrays.
* `crates/shape-vm/src/executor/snapshot.rs:465-566`: `restore_call_stack`
  rebuilds closure captures and legacy upvalue vectors. A raw closure capture
  is another ownership boundary. It either needs a capture provenance sidecar
  in `closure_raw.rs` or must retain the existing pair path only for a
  proven scalar/no-owner capture set; silently forgetting a heap carrier here
  is not an acceptable fix.
* `crates/shape-vm/src/remote.rs:1314-1327` restores remote arguments into
  `KindedSlot`s, and `:1822-1847` restores closure captures and arguments.
  Direct arguments can consume the carrier unchanged. Closure captures have
  the same `write_capture_raw_u64` sidecar dependency as snapshot call-stack
  restore and should be a dependent stage, not an unreviewed tuple conversion.

## Recommended Carrier and Ownership Rules

Use `KindedSlot` as the canonical owning restore carrier:

```text
serializable_to_slot(...)     -> Result<KindedSlot, String>
serializable_to_slot_ctx(...) -> Result<KindedSlot, String>
```

In normal builds this remains the existing bits-plus-kind layout. Under Miri,
the carrier includes `MiriSlotProvenance`. The existing accessors
`slot()`, `kind()`, and `miri_provenance()` are sufficient; add a small
`into_raw_parts`/`into_stack` helper only where ownership transfer cannot use
`push_kinded_slot_preserving_miri`. Do not add a second `RestoredSlot` whose
`Drop` would have to duplicate `KindedSlot`'s refcount table.

For `RestoreLinkCtx`, use sidecar-bearing owning entries rather than raw `u64`:

* `identity_map` and `heap_node_map` should store an owning `KindedSlot` base
  share. The recorded `NativeKind` remains authoritative in the slot.
* Keep the LIFO release order with a ledger of map/handle identifiers, or a
  small enum identifying which map owns each base. Reverse removal from the map
  drops the sidecar-bearing `KindedSlot`; do not keep a second owning copy in
  the ledger.
* `resolve_child` and the HeapNode/HeapRef Pass-2 arms should clone the map
  entry. The clone performs the carrier-specific retain and returns the child
  `KindedSlot`; it must not call `v2_retain` after projecting the pointer to an
  integer.
* TypedObject shell field writes must use
  `write_slot_in_place_with_miri_provenance` and save each child's provenance
  in `field_provenance`. TypedArray pushes must obtain a typed
  `*const TypedObjectStorage` from the child carrier's provenance. Map values
  must do the same before constructing `TypedObjectPtr`.
* Extend `MiriSlotProvenance` in
  `crates/shape-value/src/heap_value.rs:533-539` for every pointer kind that
  the generalized restore carrier can return through these paths, at minimum
  `HashMap`; for full `RestoreLinkCtx` parity also add `SharedCell` and
  `Reference`. Add matching clone/drop branches in `kinded_slot.rs` and the
  VM stack helpers. A `None` sidecar for a non-null owning pointer must remain
  a panic, not a fallback to integer reconstruction.

The legacy `serializable_to_slot` pair shape may remain as a private scalar
projection helper during migration, but production heap restore must not call
it and then construct `KindedSlot::new`. A compatibility wrapper that returns
only `(bits, kind)` is acceptable only when it is explicitly non-owning or
restricted to inline scalar kinds.

## Staged Worker Scope

1. **Carrier and runtime restore.** Update `KindedSlot` provenance variants
   and its clone/drop constructors. Change snapshot restore return types,
   `RestoreLinkCtx` maps/ledger, `resolve_child`, the three HeapNode builders,
   `sv_typed_object_to_ptr`, and Result/Option normalization. Preserve normal
   ABI bits and kinds. Add sidecar-aware field writes, typed-array element
   transfers, map value transfers, abort cleanup, and success cleanup.
2. **VM installation.** Update `executor/snapshot.rs` to transfer carriers to
   the stack sidecar and add a module-binding sidecar/write path. Update
   `context/mod.rs` to retain the carrier directly. This is required for a
   whole-VM restore; otherwise the runtime fix is lost at the next boundary.
3. **Closure and remote boundaries.** Audit `restore_call_stack` and
   `remote.rs`. Add capture provenance storage to the closure raw carrier if
   heap captures are in scope. If it is deliberately deferred, make the
   boundary refuse heap-bearing captures under Miri and document the staged
   limitation; do not `mem::forget` a sidecar-bearing slot into a raw
   `write_capture_raw_u64` destination.
4. **Proof expansion.** Only after the default probe passes, run the existing
   runtime filter under Tree Borrows and Strict Provenance. Keep the four
   focused probes: HeapNode/HeapRef identity, Array<TypedObject> elements,
   HashMap<string, TypedObject> shared values, and Result/Option normalization.

## Cast Disposition

Normal production builds may continue to use pointer-to-`u64` conversion at
the stable slot ABI and wire boundary. Scalar integer casts and float bitcasts
also remain unrelated to provenance.

For `cfg(miri)` in all three modes (default/Stacked, Tree, and Strict), the
following live-owner casts must disappear from the targeted restore path:

* `RestoreLinkCtx` map/ledger casts in `snapshot.rs:1353-1366`, `:2481-2484`,
  `:2528-2531`, and `:2613-2616`.
* Child and container ownership casts at `:2542-2547`, `:2603-2610`, and
  `:2672-2707`.
* The typed-object field/Result/Option construction paths at `:3030-3047`
  and `:3607-3624`, where a pair currently discards provenance.
* VM installation that currently calls `KindedSlot::new` or
  `push_kinded(bits, kind)` after restore, especially
  `executor/snapshot.rs:367-391` and `context/mod.rs:577-580`.

The read-side serializer still contains integer-to-pointer casts at
`snapshot.rs:1489-1530`, `:1644-1754`, `:1850-2040`, and related opaque
carrier arms. They are outside this smallest restore-fix proof unless the
worker also threads provenance into serialization reads. Do not claim strict
provenance coverage for those paths merely because the restore probes pass.

## Focused Proofs and Supervisor Commands

The supervisor should first run the existing isolated provenance gate in its
single cargo/Miri lane, with the cgroup policy already documented by
`scripts/check-miri-provenance.sh`:

```text
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; \
    direnv exec "$PWD" env CARGO_BUILD_JOBS=2 \
    bash scripts/check-miri-provenance.sh'
```

For a focused rerun, the supervisor should use the existing
`shape-runtime --lib miri_snapshot_wire_restore_provenance` filter in each
mode, then the existing `shape-vm --lib result_option_carrier` and
`shape-vm --lib miri_stack_provenance` filters. The expected order is default
Miri first, `MIRIFLAGS=-Zmiri-tree-borrows` second, and
`MIRIFLAGS=-Zmiri-strict-provenance` last. The four runtime probe names are
`miri_snapshot_wire_restore_provenance_heapnode_heapref_typed_object_identity`,
`miri_snapshot_wire_restore_provenance_typed_array_typed_object_elements`,
`miri_snapshot_wire_restore_provenance_hashmap_typed_object_shared_values`,
and `miri_snapshot_wire_restore_provenance_result_option_normalize_to_typed_objects`.

The proof boundary is the listed restore allocations, sidecars, ownership
transfers, and drops. A passing gate is not evidence that arbitrary VM/JIT/FFI
pointer reconstruction, all snapshot carriers, all closure captures, or the
whole runtime is UB-free.

## Closeout

* **Recommended interface:** `serializable_to_slot` and its ctx variant return
  an owning sidecar-aware `KindedSlot`; normal builds retain bits plus kind.
* **Staged patch:** runtime restore and identity ledger first, VM stack/module
  installation second, closure/remote capture boundaries third, then the
  three-mode focused gate.
* **Primary changed file for the first worker:**
  `crates/shape-runtime/src/snapshot.rs`, with required sidecar support in
  `crates/shape-value/src/{heap_value.rs,kinded_slot.rs}` and VM installation
  support in `crates/shape-vm/src/{executor/snapshot.rs,executor/mod.rs}`.
* **Risk:** changing the return type exposes every raw-pair ownership boundary;
  leaving any one of them on `KindedSlot::new` recreates the same failure at a
  different carrier. Closure capture storage is the principal dependent scope.

