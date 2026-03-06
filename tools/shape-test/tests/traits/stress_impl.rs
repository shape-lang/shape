//! Stress tests for trait impl blocks: single method, field access, returns number/bool/int,
//! with params, for primitives, multiple methods, for custom types, with return type annotation,
//! using fn keyword, named impl, order independence, associated types, realistic domain modeling.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 2. IMPL BLOCKS -- BASIC
// =========================================================================

/// Verifies impl single method for type.
#[test]
fn impl_single_method_for_type() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        trait Printable {
            display(self): string
        }
        impl Printable for Point {
            method display() {
                "Point"
            }
        }
        let p = Point { x: 1.0, y: 2.0 }
        p.display()
    "#,
    )
    .expect_string("Point");
}

/// Verifies impl method accesses self fields.
#[test]
fn impl_method_accesses_self_fields() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        trait Describable {
            describe(self): string
        }
        impl Describable for Point {
            method describe() {
                "(" + self.x.to_string() + ", " + self.y.to_string() + ")"
            }
        }
        let p = Point { x: 3.0, y: 4.0 }
        p.describe()
    "#,
    )
    .expect_string("(3, 4)");
}

/// Verifies impl method returns number.
#[test]
fn impl_method_returns_number() {
    ShapeTest::new(
        r#"
        type Rectangle { w: number, h: number }
        trait Area {
            area(self): number
        }
        impl Area for Rectangle {
            method area() {
                self.w * self.h
            }
        }
        let r = Rectangle { w: 5.0, h: 3.0 }
        r.area()
    "#,
    )
    .expect_number(15.0);
}

/// Verifies impl method returns bool.
#[test]
fn impl_method_returns_bool() {
    ShapeTest::new(
        r#"
        type Box { weight: number }
        trait Heavy {
            is_heavy(self): bool
        }
        impl Heavy for Box {
            method is_heavy() {
                self.weight > 100.0
            }
        }
        let b = Box { weight: 150.0 }
        b.is_heavy()
    "#,
    )
    .expect_bool(true);
}

/// Verifies impl method with parameter.
#[test]
fn impl_method_with_parameter() {
    ShapeTest::new(
        r#"
        type Counter { value: int }
        trait Scalable {
            scale(self, factor: int): int
        }
        impl Scalable for Counter {
            method scale(factor: int) {
                self.value * factor
            }
        }
        let c = Counter { value: 5 }
        c.scale(3)
    "#,
    )
    .expect_number(15.0);
}

// =========================================================================
// 13. IMPL FOR PRIMITIVE TYPES
// =========================================================================

