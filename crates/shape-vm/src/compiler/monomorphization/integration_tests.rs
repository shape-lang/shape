// ADR-006 §2.7.4 / §2.7.7 — Phase 2c deferral.
//
// The original integration test harness exercised the full
// monomorphization pipeline through `eval()` / `run_program()` whose
// return type is the deleted `shape_value::ValueWord`. It asserted
// against the carrier via `ValueWordExt::as_i64`. Per playbook §7
// REVISED #2 / #4 the correct surface for a non-migratable test site
// is to replace the file body with a `cfg(any())`-gated `todo!()`
// placeholder rather than re-introduce the carrier.
//
// Re-enabling is Phase 2c work tracked in playbook §10's Wave-β B12
// deferral pattern: rebuild against the kinded `(bits, kind)` stack ABI
// once the supervisor's compiler-side ValueWord migration lands.

#![cfg(any())]

#[test]
fn _phase_2c_rebuild() {
    todo!("phase-2c — see ADR-006 §2.7.4");
}
