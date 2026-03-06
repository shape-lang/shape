//! Reverse declaration order drop (LIFO), multiple drops in same scope.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// LIFO ordering
// =========================================================================

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drops_in_reverse_declaration_order() {
    ShapeTest::new(
        r#"
        type R { name: string }
        impl Drop for R {
            method drop() {
                print(f"drop:{self.name}")
            }
        }
        {
            let a = R { name: "first" }
            let b = R { name: "second" }
            let c = R { name: "third" }
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("drop:third\ndrop:second\ndrop:first");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn two_drops_lifo() {
    ShapeTest::new(
        r#"
        type D { id: int }
        impl Drop for D {
            method drop() {
                print(self.id)
            }
        }
        {
            let x = D { id: 1 }
            let y = D { id: 2 }
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n1");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn mixed_drop_and_non_drop_types() {
    ShapeTest::new(
        r#"
        type Tracked { name: string }
        type Plain { val: int }
        impl Drop for Tracked {
            method drop() {
                print(f"drop:{self.name}")
            }
        }
        {
            let a = Tracked { name: "t1" }
            let b = Plain { val: 99 }
            let c = Tracked { name: "t2" }
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("drop:t2\ndrop:t1");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn five_drops_in_same_scope() {
    ShapeTest::new(
        r#"
        type X { n: int }
        impl Drop for X {
            method drop() {
                print(self.n)
            }
        }
        {
            let a = X { n: 1 }
            let b = X { n: 2 }
            let c = X { n: 3 }
            let d = X { n: 4 }
            let e = X { n: 5 }
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("5\n4\n3\n2\n1");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_order_across_sequential_blocks() {
    ShapeTest::new(
        r#"
        type S { tag: string }
        impl Drop for S {
            method drop() {
                print(f"drop:{self.tag}")
            }
        }
        {
            let a = S { tag: "block1" }
        }
        {
            let b = S { tag: "block2" }
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("drop:block1\ndrop:block2");
}
