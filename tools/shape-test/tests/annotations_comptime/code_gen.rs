//! Tests for annotation-driven code generation at compile time.
//!
//! Covers: annotations generating wrapper functions, generating serialization
//! methods, generating accessor methods, and composing generated code.

use shape_test::shape_test::ShapeTest;

#[test]
fn annotation_generates_display_method() {
    ShapeTest::new(
        r#"
annotation displayable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method display() { f"({self.x}, {self.y})" }
    }
  }
}

@displayable()
type Point { x: int, y: int }

let p = Point { x: 3, y: 4 }
print(p.display())
"#,
    )
    .expect_run_ok()
    .expect_output("(3, 4)");
}

#[test]
fn annotation_generates_getter_method() {
    ShapeTest::new(
        r#"
annotation with_getter() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method get_value() { self.value }
    }
  }
}

@with_getter()
type Container { value: int }

let c = Container { value: 42 }
print(c.get_value())
"#,
    )
    .expect_run_ok()
    .expect_output("42");
}

#[test]
fn annotation_replace_body_generates_constant_function() {
    // TDD: comptime post handlers with annotation params error "too many annotation arguments"
    ShapeTest::new(
        r#"
annotation stub_return(val) {
  targets: [function]
  comptime post(target, ctx) {
    replace body {
      "stubbed"
    }
  }
}

@stub_return("placeholder")
fn fetch_data() -> string {
  "real data from server"
}

print(fetch_data())
"#,
    )
    .expect_run_ok()
    .expect_output("stubbed");
}

#[test]
fn annotation_extends_type_with_equality_check() {
    ShapeTest::new(
        r#"
annotation with_eq() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method eq(other) { self.id == other.id }
    }
  }
}

@with_eq()
type Entity { id: int, name: string }

let a = Entity { id: 1, name: "Alice" }
let b = Entity { id: 1, name: "Bob" }
let c = Entity { id: 2, name: "Alice" }

print(a.eq(b))
print(a.eq(c))
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

#[test]
fn stacked_annotations_both_extend_type() {
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

annotation with_diff() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method diff() { self.a - self.b }
    }
  }
}

@with_sum()
@with_diff()
type Numbers { a: int, b: int }

let n = Numbers { a: 10, b: 3 }
print(n.sum())
print(n.diff())
"#,
    )
    .expect_run_ok()
    .expect_output("13\n7");
}

// TDD: annotation code gen for serialization methods requires string building from fields
#[test]
fn annotation_generates_to_string_method() {
    ShapeTest::new(
        r#"
annotation stringable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method to_str() { f"{self.name}:{self.value}" }
    }
  }
}

@stringable()
type Config { name: string, value: int }

let c = Config { name: "timeout", value: 30 }
print(c.to_str())
"#,
    )
    .expect_run_ok()
    .expect_output("timeout:30");
}

#[test]
fn annotation_generates_predicate_method() {
    ShapeTest::new(
        r#"
annotation checkable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method is_positive() { self.value > 0 }
    }
  }
}

@checkable()
type Measurement { value: int }

let pos = Measurement { value: 5 }
let neg = Measurement { value: -3 }
print(pos.is_positive())
print(neg.is_positive())
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}
