use crate::common::{eval_to_number, init_runtime};

#[test]
fn test_null_coalescing() {
    init_runtime();

    assert_eq!(eval_to_number("None ?? 42"), 42.0);
    assert_eq!(eval_to_number("10 ?? 42"), 10.0);
}
