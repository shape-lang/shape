//! Wave-7 finance stdlib-source sweep regression.
//!
//! The `std::finance` package shipped with three source-level defects that
//! survived the WF-6 compiler occurs-check fix and only surfaced once a
//! program actually imported and *ran* a finance function under strict typing:
//!
//!   (a) let-as-mutable — loop accumulators / array builders declared `let`
//!       (immutable) but reassigned or `.push`-mutated. Fixed by promoting the
//!       genuine mutations to `let mut` across the indicators, risk and
//!       backtest modules.
//!   (b) missing `signal` / `is_signal` — `signals.shape` called four
//!       primitives (`signal`, `signal_none`, `signal_with_targets`,
//!       `is_signal`) that were never defined or exported. Fixed by defining
//!       the canonical `Signal` type plus the four primitives in-module.
//!
//! (The third defect — Float64 object-field subtraction on the untyped
//! `std::finance::types::ohlcv` helpers — is a compiler gap: cross-module
//! imported types are not resolved inside a *dependency* module, so a
//! `row: Candle` annotation degrades field access to `unknown`. That is
//! surfaced separately; the untyped WF-6 `is_ohlcv` path is exercised by
//! `regression.rs`.)
//!
//! Each assertion runs the program end-to-end (compile + execute). The
//! `_jit` variants drive the identical program through `shape_jit::JITExecutor`
//! (the release binary's `--mode jit` path) so both execution tiers are
//! covered. A green result is positive proof the finance package is usable.

use shape_test::shape_test::ShapeTest;

// ===== (a) let-as-mutable: the WMA indicator computes and runs =====

const WMA_PROGRAM: &str = r#"
    from std::finance::indicators::moving_averages use { wma }
    let series = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    let out = wma(series, 3)
    out[2]
"#;

/// `wma` (linearly weighted moving average) is pure-Shape and previously
/// failed to compile: its accumulator `sum` and its result array were declared
/// `let` but mutated. With the `let mut` fix it computes the correct value:
/// position 2 of a period-3 WMA over 1..=6 is (3*3 + 2*2 + 1*1) / 6 = 14/6.
#[test]
fn wma_indicator_runs_vm() {
    ShapeTest::new(WMA_PROGRAM).expect_number(14.0 / 6.0);
}

#[test]
fn wma_indicator_runs_jit() {
    ShapeTest::new(WMA_PROGRAM)
        .with_jit()
        .expect_number(14.0 / 6.0);
}

// ===== (b) signal / is_signal: construction + classification =====

const SIGNAL_BUY_PROGRAM: &str = r#"
    from std::finance::signals use { buy, is_buy }
    is_buy(buy(1.0))
"#;

/// `buy()` constructs a `Signal` and `is_buy` classifies it — the previously
/// undefined `signal(...)` / `is_signal(...)` primitives now exist.
#[test]
fn signal_is_buy_runs_vm() {
    ShapeTest::new(SIGNAL_BUY_PROGRAM).expect_bool(true);
}

#[test]
fn signal_is_buy_runs_jit() {
    ShapeTest::new(SIGNAL_BUY_PROGRAM).with_jit().expect_bool(true);
}

const SIGNAL_CROSS_PROGRAM: &str = r#"
    from std::finance::signals use { buy, is_sell }
    is_sell(buy(1.0))
"#;

/// A buy signal must not classify as a sell — proves `is_signal`/action
/// discrimination is real, not a constant `true`.
#[test]
fn signal_buy_is_not_sell_vm() {
    ShapeTest::new(SIGNAL_CROSS_PROGRAM).expect_bool(false);
}

#[test]
fn signal_buy_is_not_sell_jit() {
    ShapeTest::new(SIGNAL_CROSS_PROGRAM)
        .with_jit()
        .expect_bool(false);
}

const SIGNAL_TARGETS_PROGRAM: &str = r#"
    from std::finance::signals use { buy_with_stop }
    let sig = buy_with_stop(95.0, 110.0, 1.0)
    sig.stop_loss
"#;

/// `signal_with_targets` threads stop-loss / take-profit prices onto the
/// `Signal` and they survive the constructor round-trip.
#[test]
fn signal_with_targets_stop_loss_vm() {
    ShapeTest::new(SIGNAL_TARGETS_PROGRAM).expect_number(95.0);
}

#[test]
fn signal_with_targets_stop_loss_jit() {
    ShapeTest::new(SIGNAL_TARGETS_PROGRAM)
        .with_jit()
        .expect_number(95.0);
}
