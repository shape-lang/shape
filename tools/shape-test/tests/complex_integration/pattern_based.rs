//! Pattern-Based Programs (15 tests)
//!
//! Tests for state machines, command dispatch, visitor patterns, pipelines,
//! option/result chains, enum loops, dispatch tables, and fizzbuzz.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Pattern-Based Programs (15 tests)
// =========================================================================

#[test]
fn test_complex_state_machine_traffic_light() {
    ShapeTest::new(
        r#"
        enum Light { Red, Yellow, Green }
        fn next_light(current) {
            match current {
                Light::Red => Light::Green,
                Light::Green => Light::Yellow,
                Light::Yellow => Light::Red
            }
        }
        fn light_name(l) {
            match l {
                Light::Red => "Red",
                Light::Green => "Green",
                Light::Yellow => "Yellow"
            }
        }
        let mut light = Light::Red
        let mut i = 0
        while i < 6 {
            print(light_name(light))
            light = next_light(light)
            i = i + 1
        }
    "#,
    )
    .expect_output("Red\nGreen\nYellow\nRed\nGreen\nYellow");
}

#[test]
fn test_complex_command_dispatcher() {
    ShapeTest::new(
        r#"
        enum Command { Add(int, int), Mul(int, int), Neg(int) }
        fn execute(cmd) {
            match cmd {
                Command::Add(a, b) => a + b,
                Command::Mul(a, b) => a * b,
                Command::Neg(a) => 0 - a
            }
        }
        print(execute(Command::Add(3, 4)))
        print(execute(Command::Mul(5, 6)))
        print(execute(Command::Neg(10)))
    "#,
    )
    .expect_output("7\n30\n-10");
}

#[test]
fn test_complex_visitor_pattern_with_match() {
    ShapeTest::new(
        r#"
        enum Expr { Num(int), Add(int, int), Mul(int, int) }
        fn eval(e) {
            match e {
                Expr::Num(n) => n,
                Expr::Add(a, b) => a + b,
                Expr::Mul(a, b) => a * b
            }
        }
        fn to_string_repr(e) {
            match e {
                Expr::Num(n) => "num",
                Expr::Add(a, b) => "add",
                Expr::Mul(a, b) => "mul"
            }
        }
        let e = Expr::Add(3, 4)
        print(eval(e))
        print(to_string_repr(e))
    "#,
    )
    .expect_output("7\nadd");
}

#[test]
fn test_complex_pipeline_transform_filter_reduce() {
    ShapeTest::new(
        r#"
        let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        let result = data
            .map(|x| x * x)
            .filter(|x| x > 10)
            .reduce(|acc, x| acc + x, 0)
        result
    "#,
    )
    .expect_number(371.0);
}

#[test]
fn test_complex_pipeline_with_named_functions() {
    ShapeTest::new(
        r#"
        let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        let result = data.map(|x| x * 2).filter(|x| x > 10)
        print(result.length)
        print(result[0])
    "#,
    )
    .expect_output("5\n12");
}

#[test]
fn test_complex_option_chain_safe_operations() {
    ShapeTest::new(
        r#"
        fn safe_div(a, b) {
            if b == 0 { Err("div error") } else { Ok(a / b) }
        }
        fn safe_sqrt(x) {
            if x < 0 { Err("sqrt error") } else { Ok(x) }
        }

        fn compute(a, b) {
            match safe_div(a, b) {
                Ok(v) => match safe_sqrt(v) {
                    Ok(r) => "ok: " + r,
                    Err(e) => e
                },
                Err(e) => e
            }
        }
        print(compute(100, 4))
        print(compute(10, 0))
    "#,
    )
    .expect_output("ok: 25\ndiv error");
}

#[test]
fn test_complex_result_chain() {
    ShapeTest::new(
        r#"
        fn parse_positive(s) {
            if s == "5" { Ok(5) }
            else if s == "10" { Ok(10) }
            else { Err("parse error") }
        }
        fn validate(n) {
            if n > 0 { Ok(n) } else { Err("non-positive") }
        }
        fn process(s) {
            match parse_positive(s) {
                Ok(v) => match validate(v) {
                    Ok(n) => "valid: " + n,
                    Err(e) => "validation: " + e
                },
                Err(e) => "parse: " + e
            }
        }
        print(process("5"))
        print(process("bad"))
    "#,
    )
    .expect_output("valid: 5\nparse: parse error");
}

