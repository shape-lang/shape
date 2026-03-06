use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. Printing Enum Values
// =========================================================================

#[test]
fn print_unit_enum_variant() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

print(Color::Red)
print(Color::Green)
print(Color::Blue)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Red")
        .expect_output_contains("Green")
        .expect_output_contains("Blue");
}

#[test]
fn print_data_enum_variant() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

print(Status::Ok(200))
print(Status::Error("bad request"))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("200")
        .expect_output_contains("Error")
        .expect_output_contains("bad request");
}

// =========================================================================
// 2. Enum as Object Field Value
// =========================================================================

#[test]
fn enum_stored_in_object_field() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let response = { status: Status::Ok(200), body: "hello" }
let s = match response.status {
  Status::Ok(code) => f"status: {code}"
  Status::Error(msg) => f"error: {msg}"
}
print(s)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("status: 200");
}

// =========================================================================
// 3. Exhaustive Matching Enforcement
// =========================================================================

#[test]
fn enum_match_with_wildcard_is_exhaustive() {
    // Using _ wildcard should be accepted
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

fn to_num(c: Color) -> int {
  match c {
    Color::Red => 1
    _ => 0
  }
}
print(to_num(Color::Red))
print(to_num(Color::Green))
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1\n0");
}

// =========================================================================
// 4. Enum with Struct Variants
// =========================================================================

#[test]
fn enum_struct_variant_parses() {
    let code = r#"
enum Signal {
  Buy,
  Sell = "sell",
  Limit { price: number, size: number },
  Market(number, number)
}
0
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn enum_struct_variant_construction() {
    let code = r#"
enum Signal {
  Buy,
  Limit { price: number, size: number }
}

let a = Signal::Buy
let b = Signal::Limit { price: 100, size: 10 }
print(a)
print(b)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Buy")
        .expect_output_contains("Limit");
}

/// Regression: BUG-ENUM-003 is fixed. Struct variants now print with the
/// variant name (e.g. "Signal::Limit(...)" not "Signal::(...)").
#[test]
fn enum_struct_variant_construction_with_correct_print() {
    let code = r#"
enum Signal {
  Buy,
  Limit { price: number, size: number }
}

let a = Signal::Buy
let b = Signal::Limit { price: 100, size: 10 }
print(a)
print(b)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Buy")
        .expect_output_contains("Limit");
}

#[test]
fn enum_struct_variant_match() {
    let code = r#"
enum Signal {
  Buy,
  Limit { price: number, size: number }
}

let sig = Signal::Limit { price: 100, size: 10 }
let desc = match sig {
  Signal::Buy => "buy"
  Signal::Limit { price: p } => f"limit at {p}"
}
print(desc)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("limit at 100");
}

// =========================================================================
// 5. Enum with Assigned Values
// =========================================================================

#[test]
fn enum_with_string_assigned_value_parses() {
    let code = r#"
enum Signal {
  Buy = "buy",
  Sell = "sell",
  Hold = "hold"
}
0
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn enum_with_numeric_assigned_value_parses() {
    let code = r#"
enum Priority {
  Low = 0,
  Medium = 1,
  High = 2
}
0
"#;
    ShapeTest::new(code).expect_parse_ok();
}

// =========================================================================
// 6. Multiple Enums in Same Program
// =========================================================================

#[test]
fn multiple_enums_coexist() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

enum Status {
  Ok(int),
  Error(string)
}

let c = Color::Red
let s = Status::Ok(200)

let color_name = match c {
  Color::Red => "red"
  Color::Green => "green"
  Color::Blue => "blue"
}

let status_name = match s {
  Status::Ok(code) => f"ok({code})"
  Status::Error(msg) => f"err({msg})"
}

print(color_name)
print(status_name)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("red\nok(200)");
}

// =========================================================================
// 7. Enum Used in Loop
// =========================================================================

#[test]
fn enum_used_in_for_loop() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

fn name(c: Color) -> string {
  match c {
    Color::Red => "red"
    Color::Green => "green"
    Color::Blue => "blue"
  }
}

let colors = [Color::Red, Color::Green, Color::Blue]
for c in colors {
  print(name(c))
}
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("red\ngreen\nblue");
}

// =========================================================================
// 8. Enum Reassignment
// =========================================================================

#[test]
fn enum_variable_reassignment() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

let mut c = Color::Red
print(match c { Color::Red => "red" _ => "other" })
c = Color::Blue
print(match c { Color::Blue => "blue" _ => "other" })
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("red\nblue");
}

// =========================================================================
// 9. Parse-Only Tests for Grammar Coverage
// =========================================================================

