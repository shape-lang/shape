//! Auto drop at block exit, drop for custom types, drop on function return.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Auto drop at block exit
// =========================================================================

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_called_at_scope_exit() {
    ShapeTest::new(
        r#"
        type Resource { name: string }
        impl Drop for Resource {
            method drop() {
                print(f"dropped {self.name}")
            }
        }
        let r = Resource { name: "test" }
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("dropped test");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_called_at_block_exit() {
    ShapeTest::new(
        r#"
        type Handle { id: int }
        impl Drop for Handle {
            method drop() {
                print(f"closed {self.id}")
            }
        }
        {
            let h = Handle { id: 1 }
        }
        print("after block")
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("closed 1");
}

#[test]
fn drop_type_without_drop_trait_is_fine() {
    ShapeTest::new(
        r#"
        type Plain { x: int }
        let p = Plain { x: 42 }
        p.x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn drop_impl_syntax_parses() {
    ShapeTest::new(
        r#"
        type Foo { val: int }
        impl Drop for Foo {
            method drop() {
                print("drop Foo")
            }
        }
    "#,
    )
    .expect_parse_ok();
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn drop_called_on_function_return() {
    ShapeTest::new(
        r#"
        type Logger { tag: string }
        impl Drop for Logger {
            method drop() {
                print(f"log-drop:{self.tag}")
            }
        }
        fn make_logger() {
            let l = Logger { tag: "inner" }
            42
        }
        let result = make_logger()
        print(f"result={result}")
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("log-drop:inner");
}

// TDD: Drop trait print output not captured by ShapeTest CaptureAdapter
#[test]
fn multiple_drops_in_function() {
    ShapeTest::new(
        r#"
        type Item { name: string }
        impl Drop for Item {
            method drop() {
                print(f"drop:{self.name}")
            }
        }
        fn work() {
            let a = Item { name: "alpha" }
            let b = Item { name: "beta" }
            0
        }
        work()
    "#,
    )
    .expect_run_ok()
    .expect_output_contains("drop:alpha");
}
