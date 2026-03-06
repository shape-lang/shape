//! Stress tests for operator trait overloading: Add, Sub, Mul, Div, Neg,
//! multiple operators on one type, chained operations, without impl fails,
//! composition with custom traits and Display, chain access on operator result.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 10. OPERATOR TRAIT OVERLOADING
// =========================================================================

/// Verifies impl Add for custom type.
#[test]
fn impl_add_for_custom_type() {
    ShapeTest::new(
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
    )
    .expect_number(10.0);
}

/// Verifies impl Sub for custom type.
#[test]
fn impl_sub_for_custom_type() {
    ShapeTest::new(
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
    )
    .expect_number(11.0);
}

/// Verifies impl Mul for custom type.
#[test]
fn impl_mul_for_custom_type() {
    ShapeTest::new(
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
    )
    .expect_number(23.0);
}

/// Verifies impl Div for custom type.
#[test]
fn impl_div_for_custom_type() {
    ShapeTest::new(
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
    )
    .expect_number(9.0);
}

/// Verifies impl Neg for custom type.
#[test]
fn impl_neg_for_custom_type() {
    ShapeTest::new(
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
    )
    .expect_number(4.0);
}

/// Verifies multiple operator traits on one type.
#[test]
fn multiple_operator_traits_on_one_type() {
    ShapeTest::new(
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
    )
    .expect_number(1000.0);
}

/// Verifies operator overload chained operations.
#[test]
fn operator_overload_chained_operations() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        impl Add for Vec2 {
            method add(other: Vec2) -> Vec2 {
                Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
        }
        let a = Vec2 { x: 1.0, y: 1.0 }
        let b = Vec2 { x: 2.0, y: 2.0 }
        let c = Vec2 { x: 3.0, y: 3.0 }
        let result = a + b + c
        result.x + result.y
    "#,
    )
    .expect_number(12.0);
}

// =========================================================================
// 11. OPERATOR OVERLOAD WITHOUT TRAIT -- SHOULD FAIL
// =========================================================================

/// Verifies operator overload without impl fails.
#[test]
fn operator_overload_without_impl_fails() {
    ShapeTest::new(
        r#"
        type Foo { x: int }
        let a = Foo { x: 1 }
        let b = Foo { x: 2 }
        a - b
    "#,
    )
    .expect_run_err();
}

// =========================================================================
// 24. TRAIT COMPOSITION PATTERNS
// =========================================================================

/// Verifies type with both custom trait and operator trait.
#[test]
fn type_with_both_custom_trait_and_operator_trait() {
    ShapeTest::new(
        r#"
        type Money { cents: int }
        trait Printable { label(self): string }
        impl Printable for Money {
            method label() { "Money" }
        }
        impl Add for Money {
            method add(other: Money) -> Money {
                Money { cents: self.cents + other.cents }
            }
        }
        let a = Money { cents: 100 }
        let b = Money { cents: 200 }
        let c = a + b
        c.cents
    "#,
    )
    .expect_number(300.0);
}

/// Verifies type with display and operator.
#[test]
fn type_with_display_and_operator() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        trait Display { display(self): string }
        impl Display for Vec2 {
            method display() {
                "Vec2(" + self.x.to_string() + ", " + self.y.to_string() + ")"
            }
        }
        impl Add for Vec2 {
            method add(other: Vec2) -> Vec2 {
                Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
        }
        let a = Vec2 { x: 1.0, y: 2.0 }
        let b = Vec2 { x: 3.0, y: 4.0 }
        let c = a + b
        c.to_string()
    "#,
    )
    .expect_string("Vec2(4, 6)");
}

// =========================================================================
// 39. OPERATOR TRAITS -- INT FIELDS
// =========================================================================

/// Verifies add trait with int fields.
#[test]
fn add_trait_with_int_fields() {
    ShapeTest::new(
        r#"
        type IntVec { x: int, y: int }
        impl Add for IntVec {
            method add(other: IntVec) -> IntVec {
                IntVec { x: self.x + other.x, y: self.y + other.y }
            }
        }
        let a = IntVec { x: 10, y: 20 }
        let b = IntVec { x: 30, y: 40 }
        let c = a + b
        c.x
    "#,
    )
    .expect_number(40.0);
}