#[test]
fn test_complex_enum_with_loop_accumulation() {
    ShapeTest::new(r#"
        enum Action { Add(int), Sub(int), Reset }
        fn apply_actions(actions) {
            let mut total = 0
            for action in actions {
                total = match action {
                    Action::Add(n) => total + n,
                    Action::Sub(n) => total - n,
                    Action::Reset => 0
                }
            }
            total
        }
        apply_actions([Action::Add(10), Action::Add(20), Action::Sub(5), Action::Reset, Action::Add(42)])
    "#)
    .expect_number(42.0);
}

#[test]
fn test_complex_option_find_in_array() {
    ShapeTest::new(
        r#"
        fn find_first(arr, pred) {
            for item in arr {
                if pred(item) { return Ok(item) }
            }
            Err("not found")
        }
        let result = find_first([1, 3, 5, 8, 10], |x| x % 2 == 0)
        match result {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(8.0);
}

#[test]
fn test_complex_option_find_none() {
    ShapeTest::new(
        r#"
        fn find_first(arr, pred) {
            for item in arr {
                if pred(item) { return Ok(item) }
            }
            Err("not found")
        }
        let result = find_first([1, 3, 5, 7], |x| x > 100)
        match result {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_complex_dispatch_table() {
    ShapeTest::new(
        r#"
        let ops = HashMap()
            .set("add", |a, b| a + b)
            .set("mul", |a, b| a * b)
            .set("sub", |a, b| a - b)
        let add_fn = ops.get("add")
        let mul_fn = ops.get("mul")
        print(add_fn(3, 4))
        print(mul_fn(5, 6))
    "#,
    )
    .expect_output("7\n30");
}

#[test]
fn test_complex_fizzbuzz_match_guards() {
    ShapeTest::new(
        r#"
        fn fizzbuzz(n) {
            match n {
                x where x % 15 == 0 => "FizzBuzz",
                x where x % 3 == 0 => "Fizz",
                x where x % 5 == 0 => "Buzz",
                x => x
            }
        }
        print(fizzbuzz(3))
        print(fizzbuzz(5))
        print(fizzbuzz(15))
        print(fizzbuzz(7))
    "#,
    )
    .expect_output("Fizz\nBuzz\nFizzBuzz\n7");
}

#[test]
fn test_complex_enum_state_machine_with_payload() {
    ShapeTest::new(
        r#"
        enum State { Idle, Running(int), Done(string) }
        fn step(state) {
            match state {
                State::Idle => State::Running(0),
                State::Running(n) where n >= 3 => State::Done("finished"),
                State::Running(n) => State::Running(n + 1),
                State::Done(msg) => State::Done(msg)
            }
        }
        fn state_name(s) {
            match s {
                State::Idle => "idle",
                State::Running(n) => "running",
                State::Done(msg) => msg
            }
        }
        let mut s = State::Idle
        let mut i = 0
        while i < 6 {
            s = step(s)
            i = i + 1
        }
        state_name(s)
    "#,
    )
    .expect_string("finished");
}

#[test]
fn test_complex_recursive_flatten() {
    ShapeTest::new(
        r#"
        let data = [[1, 2], [3], [4, 5, 6]]
        let flat = data.flatMap(|x| x)
        print(flat.length)
        let sum = flat.reduce(|acc, x| acc + x, 0)
        print(sum)
    "#,
    )
    .expect_output("6\n21");
}

#[test]
fn test_complex_map_filter_chain_with_closures() {
    ShapeTest::new(
        r#"
        let threshold = 50
        let data = [10, 20, 30, 40, 50, 60, 70]
        let result = data
            .map(|x| x * 2)
            .filter(|x| x > threshold)
            .map(|x| x - threshold)
        for x in result { print(x) }
    "#,
    )
    .expect_output("10\n30\n50\n70\n90");
}
