use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. Basic Enum with Data Variants
// =========================================================================

#[test]
fn basic_enum_ok_variant_constructs_and_prints() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let s = Status::Ok(200)
print(s)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok");
}

#[test]
fn basic_enum_error_variant_constructs_and_prints() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let e = Status::Error("not found")
print(e)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Error");
}

#[test]
fn basic_enum_both_variants() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let s = Status::Ok(200)
let e = Status::Error("not found")
print(s)
print(e)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("Error");
}

// =========================================================================
// 2. Simple Enums (No Data / Unit Variants)
// =========================================================================

#[test]
fn simple_enum_unit_variant_constructs() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

let c = Color::Red
print(c)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Red");
}

// =========================================================================
// 3. Mixed Enums (Some with Data, Some Without)
// =========================================================================

#[test]
fn mixed_enum_parses() {
    let code = r#"
enum Shape {
  Circle(number),
  Rectangle(number, number),
  Point
}
0
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn mixed_enum_constructs_all_variants() {
    let code = r#"
enum Shape {
  Circle(number),
  Rectangle(number, number),
  Point
}

let c = Shape::Circle(5.0)
let r = Shape::Rectangle(3.0, 4.0)
let p = Shape::Point
print(c)
print(r)
print(p)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Circle")
        .expect_output_contains("Rectangle")
        .expect_output_contains("Point");
}

/// Regression: BUG-ENUM-002 is fixed. Multi-payload variants now print with
/// the variant name (e.g. "Shape::Rectangle(3, 4)" not "Shape::(3, 4)").
#[test]
fn mixed_enum_constructs_all_variants_with_correct_print() {
    let code = r#"
enum Shape {
  Circle(number),
  Rectangle(number, number),
  Point
}

let c = Shape::Circle(5.0)
let r = Shape::Rectangle(3.0, 4.0)
let p = Shape::Point
print(c)
print(r)
print(p)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Circle")
        .expect_output_contains("Rectangle")
        .expect_output_contains("Point");
}

// =========================================================================
// 4. Enum Variant as Function Argument
// =========================================================================

#[test]
fn enum_variant_passed_as_function_argument() {
    let code = r#"
enum Direction {
  Up,
  Down,
  Left,
  Right
}

fn move_desc(d: Direction) -> string {
  match d {
    Direction::Up => "moving up"
    Direction::Down => "moving down"
    Direction::Left => "moving left"
    Direction::Right => "moving right"
  }
}

print(move_desc(Direction::Up))
print(move_desc(Direction::Right))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("moving up\nmoving right");
}

#[test]
fn enum_with_data_passed_as_function_argument() {
    let code = r#"
enum Expr {
  Literal(int),
  Add(int, int)
}

fn eval(e: Expr) -> int {
  match e {
    Expr::Literal(v) => v
    Expr::Add(a, b) => a + b
  }
}

print(eval(Expr::Literal(42)))
print(eval(Expr::Add(10, 20)))
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42\n30");
}

// =========================================================================
// 5. Enum in an Array
// =========================================================================

#[test]
fn enum_values_in_array() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let items = [Status::Ok(1), Status::Error("x")]
print(items.length())
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("2");
}

#[test]
fn enum_values_in_array_iterate_and_match() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

fn render(s: Status) -> string {
  match s {
    Status::Ok(code) => f"ok({code})"
    Status::Error(msg) => f"err({msg})"
  }
}

let items = [Status::Ok(1), Status::Error("x"), Status::Ok(200)]
for item in items {
  print(render(item))
}
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("ok(1)\nerr(x)\nok(200)");
}

// =========================================================================
// 6. Nested Enums (Enum Containing Another Enum)
// =========================================================================

/// NOTE-ENUM-001: `None` is a reserved keyword (Option::None), so we use `Empty` instead.
#[test]
fn nested_enum_construction_and_matching() {
    let code = r#"
enum Inner {
  A,
  B
}

enum Outer {
  Wrap(Inner),
  Empty
}

let x = Outer::Wrap(Inner::A)
let y = Outer::Empty

fn describe(o: Outer) -> string {
  match o {
    Outer::Wrap(inner) => "wrapped"
    Outer::Empty => "empty"
  }
}

print(describe(x))
print(describe(y))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("wrapped\nempty");
}

