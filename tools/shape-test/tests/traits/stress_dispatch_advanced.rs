//! Stress tests for advanced trait dispatch: conditional logic, local variables,
//! string interpolation, number method chains, chained trait calls, trait+extend
//! coexistence, null return, function interaction, bool fields, pythagorean,
//! nested fields, early return, loops, array/string ops, nested if, while
//! accumulator, complex expressions, negative numbers, boundary values,
//! recursion, and boolean logic.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 29. CONDITIONAL LOGIC + 33. LOCAL VARIABLES + 35. STRING INTERPOLATION
// =========================================================================

/// Verifies trait method with if expression.
#[test]
fn trait_method_with_if_expression() {
    ShapeTest::new(
        r#"
        type Score { value: int }
        trait Graded { grade(self): string }
        impl Graded for Score {
            method grade() {
                if self.value >= 90 { "A" }
                else if self.value >= 80 { "B" }
                else if self.value >= 70 { "C" }
                else { "F" }
            }
        }
        let s = Score { value: 85 }
        s.grade()
    "#,
    )
    .expect_string("B");
}

/// Verifies trait method conditional A.
#[test]
fn trait_method_conditional_a() {
    ShapeTest::new(
        r#"
        type Score { value: int }
        trait Graded { grade(self): string }
        impl Graded for Score {
            method grade() {
                if self.value >= 90 { "A" }
                else if self.value >= 80 { "B" }
                else { "F" }
            }
        }
        let s = Score { value: 95 }
        s.grade()
    "#,
    )
    .expect_string("A");
}

/// Verifies trait method with local variables.
#[test]
fn trait_method_with_local_variables() {
    ShapeTest::new(
        r#"
        type Triangle { a: number, b: number, c: number }
        trait Perimeterable { perimeter(self): number }
        impl Perimeterable for Triangle {
            method perimeter() {
                let sum = self.a + self.b + self.c
                sum
            }
        }
        let t = Triangle { a: 3.0, b: 4.0, c: 5.0 }
        t.perimeter()
    "#,
    )
    .expect_number(12.0);
}

/// Verifies trait method bio with f string.
#[test]
fn trait_method_with_f_string() {
    ShapeTest::new(
        r#"
        type User { name: string, age: int }
        trait Bio { bio(self): string }
        impl Bio for User {
            method bio() {
                self.name + " is " + self.age.to_string() + " years old"
            }
        }
        let u = User { name: "Bob", age: 30 }
        u.bio()
    "#,
    )
    .expect_string("Bob is 30 years old");
}

// =========================================================================
// 37. TRAIT NUMBER METHOD CALL CHAIN
// =========================================================================

/// Verifies trait number method call chain.
#[test]
fn trait_number_method_call_chain() {
    ShapeTest::new(
        r#"
        type Stats { sum: number, count: int }
        trait Aggregatable { avg(self): number }
        impl Aggregatable for Stats {
            method avg() { self.sum / self.count.to_number() }
        }
        let s = Stats { sum: 100.0, count: 4 }
        s.avg()
    "#,
    )
    .expect_number(25.0);
}

// =========================================================================
// 40. CHAINED TRAIT METHOD CALLS
// =========================================================================

/// Verifies chained trait method calls.
#[test]
fn chained_trait_method_calls() {
    ShapeTest::new(
        r#"
        type Builder { parts: int }
        trait Buildable { add_part(self): any }
        impl Buildable for Builder {
            method add_part() { Builder { parts: self.parts + 1 } }
        }
        let b = Builder { parts: 0 }
        let b2 = b.add_part()
        let b3 = b2.add_part()
        let b4 = b3.add_part()
        b4.parts
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// 41. TRAIT AND REGULAR METHOD COEXISTENCE
// =========================================================================

/// Verifies trait method alongside extend method.
#[test]
fn trait_method_alongside_extend_method() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        trait Printable { label(self): string }
        impl Printable for Point { method label() { "Point" } }
        extend Point { method magnitude() { self.x * self.x + self.y * self.y } }
        let p = Point { x: 3.0, y: 4.0 }
        p.magnitude()
    "#,
    )
    .expect_number(25.0);
}

