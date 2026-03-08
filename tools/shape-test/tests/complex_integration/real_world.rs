//! Real-World Style Programs (15 tests)
//!
//! Tests simulating practical applications: score trackers, task managers,
//! tokenizers, config mergers, validators, interpreters, and more.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Real-World Style Programs (15 tests)
// =========================================================================

#[test]
fn test_program_score_tracker() {
    ShapeTest::new(
        r#"
        var scores = []
        fn add_score(score) { scores = scores.push(score) }
        fn get_average() {
            if scores.length == 0 { return 0 }
            scores.reduce(|acc, x| acc + x, 0) / scores.length
        }
        fn get_max() {
            scores.reduce(|max, x| if x > max { x } else { max }, scores[0])
        }
        add_score(85)
        add_score(92)
        add_score(78)
        add_score(95)
        add_score(88)
        print(get_average())
        print(get_max())
        print(scores.length)
    "#,
    )
    .expect_output_contains("87");
}

#[test]
fn test_program_task_list_manager() {
    ShapeTest::new(
        r#"
        var tasks = []
        fn add_task(name, done) {
            tasks = tasks.push(HashMap().set("name", name).set("done", done))
        }
        fn count_done() {
            var c = 0
            for t in tasks {
                if t.get("done") == true { c = c + 1 }
            }
            c
        }
        fn count_pending() {
            tasks.length - count_done()
        }
        add_task("Buy milk", true)
        add_task("Write code", false)
        add_task("Review PR", true)
        add_task("Deploy", false)
        print(tasks.length)
        print(count_done())
        print(count_pending())
    "#,
    )
    .expect_output("4\n2\n2");
}

#[test]
fn test_program_simple_tokenizer() {
    ShapeTest::new(
        r#"
        fn tokenize(input, delimiter) {
            input.split(delimiter)
        }
        let tokens = tokenize("let x = 42", " ")
        for t in tokens { print(t) }
    "#,
    )
    .expect_output("let\nx\n=\n42");
}

#[test]
fn test_program_config_merger() {
    ShapeTest::new(
        r#"
        let defaults = HashMap()
            .set("host", "localhost")
            .set("port", "8080")
            .set("debug", "false")
        let overrides = HashMap()
            .set("port", "3000")
            .set("debug", "true")
        // Merge: overrides win
        let merged = defaults
            .set("port", overrides.get("port"))
            .set("debug", overrides.get("debug"))
        print(merged.get("host"))
        print(merged.get("port"))
        print(merged.get("debug"))
    "#,
    )
    .expect_output("localhost\n3000\ntrue");
}

#[test]
fn test_program_event_emitter() {
    // Event emitter pattern using array of closures
    ShapeTest::new(
        r#"
        let handlers = [
            |d| print("Handler1: " + d),
            |d| print("Handler2: " + d)
        ]
        for h in handlers { h("click") }
        for h in handlers { h("submit") }
    "#,
    )
    .expect_output("Handler1: click\nHandler2: click\nHandler1: submit\nHandler2: submit");
}

#[test]
fn test_program_validator() {
    ShapeTest::new(
        r#"
        fn validate_name(name) {
            if name.length == 0 { return Err("name is empty") }
            if name.length > 50 { return Err("name too long") }
            Ok(name)
        }
        fn validate_age(age) {
            if age < 0 { return Err("age negative") }
            if age > 150 { return Err("age too large") }
            Ok(age)
        }
        fn validate(name, age) {
            match validate_name(name) {
                Err(e) => Err(e),
                Ok(n) => match validate_age(age) {
                    Err(e) => Err(e),
                    Ok(a) => Ok("valid")
                }
            }
        }
        print(match validate("Alice", 30) { Ok(v) => v, Err(e) => e })
        print(match validate("", 30) { Ok(v) => v, Err(e) => e })
        print(match validate("Bob", -1) { Ok(v) => v, Err(e) => e })
    "#,
    )
    .expect_output("valid\nname is empty\nage negative");
}

#[test]
fn test_program_number_formatter() {
    ShapeTest::new(
        r#"
        fn format_number(n) {
            if n < 1000 { return n + "" }
            if n < 1000000 { return (n / 1000) + "K" }
            return (n / 1000000) + "M"
        }
        print(format_number(42))
        print(format_number(1500))
        print(format_number(2500000))
    "#,
    )
    .expect_output("42\n1K\n2M");
}

#[test]
fn test_program_simple_calculator_repl() {
    ShapeTest::new(
        r#"
        enum Op { Add, Sub, Mul, Div }
        fn calc(a, op, b) {
            match op {
                Op::Add => Ok(a + b),
                Op::Sub => Ok(a - b),
                Op::Mul => Ok(a * b),
                Op::Div => if b == 0 { Err("divide by zero") } else { Ok(a / b) }
            }
        }
        let operations = [
            [10, Op::Add, 5],
            [20, Op::Sub, 8],
            [6, Op::Mul, 7],
            [15, Op::Div, 3],
            [10, Op::Div, 0]
        ]
        for op in operations {
            match calc(op[0], op[1], op[2]) {
                Ok(v) => print(v),
                Err(e) => print("Error: " + e)
            }
        }
    "#,
    )
    .expect_output("15\n12\n42\n5\nError: divide by zero");
}

