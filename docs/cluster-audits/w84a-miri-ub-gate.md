# W84A Miri / UB Proof-Gate Hardening

Date: 2026-07-02
Branch: `strict-flip-w84a-miri-ub-gate`
Base: `227764d0`

## Scope

This slice hardens `scripts/check-miri-provenance.sh` as a targeted Miri
provenance gate. It does not edit runtime, compiler, JIT, FFI, or Shape
program execution code.

The gate must not be described as a full "no UB" proof. A passing run means
only that Miri did not report UB for the listed test filters under the listed
Miri modes.

## Script Behavior

- Defaults `CARGO_BUILD_JOBS=2` and rejects caller-supplied values other than
  `1` or `2`.
- Uses a private `CARGO_TARGET_DIR` under
  `${XDG_CACHE_HOME:-$HOME/.cache}/shape-miri-targets` when the caller did not
  set one, and removes that private target directory on exit.
- Leaves caller-supplied `CARGO_TARGET_DIR` intact and reports that cleanup is
  disabled for it.
- Prints the exact probe coverage and each cargo-miri command before running.
- Keeps the existing `rustup run nightly cargo miri test ...` form because the
  active devenv cargo is stable.

## Targeted Coverage

| Crate/filter | Miri modes |
|---|---|
| `shape-value --lib provenance` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows` |
| `shape-vm --lib result_option_carrier` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows`; `-Zmiri-strict-provenance` |
| `shape-vm --lib get_prop_typed_object_int_field_reads_via_raw` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows` |

Not covered by this gate: full VM execution, runtime type-system code, JIT code,
FFI paths, snapshots, arbitrary Shape programs, all heap carriers, and all raw
pointer consumers. Those require separate targeted Miri probes or other proof
work.

## Worker Checks

This worker was constrained not to run `cargo`, `rustc`, `cargo miri`,
`nextest`, `just`, or Shape binaries. Verification for this slice is therefore
limited to shell syntax and text checks. The supervisor must run the Miri gate
after merge under the serialized cgroup lane.

## Supervisor Commands After Merge

Run the Miri/provenance gate in the merge worktree:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
```

If rerunning the curated VM-vs-JIT gate from W83C, keep it in the same
serialized cgroup policy:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
  --setenv=PATH="$PATH" \
  bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 just differential-gate'
```
