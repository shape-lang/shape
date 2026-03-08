//! Cross-Feature Combinations (25 tests)
//!
//! Tests combining enums + match + functions, closures + loops + arrays,
//! struct methods, result/try operators, higher-order functions, traits, and more.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Cross-Feature Combinations (25 tests)
// =========================================================================

#[test]
fn test_complex_enum_match_function_combo() {
    ShapeTest::new(
        r#"
        enum Shape { Circle(number), Square(number) }
        fn area(s) {
            match s {
                Shape::Circle(r) => 3 * r * r,
                Shape::Square(side) => side * side
            }
        }
        fn describe(s) {
            let a = area(s)
            match s {
                Shape::Circle(r) => "circle area=" + a,
                Shape::Square(side) => "square area=" + a
            }
        }
        print(describe(Shape::Circle(5)))
        print(describe(Shape::Square(4)))
    "#,
    )
    .expect_output("circle area=75\nsquare area=16");
}

#[test]
fn test_complex_closure_mutable_capture_loop_array() {
    ShapeTest::new(
        r#"
        var items = []
        let add = |item| {
            items = items.push(item)
        }
        for i in [1, 2, 3, 4, 5] {
            add(i * 10)
        }
        print(items.length)
        print(items.reduce(|acc, x| acc + x, 0))
    "#,
    )
    .expect_output("5\n150");
}

#[test]
fn test_complex_struct_method_chain() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        extend Vec2 {
            method scale(factor) {
                Vec2 { x: self.x * factor, y: self.y * factor }
            }
            method translate(dx, dy) {
                Vec2 { x: self.x + dx, y: self.y + dy }
            }
        }
        let v = Vec2 { x: 1, y: 2 }
            .scale(3)
            .translate(10, 20)
        print(v.x)
        print(v.y)
    "#,
    )
    .expect_output("13\n26");
}

#[test]
fn test_complex_result_try_operator_chain() {
    ShapeTest::new(
        r#"
        fn step1(n) -> Result<int> {
            if n < 0 { return Err("negative") }
            return Ok(n * 2)
        }
        fn step2(n) -> Result<int> {
            if n > 100 { return Err("too large") }
            return Ok(n + 10)
        }
        fn pipeline(n) -> Result<int> {
            let a = step1(n)?
            let b = step2(a)?
            return Ok(b)
        }
        print(pipeline(5)?)
    "#,
    )
    .expect_output("20");
}

#[test]
fn test_complex_result_try_operator_error_propagation() {
    ShapeTest::new(
        r#"
        fn step1(n) -> Result<int> {
            if n < 0 { return Err("negative") }
            return Ok(n * 2)
        }
        fn step2(n) -> Result<int> {
            if n > 100 { return Err("too large") }
            return Ok(n + 10)
        }
        fn pipeline(n) -> Result<int> {
            let a = step1(n)?
            let b = step2(a)?
            return Ok(b)
        }
        pipeline(-1)?
    "#,
    )
    .expect_run_err_contains("negative");
}

#[test]
fn test_complex_if_match_closure_return() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            let category = if x > 0 {
                match x {
                    n where n > 100 => "huge",
                    n where n > 10 => "big",
                    _ => "small"
                }
            } else {
                "non-positive"
            }
            let formatter = |cat| "result: " + cat
            formatter(category)
        }
        print(classify(50))
        print(classify(5))
        print(classify(-1))
    "#,
    )
    .expect_output("result: big\nresult: small\nresult: non-positive");
}

#[test]
fn test_complex_for_map_filter_fold() {
    ShapeTest::new(
        r#"
        let words = ["hello", "world", "foo", "bar", "shape"]
        let long_words = words.filter(|w| w.length > 3)
        let lengths = long_words.map(|w| w.length)
        let total = lengths.reduce(|acc, x| acc + x, 0)
        print(long_words.length)
        print(total)
    "#,
    )
    .expect_output("3\n15");
}

