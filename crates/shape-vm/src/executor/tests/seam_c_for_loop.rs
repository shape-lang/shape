//! Wave 1b SEAM C (2026-06-15): for-x-in-iter loop drive — end-to-end.
//!
//! `for x in arr.iter()` / `for x in iter.filter(..)` / `for c in str.iter()`
//! exercises the bytecode `IterDone` / `IterNext` loop protocol
//! (`compiler/loops.rs:427`) over a lazy `Arc<IteratorState>` carrier
//! (ADR-006 §2.7.16 / Q17). The iterator pipeline (source + map/filter/take/
//! skip/enumerate/chain transforms) is driven ONCE through the SEAM B
//! `materialize_yields` terminal driver and memoized on the `IteratorState`
//! (`iterator_methods::drive_for_loop_yields`); subsequent positional
//! `IterDone`/`IterNext` reads index the memo in O(1) and side-effecting
//! `map`/`filter` closures fire exactly once per element.
//!
//! This module is NON-gated (runs under `just test-fast`); the legacy
//! `iterator_ops` module is `deep-tests`-gated and references deleted
//! `ValueWord` / `heap_value::IteratorState` shapes (pre-existing stale
//! build, unrelated to SEAM C).

use crate::executor::tests::test_utils::eval_with_prelude;

#[test]
fn for_x_in_array_iter_sums() {
    // Element-type propagation (`iter_element_type_name` `.iter()` arm) types
    // `x` so `total + x` emits AddInt rather than falling into trait dispatch.
    let v = eval_with_prelude(
        "let arr = [1, 2, 3]; let mut total = 0; \
         for x in arr.iter() { total = total + x }; total",
    );
    assert_eq!(v.as_i64(), Some(6));
}

#[test]
fn for_x_in_array_iter_filter_pipeline() {
    // Transform pipeline: the predicate closure is invoked once per element
    // via the memoized drive (no double-fire across IterDone + IterNext, no
    // O(n²) re-drive across the loop).
    let v = eval_with_prelude(
        "let arr = [1, 2, 3, 4, 5, 6]; let mut total = 0; \
         for x in arr.iter().filter(|n| n > 3) { total = total + x }; total",
    );
    assert_eq!(v.as_i64(), Some(15)); // 4 + 5 + 6
}

#[test]
fn for_x_in_array_iter_map_pipeline() {
    // `map` changes the element type to the closure's return type, which
    // isn't statically recovered, so the loop var stays untyped — count the
    // mapped yields (drive correctness) rather than typed arithmetic on `y`.
    let v = eval_with_prelude(
        "let arr = [10, 20, 30]; let mut count = 0; \
         for y in arr.iter().map(|n| n * 2) { count = count + 1 }; count",
    );
    assert_eq!(v.as_i64(), Some(3));
}

#[test]
fn for_x_in_array_iter_break_early_exits() {
    // Break before exhaustion: the memo retains its owned heap-element shares
    // and retires them with the iterator Arc — no leak / no double-free.
    let v = eval_with_prelude(
        "let arr = [10, 20, 30, 40]; let mut total = 0; \
         for x in arr.iter() { if x == 30 { break }; total = total + x }; total",
    );
    assert_eq!(v.as_i64(), Some(30)); // 10 + 20
}

#[test]
fn for_c_in_string_iter_counts_codepoints() {
    let v = eval_with_prelude("let mut n = 0; for c in \"abcλ\".iter() { n = n + 1 }; n");
    assert_eq!(v.as_i64(), Some(4)); // codepoints, not bytes
}
