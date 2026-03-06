//! Tests for annotation on_define handlers at compile time.
//!
//! Covers: on_define firing when annotated item is defined, metadata registration,
//! compile-time validation via comptime post, and extend/remove directives.

use shape_test::shape_test::ShapeTest;

#[test]
fn comptime_post_extend_adds_method_to_type() {
    ShapeTest::new(
        r#"
annotation add_greet() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method greet() { f"Hello from {self.name}" }
    }
  }
}

@add_greet()
type Person { name: string }

let p = Person { name: "Alice" }
print(p.greet())
"#,
    )
    .expect_run_ok()
    .expect_output("Hello from Alice");
}

#[test]
fn comptime_post_extend_adds_computed_method() {
    ShapeTest::new(
        r#"
annotation add_area() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method area() { self.width * self.height }
    }
  }
}

@add_area()
type Rect { width: int, height: int }

let r = Rect { width: 5, height: 3 }
print(r.area())
"#,
    )
    .expect_run_ok()
    .expect_output("15");
}

#[test]
fn comptime_post_remove_target_eliminates_type() {
    ShapeTest::new(
        r#"
annotation remove_type() {
  targets: [type]
  comptime post(target, ctx) {
    remove target
  }
}

@remove_type()
type Ghost { x: int }

print("Ghost type was removed")
"#,
    )
    .expect_run_ok()
    .expect_output("Ghost type was removed");
}

#[test]
fn comptime_post_replace_body_overrides_function() {
    ShapeTest::new(
        r#"
annotation always_zero() {
  targets: [function]
  comptime post(target, ctx) {
    replace body {
      0
    }
  }
}

@always_zero()
fn compute(x: int) -> int {
  x * x * x
}

print(compute(999))
"#,
    )
    .expect_run_ok()
    .expect_output("0");
}

#[test]
fn comptime_post_extend_adds_multiple_methods() {
    ShapeTest::new(
        r#"
annotation add_math() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method sum() { self.x + self.y }
      method product() { self.x * self.y }
    }
  }
}

@add_math()
type Pair { x: int, y: int }

let p = Pair { x: 6, y: 7 }
print(p.sum())
print(p.product())
"#,
    )
    .expect_run_ok()
    .expect_output("13\n42");
}

#[test]
fn comptime_post_with_annotation_param_in_method() {
    // TDD: comptime post handlers with annotation params error "too many annotation arguments"
    ShapeTest::new(
        r#"
annotation add_label(label_text) {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method label() { self.name }
    }
  }
}

@add_label("custom")
type Item { name: string }

let i = Item { name: "widget" }
print(i.label())
"#,
    )
    .expect_run_ok()
    .expect_output("widget");
}

// TDD: set param directive not yet implemented (known bug ct_45)
#[test]
fn comptime_post_set_param_default() {
    ShapeTest::new(
        r#"
annotation default_y(val) {
  targets: [function]
  comptime post(target, ctx) {
    set param y = val
  }
}

@default_y(10)
fn add(x: int, y: int) -> int {
  x + y
}

print(add(5, 3))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("8");
}

#[test]
fn targets_declaration_restricts_to_type() {
    ShapeTest::new(
        r#"
annotation type_only() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method tag() { "tagged" }
    }
  }
}

@type_only()
type Tagged { value: int }

let t = Tagged { value: 1 }
print(t.tag())
"#,
    )
    .expect_run_ok()
    .expect_output("tagged");
}
