# W86C Ignored Tests and Miri Classification

Date: 2026-07-02
Branch: `strict-flip-w86c-ignored-miri-classification`
Base: `6ad7dd1a`

## Scope

This slice answers the reviewer concern that "ignored tests and Miri are not a
full proof" by making both boundaries explicit and checkable.

W87C follow-up: `docs/cluster-audits/w87c-ignored-active-gap-triage.md`
contains the per-test active-gap/stale inventory and reclassifies two stale
rows as deleted v1 paths.

No ignored tests were unignored. In particular, process-aborting `extern "C"`
SURFACE tests stay ignored until their underlying todo bodies are replaced by
non-aborting result paths.

## Count Baseline

The supervisor-observed broad lib-test gates currently report:

| Crate | Reported ignored in `--lib` gate | Source `#[ignore]` attrs scanned here | Source-only gated attrs |
|---|---:|---:|---:|
| `shape-vm` | 57 | 121 | 39 behind `deep-tests` |
| `shape-jit` | 26 | 29 | 2 behind `deep-tests`, 1 behind `cfg(any())` |

This worker did not rerun cargo or nextest. The new checker is intentionally
source-only and cheap; it guards the source ignore count and reason taxonomy
without requiring a cargo test listing.

For `shape-vm`, the 121 source attributes do not mechanically reduce to the
reported 57 ignored lib tests from source-level module gates alone. Resolving
that exact active-harness projection requires a cargo test listing, which this
slice intentionally did not run. The enforceable invariant added here is the
source-level taxonomy; the 57/26 counts remain the supervisor-observed lib gate
baseline.

## Cause Taxonomy

`scripts/check-ignored-test-classification.py` enforces the following
source-level baseline:

| Cause | `shape-vm` | `shape-jit` | Meaning |
|---|---:|---:|---|
| `phase_2c_surface` | 93 | 0 | Deferred strict-flip surfaces such as state snapshots, comptime host conversion, typed annotations, iterator materialization, and host-tier eval/marshal rebuilds. |
| `deleted_v1_path` | 5 | 21 | Tests still describe removed carriers or paths such as BytecodeToIR, JitArray, deleted NaN-box roundtrips, deleted native-pointer helpers, deleted TypedArrayData enum paths, v1 VMArray aliasing, or retired Tier 1 whole-function JIT. |
| `process_aborting_extern_c_todo` | 0 | 3 | `extern "C"` functions currently hit `todo!()`/SURFACE bodies; attempting `#[should_panic]` would abort the test process. |
| `stale_semantic_expectation` | 4 | 0 | Assertions tied to stale strict-solver diagnostics, stale extern-C sugar, or stale Numeric trait shape. |
| `active_feature_gap` | 18 | 5 | Real feature gaps or known bugs: generic/module/method resolution, const-specialization, turbofish grammar, MIR reference escape, JIT kernel stubs, and JIT closure-cell bugs. |
| `diagnostic_only` | 1 | 0 | A local debug-only opcode tracing test. |
| Total | 121 | 29 | Source inventory, not a cargo-run proof. |

The source-only gated subset is also classified:

| Crate | Gated category contribution |
|---|---|
| `shape-vm` | 30 `phase_2c_surface`, 7 `active_feature_gap`, 1 `deleted_v1_path`, 1 `stale_semantic_expectation` behind `deep-tests`. |
| `shape-jit` | 2 `active_feature_gap` behind `deep-tests`; 1 `deleted_v1_path` behind `cfg(any())`. |

## Process-Aborting Ignores

These should not be casually unignored:

| Test | Cause |
|---|---|
| `crates/shape-jit/src/ffi_symbols/simulation/mod.rs::test_simulation_with_function_handler` | Calls `jit_call_value`, an `extern "C"` value-call SURFACE/todo path. |
| `crates/shape-jit/src/ffi/async_ops.rs::test_cancel_task_null_trampoline` | Calls `jit_cancel_task`, an `extern "C"` future-classification SURFACE/todo path. |
| `crates/shape-jit/src/ffi/control/mod.rs::native_fixed_arity_helpers_surface_pending_kinded_abi` | Calls `jit_call_foreign_native_0`, an `extern "C"` foreign-call SURFACE/todo path. |

Re-enable only after the underlying function bodies return structured results
or errors instead of panicking across the non-unwinding ABI boundary.

## Miri Boundary

`scripts/check-miri-provenance.sh` runs targeted probes only:

| Probe | Modes |
|---|---|
| `shape-value --lib provenance` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows` |
| `shape-vm --lib result_option_carrier` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows`; `-Zmiri-strict-provenance` |
| `shape-vm --lib get_prop_typed_object_int_field_reads_via_raw` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows` |

Passing this gate is evidence for only those filters and modes. It is not a
full UB-free proof for the VM, runtime, JIT, FFI, snapshots, arbitrary Shape
program execution, all heap carriers, all raw pointer consumers, or ignored
tests.

## Next Miri Candidate

A narrow low-risk next probe would be the sibling property-access test:

```bash
cargo miri test -p shape-vm --lib get_prop_typed_object_string_field_reads_via_raw
```

Run it first in the serialized Miri lane under both default Miri and
`MIRIFLAGS=-Zmiri-tree-borrows`. If it passes there, add it to
`scripts/check-miri-provenance.sh`. This slice did not add it because an
unrun Miri probe would make the gate aspirational rather than enforceable.

## Checker

Run:

```bash
scripts/check-ignored-test-classification.py
```

The checker fails when:

- a new source-level `#[ignore]` appears without matching the taxonomy;
- a known ignored-test count moves between cause buckets;
- a new ignored test has no reason string, except the two explicitly allowed
  legacy cases (`test_nested_generic_call` and `debug_decimal_opcodes`);
- the source-only gated baseline drifts.
