//! Stress tests for default trait methods: used when not overridden, overridden,
//! calls required method, all default, returning number, empty impl blocks,
//! mixed override and default, multiple defaults.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 4. DEFAULT METHODS
// =========================================================================

/// Verifies trait default method used when not overridden.
#[test]
fn trait_default_method_used_when_not_overridden() {
    ShapeTest::new(
        r#"
        type Widget { label: string }
        trait Describable {
            name(self): string
            method describe() {
                "Object: " + self.name()
            }
        }
        impl Describable for Widget {
            method name() { self.label }
        }
        let w = Widget { label: "Button" }
        w.describe()
    "#,
    )
    .expect_string("Object: Button");
}

/// Verifies trait default method overridden.
#[test]
fn trait_default_method_overridden() {
    ShapeTest::new(
        r#"
        type Widget { label: string }
        trait Describable {
            name(self): string
            method describe() {
                "Object: " + self.name()
            }
        }
        impl Describable for Widget {
            method name() { self.label }
            method describe() { "Custom: " + self.name() }
        }
        let w = Widget { label: "Button" }
        w.describe()
    "#,
    )
    .expect_string("Custom: Button");
}

/// Verifies trait default method calls required method.
#[test]
fn trait_default_method_calls_required_method() {
    ShapeTest::new(
        r#"
        type Person { first: string, last: string }
        trait Named {
            first_name(self): string
            last_name(self): string
            method full_name() {
                self.first_name() + " " + self.last_name()
            }
        }
        impl Named for Person {
            method first_name() { self.first }
            method last_name() { self.last }
        }
        let p = Person { first: "John", last: "Doe" }
        p.full_name()
    "#,
    )
    .expect_string("John Doe");
}

// =========================================================================
// 15. TRAIT WITH ONLY DEFAULT METHODS
// =========================================================================

/// Verifies trait all default methods.
#[test]
fn trait_all_default_methods() {
    ShapeTest::new(
        r#"
        type Anything { val: int }
        trait Defaulted {
            method hello() { "hello" }
        }
        impl Defaulted for Anything {
        }
        let a = Anything { val: 1 }
        a.hello()
    "#,
    )
    .expect_string("hello");
}

/// Verifies trait default method returning number.
#[test]
fn trait_default_method_returning_number() {
    ShapeTest::new(
        r#"
        type Anything { val: int }
        trait WithDefault {
            method default_value() { 99 }
        }
        impl WithDefault for Anything {
        }
        let a = Anything { val: 1 }
        a.default_value()
    "#,
    )
    .expect_number(99.0);
}

// =========================================================================
// 47. TRAIT IMPL -- EMPTY IMPL BLOCK (all defaults)
// =========================================================================

/// Verifies empty impl block uses all defaults value.
#[test]
fn empty_impl_block_uses_all_defaults() {
    ShapeTest::new(
        r#"
        type X { n: int }
        trait WithDefaults {
            method value() { 77 }
            method label() { "default" }
        }
        impl WithDefaults for X {}
        let x = X { n: 1 }
        x.value()
    "#,
    )
    .expect_number(77.0);
}

/// Verifies empty impl block default label.
#[test]
fn empty_impl_block_default_label() {
    ShapeTest::new(
        r#"
        type X { n: int }
        trait WithDefaults {
            method value() { 77 }
            method label() { "default" }
        }
        impl WithDefaults for X {}
        let x = X { n: 1 }
        x.label()
    "#,
    )
    .expect_string("default");
}

// =========================================================================
// 59. TRAIT OVERRIDE DEFAULT WITH DIFFERENT LOGIC
// =========================================================================

/// Verifies override default with custom logic.
#[test]
fn override_default_with_custom_logic() {
    ShapeTest::new(
        r#"
        type Special { val: int }
        trait Defaulted {
            method calculate() { 0 }
        }
        impl Defaulted for Special {
            method calculate() { self.val * 100 }
        }
        let s = Special { val: 5 }
        s.calculate()
    "#,
    )
    .expect_number(500.0);
}