#[test]
fn test_program_word_counter() {
    ShapeTest::new(
        r#"
        fn count_words(text) {
            let words = text.split(" ")
            var counts = HashMap()
            for word in words {
                let existing = counts.get(word)
                if existing == None {
                    counts = counts.set(word, 1)
                } else {
                    counts = counts.set(word, existing + 1)
                }
            }
            counts
        }
        let wc = count_words("the cat sat on the mat the cat")
        print(wc.get("the"))
        print(wc.get("cat"))
        print(wc.get("sat"))
    "#,
    )
    .expect_output("3\n2\n1");
}

#[test]
fn test_program_matrix_operations() {
    ShapeTest::new(
        r#"
        // 2x2 matrix as array of arrays
        fn mat_add(a, b) {
            [
                [a[0][0] + b[0][0], a[0][1] + b[0][1]],
                [a[1][0] + b[1][0], a[1][1] + b[1][1]]
            ]
        }
        fn mat_mul_scalar(m, s) {
            [
                [m[0][0] * s, m[0][1] * s],
                [m[1][0] * s, m[1][1] * s]
            ]
        }
        let a = [[1, 2], [3, 4]]
        let b = [[5, 6], [7, 8]]
        let sum = mat_add(a, b)
        print(sum[0][0])
        print(sum[0][1])
        print(sum[1][0])
        print(sum[1][1])
        let scaled = mat_mul_scalar(a, 3)
        print(scaled[0][0])
        print(scaled[1][1])
    "#,
    )
    .expect_output("6\n8\n10\n12\n3\n12");
}

#[test]
fn test_program_running_statistics() {
    // NOTE: 'data' is a reserved keyword in Shape, use 'values' instead
    ShapeTest::new(
        r#"
        var values = []
        fn add_value(v) { values = values.push(v) }
        fn avg() {
            values.reduce(|acc, x| acc + x, 0) / values.length
        }
        fn min_val() {
            values.reduce(|m, x| if x < m { x } else { m }, values[0])
        }
        fn max_val() {
            values.reduce(|m, x| if x > m { x } else { m }, values[0])
        }
        for v in [10, 20, 30, 40, 50] {
            add_value(v)
        }
        print(avg())
        print(min_val())
        print(max_val())
    "#,
    )
    .expect_output("30\n10\n50");
}

#[test]
fn test_program_string_builder() {
    ShapeTest::new(
        r#"
        var buffer = ""
        fn append(s) { buffer = buffer + s }
        fn append_line(s) { buffer = buffer + s + "\n" }
        fn build() { buffer }

        append_line("Hello")
        append("World")
        append("!")
        print(build())
    "#,
    )
    .expect_output("Hello\nWorld!");
}

#[test]
fn test_program_retry_logic() {
    ShapeTest::new(
        r#"
        var attempt = 0
        fn flaky_operation() {
            attempt = attempt + 1
            if attempt < 3 { return Err("failed") }
            Ok("success")
        }
        fn retry(max_retries) {
            var i = 0
            while i < max_retries {
                match flaky_operation() {
                    Ok(v) => { return Ok(v) },
                    Err(e) => { i = i + 1 }
                }
            }
            Err("max retries exceeded")
        }
        match retry(5) {
            Ok(v) => v,
            Err(e) => e
        }
    "#,
    )
    .expect_string("success");
}

#[test]
fn test_program_group_by() {
    ShapeTest::new(
        r#"
        fn group_by_parity(arr) {
            var evens = []
            var odds = []
            for x in arr {
                if x % 2 == 0 {
                    evens = evens.push(x)
                } else {
                    odds = odds.push(x)
                }
            }
            [evens, odds]
        }
        let groups = group_by_parity([1, 2, 3, 4, 5, 6, 7, 8])
        print(groups[0].length)
        print(groups[1].length)
        print(groups[0].reduce(|acc, x| acc + x, 0))
        print(groups[1].reduce(|acc, x| acc + x, 0))
    "#,
    )
    .expect_output("4\n4\n20\n16");
}

#[test]
fn test_program_simple_interpreter() {
    ShapeTest::new(
        r#"
        enum Instr { Push(int), Add, Mul, Print }
        fn run(program) {
            var stack = []
            for instr in program {
                match instr {
                    Instr::Push(n) => {
                        stack = stack.push(n)
                    },
                    Instr::Add => {
                        let b = stack[stack.length - 1]
                        stack = stack.slice(0, stack.length - 1)
                        let a = stack[stack.length - 1]
                        stack = stack.slice(0, stack.length - 1)
                        stack = stack.push(a + b)
                    },
                    Instr::Mul => {
                        let b = stack[stack.length - 1]
                        stack = stack.slice(0, stack.length - 1)
                        let a = stack[stack.length - 1]
                        stack = stack.slice(0, stack.length - 1)
                        stack = stack.push(a * b)
                    },
                    Instr::Print => {
                        print(stack[stack.length - 1])
                    }
                }
            }
        }
        // Compute (3 + 4) * 2 = 14
        run([Instr::Push(3), Instr::Push(4), Instr::Add, Instr::Push(2), Instr::Mul, Instr::Print])
    "#,
    )
    .expect_output("14");
}