/// Verifies add trait result field y.
#[test]
fn add_trait_result_field_y() {
    ShapeTest::new(
        r#"
        type IntVec { x: int, y: int }
        impl Add for IntVec {
            method add(other: IntVec) -> IntVec {
                IntVec { x: self.x + other.x, y: self.y + other.y }
            }
        }
        let a = IntVec { x: 10, y: 20 }
        let b = IntVec { x: 30, y: 40 }
        let c = a + b
        c.y
    "#,
    )
    .expect_number(60.0);
}

// =========================================================================
// 51. IMPL ALL FOUR OPERATOR TRAITS ON ONE TYPE
// =========================================================================

/// Verifies all four arithmetic operators.
#[test]
fn all_four_arithmetic_operators() {
    ShapeTest::new(
        r#"
        type Num { v: number }
        impl Add for Num {
            method add(other: Num) -> Num { Num { v: self.v + other.v } }
        }
        impl Sub for Num {
            method sub(other: Num) -> Num { Num { v: self.v - other.v } }
        }
        impl Mul for Num {
            method mul(other: Num) -> Num { Num { v: self.v * other.v } }
        }
        impl Div for Num {
            method div(other: Num) -> Num { Num { v: self.v / other.v } }
        }
        let a = Num { v: 10.0 }
        let b = Num { v: 3.0 }
        let sum = a + b
        let diff = a - b
        let prod = a * b
        let quot = a / b
        sum.v + diff.v + prod.v
    "#,
    )
    .expect_number(50.0);
}

// =========================================================================
// 61. ADD OPERATOR WITH CHAINED RESULT ACCESS
// =========================================================================

/// Verifies add operator chain access x.
#[test]
fn add_operator_chain_access() {
    ShapeTest::new(
        r#"
        type Pt { x: number, y: number }
        impl Add for Pt {
            method add(other: Pt) -> Pt {
                Pt { x: self.x + other.x, y: self.y + other.y }
            }
        }
        let a = Pt { x: 1.0, y: 2.0 }
        let b = Pt { x: 10.0, y: 20.0 }
        (a + b).x
    "#,
    )
    .expect_number(11.0);
}

/// Verifies add operator chain access y.
#[test]
fn add_operator_chain_access_y() {
    ShapeTest::new(
        r#"
        type Pt { x: number, y: number }
        impl Add for Pt {
            method add(other: Pt) -> Pt {
                Pt { x: self.x + other.x, y: self.y + other.y }
            }
        }
        let a = Pt { x: 1.0, y: 2.0 }
        let b = Pt { x: 10.0, y: 20.0 }
        (a + b).y
    "#,
    )
    .expect_number(22.0);
}

// =========================================================================
// 8. DISPLAY TRAIT -- BUILT-IN BEHAVIOR
// =========================================================================

/// Verifies display trait basic.
#[test]
fn display_trait_basic() {
    ShapeTest::new(
        r#"
        type User { name: string }
        trait Display { display(self): string }
        impl Display for User {
            method display() { "User:" + self.name }
        }
        let u = User { name: "Alice" }
        u.to_string()
    "#,
    )
    .expect_string("User:Alice");
}

/// Verifies display trait with multiple fields.
#[test]
fn display_trait_with_multiple_fields() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        trait Display { display(self): string }
        impl Display for Point {
            method display() {
                "(" + self.x.to_string() + ", " + self.y.to_string() + ")"
            }
        }
        let p = Point { x: 1.0, y: 2.0 }
        p.to_string()
    "#,
    )
    .expect_string("(1, 2)");
}

// =========================================================================
// 9. NAMED IMPL BLOCKS
// =========================================================================

/// Verifies named impl basic.
#[test]
fn named_impl_basic() {
    ShapeTest::new(
        r#"
        type User { name: string }
        trait Display { display(self): string }
        impl Display for User {
            method display() { "default:" + self.name }
        }
        impl Display for User as JsonDisplay {
            method display() { "json:" + self.name }
        }
        let u = User { name: "Alice" }
        u.to_string()
    "#,
    )
    .expect_string("default:Alice");
}

