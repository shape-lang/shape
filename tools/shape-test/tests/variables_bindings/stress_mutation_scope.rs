//! Stress tests for mutable variables (var, let mut), reassignment patterns,
//! block scoping, control flow interaction, and function-scoped mutation.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 3. Mutable variables
// =========================================================================

/// Verifies var basic reassignment.
#[test]
fn test_var_basic_reassign() {
    ShapeTest::new("let mut x = 1\nx = 2\nx").expect_number(2.0);
}

/// Verifies let mut basic reassignment.
#[test]
fn test_let_mut_basic_reassign() {
    ShapeTest::new("fn test() -> int { let mut x = 1\nx = 2\nreturn x }\ntest()")
        .expect_number(2.0);
}

/// Verifies var multiple reassignments.
#[test]
fn test_var_multiple_reassign() {
    ShapeTest::new("let mut x = 0\nx = 1\nx = 2\nx = 3\nx").expect_number(3.0);
}

/// Verifies var reassign different value.
#[test]
fn test_var_reassign_different_value() {
    ShapeTest::new("fn test() -> int { let mut x = 10\nx = 20\nreturn x }\ntest()").expect_number(20.0);
}

/// Verifies self-increment pattern.
#[test]
fn test_var_self_increment() {
    ShapeTest::new("fn test() -> int { let mut x = 5\nx = x + 1\nreturn x }\ntest()")
        .expect_number(6.0);
}

/// Verifies self-decrement pattern.
#[test]
fn test_var_self_decrement() {
    ShapeTest::new("fn test() -> int { let mut x = 10\nx = x - 3\nreturn x }\ntest()")
        .expect_number(7.0);
}

/// Verifies self-multiply pattern.
#[test]
fn test_var_self_multiply() {
    ShapeTest::new("fn test() -> int { let mut x = 4\nx = x * 3\nreturn x }\ntest()")
        .expect_number(12.0);
}

/// Verifies let mut accumulate.
#[test]
fn test_let_mut_accumulate() {
    ShapeTest::new(
        "fn test() -> int {
            let mut sum = 0
            sum = sum + 1
            sum = sum + 2
            sum = sum + 3
            return sum
        }\ntest()",
    )
    .expect_number(6.0);
}

/// Verifies var string reassignment.
#[test]
fn test_var_string_reassign() {
    ShapeTest::new(
        "fn test() -> string {
            let mut s = \"hello\"
            s = \"world\"
            return s
        }\ntest()",
    )
    .expect_string("world");
}

/// Verifies var bool reassignment.
#[test]
fn test_var_bool_reassign() {
    ShapeTest::new(
        "fn test() -> bool {
            let mut flag = true
            flag = false
            return flag
        }\ntest()",
    )
    .expect_bool(false);
}

// =========================================================================
// 5. Block scoping
// =========================================================================

/// Verifies inner block scope access.
#[test]
fn test_block_inner_scope() {
    ShapeTest::new(
        "fn test() -> int {
            let x = 1
            let y = {
                let z = 10
                z + x
            }
            return y
        }\ntest()",
    )
    .expect_number(11.0);
}

/// Verifies block expression value.
#[test]
fn test_block_expression_value() {
    ShapeTest::new(
        "fn test() -> int {
            let x = {
                let a = 3
                let b = 4
                a + b
            }
            return x
        }\ntest()",
    )
    .expect_number(7.0);
}

/// Verifies block scope does not leak.
#[test]
fn test_block_scope_does_not_leak() {
    ShapeTest::new(
        "fn test() -> int {
            let x = 100
            {
                let x = 999
            }
            return x
        }\ntest()",
    )
    .expect_number(100.0);
}

/// Verifies nested blocks three levels deep.
#[test]
fn test_nested_blocks_three_levels() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 1
            let b = {
                let c = {
                    let d = 10
                    d + a
                }
                c + 5
            }
            return b
        }\ntest()",
    )
    .expect_number(16.0);
}

/// Verifies block shadows outer variable.
#[test]
fn test_block_shadow_outer() {
    ShapeTest::new(
        "fn test() -> int {
            let x = 1
            let y = {
                let x = 2
                x
            }
            return x + y
        }\ntest()",
    )
    .expect_number(3.0);
}

