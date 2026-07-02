//! Integration tests for the `set` stdlib module via Shape source code.

use shape_test::shape_test::ShapeTest;

#[test]
fn set_new_empty() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s: Set<int> = set::new()
        print(set::len(s))
    "#,
    )
    .with_stdlib()
    .expect_output("0");
}

#[test]
fn set_direct_ctor_explicit_int() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s: Set<int> = Set()
        let s1 = set::add(s, 42)
        print(set::includes(s1, 42))
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

#[test]
fn set_direct_ctor_usage_pinned_string() {
    ShapeTest::new(
        r#"
        let mut s = Set()
        s.add("a")
        print(s.len())
    "#,
    )
    .with_stdlib()
    .expect_output("1");
}

#[test]
fn set_direct_ctor_rvalue_receiver_usage_pinned_string() {
    ShapeTest::new(
        r#"
        print(Set().add("x").len())
    "#,
    )
    .with_stdlib()
    .expect_output("1");
}

#[test]
fn set_from_array_dedup() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s = set::from_array([1, 2, 2, 3, 3, 3])
        print(set::len(s))
    "#,
    )
    .with_stdlib()
    .expect_output("3");
}

#[test]
fn set_add_item() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s1 = set::add(set::new(), 42)
        print(set::len(s1))
    "#,
    )
    .with_stdlib()
    .expect_output("1");
}

#[test]
fn set_add_duplicate() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s1 = set::add(set::new(), 42)
        let s2 = set::add(s1, 42)
        print(set::len(s2))
    "#,
    )
    .with_stdlib()
    .expect_output("1");
}

#[test]
fn set_contains_true() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s = set::from_array([10, 20, 30])
        print(set::includes(s, 20))
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

#[test]
fn set_contains_false() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s = set::from_array([10, 20, 30])
        print(set::includes(s, 99))
    "#,
    )
    .with_stdlib()
    .expect_output("false");
}

#[test]
fn set_union() {
    ShapeTest::new(
        r#"
        use std::core::set
        let a = set::from_array([1, 2])
        let b = set::from_array([2, 3])
        let u = set::union(a, b)
        print(set::len(u))
    "#,
    )
    .with_stdlib()
    .expect_output("3");
}

#[test]
fn set_intersection() {
    ShapeTest::new(
        r#"
        use std::core::set
        let a = set::from_array([1, 2, 3])
        let b = set::from_array([2, 3, 4])
        let i = set::intersection(a, b)
        print(set::len(i))
    "#,
    )
    .with_stdlib()
    .expect_output("2");
}

#[test]
fn set_difference() {
    ShapeTest::new(
        r#"
        use std::core::set
        let a = set::from_array([1, 2, 3])
        let b = set::from_array([2, 4])
        let d = set::difference(a, b)
        print(set::len(d))
    "#,
    )
    .with_stdlib()
    .expect_output("2");
}

#[test]
fn set_to_array() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s = set::from_array([10, 20])
        let arr = set::to_array(s)
        print(arr.length())
    "#,
    )
    .with_stdlib()
    .expect_output("2");
}

#[test]
fn set_remove() {
    ShapeTest::new(
        r#"
        use std::core::set
        let s1 = set::from_array([1, 2, 3])
        let s2 = set::remove(s1, 2)
        print(set::len(s2))
        print(set::includes(s2, 2))
    "#,
    )
    .with_stdlib()
    .expect_output("2\nfalse");
}
