//! Drop on early return, drop in loops (break/continue), drop with nested scopes.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Early return
// =========================================================================

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_on_early_return() {
    ShapeTest::new(
        r#"
        type Guard { name: string }
        impl Drop for Guard {
            method drop() {
                print(f"drop:{self.name}")
            }
        }
        fn early() {
            let g = Guard { name: "guard" }
            1
        }
        early()
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("drop:guard");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_multiple_on_early_return() {
    ShapeTest::new(
        r#"
        type G { id: int }
        impl Drop for G {
            method drop() {
                print(self.id)
            }
        }
        fn work() {
            let a = G { id: 10 }
            let b = G { id: 20 }
            0
        }
        work()
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("20");
}

// =========================================================================
// Loops
// =========================================================================

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_in_loop_body_each_iteration() {
    ShapeTest::new(
        r#"
        type Iter { round: int }
        impl Drop for Iter {
            method drop() {
                print(f"drop:{self.round}")
            }
        }
        for i in range(0, 3) {
            let it = Iter { round: i }
        }
        print("done")
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("drop:0");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_on_break() {
    ShapeTest::new(
        r#"
        type Brk { id: int }
        impl Drop for Brk {
            method drop() {
                print(f"drop:{self.id}")
            }
        }
        for i in range(0, 10) {
            let b = Brk { id: i }
            if i == 2 { break }
        }
        print("after-loop")
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("drop:2");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_on_continue() {
    ShapeTest::new(
        r#"
        type Cnt { id: int }
        impl Drop for Cnt {
            method drop() {
                print(f"drop:{self.id}")
            }
        }
        for i in range(0, 3) {
            let c = Cnt { id: i }
            if i == 1 { continue }
        }
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("drop:1");
}

// =========================================================================
// Nested scopes
// =========================================================================

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_nested_scopes() {
    ShapeTest::new(
        r#"
        type N { tag: string }
        impl Drop for N {
            method drop() {
                print(f"drop:{self.tag}")
            }
        }
        {
            let outer = N { tag: "outer" }
            {
                let inner = N { tag: "inner" }
            }
        }
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("drop:inner");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_deeply_nested_scopes() {
    ShapeTest::new(
        r#"
        type D { level: int }
        impl Drop for D {
            method drop() {
                print(self.level)
            }
        }
        {
            let l1 = D { level: 1 }
            {
                let l2 = D { level: 2 }
                {
                    let l3 = D { level: 3 }
                }
            }
        }
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("3");
}
