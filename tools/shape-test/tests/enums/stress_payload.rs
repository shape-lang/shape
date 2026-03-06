//! Stress tests for enums with payloads (tuple, struct, mixed) and payload extraction.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 9: Enum with Tuple Payload (7 tests)
// =============================================================================

/// Verifies string payload extraction via match.
#[test]
fn test_enum_tuple_payload_string() {
    ShapeTest::new(
        r#"
        enum Message { Text(string), Empty }
        let m = Message::Text("hello")
        match m {
            Message::Text(s) => s,
            Message::Empty => "none",
        }
    "#,
    )
    .expect_string("hello");
}

/// Verifies number payload extraction via match.
#[test]
fn test_enum_tuple_payload_number() {
    ShapeTest::new(
        r#"
        enum Wrapped { Val(number), Nothing }
        let w = Wrapped::Val(42)
        match w {
            Wrapped::Val(n) => n,
            Wrapped::Nothing => 0,
        }
    "#,
    )
    .expect_number(42.0);
}

/// Verifies int payload extraction via match.
#[test]
fn test_enum_tuple_payload_int() {
    ShapeTest::new(
        r#"
        enum BoxedInt { Just(int), Nothing }
        let b = BoxedInt::Just(99)
        match b {
            BoxedInt::Just(n) => n,
            BoxedInt::Nothing => 0,
        }
    "#,
    )
    .expect_number(99.0);
}

/// Verifies bool payload extraction via match.
#[test]
fn test_enum_tuple_payload_bool() {
    ShapeTest::new(
        r#"
        enum Flag { Set(bool), Unset }
        let f = Flag::Set(true)
        match f {
            Flag::Set(b) => b,
            Flag::Unset => false,
        }
    "#,
    )
    .expect_bool(true);
}

/// Verifies matching the empty variant when payload variant exists.
#[test]
fn test_enum_tuple_empty_variant_match() {
    ShapeTest::new(
        r#"
        enum Message { Text(string), Empty }
        let m = Message::Empty
        match m {
            Message::Text(s) => s,
            Message::Empty => "was_empty",
        }
    "#,
    )
    .expect_string("was_empty");
}

/// Verifies both variants having payloads — Err path.
#[test]
fn test_enum_both_variants_have_payloads() {
    ShapeTest::new(
        r#"
        enum Outcome { Ok(number), Err(string) }
        let r = Outcome::Err("bad")
        match r {
            Outcome::Ok(n) => "ok",
            Outcome::Err(s) => s,
        }
    "#,
    )
    .expect_string("bad");
}

/// Verifies both variants having payloads — Ok path.
#[test]
fn test_enum_ok_variant_payload() {
    ShapeTest::new(
        r#"
        enum Outcome { Ok(number), Err(string) }
        let r = Outcome::Ok(3.14)
        match r {
            Outcome::Ok(n) => n,
            Outcome::Err(s) => 0,
        }
    "#,
    )
    .expect_number(3.14);
}

// =============================================================================
// SECTION 10: Enum with Struct Payload (2 tests)
// =============================================================================

/// Verifies struct payload field extraction for Circle variant.
#[test]
fn test_enum_struct_payload() {
    ShapeTest::new(
        r#"
        enum Shape {
            Circle { radius: number },
            Rect { w: number, h: number }
        }
        let s = Shape::Circle { radius: 5.0 }
        match s {
            Shape::Circle { radius } => radius,
            Shape::Rect { w, h } => w * h,
        }
    "#,
    )
    .expect_number(5.0);
}

/// Verifies struct payload field extraction for Rect variant.
#[test]
fn test_enum_struct_payload_second_variant() {
    ShapeTest::new(
        r#"
        enum Shape {
            Circle { radius: number },
            Rect { w: number, h: number }
        }
        let s = Shape::Rect { w: 3.0, h: 4.0 }
        match s {
            Shape::Circle { radius } => radius,
            Shape::Rect { w, h } => w * h,
        }
    "#,
    )
    .expect_number(12.0);
}

// =============================================================================
// SECTION 19: Enum with Tuple Payload — Advanced (3 tests)
// =============================================================================

