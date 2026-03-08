//! Tests for annotation-driven type mutation at compile time.
//!
//! Covers: annotations modifying type information via extend, adding fields
//! through generated methods, comptime pre/post handler behavior,
//! and type-level annotation interactions.

use shape_test::shape_test::ShapeTest;

#[test]
fn extend_target_adds_derived_method() {
    ShapeTest::new(
        r#"
annotation with_double() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method doubled() { self.value * 2 }
    }
  }
}

@with_double()
type Wrapper { value: int }

let w = Wrapper { value: 21 }
print(w.doubled())
"#,
    )
    .expect_run_ok()
    .expect_output("42");
}

#[test]
fn extend_target_adds_method_using_multiple_fields() {
    ShapeTest::new(
        r#"
annotation with_magnitude() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method magnitude() { self.x + self.y + self.z }
    }
  }
}

@with_magnitude()
type Vec3 { x: int, y: int, z: int }

let v = Vec3 { x: 1, y: 2, z: 3 }
print(v.magnitude())
"#,
    )
    .expect_run_ok()
    .expect_output("6");
}

#[test]
fn extend_target_method_with_parameters() {
    ShapeTest::new(
        r#"
annotation with_scale() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method scale(factor) { self.value * factor }
    }
  }
}

@with_scale()
type Scalar { value: int }

let s = Scalar { value: 5 }
print(s.scale(3))
print(s.scale(10))
"#,
    )
    .expect_run_ok()
    .expect_output("15\n50");
}

#[test]
fn annotation_removes_and_replaces_type() {
    ShapeTest::new(
        r#"
annotation deprecated() {
  targets: [type]
  comptime post(target, ctx) {
    remove target
  }
}

@deprecated()
type OldConfig { value: int }

print("OldConfig removed successfully")
"#,
    )
    .expect_run_ok()
    .expect_output("OldConfig removed successfully");
}

#[test]
fn annotation_extends_type_with_boolean_method() {
    ShapeTest::new(
        r#"
annotation with_empty_check() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method is_empty() { self.count == 0 }
    }
  }
}

@with_empty_check()
type Collection { count: int }

let empty = Collection { count: 0 }
let full = Collection { count: 5 }
print(empty.is_empty())
print(full.is_empty())
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

#[test]
fn annotation_extends_type_with_string_method() {
    ShapeTest::new(
        r#"
annotation with_info() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method info() { f"{self.name} (age {self.age})" }
    }
  }
}

@with_info()
type Person { name: string, age: int }

let p = Person { name: "Alice", age: 30 }
print(p.info())
"#,
    )
    .expect_run_ok()
    .expect_output("Alice (age 30)");
}

// TDD: comptime post handlers with annotation params error "too many annotation arguments"
#[test]
fn annotation_with_param_used_in_generated_method() {
    ShapeTest::new(
        r#"
annotation with_default(default_val) {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method get_or_default() {
        if self.value == 0 {
          default_val
        } else {
          self.value
        }
      }
    }
  }
}

@with_default(42)
type MaybeValue { value: int }

let zero = MaybeValue { value: 0 }
let nonzero = MaybeValue { value: 7 }
print(zero.get_or_default())
print(nonzero.get_or_default())
"#,
    )
    .expect_run_err_contains("Undefined variable: default_val");
}

#[test]
fn replace_body_on_function_target() {
    ShapeTest::new(
        r#"
annotation mock() {
  targets: [function]
  comptime post(target, ctx) {
    replace body {
      "mocked"
    }
  }
}

@mock()
fn get_api_data() -> string {
  "real api response"
}

print(get_api_data())
"#,
    )
    .expect_run_ok()
    .expect_output("mocked");
}
