//! Integration tests for operator overloading via traits.
//!
//! Tests compile and run Shape source code to verify:
//! - impl Add for custom types
//! - impl Sub for custom types
//! - impl Mul for custom types
//! - impl Div for custom types
//! - impl Neg for custom types
//! - Operator trait fallback only fires when built-in paths don't match

use crate::executor::tests::test_utils::{eval, eval_result};
use shape_value::ValueWord;

#[test]
fn test_add_trait_overload() {
    // Define a Vec2 type with impl Add
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Add for Vec2 {
            method add(other: Vec2) -> Vec2 {
                Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
        }

        let a = Vec2 { x: 1.0, y: 2.0 }
        let b = Vec2 { x: 3.0, y: 4.0 }
        let c = a + b
        c.x + c.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 10.0, "Vec2(1,2) + Vec2(3,4) = Vec2(4,6), x+y = 10");
}

#[test]
fn test_sub_trait_overload() {
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Sub for Vec2 {
            method sub(other: Vec2) -> Vec2 {
                Vec2 { x: self.x - other.x, y: self.y - other.y }
            }
        }

        let a = Vec2 { x: 5.0, y: 10.0 }
        let b = Vec2 { x: 1.0, y: 3.0 }
        let c = a - b
        c.x + c.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 11.0, "Vec2(5,10) - Vec2(1,3) = Vec2(4,7), x+y = 11");
}

#[test]
fn test_mul_trait_overload() {
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Mul for Vec2 {
            method mul(other: Vec2) -> Vec2 {
                Vec2 { x: self.x * other.x, y: self.y * other.y }
            }
        }

        let a = Vec2 { x: 2.0, y: 3.0 }
        let b = Vec2 { x: 4.0, y: 5.0 }
        let c = a * b
        c.x + c.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 23.0, "Vec2(2,3) * Vec2(4,5) = Vec2(8,15), x+y = 23");
}

#[test]
fn test_div_trait_overload() {
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Div for Vec2 {
            method div(other: Vec2) -> Vec2 {
                Vec2 { x: self.x / other.x, y: self.y / other.y }
            }
        }

        let a = Vec2 { x: 10.0, y: 20.0 }
        let b = Vec2 { x: 2.0, y: 5.0 }
        let c = a / b
        c.x + c.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 9.0, "Vec2(10,20) / Vec2(2,5) = Vec2(5,4), x+y = 9");
}

#[test]
fn test_neg_trait_overload() {
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Neg for Vec2 {
            method neg() -> Vec2 {
                Vec2 { x: -self.x, y: -self.y }
            }
        }

        let a = Vec2 { x: 3.0, y: -7.0 }
        let b = -a
        b.x + b.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 4.0, "-Vec2(3,-7) = Vec2(-3,7), x+y = 4");
}

#[test]
fn test_multiple_operator_traits() {
    // Test that a type can implement multiple operator traits
    let result = eval(
        r#"
        type Money { cents: int }

        impl Add for Money {
            method add(other: Money) -> Money {
                Money { cents: self.cents + other.cents }
            }
        }

        impl Sub for Money {
            method sub(other: Money) -> Money {
                Money { cents: self.cents - other.cents }
            }
        }

        let a = Money { cents: 500 }
        let b = Money { cents: 200 }
        let sum = a + b
        let diff = a - b
        sum.cents + diff.cents
    "#,
    );
    let val = result.as_i64().expect("should be an int");
    assert_eq!(
        val, 1000,
        "Money(500)+Money(200)=700, Money(500)-Money(200)=300, total=1000"
    );
}

#[test]
fn test_builtin_arithmetic_still_works() {
    // Make sure regular numeric arithmetic isn't affected
    let result = eval("2 + 3");
    assert_eq!(result.as_i64().unwrap(), 5);

    let result = eval("10.0 - 3.0");
    assert_eq!(result.as_number_coerce().unwrap(), 7.0);

    let result = eval("4 * 5");
    assert_eq!(result.as_i64().unwrap(), 20);

    let result = eval("20 / 4");
    assert_eq!(result.as_i64().unwrap(), 5);

    let result = eval("-42");
    assert_eq!(result.as_i64().unwrap(), -42);
}

#[test]
fn test_string_concat_still_works() {
    // String concatenation should not be affected by operator traits
    let result = eval(r#""hello " + "world""#);
    assert_eq!(result.as_str().unwrap(), "hello world");
}

#[test]
fn test_operator_overload_without_trait_fails() {
    // Without implementing Sub, - on custom types should fail at compile time
    let result = eval_result(
        r#"
        type Foo { x: int }
        let a = Foo { x: 1 }
        let b = Foo { x: 2 }
        a - b
    "#,
    );
    assert!(
        result.is_err(),
        "Subtracting two Foo without impl Sub should fail"
    );
}
