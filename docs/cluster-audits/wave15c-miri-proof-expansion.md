# Wave-15C Miri Proof Expansion

Date: 2026-07-09
Worker: Wave-15C Miri/proof expansion

## Scope

This slice expands the targeted Miri/provenance proof surface. It does not
claim global no-UB for Shape. A passing supervisor run means only that Miri did
not report UB for the listed filters under the listed Miri modes.

The work keeps production runtime behavior unchanged. The implementation hook
is a tiny `#[cfg(miri)]` sidecar expansion so strict-provenance probes can
carry the original raw pointers next to the unchanged `u64` slot ABI.

## Added Probes

| Probe | Evidence boundary |
|---|---|
| `miri_typed_array_field_clone_and_drop` | A scalar `TypedArray<i64>` carrier stored as a `TypedObjectStorage` field can transfer Miri pointer provenance through `clone_field_kinded`, `KindedSlot::Clone` / `Drop`, and final `TypedObjectStorage` field release. This covers the carrier header refcount path, not heap-element array children. |
| `miri_stack_provenance_typed_array_read_and_pop` | The VM stack sidecar preserves a `Ptr(HeapKind::TypedArray)` carrier through `push_kinded_with_miri_provenance`, `read_owned_kinded`, pop, and drop. This covers stack retain/release for the array header only. |
| `miri_trait_object_raw_carrier_clone_and_drop` | A raw `TraitObjectStorage` carrier can be cloned and dropped through `KindedSlot::from_trait_object_raw`, retaining/releasing the outer trait-object header and retiring its inner `TypedObjectStorage` plus `Arc<VTable>` shares on final release. |

## Gate Change

`scripts/check-miri-provenance.sh` now runs the two new shape-value filters
under default Miri / Stacked Borrows, Tree Borrows, and strict provenance:

- `shape-value --lib miri_typed_array_field_clone_and_drop`
- `shape-value --lib miri_trait_object_raw_carrier_clone_and_drop`

The existing `shape-vm --lib miri_stack_provenance` filter now includes the
new typed-array stack read/pop/drop probe and continues to run under the same
three modes.

## Boundary Not Proved

This is still targeted evidence only. It does not prove arbitrary Shape
program execution, all typed-array producers, heap-element arrays, nested array
children, arbitrary trait method dispatch, trait-object arrays, snapshot/wire
restore, JIT, FFI, GC cycle paths, concurrency behavior, ignored tests, or a
global VM/runtime no-UB property.

The highest remaining proof gaps are:

- snapshot/wire restore provenance for typed objects, typed arrays, trait
  objects, Result/Option, and JsonValue object/array carriers;
- heap-element `TypedArray` probes for string, typed-object, trait-object, and
  nested-array element release;
- trait-object dispatch probes beyond carrier clone/drop, including vtable
  lookup through `DynMethodCall` and self-return wrapping;
- JIT/FFI boundary values, where Miri can only reach wrapper code and cannot
  execute the native JIT surface as a whole.

## Worker Verification

This worker did not run cargo, rustc, just, nextest, shape-test, Miri, build
commands, or book-truth. Verification is limited to static shell/text checks.
The supervisor owns the serialized cgroup build/test lane.

## Supervisor Command

Full targeted Miri gate:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```
