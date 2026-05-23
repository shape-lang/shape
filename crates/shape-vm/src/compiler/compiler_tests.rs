// ADR-006 §2.7.4 / §2.7.7 — Phase 2c deferral (R8 C3 status update,
// 2026-05-23).
//
// The original `compiler_tests.rs` is a 4486-line deep-test harness that
// ran end-to-end VM execution through `compile_and_run() -> ValueWord`
// and asserted via `ValueWordExt::{as_i64, as_str, as_bool,
// as_number_coerce, as_native_scalar, as_err_inner, ...}`. It also
// constructed `ValueWord::none()` / `ValueWord::from_i64` /
// `ValueWord::from_f64` literals through the deleted carrier and
// exercises the `__native_*` builtin family (`__native_ptr_*`,
// `__native_table_from_arrow_c_typed`, `__native_arrow_view_*`) plus
// `register_test_function` — the V3-S5 host-tier eval/marshal API +
// T1-host-tier-marshal-rebuild territory (see
// `docs/cluster-audits/v0.3-phase-2c-audit.md` §2 Host-tier rows).
//
// Per playbook §7 REVISED #2 / #4 the correct surface for a non-
// migratable test site is `cfg(any())`-gating rather than reintroducing
// the §2.7.7 forbidden carrier.
//
// R8 C3 (this commit, 2026-05-23) rebuilt `compiler/monomorphization/
// integration_tests.rs` (its C3 sibling site) — the rebuild used the
// post-`ValueWord` `KindedSlot` API (`§2.7.6 / Q8`) and reduced array
// assertions to scalar `eval_with_prelude(...).as_i64()` checks. That
// pattern works for cache-keys + scalar-result tests, but
// `compiler_tests.rs`'s 4486-line content is dominated by `__native_*`
// builtin exercises that have no kinded `(bits, kind)` API today (T1's
// host-tier marshal landing is the gate). Migrating each test would
// require T1's `eval_*` marshal extensions + `as_native_scalar`-equivalent
// accessors on `KindedSlot` (Phase 4 territory). So this file stays
// gated until T1-host-tier-marshal-rebuild lands; the include site at
// `compiler/mod.rs` is itself gated `#[cfg(any())]`.
//
// Audit reference: `docs/cluster-audits/v0.3-phase-2c-audit.md`
// §1.D / §2 Host-tier Eval/Marshal API (V3-S5), §2 Compile-tier Rebuild
// (C1/C2/C3). C3's expr-lowering-misc territory closes via the
// `monomorphization/integration_tests.rs` rebuild; this fixture site
// re-anchors on T1 + V3-S5.

#![cfg(any())]

#[test]
fn _phase_2c_rebuild() {
    todo!(
        "phase-2c — see ADR-006 §2.7.4. Gated on T1-host-tier-marshal-rebuild + V3-S5: \
the `__native_*` builtin tests need a kinded host-tier marshal API that R8 C3 \
does not deliver."
    );
}
