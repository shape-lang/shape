// ADR-006 §2.7.4 / §2.7.7 — Phase 2c deferral.
//
// The original `compiler_tests.rs` deep-test harness ran end-to-end VM
// execution through `compile_and_run() -> ValueWord` and asserted via
// `ValueWordExt::{as_i64, as_str, as_bool, as_number_coerce}`. It also
// constructed `ValueWord::none()` / `ValueWord::from_i64` / `ValueWord::
// from_f64` literals through the deleted carrier. Every line referenced
// the §2.7.7 forbidden ValueWord shape; per playbook §7 REVISED #2 / #4
// the correct surface is to replace the file body with a `cfg(any())`-
// gated `todo!()` placeholder rather than re-introduce the carrier.
//
// Re-enabling these tests is Phase 2c work tracked in playbook §10's
// Wave-β B12 deferral pattern: rebuild against the kinded `(bits, kind)`
// stack ABI once the supervisor's compiler-side ValueWord migration
// (statements.rs, comptime.rs, specialization.rs) lands. The include
// site at `compiler/mod.rs` is itself gated `#[cfg(any())]`, so this
// file is effectively dormant until that supervisor sweep completes.

#![cfg(any())]

#[test]
fn _phase_2c_rebuild() {
    todo!("phase-2c — see ADR-006 §2.7.4");
}
