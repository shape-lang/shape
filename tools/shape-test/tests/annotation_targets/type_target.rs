//! Tests for annotations applied to type targets.
//!
//! Covers: annotations on struct/type definitions using `targets: [type]`,
//! extend target adding methods, remove target removing types,
//! and multiple annotations on type definitions.

use shape_test::shape_test::ShapeTest;

#[test]
fn annotation_on_type_with_extend() {
    ShapeTest::new(
        r#"
annotation with_describe() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method describe() { f"Point({self.x}, {self.y})" }
    }
  }
}

@with_describe()
type Point { x: int, y: int }

let p = Point { x: 1, y: 2 }
print(p.describe())
"#,
    )
    .expect_run_ok()
    .expect_output("Point(1, 2)");
}

#[test]
fn annotation_on_type_with_remove() {
    ShapeTest::new(
        r#"
annotation strip() {
  targets: [type]
  comptime post(target, ctx) {
    remove target
  }
}

@strip()
type Temporary { data: int }

print("type removed at compile time")
"#,
    )
    .expect_run_ok()
    .expect_output("type removed at compile time");
}

#[test]
fn annotation_on_type_adds_computed_method() {
    ShapeTest::new(
        r#"
annotation measurable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method perimeter() { (self.w + self.h) * 2 }
    }
  }
}

@measurable()
type Rect { w: int, h: int }

let r = Rect { w: 10, h: 5 }
print(r.perimeter())
"#,
    )
    .expect_run_ok()
    .expect_output("30");
}

#[test]
fn two_annotations_on_same_type() {
    ShapeTest::new(
        r#"
annotation with_sum() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method sum() { self.a + self.b }
    }
  }
}

annotation with_product() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method product() { self.a * self.b }
    }
  }
}

@with_sum()
@with_product()
type Pair { a: int, b: int }

let p = Pair { a: 6, b: 7 }
print(p.sum())
print(p.product())
"#,
    )
    .expect_run_ok()
    .expect_output("13\n42");
}

#[test]
fn annotation_on_type_adds_boolean_method() {
    ShapeTest::new(
        r#"
annotation with_valid_check() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method is_valid() { self.status == 1 }
    }
  }
}

@with_valid_check()
type Record { id: int, status: int }

let valid = Record { id: 1, status: 1 }
let invalid = Record { id: 2, status: 0 }
print(valid.is_valid())
print(invalid.is_valid())
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

#[test]
fn annotation_on_type_with_before_after_hooks() {
    // before/after hooks only apply to function targets; on type targets
    // only comptime post handlers are meaningful. This tests that a
    // type annotation with comptime post works correctly.
    ShapeTest::new(
        r#"
annotation enriched() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method full_name() { f"{self.first} {self.last}" }
    }
  }
}

@enriched()
type Name { first: string, last: string }

let n = Name { first: "Jane", last: "Doe" }
print(n.full_name())
"#,
    )
    .expect_run_ok()
    .expect_output("Jane Doe");
}

// TDD: annotations on enum definitions parse error "expected an expression, found identifier `enum`"
#[test]
fn annotation_on_enum_type() {
    ShapeTest::new(
        r#"
annotation with_label() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method label() { "enum_value" }
    }
  }
}

@with_label()
enum Color {
  Red,
  Green,
  Blue
}

print("enum annotation defined")
"#,
    )
    .expect_run_ok()
    .expect_output_contains("enum annotation defined");
}

// TDD: annotations on trait definitions parse error "expected an expression, found identifier `trait`"
#[test]
fn annotation_on_trait_definition() {
    ShapeTest::new(
        r#"
annotation documented(desc) {
  targets: [type]
  comptime post(target, ctx) {
    // no-op, just metadata registration
  }
}

@documented("A printable trait")
trait Printable {
  to_string(self): string
}

print("trait annotation defined")
"#,
    )
    .expect_run_ok()
    .expect_output_contains("trait annotation defined");
}
