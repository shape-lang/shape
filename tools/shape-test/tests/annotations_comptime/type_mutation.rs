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
annotation with_double() on type {
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
annotation with_magnitude() on type {
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
annotation with_scale() on type {
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
annotation deprecated() on type {
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
annotation with_empty_check() on type {
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
annotation with_info() on type {
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

// Annotation params are not captured into generated method bodies; the
// canonical undefined-variable diagnostic quotes the missing identifier.
#[test]
fn annotation_with_param_used_in_generated_method() {
    ShapeTest::new(
        r#"
annotation with_default(default_val: int) on type {
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
    .expect_run_err_contains("Undefined variable: 'default_val'");
}

#[test]
fn replace_body_on_function_target() {
    ShapeTest::new(
        r#"
annotation mock() on function {
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

#[test]
fn set_return_incompatible_with_body_is_compile_error() {
    // §4.5 / S3: a `set return` that disagrees with the body re-enters the
    // ordinary body-vs-signature type checker (the same path the explicit
    // `-> string` annotation takes). Previously this reinterpreted an int as a
    // string pointer and segfaulted; now it is an ordinary compile error.
    ShapeTest::new(
        r#"
annotation force_string_return() on function {
  comptime post(target, ctx) {
    set return string
  }
}

@force_string_return()
fn answer() {
  42
}

print(answer())
"#,
    )
    .expect_run_err_contains("comptime directive");
}

#[test]
fn set_return_compatible_with_body_still_compiles() {
    // The re-check only fires as an error on a genuine mismatch — a compatible
    // `set return int` on an int-bodied function still compiles and runs.
    ShapeTest::new(
        r#"
annotation force_int_return() on function {
  comptime post(target, ctx) {
    set return int
  }
}

@force_int_return()
fn answer() {
  42
}

print(answer())
"#,
    )
    .expect_run_ok()
    .expect_output("42");
}

#[test]
fn target_params_and_return_expose_type_refs() {
    ShapeTest::new(
        r#"
annotation assert_signature_refs() on function {
  comptime post(target, ctx) {
    if target.params[0].type_ref.kind != "String" {
      error(f"expected string param TypeRef, got {target.params[0].type_ref.kind}")
    }
    if target.return_type_ref.source != "string" {
      error(f"expected string return TypeRef, got {target.return_type_ref.source}")
    }
  }
}

@assert_signature_refs()
fn echo(value: string) -> string {
  value
}

print(echo("ok"))
"#,
    )
    .expect_run_ok()
    .expect_output("ok");
}

#[test]
fn set_return_accepts_type_ref_expression() {
    ShapeTest::new(
        r#"
annotation return_like_first_param() on function {
  comptime post(target, ctx) {
    set return (target.params[0].type_ref)
  }
}

@return_like_first_param()
fn echo(value: string) {
  value
}

print(echo("typed"))
"#,
    )
    .expect_run_ok()
    .expect_output("typed");
}

#[test]
fn set_param_type_accepts_type_ref_expression() {
    ShapeTest::new(
        r#"
annotation first_param_like_second() on function {
  comptime post(target, ctx) {
    if target.params[1].type != "string" {
      error(f"legacy param type string changed: {target.params[1].type}")
    }
    set param left: (target.params[1].type_ref)
  }
}

@first_param_like_second()
fn join(left, right: string) -> string {
  left + right
}

print(join("type", "ref"))
"#,
    )
    .expect_run_ok()
    .expect_output("typeref");
}

// ADR-009 E1 #17 slice-5 A-FULL composite e2e (integration canary). A
// `set return (target.params[0].type_ref)` where param 0 is an inline TUPLE
// `[int, int]` — a reconstructable STRUCTURAL composite (NOT `Array<int>`,
// proven non-reconstructable in E1 / stage 1; using it would force a forbidden
// silent fallback). A tuple is treated uniformly by the freeze canonicalizer
// regardless of element homogeneity (`FrozenTypeCategory::Tuple` with ordered
// element identities, type_reflection.rs), so `[int, int]` routes the composite
// IDENTITY path exactly as the heterogeneous form does — but `[int, int]` is
// also a constructible runtime VALUE (`[3, 4]`), while a heterogeneous tuple
// value is rejected (bracket VALUEs are homogeneous-only). The handler runs at
// compile time: the producer (stage 3) stamps the tuple type_ref with its
// composite identity into the ONE shared `Arc<FreezeOverlay>`, and the consumer
// (stage 4) resolves it identity-only off that SAME overlay back to `[int, int]`.
// If the shared-overlay plumbing were wrong the composite identity would not
// round-trip and this fails LOUDLY at compile time (E1-D7(a): no `.source`
// fallback) while the leaf corpus stays green — the exact differential the canary
// exists for.
#[test]
fn set_return_accepts_composite_type_ref_expression() {
    ShapeTest::new(
        r#"
annotation return_like_first_param() on function {
  comptime post(target, ctx) {
    set return (target.params[0].type_ref)
  }
}

@return_like_first_param()
fn echo(value: [int, int]) {
  value
}

let arg: [int, int] = [7, 9]
let pair = echo(arg)
print(pair[1])
"#,
    )
    .expect_run_ok()
    .expect_output("9");
}

// The `set param` sibling: `set param left: (target.params[1].type_ref)` where
// param 1 `right` is the inline TUPLE `[int, int]`. `left` (initially
// unannotated) is retyped to the composite via the identity route, then indexed
// as a tuple in the body. Same shared-overlay canary on the PARAM producer path
// (functions_annotations.rs site) rather than the return path.
#[test]
fn set_param_type_accepts_composite_type_ref_expression() {
    ShapeTest::new(
        r#"
annotation first_param_like_second() on function {
  comptime post(target, ctx) {
    set param left: (target.params[1].type_ref)
  }
}

@first_param_like_second()
fn pick(left, right: [int, int]) -> int {
  left[1]
}

let a: [int, int] = [1, 42]
let b: [int, int] = [2, 3]
print(pick(a, b))
"#,
    )
    .expect_run_ok()
    .expect_output("42");
}

#[test]
fn duplicate_annotation_application_is_compile_error() {
    // Q47 / §4.1.1: applying the same annotation twice to one target is a v1
    // compile error naming both application sites.
    ShapeTest::new(
        r#"
annotation tag() on function {
  comptime post(target, ctx) {
    set return int
  }
}

@tag()
@tag()
fn foo() {
  1
}

print(foo())
"#,
    )
    .expect_run_err_contains("more than once");
}
