use shape_test::shape_test::ShapeTest;

// =============================================================================
// Drop/RAII — from programs_borrow_and_refs.rs (test_drop_*)
// =============================================================================

#[test]
fn test_drop_let_binding_at_scope_exit() {
    // Basic: let binding dropped at scope exit, verify via correct execution
    ShapeTest::new(
        r#"
        fn f() {
            let x = 42
            return x
        }
        f()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_drop_early_return_drops_locals() {
    ShapeTest::new(
        r#"
        fn f(cond) {
            let x = 10
            if cond {
                let y = 20
                return y
            }
            return x
        }
        f(true)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_drop_early_return_false_branch() {
    ShapeTest::new(
        r#"
        fn f(cond) {
            let x = 10
            if cond {
                let y = 20
                return y
            }
            return x
        }
        f(false)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_drop_break_drops_loop_locals() {
    ShapeTest::new(
        r#"
        fn f() {
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                let x = i * 2
                if x > 6 {
                    break
                }
                sum = sum + x
            }
            return sum
        }
        f()
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_drop_continue_drops_iteration_locals() {
    ShapeTest::new(
        r#"
        fn f() {
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                let x = i
                if i == 3 {
                    continue
                }
                sum = sum + x
            }
            return sum
        }
        f()
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_drop_nested_scope_reverse_order() {
    // Multiple nested scopes: execution correctness implies proper drops
    ShapeTest::new(
        r#"
        fn f() {
            let a = 1
            {
                let b = 2
                {
                    let c = 3
                }
            }
            return a
        }
        f()
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_drop_in_if_branch() {
    ShapeTest::new(
        r#"
        fn f(cond) {
            if cond {
                let a = 10
                return a
            } else {
                let b = 20
                return b
            }
        }
        f(true) + f(false)
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_drop_block_expression_drops_temporaries() {
    ShapeTest::new(
        r#"
        fn f() {
            let x = {
                let tmp = 21
                tmp * 2
            }
            return x
        }
        f()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_drop_multiple_functions_independent() {
    ShapeTest::new(
        r#"
        fn f() {
            let x = 10
            return x
        }
        fn g() {
            let y = 20
            return y
        }
        f() + g()
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_drop_deeply_nested_return() {
    ShapeTest::new(
        r#"
        fn f() {
            let a = 1
            {
                let b = 2
                {
                    let c = 3
                    {
                        let d = 4
                        return a + b + c + d
                    }
                }
            }
        }
        f()
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_drop_break_only_drops_loop_scope() {
    ShapeTest::new(
        r#"
        fn f() {
            let outer = 100
            for i in [1, 2, 3] {
                let inner = i
                if i == 2 { break }
            }
            return outer
        }
        f()
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn test_drop_while_loop_break() {
    ShapeTest::new(
        r#"
        fn f() {
            let mut i = 0
            let mut sum = 0
            while i < 10 {
                let val = i
                if i == 5 { break }
                sum = sum + val
                i = i + 1
            }
            return sum
        }
        f()
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_drop_while_loop_continue() {
    ShapeTest::new(
        r#"
        fn f() {
            let mut i = 0
            let mut sum = 0
            while i < 5 {
                i = i + 1
                let val = i
                if i == 3 { continue }
                sum = sum + val
            }
            return sum
        }
        f()
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_drop_for_empty_iterable() {
    ShapeTest::new(
        r#"
        fn f() {
            let mut sum = 0
            for i in [] {
                let x = i
                sum = sum + x
            }
            return sum
        }
        f()
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn test_drop_return_from_loop() {
    ShapeTest::new(
        r#"
        fn f() {
            let outer = 100
            for i in [1, 2, 3] {
                let inner = i
                if inner == 2 {
                    return outer + inner
                }
            }
            return outer
        }
        f()
    "#,
    )
    .expect_number(102.0);
}

#[test]
fn test_drop_custom_type_without_drop_impl() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        fn f() {
            let p = Point { x: 1, y: 2 }
            return p.x + p.y
        }
        f()
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_drop_custom_type_in_loop() {
    ShapeTest::new(
        r#"
        type Wrapper { value: number }
        fn f() {
            let mut sum = 0
            for i in [1, 2, 3] {
                let w = Wrapper { value: i * 10 }
                sum = sum + w.value
            }
            return sum
        }
        f()
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn test_drop_custom_type_returned_not_dropped() {
    ShapeTest::new(
        r#"
        type Res { value: number }
        fn make() {
            let r = Res { value: 42 }
            return r
        }
        fn f() {
            let r = make()
            return r.value
        }
        f()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_drop_many_variables_same_scope() {
    ShapeTest::new(
        r#"
        fn f() {
            let a = 1
            let b = 2
            let c = 3
            let d = 4
            let e = 5
            let g = 6
            let h = 7
            let i = 8
            let j = 9
            let k = 10
            return a + b + c + d + e + g + h + i + j + k
        }
        f()
    "#,
    )
    .expect_number(55.0);
}

#[test]
fn test_drop_variable_shadowing() {
    ShapeTest::new(
        r#"
        fn f() {
            let x = 10
            {
                let x = 20
            }
            return x
        }
        f()
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_drop_sequential_blocks() {
    ShapeTest::new(
        r#"
        fn f() {
            {
                let a = 1
            }
            {
                let b = 2
            }
            return 3
        }
        f()
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_drop_closure_captures_value() {
    ShapeTest::new(
        r#"
        fn f() {
            let x = 10
            let add_x = |a| a + x
            return add_x(5)
        }
        f()
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_drop_closure_returned_from_function() {
    ShapeTest::new(
        r#"
        fn make_adder(x) {
            let offset = x
            return |a| a + offset
        }
        fn f() {
            let add5 = make_adder(5)
            return add5(10)
        }
        f()
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_drop_recursive_function() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            let current = n
            if current <= 1 {
                return 1
            }
            return current * factorial(current - 1)
        }
        factorial(5)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn test_drop_iterative_factorial() {
    ShapeTest::new(
        r#"
        fn f() {
            let mut n = 5
            let mut result = 1
            while n > 0 {
                let current = n
                result = result * current
                n = n - 1
            }
            return result
        }
        f()
    "#,
    )
    .expect_number(120.0);
}