/// Verifies block reads outer variable.
#[test]
fn test_block_reads_outer_variable() {
    ShapeTest::new(
        "fn test() -> int {
            let outer = 42
            let val = {
                outer
            }
            return val
        }\ntest()",
    )
    .expect_number(42.0);
}

// =========================================================================
// 10. Scope nesting 3+ levels
// =========================================================================

/// Verifies three levels of scope nesting.
#[test]
fn test_scope_three_levels() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 1
            let b = {
                let c = {
                    let d = {
                        a + 10
                    }
                    d + 5
                }
                c + 2
            }
            return b
        }\ntest()",
    )
    .expect_number(18.0);
}

/// Verifies four levels of scope nesting.
#[test]
fn test_scope_four_levels() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 1
            let r = {
                let b = {
                    let c = {
                        let d = {
                            a + 100
                        }
                        d + 10
                    }
                    c + 1
                }
                b
            }
            return r
        }\ntest()",
    )
    .expect_number(112.0);
}

/// Verifies each level shadows independently.
#[test]
fn test_each_level_shadows_independently() {
    ShapeTest::new(
        "fn test() -> int {
            let x = 1
            let a = {
                let x = 10
                let b = {
                    let x = 100
                    x
                }
                b + x
            }
            return a + x
        }\ntest()",
    )
    .expect_number(111.0);
}

// =========================================================================
// 13. Reassignment patterns
// =========================================================================

/// Verifies var self-add pattern.
#[test]
fn test_var_self_add() {
    ShapeTest::new(
        "fn test() -> int {
            let mut x = 0
            x = x + 1
            x = x + 1
            x = x + 1
            return x
        }\ntest()",
    )
    .expect_number(3.0);
}

/// Verifies swap pattern.
#[test]
fn test_swap_pattern() {
    ShapeTest::new(
        "fn test() -> int {
            let mut a = 1
            let mut b = 2
            let mut tmp = a
            a = b
            b = tmp
            return a * 10 + b
        }\ntest()",
    )
    .expect_number(21.0);
}

/// Verifies accumulator pattern.
#[test]
fn test_accumulator_pattern() {
    ShapeTest::new(
        "fn test() -> int {
            let mut acc = 0
            acc = acc + 10
            acc = acc + 20
            acc = acc + 30
            return acc
        }\ntest()",
    )
    .expect_number(60.0);
}

/// Verifies counter with multiply.
#[test]
fn test_counter_with_multiply() {
    ShapeTest::new(
        "fn test() -> int {
            let mut x = 1
            x = x * 2
            x = x * 2
            x = x * 2
            return x
        }\ntest()",
    )
    .expect_number(8.0);
}

// =========================================================================
// 14. Variable in control flow
// =========================================================================

/// Verifies var in if true branch.
#[test]
fn test_var_in_if_true_branch() {
    ShapeTest::new(
        "fn test() -> int {
            let cond = true
            let x = if cond { 10 } else { 20 }
            return x
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies var in if false branch.
#[test]
fn test_var_in_if_false_branch() {
    ShapeTest::new(
        "fn test() -> int {
            let cond = false
            let x = if cond { 10 } else { 20 }
            return x
        }\ntest()",
    )
    .expect_number(20.0);
}

/// Verifies var defined before if.
#[test]
fn test_var_defined_before_if() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 5
            let b = if a > 3 { a * 2 } else { a + 2 }
            return b
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies mutable var modified in if.
#[test]
fn test_mutable_var_modified_in_if() {
    ShapeTest::new(
        "fn test() -> int {
            let mut x = 0
            if true {
                x = 42
            }
            return x
        }\ntest()",
    )
    .expect_number(42.0);
}

/// Verifies mutable var in both branches.
#[test]
fn test_mutable_var_both_branches() {
    ShapeTest::new(
        "fn test() -> int {
            let mut x = 0
            if false {
                x = 10
            } else {
                x = 20
            }
            return x
        }\ntest()",
    )
    .expect_number(20.0);
}

/// Verifies nested if with vars.
#[test]
fn test_nested_if_with_vars() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 5
            let b = 10
            let r = if a < b {
                if a > 3 {
                    a + b
                } else {
                    a - b
                }
            } else {
                0
            }
            return r
        }\ntest()",
    )
    .expect_number(15.0);
}

// =========================================================================
// Additional mutation patterns
// =========================================================================

