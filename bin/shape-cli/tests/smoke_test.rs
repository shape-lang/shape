//! Smoke test to verify CLI integration test infrastructure works.

mod common;

#[test]
fn test_eval_infrastructure() {
    common::init_runtime();
    assert_eq!(common::eval_to_number("1 + 1"), 2.0);
}
