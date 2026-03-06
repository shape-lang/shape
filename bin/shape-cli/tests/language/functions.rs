use crate::common::{eval_to_number, init_runtime};

#[test]
fn test_builtin_functions() {
    init_runtime();

    assert_eq!(eval_to_number("abs(-5)"), 5.0);
    assert_eq!(eval_to_number("abs(5)"), 5.0);
    assert_eq!(eval_to_number("min(3, 7)"), 3.0);
    assert_eq!(eval_to_number("max(3, 7)"), 7.0);
    assert_eq!(eval_to_number("floor(3.7)"), 3.0);
    assert_eq!(eval_to_number("ceil(3.2)"), 4.0);
    assert_eq!(eval_to_number("round(3.5)"), 4.0);
    assert_eq!(eval_to_number("sqrt(16)"), 4.0);
}

#[test]
fn test_user_defined_functions() {
    init_runtime();

    assert_eq!(
        eval_to_number("function double(x) { return x * 2 }; double(21)"),
        42.0
    );
    assert_eq!(
        eval_to_number("function add(a, b) { return a + b }; add(10, 20)"),
        30.0
    );
}

#[test]
fn test_recursive_functions() {
    init_runtime();

    assert_eq!(
        eval_to_number(
            "function factorial(n) { if n <= 1 { return 1 } else { return n * factorial(n - 1) } }; factorial(5)"
        ),
        120.0
    );
    assert_eq!(
        eval_to_number(
            "function fib(n) { if n <= 1 { return n } else { return fib(n-1) + fib(n-2) } }; fib(10)"
        ),
        55.0
    );
}