/// Verifies trait method and extend method both callable.
#[test]
fn trait_method_and_extend_method_both_callable() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        trait Printable { label(self): string }
        impl Printable for Point { method label() { "Point" } }
        extend Point { method magnitude() { self.x * self.x + self.y * self.y } }
        let p = Point { x: 3.0, y: 4.0 }
        p.label()
    "#,
    )
    .expect_string("Point");
}

// =========================================================================
// 42-57. VARIOUS DISPATCH SCENARIOS
// =========================================================================

/// Verifies trait method returning null.
#[test]
fn trait_method_returning_null() {
    ShapeTest::new(
        r#"
        type Maybe { has_value: bool }
        trait Optional { get_or_null(self): any }
        impl Optional for Maybe {
            method get_or_null() { if self.has_value { 42 } else { None } }
        }
        let m = Maybe { has_value: false }
        m.get_or_null() ?? 0
    "#,
    )
    .expect_number(0.0);
}

/// Verifies trait method returning value when present.
#[test]
fn trait_method_returning_value_when_present() {
    ShapeTest::new(
        r#"
        type Maybe { has_value: bool }
        trait Optional { get_or_null(self): any }
        impl Optional for Maybe {
            method get_or_null() { if self.has_value { 42 } else { None } }
        }
        let m = Maybe { has_value: true }
        m.get_or_null() ?? 0
    "#,
    )
    .expect_number(42.0);
}

/// Verifies function calls trait method.
#[test]
fn function_calls_trait_method() {
    ShapeTest::new(
        r#"
        type Dog { name: string }
        trait Speaker { speak(self): string }
        impl Speaker for Dog { method speak() { self.name + " says woof" } }
        function make_speak(d: Dog) -> string { d.speak() }
        let d = Dog { name: "Rex" }
        make_speak(d)
    "#,
    )
    .expect_string("Rex says woof");
}