#[test]
fn enum_all_variant_kinds_parse() {
    let code = r#"
enum Signal {
  Buy,
  Sell = "sell",
  Hold = 0,
  Limit { price: number, size: number },
  Market(number, number)
}
0
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn enum_constructor_expressions_parse() {
    let code = r#"
enum Signal {
  Buy,
  Limit { price: number, size: number },
  Market(number, number)
}

let a = Signal::Buy
let b = Signal::Limit { price: 1, size: 2 }
let c = Signal::Market(1, 2)
0
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn enum_match_pattern_qualified_parse() {
    let code = r#"
enum Signal {
  Buy,
  Limit { price: number }
}

let signal = Signal::Buy
let x = match signal {
  Signal::Buy => 1
  Signal::Limit { price: p } => p
}
0
"#;
    ShapeTest::new(code).expect_parse_ok();
}

// =========================================================================
// 10. LSP/Semantic Tests
// =========================================================================

#[test]
fn enum_definition_has_semantic_tokens() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}
"#;
    ShapeTest::new(code)
        .expect_parse_ok()
        .expect_semantic_tokens();
}

#[test]
fn enum_with_match_has_semantic_tokens() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

fn render(status: Status) -> string {
  match status {
    Status::Ok(code) => f"ok({code})"
    Status::Error(msg) => f"error({msg})"
  }
}
"#;
    ShapeTest::new(code)
        .expect_parse_ok()
        .expect_semantic_tokens();
}

// =========================================================================
// 11. Enum with Typed Function Parameters (Parser Test)
// =========================================================================

#[test]
fn enum_with_typed_function_param_parses() {
    let code = r#"
enum Status { Active, Inactive, Pending }

function check(s: Status) {
  return match s {
    Status::Active => "yes"
    _ => "no"
  }
}
0
"#;
    ShapeTest::new(code).expect_parse_ok();
}

// =========================================================================
// 12. Enum Basics from Program Tests (programs_enums_and_matching.rs)
// =========================================================================

#[test]
fn test_enum_unit_variant_definition() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red
    "#,
    )
    .expect_run_ok();
}

#[test]
fn test_enum_unit_variant_red() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        print(Color::Red)
    "#,
    )
    .expect_output_contains("Red");
}

#[test]
fn test_enum_unit_variant_green() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        print(Color::Green)
    "#,
    )
    .expect_output_contains("Green");
}

#[test]
fn test_enum_unit_variant_blue() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        print(Color::Blue)
    "#,
    )
    .expect_output_contains("Blue");
}

#[test]
fn test_enum_payload_variant_single() {
    ShapeTest::new(
        r#"
        enum Shape { Circle(number) }
        let c = Shape::Circle(5.0)
        print(c)
    "#,
    )
    .expect_output_contains("Circle");
}

#[test]
fn test_enum_payload_variant_multi() {
    ShapeTest::new(
        r#"
        enum Shape { Rect(number, number) }
        let r = Shape::Rect(3.0, 4.0)
        print(r)
    "#,
    )
    .expect_output_contains("Rect");
}

#[test]
fn test_enum_mixed_unit_and_payload() {
    ShapeTest::new(
        r#"
        enum Shape { Point, Circle(number), Rect(number, number) }
        let a = Shape::Point
        let b = Shape::Circle(1.0)
        let c = Shape::Rect(2.0, 3.0)
        print(a)
        print(b)
        print(c)
    "#,
    )
    .expect_output_contains("Point")
    .expect_output_contains("Circle")
    .expect_output_contains("Rect");
}

#[test]
fn test_enum_in_variable() {
    ShapeTest::new(
        r#"
        enum Dir { Up, Down, Left, Right }
        let d = Dir::Up
        print(d)
    "#,
    )
    .expect_output_contains("Up");
}

#[test]
fn test_enum_equality_same_variant() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red == Color::Red
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_enum_equality_different_variants() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red == Color::Green
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_enum_inequality_different_variants() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red != Color::Blue
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_enum_inequality_same_variant() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Green != Color::Green
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_enum_as_function_param() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        fn is_red(c) {
            match c {
                Color::Red => true,
                _ => false
            }
        }
        is_red(Color::Red)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_enum_as_function_param_false() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        fn is_red(c) {
            match c {
                Color::Red => true,
                _ => false
            }
        }
        is_red(Color::Blue)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_enum_as_function_return() {
    ShapeTest::new(
        r#"
        enum Status { Active, Inactive }
        fn get_status() {
            Status::Active
        }
        let s = get_status()
        print(s)
    "#,
    )
    .expect_output_contains("Active");
}

#[test]
fn test_enum_in_array() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        let colors = [Color::Red, Color::Green, Color::Blue]
        colors.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_enum_in_array_access() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        let colors = [Color::Red, Color::Green, Color::Blue]
        print(colors[1])
    "#,
    )
    .expect_output_contains("Green");
}

#[test]
fn test_enum_five_variants() {
    ShapeTest::new(
        r#"
        enum Weekday { Mon, Tue, Wed, Thu, Fri }
        fn is_midweek(d) {
            match d {
                Weekday::Wed => true,
                _ => false
            }
        }
        is_midweek(Weekday::Wed)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_enum_payload_variant_with_string() {
    ShapeTest::new(
        r#"
        enum Msg { Text(string), Code(int) }
        let m = Msg::Text("hello")
        print(m)
    "#,
    )
    .expect_output_contains("Text");
}

#[test]
fn test_enum_multiple_payload_printing() {
    ShapeTest::new(
        r#"
        enum Signal { Limit(int, int) }
        let s = Signal::Limit(100, 10)
        print(s)
    "#,
    )
    .expect_output_contains("Limit");
}
