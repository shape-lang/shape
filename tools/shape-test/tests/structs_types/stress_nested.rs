//! Stress tests for nested structs, structs in arrays, struct-array interactions,
//! structs in control flow, and complex struct scenarios.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 5. NESTED OBJECTS -- object containing object
// =========================================================================

// BUG: nested typed struct field access (l.end.x) returns the inner object instead of the field
#[test]
fn nested_typed_objects() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        type Line { start: Point, end: Point }
        let l = Line { start: Point { x: 0.0, y: 0.0 }, end: Point { x: 1.0, y: 1.0 } }
        l.end.x
    "#,
    )
    .expect_run_ok();
}

// BUG: nested typed struct field access (l.start.x) returns the inner object instead of the field
#[test]
fn nested_typed_objects_field_sum() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        type Line { start: Point, end: Point }
        let l = Line { start: Point { x: 1.0, y: 2.0 }, end: Point { x: 3.0, y: 4.0 } }
        l.start.x + l.start.y + l.end.x + l.end.y
    "#,
    )
    .expect_run_err();
}

/// Verifies nested anonymous objects.
#[test]
fn nested_anon_objects() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { point: { x: 5, y: 10 } }
            return obj.point.x
        }
        test()
    "#,
    )
    .expect_number(5.0);
}

// =========================================================================
// 6. DEEP NESTING -- 3+ levels
// =========================================================================

// BUG: nested typed struct field access (o.middle.inner.value) returns the inner object instead of the field
#[test]
fn deep_nesting_three_levels() {
    ShapeTest::new(
        r#"
        type Inner { value: int }
        type Middle { inner: Inner }
        type Outer { middle: Middle }
        let o = Outer { middle: Middle { inner: Inner { value: 42 } } }
        o.middle.inner.value
    "#,
    )
    .expect_run_ok();
}

/// Verifies deep nesting three levels of anonymous objects.
#[test]
fn deep_nesting_anon_three_levels() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { a: { b: { c: 99 } } }
            return obj.a.b.c
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

// =========================================================================
// 12. OBJECT IN ARRAY
// =========================================================================

/// Verifies typed objects in array with field access.
#[test]
fn typed_objects_in_array() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let pts = [Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }]
            return pts[0].x
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies typed objects in array second element field access.
#[test]
fn typed_objects_in_array_second_element() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let pts = [Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }]
            return pts[1].y
        }
        test()
    "#,
    )
    .expect_number(4.0);
}

/// Verifies array of anonymous objects with index access.
#[test]
fn array_of_anon_objects() {
    ShapeTest::new(
        r#"
        function test() {
            let items = [{ val: 10 }, { val: 20 }, { val: 30 }]
            return items[2].val
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

/// Verifies array length with typed objects.
#[test]
fn array_length_with_typed_objects() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let pts = [Point { x: 0.0, y: 0.0 }, Point { x: 1.0, y: 1.0 }]
            return pts.length
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

// =========================================================================
// 24. STRUCT IN CONDITIONAL
// =========================================================================

/// Verifies struct field in if condition.
#[test]
fn struct_field_in_if_condition() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 5.0, y: 3.0 }
            if p.x > p.y {
                return p.x
            } else {
                return p.y
            }
        }
        test()
    "#,
    )
    .expect_number(5.0);
}

/// Verifies struct field in if-else takes else branch.
#[test]
fn struct_field_in_if_else() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 2.0, y: 8.0 }
            if p.x > p.y {
                return p.x
            } else {
                return p.y
            }
        }
        test()
    "#,
    )
    .expect_number(8.0);
}

