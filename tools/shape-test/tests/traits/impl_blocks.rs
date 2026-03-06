//! Trait impl blocks: implementing traits for concrete types, multiple impls.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic impl
// =========================================================================

#[test]
fn impl_trait_method_returns_string() {
    ShapeTest::new(
        r#"
        type User { name: string }
        trait Greetable {
            greet(self): string
        }
        impl Greetable for User {
            method greet() { "Hello, " + self.name }
        }
        let u = User { name: "Alice" }
        u.greet()
    "#,
    )
    .expect_string("Hello, Alice");
}

#[test]
fn impl_trait_method_returns_number() {
    ShapeTest::new(
        r#"
        type Rect { width: number, height: number }
        trait Area {
            area(self): number
        }
        impl Area for Rect {
            method area() { self.width * self.height }
        }
        let r = Rect { width: 4.0, height: 5.0 }
        r.area()
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn impl_trait_method_returns_bool() {
    ShapeTest::new(
        r#"
        type Wallet { balance: number }
        trait Checkable {
            is_empty(self): bool
        }
        impl Checkable for Wallet {
            method is_empty() { self.balance == 0 }
        }
        let w = Wallet { balance: 0 }
        w.is_empty()
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// Multiple impls for different types
// =========================================================================

#[test]
fn impl_same_trait_for_two_types() {
    ShapeTest::new(
        r#"
        trait Describable {
            describe(self): string
        }
        type Cat { name: string }
        type Dog { name: string }
        impl Describable for Cat {
            method describe() { "cat:" + self.name }
        }
        impl Describable for Dog {
            method describe() { "dog:" + self.name }
        }
        let c = Cat { name: "Whiskers" }
        let d = Dog { name: "Rex" }
        print(c.describe())
        print(d.describe())
    "#,
    )
    .expect_output("cat:Whiskers\ndog:Rex");
}

// =========================================================================
// Multiple traits on same type
// =========================================================================

#[test]
fn two_traits_on_same_type() {
    ShapeTest::new(
        r#"
        type Person { name: string, age: int }
        trait Named {
            get_name(self): string
        }
        trait Aged {
            get_age(self): int
        }
        impl Named for Person {
            method get_name() { self.name }
        }
        impl Aged for Person {
            method get_age() { self.age }
        }
        let p = Person { name: "Bob", age: 30 }
        print(p.get_name())
        print(p.get_age())
    "#,
    )
    .expect_output("Bob\n30");
}

// =========================================================================
// Impl method with parameters
// =========================================================================

#[test]
fn impl_method_with_extra_param() {
    ShapeTest::new(
        r#"
        type Counter { count: int }
        trait Addable {
            add(self, n: int): int
        }
        impl Addable for Counter {
            method add(n) { self.count + n }
        }
        let c = Counter { count: 10 }
        c.add(5)
    "#,
    )
    .expect_number(15.0);
}

// =========================================================================
// Named impl
// =========================================================================

#[test]
fn named_impl_parses() {
    ShapeTest::new(
        r#"
        trait Display {
            display(self): string
        }
        type User { name: string }
        impl Display for User as JsonDisplay {
            method display() { self.name }
        }
    "#,
    )
    .expect_parse_ok();
}
