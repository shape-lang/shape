//! Tests for advanced stdlib modules:
//! - distributions_advanced.shape (SL7)
//! - property_testing.shape (SL8)
//! - encoding.shape (SL3)

use crate::common::{eval_to_bool, eval_to_string, init_runtime};
use std::path::Path;

fn read_stdlib_module(path: &str) -> String {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("shape-core/stdlib")
        .join(path);
    std::fs::read_to_string(&base)
        .unwrap_or_else(|e| panic!("Failed to read stdlib module {}: {}", base.display(), e))
}

fn strip_import_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("import ") && !trimmed.starts_with("from ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn with_modules(module_paths: &[&str], code: &str) -> String {
    let mut merged = String::new();
    for path in module_paths {
        merged.push_str(&strip_import_lines(&read_stdlib_module(path)));
        merged.push('\n');
    }
    merged.push_str(code);
    merged
}

// ===== SL7: Advanced Distributions =====

#[test]
fn test_normal_pdf_at_zero() {
    init_runtime();
    // Standard normal PDF at x=0 should be 1/sqrt(2*pi) ≈ 0.3989
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let p = normal_pdf(0.0);
        abs(p - 0.3989422804014327) < 0.0001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_normal_cdf_symmetry() {
    init_runtime();
    // CDF(0) = 0.5 for standard normal
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let c = normal_cdf(0.0);
        abs(c - 0.5) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_normal_cdf_at_two_sigma() {
    init_runtime();
    // CDF(2) ≈ 0.9772
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let c = normal_cdf(2.0);
        abs(c - 0.9772) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_normal_quantile_roundtrip() {
    init_runtime();
    // quantile(cdf(1.0)) should ≈ 1.0
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let p = normal_cdf(1.5);
        let x = normal_quantile(p);
        abs(x - 1.5) < 0.01
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_gamma_function_factorial() {
    init_runtime();
    // Gamma(5) = 4! = 24
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let g = gamma(5.0);
        abs(g - 24.0) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_gamma_function_half() {
    init_runtime();
    // Gamma(0.5) = sqrt(pi) ≈ 1.7725
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let g = gamma(0.5);
        abs(g - 1.7724538509055159) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_beta_function_value() {
    init_runtime();
    // B(2, 3) = Gamma(2)*Gamma(3)/Gamma(5) = 1*2/24 = 1/12 ≈ 0.0833
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let bval = beta_fn(2.0, 3.0);
        abs(bval - 0.08333333) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_chi_square_pdf_positive() {
    init_runtime();
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let p = chi_square_pdf(3.0, 4);
        p > 0.0 && p < 1.0
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_chi_square_cdf_bounds() {
    init_runtime();
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let c1 = chi_square_cdf(0.0, 4);
        let c2 = chi_square_cdf(10.0, 4);
        c1 == 0.0 && c2 > 0.9
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_t_distribution_pdf_symmetric() {
    init_runtime();
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let p1 = t_pdf(1.0, 5);
        let p2 = t_pdf(-1.0, 5);
        abs(p1 - p2) < 0.0001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_t_distribution_cdf_at_zero() {
    init_runtime();
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let c = t_cdf(0.0, 10);
        abs(c - 0.5) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_beta_pdf_bounds() {
    init_runtime();
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        let p = beta_pdf(0.5, 2.0, 5.0);
        p > 0.0 && beta_pdf(0.0, 2.0, 5.0) == 0.0 && beta_pdf(1.0, 2.0, 5.0) == 0.0
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_beta_cdf_bounds() {
    init_runtime();
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        beta_cdf(0.0, 2.0, 5.0) == 0.0 && beta_cdf(1.0, 2.0, 5.0) == 1.0
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_gamma_sample_positive() {
    init_runtime();
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        __intrinsic_random_seed(42);
        var all_positive = true;
        for i in range(0, 100) {
            let s = gamma_sample(2.0, 1.0);
            if s <= 0.0 {
                all_positive = false;
            }
        }
        all_positive
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_beta_sample_in_unit_interval() {
    init_runtime();
    let code = with_modules(
        &["core/distributions_advanced.shape"],
        r#"
        __intrinsic_random_seed(42);
        var all_ok = true;
        for i in range(0, 100) {
            let s = beta_sample(2.0, 5.0);
            if s < 0.0 || s > 1.0 {
                all_ok = false;
            }
        }
        all_ok
        "#,
    );
    assert!(eval_to_bool(&code));
}

// ===== SL8: Property-Based Testing =====

#[test]
fn test_property_passing() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        __intrinsic_random_seed(42);
        let result = property("addition commutes", 100,
            || __intrinsic_random_int(0, 1000),
            |x| {
                let y = __intrinsic_random_int(0, 1000);
                x + y == y + x
            }
        );
        result.passed && result.counterexample == None
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_property_failing() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        __intrinsic_random_seed(42);
        let result = property("always less than 50", 100,
            || __intrinsic_random_int(0, 100),
            |x| x < 50
        );
        !result.passed && result.counterexample != None
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_run_properties_summary() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        __intrinsic_random_seed(42);
        let results = run_properties([
            { name: "positive", trials: 50, gen: || __intrinsic_random_int(1, 100), prop: |x| x > 0 },
            { name: "negative", trials: 50, gen: || __intrinsic_random_int(1, 100), prop: |x| x < 0 }
        ]);
        results.passed == 1 && results.failed == 1 && results.total == 2
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_gen_int_range() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        __intrinsic_random_seed(42);
        let gen = gen_int(10, 20);
        var all_in_range = true;
        for i in range(0, 50) {
            let v = gen();
            if v < 10 || v > 20 {
                all_in_range = false;
            }
        }
        all_in_range
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_gen_float_range() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        __intrinsic_random_seed(42);
        let gen = gen_float(0.0, 1.0);
        var all_ok = true;
        for i in range(0, 50) {
            let v = gen();
            if v < 0.0 || v >= 1.0 {
                all_ok = false;
            }
        }
        all_ok
        "#,
    );
    assert!(eval_to_bool(&code));
}

// ===== SL3: Encoding =====

#[test]
fn test_url_encode_simple() {
    init_runtime();
    let code = with_modules(
        &["core/encoding.shape"],
        r#"
        url_encode("hello world")
        "#,
    );
    assert_eq!(eval_to_string(&code), "hello%20world");
}

#[test]
fn test_url_encode_unreserved() {
    init_runtime();
    let code = with_modules(
        &["core/encoding.shape"],
        r#"
        url_encode("abc-123_test.txt~")
        "#,
    );
    assert_eq!(eval_to_string(&code), "abc-123_test.txt~");
}

#[test]
fn test_url_encode_special_chars() {
    init_runtime();
    let code = with_modules(
        &["core/encoding.shape"],
        r#"
        url_encode("a=1&b=2")
        "#,
    );
    assert_eq!(eval_to_string(&code), "a%3D1%26b%3D2");
}

#[test]
fn test_url_decode_simple() {
    init_runtime();
    let code = with_modules(
        &["core/encoding.shape"],
        r#"
        url_decode("hello%20world")
        "#,
    );
    assert_eq!(eval_to_string(&code), "hello world");
}

#[test]
fn test_url_decode_plus() {
    init_runtime();
    let code = with_modules(
        &["core/encoding.shape"],
        r#"
        url_decode("hello+world")
        "#,
    );
    assert_eq!(eval_to_string(&code), "hello world");
}

#[test]
fn test_url_roundtrip() {
    init_runtime();
    let code = with_modules(
        &["core/encoding.shape"],
        r#"
        let original = "hello world&foo=bar";
        url_decode(url_encode(original)) == original
        "#,
    );
    assert!(eval_to_bool(&code));
}
