//! Runtime lifecycle hooks on functions transformed by comptime annotations.

use shape_test::shape_test::ShapeTest;

#[test]
fn before_hook_preserves_a_typed_array_parameter() {
    // ADR-009 C3-S6: the typed weave delivers `data: Array<int>` as a
    // per-param carrier (the C3-G9 pseudo-tuple; the legacy
    // `Array<Array<int>>` nested packing is deleted). Returning args makes
    // the implementation consume the woven carrier end-to-end.
    ShapeTest::new(
        r#"
annotation preserve_args() on function {
  before(args) {
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