/// Verifies payload used in computation (negation).
#[test]
fn test_enum_payload_use_in_computation() {
    ShapeTest::new(
        r#"
        enum Expr { Literal(number), Negate(number) }
        let e = Expr::Negate(5)
        match e {
            Expr::Literal(n) => n,
            Expr::Negate(n) => 0 - n,
        }
    "#,
    )
    .expect_number(-5.0);
}

/// Verifies string payload used in concatenation — Hello variant.
#[test]
fn test_enum_payload_string_concat() {
    ShapeTest::new(
        r#"
        enum Greeting { Hello(string), Bye(string) }
        let g = Greeting::Hello("world")
        match g {
            Greeting::Hello(name) => "Hello, " + name,
            Greeting::Bye(name) => "Bye, " + name,
        }
    "#,
    )
    .expect_string("Hello, world");
}

/// Verifies string payload used in concatenation — Bye variant.
#[test]
fn test_enum_payload_bye_variant() {
    ShapeTest::new(
        r#"
        enum Greeting { Hello(string), Bye(string) }
        let g = Greeting::Bye("Alice")
        match g {
            Greeting::Hello(name) => "Hello, " + name,
            Greeting::Bye(name) => "Bye, " + name,
        }
    "#,
    )
    .expect_string("Bye, Alice");
}

// =============================================================================
// SECTION 20: Multiple Payload Fields (2 tests)
// =============================================================================

/// Verifies two-field tuple payload extraction.
#[test]
fn test_enum_two_field_tuple_payload() {
    ShapeTest::new(
        r#"
        enum Coord { Point2D(number, number), Origin }
        let c = Coord::Point2D(3.0, 4.0)
        match c {
            Coord::Point2D(x, y) => x + y,
            Coord::Origin => 0,
        }
    "#,
    )
    .expect_number(7.0);
}

/// Verifies origin variant matches correctly when payload variant exists.
#[test]
fn test_enum_origin_variant() {
    ShapeTest::new(
        r#"
        enum Coord { Point2D(number, number), Origin }
        let c = Coord::Origin
        match c {
            Coord::Point2D(x, y) => x + y,
            Coord::Origin => 0,
        }
    "#,
    )
    .expect_number(0.0);
}

// =============================================================================
// SECTION 25: Enum Match Arm with Block (2 tests)
// =============================================================================

/// Verifies match arm with a block body using payload.
#[test]
fn test_enum_match_arm_with_block() {
    ShapeTest::new(
        r#"
        enum Action { Move(int), Stay }
        let a = Action::Move(3)
        match a {
            Action::Move(n) => {
                let doubled = n * 2
                doubled + 1
            },
            Action::Stay => 0,
        }
    "#,
    )
    .expect_number(7.0);
}

/// Verifies match arm with multi-statement block using payload.
#[test]
fn test_enum_match_arm_with_multi_statement_block() {
    ShapeTest::new(
        r#"
        enum Cmd { Set(int), Reset }
        let c = Cmd::Set(10)
        match c {
            Cmd::Set(v) => {
                let a = v + 1
                let b = a * 2
                b
            },
            Cmd::Reset => 0,
        }
    "#,
    )
    .expect_number(22.0);
}

// =============================================================================
// SECTION 38: Mixed Unit and Payload Variants (3 tests)
// =============================================================================

/// Verifies mixed unit and tuple payload — Click variant.
#[test]
fn test_enum_mixed_unit_and_tuple_payload() {
    ShapeTest::new(
        r#"
        enum Event {
            Click(int, int),
            KeyPress(string),
            Idle
        }
        let e = Event::Click(10, 20)
        match e {
            Event::Click(x, y) => x + y,
            Event::KeyPress(k) => 0,
            Event::Idle => -1,
        }
    "#,
    )
    .expect_number(30.0);
}

/// Verifies mixed unit and tuple payload — KeyPress variant.
#[test]
fn test_enum_mixed_keypress_variant() {
    ShapeTest::new(
        r#"
        enum Event {
            Click(int, int),
            KeyPress(string),
            Idle
        }
        let e = Event::KeyPress("Enter")
        match e {
            Event::Click(x, y) => "click",
            Event::KeyPress(k) => k,
            Event::Idle => "idle",
        }
    "#,
    )
    .expect_string("Enter");
}

