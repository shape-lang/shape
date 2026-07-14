//! Runtime lifecycle hooks on functions transformed by comptime annotations.

use shape_test::shape_test::ShapeTest;

#[test]
fn before_hook_preserves_a_typed_array_parameter() {
    // The wrapper packs `data: Array<int>` into an outer `Array<Array<int>>`.
    // Returning args makes the implementation consume that nested carrier.
    ShapeTest::new(
        r#"
annotation preserve_args() {
  targets: [function]
  before(args, ctx) {
    args
  }
}

@preserve_args()
fn process(data: Array<int>) -> Array<int> {
  data
}

let processed = process([3, 5, 8])
print(processed[0] + processed[2])
"#,
    )
    .expect_run_ok()
    .expect_output("11");
}
