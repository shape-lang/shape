# W92A Miri Stack-Overwrite Provenance

Date: 2026-07-03
Branch: `strict-flip-w92a-miri-stack-overwrite`

## Scope

This slice addresses the W91C-documented gap that stack overwrite had no
provenance-bearing incoming write route. The claim remains narrow: a passing
Miri run is evidence for the listed stack-sidecar probes and modes only.

## Implementation

`crates/shape-vm/src/executor/vm_impl/stack.rs` now has a Miri-only
`stack_write_kinded_with_miri_provenance(idx, bits, kind, provenance)` helper.
It drops the overwritten slot using the old slot's sidecar provenance, writes
the fresh `(u64, NativeKind)` payload, and stores the caller-supplied incoming
`MiriSlotProvenance`.

The existing `stack_write_kinded(idx, bits, kind)` ABI is unchanged. Under
Miri, it routes through the new helper with `MiriSlotProvenance::None`. This
keeps missing provenance visible to Miri probes and does not add runtime
inference, tag probing, or kind-from-bits logic.

## Added Probe

The existing `shape-vm --lib miri_stack_provenance` filter now includes:

| Probe | Evidence boundary |
|---|---|
| `miri_stack_provenance_string_overwrite_and_drop` | Explicit incoming `Arc<String>` provenance survives stack overwrite; the old slot drops through old provenance; the fresh slot can be owning-read and later truncated without reconstructing a pointer from integer bits. |

## Boundary Not Expanded

This is not a full UB-free proof for the VM, all stack overwrite call sites,
all heap carriers, runtime/JIT/FFI boundaries, snapshots, arbitrary Shape
program execution, or ignored tests.

## Worker Verification

Per W92A instructions, this worker did not run `cargo`, `rustc`, `nextest`, or
Miri. Verification is limited to static inspection and formatting checks. The
supervisor owns serialized cargo/Miri execution in cgroups.

## Supervisor Commands

Full targeted gate:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w92a-miri-stack-overwrite; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

Isolated W92A filter:

```bash
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w92a-miri-stack-overwrite; direnv exec "$PWD" env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w92a-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib miri_stack_provenance'
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w92a-miri-stack-overwrite; direnv exec "$PWD" env MIRIFLAGS=-Zmiri-tree-borrows CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w92a-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib miri_stack_provenance'
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w92a-miri-stack-overwrite; direnv exec "$PWD" env MIRIFLAGS=-Zmiri-strict-provenance CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w92a-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib miri_stack_provenance'
```
