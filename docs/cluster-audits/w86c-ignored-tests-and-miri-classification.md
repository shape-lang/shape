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

W91B follow-up: the remaining four `stale_semantic_expectation` ignores were
redriven with real focused execution and reclassified as `active_feature_gap`.
The checker now accepts zero stale expectations.

W92B follow-up: three `shape-vm` active gaps in `compiler/functions.rs` were
implemented statically and unignored: native-ABI `out` parameter caller-visible
arity/return typing, and direct internal-intrinsic scope diagnostics before
strict type solving.

W92C follow-up: two module-qualified type active gaps were unignored after
static module-qualified trait-method lowering and let-annotation identity were
proven by source-level tests.

W92D follow-up: `test_nested_generic_call` was unignored after being rewritten
to current native PHF `flatten()` dispatch semantics. The source checker now
accepts 113 `shape-vm` ignores and 14 `shape-vm` active feature gaps.

The process-aborting `extern "C"` SURFACE tests stay ignored until their
underlying todo bodies are replaced by non-aborting result paths.

## Count Baseline

The supervisor-observed broad lib-test gates currently report:

| Crate | Reported ignored in `--lib` gate | Source `#[ignore]` attrs scanned here | Source-only gated attrs |
|---|---:|---:|---:|
| `shape-vm` | 57 | 113 | 57 behind `deep-tests` |
| `shape-jit` | 26 | 29 | 2 behind `deep-tests`, 1 behind `cfg(any())` |

This worker did not rerun cargo or nextest. The new checker is intentionally
source-only and cheap; it guards the source ignore count and reason taxonomy
without requiring a cargo test listing.

For `shape-vm`, the 113 source attributes do not mechanically reduce to the
reported 57 ignored lib tests from source-level module gates alone. Resolving
that exact active-harness projection requires a cargo test listing, which this
slice intentionally did not run. The enforceable invariant added here is the
source-level taxonomy; the 57/26 counts remain the supervisor-observed lib gate
baseline.

W92 supervisor verification additionally ran the deep-test VM gate after the
W92B/C/D closures: `shape-vm --lib --features deep-tests --no-fail-fast`
passed 2794/0/113 ignored in `run-p39810-i196101.service`. That deep-test
ignored count matches the source inventory above; the default lib ignored count
remains separately documented from the W91 default gate.

## Cause Taxonomy

`scripts/check-ignored-test-classification.py` enforces the following
source-level baseline:

| Cause | `shape-vm` | `shape-jit` | Meaning |
|---|---:|---:|---|
| `phase_2c_surface` | 93 | 0 | Deferred strict-flip surfaces such as state snapshots, comptime host conversion, typed annotations, iterator materialization, and host-tier eval/marshal rebuilds. |
| `deleted_v1_path` | 5 | 21 | Tests still describe removed carriers or paths such as BytecodeToIR, JitArray, deleted NaN-box roundtrips, deleted native-pointer helpers, deleted TypedArrayData enum paths, v1 VMArray aliasing, or retired Tier 1 whole-function JIT. |
| `process_aborting_extern_c_todo` | 0 | 3 | `extern "C"` functions currently hit `todo!()`/SURFACE bodies; attempting `#[should_panic]` would abort the test process. |
| `stale_semantic_expectation` | 0 | 0 | No accepted stale expectations remain; future rows are new drift. |
| `active_feature_gap` | 14 | 5 | Real feature gaps or known bugs: generic/method resolution, matrix/vector retargeting, const-specialization, turbofish grammar, MIR reference escape, JIT kernel stubs, and JIT closure-cell bugs. |
| `diagnostic_only` | 1 | 0 | A local debug-only opcode tracing test. |
| Total | 113 | 29 | Source inventory, not a cargo-run proof. |

The source-only gated subset is also classified:

| Crate | Gated category contribution |
|---|---|
| `shape-vm` | 45 `phase_2c_surface`, 10 `active_feature_gap`, 2 `deleted_v1_path` behind `deep-tests`. |
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
| `shape-value --lib miri_typed_object_nested_field_clone_and_drop` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows`; `-Zmiri-strict-provenance` |
| `shape-vm --lib result_option_carrier` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows`; `-Zmiri-strict-provenance` |
| `shape-vm --lib get_prop_typed_object_int_field_reads_via_raw` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows` |
| `shape-vm --lib get_prop_typed_object_string_field_reads_via_raw` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows` |
| `shape-vm --lib miri_stack_provenance` | default Miri / Stacked Borrows; `-Zmiri-tree-borrows`; `-Zmiri-strict-provenance` |

Passing this gate is evidence for only those filters and modes. It is not a
full UB-free proof for the VM, runtime, JIT, FFI, snapshots, arbitrary Shape
program execution, all heap carriers, all typed-object field producers, all
raw pointer consumers, or ignored tests.