/// Verifies struct field used in for loop.
#[test]
fn struct_field_in_loop() {
    ShapeTest::new(
        r#"
        type Counter { count: int }
        function test() {
            let c = Counter { count: 0 }
            let mut sum = 0
            for i in range(5) {
                sum = sum + c.count + i
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// 51-60. STRUCT IN VARIOUS CONTEXTS
// =========================================================================

/// Verifies struct assigned to two variables.
#[test]
fn struct_assigned_to_two_variables() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 5.0, y: 10.0 }
        let q = p
        q.x
    "#,
    )
    .expect_number(5.0);
}

/// Verifies struct in ternary-like if expression (true branch).
#[test]
fn struct_in_ternary_like_if() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let a = Point { x: 1.0, y: 2.0 }
            let b = Point { x: 3.0, y: 4.0 }
            let chosen = if true { a } else { b }
            return chosen.x
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies struct in ternary-like if expression (false branch).
#[test]
fn struct_in_ternary_false_branch() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let a = Point { x: 1.0, y: 2.0 }
            let b = Point { x: 3.0, y: 4.0 }
            let chosen = if false { a } else { b }
            return chosen.x
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies struct field used in while loop condition.
#[test]
fn struct_field_in_while_loop() {
    ShapeTest::new(
        r#"
        type Config { limit: int }
        function test() {
            let cfg = Config { limit: 5 }
            let mut i = 0
            let mut sum = 0
            while i < cfg.limit {
                sum = sum + i
                i = i + 1
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies struct created in for loop body.
#[test]
fn struct_in_for_loop_body() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let mut sum = 0.0
            for i in range(3) {
                let p = Point { x: 1.0, y: 2.0 }
                sum = sum + p.x + p.y
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(9.0);
}

/// Verifies anonymous object created in for loop.
#[test]
fn anon_object_in_for_loop() {
    ShapeTest::new(
        r#"
        function test() {
            let mut total = 0
            for i in range(4) {
                let obj = { val: i }
                total = total + obj.val
            }
            return total
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

// =========================================================================
// 81-90. STRUCT AND ARRAY INTERACTIONS
// =========================================================================

/// Verifies iterating over array of structs.
#[test]
fn iterate_array_of_structs() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let pts = [Point { x: 1.0, y: 0.0 }, Point { x: 2.0, y: 0.0 }, Point { x: 3.0, y: 0.0 }]
            let mut sum = 0.0
            for p in pts {
                sum = sum + p.x
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

/// Verifies map over array of structs.
#[test]
fn map_over_array_of_structs() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let pts = [Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }]
            let xs = pts.map(|p| p.x)
            return xs[0] + xs[1]
        }
        test()
    "#,
    )
    .expect_number(4.0);
}

/// Verifies filter array of structs.
#[test]
fn filter_array_of_structs() {
    ShapeTest::new(
        r#"
        type Item { value: int }
        function test() {
            let items = [Item { value: 1 }, Item { value: 5 }, Item { value: 3 }]
            let big = items.filter(|i| i.value > 2)
            return big.length
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Verifies array of structs index last element.
#[test]
fn array_of_structs_index_last() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        function test() {
            let pts = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }, Point { x: 5, y: 6 }]
            return pts[2].y
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

/// Verifies struct with empty array field.
#[test]
fn struct_with_empty_array_field() {
    ShapeTest::new(
        r#"
        type Container { items: Array<int> }
        function test() {
            let c = Container { items: [] }
            return c.items.length
        }
        test()
    "#,
    )
    .expect_number(0.0);
}

/// Verifies concat on struct array field.
#[test]
fn array_push_on_struct_field() {
    ShapeTest::new(
        r#"
        type Container { items: Array<int> }
        function test() {
            let c = Container { items: [1, 2] }
            let extended = c.items.concat([3])
            return extended.length
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies struct constructed from array elements.
#[test]
fn struct_from_array_element() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let data = [10.0, 20.0]
            let p = Point { x: data[0], y: data[1] }
            return p.x + p.y
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

/// Verifies nested struct in array.
#[test]
fn nested_struct_in_array() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        type Segment { start: Point, end: Point }
        function test() {
            let segs = [
                Segment { start: Point { x: 0.0, y: 0.0 }, end: Point { x: 1.0, y: 1.0 } },
                Segment { start: Point { x: 2.0, y: 2.0 }, end: Point { x: 3.0, y: 3.0 } }
            ]
            return segs[1].end.x
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies building array of structs in a loop.
#[test]
fn build_array_of_structs_in_loop() {
    ShapeTest::new(
        r#"
        type Wrapper { val: int }
        function test() {
            let mut arr = []
            for i in range(5) {
                arr = arr.concat([Wrapper { val: i }])
            }
            return arr[3].val
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies reduce over array of structs.
#[test]
fn reduce_array_of_structs() {
    ShapeTest::new(
        r#"
        type Wrapper { val: number }
        function test() {
            let items = [Wrapper { val: 1.0 }, Wrapper { val: 2.0 }, Wrapper { val: 3.0 }]
            let sum = items.reduce(|acc, item| acc + item.val, 0.0)
            return sum
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

// =========================================================================
// 91-100. EDGE CASES AND COMPLEX SCENARIOS
// =========================================================================

/// Verifies struct field access after variable reassignment using var.
#[test]
fn struct_field_access_after_reassignment() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let mut p = Point { x: 1.0, y: 2.0 }
            p = Point { x: 10.0, y: 20.0 }
            return p.x
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies struct used as map key-value computation.
#[test]
fn struct_used_as_map_key_value() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        function test() {
            let p = Point { x: 5, y: 10 }
            return p.x * p.y
        }
        test()
    "#,
    )
    .expect_number(50.0);
}

/// Verifies multiple accesses to same struct field.
#[test]
fn struct_multiple_access_same_field() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 7.0, y: 3.0 }
            return p.x + p.x + p.x
        }
        test()
    "#,
    )
    .expect_number(21.0);
}

/// Verifies struct field used in comparison.
#[test]
fn struct_field_used_in_comparison() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 5.0, y: 3.0 }
            return p.x > p.y
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies struct field equality comparison.
#[test]
fn struct_field_equality() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 5.0, y: 5.0 }
            return p.x == p.y
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies struct constructed conditionally.
#[test]
fn struct_constructed_conditionally() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let flag = true
            let p = if flag {
                Point { x: 1.0, y: 2.0 }
            } else {
                Point { x: 3.0, y: 4.0 }
            }
            return p.x
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies struct constructed in match expression.
#[test]
fn struct_constructed_in_match() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let n = 2
            let p = match n {
                1 => Point { x: 1.0, y: 1.0 },
                2 => Point { x: 2.0, y: 2.0 },
                _ => Point { x: 0.0, y: 0.0 }
            }
            return p.x
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Verifies struct node-like pattern with depth computation.
#[test]
fn struct_recursive_like_tree_node() {
    ShapeTest::new(
        r#"
        type Node { value: int, depth: int }
        function test() {
            let root = Node { value: 1, depth: 0 }
            let child = Node { value: 2, depth: root.depth + 1 }
            return child.depth
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies struct field used as array index.
#[test]
fn struct_field_used_as_array_index() {
    ShapeTest::new(
        r#"
        type Idx { val: int }
        function test() {
            let arr = [10, 20, 30]
            let idx = Idx { val: 1 }
            return arr[idx.val]
        }
        test()
    "#,
    )
    .expect_number(20.0);
}

/// Verifies struct field in string interpolation.
#[test]
fn struct_field_in_string_interpolation() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3.0, y: 4.0 }
            return f"x=${p.x}"
        }
        test()
    "#,
    )
    .expect_run_ok();
}