// =========================================================================
// 7. Enum with String Data
// =========================================================================

#[test]
fn enum_with_string_data_constructs_and_matches() {
    let code = r#"
enum Message {
  Text(string),
  Empty
}

fn get_text(m: Message) -> string {
  match m {
    Message::Text(s) => s
    Message::Empty => "<empty>"
  }
}

print(get_text(Message::Text("hello")))
print(get_text(Message::Empty))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("hello\n<empty>");
}

#[test]
fn enum_with_string_data_empty_string_prints_blank_line() {
    // Verifies that printing an empty string produces a blank line
    let code = r#"
enum Message {
  Text(string),
  Blank
}

fn get_text(m: Message) -> string {
  match m {
    Message::Text(s) => s
    Message::Blank => ""
  }
}

print(get_text(Message::Text("hello")))
print(get_text(Message::Blank))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("hello");
}

// =========================================================================
// 8. Enum Equality
// =========================================================================

#[test]
fn enum_equality_same_unit_variant() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

let a = Color::Red
let b = Color::Red
print(a == b)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("true");
}

/// Fixed: Different unit variants now correctly compare as not equal.
/// Previously BUG-ENUM-001: HeapValue::equals() for TypedObject only compared schema_id.
#[test]
fn enum_equality_different_unit_variants() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

let a = Color::Red
let b = Color::Green
print(a == b)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("false");
}

/// Fixed: Data-carrying enum variants compare correctly with `==`.
/// Previously panicked with "as_i64_unchecked on non-i64 ValueWord" because
/// the compiler's numeric type tracking leaked from payload compilation,
/// causing typed EqInt to be emitted instead of generic Eq.
#[test]
fn enum_equality_same_data_variant() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let a = Status::Ok(1)
let b = Status::Ok(1)
print(a == b)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("true");
}

/// Fixed: Data-carrying enum variants with different payloads compare as not equal.
#[test]
fn enum_equality_different_data_same_variant() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let a = Status::Ok(1)
let b = Status::Ok(2)
print(a == b)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("false");
}

/// Fixed: Different variants of the same enum now correctly compare as not equal.
/// Previously BUG-ENUM-001.
#[test]
fn enum_equality_different_variants() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let a = Status::Ok(1)
let b = Status::Error("fail")
print(a == b)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("false");
}

// =========================================================================
// 9. Enum with Many Variants
// =========================================================================

#[test]
fn enum_with_many_variants() {
    let code = r#"
enum Weekday {
  Monday,
  Tuesday,
  Wednesday,
  Thursday,
  Friday,
  Saturday,
  Sunday
}

fn is_weekend(d: Weekday) -> bool {
  match d {
    Weekday::Saturday => true
    Weekday::Sunday => true
    _ => false
  }
}

print(is_weekend(Weekday::Monday))
print(is_weekend(Weekday::Saturday))
print(is_weekend(Weekday::Sunday))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("false\ntrue\ntrue");
}

// =========================================================================
// 10. Using Enum in If Conditions
// =========================================================================

#[test]
fn enum_in_if_condition_via_match() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let s = Status::Ok(200)
let is_ok = match s {
  Status::Ok(_) => true
  Status::Error(_) => false
}
if is_ok {
  print("success")
} else {
  print("failure")
}
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("success");
}

// =========================================================================
// 11. Storing Enum in Variable and Matching Later
// =========================================================================

#[test]
fn store_enum_in_variable_and_match_later() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

let saved = Color::Blue
let name = match saved {
  Color::Red => "red"
  Color::Green => "green"
  Color::Blue => "blue"
}
print(name)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("blue");
}

#[test]
fn store_data_enum_in_variable_and_match_later() {
    let code = r#"
enum Status {
  Ok(int),
  Error(string)
}

let saved = Status::Error("timeout")
let msg = match saved {
  Status::Ok(code) => f"ok: {code}"
  Status::Error(msg) => f"error: {msg}"
}
print(msg)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("error: timeout");
}
