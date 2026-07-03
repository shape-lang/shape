# W91C Miri Proof-Boundary Expansion

Date: 2026-07-03
Branch: `strict-flip-w91c-miri-proof-expansion`

## Scope

This slice expands the targeted Miri provenance gate without broadening its
claim. A passing run is evidence only for the listed probes and Miri modes. It
is not a full UB proof for the VM, runtime, JIT, FFI, snapshots, arbitrary Shape
programs, all heap carriers, all raw-pointer consumers, or ignored tests.

## Added Probes

W91C adds the `shape-vm --lib miri_stack_provenance` filter. It contains two
`cfg(miri)` tests in `crates/shape-vm/src/executor/vm_impl/stack.rs`:

| Probe | Evidence boundary |
|---|---|
| `miri_stack_provenance_string_read_pop_and_truncate` | The VM stack's Miri sidecar preserves `Arc<String>` pointer provenance through `push_kinded_with_miri_provenance`, `read_owned_kinded`, `pop_kinded_with_miri_provenance`, and `truncate_stack`. |
| `miri_stack_provenance_typed_object_read_and_pop` | The same sidecar preserves v2-raw `TypedObjectStorage` provenance through owning read and pop/drop while exercising HeapHeader refcount retain/release. |

These are high-value because W90's existing probes cover shape-value
provenance anchors, Result/Option carriers, and typed-object raw property
reads, but not the VM stack's own provenance sidecar. The stack sidecar is the
boundary that lets Miri runs keep pointer provenance next to the unchanged
`(u64, NativeKind)` stack ABI.

## Gate Change

`scripts/check-miri-provenance.sh` now runs:

| Probe | Modes |
|---|---|
| `shape-vm --lib miri_stack_provenance` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows`; `-Zmiri-strict-provenance` |

This remains targeted evidence only. Do not summarize a passing gate as
"UB-free".

## Boundary Not Expanded

W91C did not add a stack-overwrite probe for fresh heap pointers. The current
`stack_write_kinded(idx, bits, kind)` API has no Miri provenance-bearing
incoming write parameter. A test that writes a new `String` or `TypedObject`
through that API would leave the new slot with `MiriSlotProvenance::None` and
exercise a deliberate missing-sidecar panic rather than a valid provenance
contract.

Future stack-overwrite evidence should first add or route through an API that
transfers the incoming `MiriSlotProvenance`, then probe overwrite/drop under
Stacked Borrows, Tree Borrows, and strict provenance.

## Worker Verification

Per W91C instructions, this worker did not run `cargo`, `rustc`, `nextest`, or
Miri. Verification here is limited to shell/static inspection. The supervisor
owns serialized Miri execution.

## Supervisor Commands

Full targeted gate:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w91c-miri-proof-expansion; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

If the supervisor wants to isolate the new W91C filter first:

```bash
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w91c-miri-proof-expansion; direnv exec "$PWD" env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w91c-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib miri_stack_provenance'
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w91c-miri-proof-expansion; direnv exec "$PWD" env MIRIFLAGS=-Zmiri-tree-borrows CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w91c-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib miri_stack_provenance'
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w91c-miri-proof-expansion; direnv exec "$PWD" env MIRIFLAGS=-Zmiri-strict-provenance CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w91c-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib miri_stack_provenance'
```
