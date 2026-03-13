use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. Enum Declarations (from programs_enums_matching.rs)
// =========================================================================

#[test]
fn enum_unit_variants_declaration() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red
    "#,
    )
    .expect_run_ok();
}

#[test]
fn enum_unit_variant_equality_same() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red == Color::Red
    "#,
    )
    .expect_bool(true);
}

#[test]
fn enum_unit_variant_equality_different() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red == Color::Blue
    "#,
    )
    .expect_bool(false);
}

#[test]
fn enum_unit_variant_inequality() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red != Color::Green
    "#,
    )
    .expect_bool(true);
}

#[test]
fn enum_with_single_payload() {
    ShapeTest::new(
        r#"
        enum Shape { Circle(number), Square(number) }
        let s = Shape::Circle(5.0)
        match s {
            Shape::Circle(r) => r,
            Shape::Square(side) => side
        }
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn enum_with_multi_payload() {
    ShapeTest::new(
        r#"
        enum Shape { Rect(number, number) }
        let s = Shape::Rect(3, 4)
        match s {
            Shape::Rect(w, h) => w * h
        }
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn enum_variant_construction_and_print() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        print(Color::Green)
    "#,
    )
    .expect_output_contains("Green");
}

#[test]
fn enum_payload_variant_print() {
    ShapeTest::new(
        r#"
        enum Signal { Limit(int, int) }
        let s = Signal::Limit(100, 10)
        print(s)
    "#,
    )
    .expect_output_contains("Limit");
}

#[test]
fn enum_match_all_unit_variants() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        let c = Color::Green
        match c {
            Color::Red => "red",
            Color::Green => "green",
            Color::Blue => "blue"
        }
    "#,
    )
    .expect_string("green");
}

#[test]
fn enum_match_blue_variant() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        let c = Color::Blue
        match c {
            Color::Red => 1,
            Color::Green => 2,
            Color::Blue => 3
        }
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn enum_match_with_payload_extraction() {
    ShapeTest::new(
        r#"
        enum Message { Text(string), Number(int) }
        let m = Message::Text("hello")
        match m {
            Message::Text(s) => s,
            Message::Number(n) => "number"
        }
    "#,
    )
    .expect_string("hello");
}

