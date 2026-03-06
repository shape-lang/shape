use shape_test::shape_test::{pos, ShapeTest};

#[test]
fn lsp_and_runtime_combined() {
    ShapeTest::new("let x = 1 + 2\nx\n")
        .at(pos(0, 4))
        .expect_hover_contains("Variable")
        .expect_semantic_tokens()
        .expect_run_ok()
        .expect_number(3.0);
}
