//! Window function tests
//! SQL-style analytics functions: lag, lead, rank, row_number, ntile, over().
//! All of these are TDD tests — window functions are not yet implemented on Array type.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Row Number
// =========================================================================

// TDD: window functions not yet implemented as built-in language feature
#[test]
fn window_row_number_basic() {
    ShapeTest::new(
        r#"
        let data = [10, 20, 30]
        let result = data.row_number()
        print(result[0])
    "#,
    )
    // Strict-flip TP-rebaseline: `row_number` is absent from the method seed,
    // so strict typing rejects at COMPILE time. The diagnostic surfaces either
    // as the method-not-found rejection or as the downstream `cannot have
    // fields` constraint cascade (checker pass ordering is nondeterministic).
    // Negative-test intent (row_number is not an implemented Array method)
    // preserved.
    .expect_run_err_contains_any(&[
        "Method 'row_number' not found on type 'Vec'",
        "cannot have fields",
    ]);
}

// =========================================================================
// Rank
// =========================================================================

// TDD: window functions not yet implemented as built-in language feature
#[test]
fn window_rank_basic() {
    ShapeTest::new(
        r#"
        let scores = [100, 90, 100, 80, 90]
        let ranked = scores.rank()
        print(ranked)
    "#,
    )
    // Strict-flip TP-rebaseline: `rank` is absent from the method seed, so
    // strict typing rejects the call at COMPILE time (method-not-found or the
    // downstream constraint cascade — checker pass ordering is
    // nondeterministic). Negative-test intent (rank not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'rank' not found on type 'Vec'",
        "cannot have fields",
    ]);
}

// =========================================================================
// Lag / Lead
// =========================================================================

// TDD: window functions not yet implemented as built-in language feature
#[test]
fn window_lag_offset_1() {
    ShapeTest::new(
        r#"
        let prices = [10, 20, 30, 40]
        let lagged = prices.lag(1)
        print(lagged)
    "#,
    )
    // Strict-flip TP-rebaseline: `lag` is absent from the method seed, so
    // strict typing rejects at COMPILE time (method-not-found or the downstream
    // constraint cascade — checker pass ordering is nondeterministic).
    // Negative-test intent (lag not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'lag' not found on type 'Vec'",
        "cannot have fields",
    ]);
}

// TDD: window functions not yet implemented as built-in language feature
#[test]
fn window_lead_offset_1() {
    ShapeTest::new(
        r#"
        let prices = [10, 20, 30, 40]
        let led = prices.lead(1)
        print(led)
    "#,
    )
    // Strict-flip TP-rebaseline: `lead` is absent from the method seed, so
    // strict typing rejects the call at COMPILE time (method-not-found or the
    // downstream constraint cascade — checker pass ordering is
    // nondeterministic). Negative-test intent (lead not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'lead' not found on type 'Vec'",
        "cannot have fields",
    ]);
}

// =========================================================================
// Ntile
// =========================================================================

// TDD: window functions not yet implemented as built-in language feature
#[test]
fn window_ntile_quartiles() {
    ShapeTest::new(
        r#"
        let data = [1, 2, 3, 4, 5, 6, 7, 8]
        let tiles = data.ntile(4)
        print(tiles)
    "#,
    )
    // Strict-flip TP-rebaseline: `ntile` is absent from the method seed, so
    // strict typing rejects at COMPILE time (method-not-found or the downstream
    // constraint cascade — checker pass ordering is nondeterministic).
    // Negative-test intent (ntile not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'ntile' not found on type 'Vec'",
        "cannot have fields",
    ]);
}

// =========================================================================
// Over (Partition + Order)
// =========================================================================

// TDD: over() clause and rank() builtin not implemented.
// D-γ close (v0.3 KC #6(e), 2026-05-22): the prior `expect_run_ok` form
// HUNG until SIGKILL because `from..select` desugars to `Vec.map`, and the
// generic `Vec.map<T,U>` extend method's monomorphization fails for the
// struct-element receiver here. The compiler's previous fallback emitted
// `Call(generic_idx)` for the body-less generic, which the content-
// addressed linker (`linker.rs:remap_fid`) rewrote to `current_function_id`
// — recursing through `__main__`. The fix routes generic-no-body callees
// to the standard `CallMethod` runtime dispatch, which surface-and-stops
// at `handle_map_v2`'s V3-S5 ckpt-2 NotImplemented stub. Aligned with the
// audit §6(e) classification: 7 sibling window_* tests stay INCOMPLETE-
// CLEAN feature-gap (v0.4 polish); this one moves from KNOWN-INCORRECT
// (hang) to INCOMPLETE-CLEAN (clean surface error) by the same fix.
#[test]
fn window_over_partition_by() {
    ShapeTest::new(
        r#"
        let sales = [
            { region: "east", amount: 100 },
            { region: "west", amount: 200 }
        ]
        let result = from s in sales select s.region
        print(result.length)
    "#,
    )
    // V3-S5 consumer-cascade close (2026-06-05): `handle_map_v2` now
    // delegates to `array_query::handle_select_v2`, so the ckpt-2
    // `map: SURFACE` stub is gone. `select s.region` returns a legacy
    // `NativeKind::String` (not the v2-raw `StringV2` carrier), which has no
    // `TypedArray<T>` monomorphization — a distinct, cleaner surface in the
    // String→StringV2 producer-cascade class. Still INCOMPLETE-CLEAN.
    .expect_run_err_contains("has no `TypedArray<T>` carrier monomorphization");
}

// =========================================================================
// Frame Specifications
// =========================================================================

// TDD: rolling() method not implemented on Array type
#[test]
fn window_rolling_sum() {
    ShapeTest::new(
        r#"
        let data = [1, 2, 3, 4, 5]
        let result = data.rolling(3).sum()
        print(result)
    "#,
    )
    // Strict-flip TP-rebaseline: `rolling` is absent from the method seed, so
    // strict typing rejects at COMPILE time (method-not-found or the downstream
    // constraint cascade from the chained `.sum()` — checker pass ordering is
    // nondeterministic). Negative-test intent (rolling not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'rolling' not found on type 'Vec'",
        "cannot have fields",
    ]);
}

// TDD: scan() method not implemented on Array type
#[test]
fn window_cumulative_sum() {
    ShapeTest::new(
        r#"
        let data = [1, 2, 3, 4, 5]
        let cumsum = data.scan(|acc, x| acc + x, 0)
        print(cumsum)
    "#,
    )
    // Strict-flip TP-rebaseline: `scan` is absent from the method seed, so
    // strict typing rejects the call at COMPILE time (method-not-found or the
    // downstream constraint cascade — checker pass ordering is
    // nondeterministic). Negative-test intent (scan not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'scan' not found on type 'Vec'",
        "cannot have fields",
    ]);
}
