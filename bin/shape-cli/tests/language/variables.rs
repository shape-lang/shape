use crate::common::{eval_to_number, init_runtime};

#[test]
fn test_variable_declaration() {
    init_runtime();

    assert_eq!(eval_to_number("let x = 42; x"), 42.0);
    assert_eq!(eval_to_number("let x = 10; let y = 20; x + y"), 30.0);
}

#[test]
fn test_variable_assignment() {
    init_runtime();

    assert_eq!(eval_to_number("var x = 5; x = 10; x"), 10.0);
    assert_eq!(eval_to_number("var x = 1; x = x + 1; x = x + 1; x"), 3.0);
}
