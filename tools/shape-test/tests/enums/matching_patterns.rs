use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. Pattern Matching on Literals (from programs_enums_matching.rs)
// =========================================================================

#[test]
fn match_on_int_literal() {
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
fn match_on_string_literal() {
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
fn match_on_bool_literal() {
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
fn match_wildcard_catches_unmatched() {
    ShapeTest::new(
        r#"
        let x = 999
        match x {
            1 => "one",
            2 => "two",
            _ => "other"
        }
    "#,
    )
    .expect_string("other");
}

#[test]
fn match_with_variable_binding() {
    ShapeTest::new(
        r#"
        let x = 42
        match x {
            n => n + 1
        }
    "#,
    )
    .expect_number(43.0);
}

#[test]
fn match_as_expression_returns_value() {
    ShapeTest::new(
        r#"
        let result = match 3 {
            1 => 10,
            2 => 20,
            3 => 30,
            _ => 0
        }
        result
    "#,
    )
    .expect_number(30.0);
}

// =========================================================================
// 2. Typed Patterns (from programs_enums_matching.rs)
// =========================================================================

#[test]
fn match_typed_pattern_int() {
    ShapeTest::new(
        r#"
        fn describe(val) {
            match val {
                n: int => "integer",
                s: string => "string",
                _ => "other"
            }
        }
        describe(42)
    "#,
    )
    .expect_string("integer");
}

#[test]
fn match_typed_pattern_string() {
    ShapeTest::new(
        r#"
        fn describe(val) {
            match val {
                n: int => "integer",
                s: string => "string",
                _ => "other"
            }
        }
        describe("hello")
    "#,
    )
    .expect_string("string");
}

// =========================================================================
// 3. Guards (from programs_enums_matching.rs)
// =========================================================================

#[test]
fn match_with_guard_clause() {
    ShapeTest::new(
        r#"
        let x = 15
        match x {
            n where n > 10 => "big",
            n where n > 0 => "small",
            _ => "zero or negative"
        }
    "#,
    )
    .expect_string("big");
}

#[test]
fn match_guard_second_arm() {
    ShapeTest::new(
        r#"
        let x = 5
        match x {
            n where n > 10 => "big",
            n where n > 0 => "small",
            _ => "zero or negative"
        }
    "#,
    )
    .expect_string("small");
}

#[test]
fn match_guard_fallthrough_to_wildcard() {
    ShapeTest::new(
        r#"
        let x = -1
        match x {
            n where n > 10 => "big",
            n where n > 0 => "small",
            _ => "zero or negative"
        }
    "#,
    )
    .expect_string("zero or negative");
}

// =========================================================================
// 4. Match in Functions (from programs_enums_matching.rs)
// =========================================================================

#[test]
fn match_nested_in_function() {
    ShapeTest::new(
        r#"
        fn classify(n) {
            match n {
                0 => "zero",
                1 => "one",
                _ => "many"
            }
        }
        classify(0)
    "#,
    )
    .expect_string("zero");
}

#[test]
fn match_nested_function_many() {
    ShapeTest::new(
        r#"
        fn classify(n) {
            match n {
                0 => "zero",
                1 => "one",
                _ => "many"
            }
        }
        classify(99)
    "#,
    )
    .expect_string("many");
}

#[test]
fn match_result_used_in_arithmetic() {
    ShapeTest::new(
        r#"
        let base = 10
        let bonus = match true {
            true => 5,
            false => 0
        }
        base + bonus
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn match_multiple_literal_arms_number() {
    ShapeTest::new(
        r#"
        let x = 4
        match x {
            1 => "a",
            2 => "b",
            3 => "c",
            4 => "d",
            5 => "e",
            _ => "other"
        }
    "#,
    )
    .expect_string("d");
}

#[test]
fn match_on_zero_literal() {
    ShapeTest::new(
        r#"
        let x = 0
        match x {
            0 => "zero",
            _ => "nonzero"
        }
    "#,
    )
    .expect_string("zero");
}

#[test]
fn match_with_expression_body() {
    ShapeTest::new(
        r#"
        let x = 3
        let y = match x {
            1 => 10 + 1,
            2 => 20 + 2,
            3 => 30 + 3,
            _ => 0
        }
        y
    "#,
    )
    .expect_number(33.0);
}

#[test]
fn match_on_enum_with_guard() {
    ShapeTest::new(
        r#"
        enum Value { Num(int) }
        let v = Value::Num(50)
        match v {
            Value::Num(n) where n > 100 => "big",
            Value::Num(n) where n > 10 => "medium",
            Value::Num(n) => "small"
        }
    "#,
    )
    .expect_string("medium");
}

#[test]
fn match_constructor_some_with_guard() {
    // Use direct scrutinee to avoid bare-pattern type resolution bug
    ShapeTest::new(
        r#"
        match Some(3) {
            Some(v) where v > 5 => "big",
            Some(v) => "small",
            _ => "nothing"
        }
    "#,
    )
    .expect_string("small");
}

#[test]
fn match_constructor_some_guard_matches() {
    // Use direct scrutinee to avoid bare-pattern type resolution bug
    ShapeTest::new(
        r#"
        match Some(10) {
            Some(v) where v > 5 => "big",
            Some(v) => "small",
            _ => "nothing"
        }
    "#,
    )
    .expect_string("big");
}

#[test]
fn match_chained_in_loop() {
    ShapeTest::new(
        r#"
        let sum = 0
        for i in [1, 2, 3, 4, 5] {
            sum = sum + match i {
                1 => 10,
                2 => 20,
                _ => 0
            }
        }
        sum
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn match_with_unit_arm() {
    ShapeTest::new(
        r#"
        let x = match 1 {
            1 => 42,
            _ => ()
        }
        x
    "#,
    )
    .expect_run_ok();
}

#[test]
fn match_nested_match_expressions() {
    ShapeTest::new(
        r#"
        let outer = 1
        let inner_val = 2
        match outer {
            1 => match inner_val {
                2 => "found",
                _ => "nope"
            },
            _ => "outer miss"
        }
    "#,
    )
    .expect_string("found");
}

#[test]
fn match_typed_pattern_computation() {
    ShapeTest::new(
        r#"
        fn double_if_int(val) {
            match val {
                n: int => n * 2,
                _ => 0
            }
        }
        double_if_int(21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn match_with_print_in_arm() {
    ShapeTest::new(
        r#"
        let x = 2
        match x {
            1 => print("one"),
            2 => print("two"),
            _ => print("other")
        }
    "#,
    )
    .expect_output("two");
}

// =========================================================================
// 5. Constructor Patterns (from programs_enums_matching.rs)
// =========================================================================

#[test]
fn constructor_some_extracts_payload() {
    ShapeTest::new(
        r#"
        match Some(99) {
            Some(v) => v,
            _ => 0
        }
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn constructor_none_matches_wildcard() {
    ShapeTest::new(
        r#"
        match None {
            Some(v) => "found",
            _ => "empty"
        }
    "#,
    )
    .expect_string("empty");
}

#[test]
fn constructor_ok_extracts_value() {
    ShapeTest::new(
        r#"
        match Ok(10) {
            Ok(v) => v,
            Err(e) => 0
        }
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn constructor_err_extracts_message() {
    ShapeTest::new(
        r#"
        match Err("bad") {
            Ok(v) => "ok",
            Err(e) => e
        }
    "#,
    )
    .expect_string("bad");
}

#[test]
fn constructor_qualified_enum_pattern() {
    ShapeTest::new(
        r#"
        enum Status { Active(int), Failed(string) }
        let s = Status::Active(200)
        match s {
            Status::Active(code) => code,
            Status::Failed(msg) => 0
        }
    "#,
    )
    .expect_number(200.0);
}

#[test]
fn constructor_qualified_error_variant() {
    ShapeTest::new(
        r#"
        enum Status { Active(int), Failed(string) }
        let s = Status::Failed("not found")
        match s {
            Status::Active(code) => "ok",
            Status::Failed(msg) => msg
        }
    "#,
    )
    .expect_string("not found");
}

#[test]
fn constructor_multiple_variants_in_match() {
    ShapeTest::new(
        r#"
        enum Calc { Num(int), Add(int, int), Neg(int) }
        let e = Calc::Add(3, 4)
        match e {
            Calc::Num(n) => n,
            Calc::Add(a, b) => a + b,
            Calc::Neg(n) => 0 - n
        }
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn constructor_neg_variant() {
    ShapeTest::new(
        r#"
        enum Calc { Num(int), Add(int, int), Neg(int) }
        let e = Calc::Neg(5)
        match e {
            Calc::Num(n) => n,
            Calc::Add(a, b) => a + b,
            Calc::Neg(n) => 0 - n
        }
    "#,
    )
    .expect_number(-5.0);
}

#[test]
fn constructor_pattern_with_wildcard_fallback() {
    ShapeTest::new(
        r#"
        let opt = Some(42)
        match opt {
            Some(v) => v,
            _ => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn constructor_none_with_wildcard() {
    ShapeTest::new(
        r#"
        let opt = None
        match opt {
            Some(v) => v,
            _ => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn constructor_ok_err_in_function() {
    ShapeTest::new(
        r#"
        fn handle(r) {
            match r {
                Ok(v) => v * 10,
                Err(e) => -1
            }
        }
        handle(Ok(5))
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn constructor_ok_err_function_err_path() {
    ShapeTest::new(
        r#"
        fn handle(r) {
            match r {
                Ok(v) => v * 10,
                Err(e) => -1
            }
        }
        handle(Err("oops"))
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn constructor_payload_used_in_string_concat() {
    ShapeTest::new(
        r#"
        match Some("world") {
            Some(v) => "hello " + v,
            _ => "nobody"
        }
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn constructor_qualified_three_variants() {
    ShapeTest::new(
        r#"
        enum Light { Red, Yellow, Green }
        fn action(light) {
            match light {
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

#[test]
fn constructor_mixed_unit_and_payload_match() {
    ShapeTest::new(
        r#"
        enum Event { Click, KeyPress(string), Resize(int, int) }
        let e = Event::Resize(800, 600)
        match e {
            Event::Click => "click",
            Event::KeyPress(key) => key,
            Event::Resize(w, h) => w + h
        }
    "#,
    )
    .expect_number(1400.0);
}

// =========================================================================
// 6. Edge Cases and Combinations
// =========================================================================

#[test]
fn match_first_arm_wins_over_later_match() {
    ShapeTest::new(
        r#"
        let x = 1
        match x {
            1 => "first",
            1 => "second",
            _ => "default"
        }
    "#,
    )
    .expect_string("first");
}

#[test]
fn match_result_and_option_in_same_program() {
    ShapeTest::new(
        r#"
        let a = match Some(10) {
            Some(v) => v,
            _ => 0
        }
        let b = match Ok(20) {
            Ok(v) => v,
            _ => 0
        }
        a + b
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn enum_variant_as_match_scrutinee_directly() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        match Color::Red {
            Color::Red => "red",
            Color::Green => "green",
            Color::Blue => "blue"
        }
    "#,
    )
    .expect_string("red");
}

#[test]
fn match_enum_in_loop_body() {
    ShapeTest::new(
        r#"
        enum Dir { Up, Down }
        let total = 0
        for d in [Dir::Up, Dir::Down, Dir::Up] {
            total = total + match d {
                Dir::Up => 1,
                Dir::Down => -1
            }
        }
        total
    "#,
    )
    .expect_number(1.0);
}