/// Verifies mixed unit and tuple payload — Idle variant.
#[test]
fn test_enum_mixed_idle_variant() {
    ShapeTest::new(
        r#"
        enum Event {
            Click(int, int),
            KeyPress(string),
            Idle
        }
        let e = Event::Idle
        match e {
            Event::Click(x, y) => "click",
            Event::KeyPress(k) => "key",
            Event::Idle => "idle",
        }
    "#,
    )
    .expect_string("idle");
}

// =============================================================================
// SECTION 44: Payload From Expression/Function (2 tests)
// =============================================================================

/// Verifies payload constructed from an expression.
#[test]
fn test_enum_payload_from_expression() {
    ShapeTest::new(
        r#"
        enum Box { Full(int), Empty }
        let x = 10
        let b = Box::Full(x * 2 + 3)
        match b {
            Box::Full(n) => n,
            Box::Empty => 0,
        }
    "#,
    )
    .expect_number(23.0);
}

/// Verifies payload constructed from a function call.
#[test]
fn test_enum_payload_from_function_call() {
    ShapeTest::new(
        r#"
        enum Box { Full(int), Empty }
        fn compute() -> int { 42 }
        fn test() -> int {
            let b = Box::Full(compute())
            match b {
                Box::Full(n) => n,
                Box::Empty => 0,
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =============================================================================
// SECTION 49: Complex Payload Extraction (2 tests)
// =============================================================================

/// Verifies string payload is preserved accurately.
#[test]
fn test_enum_string_payload_preserved() {
    ShapeTest::new(
        r#"
        enum Msg { Text(string), Num(int) }
        let m = Msg::Text("hello world")
        match m {
            Msg::Text(s) => s,
            Msg::Num(n) => "number",
        }
    "#,
    )
    .expect_string("hello world");
}

/// Verifies number payload is preserved accurately.
#[test]
fn test_enum_number_payload_preserved() {
    ShapeTest::new(
        r#"
        enum Msg { Text(string), Num(number) }
        let m = Msg::Num(3.14)
        match m {
            Msg::Text(s) => 0.0,
            Msg::Num(n) => n,
        }
    "#,
    )
    .expect_number(3.14);
}

// =============================================================================
// SECTION 51: Boolean Payload (2 tests)
// =============================================================================

/// Verifies bool payload true.
#[test]
fn test_enum_bool_payload_true() {
    ShapeTest::new(
        r#"
        enum Check { Result(bool), Pending }
        let c = Check::Result(true)
        match c {
            Check::Result(v) => v,
            Check::Pending => false,
        }
    "#,
    )
    .expect_bool(true);
}

/// Verifies bool payload false.
#[test]
fn test_enum_bool_payload_false() {
    ShapeTest::new(
        r#"
        enum Check { Result(bool), Pending }
        let c = Check::Result(false)
        match c {
            Check::Result(v) => v,
            Check::Pending => false,
        }
    "#,
    )
    .expect_bool(false);
}

// =============================================================================
// SECTION 57: Struct Payload — Field Access via Match (2 tests)
// =============================================================================

/// Verifies struct payload field sum for Point variant.
#[test]
fn test_enum_struct_payload_field_sum() {
    ShapeTest::new(
        r#"
        enum Geom {
            Point { x: number, y: number },
            Line { length: number }
        }
        let g = Geom::Point { x: 3.0, y: 4.0 }
        match g {
            Geom::Point { x, y } => x + y,
            Geom::Line { length } => length,
        }
    "#,
    )
    .expect_number(7.0);
}

/// Verifies struct payload field access for Line variant.
#[test]
fn test_enum_struct_payload_line_variant() {
    ShapeTest::new(
        r#"
        enum Geom {
            Point { x: number, y: number },
            Line { length: number }
        }
        let g = Geom::Line { length: 9.5 }
        match g {
            Geom::Point { x, y } => x + y,
            Geom::Line { length } => length,
        }
    "#,
    )
    .expect_number(9.5);
}

// =============================================================================
// SECTION 69: Construction Using Variable for Payload (1 test)
// =============================================================================

/// Verifies payload from a variable.
#[test]
fn test_enum_payload_from_variable() {
    ShapeTest::new(
        r#"
        enum Container { Item(string), Empty }
        let name = "test_item"
        let c = Container::Item(name)
        match c {
            Container::Item(s) => s,
            Container::Empty => "",
        }
    "#,
    )
    .expect_string("test_item");
}