#[test]
fn enum_match_second_variant_with_payload() {
    ShapeTest::new(
        r#"
        enum Message { Text(string), Number(int) }
        let m = Message::Number(42)
        match m {
            Message::Text(s) => 0,
            Message::Number(n) => n
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn enum_in_function_parameter() {
    ShapeTest::new(
        r#"
        enum Direction { Up, Down, Left, Right }
        fn describe(d) {
            match d {
                Direction::Up => "up",
                Direction::Down => "down",
                Direction::Left => "left",
                Direction::Right => "right"
            }
        }
        describe(Direction::Left)
    "#,
    )
    .expect_string("left");
}

#[test]
fn enum_as_function_return() {
    ShapeTest::new(
        r#"
        enum Direction { Up, Down }
        fn get_dir(flag) {
            if flag { Direction::Up } else { Direction::Down }
        }
        let d = get_dir(true)
        d == Direction::Up
    "#,
    )
    .expect_bool(true);
}

#[test]
fn enum_mixed_unit_and_payload_variants() {
    ShapeTest::new(
        r#"
        enum Token { Eof, Number(int), Ident(string) }
        let t = Token::Number(99)
        match t {
            Token::Eof => 0,
            Token::Number(n) => n,
            Token::Ident(s) => -1
        }
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn enum_mixed_eof_variant() {
    ShapeTest::new(
        r#"
        enum Token { Eof, Number(int), Ident(string) }
        let t = Token::Eof
        match t {
            Token::Eof => "eof",
            Token::Number(n) => "num",
            Token::Ident(s) => "id"
        }
    "#,
    )
    .expect_string("eof");
}

#[test]
fn enum_match_returns_computed_value() {
    ShapeTest::new(
        r#"
        enum Op { Add(int, int), Mul(int, int) }
        let op = Op::Add(3, 7)
        match op {
            Op::Add(a, b) => a + b,
            Op::Mul(a, b) => a * b
        }
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn enum_match_mul_variant() {
    ShapeTest::new(
        r#"
        enum Op { Add(int, int), Mul(int, int) }
        let op = Op::Mul(3, 7)
        match op {
            Op::Add(a, b) => a + b,
            Op::Mul(a, b) => a * b
        }
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn enum_many_variants() {
    ShapeTest::new(
        r#"
        enum Weekday { Mon, Tue, Wed, Thu, Fri, Sat, Sun }
        let d = Weekday::Fri
        match d {
            Weekday::Mon => 1,
            Weekday::Tue => 2,
            Weekday::Wed => 3,
            Weekday::Thu => 4,
            Weekday::Fri => 5,
            Weekday::Sat => 6,
            Weekday::Sun => 7
        }
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn enum_variant_in_let_binding() {
    ShapeTest::new(
        r#"
        enum Status { Active, Inactive }
        let s = Status::Active
        let is_active = s == Status::Active
        is_active
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// 2. Complex Programs with Enum Matching (from programs_enums_and_matching.rs)
// =========================================================================

#[test]
fn test_complex_enum_state_machine() {
    ShapeTest::new(
        r#"
        enum State { Idle, Running, Paused, Done }
        fn next_state(s) {
            match s {
                State::Idle => State::Running,
                State::Running => State::Paused,
                State::Paused => State::Running,
                State::Done => State::Done
            }
        }
        let mut s = State::Idle
        s = next_state(s)
        s = next_state(s)
        s = next_state(s)
        // Idle -> Running -> Paused -> Running
        print(s)
    "#,
    )
    .expect_output_contains("Running");
}

#[test]
fn test_complex_enum_command_pattern() {
    ShapeTest::new(
        r#"
        enum Cmd { Add(int), Sub(int), Reset }
        fn apply(state, cmd) {
            match cmd {
                Cmd::Add(n) => state + n,
                Cmd::Sub(n) => state - n,
                Cmd::Reset => 0
            }
        }
        let mut state = 0
        state = apply(state, Cmd::Add(10))
        state = apply(state, Cmd::Add(5))
        state = apply(state, Cmd::Sub(3))
        state
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_complex_enum_command_reset() {
    ShapeTest::new(
        r#"
        enum Cmd { Add(int), Sub(int), Reset }
        fn apply(state, cmd) {
            match cmd {
                Cmd::Add(n) => state + n,
                Cmd::Sub(n) => state - n,
                Cmd::Reset => 0
            }
        }
        let mut state = 100
        state = apply(state, Cmd::Add(50))
        state = apply(state, Cmd::Reset)
        state = apply(state, Cmd::Add(7))
        state
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn test_complex_enum_multi_step_matching() {
    ShapeTest::new(
        r#"
        enum Token { Num(int), Plus, Star }
        fn token_value(t) {
            match t {
                Token::Num(n) => n,
                Token::Plus => -1,
                Token::Star => -2
            }
        }
        let tokens = [Token::Num(3), Token::Plus, Token::Num(4), Token::Star, Token::Num(5)]
        let mut sum = 0
        for t in tokens {
            let v = token_value(t)
            if v >= 0 { sum = sum + v }
        }
        sum
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_complex_enum_nested_match() {
    ShapeTest::new(
        r#"
        enum Outer { A(int), B }
        fn process(x) {
            match x {
                Outer::A(n) => match n {
                    1 => "one",
                    2 => "two",
                    _ => "many"
                },
                Outer::B => "b"
            }
        }
        process(Outer::A(2))
    "#,
    )
    .expect_string("two");
}

#[test]
fn test_complex_match_with_printing() {
    ShapeTest::new(
        r#"
        enum Animal { Dog(string), Cat(string), Fish }
        fn greet(a) {
            match a {
                Animal::Dog(name) => print("Woof! I'm " + name),
                Animal::Cat(name) => print("Meow! I'm " + name),
                Animal::Fish => print("Blub!")
            }
        }
        greet(Animal::Dog("Rex"))
        greet(Animal::Cat("Whiskers"))
        greet(Animal::Fish)
    "#,
    )
    .expect_output("Woof! I'm Rex\nMeow! I'm Whiskers\nBlub!");
}

// =========================================================================
// 3. Complex Result Programs (from programs_enums_and_matching.rs)
// =========================================================================

#[test]
fn test_complex_result_pipeline() {
    ShapeTest::new(
        r#"
        fn parse(s) -> Result<int> {
            if s == "1" { Ok(1) }
            else if s == "2" { Ok(2) }
            else if s == "3" { Ok(3) }
            else { Err("parse error") }
        }
        fn double(n) -> Result<int> {
            Ok(n * 2)
        }
        fn pipeline(s) -> Result<int> {
            let n = parse(s)?
            let d = double(n)?
            Ok(d)
        }
        match pipeline("3") {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_complex_result_error_context_chain() {
    ShapeTest::new(
        r#"
        fn read_config() -> Result<string> {
            Err("file not found")
        }
        fn init() -> Result<string> {
            let cfg = (read_config() !! "config load failed")?
            Ok(cfg)
        }
        match init() {
            Ok(v) => "ok",
            Err(e) => "error"
        }
    "#,
    )
    .expect_string("error");
}

// =========================================================================
// 4. BUG Tests: Ok/Err Patterns on Untyped Variables
// =========================================================================

// Ok/Err patterns on untyped variables work correctly (unlike None)
#[test]
fn result_ok_err_patterns_work_on_untyped_ok_var() {
    ShapeTest::new(
        r#"
        let r = Ok(42)
        match r {
            Ok(v) => v,
            Err(e) => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn result_ok_err_patterns_work_on_untyped_err_var() {
    ShapeTest::new(
        r#"
        let r = Err("fail")
        match r {
            Ok(v) => "ok",
            Err(e) => e
        }
    "#,
    )
    .expect_string("fail");
}