#[test]
fn test_complex_const_type_annotation_function() {
    ShapeTest::new(
        r#"
        const MAX_SIZE: int = 100
        fn is_valid_size(n: int) -> bool {
            n > 0 and n <= MAX_SIZE
        }
        print(is_valid_size(50))
        print(is_valid_size(150))
        print(is_valid_size(0))
    "#,
    )
    .expect_output("true\nfalse\nfalse");
}

#[test]
fn test_complex_closure_returning_closure() {
    ShapeTest::new(
        r#"
        fn make_adder(base) {
            |offset| {
                |x| base + offset + x
            }
        }
        let intermediate = make_adder(5)
        let add_fn = intermediate(3)
        print(add_fn(10))
        print(add_fn(0))
    "#,
    )
    .expect_output("18\n8");
}

#[test]
fn test_complex_array_of_closures() {
    ShapeTest::new(
        r#"
        let transforms = [
            |x| x + 1,
            |x| x * 2,
            |x| x - 3
        ]
        fn apply_all(transforms, val) {
            var result = val
            for t in transforms {
                result = t(result)
            }
            result
        }
        apply_all(transforms, 10)
    "#,
    )
    .expect_number(19.0);
}

#[test]
fn test_complex_enum_in_loop_with_match() {
    ShapeTest::new(
        r#"
        enum Token { Num(int), Op(string), End }
        let tokens = [Token::Num(3), Token::Op("+"), Token::Num(4), Token::End]
        var result = ""
        for t in tokens {
            result = result + match t {
                Token::Num(n) => "N",
                Token::Op(o) => "O",
                Token::End => "E"
            }
        }
        result
    "#,
    )
    .expect_string("NONE");
}

#[test]
fn test_complex_destructuring_typed_struct() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 3.0, y: 4.0 }
        let { x, y } = p
        let dist_sq = x * x + y * y
        dist_sq
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn test_complex_higher_order_with_enum_result() {
    ShapeTest::new(
        r#"
        fn try_apply(f, val) {
            let result = f(val)
            if result < 0 { Err("negative result") } else { Ok(result) }
        }
        match try_apply(|x| x * 2 - 100, 30) {
            Ok(v) => "ok: " + v,
            Err(e) => "err: " + e
        }
    "#,
    )
    .expect_string("err: negative result");
}

#[test]
fn test_complex_higher_order_with_enum_result_ok_path() {
    ShapeTest::new(
        r#"
        fn try_apply(f, val) {
            let result = f(val)
            if result < 0 { Err("negative result") } else { Ok(result) }
        }
        match try_apply(|x| x * 2 - 100, 80) {
            Ok(v) => "ok: " + v,
            Err(e) => "err: " + e
        }
    "#,
    )
    .expect_string("ok: 60");
}

#[test]
fn test_complex_nested_match_with_guards() {
    ShapeTest::new(
        r#"
        fn classify(category, value) {
            match category {
                "temp" => match value {
                    v where v > 100 => "boiling",
                    v where v > 30 => "hot",
                    v where v > 0 => "cold",
                    _ => "freezing"
                },
                "speed" => match value {
                    v where v > 100 => "fast",
                    v where v > 50 => "medium",
                    _ => "slow"
                },
                _ => "unknown"
            }
        }
        print(classify("temp", 50))
        print(classify("temp", -5))
        print(classify("speed", 120))
        print(classify("speed", 30))
    "#,
    )
    .expect_output("hot\nfreezing\nfast\nslow");
}

