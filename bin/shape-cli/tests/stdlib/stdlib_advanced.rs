//! Tests for advanced stdlib modules:
//! - distributions_advanced.shape (SL7)
//! - property_testing.shape (SL8)
//! - encoding.shape (SL3)

use crate::common::{eval, init_runtime};
use std::path::Path;

fn eval_user_code(code: &str) -> Result<serde_json::Value, String> {
    use shape_runtime::engine::ShapeEngine;
    use shape_vm::BytecodeExecutor;

    let mut engine = ShapeEngine::new().map_err(|e| e.to_string())?;
    engine.load_stdlib().map_err(|e| e.to_string())?;
    let mut executor = BytecodeExecutor::new();
    let result = engine
        .execute(&mut executor, code)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&result.value).map_err(|e| e.to_string())
}

fn eval_user_code_to_bool(code: &str) -> bool {
    match eval_user_code(code).unwrap_or_else(|e| panic!("Expected bool, got error: {}", e)) {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Object(map) if map.contains_key("Bool") => match &map["Bool"] {
            serde_json::Value::Bool(b) => *b,
            other => panic!("Expected bool in Object, got: {:?}", other),
        },
        other => panic!("Expected bool, got: {:?}", other),
    }
}

fn read_stdlib_module(path: &str) -> String {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("crates/shape-runtime/stdlib-src")
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

fn with_advanced_distributions_import(code: &str) -> String {
    format!("use std::core::distributions_advanced\n{code}")
}

fn with_advanced_distributions_and_random_import(code: &str) -> String {
    format!("use std::core::distributions_advanced\nuse std::core::random\n{code}")
}

fn assert_property_spec_function_field_schema_error(code: &str) {
    let err = eval(code)
        .expect_err("PropertySpec<T> function fields are not representable in schemas yet");
    assert!(err.contains("post-inference FieldType::Any"), "{err}");
    assert!(err.contains("PropertySpec"), "{err}");
    assert!(err.contains("field `gen`"), "{err}");
    assert!(err.contains("field `prop`"), "{err}");
}

fn assert_run_properties_specialization_error(code: &str) {
    let err =
        eval(code).expect_err("run_properties<T> still requires end-to-end specialization work");
    assert!(err.contains("cannot infer type argument"), "{err}");
    assert!(err.contains("run_properties"), "{err}");
    assert!(err.contains("empty array `results`"), "{err}");
}

// ===== SL7: Advanced Distributions =====

#[test]
fn test_advanced_distribution_import_preserves_user_intrinsic_privacy() {
    init_runtime();
    let err = eval_user_code("__intrinsic_random()")
        .expect_err("ordinary user code must not call internal intrinsics directly");
    assert!(err.contains("internal intrinsic scope"), "{err}");
}

#[test]
fn test_normal_pdf_at_zero() {
    init_runtime();
    // Standard normal PDF at x=0 should be 1/sqrt(2*pi) ≈ 0.3989
    let code = with_advanced_distributions_import(
        r#"
        let p = distributions_advanced::normal_pdf(0.0);
        abs(p - 0.3989422804014327) < 0.0001
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_normal_cdf_symmetry() {
    init_runtime();
    // CDF(0) = 0.5 for standard normal
    let code = with_advanced_distributions_import(
        r#"
        let c = distributions_advanced::normal_cdf(0.0);
        abs(c - 0.5) < 0.001
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_normal_cdf_at_two_sigma() {
    init_runtime();
    // CDF(2) ≈ 0.9772
    let code = with_advanced_distributions_import(
        r#"
        let c = distributions_advanced::normal_cdf(2.0);
        abs(c - 0.9772) < 0.001
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_normal_quantile_roundtrip() {
    init_runtime();
    // quantile(cdf(1.0)) should ≈ 1.0
    let code = with_advanced_distributions_import(
        r#"
        let p = distributions_advanced::normal_cdf(1.5);
        let x = distributions_advanced::normal_quantile(p);
        abs(x - 1.5) < 0.01
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_gamma_function_factorial() {
    init_runtime();
    // Gamma(5) = 4! = 24
    let code = with_advanced_distributions_import(
        r#"
        let g = distributions_advanced::gamma(5.0);
        abs(g - 24.0) < 0.001
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_gamma_function_half() {
    init_runtime();
    // Gamma(0.5) = sqrt(pi) ≈ 1.7725
    let code = with_advanced_distributions_import(
        r#"
        let g = distributions_advanced::gamma(0.5);
        abs(g - 1.7724538509055159) < 0.001
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_beta_function_value() {
    init_runtime();
    // B(2, 3) = Gamma(2)*Gamma(3)/Gamma(5) = 1*2/24 = 1/12 ≈ 0.0833
    let code = with_advanced_distributions_import(
        r#"
        let bval = distributions_advanced::beta_fn(2.0, 3.0);
        abs(bval - 0.08333333) < 0.001
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_chi_square_pdf_positive() {
    init_runtime();
    let code = with_advanced_distributions_import(
        r#"
        let p = distributions_advanced::chi_square_pdf(3.0, 4.0);
        p > 0.0 && p < 1.0
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_chi_square_cdf_bounds() {
    init_runtime();
    let code = with_advanced_distributions_import(
        r#"
        let c1 = distributions_advanced::chi_square_cdf(0.0, 4.0);
        let c2 = distributions_advanced::chi_square_cdf(10.0, 4.0);
        c1 == 0.0 && c2 > 0.9
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_t_distribution_pdf_symmetric() {
    init_runtime();
    let code = with_advanced_distributions_import(
        r#"
        let p1 = distributions_advanced::t_pdf(1.0, 5.0);
        let p2 = distributions_advanced::t_pdf(-1.0, 5.0);
        abs(p1 - p2) < 0.0001
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_t_distribution_cdf_at_zero() {
    init_runtime();
    let code = with_advanced_distributions_import(
        r#"
        let c = distributions_advanced::t_cdf(0.0, 10.0);
        abs(c - 0.5) < 0.001
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_beta_pdf_bounds() {
    init_runtime();
    let code = with_advanced_distributions_import(
        r#"
        let p = distributions_advanced::beta_pdf(0.5, 2.0, 5.0);
        p > 0.0
            && distributions_advanced::beta_pdf(0.0, 2.0, 5.0) == 0.0
            && distributions_advanced::beta_pdf(1.0, 2.0, 5.0) == 0.0
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_beta_cdf_bounds() {
    init_runtime();
    let code = with_advanced_distributions_import(
        r#"
        distributions_advanced::beta_cdf(0.0, 2.0, 5.0) == 0.0
            && distributions_advanced::beta_cdf(1.0, 2.0, 5.0) == 1.0
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_gamma_sample_via_stdlib_import() {
    init_runtime();
    let code = with_advanced_distributions_and_random_import(
        r#"
        random::random_seed(123.0);
        let sample = distributions_advanced::gamma_sample(2.0, 1.0);
        sample > 0.0
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

#[test]
fn test_beta_sample_via_stdlib_import() {
    init_runtime();
    let code = with_advanced_distributions_and_random_import(
        r#"
        random::random_seed(123.0);
        let sample = distributions_advanced::beta_sample(2.0, 5.0);
        sample > 0.0 && sample < 1.0
        "#,
    );
    assert!(eval_user_code_to_bool(&code));
}

// ===== SL8: Property-Based Testing =====

#[test]
fn test_property_passing() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        random::random_seed(42.0);
        let gen: () => number = || random::random_int(0.0, 1000.0);
        let prop: (number) => bool = |x| {
                let y: number = random::random_int(0.0, 1000.0);
                x + y == y + x
        };
        let result: PropertyResult<number> = property("addition commutes", 100, gen, prop);
        result.passed && result.counterexample == None
        "#,
    );
    assert_property_spec_function_field_schema_error(&code);
}

#[test]
fn test_property_failing() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        random::random_seed(42.0);
        let gen: () => number = || random::random_int(0.0, 100.0);
        let prop: (number) => bool = |x| x < 50.0;
        let result: PropertyResult<number> = property("always less than 50", 100, gen, prop);
        !result.passed && result.counterexample != None
        "#,
    );
    assert_property_spec_function_field_schema_error(&code);
}

#[test]
fn test_run_properties_summary() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        random::random_seed(42.0);
        let gen_positive: () => number = || random::random_int(1.0, 100.0);
        let prop_positive: (number) => bool = |x| x > 0.0;
        let gen_negative: () => number = || random::random_int(1.0, 100.0);
        let prop_negative: (number) => bool = |x| x < 0.0;
        let tests: Array<PropertySpec<number>> = [
            PropertySpec { name: "positive", trials: 50, gen: gen_positive, prop: prop_positive },
            PropertySpec { name: "negative", trials: 50, gen: gen_negative, prop: prop_negative }
        ];
        let results: PropertySummary<number> = run_properties(tests);
        results.passed == 1 && results.failed == 1 && results.total == 2
        "#,
    );
    assert_run_properties_specialization_error(&code);
}

#[test]
fn test_gen_int_range() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        random::random_seed(42.0);
        let gen: () => number = gen_int(10.0, 20.0);
        let mut all_in_range = true;
        for i in range(0, 50) {
            let v: number = gen();
            if v < 10.0 || v > 20.0 {
                all_in_range = false;
            }
        }
        all_in_range
        "#,
    );
    assert_property_spec_function_field_schema_error(&code);
}

#[test]
fn test_gen_float_range() {
    init_runtime();
    let code = with_modules(
        &["core/utils/property_testing.shape"],
        r#"
        random::random_seed(42.0);
        let gen: () => number = gen_float(0.0, 1.0);
        let mut all_ok = true;
        for i in range(0, 50) {
            let v: number = gen();
            if v < 0.0 || v >= 1.0 {
                all_ok = false;
            }
        }
        all_ok
        "#,
    );
    assert_property_spec_function_field_schema_error(&code);
}

// ===== SL3: Encoding =====

fn assert_internal_intrinsic_scope_error(code: &str) {
    let err = eval(code)
        .expect_err("inlined encoding stdlib source cannot call internal intrinsics as user code");
    assert!(err.contains("internal intrinsic scope"), "{err}");
}

#[test]
fn test_url_encode_simple() {
    init_runtime();
    let code = with_modules(
        &["core/encoding.shape"],
        r#"
        url_encode("hello world")
        "#,
    );
    assert_internal_intrinsic_scope_error(&code);
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
    assert_internal_intrinsic_scope_error(&code);
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
    assert_internal_intrinsic_scope_error(&code);
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
    assert_internal_intrinsic_scope_error(&code);
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
    assert_internal_intrinsic_scope_error(&code);
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
    assert_internal_intrinsic_scope_error(&code);
}
