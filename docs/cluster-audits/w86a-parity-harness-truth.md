# W86A Parity Harness Truth Audit

Date: 2026-07-02

## Finding

`crates/shape-vm/src/feature_tests/backends.rs` previously exposed a
`JITBackend` named `JIT`, but its `execute` implementation delegated to
`crate::BytecodeExecutor::new()`. That made the legacy feature-test matrix look
like VM-vs-JIT evidence while exercising only the VM.

## Disposition

The feature-test JIT lane is retired and now skips closed. `shape-vm` cannot
narrowly instantiate the real `shape_jit::JITExecutor` because `shape-jit`
already depends on `shape-vm`; linking it back into this crate would introduce a
crate cycle and expand the change beyond the harness truth fix.

The maintained JIT evidence remains the subprocess differential gate:

- `scripts/differential-gate.sh`
- `.github/workflows/ci.yml` step `VM vs JIT differential gate`

That gate runs real `shape run --mode jit` through `shape-fuzz run` against the
curated per-commit corpus. This patch does not weaken or edit that gate.

## Code Changes

- `JITBackend` no longer delegates to `BytecodeExecutor`.
- The default feature-test runner reports the JIT lane as skipped with a pointer
  to `scripts/differential-gate.sh`.
- Text, JSON, markdown, and CLI labels now describe the matrix as a legacy
  in-process feature matrix rather than three-way JIT parity evidence.

## Classification

Residual risk is documentation/reporting only: this harness still does not run
real JIT code. That is intentional. Real VM-vs-JIT parity coverage belongs to
the existing subprocess differential gate.
