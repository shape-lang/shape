//! Tests for variable scoping rules.
//!
//! Covers: block scoping, shadowing in nested blocks, function scope.

use shape_test::shape_test::ShapeTest;

#[test]
fn block_scope_inner_variable() {
    ShapeTest::new(
        r#"
        let x = 10
        {
            let y = 20
            print(x + y)
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("30");
}

#[test]
fn block_scope_shadowing() {
    ShapeTest::new(
        r#"
        let x = 10
        {
            let x = 99
            print(x)
        }
        print(x)
    "#,
    )
    .expect_run_ok()
    .expect_output("99\n10");
}

#[test]
fn nested_block_scoping() {
    ShapeTest::new(
        r#"
        let x = 1
        {
            let x = 2
            {
                let x = 3
                print(x)
            }
            print(x)
        }
        print(x)
    "#,
    )
    .expect_run_ok()
    .expect_output("3\n2\n1");
}

#[test]
fn function_scope_isolation() {
    ShapeTest::new(
        r#"
        let x = 100
        fn foo() {
            let x = 42
            x
        }
        print(foo())
        print(x)
    "#,
    )
    .expect_run_ok()
    .expect_output("42\n100");
}

#[test]
fn var_mutation_visible_in_same_scope() {
    ShapeTest::new(
        r#"
        let mut x = 0
        {
            x = 42
        }
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn loop_body_scope() {
    ShapeTest::new(
        r#"
        let mut total = 0
        for i in 0..3 {
            let temp = i * 10
            total = total + temp
        }
        total
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn if_block_scope() {
    ShapeTest::new(
        r#"
        let x = 5
        if x > 0 {
            let msg = "positive"
            print(msg)
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("positive");
}