// =========================================================================
// 49. DISPLAY TRAIT -- TO_STRING
// =========================================================================

/// Verifies display trait to string numeric.
#[test]
fn display_trait_to_string_numeric() {
    ShapeTest::new(
        r#"
        type Counter { count: int }
        trait Display { display(self): string }
        impl Display for Counter {
            method display() { "Count: " + self.count.to_string() }
        }
        let c = Counter { count: 42 }
        c.to_string()
    "#,
    )
    .expect_string("Count: 42");
}

// =========================================================================
// 58. DISPLAY TRAIT -- FORMAT STRING
// =========================================================================

/// Verifies display trait with formatting.
#[test]
fn display_trait_with_formatting() {
    ShapeTest::new(
        r#"
        type Coord { lat: number, lon: number }
        trait Display { display(self): string }
        impl Display for Coord {
            method display() {
                "(" + self.lat.to_string() + ", " + self.lon.to_string() + ")"
            }
        }
        let c = Coord { lat: 40.7, lon: -74.0 }
        c.to_string()
    "#,
    )
    .expect_string("(40.7, -74)");
}

// =========================================================================
// 7. TRAIT BOUNDS ON GENERICS
// =========================================================================

/// Verifies generic function with trait bound.
#[test]
fn generic_function_with_trait_bound() {
    ShapeTest::new(
        r#"
        trait NumericLike {
            to_number(): number
        }
        impl NumericLike for number {
            method to_number() { self }
        }
        function first<T: NumericLike>(value: T) {
            value.to_number()
        }
        first(3.0)
    "#,
    )
    .expect_number(3.0);
}

/// Verifies trait bound with custom type.
#[test]
fn trait_bound_with_custom_type() {
    ShapeTest::new(
        r#"
        type Wrapper { val: number }
        trait HasValue {
            value(self): number
        }
        impl HasValue for Wrapper {
            method value() { self.val }
        }
        function extract<T: HasValue>(item: T) {
            item.value()
        }
        let w = Wrapper { val: 42.0 }
        extract(w)
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 73. OPERATOR RESULT IN FURTHER COMPUTATION
// =========================================================================

/// Verifies operator result in further computation.
#[test]
fn operator_result_in_further_computation() {
    ShapeTest::new(
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
        c.x * c.y
    "#,
    )
    .expect_number(24.0);
}

// =========================================================================
// 89. OPERATOR OVERLOAD PRESERVES ORIGINAL ARITHMETIC
// =========================================================================

/// Verifies regular arithmetic unaffected by operator traits.
#[test]
fn regular_arithmetic_unaffected_by_operator_traits() {
    ShapeTest::new(
        r#"
        type Custom { v: int }
        impl Add for Custom {
            method add(other: Custom) -> Custom {
                Custom { v: self.v + other.v }
            }
        }
        2 + 3
    "#,
    )
    .expect_number(5.0);
}

/// Verifies string concat unaffected by operator traits.
#[test]
fn string_concat_unaffected_by_operator_traits() {
    ShapeTest::new(
        r#"
        type Custom { v: int }
        impl Add for Custom {
            method add(other: Custom) -> Custom {
                Custom { v: self.v + other.v }
            }
        }
        "hello " + "world"
    "#,
    )
    .expect_string("hello world");
}

// =========================================================================
// 101. OPERATOR TRAIT + DISPLAY TRAIT TOGETHER
// =========================================================================

/// Verifies operator and display on same type.
#[test]
fn operator_and_display_on_same_type() {
    ShapeTest::new(
        r#"
        type Fraction { num: int, den: int }
        trait Display { display(self): string }
        impl Display for Fraction {
            method display() {
                self.num.to_string() + "/" + self.den.to_string()
            }
        }
        impl Add for Fraction {
            method add(other: Fraction) -> Fraction {
                Fraction {
                    num: self.num * other.den + other.num * self.den,
                    den: self.den * other.den
                }
            }
        }
        let a = Fraction { num: 1, den: 2 }
        let b = Fraction { num: 1, den: 3 }
        let c = a + b
        c.to_string()
    "#,
    )
    .expect_string("5/6");
}
