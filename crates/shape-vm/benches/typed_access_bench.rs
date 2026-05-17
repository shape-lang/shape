//! Benchmarks comparing typed (LoadCol*) vs dynamic (GetProp) column access
//!
//! Tests the performance difference between compile-time resolved column access
//! (LoadColF64/LoadColStr) and runtime dynamic property access (GetProp).
//!
//! # Phase 2d Item 5 — surface-and-stop
//!
//! The four bench workloads (`load_col_f64`, `get_prop_f64`, `load_col_str`,
//! `get_prop_str`) each pushed a `DataTable` `RowView` onto the VM stack via
//! a `Constant::Value(ValueWord)` constant-pool entry, then exercised
//! `LoadColF64` / `LoadColStr` / `GetProp` against that RowView.
//!
//! The injection surface no longer exists post-strict-typing:
//!
//! - `shape_value::ValueWord::from_row_view(...)` — DELETED (CLAUDE.md
//!   "Forbidden Patterns": `ValueWord` at runtime is deleted; the entire
//!   tagged-word dynamic-fallback path is gone).
//! - `shape_value::ValueWordExt` — DELETED (same).
//! - `Constant::Value(ValueWord)` — the variant survives as a unit-shape
//!   `Constant::Value` placeholder; `op_push_const` in
//!   `executor/stack_ops/mod.rs` returns a `RuntimeError("unsupported
//!   constant variant in PushConst")` for any constant whose runtime
//!   carrier was not migrated to the per-kind heap-Arc surface. RowView
//!   is one such carrier — there is no `Constant::RowView` arm and no
//!   public bench-tier path that pushes a `KindedSlot { kind: Ptr(TableView), ... }`
//!   onto the stack from outside the crate.
//! - `VirtualMachine::push_value(KindedSlot)` — public on the VM, but the
//!   body is `todo!("phase-2c — see ADR-006 §2.7.4: push_value(KindedSlot)
//!   is a host-boundary helper; the in-VM surface uses push_kinded(bits,
//!   kind) sourced directly from the producer.")`.
//! - `VirtualMachine::push_kinded(...)` — exists and is the correct entry
//!   point, but its visibility is `pub(crate)` so the bench (which uses
//!   shape-vm as an external crate) cannot call it.
//!
//! All four workloads are workload-semantic blockers, not API renames: the
//! migration playbook (`docs/cluster-audits/phase-2d-playbook.md` §0 +
//! handover §1 Item 5) bounds diff scope to "API migration only — do NOT
//! change the workload itself". Adding a new `Constant::RowView(Arc<TableViewData>)`
//! variant or exposing `push_kinded` as `pub` falls under "crate-level
//! setup" (out of scope) and would constitute a host-tier API design
//! ruling for the heap-Arc constant-injection path.
//!
//! Per Item 5 surface-and-stop instructions, the bench-target file remains
//! present and compiles cleanly so the bench binary builds (`cargo bench
//! --no-run -p shape-vm` exits 0). The four benches are NOT registered
//! into the criterion group — there is nothing meaningful to measure
//! without an injection surface for the RowView setup.
//!
//! # Re-enabling
//!
//! When the host-tier eval/marshal API rebuild (ADR-006 §2.7.4) lands and
//! `Constant::*` grows a kinded heap-Arc carrier — i.e. the
//! `Constant::Value(ValueWord)` surface in `executor/tests/typed_array_ops.rs`
//! and `executor/tests/matrix_ops.rs` is filled — port the four bench
//! workloads onto the new constant-injection surface. The workloads
//! themselves (Arrow-backed 10K-row table; `LoadColF64`/`LoadColStr` vs
//! `GetProp`; 100 row-index samples per group) must NOT be modified;
//! CLAUDE.md "Benchmark Integrity" is binding.
//!
//! ADR-006 §2.7.4 cite: "host-tier eval/marshal API rebuild — deleted
//! ValueWord/Constant::Value(ValueWord) carrier".

use criterion::{Criterion, criterion_group, criterion_main};

/// Stub benchmark group — registers an empty group so the bench-target
/// binary compiles and `criterion_main!` has something to wire up. See the
/// module-level doc-comment for the surface-and-stop rationale.
fn benchmark_typed_vs_dynamic(c: &mut Criterion) {
    let group = c.benchmark_group("typed_vs_dynamic_access");
    // SURFACE — Phase 2d Item 5: see module doc-comment + ADR-006 §2.7.4.
    // Re-add `load_col_f64`, `get_prop_f64`, `load_col_str`, `get_prop_str`
    // here once `Constant::Value(ValueWord)` is replaced by a kinded
    // heap-Arc constant variant and a bench-tier `RowView` injection
    // surface is available.
    group.finish();
}

criterion_group!(benches, benchmark_typed_vs_dynamic);
criterion_main!(benches);
