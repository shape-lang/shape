//! Basic extend blocks: extending types with methods, calling extended methods.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Extend Number
// =========================================================================

#[test]
fn extend_number_double() {
    ShapeTest::new(
        r#"
        extend Number {
            method double() { self * 2.0 }
        }
        let x = 21.0
        x.double()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn extend_number_is_positive() {
    ShapeTest::new(
        r#"
        extend Number {
            method is_positive() { self > 0 }
        }
        let x = 5.0
        x.is_positive()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn extend_number_is_positive_false() {
    ShapeTest::new(
        r#"
        extend Number {
            method is_positive() { self > 0 }
        }
        let x = -3.0
        x.is_positive()
    "#,
    )
    .expect_bool(false);
}

// =========================================================================
// Extend String
// =========================================================================

#[test]
fn extend_string_shout() {
    // BUG: extend blocks on built-in String type are not yet supported at runtime.
    // The method is registered but not dispatched for String values.
    ShapeTest::new(
        r#"
        extend String {
            method shout() { self + "!" }
        }
        let s = "hello"
        s.shout()
    "#,
    )
    .expect_run_err();
}

#[test]
fn extend_string_wrap() {
    // BUG: extend blocks on built-in String type are not yet supported at runtime.
    ShapeTest::new(
        r#"
        extend String {
            method wrap(prefix, suffix) { prefix + self + suffix }
        }
        let s = "world"
        s.wrap("[", "]")
    "#,
    )
    .expect_run_err();
}

// =========================================================================
// Extend user-defined type
// =========================================================================

#[test]
fn extend_custom_type_method() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        extend Vec2 {
            method magnitude_sq() {
                self.x * self.x + self.y * self.y
            }
        }
        let v = Vec2 { x: 3, y: 4 }
        v.magnitude_sq()
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn extend_custom_type_string_method() {
    ShapeTest::new(
        r#"
        type User { name: string }
        extend User {
            method greeting() {
                "Hello, " + self.name
            }
        }
        let u = User { name: "Alice" }
        u.greeting()
    "#,
    )
    .expect_string("Hello, Alice");
}

#[test]
fn extend_custom_type_bool_method() {
    ShapeTest::new(
        r#"
        type Account { balance: number }
        extend Account {
            method is_overdrawn() {
                self.balance < 0
            }
        }
        let a = Account { balance: -5 }
        a.is_overdrawn()
    "#,
    )
    .expect_bool(true);
}
