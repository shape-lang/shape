use crate::common::{eval_to_number, init_runtime};

#[test]
fn test_if_else() {
    init_runtime();

    // if-else is a statement in Shape, so use variable assignment to capture the result
    assert_eq!(
        eval_to_number("let x = 0; if true { x = 1 } else { x = 2 }; x"),
        1.0
    );
    assert_eq!(
        eval_to_number("let x = 0; if false { x = 1 } else { x = 2 }; x"),
        2.0
    );
    assert_eq!(
        eval_to_number("let x = 0; if 5 > 3 { x = 10 } else { x = 20 }; x"),
        10.0
    );
}

#[test]
fn test_ternary() {
    init_runtime();

    assert_eq!(eval_to_number("true ? 1 : 2"), 1.0);
    assert_eq!(eval_to_number("false ? 1 : 2"), 2.0);
    assert_eq!(eval_to_number("5 > 3 ? 10 : 20"), 10.0);
}

#[test]
fn test_for_loop() {
    init_runtime();

    assert_eq!(
        eval_to_number("let sum = 0; for i in range(5) { sum = sum + i }; sum"),
        10.0
    );
    assert_eq!(
        eval_to_number("let sum = 0; for i in range(1, 6) { sum = sum + i }; sum"),
        15.0
    );
}

#[test]
fn test_while_loop() {
    init_runtime();

    assert_eq!(
        eval_to_number("let i = 0; while i < 5 { i = i + 1 }; i"),
        5.0
    );
    assert_eq!(
        eval_to_number("let sum = 0; let i = 1; while i <= 5 { sum = sum + i; i = i + 1 }; sum"),
        15.0
    );
}
