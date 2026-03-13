//! Advanced extend blocks: multiple methods, method with params, extend with logic.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Multiple methods in one extend block
// =========================================================================

#[test]
fn extend_multiple_methods_on_type() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        extend Point {
            method sum() { self.x + self.y }
            method diff() { self.x - self.y }
        }
        let p = Point { x: 10, y: 3 }
        p.sum() + p.diff()
    "#,
    )
    .expect_number(20.0);
}

// =========================================================================
// Extend method with parameter
// =========================================================================

#[test]
fn extend_method_with_parameter() {
    ShapeTest::new(
        r#"
        type Account { balance: number }
        extend Account {
            method deposit(amount) {
                self.balance + amount
            }
        }
        let a = Account { balance: 100 }
        a.deposit(50)
    "#,
    )
    .expect_number(150.0);
}

// =========================================================================
// Extend method with conditional
// =========================================================================

#[test]
fn extend_method_with_conditional() {
    ShapeTest::new(
        r#"
        type Value { n: number }
        extend Value {
            method classify() {
                if self.n > 0 { "positive" } else if self.n < 0 { "negative" } else { "zero" }
            }
        }
        let v = Value { n: 0 }
        v.classify()
    "#,
    )
    .expect_string("zero");
}

// =========================================================================
// Extend method with loop
// =========================================================================

#[test]
fn extend_method_with_loop() {
    ShapeTest::new(
        r#"
        type Range { start: int, end: int }
        extend Range {
            method sum() {
                let mut total = 0
                for i in self.start..self.end {
                    total = total + i
                }
                total
            }
        }
        let r = Range { start: 0, end: 5 }
        r.sum()
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// Extend method print output
// =========================================================================

#[test]
fn extend_method_print_output() {
    ShapeTest::new(
        r#"
        type Item { name: string, qty: int }
        extend Item {
            method label() {
                self.name + ":" + self.qty
            }
        }
        let i = Item { name: "apple", qty: 5 }
        print(i.label())
    "#,
    )
    .expect_output("apple:5");
}

// =========================================================================
// Extend with UFCS (Uniform Function Call Syntax)
// =========================================================================

#[test]
fn extend_method_called_like_function() {
    // TDD: UFCS allows calling extend methods as receiver.method()
    ShapeTest::new(
        r#"
        type Num { val: number }
        extend Num {
            method doubled() { self.val * 2.0 }
        }
        let n = Num { val: 21 }
        n.doubled()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Multiple extend blocks on same type
// =========================================================================

#[test]
fn two_extend_blocks_same_type() {
    // TDD: two separate extend blocks for the same type
    ShapeTest::new(
        r#"
        type Counter { count: int }
        extend Counter {
            method value() { self.count }
        }
        extend Counter {
            method is_zero() { self.count == 0 }
        }
        let c = Counter { count: 0 }
        print(c.value())
        print(c.is_zero())
    "#,
    )
    .expect_output("0\ntrue");
}

// =========================================================================
// Extend builtin array-like
// =========================================================================

#[test]
fn extend_array_with_custom_method() {
    // BUG: extending Array with a custom method is not supported at runtime.
    // The method is not dispatched for Array values.
    ShapeTest::new(
        r#"
        extend Array {
            method first_or_default(default_val) {
                if self.length > 0 { self[0] } else { default_val }
            }
        }
        let arr = [10, 20, 30]
        arr.first_or_default(0)
    "#,
    )
    .expect_run_err();
}
