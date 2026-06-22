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
        .expect_output("circle(r=5.0)\nrect(3.0x4.0)\npoint");
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
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("12.0\n0");
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
        let mut sum = 0
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
        let mut sum = 0
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
        type Point { x: int, y: int }
        fn classify_point(point: Point) {
            match point {
                {x, y} where x > y => "x wins",
                {x, y} where y > x => "y wins",
                _ => "tie"
            }
        }
        classify_point(Point {x: 10, y: 5})
    "#,
    )
    .expect_string("x wins");
}

// =========================================================================
// STAGE-Fix (v0.3.3 strict-flip): pattern-variant-ownership.
//
// A constructor/variant pattern must BELONG to the scrutinee enum type. A
// foreign variant (e.g. an `Option` `Some`/`None` pattern over a `Result`
// scrutinee, or a `Color` variant over a `Shape` scrutinee) previously
// collided by discriminant slot, binding the payload binder to RAW heap-
// pointer bits without a type check — a catastrophic reinterpret (VM != JIT,
// ASLR-nondeterministic). These assert the now-clean compile-error, and that
// valid same-enum matches still work.
// =========================================================================

#[test]
fn cross_enum_some_pattern_over_result_scrutinee_rejected() {
    // The catastrophic heap-reinterpret repro: Some/None (Option) over a
    // Result scrutinee. Must be a clean compile error, never a structural
    // discriminant-slot match.
    let code = r#"
let v: Result<int,string> = Ok(42)
match v { Some(n) => print(n + 1), None => print(-1) }
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("does not belong to scrutinee type 'Result'");
}

#[test]
fn cross_enum_some_over_result_err_value_rejected() {
    // Same rejection regardless of the runtime variant carried by the
    // Result value (Err here) — the check is at type-check time.
    let code = r#"
let v: Result<int,string> = Err("boom")
match v { Some(n) => print(n + 1), None => print(-1) }
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("does not belong to scrutinee type 'Result'");
}

#[test]
fn cross_enum_foreign_user_variant_over_user_enum_rejected() {
    let code = r#"
enum Shape { Circle(number), Square(number) }
enum Color { Red, Green }
let s: Shape = Shape::Circle(2.0)
match s {
  Color::Red => print(1),
  Shape::Square(side) => print(side),
  Shape::Circle(r) => print(r)
}
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("does not belong to enum 'Shape'");
}

#[test]
fn valid_option_match_still_works() {
    let code = r#"
let v: Option<int> = Some(5)
match v { Some(n) => print(n + 1), None => print(0) }
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("6");
}

#[test]
fn valid_result_match_still_works() {
    let code = r#"
let v: Result<int,string> = Ok(7)
match v { Ok(x) => print(x), Err(e) => print(-1) }
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("7");
}

#[test]
fn valid_user_enum_match_still_works() {
    let code = r#"
enum Shape { Circle(number), Square(number) }
let s: Shape = Shape::Circle(2.0)
match s { Shape::Circle(r) => print(r), Shape::Square(side) => print(side) }
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("2.0");
}

#[test]
fn nested_constructor_pattern_still_works() {
    // The G1 S3 nested-constructor fix must survive: Ok(Some(n)).
    let code = r#"
let v: Result<Option<int>, string> = Ok(Some(9))
match v { Ok(Some(n)) => print(n), Ok(None) => print(-1), Err(e) => print(-2) }
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("9");
}

#[test]
fn nested_foreign_inner_variant_rejected() {
    // A foreign variant in the INNER position (Color::Red where the inner
    // payload type is Option<int>) must reject too.
    let code = r#"
enum Color { Red, Green }
let v: Result<Option<int>, string> = Ok(Some(9))
match v {
  Ok(Color::Red) => print(0),
  Ok(Some(n)) => print(n),
  Ok(None) => print(-1),
  Err(e) => print(-2)
}
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("does not belong to scrutinee type 'Option'");
}

// =========================================================================
// R1 (v0.3.3 strict-flip): the pattern-ownership fix must NOT false-positive
// a valid `?`-on-Option chain. A function declared `-> Option<int>` that uses
// `?` must keep its `Option` return identity (not be re-wrapped into
// `Result<Option<int>>`), so a downstream `match h() { Some(v) => … }` sees an
// `Option` scrutinee and the variant-ownership check accepts the `Some`
// pattern. Before the fix `apply_fallibility_to_return_type` re-wrapped the
// already-Option return into Result and the match was spuriously rejected
// ("variant pattern 'Some' does not belong to scrutinee type 'Result'").
// =========================================================================

#[test]
fn option_try_chain_match_not_false_positive() {
    let code = r#"
fn g() -> Option<int> { Some(5) }
fn h() -> Option<int> { let x = g()?; Some(x) }
match h() { Some(v) => print(v), None => print(-1) }
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("5");
}

