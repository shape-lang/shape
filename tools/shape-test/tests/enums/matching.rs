use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. Enum Matching (from main.rs)
// =========================================================================

#[test]
fn enum_match_returns_correct_string_for_ok() {
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
print(render(Status::Ok(200)))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("ok(200)");
}

#[test]
fn enum_match_returns_correct_string_for_error() {
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
print(render(Status::Error("not found")))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("error(not found)");
}

#[test]
fn enum_match_both_arms() {
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
print(render(Status::Ok(200)))
print(render(Status::Error("not found")))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("ok(200)\nerror(not found)");
}

#[test]
fn simple_enum_match_all_variants() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

fn describe_color(c: Color) -> string {
  match c {
    Color::Red => "red"
    Color::Green => "green"
    Color::Blue => "blue"
  }
}
print(describe_color(Color::Green))
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("green");
}

#[test]
fn simple_enum_match_each_variant() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

fn describe_color(c: Color) -> string {
  match c {
    Color::Red => "red"
    Color::Green => "green"
    Color::Blue => "blue"
  }
}
print(describe_color(Color::Red))
print(describe_color(Color::Green))
print(describe_color(Color::Blue))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("red\ngreen\nblue");
}

#[test]
fn mixed_enum_match_all_variants() {
    let code = r#"
enum Shape {
  Circle(number),
  Rectangle(number, number),
  Point
}

fn describe(s: Shape) -> string {
  match s {
    Shape::Circle(r) => f"circle(r={r})"
    Shape::Rectangle(w, h) => f"rect({w}x{h})"
    Shape::Point => "point"
  }
}
print(describe(Shape::Circle(5.0)))
print(describe(Shape::Rectangle(3.0, 4.0)))
print(describe(Shape::Point))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("circle(r=5)\nrect(3x4)\npoint");
}

// =========================================================================
// 2. Match Is an Expression
// =========================================================================

#[test]
fn match_is_an_expression_returns_value() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

let color = Color::Green
let val = match color {
  Color::Red => 1
  Color::Green => 2
  Color::Blue => 3
}
print(val)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("2");
}

// =========================================================================
// 3. Enum with Complex Match Logic
// =========================================================================

#[test]
fn enum_match_with_computation_in_arms() {
    let code = r#"
enum Shape {
  Circle(number),
  Rectangle(number, number),
  Point
}

fn area(s: Shape) -> number {
  match s {
    Shape::Circle(r) => 3.14159 * r * r
    Shape::Rectangle(w, h) => w * h
    Shape::Point => 0
  }
}

print(area(Shape::Rectangle(3.0, 4.0)))
print(area(Shape::Point))
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("12\n0");
}

// =========================================================================
// 4. Pattern Matching on Literals
// =========================================================================

#[test]
fn test_match_on_int_literal() {
    ShapeTest::new(
        r#"
        let x = 2
        match x {
            1 => "one",
            2 => "two",
            3 => "three",
            _ => "other"
        }
    "#,
    )
    .expect_string("two");
}

#[test]
fn test_match_on_string_literal() {
    ShapeTest::new(
        r#"
        let s = "hello"
        match s {
            "hello" => 1,
            "world" => 2,
            _ => 0
        }
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_match_on_bool_literal() {
    ShapeTest::new(
        r#"
        let b = false
        match b {
            true => "yes",
            false => "no"
        }
    "#,
    )
    .expect_string("no");
}

#[test]
fn test_match_wildcard() {
    ShapeTest::new(
        r#"
        let x = 99
        match x {
            1 => "one",
            _ => "other"
        }
    "#,
    )
    .expect_string("other");
}

// =========================================================================
// 5. Match with Guards
// =========================================================================

#[test]
fn test_match_with_guard_positive() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            match x {
                n where n > 0 => "positive",
                n where n < 0 => "negative",
                _ => "zero"
            }
        }
        classify(5)
    "#,
    )
    .expect_string("positive");
}

#[test]
fn test_match_with_guard_negative() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            match x {
                n where n > 0 => "positive",
                n where n < 0 => "negative",
                _ => "zero"
            }
        }
        classify(-3)
    "#,
    )
    .expect_string("negative");
}

