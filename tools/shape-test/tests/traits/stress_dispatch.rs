//! Stress tests: Basic trait method dispatch — calling trait methods on concrete types,
//! multiple methods, computed values, multiple traits on one type, same trait on
//! different types, and trait method return types.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 3. CALLING TRAIT METHODS
// =========================================================================

/// Call trait method on concrete type.
#[test]
fn call_trait_method_on_concrete_type() {
    ShapeTest::new(
        r#"
        type Dog { name: string }
        trait Animal { speak(self): string }
        impl Animal for Dog { method speak() { "Woof!" } }
        let d = Dog { name: "Rex" }
        d.speak()
    "#,
    )
    .expect_string("Woof!");
}

/// Call multiple trait methods on same instance.
#[test]
fn call_multiple_trait_methods_on_same_instance() {
    ShapeTest::new(
        r#"
        type Circle { radius: number }
        trait Shape {
            area(self): number,
            name(self): string
        }
        impl Shape for Circle {
            method area() { 3.14159 * self.radius * self.radius }
            method name() { "Circle" }
        }
        let c = Circle { radius: 2.0 }
        c.name()
    "#,
    )
    .expect_string("Circle");
}

/// Call trait method returns computed value.
#[test]
fn call_trait_method_returns_computed_value() {
    ShapeTest::new(
        r#"
        type Circle { radius: number }
        trait Shape { area(self): number }
        impl Shape for Circle {
            method area() { 3.14159 * self.radius * self.radius }
        }
        let c = Circle { radius: 10.0 }
        c.area()
    "#,
    )
    .expect_number(314.159);
}

// =========================================================================
// 5. MULTIPLE TRAITS ON ONE TYPE
// =========================================================================

/// Type implements two traits — first trait method.
#[test]
fn type_implements_two_traits() {
    ShapeTest::new(
        r#"
        type Item { name: string, price: number }
        trait Nameable { get_name(self): string }
        trait Priceable { get_price(self): number }
        impl Nameable for Item { method get_name() { self.name } }
        impl Priceable for Item { method get_price() { self.price } }
        let i = Item { name: "Widget", price: 9.99 }
        i.get_name()
    "#,
    )
    .expect_string("Widget");
}

/// Type implements two traits — second trait method.
#[test]
fn type_implements_two_traits_use_second() {
    ShapeTest::new(
        r#"
        type Item { name: string, price: number }
        trait Nameable { get_name(self): string }
        trait Priceable { get_price(self): number }
        impl Nameable for Item { method get_name() { self.name } }
        impl Priceable for Item { method get_price() { self.price } }
        let i = Item { name: "Widget", price: 9.99 }
        i.get_price()
    "#,
    )
    .expect_number(9.99);
}

/// Type implements three traits.
#[test]
fn type_implements_three_traits() {
    ShapeTest::new(
        r#"
        type Product { name: string, price: number, weight: number }
        trait Nameable { get_name(self): string }
        trait Priceable { get_price(self): number }
        trait Weighable { get_weight(self): number }
        impl Nameable for Product { method get_name() { self.name } }
        impl Priceable for Product { method get_price() { self.price } }
        impl Weighable for Product { method get_weight() { self.weight } }
        let p = Product { name: "Book", price: 12.99, weight: 0.5 }
        p.get_weight()
    "#,
    )
    .expect_number(0.5);
}

// =========================================================================
// 6. MULTIPLE TYPES IMPLEMENTING SAME TRAIT
// =========================================================================

/// Two types implement same trait — first type.
#[test]
fn two_types_implement_same_trait() {
    ShapeTest::new(
        r#"
        type Dog { name: string }
        type Cat { name: string }
        trait Animal { speak(self): string }
        impl Animal for Dog { method speak() { "Woof" } }
        impl Animal for Cat { method speak() { "Meow" } }
        let d = Dog { name: "Rex" }
        d.speak()
    "#,
    )
    .expect_string("Woof");
}

