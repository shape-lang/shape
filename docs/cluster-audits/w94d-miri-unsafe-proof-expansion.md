# W94D Miri Unsafe-Proof Expansion

Date: 2026-07-03
Branch: `strict-flip-w94d-miri-unsafe-proof-expansion`
Baseline: `f2dfb581`

## Scope

This slice expands the targeted Miri/provenance proof surface by one narrow
probe. It does not claim global UB absence. A passing supervisor run means only
that Miri did not report UB for the listed test filter under the listed Miri
modes.

## Added Probe

`crates/shape-vm/src/executor/result_option_carrier.rs` now has a
`#[cfg(miri)]` test:

| Probe | Evidence boundary |
|---|---|
| `miri_result_option_typed_object_payload_clone_and_drop` | A schema-backed `__Option.Some` carrier can own a v2-raw `TypedObjectStorage` payload while the Miri provenance sidecar transfers through `build_variant_object`, `clone_payload`, `KindedSlot::Clone` / `Drop`, and final carrier drop. The probe checks the v2 raw refcount path, not arbitrary Shape program execution. |

This fills the highest-value gap left after W93D: W93D covered a plain nested
TypedObject field sidecar, while W94D covers the same raw typed-object payload
when it is embedded in the canonical schema-backed Result/Option carrier.

`scripts/check-miri-provenance.sh` already runs the
`shape-vm --lib result_option_carrier` filter under default Miri / Stacked
Borrows, Tree Borrows, and strict provenance. The coverage text now names the
new typed-object payload probe explicitly.

## Ranked Remaining Surfaces

1. `TypedArray<T>` raw carriers remain the largest unproven provenance surface.
   They still need a pointer-preserving Miri sidecar or a safe isolated
   constructor before a strict-provenance probe can be meaningful.
2. Trait-object raw inner `TypedObjectStorage` plus vtable payload remains
   separate from the Result/Option and direct TypedObject probes.
3. Snapshot/wire restore and JIT/FFI return boundaries remain outside current
   Miri coverage because they cross old carrier, ABI, or process-facing paths.

## Boundary Not Proved

This does not prove Shape is UB-free. It does not cover arbitrary Shape program
execution, all stack overwrite sites, all typed-object field producers, all
typed-array carriers, trait objects, snapshots, wire conversion, JIT, FFI,
ignored tests, or concurrency behavior.

## Worker Verification

Per W94D constraints, this worker did not run cargo, rustc, nextest, Miri,
`just`, build commands, `shape-test`, or any test binary. Verification is
limited to static shell/text checks; the supervisor owns serialized Miri
execution in cgroups.

## Supervisor Commands

Full targeted Miri gate:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w94d-miri-unsafe-proof-expansion; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

Isolated W94D probe under the three intended Miri modes:

```bash
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w94d-miri-unsafe-proof-expansion; for flags in "" "-Zmiri-tree-borrows" "-Zmiri-strict-provenance"; do if [ -n "$flags" ]; then direnv exec "$PWD" env MIRIFLAGS="$flags" CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w94d-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib miri_result_option_typed_object_payload_clone_and_drop; else direnv exec "$PWD" env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w94d-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib miri_result_option_typed_object_payload_clone_and_drop; fi; done'
```