/// Verifies function returns trait method result.
#[test]
fn function_returns_trait_method_result() {
    ShapeTest::new(
        r#"
        type Adder { base: int }
        trait Addable { plus(self, n: int): int }
        impl Addable for Adder { method plus(n: int) { self.base + n } }
        function add_ten(a: Adder) -> int { a.plus(10) }
        let a = Adder { base: 32 }
        add_ten(a)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies trait method on bool field.
#[test]
fn trait_method_on_bool_field() {
    ShapeTest::new(
        r#"
        type Flag { active: bool }
        trait Toggleable { status(self): string }
        impl Toggleable for Flag {
            method status() { if self.active { "on" } else { "off" } }
        }
        let f = Flag { active: true }
        f.status()
    "#,
    )
    .expect_string("on");
}

/// Verifies trait method on bool field false.
#[test]
fn trait_method_on_bool_field_false() {
    ShapeTest::new(
        r#"
        type Flag { active: bool }
        trait Toggleable { status(self): string }
        impl Toggleable for Flag {
            method status() { if self.active { "on" } else { "off" } }
        }
        let f = Flag { active: false }
        f.status()
    "#,
    )
    .expect_string("off");
}

/// Verifies trait method pythagorean.
#[test]
fn trait_method_pythagorean() {
    ShapeTest::new(
        r#"
        type RightTriangle { a: number, b: number }
        trait Hypotenuse { hyp_squared(self): number }
        impl Hypotenuse for RightTriangle {
            method hyp_squared() { self.a * self.a + self.b * self.b }
        }
        let t = RightTriangle { a: 3.0, b: 4.0 }
        t.hyp_squared()
    "#,
    )
    .expect_number(25.0);
}

/// Verifies trait method nested field access.
#[test]
fn trait_method_nested_field_access() {
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Outer { inner: any }
        trait Extractable { extract(self): int }
        impl Extractable for Outer {
            method extract() {
                let i = self.inner
                i.val
            }
        }
        let inner = Inner { val: 99 }
        let outer = Outer { inner: inner }
        outer.extract()
    "#,
    )
    .expect_number(99.0);
}

/// Verifies trait method with early return.
#[test]
fn trait_method_with_early_return() {
    ShapeTest::new(
        r#"
        type Guard { level: int }
        trait Checkable { check(self): string }
        impl Checkable for Guard {
            method check() {
                if self.level > 5 { return "high" }
                return "low"
            }
        }
        let g = Guard { level: 8 }
        g.check()
    "#,
    )
    .expect_string("high");
}

/// Verifies trait method with early return low.
#[test]
fn trait_method_with_early_return_low() {
    ShapeTest::new(
        r#"
        type Guard { level: int }
        trait Checkable { check(self): string }
        impl Checkable for Guard {
            method check() {
                if self.level > 5 { return "high" }
                return "low"
            }
        }
        let g = Guard { level: 3 }
        g.check()
    "#,
    )
    .expect_string("low");
}

/// Verifies trait method with loop.
#[test]
fn trait_method_with_loop() {
    ShapeTest::new(
        r#"
        type Repeater { count: int }
        trait Repeatable { repeat_str(self, s: string): string }
        impl Repeatable for Repeater {
            method repeat_str(s: string) {
                let result = ""
                let i = 0
                while i < self.count {
                    result = result + s
                    i = i + 1
                }
                result
            }
        }
        let r = Repeater { count: 3 }
        r.repeat_str("ab")
    "#,
    )
    .expect_string("ababab");
}

/// Verifies trait method returns array length.
#[test]
fn trait_method_returns_array_length() {
    ShapeTest::new(
        r#"
        type Bag { items: any }
        trait Countable { count(self): int }
        impl Countable for Bag {
            method count() { self.items.length() }
        }
        let b = Bag { items: [1, 2, 3, 4, 5] }
        b.count()
    "#,
    )
    .expect_number(5.0);
}

/// Verifies trait method string length.
#[test]
fn trait_method_string_length() {
    ShapeTest::new(
        r#"
        type Message { text: string }
        trait Measurable { char_count(self): int }
        impl Measurable for Message {
            method char_count() { self.text.length() }
        }
        let m = Message { text: "hello" }
        m.char_count()
    "#,
    )
    .expect_number(5.0);
}

/// Verifies trait method on different instances.
#[test]
fn trait_method_on_different_instances() {
    ShapeTest::new(
        r#"
        type Box { size: int }
        trait Sizable { get_size(self): int }
        impl Sizable for Box { method get_size() { self.size } }
        let small = Box { size: 1 }
        let medium = Box { size: 5 }
        let large = Box { size: 10 }
        small.get_size() + medium.get_size() + large.get_size()
    "#,
    )
    .expect_number(16.0);
}

/// Verifies trait method nested if.
#[test]
fn trait_method_nested_if() {
    ShapeTest::new(
        r#"
        type Classifier { value: int }
        trait Classifiable { classify(self): string }
        impl Classifiable for Classifier {
            method classify() {
                if self.value > 0 {
                    if self.value > 100 { "large positive" }
                    else { "small positive" }
                } else {
                    if self.value < -100 { "large negative" }
                    else { "small negative or zero" }
                }
            }
        }
        let c = Classifier { value: 50 }
        c.classify()
    "#,
    )
    .expect_string("small positive");
}

/// Verifies trait method while accumulator.
#[test]
fn trait_method_while_accumulator() {
    ShapeTest::new(
        r#"
        type Summer { n: int }
        trait Summable { sum_to(self): int }
        impl Summable for Summer {
            method sum_to() {
                let total = 0
                let i = 1
                while i <= self.n {
                    total = total + i
                    i = i + 1
                }
                total
            }
        }
        let s = Summer { n: 10 }
        s.sum_to()
    "#,
    )
    .expect_number(55.0);
}

/// Verifies trait method complex expression (matrix determinant).
#[test]
fn trait_method_complex_expression() {
    ShapeTest::new(
        r#"
        type Matrix2x2 { a: number, b: number, c: number, d: number }
        trait Determinant { det(self): number }
        impl Determinant for Matrix2x2 {
            method det() { self.a * self.d - self.b * self.c }
        }
        let m = Matrix2x2 { a: 1.0, b: 2.0, c: 3.0, d: 4.0 }
        m.det()
    "#,
    )
    .expect_number(-2.0);
}

/// Verifies trait method handles negative numbers.
#[test]
fn trait_method_handles_negative_numbers() {
    ShapeTest::new(
        r#"
        type Num { val: int }
        trait Abs { abs_val(self): int }
        impl Abs for Num {
            method abs_val() { if self.val < 0 { -self.val } else { self.val } }
        }
        let n = Num { val: -42 }
        n.abs_val()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies trait method boundary zero.
#[test]
fn trait_method_boundary_zero() {
    ShapeTest::new(
        r#"
        type Counter { val: int }
        trait IsZero { is_zero(self): bool }
        impl IsZero for Counter { method is_zero() { self.val == 0 } }
        let c = Counter { val: 0 }
        c.is_zero()
    "#,
    )
    .expect_bool(true);
}

/// Verifies trait method boundary nonzero.
#[test]
fn trait_method_boundary_nonzero() {
    ShapeTest::new(
        r#"
        type Counter { val: int }
        trait IsZero { is_zero(self): bool }
        impl IsZero for Counter { method is_zero() { self.val == 0 } }
        let c = Counter { val: 1 }
        c.is_zero()
    "#,
    )
    .expect_bool(false);
}

/// Verifies trait method calls standalone recursive function.
#[test]
fn trait_method_calls_standalone_recursive_function() {
    ShapeTest::new(
        r#"
        function factorial(n: int) -> int {
            if n <= 1 { return 1 }
            return n * factorial(n - 1)
        }
        type MathBox { n: int }
        trait Factorable { fact(self): int }
        impl Factorable for MathBox { method fact() { factorial(self.n) } }
        let m = MathBox { n: 5 }
        m.fact()
    "#,
    )
    .expect_number(120.0);
}

/// Verifies trait method boolean logic and.
#[test]
fn trait_method_boolean_logic() {
    ShapeTest::new(
        r#"
        type Gate { a: bool, b: bool }
        trait Logic { and_gate(self): bool }
        impl Logic for Gate { method and_gate() { self.a and self.b } }
        let g = Gate { a: true, b: true }
        g.and_gate()
    "#,
    )
    .expect_bool(true);
}

/// Verifies trait method boolean or.
#[test]
fn trait_method_boolean_or() {
    ShapeTest::new(
        r#"
        type Gate { a: bool, b: bool }
        trait Logic { or_gate(self): bool }
        impl Logic for Gate { method or_gate() { self.a or self.b } }
        let g = Gate { a: false, b: true }
        g.or_gate()
    "#,
    )
    .expect_bool(true);
}

/// Trait method boolean — false and.
#[test]
fn trait_method_boolean_false_and() {
    ShapeTest::new(
        r#"
        type Gate { a: bool, b: bool }
        trait Logic { and_gate(self): bool }
        impl Logic for Gate { method and_gate() { self.a and self.b } }
        let g = Gate { a: true, b: false }
        g.and_gate()
    "#,
    )
    .expect_bool(false);
}

/// Trait method conditional — grade F.
#[test]
fn trait_method_conditional_f() {
    ShapeTest::new(
        r#"
        type Score { value: int }
        trait Graded { grade(self): string }
        impl Graded for Score {
            method grade() {
                if self.value >= 90 { "A" }
                else if self.value >= 80 { "B" }
                else { "F" }
            }
        }
        let s = Score { value: 50 }
        s.grade()
    "#,
    )
    .expect_string("F");
}

/// Trait method abs — positive input.
#[test]
fn trait_method_abs_positive() {
    ShapeTest::new(
        r#"
        type Num { val: int }
        trait Abs { abs_val(self): int }
        impl Abs for Num {
            method abs_val() { if self.val < 0 { -self.val } else { self.val } }
        }
        let n = Num { val: 42 }
        n.abs_val()
    "#,
    )
    .expect_number(42.0);
}

/// Trait method comparison — max.
#[test]
fn trait_method_comparison() {
    ShapeTest::new(
        r#"
        type Pair { a: int, b: int }
        trait Comparable { max_val(self): int }
        impl Comparable for Pair {
            method max_val() { if self.a > self.b { self.a } else { self.b } }
        }
        let p = Pair { a: 7, b: 12 }
        p.max_val()
    "#,
    )
    .expect_number(12.0);
}

/// Trait method min value.
#[test]
fn trait_method_min_val() {
    ShapeTest::new(
        r#"
        type Pair { a: int, b: int }
        trait Comparable { min_val(self): int }
        impl Comparable for Pair {
            method min_val() { if self.a < self.b { self.a } else { self.b } }
        }
        let p = Pair { a: 7, b: 12 }
        p.min_val()
    "#,
    )
    .expect_number(7.0);
}

/// Trait method ternary if.
#[test]
fn trait_method_ternary_if() {
    ShapeTest::new(
        r#"
        type Switch { on: bool }
        trait Valued { numeric(self): int }
        impl Valued for Switch {
            method numeric() { if self.on { 1 } else { 0 } }
        }
        let s = Switch { on: true }
        s.numeric()
    "#,
    )
    .expect_number(1.0);
}

/// Trait method on negative field.
#[test]
fn trait_method_on_negative_field() {
    ShapeTest::new(
        r#"
        type Temp { celsius: number }
        trait Weather { is_freezing(self): bool }
        impl Weather for Temp { method is_freezing() { self.celsius < 0.0 } }
        let t = Temp { celsius: -5.0 }
        t.is_freezing()
    "#,
    )
    .expect_bool(true);
}

/// Trait method returns constant.
#[test]
fn trait_method_returns_constant() {
    ShapeTest::new(
        r#"
        type Anything { x: int }
        trait Constant { magic(self): int }
        impl Constant for Anything { method magic() { 42 } }
        let a = Anything { x: 0 }
        a.magic()
    "#,
    )
    .expect_number(42.0);
}

/// Trait method returns new instance.
#[test]
fn trait_method_returns_new_instance() {
    ShapeTest::new(
        r#"
        type Counter { val: int }
        trait Incrementable { inc(self): any }
        impl Incrementable for Counter {
            method inc() { Counter { val: self.val + 1 } }
        }
        let c = Counter { val: 0 }
        let c1 = c.inc()
        let c2 = c1.inc()
        let c3 = c2.inc()
        c3.val
    "#,
    )
    .expect_number(3.0);
}

/// Trait method with large numbers.
#[test]
fn trait_method_with_large_numbers() {
    ShapeTest::new(
        r#"
        type Big { val: int }
        trait Doubler { double(self): int }
        impl Doubler for Big { method double() { self.val * 2 } }
        let b = Big { val: 1000000 }
        b.double()
    "#,
    )
    .expect_number(2000000.0);
}

/// Same field names different types same trait.
#[test]
fn same_field_names_different_types_same_trait() {
    ShapeTest::new(
        r#"
        type A { val: int }
        type B { val: int }
        trait GetVal { get_val(self): int }
        impl GetVal for A { method get_val() { self.val * 10 } }
        impl GetVal for B { method get_val() { self.val * 100 } }
        let a = A { val: 5 }
        let b = B { val: 5 }
        a.get_val() + b.get_val()
    "#,
    )
    .expect_number(550.0);
}

/// Trait declared far from impl.
#[test]
fn trait_declared_far_from_impl() {
    ShapeTest::new(
        r#"
        trait Describable { describe(self): string }

        type Apple { variety: string }
        type Orange { origin: string }

        let x = 1 + 2

        impl Describable for Apple {
            method describe() { "Apple: " + self.variety }
        }

        impl Describable for Orange {
            method describe() { "Orange from " + self.origin }
        }

        let a = Apple { variety: "Fuji" }
        a.describe()
    "#,
    )
    .expect_string("Apple: Fuji");
}

/// Five types implement same trait.
#[test]
fn five_types_implement_same_trait() {
    ShapeTest::new(
        r#"
        trait Named { name(self): string }
        type A { n: string }
        type B { n: string }
        type C { n: string }
        type D { n: string }
        type E { n: string }
        impl Named for A { method name() { "A:" + self.n } }
        impl Named for B { method name() { "B:" + self.n } }
        impl Named for C { method name() { "C:" + self.n } }
        impl Named for D { method name() { "D:" + self.n } }
        impl Named for E { method name() { "E:" + self.n } }
        let e = E { n: "test" }
        e.name()
    "#,
    )
    .expect_string("E:test");
}

/// Display on multiple types.
#[test]
fn display_on_multiple_types() {
    ShapeTest::new(
        r#"
        type Dog { name: string }
        type Cat { name: string }
        trait Display { display(self): string }
        impl Display for Dog { method display() { "Dog:" + self.name } }
        impl Display for Cat { method display() { "Cat:" + self.name } }
        let d = Dog { name: "Rex" }
        let c = Cat { name: "Whiskers" }
        d.to_string() + " and " + c.to_string()
    "#,
    )
    .expect_string("Dog:Rex and Cat:Whiskers");
}

/// Trait method calling another trait method.
#[test]
fn trait_method_calling_another_trait_method() {
    ShapeTest::new(
        r#"
        type Rect { w: number, h: number }
        trait Shape {
            area(self): number,
            perimeter(self): number
        }
        impl Shape for Rect {
            method area() { self.w * self.h }
            method perimeter() { 2.0 * (self.w + self.h) }
        }
        let r = Rect { w: 5.0, h: 3.0 }
        r.area() + r.perimeter()
    "#,
    )
    .expect_number(31.0);
}

/// Trait method using closure variable.
#[test]
fn trait_method_using_closure_variable() {
    ShapeTest::new(
        r#"
        type Mapper { factor: number }
        trait Applicable { apply(self, values: any): any }
        impl Applicable for Mapper {
            method apply(values: any) {
                let f = self.factor
                values.map(|x| x * f)
            }
        }
        let m = Mapper { factor: 2.0 }
        let result = m.apply([1.0, 2.0, 3.0])
        result.length()
    "#,
    )
    .expect_number(3.0);
}

/// Trait mixed return types — int.
#[test]
fn trait_mixed_return_types() {
    ShapeTest::new(
        r#"
        type Data { n: int, s: string, b: bool }
        trait Accessor {
            get_n(self): int,
            get_s(self): string,
            get_b(self): bool
        }
        impl Accessor for Data {
            method get_n() { self.n }
            method get_s() { self.s }
            method get_b() { self.b }
        }
        let d = Data { n: 42, s: "hello", b: true }
        d.get_n()
    "#,
    )
    .expect_number(42.0);
}

/// Trait mixed return types — string.
#[test]
fn trait_mixed_return_types_string() {
    ShapeTest::new(
        r#"
        type Data { n: int, s: string, b: bool }
        trait Accessor {
            get_n(self): int,
            get_s(self): string,
            get_b(self): bool
        }
        impl Accessor for Data {
            method get_n() { self.n }
            method get_s() { self.s }
            method get_b() { self.b }
        }
        let d = Data { n: 42, s: "hello", b: true }
        d.get_s()
    "#,
    )
    .expect_string("hello");
}

/// Trait mixed return types — bool.
#[test]
fn trait_mixed_return_types_bool() {
    ShapeTest::new(
        r#"
        type Data { n: int, s: string, b: bool }
        trait Accessor {
            get_n(self): int,
            get_s(self): string,
            get_b(self): bool
        }
        impl Accessor for Data {
            method get_n() { self.n }
            method get_s() { self.s }
            method get_b() { self.b }
        }
        let d = Data { n: 42, s: "hello", b: true }
        d.get_b()
    "#,
    )
    .expect_bool(true);
}

/// Two traits same method name different types.
#[test]
fn two_traits_same_method_name_different_types() {
    ShapeTest::new(
        r#"
        trait InfoA { info(self): string }
        trait InfoB { info(self): string }
        type TypeA { name: string }
        type TypeB { code: int }
        impl InfoA for TypeA { method info() { "A:" + self.name } }
        impl InfoB for TypeB { method info() { "B:" + self.code.to_string() } }
        let a = TypeA { name: "test" }
        a.info()
    "#,
    )
    .expect_string("A:test");
}

/// Call same trait method on two different types.
#[test]
fn call_same_trait_method_on_two_different_types() {
    ShapeTest::new(
        r#"
        trait Greetable { greet(self): string }
        type Human { name: string }
        type Robot { id: int }
        impl Greetable for Human { method greet() { "Hi, I'm " + self.name } }
        impl Greetable for Robot { method greet() { "Unit " + self.id.to_string() } }
        let h = Human { name: "Alice" }
        let r = Robot { id: 42 }
        h.greet() + " & " + r.greet()
    "#,
    )
    .expect_string("Hi, I'm Alice & Unit 42");
}

/// Nested trait method in expression.
#[test]
fn nested_trait_method_in_expression() {
    ShapeTest::new(
        r#"
        type A { v: int }
        type B { v: int }
        trait GetV { get_v(self): int }
        impl GetV for A { method get_v() { self.v } }
        impl GetV for B { method get_v() { self.v } }
        let a = A { v: 10 }
        let b = B { v: 20 }
        a.get_v() * b.get_v()
    "#,
    )
    .expect_number(200.0);
}

/// Function wraps trait method call.
#[test]
fn function_wraps_trait_method_call() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        trait Measurable { distance_from_origin(self): number }
        impl Measurable for Point {
            method distance_from_origin() { self.x * self.x + self.y * self.y }
        }
        function test() {
            let p = Point { x: 3.0, y: 4.0 }
            return p.distance_from_origin()
        }
        test()
    "#,
    )
    .expect_number(25.0);
}

/// Trait method called in loop.
#[test]
fn trait_method_called_in_loop() {
    ShapeTest::new(
        r#"
        type Counter { val: int }
        trait Incrementable { inc(self): any }
        impl Incrementable for Counter {
            method inc() { Counter { val: self.val + 1 } }
        }
        let mut c = Counter { val: 0 }
        let i = 0
        while i < 10 {
            c = c.inc()
            i = i + 1
        }
        c.val
    "#,
    )
    .expect_number(10.0);
}

/// Multiple named display impls — default used.
#[test]
fn multiple_named_display_impls_default_used() {
    ShapeTest::new(
        r#"
        type Msg { text: string }
        trait Display { display(self): string }
        impl Display for Msg { method display() { "plain:" + self.text } }
        impl Display for Msg as Fancy { method display() { "fancy:" + self.text } }
        let m = Msg { text: "hi" }
        m.to_string()
    "#,
    )
    .expect_string("plain:hi");
}