/// Two types implement same trait — second type.
#[test]
fn two_types_implement_same_trait_second_type() {
    ShapeTest::new(
        r#"
        type Dog { name: string }
        type Cat { name: string }
        trait Animal { speak(self): string }
        impl Animal for Dog { method speak() { "Woof" } }
        impl Animal for Cat { method speak() { "Meow" } }
        let c = Cat { name: "Whiskers" }
        c.speak()
    "#,
    )
    .expect_string("Meow");
}

/// Three types implement same trait.
#[test]
fn three_types_implement_same_trait() {
    ShapeTest::new(
        r#"
        type Circle { radius: number }
        type Square { side: number }
        type Triangle { base: number, height: number }
        trait Shape { area(self): number }
        impl Shape for Circle { method area() { 3.14159 * self.radius * self.radius } }
        impl Shape for Square { method area() { self.side * self.side } }
        impl Shape for Triangle { method area() { 0.5 * self.base * self.height } }
        let t = Triangle { base: 6.0, height: 4.0 }
        t.area()
    "#,
    )
    .expect_number(12.0);
}

// =========================================================================
// 18. DIFFERENT IMPL BEHAVIORS FOR SAME TRAIT
// =========================================================================

/// Same trait different implementations — square.
#[test]
fn same_trait_different_implementations() {
    ShapeTest::new(
        r#"
        type Square { side: number }
        type Circle { radius: number }
        trait Shape { area(self): number }
        impl Shape for Square { method area() { self.side * self.side } }
        impl Shape for Circle { method area() { 3.14159 * self.radius * self.radius } }
        let s = Square { side: 4.0 }
        s.area()
    "#,
    )
    .expect_number(16.0);
}

/// Same trait different implementations — circle.
#[test]
fn same_trait_different_implementations_circle() {
    ShapeTest::new(
        r#"
        type Square { side: number }
        type Circle { radius: number }
        trait Shape { area(self): number }
        impl Shape for Square { method area() { self.side * self.side } }
        impl Shape for Circle { method area() { 3.14159 * self.radius * self.radius } }
        let c = Circle { radius: 3.0 }
        c.area()
    "#,
    )
    .expect_number(28.27431);
}

// =========================================================================
// 23. TRAIT METHOD RETURNING DIFFERENT TYPES
// =========================================================================

/// Trait method returns string.
#[test]
fn trait_method_returns_string() {
    ShapeTest::new(
        r#"
        type Color { r: int, g: int, b: int }
        trait Stringifiable { to_hex(self): string }
        impl Stringifiable for Color { method to_hex() { "rgb" } }
        let c = Color { r: 255, g: 128, b: 0 }
        c.to_hex()
    "#,
    )
    .expect_string("rgb");
}

/// Trait method returns int.
#[test]
fn trait_method_returns_int_summable() {
    ShapeTest::new(
        r#"
        type Pair { a: int, b: int }
        trait Summable { total(self): int }
        impl Summable for Pair { method total() { self.a + self.b } }
        let p = Pair { a: 15, b: 25 }
        p.total()
    "#,
    )
    .expect_number(40.0);
}

/// Trait method returns bool — true case.
#[test]
fn trait_method_returns_bool_checkable() {
    ShapeTest::new(
        r#"
        type Range { min: number, max: number }
        trait Checkable { is_valid(self): bool }
        impl Checkable for Range { method is_valid() { self.min < self.max } }
        let r = Range { min: 0.0, max: 100.0 }
        r.is_valid()
    "#,
    )
    .expect_bool(true);
}

/// Trait method returns bool — false case.
#[test]
fn trait_method_returns_bool_false() {
    ShapeTest::new(
        r#"
        type Range { min: number, max: number }
        trait Checkable { is_valid(self): bool }
        impl Checkable for Range { method is_valid() { self.min < self.max } }
        let r = Range { min: 100.0, max: 0.0 }
        r.is_valid()
    "#,
    )
    .expect_bool(false);
}
