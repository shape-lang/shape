//! VM/Interpreter Parity Tests
//!
//! These tests verify that the Shape engine produces consistent results
//! for various expressions and operations.

use crate::common::{eval, init_runtime};

fn extract_number(val: &serde_json::Value) -> Option<f64> {
    match val {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::Object(map) if map.contains_key("Integer") => map["Integer"].as_f64(),
        serde_json::Value::Object(map) if map.contains_key("Number") => map["Number"].as_f64(),
        _ => None,
    }
}

fn check_number(name: &str, code: &str, expected: f64) -> bool {
    match eval(code) {
        Ok(ref val) => {
            if let Some(actual) = extract_number(val) {
                let equal = (actual - expected).abs() < 1e-10;
                if !equal {
                    eprintln!("{}: expected {}, got {}", name, expected, actual);
                }
                equal
            } else {
                eprintln!("{}: expected number {}, got {:?}", name, expected, val);
                false
            }
        }
        Err(e) => {
            eprintln!("{}: error - {}", name, e);
            false
        }
    }
}

fn extract_bool(val: &serde_json::Value) -> Option<bool> {
    match val {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Object(map) if map.contains_key("Bool") => map["Bool"].as_bool(),
        _ => None,
    }
}

fn check_bool(name: &str, code: &str, expected: bool) -> bool {
    match eval(code) {
        Ok(ref val) => {
            if let Some(actual) = extract_bool(val) {
                if actual != expected {
                    eprintln!("{}: expected {}, got {}", name, expected, actual);
                }
                actual == expected
            } else {
                eprintln!("{}: expected bool {}, got {:?}", name, expected, val);
                false
            }
        }
        Err(e) => {
            eprintln!("{}: error - {}", name, e);
            false
        }
    }
}

#[test]
fn test_arithmetic_consistency() {
    init_runtime();

    assert!(check_number("add", "1 + 2", 3.0));
    assert!(check_number("subtract", "10 - 3", 7.0));
    assert!(check_number("multiply", "4 * 5", 20.0));
    assert!(check_number("divide", "20 / 4", 5.0));
    assert!(check_number("modulo", "17 % 5", 2.0));
    assert!(check_number("power", "2 ** 8", 256.0)); // Power operator is **
    assert!(check_number("negative", "-42", -42.0));
    assert!(check_number("complex_expr", "(1 + 2) * 3 - 4 / 2", 7.0));
}

#[test]
fn test_comparison_consistency() {
    init_runtime();

    assert!(check_bool("equal_true", "5 == 5", true));
    assert!(check_bool("equal_false", "5 == 6", false));
    assert!(check_bool("not_equal", "5 != 6", true));
    assert!(check_bool("less_than", "3 < 5", true));
    assert!(check_bool("greater_than", "5 > 3", true));
    assert!(check_bool("less_equal", "5 <= 5", true));
    assert!(check_bool("greater_equal", "5 >= 5", true));
}

#[test]
fn test_logical_consistency() {
    init_runtime();

    assert!(check_bool("and_true", "true && true", true));
    assert!(check_bool("and_false", "true && false", false));
    assert!(check_bool("or_true", "false || true", true));
    assert!(check_bool("or_false", "false || false", false));
    assert!(check_bool("not_true", "!false", true));
    assert!(check_bool("not_false", "!true", false));
}

#[test]
fn test_variable_consistency() {
    init_runtime();

    assert!(check_number("simple_var", "let x = 42; x", 42.0));
    assert!(check_number(
        "var_arithmetic",
        "let x = 10; let y = 20; x + y",
        30.0
    ));
    assert!(check_number("var_reassign", "let mut x = 5; x = x + 1; x", 6.0));
}

#[test]
fn test_conditional_consistency() {
    init_runtime();

    assert!(check_number("if_true", "if true { 1 } else { 2 }", 1.0));
    assert!(check_number("if_false", "if false { 1 } else { 2 }", 2.0));
    assert!(check_number("ternary_true", "true ? 10 : 20", 10.0));
    assert!(check_number("ternary_false", "false ? 10 : 20", 20.0));
}

#[test]
fn test_array_consistency() {
    init_runtime();

    assert!(check_number(
        "array_index_0",
        "let arr = [10, 20, 30]; arr[0]",
        10.0
    ));
    assert!(check_number(
        "array_index_1",
        "let arr = [10, 20, 30]; arr[1]",
        20.0
    ));
    assert!(check_number("array_length", "len([1, 2, 3, 4, 5])", 5.0));
}

#[test]
fn test_function_consistency() {
    init_runtime();

    assert!(check_number("abs_positive", "abs(5)", 5.0));
    assert!(check_number("abs_negative", "abs(-5)", 5.0));
    assert!(check_number("min", "min(3, 7)", 3.0));
    assert!(check_number("max", "max(3, 7)", 7.0));
    assert!(check_number("floor", "floor(3.7)", 3.0));
    assert!(check_number("ceil", "ceil(3.2)", 4.0));
    assert!(check_number("round", "round(3.5)", 4.0));
    assert!(check_number("sqrt", "sqrt(16)", 4.0));
}

#[test]
fn test_loop_consistency() {
    init_runtime();

    assert!(check_number(
        "for_sum",
        "let mut sum = 0; for i in range(5) { sum = sum + i }; sum",
        10.0
    ));
    assert!(check_number(
        "while_count",
        "let mut i = 0; while i < 5 { i = i + 1 }; i",
        5.0
    ));
}

#[test]
fn test_user_function_consistency() {
    init_runtime();

    assert!(check_number(
        "simple_func",
        "function double(x) { return x * 2 }; double(21)",
        42.0
    ));
    assert!(check_number(
        "recursive_fib",
        "function fib(n) { if n <= 1 { return n } else { return fib(n-1) + fib(n-2) } }; fib(10)",
        55.0
    ));
}
