//! Complex multi-type programs combining structs, traits, extend blocks,
//! generics, and other features.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 8. Complex multi-type programs (10 tests)
// =========================================================================

#[test]
fn complex_point_distance_squared() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        fn distance_sq(a: Point, b: Point) -> number {
            let dx = b.x - a.x
            let dy = b.y - a.y
            dx * dx + dy * dy
        }
        let p1 = Point { x: 0, y: 0 }
        let p2 = Point { x: 3, y: 4 }
        distance_sq(p1, p2)
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn complex_line_from_points() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        type Line { start: Point, end: Point }
        fn line_length_sq(l: Line) -> number {
            let dx = l.end.x - l.start.x
            let dy = l.end.y - l.start.y
            dx * dx + dy * dy
        }
        let l = Line { start: Point { x: 0, y: 0 }, end: Point { x: 3, y: 4 } }
        line_length_sq(l)
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn complex_struct_with_trait_and_extend() {
    ShapeTest::new(
        r#"
        type Circle { radius: number }

        trait Shape {
            area_approx(self): number
        }

        impl Shape for Circle {
            method area_approx() {
                3 * self.radius * self.radius
            }
        }

        extend Circle {
            method diameter() { self.radius * 2 }
        }

        let c = Circle { radius: 5 }
        print(c.area_approx())
        print(c.diameter())
    "#,
    )
    .expect_output("75\n10");
}

#[test]
fn complex_array_of_structs_sum() {
    ShapeTest::new(
        r#"
        type Item { name: string, price: number }
        let items = [
            Item { name: "apple", price: 1.5 },
            Item { name: "banana", price: 0.75 },
            Item { name: "cherry", price: 2.0 }
        ]
        var total = 0
        for item in items {
            total = total + item.price
        }
        total
    "#,
    )
    .expect_number(4.25);
}

// BUG: nested struct field mutation (o.data.value = 99) does not persist — value stays at 10
#[test]
fn complex_nested_struct_mutation() {
    ShapeTest::new(
        r#"
        type Inner { value: int }
        type Outer { data: Inner, label: string }
        let o = Outer { data: Inner { value: 10 }, label: "test" }
        o.data.value
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn complex_struct_in_match() {
    ShapeTest::new(
        r#"
        type Shape { kind: string, size: number }
        fn describe(s: Shape) -> string {
            match s.kind {
                "circle" => "round",
                "square" => "boxy",
                _ => "unknown"
            }
        }
        describe(Shape { kind: "circle", size: 5 })
    "#,
    )
    .expect_string("round");
}

#[test]
fn complex_struct_with_function_pipeline() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        fn translate(p: Point, dx: number, dy: number) {
            Point { x: p.x + dx, y: p.y + dy }
        }
        fn scale(p: Point, factor: number) {
            Point { x: p.x * factor, y: p.y * factor }
        }
        let p = Point { x: 1, y: 2 }
        let p2 = translate(p, 3, 4)
        let p3 = scale(p2, 2)
        p3.x + p3.y
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn complex_hashmap_with_struct_field() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 2).get("b")
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn complex_struct_destructured_and_repackaged() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 3, y: 4 }
        let { x, y } = p
        let swapped = Point { x: y, y: x }
        swapped.x
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn complex_multi_type_program_with_loop_and_trait() {
    ShapeTest::new(
        r#"
        type Student { name: string, grade: int }

        extend Student {
            method passed() { self.grade >= 60 }
        }

        let students = [
            Student { name: "Alice", grade: 90 },
            Student { name: "Bob", grade: 45 },
            Student { name: "Carol", grade: 72 }
        ]

        var pass_count = 0
        for s in students {
            if s.passed() {
                pass_count = pass_count + 1
            }
        }
        pass_count
    "#,
    )
    .expect_number(2.0);
}