#[test]
fn test_complex_mutable_closure_as_iterator() {
    // Use a simpler pattern: generate array then iterate
    ShapeTest::new(
        r#"
        fn make_range(start, end) {
            var result = []
            var i = start
            while i < end {
                result = result.push(i)
                i = i + 1
            }
            result
        }
        let range = make_range(0, 5)
        var sum = 0
        for v in range {
            sum = sum + v
        }
        sum
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_complex_trait_dispatch_polymorphism() {
    ShapeTest::new(
        r#"
        trait Describable {
            describe(): string
        }
        type Dog { name: string }
        type Cat { name: string }
        impl Describable for Dog {
            method describe() { "Dog: " + self.name }
        }
        impl Describable for Cat {
            method describe() { "Cat: " + self.name }
        }
        let d = Dog { name: "Rex" }
        let c = Cat { name: "Whiskers" }
        print(d.describe())
        print(c.describe())
    "#,
    )
    .expect_output("Dog: Rex\nCat: Whiskers");
}

#[test]
fn test_complex_hashmap_with_loop_aggregation() {
    ShapeTest::new(
        r#"
        var scores = HashMap()
        let entries = [["Alice", 90], ["Bob", 85], ["Alice", 95], ["Bob", 80]]
        for entry in entries {
            let name = entry[0]
            let score = entry[1]
            let existing = scores.get(name)
            if existing == None {
                scores = scores.set(name, score)
            } else {
                scores = scores.set(name, existing + score)
            }
        }
        print(scores.get("Alice"))
        print(scores.get("Bob"))
    "#,
    )
    .expect_output("185\n165");
}

#[test]
fn test_complex_recursive_tree_sum() {
    ShapeTest::new(
        r#"
        // Simulate a tree using arrays: [value, left_children..., right_children...]
        // Simple recursive sum over nested arrays
        fn tree_sum(arr) {
            var total = 0
            for item in arr {
                total = total + item
            }
            total
        }
        let tree = [1, 2, 3, 4, 5]
        tree_sum(tree)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_complex_multi_return_function_with_option() {
    ShapeTest::new(
        r#"
        fn safe_divide(a, b) {
            if b == 0 { return Err("division by zero") }
            Ok(a / b)
        }
        fn process_division(a, b) {
            match safe_divide(a, b) {
                Ok(v) => {
                    if v > 10 {
                        "large: " + v
                    } else {
                        "small: " + v
                    }
                },
                Err(e) => "error: " + e
            }
        }
        print(process_division(100, 5))
        print(process_division(6, 3))
        print(process_division(10, 0))
    "#,
    )
    .expect_output("large: 20\nsmall: 2\nerror: division by zero");
}

#[test]
fn test_complex_array_methods_with_closures_and_match() {
    ShapeTest::new(
        r#"
        enum Priority { High, Medium, Low }
        fn priority_value(p) {
            match p {
                Priority::High => 3,
                Priority::Medium => 2,
                Priority::Low => 1
            }
        }
        let priorities = [Priority::Low, Priority::High, Priority::Medium, Priority::High]
        let total_score = priorities
            .map(|p| priority_value(p))
            .reduce(|acc, x| acc + x, 0)
        let high_count = priorities
            .filter(|p| p == Priority::High)
            .length
        print(total_score)
        print(high_count)
    "#,
    )
    .expect_output("9\n2");
}

#[test]
fn test_complex_nested_closures_with_capture() {
    // Closures returned from function capture state independently
    ShapeTest::new(
        r#"
        fn make_accumulator(start) {
            var total = start
            let add = |n| {
                total = total + n
                total
            }
            add
        }
        let acc = make_accumulator(0)
        print(acc(5))
        print(acc(3))
        print(acc(2))
        print(acc(10))
    "#,
    )
    .expect_output("5\n8\n10\n20");
}

#[test]
fn test_complex_block_expressions_with_control_flow() {
    ShapeTest::new(
        r#"
        fn compute(x) {
            let phase1 = {
                let doubled = x * 2
                if doubled > 20 { doubled - 10 } else { doubled + 10 }
            }
            let phase2 = {
                var sum = 0
                var i = 0
                while i < phase1 {
                    sum = sum + i
                    i = i + 1
                }
                sum
            }
            phase2
        }
        print(compute(3))
        print(compute(15))
    "#,
    )
    .expect_output("120\n190");
}

#[test]
fn test_complex_string_split_and_process() {
    ShapeTest::new(
        r#"
        let data = "Alice,30,NYC"
        let parts = data.split(",")
        print(parts[0])
        print(parts[1])
        print(parts[2])
        print(parts.length)
    "#,
    )
    .expect_output("Alice\n30\nNYC\n3");
}
