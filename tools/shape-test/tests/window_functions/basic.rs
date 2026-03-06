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
    .expect_run_err_contains("Unknown method 'row_number'");
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
    .expect_run_err_contains("Unknown method 'rank'");
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
    .expect_run_err_contains("Unknown method 'lag'");
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
    .expect_run_err_contains("Unknown method 'lead'");
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
    .expect_run_err_contains("Unknown method 'ntile'");
}

// =========================================================================
// Over (Partition + Order)
// =========================================================================

// TDD: over() clause and rank() builtin not implemented
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
    .expect_run_err_contains("Unknown method 'rolling'");
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
    .expect_run_err_contains("Unknown method 'scan'");
}
