use crate::common::{eval_to_number, init_runtime};

#[test]
fn test_basic_arithmetic() {
    init_runtime();

    assert_eq!(eval_to_number("1 + 2"), 3.0);
    assert_eq!(eval_to_number("10 - 3"), 7.0);
    assert_eq!(eval_to_number("4 * 5"), 20.0);
    assert_eq!(eval_to_number("20 / 4"), 5.0);
    assert_eq!(eval_to_number("17 % 5"), 2.0);
    assert_eq!(eval_to_number("2 ** 3"), 8.0); // Power operator is **
}

#[test]
fn test_operator_precedence() {
    init_runtime();

    assert_eq!(eval_to_number("2 + 3 * 4"), 14.0);
    assert_eq!(eval_to_number("(2 + 3) * 4"), 20.0);
    assert_eq!(eval_to_number("10 - 4 / 2"), 8.0);
}

#[test]
fn test_unary_operators() {
    init_runtime();

    assert_eq!(eval_to_number("-5"), -5.0);
    assert_eq!(eval_to_number("-(-5)"), 5.0);
}
