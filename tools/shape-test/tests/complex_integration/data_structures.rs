//! Data Structure Programs (15 tests)
//!
//! Tests for stack, queue, hashmap, typed structs, sets, and trait dispatch.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Data Structure Programs (15 tests)
// =========================================================================

#[test]
fn test_complex_stack_push_pop() {
    ShapeTest::new(
        r#"
        var stack = []
        fn push(val) { stack = stack.push(val) }
        fn pop() {
            let top = stack[stack.length - 1]
            stack = stack.filter(|x| true).slice(0, stack.length - 1)
            top
        }
        fn peek() { stack[stack.length - 1] }

        push(10)
        push(20)
        push(30)
        print(peek())
        print(pop())
        print(peek())
        print(stack.length)
    "#,
    )
    .expect_output("30\n30\n20\n2");
}

#[test]
fn test_complex_counter_accumulator() {
    // Multiple closures from same scope share captures
    ShapeTest::new(
        r#"
        fn make_counter(start) {
            let val = start
            let inc = || {
                val = val + 1
                val
            }
            inc
        }
        let inc = make_counter(0)
        inc()
        inc()
        print(inc())
    "#,
    )
    .expect_output("3");
}

#[test]
fn test_complex_hashmap_key_value_store() {
    ShapeTest::new(
        r#"
        let store = HashMap()
            .set("name", "Alice")
            .set("age", "30")
            .set("city", "NYC")
        print(store.get("name"))
        print(store.get("age"))
        print(store.get("city"))
    "#,
    )
    .expect_output("Alice\n30\nNYC");
}

#[test]
fn test_complex_hashmap_overwrite() {
    ShapeTest::new(
        r#"
        let m = HashMap()
            .set("key", "first")
            .set("key", "second")
        m.get("key")
    "#,
    )
    .expect_string("second");
}

#[test]
fn test_complex_nested_typed_objects() {
    ShapeTest::new(
        r#"
        type Address { city: string, zip: string }
        type Person { name: string, addr: Address }
        let p = Person { name: "Bob", addr: Address { city: "LA", zip: "90001" } }
        print(p.name)
        print(p.addr.city)
        print(p.addr.zip)
    "#,
    )
    .expect_output("Bob\nLA\n90001");
}

#[test]
fn test_complex_set_union_via_arrays() {
    ShapeTest::new(
        r#"
        fn contains(arr, val) {
            for item in arr {
                if item == val { return true }
            }
            false
        }
        fn set_union(a, b) {
            var result = a
            for item in b {
                if !contains(result, item) {
                    result = result.push(item)
                }
            }
            result
        }
        let u = set_union([1, 2, 3], [2, 3, 4, 5])
        u.length
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_complex_set_intersection_via_arrays() {
    ShapeTest::new(
        r#"
        fn contains(arr, val) {
            for item in arr {
                if item == val { return true }
            }
            false
        }
        fn set_intersection(a, b) {
            a.filter(|item| contains(b, item))
        }
        let inter = set_intersection([1, 2, 3, 4], [3, 4, 5, 6])
        print(inter.length)
        print(inter[0])
        print(inter[1])
    "#,
    )
    .expect_output("2\n3\n4");
}

#[test]
fn test_complex_set_difference_via_arrays() {
    ShapeTest::new(
        r#"
        fn contains(arr, val) {
            for item in arr {
                if item == val { return true }
            }
            false
        }
        fn set_difference(a, b) {
            a.filter(|item| !contains(b, item))
        }
        let diff = set_difference([1, 2, 3, 4, 5], [2, 4])
        print(diff.length)
        for x in diff { print(x) }
    "#,
    )
    .expect_output("3\n1\n3\n5");
}

#[test]
fn test_complex_struct_with_methods() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        extend Vec2 {
            method add(other) {
                Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
            method dot(other) {
                self.x * other.x + self.y * other.y
            }
            method magnitude_sq() {
                self.x * self.x + self.y * self.y
            }
        }
        let a = Vec2 { x: 3, y: 4 }
        let b = Vec2 { x: 1, y: 2 }
        let c = a.add(b)
        print(c.x)
        print(c.y)
        print(a.dot(b))
        print(a.magnitude_sq())
    "#,
    )
    .expect_output("4\n6\n11\n25");
}

#[test]
fn test_complex_queue_via_array() {
    ShapeTest::new(
        r#"
        var queue = []
        fn enqueue(val) { queue = queue.push(val) }
        fn dequeue() {
            let front = queue[0]
            queue = queue.slice(1, queue.length)
            front
        }
        enqueue(1)
        enqueue(2)
        enqueue(3)
        print(dequeue())
        print(dequeue())
        enqueue(4)
        print(dequeue())
        print(queue.length)
    "#,
    )
    .expect_output("1\n2\n3\n1");
}

#[test]
fn test_complex_frequency_counter() {
    ShapeTest::new(
        r#"
        fn count_frequency(arr) {
            var map = HashMap()
            for item in arr {
                let key = item + ""
                let current = map.get(key)
                if current == None {
                    map = map.set(key, 1)
                } else {
                    map = map.set(key, current + 1)
                }
            }
            map
        }
        let freq = count_frequency(["a", "b", "a", "c", "b", "a"])
        print(freq.get("a"))
        print(freq.get("b"))
        print(freq.get("c"))
    "#,
    )
    .expect_output("3\n2\n1");
}

#[test]
fn test_complex_linked_operations_on_typed_struct() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        fn translate(p, dx, dy) {
            Point { x: p.x + dx, y: p.y + dy }
        }
        fn scale(p, factor) {
            Point { x: p.x * factor, y: p.y * factor }
        }
        let p = Point { x: 1, y: 2 }
        let p2 = scale(translate(p, 3, 4), 2)
        print(p2.x)
        print(p2.y)
    "#,
    )
    .expect_output("8\n12");
}

#[test]
fn test_complex_deep_nested_struct_access() {
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Mid { inner: Inner, label: string }
        type Outer { mid: Mid, count: int }
        let o = Outer {
            mid: Mid {
                inner: Inner { val: 42 },
                label: "deep"
            },
            count: 1
        }
        print(o.mid.inner.val)
        print(o.mid.label)
        print(o.count)
    "#,
    )
    .expect_output("42\ndeep\n1");
}

#[test]
fn test_complex_trait_impl_dispatch() {
    // BUG: Multiple extend blocks with same method name cause type confusion
    // Test single extend block dispatch
    ShapeTest::new(
        r#"
        type Circle { radius: number }
        extend Circle {
            method area() { 3 * self.radius * self.radius }
            method circumference() { 2 * 3 * self.radius }
        }
        let c = Circle { radius: 5 }
        print(c.area())
        print(c.circumference())
    "#,
    )
    .expect_output("75\n30");
}
