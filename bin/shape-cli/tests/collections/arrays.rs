use crate::common::{eval_to_number, init_runtime};

#[test]
fn test_array_literals() {
    init_runtime();

    assert_eq!(eval_to_number("let arr = [1, 2, 3]; arr[0]"), 1.0);
    assert_eq!(eval_to_number("let arr = [1, 2, 3]; arr[2]"), 3.0);
    assert_eq!(eval_to_number("len([1, 2, 3, 4, 5])"), 5.0);
}

#[test]
fn test_array_operations() {
    init_runtime();

    assert_eq!(eval_to_number("let arr = [10, 20, 30]; arr[1]"), 20.0);
    assert_eq!(
        eval_to_number("let arr = [1, 2, 3]; let mut sum = 0; for x in arr { sum = sum + x }; sum"),
        6.0
    );
}
