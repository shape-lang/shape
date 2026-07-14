//! VM/JIT parity checks for `std::core::math` statistical intrinsics.

use shape_test::shape_test::ShapeTest;

const CORRELATION: &str = r#"
    from std::core::math use { correlation }
    correlation([10.0, 20.0, 30.0], [2.0, 4.0, 6.0])
"#;

const COVARIANCE: &str = r#"
    from std::core::math use { covariance }
    covariance([1.0, 2.0, 3.0], [2.0, 4.0, 6.0])
"#;

const PERCENTILE_FIFTY: &str = r#"
    from std::core::math use { percentile }
    percentile([10.0, 20.0, 30.0, 40.0, 50.0], 50.0)
"#;

const PERCENTILE_NINETY_FIVE: &str = r#"
    from std::core::math use { percentile }
    percentile([10.0, 20.0, 30.0, 40.0, 50.0], 95.0)
"#;

#[test]
fn correlation_is_one_in_vm_and_jit() {
    ShapeTest::new(CORRELATION).expect_number(1.0);
    ShapeTest::new(CORRELATION).with_jit().expect_number(1.0);
}

#[test]
fn covariance_uses_the_sample_denominator_in_vm_and_jit() {
    ShapeTest::new(COVARIANCE).expect_number(2.0);
    ShapeTest::new(COVARIANCE).with_jit().expect_number(2.0);
}

#[test]
fn percentile_uses_the_rounded_order_statistic_in_vm_and_jit() {
    ShapeTest::new(PERCENTILE_FIFTY).expect_number(30.0);
    ShapeTest::new(PERCENTILE_FIFTY)
        .with_jit()
        .expect_number(30.0);
}

#[test]
fn percentile_ninety_five_rounds_up_in_vm_and_jit() {
    ShapeTest::new(PERCENTILE_NINETY_FIVE).expect_number(50.0);
    ShapeTest::new(PERCENTILE_NINETY_FIVE)
        .with_jit()
        .expect_number(50.0);
}

#[test]
fn integer_form_literals_adopt_number_carriers_in_vm_and_jit() {
    let source = r#"
        from std::core::math use { correlation }
        let series_a: Array<number> = [1, 2, 3]
        let series_b: Array<number> = [2, 4, 6]
        correlation(series_a, series_b)
    "#;
    ShapeTest::new(source).expect_number(1.0);
    ShapeTest::new(source).with_jit().expect_number(1.0);
}

#[test]
fn unequal_series_lengths_fail_in_vm_and_jit() {
    let source = r#"
        from std::core::math use { covariance }
        covariance([1.0, 2.0], [1.0])
    "#;
    ShapeTest::new(source).expect_run_err_contains("Column lengths must match");
    ShapeTest::new(source)
        .with_jit()
        .expect_run_err_contains("Column lengths must match");
}

#[test]
fn percentile_rejects_out_of_range_values_in_vm_and_jit() {
    let source = r#"
        from std::core::math use { percentile }
        percentile([1.0, 2.0], 101.0)
    "#;
    ShapeTest::new(source).expect_run_err_contains("Percentile must be between 0 and 100");
    ShapeTest::new(source)
        .with_jit()
        .expect_run_err_contains("Percentile must be between 0 and 100");
}

#[test]
fn empty_series_nan_is_observable_in_vm_and_jit() {
    let source = r#"
        from std::core::math use { percentile }
        let empty: Array<number> = []
        isNaN(percentile(empty, 50.0))
    "#;
    ShapeTest::new(source).expect_bool(true);
    ShapeTest::new(source).with_jit().expect_bool(true);
}