#[test]
fn test_match_with_guard_zero() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            match x {
                n where n > 0 => "positive",
                n where n < 0 => "negative",
                _ => "zero"
            }
        }
        classify(0)
    "#,
    )
    .expect_string("zero");
}

#[test]
fn test_match_as_expression_in_let() {
    ShapeTest::new(
        r#"
        let x = 42
        let label = match x {
            42 => "the answer",
            _ => "not the answer"
        }
        label
    "#,
    )
    .expect_string("the answer");
}

// =========================================================================
// 6. Constructor Patterns in Match
// =========================================================================

// BUG: Bare enum variant patterns (Some/None) require type-resolved enum context
#[test]
fn test_match_constructor_some() {
    ShapeTest::new(
        r#"
        let opt = Some(7)
        match opt {
            Some(v) => v + 3,
            None => 0
        }
    "#,
    )
    .expect_number(10.0);
}

// BUG: Bare enum variant patterns (Some/None) require type-resolved enum context
#[test]
fn test_match_constructor_none() {
    ShapeTest::new(
        r#"
        let opt = None
        match opt {
            Some(v) => v,
            None => 99
        }
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn test_match_constructor_ok() {
    ShapeTest::new(
        r#"
        let r = Ok(100)
        match r {
            Ok(v) => v,
            Err(e) => 0
        }
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn test_match_constructor_err() {
    ShapeTest::new(
        r#"
        let r = Err("oops")
        match r {
            Ok(v) => 0,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// 7. Multiple Arms and First-Match Semantics
// =========================================================================

#[test]
fn test_match_multiple_arms_first_wins() {
    ShapeTest::new(
        r#"
        let x = 10
        match x {
            n where n > 5 => "big",
            n where n > 0 => "small",
            _ => "zero or negative"
        }
    "#,
    )
    .expect_string("big");
}

#[test]
fn test_match_multiple_arms_second_wins() {
    ShapeTest::new(
        r#"
        let x = 3
        match x {
            n where n > 5 => "big",
            n where n > 0 => "small",
            _ => "zero or negative"
        }
    "#,
    )
    .expect_string("small");
}

// =========================================================================
// 8. Complex Guards
// =========================================================================

#[test]
fn test_match_complex_guard_and() {
    ShapeTest::new(
        r#"
        fn check(x) {
            match x {
                n where n > 10 and n < 20 => "teen",
                _ => "other"
            }
        }
        check(15)
    "#,
    )
    .expect_string("teen");
}

#[test]
fn test_match_complex_guard_or() {
    ShapeTest::new(
        r#"
        fn check(x) {
            match x {
                n where n == 1 or n == 2 => "low",
                _ => "other"
            }
        }
        check(2)
    "#,
    )
    .expect_string("low");
}

// =========================================================================
// 9. Match in Function Bodies
// =========================================================================

#[test]
fn test_match_in_function_body() {
    ShapeTest::new(
        r#"
        fn to_string(b) {
            match b {
                true => "yes",
                false => "no"
            }
        }
        to_string(true)
    "#,
    )
    .expect_string("yes");
}

#[test]
fn test_match_return_from_function() {
    ShapeTest::new(
        r#"
        fn label(x) {
            return match x {
                1 => "one",
                2 => "two",
                _ => "many"
            }
        }
        label(2)
    "#,
    )
    .expect_string("two");
}

// =========================================================================
// 10. Match on Enum Variants
// =========================================================================

#[test]
fn test_match_on_enum_variants() {
    ShapeTest::new(
        r#"
        enum Dir { Up, Down, Left, Right }
        fn delta(d) {
            match d {
                Dir::Up => 1,
                Dir::Down => -1,
                Dir::Left => -10,
                Dir::Right => 10
            }
        }
        delta(Dir::Right)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_match_enum_with_binding() {
    ShapeTest::new(
        r#"
        enum Msg { Text(string), Number(int) }
        fn describe(m) {
            match m {
                Msg::Text(s) => "text:" + s,
                Msg::Number(n) => "num"
            }
        }
        describe(Msg::Text("hi"))
    "#,
    )
    .expect_string("text:hi");
}

// =========================================================================
// 11. Match Inside Loops
// =========================================================================

// BUG: Bare enum variant patterns (Some/None) require type-resolved enum context
#[test]
fn test_match_inside_loop() {
    ShapeTest::new(
        r#"
        let items = [Some(1), None, Some(3), None, Some(5)]
        let sum = 0
        for item in items {
            sum = sum + match item {
                Some(v) => v,
                None => 0
            }
        }
        sum
    "#,
    )
    .expect_number(9.0);
}

#[test]
fn test_match_on_array_elements() {
    ShapeTest::new(
        r#"
        let arr = [Ok(10), Err("skip"), Ok(20)]
        let sum = 0
        for el in arr {
            let v = match el {
                Ok(n) => n,
                Err(e) => 0
            }
            sum = sum + v
        }
        sum
    "#,
    )
    .expect_number(30.0);
}

// =========================================================================
// 12. Typed Patterns
// =========================================================================

#[test]
fn test_match_typed_pattern_int() {
    ShapeTest::new(
        r#"
        fn process(x) {
            match x {
                n: int => n + 1,
                _ => 0
            }
        }
        process(41)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_match_typed_pattern_string() {
    ShapeTest::new(
        r#"
        fn describe(x) {
            match x {
                s: string => "got string",
                _ => "not string"
            }
        }
        describe("hello")
    "#,
    )
    .expect_string("got string");
}

#[test]
fn test_match_fall_through_to_wildcard() {
    ShapeTest::new(
        r#"
        let x = 100
        match x {
            1 => "one",
            2 => "two",
            3 => "three",
            _ => "unknown"
        }
    "#,
    )
    .expect_string("unknown");
}

// =========================================================================
// 13. Exhaustive Matching
// =========================================================================

#[test]
fn test_match_enum_exhaustive_all_covered() {
    ShapeTest::new(
        r#"
        enum Light { Red, Yellow, Green }
        fn action(l) {
            match l {
                Light::Red => "stop",
                Light::Yellow => "caution",
                Light::Green => "go"
            }
        }
        action(Light::Yellow)
    "#,
    )
    .expect_string("caution");
}

// =========================================================================
// 14. Guards with Function Calls and Arithmetic
// =========================================================================

#[test]
fn test_match_guard_with_function_call() {
    ShapeTest::new(
        r#"
        fn is_even(n) { n % 2 == 0 }
        fn classify(x) {
            match x {
                n where is_even(n) => "even",
                _ => "odd"
            }
        }
        classify(8)
    "#,
    )
    .expect_string("even");
}

#[test]
fn test_match_guard_with_arithmetic() {
    ShapeTest::new(
        r#"
        fn fizzbuzz(n) {
            match n {
                x where x % 15 == 0 => "fizzbuzz",
                x where x % 3 == 0 => "fizz",
                x where x % 5 == 0 => "buzz",
                x => "num"
            }
        }
        fizzbuzz(15)
    "#,
    )
    .expect_string("fizzbuzz");
}

// =========================================================================
// 15. Nested Conditionals and Destructuring in Match Arms
// =========================================================================

// BUG: Bare enum variant patterns (Some/None) require type-resolved enum context
#[test]
fn test_match_nested_if_in_arm() {
    ShapeTest::new(
        r#"
        let x = Some(10)
        match x {
            Some(v) => if v > 5 { "big" } else { "small" },
            None => "none"
        }
    "#,
    )
    .expect_string("big");
}

// BUG: Object destructuring {x, y} in match patterns fails to parse
#[test]
fn test_match_object_destructuring() {
    ShapeTest::new(
        r#"
        fn classify_point(point) {
            match point {
                {x, y} where x > y => "x wins",
                {x, y} where y > x => "y wins",
                _ => "tie"
            }
        }
        classify_point({x: 10, y: 5})
    "#,
    )
    .expect_string("x wins");
}
