//! LSP semantic diagnostics tests: type constraint errors, undefined variables,
//! hoisted property access, struct literal validation, match exhaustiveness,
//! and intersection type assertions.

use shape_test::shape_test::{ShapeTest, pos};

// == Hoisted property diagnostics (from lsp_type_display) ====================

#[test]
fn semantic_diagnostic_reports_hoisted_read_in_formatted_string_before_assignment() {
    let code = "let x = { x: 1 }\nprint(f\": {x.y}\")\nx.y = 1\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Property 'y' does not exist")
        .expect_semantic_diagnostic_at_line_contains(1, "Property 'y' does not exist");
}

#[test]
fn semantic_diagnostic_does_not_anchor_unknown_property_to_commented_line() {
    let code = "let x = { x: 1 }\n// print(f\": {x.y}\")\nlet aa = x.y\nx.y = 1\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Property 'y' does not exist")
        .expect_semantic_diagnostic_at_line_contains(2, "Property 'y' does not exist");
}

#[test]
fn struct_literal_missing_field_diagnostic_points_to_literal_line() {
    let code = "type User { name: string }\nlet u = User { name: \"John\" }\nprint(User { user: \"John\" })\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Missing field 'name' in User struct literal")
        .expect_semantic_diagnostic_at_line_contains(
            2,
            "Missing field 'name' in User struct literal",
        );
}

// == Intersection type assertions (from lsp_type_display) ====================

#[test]
fn semantic_diagnostic_does_not_reject_valid_named_intersection_assertion() {
    let code = r#"
let a = { x: 1}
let b = { z: 3}
a.y = 2
let c = a+b
type TypeA {x: int, y: int}
type TypeB {z: int}
let (f:TypeA, g: TypeB) = c as (TypeA+TypeB)
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn named_intersection_destructure_does_not_report_f_or_g_undefined() {
    let code = r#"
let a = { x: 1}
let b = { z: 3}
a.y = 2
let c = a+b
type TypeA {x: int, y: int}
type TypeB {z: int}
let (f:TypeA, g: TypeB) = c as (TypeA+TypeB)
print(f, g)
"#;
    ShapeTest::new(code)
        .expect_no_semantic_diagnostic_contains("Undefined variable: 'f'")
        .expect_no_semantic_diagnostic_contains("Undefined variable: 'g'");
}

#[test]
fn exact_named_and_inline_destructure_keeps_f_and_g_defined_for_lsp() {
    let code = r#"
let a = { x: 1}
let b = { z: 3}
//print(a.y) //compiler error: no y (even though a has y in the shape via optimistic hoisting, see next line)
a.y = 2
print(a.y) //works!
let c = a+b //resulting type is {x: int, y: int, z: int}
//destructuring works, e.g.
let (d:{x}, e: {y, z})  = c
//destructuring to named structs works also but need the as keyword:
type TypeA {x: int, y: int}
type TypeB {z: int}
let (f:TypeA, g: TypeB) = c as (TypeA+TypeB)
print(f, g)
"#;
    ShapeTest::new(code)
        .expect_no_semantic_diagnostic_contains("Undefined variable: 'f'")
        .expect_no_semantic_diagnostic_contains("Undefined variable: 'g'")
        .at(pos(13, 6))
        .expect_hover_exists()
        .expect_hover_contains("Variable")
        .at(pos(13, 9))
        .expect_hover_exists()
        .expect_hover_contains("Variable");
}

// == Undefined variable diagnostics (from lsp_type_display) ==================

#[test]
fn semantic_diagnostic_combines_undefined_variables_on_same_line() {
    let code = "print(h, i)\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Undefined variables: 'h', 'i'")
        .expect_no_semantic_diagnostic_contains("Undefined variable: 'h'")
        .expect_no_semantic_diagnostic_contains("Undefined variable: 'i'");
}

#[test]
fn semantic_diagnostic_reports_undefined_variables_across_lines() {
    let code = "print(h)\nprint(i)\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_count(2)
        .expect_semantic_diagnostic_count_at_line(0, 1)
        .expect_semantic_diagnostic_count_at_line(1, 1)
        .expect_semantic_diagnostic_at_line_contains(0, "Undefined variable")
        .expect_semantic_diagnostic_at_line_contains(0, "h")
        .expect_semantic_diagnostic_at_line_contains(1, "Undefined variable")
        .expect_semantic_diagnostic_at_line_contains(1, "i");
}

