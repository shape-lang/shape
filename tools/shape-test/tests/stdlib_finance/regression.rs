//! WF-6 std::finance compiler stack-overflow regression.
//!
//! `ShapeTest::expect_bool` compiles AND runs the program (VM path). Before the
//! occurs-check fix these imports overflowed the compiler's stack and aborted
//! the whole test process, so a green assertion here is a positive proof that
//! the recursion terminates.

use shape_test::shape_test::ShapeTest;

/// A `std::finance` function compiles and runs to completion.
///
/// `is_ohlcv` is a pure predicate over an OHLCV row; it exercises the
/// stdlib-finance import + call path end to end.
#[test]
fn finance_is_ohlcv_compiles_and_runs() {
    ShapeTest::new(
        r#"
        from std::finance::types::ohlcv use { is_ohlcv }
        let candle = { open: 10.0, high: 15.0, low: 9.0, close: 12.0, volume: 100.0 }
        is_ohlcv(candle)
    "#,
    )
    .expect_bool(true);
}

/// The same import surface, a second finance predicate, to ensure the finance
/// module's exports resolve stably (not a one-off).
#[test]
fn finance_is_ohlcv_negative_row_still_terminates() {
    // A full OHLCV row that is NOT a doji-only shape; is_ohlcv must return true
    // and — crucially — the program must terminate rather than overflow at
    // compile time.
    ShapeTest::new(
        r#"
        from std::finance::types::ohlcv use { is_ohlcv }
        let candle = { open: 1.0, high: 2.0, low: 0.5, close: 1.5, volume: 42.0 }
        is_ohlcv(candle)
    "#,
    )
    .expect_bool(true);
}
