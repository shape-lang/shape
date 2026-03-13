//! Trait definitions, impl blocks, and extend block tests.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 4. Traits and impl (20 tests)
// =========================================================================

#[test]
fn trait_definition_basic_parses() {
    ShapeTest::new(
        r#"
        trait Display {
            display(self): string
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_with_multiple_methods_parses() {
    ShapeTest::new(
        r#"
        trait Collection {
            size(self): int;
            is_empty(self): bool;
            first(self): any
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_impl_method_call() {
    ShapeTest::new(
        r#"
        type User { name: string }
        impl Display for User {
            method display() { self.name }
        }
        let u = User { name: "Alice" }
        u.display()
    "#,
    )
    .expect_string("Alice");
}

#[test]
fn trait_impl_method_returns_computed_value() {
    ShapeTest::new(
        r#"
        type Rect { width: number, height: number }
        trait Area {
            area(self): number
        }
        impl Area for Rect {
            method area() {
                self.width * self.height
            }
        }
        let r = Rect { width: 5, height: 10 }
        r.area()
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn trait_impl_with_string_return() {
    ShapeTest::new(
        r#"
        type Color { r: int, g: int, b: int }
        trait Describe {
            describe(self): string
        }
        impl Describe for Color {
            method describe() { "color" }
        }
        let c = Color { r: 255, g: 0, b: 0 }
        c.describe()
    "#,
    )
    .expect_string("color");
}

#[test]
fn trait_impl_method_accesses_multiple_fields() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        trait Magnitude {
            mag_sq(self): number
        }
        impl Magnitude for Vec2 {
            method mag_sq() {
                self.x * self.x + self.y * self.y
            }
        }
        let v = Vec2 { x: 3, y: 4 }
        v.mag_sq()
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn trait_multiple_impls_same_type() {
    ShapeTest::new(
        r#"
        type User { name: string, age: int }

        trait Greet {
            greet(self): string
        }

        trait Info {
            info(self): string
        }

        impl Greet for User {
            method greet() { "Hello " + self.name }
        }

        impl Info for User {
            method info() { self.name }
        }

        let u = User { name: "Bob", age: 25 }
        print(u.greet())
        print(u.info())
    "#,
    )
    .expect_output("Hello Bob\nBob");
}

#[test]
fn trait_bound_function_dispatch() {
    ShapeTest::new(
        r#"
        trait Displayable {
            display(): string
        }

        type User { name: string }

        impl Displayable for User {
            method display() { "user:" + self.name }
        }

        fn render<T: Displayable>(value: T) -> string {
            return value.display()
        }

        render(User { name: "Ada" })
    "#,
    )
    .expect_string("user:Ada");
}

#[test]
fn trait_impl_boolean_method() {
    ShapeTest::new(
        r#"
        type Box { items: int }
        trait Checkable {
            is_empty(self): bool
        }
        impl Checkable for Box {
            method is_empty() { self.items == 0 }
        }
        let b = Box { items: 0 }
        b.is_empty()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn trait_impl_boolean_method_false() {
    ShapeTest::new(
        r#"
        type Box { items: int }
        trait Checkable {
            is_empty(self): bool
        }
        impl Checkable for Box {
            method is_empty() { self.items == 0 }
        }
        let b = Box { items: 5 }
        b.is_empty()
    "#,
    )
    .expect_bool(false);
}

#[test]
fn trait_where_clause_parses() {
    ShapeTest::new(
        r#"
        function process<T>(x: T) -> string where T: Display {
            return "ok"
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_where_clause_multiple_bounds_parses() {
    ShapeTest::new(
        r#"
        function transform<T>(x: T) where T: Display + Serializable {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_associated_type_parses() {
    ShapeTest::new(
        r#"
        trait Iterator {
            type Item;
            next(self): any
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_impl_with_associated_type_parses() {
    ShapeTest::new(
        r#"
        impl Iterator for Range {
            type Item = number;
            method next() {
                return self.current
            }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_default_method_parses() {
    ShapeTest::new(
        r#"
        trait Queryable {
            filter(pred): any;
            method execute() {
                return self
            }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_named_impl_parses() {
    ShapeTest::new(
        r#"
        impl Display for User as JsonDisplay {
            method display() {
                return ""
            }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_dyn_type_parses() {
    ShapeTest::new(
        r#"
        function render(obj: dyn Display) -> string {
            return ""
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_dyn_multiple_bounds_parses() {
    ShapeTest::new(
        r#"
        let x: dyn Display + Serializable = value
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_impl_print_output() {
    ShapeTest::new(
        r#"
        type Animal { kind: string, sound: string }
        trait Speak {
            speak(self): string
        }
        impl Speak for Animal {
            method speak() { self.kind + " says " + self.sound }
        }
        let a = Animal { kind: "Dog", sound: "Woof" }
        print(a.speak())
    "#,
    )
    .expect_output("Dog says Woof");
}

#[test]
fn trait_impl_method_with_param() {
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
// 5. Extend blocks (10 tests)
// =========================================================================

#[test]
fn extend_basic_method() {
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
fn extend_multiple_methods() {
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

#[test]
fn extend_method_returns_string() {
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
fn extend_method_returns_bool() {
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
        let v = Value { n: 5 }
        v.classify()
    "#,
    )
    .expect_string("positive");
}

#[test]
fn extend_method_negative_classify() {
    ShapeTest::new(
        r#"
        type Value { n: number }
        extend Value {
            method classify() {
                if self.n > 0 { "positive" } else if self.n < 0 { "negative" } else { "zero" }
            }
        }
        let v = Value { n: -3 }
        v.classify()
    "#,
    )
    .expect_string("negative");
}

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

#[test]
fn extend_method_chained_call() {
    ShapeTest::new(
        r#"
        type Num { val: number }
        extend Num {
            method doubled() { self.val * 2 }
        }
        let n = Num { val: 21 }
        n.doubled()
    "#,
    )
    .expect_number(42.0);
}

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