/// Verifies impl trait for number.
#[test]
fn impl_trait_for_number() {
    ShapeTest::new(
        r#"
        trait NumericLike {
            to_number(): number
        }
        impl NumericLike for number {
            method to_number() { self }
        }
        let x = 42.0
        x.to_number()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 14. IMPL BLOCK -- MULTIPLE METHODS
// =========================================================================

/// Verifies impl block with two methods.
#[test]
fn impl_block_with_two_methods() {
    ShapeTest::new(
        r#"
        type Rectangle { w: number, h: number }
        trait Shape {
            area(self): number,
            perimeter(self): number
        }
        impl Shape for Rectangle {
            method area() { self.w * self.h }
            method perimeter() { 2.0 * (self.w + self.h) }
        }
        let r = Rectangle { w: 3.0, h: 4.0 }
        r.perimeter()
    "#,
    )
    .expect_number(14.0);
}

/// Verifies impl block with three methods.
#[test]
fn impl_block_with_three_methods() {
    ShapeTest::new(
        r#"
        type Circle { radius: number }
        trait ShapeInfo {
            area(self): number,
            circumference(self): number,
            diameter(self): number
        }
        impl ShapeInfo for Circle {
            method area() { 3.14159 * self.radius * self.radius }
            method circumference() { 2.0 * 3.14159 * self.radius }
            method diameter() { 2.0 * self.radius }
        }
        let c = Circle { radius: 5.0 }
        c.diameter()
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// 20. TRAIT IMPL USING FN KEYWORD
// =========================================================================

/// Verifies impl using fn keyword.
#[test]
fn impl_using_fn_keyword() {
    ShapeTest::new(
        r#"
        type Pair { a: int, b: int }
        trait Summable { sum(self): int }
        impl Summable for Pair {
            fn sum() { self.a + self.b }
        }
        let p = Pair { a: 3, b: 7 }
        p.sum()
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// 21. IMPL FOR CUSTOM TYPE -- QUERYABLE PATTERN
// =========================================================================

/// Verifies queryable pattern compiles.
#[test]
fn queryable_pattern_compiles() {
    ShapeTest::new(
        r#"
        trait Queryable {
            filter(predicate): any,
            execute(): any
        }
        type MyQuery { data: int }
        impl Queryable for MyQuery {
            method filter(predicate) { self }
            method execute() { self.data }
        }
        let q = MyQuery { data: 42 }
        q.execute()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 27. IMPL WITH RETURN TYPE ANNOTATION
// =========================================================================

/// Verifies impl method with return type annotation.
#[test]
fn impl_method_with_return_type_annotation() {
    ShapeTest::new(
        r#"
        type Num { val: number }
        trait Doubler { double(self): number }
        impl Doubler for Num {
            method double() -> number {
                self.val * 2.0
            }
        }
        let n = Num { val: 21.0 }
        n.double()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 28. TRAIT METHOD ON TYPE WITH INT FIELDS
// =========================================================================

/// Verifies trait method on int fields.
#[test]
fn trait_method_on_int_fields() {
    ShapeTest::new(
        r#"
        type IntPair { a: int, b: int }
        trait Arithmetic {
            sum(self): int,
            product(self): int
        }
        impl Arithmetic for IntPair {
            method sum() { self.a + self.b }
            method product() { self.a * self.b }
        }
        let p = IntPair { a: 6, b: 7 }
        p.product()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 30. TRAIT ON TYPE WITH STRING FIELDS
// =========================================================================

/// Verifies trait method concatenates string fields.
#[test]
fn trait_method_concatenates_string_fields() {
    ShapeTest::new(
        r#"
        type Name { first: string, last: string }
        trait Fullname { full(self): string }
        impl Fullname for Name {
            method full() { self.first + " " + self.last }
        }
        let n = Name { first: "Jane", last: "Doe" }
        n.full()
    "#,
    )
    .expect_string("Jane Doe");
}

// =========================================================================
// 31. TRAIT WITH ONLY ONE METHOD (MINIMAL)
// =========================================================================

/// Verifies minimal trait one method.
#[test]
fn minimal_trait_one_method() {
    ShapeTest::new(
        r#"
        type Wrapped { inner: int }
        trait Unwrappable { unwrap(self): int }
        impl Unwrappable for Wrapped {
            method unwrap() { self.inner }
        }
        let w = Wrapped { inner: 999 }
        w.unwrap()
    "#,
    )
    .expect_number(999.0);
}

// =========================================================================
// 32. IMPL BLOCK ORDER INDEPENDENCE
// =========================================================================

/// Verifies trait defined after type.
#[test]
fn trait_defined_after_type() {
    ShapeTest::new(
        r#"
        type Config { debug: bool }
        trait Togglable { is_on(self): bool }
        impl Togglable for Config {
            method is_on() { self.debug }
        }
        let c = Config { debug: true }
        c.is_on()
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// 34. IMPL BLOCK -- REALISTIC DOMAIN MODELING
// =========================================================================

/// Verifies banking domain trait deposit.
#[test]
fn banking_domain_trait() {
    ShapeTest::new(
        r#"
        type Account { id: int, balance: number }
        trait Depositable {
            deposit(self, amount: number): any
        }
        impl Depositable for Account {
            method deposit(amount: number) {
                Account { id: self.id, balance: self.balance + amount }
            }
        }
        let acc = Account { id: 1, balance: 100.0 }
        let acc2 = acc.deposit(50.0)
        acc2.balance
    "#,
    )
    .expect_number(150.0);
}

/// Verifies banking domain withdrawal.
#[test]
fn banking_domain_withdrawal() {
    ShapeTest::new(
        r#"
        type Account { id: int, balance: number }
        trait Withdrawable {
            withdraw(self, amount: number): any
        }
        impl Withdrawable for Account {
            method withdraw(amount: number) {
                Account { id: self.id, balance: self.balance - amount }
            }
        }
        let acc = Account { id: 1, balance: 100.0 }
        let acc2 = acc.withdraw(30.0)
        acc2.balance
    "#,
    )
    .expect_number(70.0);
}

// =========================================================================
// 36. IMPL BLOCK -- ASSOCIATED TYPE BINDINGS
// =========================================================================

/// Verifies impl with associated type compiles.
#[test]
fn impl_with_associated_type_compiles() {
    ShapeTest::new(
        r#"
        trait Container {
            type Item;
            get(self): any
        }
        type IntBox { val: int }
        impl Container for IntBox {
            type Item = int;
            method get() { self.val }
        }
        let b = IntBox { val: 42 }
        b.get()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 43. TRAIT WITH GENERIC IMPL TYPE
// =========================================================================

/// Verifies impl generic trait for concrete type.
#[test]
fn impl_generic_trait_for_concrete_type() {
    ShapeTest::new(
        r#"
        trait Queryable<T> {
            filter(predicate): any,
            execute(): any
        }
        type IntList { data: int }
        impl Queryable<int> for IntList {
            method filter(predicate) { self }
            method execute() { self.data }
        }
        let q = IntList { data: 7 }
        q.execute()
    "#,
    )
    .expect_number(7.0);
}

// =========================================================================
// 45. MULTIPLE IMPLS -- DIFFERENT TRAIT SAME TYPE
// =========================================================================

/// Verifies four traits on one type.
#[test]
fn four_traits_on_one_type() {
    ShapeTest::new(
        r#"
        type Obj { val: int }
        trait A { a_val(self): int }
        trait B { b_val(self): int }
        trait C { c_val(self): int }
        trait D { d_val(self): int }
        impl A for Obj { method a_val() { self.val + 1 } }
        impl B for Obj { method b_val() { self.val + 2 } }
        impl C for Obj { method c_val() { self.val + 3 } }
        impl D for Obj { method d_val() { self.val + 4 } }
        let o = Obj { val: 10 }
        o.a_val() + o.b_val() + o.c_val() + o.d_val()
    "#,
    )
    .expect_number(50.0);
}

// =========================================================================
// 55. TRAIT ON TYPE WITH NO FIELDS
// =========================================================================

/// Verifies trait on fieldless type.
#[test]
fn trait_on_fieldless_type() {
    ShapeTest::new(
        r#"
        type Unit {}
        trait Identity { id(self): string }
        impl Identity for Unit {
            method id() { "unit" }
        }
        let u = Unit {}
        u.id()
    "#,
    )
    .expect_string("unit");
}

// =========================================================================
// 66. TRAIT IMPL -- TYPE WITH MANY FIELDS
// =========================================================================

/// Verifies trait on type with many fields.
#[test]
fn trait_on_type_with_many_fields() {
    ShapeTest::new(
        r#"
        type Record {
            a: int, b: int, c: int, d: int, e: int
        }
        trait Summable { total(self): int }
        impl Summable for Record {
            method total() { self.a + self.b + self.c + self.d + self.e }
        }
        let r = Record { a: 1, b: 2, c: 3, d: 4, e: 5 }
        r.total()
    "#,
    )
    .expect_number(15.0);
}

// =========================================================================
// 77. TRAIT — IMPL FOR TYPE WITH SINGLE FIELD
// =========================================================================

/// Trait on single field type returning a string.
#[test]
fn trait_on_single_field_type() {
    ShapeTest::new(
        r#"
        type Wrapper { inner: string }
        trait Unwrap { unwrap_str(self): string }
        impl Unwrap for Wrapper {
            method unwrap_str() { self.inner }
        }
        let w = Wrapper { inner: "wrapped_value" }
        w.unwrap_str()
    "#,
    )
    .expect_string("wrapped_value");
}

// =========================================================================
// 82. TRAIT — IMPL WITH STRING CONCATENATION
// =========================================================================

/// Trait method with multi-string concatenation.
#[test]
fn trait_method_multi_string_concat() {
    ShapeTest::new(
        r#"
        type Address { street: string, city: string, state: string }
        trait Formattable { format(self): string }
        impl Formattable for Address {
            method format() {
                self.street + ", " + self.city + ", " + self.state
            }
        }
        let a = Address { street: "123 Main", city: "Springfield", state: "IL" }
        a.format()
    "#,
    )
    .expect_string("123 Main, Springfield, IL");
}

// =========================================================================
// 88. TRAIT — STRING METHOD IN IMPL
// =========================================================================

/// Trait method uses to_string on an int field.
#[test]
fn trait_method_uses_to_string() {
    ShapeTest::new(
        r#"
        type Age { years: int }
        trait Describable { describe(self): string }
        impl Describable for Age {
            method describe() { self.years.to_string() + " years old" }
        }
        let a = Age { years: 25 }
        a.describe()
    "#,
    )
    .expect_string("25 years old");
}

// =========================================================================
// 91. TRAIT — IMPL WITH DECIMAL FIELD
// =========================================================================

/// Trait method decimal computation — tax calculation.
#[test]
fn trait_method_decimal_computation() {
    ShapeTest::new(
        r#"
        type Price { amount: number, tax_rate: number }
        trait Taxable { total(self): number }
        impl Taxable for Price {
            method total() { self.amount * (1.0 + self.tax_rate) }
        }
        let p = Price { amount: 100.0, tax_rate: 0.08 }
        p.total()
    "#,
    )
    .expect_number(108.0);
}

// =========================================================================
// 93. TRAIT — IMPL WITH VARIABLE SHADOWING
// =========================================================================

/// Trait method with variable shadowing.
#[test]
fn trait_method_variable_shadowing() {
    ShapeTest::new(
        r#"
        type Num { val: int }
        trait Transformer { transform(self): int }
        impl Transformer for Num {
            method transform() {
                let val = self.val * 2
                let val = val + 10
                val
            }
        }
        let n = Num { val: 5 }
        n.transform()
    "#,
    )
    .expect_number(20.0);
}

// =========================================================================
// 94. TRAIT — IMPL USES BUILTIN FUNCTIONS
// =========================================================================

/// Trait method calls builtin range function.
#[test]
fn trait_method_calls_builtin_range() {
    ShapeTest::new(
        r#"
        type Ranger { max: int }
        trait Rangeable { make_range(self): any }
        impl Rangeable for Ranger {
            method make_range() { range(0, self.max) }
        }
        let r = Ranger { max: 5 }
        let arr = r.make_range()
        arr.length()
    "#,
    )
    .expect_number(5.0);
}

// =========================================================================
// 96. TRAIT — METHOD ON TYPE WITH MIXED FIELD TYPES
// =========================================================================

/// Trait on mixed field types.
#[test]
fn trait_on_mixed_field_types() {
    ShapeTest::new(
        r#"
        type Entry { name: string, value: number, active: bool }
        trait Summarizable { summary(self): string }
        impl Summarizable for Entry {
            method summary() {
                self.name + "=" + self.value.to_string()
            }
        }
        let e = Entry { name: "score", value: 95.5, active: true }
        e.summary()
    "#,
    )
    .expect_string("score=95.5");
}

// =========================================================================
// 108. TRAIT — EDGE CASE: METHOD NAME MATCHES FIELD NAME
// =========================================================================

/// Trait method name same as field name — method takes priority.
#[test]
fn trait_method_name_same_as_field() {
    ShapeTest::new(
        r#"
        type Thing { value: int }
        trait HasValue { value(self): int }
        impl HasValue for Thing {
            method value() { self.value * 2 }
        }
        let t = Thing { value: 21 }
        t.value()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 106. TRAIT — TYPE WITH ONLY NUMBER FIELD
// =========================================================================

/// Verifies trait on number only type.
#[test]
fn trait_on_number_only_type() {
    ShapeTest::new(
        r#"
        type Weight { kg: number }
        trait Convertible { to_pounds(self): number }
        impl Convertible for Weight {
            method to_pounds() { self.kg * 2.20462 }
        }
        let w = Weight { kg: 100.0 }
        w.to_pounds()
    "#,
    )
    .expect_number(220.462);
}

// =========================================================================
// 107. TRAIT — IMPL WITH ARITHMETIC CHAIN
// =========================================================================

/// Verifies trait method arithmetic chain.
#[test]
fn trait_method_arithmetic_chain() {
    ShapeTest::new(
        r#"
        type Quad { a: int, b: int, c: int, d: int }
        trait Math { formula(self): int }
        impl Math for Quad {
            method formula() { (self.a + self.b) * (self.c - self.d) }
        }
        let q = Quad { a: 3, b: 7, c: 15, d: 5 }
        q.formula()
    "#,
    )
    .expect_number(100.0);
}