## W87A Miri Expansion

W87A ran the sibling string-field property-access probe before adding it to the
gate:

```bash
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w87a-miri-string-provenance; direnv exec "$PWD" env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w87a-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib get_prop_typed_object_string_field_reads_via_raw'
systemd-run --user --wait --collect --pipe -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 --setenv=PATH="$PATH" bash -c 'set -euo pipefail; cd /home/dev/dev/shape-lang/shape-strict-flip-w87a-miri-string-provenance; direnv exec "$PWD" env MIRIFLAGS=-Zmiri-tree-borrows CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/shape-w87a-miri-target /home/dev/.cargo/bin/rustup run nightly cargo miri test -p shape-vm --lib get_prop_typed_object_string_field_reads_via_raw'
```

Both modes passed with `1 passed; 0 failed; 2467 filtered out`. This is targeted
evidence for that string-field raw-read path only, not a proof that every typed
object string consumer, heap carrier, VM path, or ignored test is UB-free.

## W91C Miri Expansion

W91C adds the `shape-vm --lib miri_stack_provenance` filter to the gate. The
filter contains two `cfg(miri)` stack-sidecar probes:

- `miri_stack_provenance_string_read_pop_and_truncate` covers legacy
  `Arc<String>` pointer provenance through `push_kinded_with_miri_provenance`,
  `read_owned_kinded`, `pop_kinded_with_miri_provenance`, and
  `truncate_stack`.
- `miri_stack_provenance_typed_object_read_and_pop` covers v2-raw
  `TypedObjectStorage` provenance through the same owning-read and pop/drop
  boundary, including HeapHeader refcount retain/release.

The gate runs this filter under default Miri / Stacked Borrows, Tree Borrows,
and strict provenance. Passing it is targeted evidence for the VM stack Miri
sidecar paths above only. It is not a full UB-free proof for stack overwrites,
full VM execution, snapshots, JIT/FFI boundaries, all heap carriers, or ignored
tests.

At W91C close, it did not add a stack-overwrite probe for a fresh heap pointer:
`stack_write_kinded(idx, bits, kind)` has no Miri provenance-bearing incoming
write API. A future overwrite probe should first add or route through an API
that transfers the incoming `MiriSlotProvenance`; otherwise the new stack slot
would intentionally carry `None` provenance and exercise the wrong contract.

## W92A Miri Stack-Overwrite Expansion

W92A resolves the W91C-documented stack-overwrite sidecar gap by adding a
Miri-only `stack_write_kinded_with_miri_provenance(...)` overwrite API. The
regular `stack_write_kinded(idx, bits, kind)` ABI remains unchanged; under Miri
it routes through the new helper with `MiriSlotProvenance::None` rather than
inferring provenance from raw bits.

The `shape-vm --lib miri_stack_provenance` filter now also includes
`miri_stack_provenance_string_overwrite_and_drop`. That probe overwrites an
existing `Arc<String>` stack slot with a fresh `Arc<String>` pointer while
passing explicit incoming provenance, checks that the old slot is dropped
through its old provenance, then reads and truncates the fresh slot through the
transferred sidecar.

Passing the expanded filter remains targeted evidence for the stack sidecar
paths above only. It is not a full UB-free proof for the VM, arbitrary stack
overwrite call sites, JIT/FFI boundaries, snapshots, all heap carriers, or
ignored tests.

## W93D Miri Nested TypedObject Field Expansion

W93D adds a Miri-only shape-value probe named
`miri_typed_object_nested_field_clone_and_drop` and runs it under default
Miri / Stacked Borrows, Tree Borrows, and strict provenance.

The probe constructs an outer v2-raw `TypedObjectStorage` whose field owns a
nested v2-raw `TypedObjectStorage` pointer. The test supplies explicit
`MiriSlotProvenance::TypedObject` field sidecar data, then exercises
`clone_field_kinded`, `KindedSlot::Clone` / `Drop` for
`Ptr(HeapKind::TypedObject)`, and finally outer `drop_fields` releasing the
original nested field share.

Passing this probe is targeted evidence for that nested typed-object field
sidecar path only. It does not prove every typed-object field producer,
TypedArray field carrier, TraitObject carrier, HashMap object payload,
snapshot/wire restore path, or VM/JIT/FFI boundary is UB-free.

## Checker

Run:

```bash
scripts/check-ignored-test-classification.py
```

The checker fails when:

- a new source-level `#[ignore]` appears without matching the taxonomy;
- a known ignored-test count moves between cause buckets;
- a new ignored test has no reason string, except the one explicitly allowed
  diagnostic-only legacy case (`debug_decimal_opcodes`);
- the source-only gated baseline drifts.
