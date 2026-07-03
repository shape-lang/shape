# W93D Miri Unsafe-Proof Boundary

Date: 2026-07-03
Branch: `strict-flip-w93d-miri-unsafe-proof-next`

## Scope

W92 proved targeted provenance gates, not global no-UB. This slice makes the
next boundary concrete by adding one self-contained Miri probe and documenting
the remaining high-value unsafe/provenance surfaces.

The claim remains narrow: a passing gate means only that Miri did not report UB
for the listed filters under the listed modes.

## Added Probe

`crates/shape-value/src/heap_value.rs` now has a `#[cfg(miri)]` test:

| Probe | Evidence boundary |
|---|---|
| `miri_typed_object_nested_field_clone_and_drop` | An outer v2-raw `TypedObjectStorage` field can own a nested v2-raw `TypedObjectStorage` pointer when the field sidecar carries `MiriSlotProvenance::TypedObject`; `clone_field_kinded`, `KindedSlot::Clone` / `Drop`, and outer `drop_fields` preserve and release the right shares. |

`scripts/check-miri-provenance.sh` runs this probe under default Miri /
Stacked Borrows, Tree Borrows, and strict provenance.

## Ranked Remaining Surfaces

1. `TypedArray` raw carriers in `KindedSlot`, `TypedObjectStorage`, and stack
   retain/release tables. These are high risk because many constructors still
   store `arr as u64` and later reconstruct `*mut u8`; a strict-provenance
   probe is not safe until a typed-array provenance sidecar or pointer-carrying
   constructor exists.
2. VM module-return `JsonValue` object/array projection. Existing tests named
   `json_value_*_payload_clone_preserves_field_provenance` exercise real
   nested `TypedObjectPtr`, HashMap, and `TypedArray<*const TypedObjectStorage>`
   paths, but they are VM-heavy and were not added to the gate here.
3. `TraitObjectStorage` raw inner `TypedObjectStorage` plus vtable carrier.
   It mirrors the typed-object raw-pointer shape but has separate release and
   drop code; add a dedicated Miri sidecar/probe before claiming coverage.
4. Snapshot/wire restore and JIT/FFI return boundaries. These remain outside
   current Miri coverage and may cross process/ABI or old-carrier paths; they
   need separate proof slices, not inclusion in this narrow gate.

## Boundary Not Proved

This does not prove global UB freedom for Shape. It does not cover arbitrary
Shape program execution, all stack overwrite call sites, all typed-object field
producers, `TypedArray` field carriers, trait objects, snapshots, wire
conversion, JIT, FFI, or ignored tests.

## Worker Verification

Per W93D constraints, this worker must not run cargo, rustc, nextest, Miri,
`just test-all`, or book truth. Verification is limited to static shell/text
checks; the supervisor owns serialized Miri execution in cgroups.

## Supervisor Commands

Full targeted Miri gate:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w93d-miri-unsafe-proof-next; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

Isolated W93D probe:

```bash
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w93d-miri-unsafe-proof-next; direnv exec "$PWD" env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w93d-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-value --lib miri_typed_object_nested_field_clone_and_drop'
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w93d-miri-unsafe-proof-next; direnv exec "$PWD" env MIRIFLAGS=-Zmiri-tree-borrows CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w93d-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-value --lib miri_typed_object_nested_field_clone_and_drop'
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w93d-miri-unsafe-proof-next; direnv exec "$PWD" env MIRIFLAGS=-Zmiri-strict-provenance CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w93d-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-value --lib miri_typed_object_nested_field_clone_and_drop'
```
