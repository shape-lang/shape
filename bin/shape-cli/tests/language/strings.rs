use crate::common::{eval_to_string, init_runtime};

#[test]
fn test_string_literals() {
    init_runtime();

    assert_eq!(eval_to_string(r#""hello""#), "hello");
    assert_eq!(eval_to_string(r#""hello" + " " + "world""#), "hello world");
}
