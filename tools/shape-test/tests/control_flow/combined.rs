//! Combined / edge-case control flow tests.
//!
//! Tests that exercise multiple control flow features together:
//! - If inside for loop
//! - Match inside for loop
//! - Function with for loop and match
//! - While loop with match inside
//! - Block expression with early return in function

use shape_test::shape_test::ShapeTest;

#[test]
fn if_inside_for_loop() {
    ShapeTest::new(
        r#"
        var positives = 0
        var negatives = 0
        for x in [3, -1, 4, -1, 5, -9] {
            if x > 0 {
                positives = positives + 1
            } else {
                negatives = negatives + 1
            }
        }
        positives
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn match_inside_for_loop() {
    ShapeTest::new(
        r#"
        var ones = 0
        var twos = 0
        var others = 0
        for x in [1, 2, 1, 3, 2, 1] {
            match x {
                1 => { ones = ones + 1 },
                2 => { twos = twos + 1 },
                _ => { others = others + 1 }
            }
        }
        print(ones)
        print(twos)
        print(others)
    "#,
    )
    .expect_output("3\n2\n1");
}

#[test]
fn function_with_for_loop_and_match() {
    ShapeTest::new(
        r#"
        fn count_category(arr) {
            var small = 0
            var big = 0
            for x in arr {
                match x {
                    n where n <= 10 => { small = small + 1 },
                    _ => { big = big + 1 }
                }
            }
            small
        }
        count_category([5, 15, 3, 20, 8])
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn while_loop_with_match_inside() {
    ShapeTest::new(
        r#"
        var i = 0
        var result = ""
        while i < 5 {
            result = result + match i {
                0 => "a",
                1 => "b",
                2 => "c",
                _ => "x"
            }
            i = i + 1
        }
        result
    "#,
    )
    .expect_string("abcxx");
}

#[test]
fn block_expression_with_early_return_in_function() {
    ShapeTest::new(
        r#"
        fn test() {
            let x = {
                let val = 42
                if val > 100 { return "big" }
                val
            }
            return x
        }
        test()
    "#,
    )
    .expect_number(42.0);
}