#[test]
fn semantic_diagnostic_combines_same_line_and_keeps_next_line() {
    let code = "print(h, i)\nprint(j)\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_count(2)
        .expect_semantic_diagnostic_contains("Undefined variables: 'h', 'i'")
        .expect_semantic_diagnostic_count_at_line(0, 1)
        .expect_semantic_diagnostic_count_at_line(1, 1)
        .expect_semantic_diagnostic_at_line_contains(1, "Undefined variable")
        .expect_semantic_diagnostic_at_line_contains(1, "j");
}

// == Type constraint diagnostics (from lsp_type_display) =====================

#[test]
fn function_param_numeric_constraint_rejects_object_callsites() {
    let code = "fn afunc(c) {\n  c = c + 1\n  return c\n}\nlet x = { x: 1 }\nprint(afunc(x))\nprint(afunc(1))\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Could not solve type constraints")
        .expect_semantic_diagnostic_at_line_contains(1, "Could not solve type constraints");
}

#[test]
fn function_param_numeric_constraint_with_print_reports_on_numeric_line() {
    let code = "fn afunc(c) {\n  print(\"func called with \" + c)\n  c = c + 1\n  return c\n}\nlet x = { x: 1 }\nprint(afunc(x))\nprint(afunc(1))\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Could not solve type constraints")
        .expect_semantic_diagnostic_at_line_contains(2, "Could not solve type constraints");
}

#[test]
fn function_param_numeric_constraint_with_typed_match_reports_assignment_line() {
    let code = r#"
fn afunc(c) {
  print("func called with " + c)
  let result = match c {
    c: int => c + 1
    _ => 1
  }
  c = c + 1
  return c
}
let x = { x: 1, y: 2 }
print(afunc(x))
print(afunc(1))
"#;

    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Could not solve type constraints")
        .expect_semantic_diagnostic_at_line_contains(7, "Could not solve type constraints");
}

#[test]
fn empty_match_arm_does_not_suppress_following_numeric_diagnostic() {
    let code = r#"
fn afunc(c) {
  print("func called with " + c)
  match c {

  }
  c = c + 1
  return c
}
let x = { x: 1, y: 2 }
print(afunc(x))
print(afunc(1))
"#;

    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Could not solve type constraints")
        .expect_semantic_diagnostic_at_line_contains(6, "Could not solve type constraints");
}

#[test]
fn empty_enum_match_reports_non_exhaustive_diagnostic() {
    let code = r#"
enum Snapshot {
  Hash(int),
  Resumed
}

fn take(s: Snapshot) {
  match s {
  }
}
"#;

    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Non-exhaustive match")
        .expect_semantic_diagnostic_at_line_contains(7, "Non-exhaustive match");
}

#[test]
fn semantic_diagnostic_reports_unknown_property_access() {
    let code = "let a = { x: 1 }\nprint(a.y)\na.y = 2\n";
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Property 'y' does not exist");
}

#[test]
fn semantic_diagnostic_reports_unconstrained_result_generic_from_err_only() {
    let code = "fn test() {\n  Err(\"some error\")\n}\n";
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Could not infer generic type arguments for 'Result'")
        .expect_semantic_diagnostic_at_line_contains(
            1,
            "Could not infer generic type arguments for 'Result'",
        );
}

// == Comptime diagnostics (from lsp_comptime) ================================

#[test]
fn generated_method_call_from_comptime_extend_has_no_semantic_diagnostics() {
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
let p = Point { x: 1, y: 2 }
let v = p.sum()
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

// == Diagnostics deep (from programs_lsp_completeness) =======================

#[test]
fn test_lsp_diagnostic_missing_struct_field() {
    let code =
        "type User { name: string }\nlet u = User { name: \"Alice\" }\nprint(User { age: 30 })\n";
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Missing field 'name'");
}

#[test]
fn test_lsp_diagnostic_no_errors_on_valid() {
    let code = "let x = 42\nlet y = x + 1\n";
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn test_lsp_diagnostic_hoisted_property_read_before_write() {
    let code = "let obj = { a: 1 }\nprint(obj.b)\nobj.b = 2\n";
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Property 'b' does not exist");
}

#[test]
fn test_lsp_diagnostic_non_exhaustive_enum_match() {
    let code =
        "enum Light {\n  Red,\n  Yellow,\n  Green\n}\nfn check(l: Light) {\n  match l {\n  }\n}\n";
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Non-exhaustive match");
}

#[test]
fn test_lsp_diagnostic_type_constraint_error() {
    let code = "fn add_one(c) {\n  c = c + 1\n  return c\n}\nlet obj = { x: 1 }\nprint(add_one(obj))\nprint(add_one(1))\n";
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Could not solve type constraints");
}