/// Verifies var used in loop accumulator.
#[test]
fn test_var_used_in_loop_accumulator() {
    ShapeTest::new(
        "fn test() -> int {
            let mut sum = 0
            for i in 1..6 {
                sum = sum + i
            }
            return sum
        }\ntest()",
    )
    .expect_number(15.0);
}

/// Verifies var loop counter.
#[test]
fn test_var_loop_counter() {
    ShapeTest::new(
        "fn test() -> int {
            let mut count = 0
            for i in 0..10 {
                count = count + 1
            }
            return count
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies multiple vars in different blocks.
#[test]
fn test_multiple_vars_in_different_blocks() {
    ShapeTest::new(
        "fn test() -> int {
            let a = {
                let x = 10
                x
            }
            let b = {
                let x = 20
                x
            }
            return a + b
        }\ntest()",
    )
    .expect_number(30.0);
}

/// Verifies var int to negative.
#[test]
fn test_var_int_to_negative() {
    ShapeTest::new(
        "fn test() -> int {
            let mut x = 5
            x = x - 10
            return x
        }\ntest()",
    )
    .expect_number(-5.0);
}

/// Verifies var toggle bool.
#[test]
fn test_var_toggle_bool() {
    ShapeTest::new(
        "fn test() -> bool {
            let mut flag = true
            flag = !flag
            return flag
        }\ntest()",
    )
    .expect_bool(false);
}

/// Verifies var double toggle bool.
#[test]
fn test_var_double_toggle_bool() {
    ShapeTest::new(
        "fn test() -> bool {
            let mut flag = true
            flag = !flag
            flag = !flag
            return flag
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies mutable number var.
#[test]
fn test_mutable_number_var() {
    ShapeTest::new(
        "fn test() -> number {
            let mut x = 1.0
            x = x + 0.5
            x = x + 0.5
            return x
        }\ntest()",
    )
    .expect_number(2.0);
}

/// Verifies var countdown.
#[test]
fn test_var_countdown() {
    ShapeTest::new(
        "fn test() -> int {
            let mut n = 10
            let mut steps = 0
            while n > 0 {
                n = n - 1
                steps = steps + 1
            }
            return steps
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies var conditional assign.
#[test]
fn test_var_conditional_assign() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 5
            let mut result = 0
            if a > 3 {
                result = 1
            } else {
                result = 2
            }
            return result
        }\ntest()",
    )
    .expect_number(1.0);
}

/// Verifies var reassign with function call.
#[test]
fn test_var_reassign_with_function_call() {
    ShapeTest::new(
        "fn double(n: int) -> int { return n * 2 }
        fn test() -> int {
            let mut x = 3
            x = double(x)
            return x
        }\ntest()",
    )
    .expect_number(6.0);
}

/// Verifies var reassign repeatedly with fn.
#[test]
fn test_var_reassign_repeatedly_with_fn() {
    ShapeTest::new(
        "fn double(n: int) -> int { return n * 2 }
        fn test() -> int {
            let mut x = 1
            x = double(x)
            x = double(x)
            x = double(x)
            return x
        }\ntest()",
    )
    .expect_number(8.0);
}

// =========================================================================
// 24. Block as expression in various positions
// =========================================================================

/// Verifies block in function arg.
#[test]
fn test_block_in_function_arg() {
    ShapeTest::new(
        "fn identity(n: int) -> int { return n }
        fn test() -> int {
            return identity({
                let a = 5
                a * 2
            })
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies block in arithmetic.
#[test]
fn test_block_in_arithmetic() {
    ShapeTest::new(
        "fn test() -> int {
            let x = { 3 } + { 4 }
            return x
        }\ntest()",
    )
    .expect_number(7.0);
}

// =========================================================================
// 27. Variable interaction with boolean expressions
// =========================================================================

/// Verifies var as condition.
#[test]
fn test_var_as_condition() {
    ShapeTest::new(
        "fn test() -> int {
            let flag = true
            if flag { return 1 }
            return 0
        }\ntest()",
    )
    .expect_number(1.0);
}

/// Verifies var comparison as condition.
#[test]
fn test_var_comparison_as_condition() {
    ShapeTest::new(
        "fn test() -> int {
            let x = 10
            let y = 20
            if x < y { return 1 } else { return 0 }
        }\ntest()",
    )
    .expect_number(1.0);
}
