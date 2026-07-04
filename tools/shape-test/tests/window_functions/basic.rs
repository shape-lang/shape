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
    // so strict typing rejects at COMPILE time. The collection type name in
    // the diagnostic has moved from `Vec` to `Array` on newer checker paths;
    // both spellings preserve the same missing-method contract.
    // Negative-test intent (row_number is not an implemented Array method)
    // preserved.
    .expect_run_err_contains_any(&[
        "Method 'row_number' not found on type 'Vec'",
        "Method 'row_number' not found on type 'Array'",
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
    // strict typing rejects the call at COMPILE time. The checker may surface
    // either the legacy `Vec` spelling, the current `Array` spelling, or the
    // downstream constraint cascade. Negative-test intent (rank not
    // implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'rank' not found on type 'Vec'",
        "Method 'rank' not found on type 'Array'",
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
    // strict typing rejects at COMPILE time. Accept the legacy `Vec` wording,
    // the current `Array` wording, or the downstream constraint cascade.
    // Negative-test intent (lag not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'lag' not found on type 'Vec'",
        "Method 'lag' not found on type 'Array'",
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
    // strict typing rejects the call at COMPILE time. Accept the legacy `Vec`
    // wording, the current `Array` wording, or the downstream constraint
    // cascade. Negative-test intent (lead not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'lead' not found on type 'Vec'",
        "Method 'lead' not found on type 'Array'",
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
    // strict typing rejects at COMPILE time. Accept the legacy `Vec` wording,
    // the current `Array` wording, or the downstream constraint cascade.
    // Negative-test intent (ntile not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'ntile' not found on type 'Vec'",
        "Method 'ntile' not found on type 'Array'",
        "cannot have fields",
    ]);
}

// =========================================================================
// Over (Partition + Order)
// =========================================================================

// This fixture no longer exercises SQL `over(...)` syntax directly. It is the
// historical D-γ hang repro: `from..select` over object rows used to recurse
// through generic-no-body lowering, then later surfaced at missing
// `TypedArray<T>` string-carrier monomorphization. Query `select` over object
// rows is now a supported v0.3 behavior (see `query_language::query_over_objects`),
// so keep this as a positive regression check against the old hang/stub class.
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
    .expect_run_ok()
    .expect_output("2");
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
    // strict typing rejects at COMPILE time. Accept the legacy `Vec` wording,
    // the current `Array` wording, or the downstream constraint cascade from
    // the chained `.sum()`. Negative-test intent (rolling not implemented)
    // preserved.
    .expect_run_err_contains_any(&[
        "Method 'rolling' not found on type 'Vec'",
        "Method 'rolling' not found on type 'Array'",
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
    // strict typing rejects the call at COMPILE time. Accept the legacy `Vec`
    // wording, the current `Array` wording, or the downstream constraint
    // cascade. Negative-test intent (scan not implemented) preserved.
    .expect_run_err_contains_any(&[
        "Method 'scan' not found on type 'Vec'",
        "Method 'scan' not found on type 'Array'",
        "cannot have fields",
    ]);
}