#[test]
fn result_try_chain_match_still_works() {
    // The sibling `?`-on-Result chain must remain valid after the R1 fix.
    let code = r#"
fn g() -> Result<int,string> { Ok(5) }
fn h() -> Result<int,string> { let x = g()?; Ok(x + 10) }
match h() { Ok(v) => print(v), Err(e) => print(-1) }
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("15");
}

// =========================================================================
// R2 (v0.3.3 strict-flip): nested-inner reinterpret hole. The ownership check
// must RECURSE into nested constructor patterns and reject a constructor
// pattern matched against a PROVABLY non-enum payload position. Previously the
// check returned Ok(()) for a non-enum scrutinee, so `Err(Some(n))` over a
// `Result<int,string>` bound the inner `n` to RAW heap-pointer bits of the
// `Err` string payload — a catastrophic reinterpret one level down.
// =========================================================================

#[test]
fn nested_constructor_over_string_payload_rejected() {
    // `Err(Some(n))`: `Some` against `Err`'s `string` payload (a non-enum)
    // must be a clean compile error, never a heap-reinterpret.
    let code = r#"
fn get() -> Result<int,string> { Err("hello") }
let v = get()
match v { Ok(n) => print(n + 1), Err(Some(n)) => print(n + 1000), Err(None) => print(-1) }
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("requires an enum-typed value");
}

#[test]
fn nested_constructor_over_int_payload_rejected() {
    // Three-level: `Ok(Some(Color::Red))` where `Ok`'s payload is `int` — the
    // inner `Some` against an `int` position must reject.
    let code = r#"
enum Color { Red, Blue }
fn get() -> Result<int,string> { Ok(5) }
let v = get()
match v { Ok(Some(c)) => print(1), Ok(_) => print(2), Err(_) => print(3) }
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("requires an enum-typed value");
}

#[test]
fn nested_constructor_over_enum_payload_valid() {
    // The legitimate counterpart: `Ok(Some(n))` where `Ok`'s payload is
    // `Option<int>` (an enum carrier) must still compile and run.
    let code = r#"
fn get() -> Result<Option<int>,string> { Ok(Some(9)) }
let v = get()
match v { Ok(Some(n)) => print(n), Ok(None) => print(-1), Err(e) => print(-2) }
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("9");
}

// =========================================================================
// FIX A (v0.3.3 strict-flip): constructor-over-REGISTERED-STRUCT reinterpret
// hole. `Ok(Some(n))` over `Result<Point,string>` recurses into `Ok`'s
// payload — a REGISTERED STRUCT `Point`. R2's positive non-enum classifier
// covered primitives + builtin collections but NOT a bare nominal struct, so
// `Some(n)` surfaced-and-stopped and `n` bound to RAW struct-pointer bits;
// `sink(n)` then did arithmetic on the raw heap pointer (VM != JIT,
// nondeterministic). FIX A looks the bare nominal up in the type registry: a
// registered struct is POSITIVELY provable as non-enum, so the constructor
// pattern is rejected.
// =========================================================================

#[test]
fn nested_constructor_over_registered_struct_payload_rejected() {
    // The CONFIRMED CATASTROPHIC repro. `Some(n)` against `Ok`'s `Point`
    // payload (a registered struct) must be a clean compile error.
    let code = r#"
type Point { x: int, y: int }
fn sink(v: int) -> int { v + 100 }
fn g() -> Result<Point,string> { Ok(Point { x: 42, y: 2 }) }
match g() { Ok(Some(n)) => print(sink(n)), Err(e) => print(e) }
"#;
    ShapeTest::new(code).expect_run_err_contains("requires an enum-typed value");
}

#[test]
fn nested_constructor_over_registered_struct_three_levels_rejected() {
    // Three-level nesting: `Ok(Ok(Some(n)))` where the innermost `Some` is
    // matched against a registered struct `Point` payload.
    let code = r#"
type Point { x: int, y: int }
fn g() -> Result<Result<Point,string>,string> { Ok(Ok(Point { x: 1, y: 2 })) }
match g() { Ok(Ok(Some(n))) => print(n), Err(e) => print(e) }
"#;
    ShapeTest::new(code).expect_run_err_contains("requires an enum-typed value");
}

// =========================================================================
// FIX B (v0.3.3 strict-flip, THE GENERAL ROOT): an `unknown`/un-inferable
// value must NOT launder through a typed function-call argument boundary into
// a PROVEN concrete parameter slot. This mirrors the keystone's no-any-sink
// rule for binary-op operands, extended to call arguments — it closes the
// launder boundary regardless of pattern nesting.
//
// CRITICAL no-FP: after the T1 keystone, legitimate dispatch results
// (`.map`/`.get`/match-arm binders) resolve to CONCRETE types, so a VALID
// program never passes `unknown` here — the keystone-dispatch tests below
// confirm those still compile + run.
// =========================================================================

