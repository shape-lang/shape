use crate::common::{eval_to_bool, init_runtime};

#[test]
fn test_equality() {
    init_runtime();

    assert!(eval_to_bool("5 == 5"));
    assert!(!eval_to_bool("5 == 6"));
    assert!(eval_to_bool("5 != 6"));
    assert!(!eval_to_bool("5 != 5"));
}

#[test]
fn test_ordering() {
    init_runtime();

    assert!(eval_to_bool("3 < 5"));
    assert!(!eval_to_bool("5 < 3"));
    assert!(eval_to_bool("5 > 3"));
    assert!(!eval_to_bool("3 > 5"));
    assert!(eval_to_bool("5 <= 5"));
    assert!(eval_to_bool("5 >= 5"));
}

#[test]
fn test_logical_operators() {
    init_runtime();

    assert!(eval_to_bool("true && true"));
    assert!(!eval_to_bool("true && false"));
    assert!(eval_to_bool("false || true"));
    assert!(!eval_to_bool("false || false"));
    assert!(eval_to_bool("!false"));
    assert!(!eval_to_bool("!true"));
}