// =========================================================================
// 60. IMPL BLOCK WITH MIXED OVERRIDES AND DEFAULTS
// =========================================================================

/// Verifies mixed override and default description.
#[test]
fn mixed_override_and_default() {
    ShapeTest::new(
        r#"
        type Item { name: string }
        trait Descriptive {
            get_name(self): string
            method description() { "An item named: " + self.get_name() }
            method category() { "general" }
        }
        impl Descriptive for Item {
            method get_name() { self.name }
        }
        let i = Item { name: "Widget" }
        i.description()
    "#,
    )
    .expect_string("An item named: Widget");
}

/// Verifies mixed override and default category.
#[test]
fn mixed_override_and_default_category() {
    ShapeTest::new(
        r#"
        type Item { name: string }
        trait Descriptive {
            get_name(self): string
            method description() { "An item named: " + self.get_name() }
            method category() { "general" }
        }
        impl Descriptive for Item {
            method get_name() { self.name }
        }
        let i = Item { name: "Widget" }
        i.category()
    "#,
    )
    .expect_string("general");
}

// =========================================================================
// 63. MULTIPLE DEFAULT METHODS
// =========================================================================

/// Verifies trait multiple defaults.
#[test]
fn trait_multiple_defaults() {
    ShapeTest::new(
        r#"
        type Anything { x: int }
        trait MultiDefault {
            method alpha() { 1 }
            method beta() { 2 }
            method gamma() { 3 }
        }
        impl MultiDefault for Anything {}
        let a = Anything { x: 0 }
        a.alpha() + a.beta() + a.gamma()
    "#,
    )
    .expect_number(6.0);
}

// =========================================================================
// 92. MULTIPLE METHODS WITH DEFAULTS, PARTIAL OVERRIDE
// =========================================================================

/// Verifies partial override of defaults.
#[test]
fn partial_override_of_defaults() {
    ShapeTest::new(
        r#"
        type Item { name: string }
        trait Formatted {
            method prefix() { "Item" }
            method suffix() { "!" }
            method display_name() { self.prefix() + ": " + self.name + self.suffix() }
        }
        impl Formatted for Item {
            method prefix() { "Product" }
        }
        let i = Item { name: "Widget" }
        i.display_name()
    "#,
    )
    .expect_string("Product: Widget!");
}

// =========================================================================
// 98. DEFAULT METHOD WITH COMPLEX EXPRESSION
// =========================================================================

/// Verifies default method complex computation.
#[test]
fn default_method_complex_computation() {
    ShapeTest::new(
        r#"
        type Quad { a: number, b: number, c: number, d: number }
        trait Computable {
            method sum_all() {
                self.a + self.b + self.c + self.d
            }
            method avg() {
                (self.a + self.b + self.c + self.d) / 4.0
            }
        }
        impl Computable for Quad {}
        let q = Quad { a: 10.0, b: 20.0, c: 30.0, d: 40.0 }
        q.avg()
    "#,
    )
    .expect_number(25.0);
}

// =========================================================================
// 100. LARGE DEFAULT METHOD BODY
// =========================================================================

/// Verifies default method large body with min/max computation.
#[test]
fn default_method_large_body() {
    ShapeTest::new(
        r#"
        type Stats { a: int, b: int, c: int }
        trait Analyzer {
            method compute_stats() {
                let sum = self.a + self.b + self.c
                let min_val = if self.a < self.b {
                    if self.a < self.c { self.a } else { self.c }
                } else {
                    if self.b < self.c { self.b } else { self.c }
                }
                let max_val = if self.a > self.b {
                    if self.a > self.c { self.a } else { self.c }
                } else {
                    if self.b > self.c { self.b } else { self.c }
                }
                max_val - min_val + sum
            }
        }
        impl Analyzer for Stats {}
        let s = Stats { a: 10, b: 30, c: 20 }
        s.compute_stats()
    "#,
    )
    .expect_number(80.0);
}
