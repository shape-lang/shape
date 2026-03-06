//! Trait method dispatch: calling trait methods on concrete types, polymorphic dispatch.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Direct trait method call on concrete type
// =========================================================================

#[test]
fn call_trait_method_on_instance() {
    ShapeTest::new(
        r#"
        type Animal { kind: string, sound: string }
        trait Speak {
            speak(self): string
        }
        impl Speak for Animal {
            method speak() { self.kind + " says " + self.sound }
        }
        let a = Animal { kind: "Cat", sound: "Meow" }
        a.speak()
    "#,
    )
    .expect_string("Cat says Meow");
}

#[test]
fn call_trait_method_print_output() {
    ShapeTest::new(
        r#"
        type Product { name: string, price: number }
        trait Labelable {
            label(self): string
        }
        impl Labelable for Product {
            method label() { self.name + " $" + self.price }
        }
        let p = Product { name: "Pen", price: 1.5 }
        print(p.label())
    "#,
    )
    .expect_output("Pen $1.5");
}

// =========================================================================
// Multiple types implementing same trait
// =========================================================================

#[test]
fn same_trait_different_types_dispatch() {
    ShapeTest::new(
        r#"
        trait Summarize {
            summary(self): string
        }
        type Book { title: string }
        type Movie { title: string }
        impl Summarize for Book {
            method summary() { "book:" + self.title }
        }
        impl Summarize for Movie {
            method summary() { "movie:" + self.title }
        }
        let b = Book { title: "Dune" }
        let m = Movie { title: "Arrival" }
        print(b.summary())
        print(m.summary())
    "#,
    )
    .expect_output("book:Dune\nmovie:Arrival");
}

// =========================================================================
// Trait-bounded function dispatch
// =========================================================================

#[test]
fn bounded_function_dispatches_trait_method() {
    ShapeTest::new(
        r#"
        trait Describable {
            describe(self): string
        }
        type Color { name: string }
        impl Describable for Color {
            method describe() { "color:" + self.name }
        }
        fn show<T: Describable>(x: T) -> string {
            return x.describe()
        }
        show(Color { name: "red" })
    "#,
    )
    .expect_string("color:red");
}

// =========================================================================
// Chained trait method calls
// =========================================================================

#[test]
fn trait_method_result_used_in_expression() {
    ShapeTest::new(
        r#"
        type Score { value: int }
        trait Evaluable {
            evaluate(self): int
        }
        impl Evaluable for Score {
            method evaluate() { self.value * 2 }
        }
        let s = Score { value: 21 }
        s.evaluate()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn trait_method_bool_check() {
    ShapeTest::new(
        r#"
        type Container { items: int }
        trait Emptyable {
            empty(self): bool
        }
        impl Emptyable for Container {
            method empty() { self.items == 0 }
        }
        let c = Container { items: 3 }
        c.empty()
    "#,
    )
    .expect_bool(false);
}

// =========================================================================
// Multiple methods in one impl
// =========================================================================

#[test]
fn impl_with_multiple_methods_dispatch() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        trait Geometry {
            mag_sq(self): number;
            is_zero(self): bool
        }
        impl Geometry for Vec2 {
            method mag_sq() { self.x * self.x + self.y * self.y }
            method is_zero() { self.x == 0 && self.y == 0 }
        }
        let v = Vec2 { x: 3, y: 4 }
        print(v.mag_sq())
        print(v.is_zero())
    "#,
    )
    .expect_output("25\nfalse");
}
