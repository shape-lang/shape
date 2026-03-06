//! LSP comptime integration tests: comptime extend/execute runtime behavior.

use shape_test::shape_test::ShapeTest;

// == Comptime extend: runtime execution (from lsp_comptime) ==================

#[test]
fn generated_method_call_from_comptime_extend_executes() {
    let code = r#"
annotation add_sum() {
    targets: [type]
    comptime post(target, ctx) {
        extend target {
            method sum() { self.x + self.y }
        }
    }
}
@add_sum()
type Point { x: int, y: int }
Point { x: 1, y: 2 }.sum()
"#;
    ShapeTest::new(code).expect_number(3.0);
}
