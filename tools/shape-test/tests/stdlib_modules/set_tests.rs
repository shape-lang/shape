//! Integration tests for the `set` stdlib module via Shape source code.

use shape_test::shape_test::ShapeTest;

#[test]
fn set_new_empty() {
    ShapeTest::new(
        r#"
        let s = set.new()
        print(set.size(s))
    "#,
    )
    .with_stdlib()
    .expect_output("0");
}

#[test]
fn set_from_array_dedup() {
    ShapeTest::new(
        r#"
        let s = set.from_array([1, 2, 2, 3, 3, 3])
        print(set.size(s))
    "#,
    )
    .with_stdlib()
    .expect_output("3");
}

#[test]
fn set_add_item() {
    ShapeTest::new(
        r#"
        let s1 = set.add(set.new(), 42)
        print(set.size(s1))
    "#,
    )
    .with_stdlib()
    .expect_output("1");
}

#[test]
fn set_add_duplicate() {
    ShapeTest::new(
        r#"
        let s1 = set.add(set.new(), 42)
        let s2 = set.add(s1, 42)
        print(set.size(s2))
    "#,
    )
    .with_stdlib()
    .expect_output("1");
}

#[test]
fn set_contains_true() {
    ShapeTest::new(
        r#"
        let s = set.from_array([10, 20, 30])
        print(set.contains(s, 20))
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

#[test]
fn set_contains_false() {
    ShapeTest::new(
        r#"
        let s = set.from_array([10, 20, 30])
        print(set.contains(s, 99))
    "#,
    )
    .with_stdlib()
    .expect_output("false");
}

#[test]
fn set_union() {
    ShapeTest::new(
        r#"
        let a = set.from_array([1, 2])
        let b = set.from_array([2, 3])
        let u = set.union(a, b)
        print(set.size(u))
    "#,
    )
    .with_stdlib()
    .expect_output("3");
}

#[test]
fn set_intersection() {
    ShapeTest::new(
        r#"
        let a = set.from_array([1, 2, 3])
        let b = set.from_array([2, 3, 4])
        let i = set.intersection(a, b)
        print(set.size(i))
    "#,
    )
    .with_stdlib()
    .expect_output("2");
}

#[test]
fn set_difference() {
    ShapeTest::new(
        r#"
        let a = set.from_array([1, 2, 3])
        let b = set.from_array([2, 4])
        let d = set.difference(a, b)
        print(set.size(d))
    "#,
    )
    .with_stdlib()
    .expect_output("2");
}

#[test]
fn set_to_array() {
    ShapeTest::new(
        r#"
        let s = set.from_array([10, 20])
        let arr = set.to_array(s)
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
        let s1 = set.from_array([1, 2, 3])
        let s2 = set.remove(s1, 2)
        print(set.size(s2))
        print(set.contains(s2, 2))
    "#,
    )
    .with_stdlib()
    .expect_output("2\nfalse");
}