#[test]
fn keystone_map_into_for_into_typed_fn_still_passes() {
    // `.map` result → `for` binder → typed `sink(int)`. The dispatch result is
    // a concrete `int` post-keystone, so FIX B does NOT false-positive.
    let code = r#"
fn sink(v: int) -> int { v + 100 }
let xs = [1, 2, 3]
let ys = xs.map(|x| x * 2)
for v in ys { print(sink(v)) }
"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn keystone_get_into_match_into_typed_fn_still_passes() {
    // HashMap `.get` → match-arm `Some(n)` binder → typed `sink(int)`. The
    // binder resolves to a concrete `int`, so FIX B does NOT false-positive.
    let code = r#"
fn sink(v: int) -> int { v + 1 }
fn run() -> int {
  let m: HashMap<string,int> = HashMap()
  m.set("a", 10)
  match m.get("a") { Some(n) => sink(n), None => 0 }
}
print(run())
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("11");
}

#[test]
fn keystone_filter_into_reduce_with_typed_fn_still_passes() {
    // `.filter` → `.reduce` with a typed `add(int,int)` callback. Concrete
    // throughout post-keystone.
    let code = r#"
fn add(a: int, b: int) -> int { a + b }
let xs = [1, 2, 3, 4]
let evens = xs.filter(|x| x % 2 == 0)
let s = evens.reduce(|acc, x| add(acc, x), 0)
print(s)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("6");
}

// =========================================================================
// STAGE-Fix (v0.3.3 strict-flip): RETURN/TAIL-POSITION reinterpret hole.
// The bidirectional `check_against` Match arm (an annotated fn's tail/return
// position) previously INFERRED the scrutinee but DISCARDED it and called the
// scrutinee-LESS `bind_pattern_vars`, so a foreign constructor pattern in the
// match arm reached `check_constructor_pattern_ownership(None, …)` — which
// surfaces-and-stops on a `None` scrutinee and accepts the foreign pattern.
// The payload binder then bound to a fresh unknown that flowed past FIX-B and
// was coerced to the declared return type at the boundary: a raw heap-pointer
// reinterpret (VM == JIT both leak, nondeterministic). The fix threads the
// substituted scrutinee into `bind_pattern_vars_typed`, mirroring the
// `infer_expr` Match path, so the ownership check gets the type it needs.
// =========================================================================

#[test]
fn return_position_constructor_over_struct_payload_rejected() {
    // The CONFIRMED return-position repro: the match is the TAIL expression of
    // an annotated `fn use_it() -> int`. `Some(n)` over `Ok`'s `Point` payload
    // must reject cleanly, not reinterpret the raw struct pointer.
    let code = r#"
type Point { x: int, y: int }
fn sink(v: int) -> int { v + 100 }
fn g() -> Result<Point,string> { Ok(Point { x: 7, y: 9 }) }
fn use_it() -> int { match g() { Ok(Some(n)) => sink(n), Err(e) => -1 } }
print(use_it())
"#;
    ShapeTest::new(code).expect_run_err_contains("requires an enum-typed value");
}

#[test]
fn return_position_constructor_over_int_payload_rejected() {
    // int-payload variant in return/tail position: `Some(n)` over `Ok`'s `int`
    // payload (a primitive) must reject.
    let code = r#"
fn g() -> Result<int,string> { Ok(5) }
fn use_it() -> int { match g() { Ok(Some(n)) => n + 1, Err(e) => -1 } }
print(use_it())
"#;
    ShapeTest::new(code).expect_run_err_contains("requires an enum-typed value");
}

#[test]
fn explicit_return_match_constructor_over_struct_payload_rejected() {
    // The `return <match>` syntactic form must reject identically to the
    // bare-tail form — both route through the bidirectional `check_against`
    // Match arm.
    let code = r#"
type Point { x: int, y: int }
fn sink(v: int) -> int { v + 100 }
fn g() -> Result<Point,string> { Ok(Point { x: 7, y: 9 }) }
fn use_it() -> int { return match g() { Ok(Some(n)) => sink(n), Err(e) => -1 } }
print(use_it())
"#;
    ShapeTest::new(code).expect_run_err_contains("requires an enum-typed value");
}

#[test]
fn return_position_legit_option_payload_still_passes() {
    // No-FP: `Result<Option<int>,string>` matched in RETURN position. `Ok`'s
    // payload IS an enum carrier (`Option<int>`), so `Ok(Some(n))` is valid and
    // the threaded scrutinee must NOT reject it. Returns 107 (7 + 100).
    let code = r#"
fn g() -> Result<Option<int>,string> { Ok(Some(7)) }
fn use_it() -> int { match g() { Ok(Some(n)) => n + 100, Ok(None) => -2, Err(e) => -1 } }
print(use_it())
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("107");
}

#[test]
fn return_position_valid_nested_ok_some_still_passes() {
    // No-FP: valid `Ok(Some(9))` in return position binds `n = 9` and returns 9.
    let code = r#"
fn g() -> Result<Option<int>,string> { Ok(Some(9)) }
fn use_it() -> int { match g() { Ok(Some(n)) => n, Ok(None) => -2, Err(e) => -1 } }
print(use_it())
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("9");
}
