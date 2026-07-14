//! Focused regression coverage for mutable typed-object Option fields.

use shape_test::shape_test::ShapeTest;

#[test]
fn typed_object_option_field_mutation_some() {
    ShapeTest::new(
        r#"
        type Node { value: int, peer: Option<Node> }
        function test() -> bool {
            let b = Node { value: 2, peer: None }
            let mut a = Node { value: 1, peer: None }
            a.peer = Some(b)
            return a.peer != None
        }
        test()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn typed_object_option_field_mutation_none() {
    ShapeTest::new(
        r#"
        type Node { value: int, peer: Option<Node> }
        function test() -> Option<Node> {
            let b = Node { value: 2, peer: None }
            let mut a = Node { value: 1, peer: None }
            a.peer = Some(b)
            a.peer = None
            return a.peer
        }
        test()
    "#,
    )
    .expect_none();
}

#[test]
fn typed_object_option_field_nested_mutation_some() {
    ShapeTest::new(
        r#"
        type Node { value: int, peer: Option<Node> }
        type Holder { node: Node }
        function test() -> bool {
            let b = Node { value: 7, peer: None }
            let mut h = Holder { node: Node { value: 1, peer: None } }
            h.node.peer = Some(b)
            return h.node.peer != None
        }
        test()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn typed_object_option_field_payload_readback() {
    ShapeTest::new(
        r#"
        type Box { value: Option<int> }
        function test() {
            let mut b = Box { value: None }
            b.value = Some(7)
            print(b.value)
        }
        test()
    "#,
    )
    .expect_output("Some(7)");
}

#[test]
fn typed_object_option_field_overwrites_some_payload() {
    ShapeTest::new(
        r#"
        type Node { value: int, peer: Option<Node> }
        function test() -> int {
            let first = Node { value: 2, peer: None }
            let second = Node { value: 3, peer: None }
            let mut a = Node { value: 1, peer: None }
            a.peer = Some(first)
            a.peer = Some(second)
            return match a.peer { Some(n) => n.value, None => -1 }
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn typed_object_option_field_self_cycle_smoke() {
    ShapeTest::new(
        r#"
        type Node { value: int, peer: Option<Node> }
        function test() -> bool {
            let mut a = Node { value: 1, peer: None }
            a.peer = Some(a)
            return a.peer != None
        }
        test()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn typed_object_option_field_rejects_raw_object_assignment() {
    ShapeTest::new(
        r#"
        type Node { value: int, peer: Option<Node> }
        function test() {
            let b = Node { value: 2, peer: None }
            let mut a = Node { value: 1, peer: None }
            a.peer = b
            1
        }
        test()
    "#,
    )
    .expect_run_err_contains_any(&[
        "cannot assign value of type 'Node' to field 'peer' of type 'Option<Node>'",
        "canonical __Option.Some/None typed-object carrier",
        "Option<Node> is not compatible with Node",
        "not compatible with Option<Node>",
    ]);
}

#[test]
fn typed_object_option_field_rejects_wrong_some_payload() {
    ShapeTest::new(
        r#"
        type Node { value: int, peer: Option<Node> }
        type Other { value: int }
        function test() {
            let other = Other { value: 9 }
            let mut a = Node { value: 1, peer: None }
            a.peer = Some(other)
            1
        }
        test()
    "#,
    )
    .expect_run_err_contains_any(&[
        "cannot assign value of type 'Option<Other>' to field 'peer' of type 'Option<Node>'",
        "not compatible with Option<Node>",
    ]);
}

#[test]
fn typed_object_option_field_rejects_reference_assignment() {
    ShapeTest::new(
        r#"
        type Node { value: int, peer: Option<Node> }
        function test() {
            let b = Node { value: 2, peer: None }
            let mut a = Node { value: 1, peer: None }
            a.peer = &b
            1
        }
        test()
    "#,
    )
    .expect_run_err_contains_any(&[
        "Option<Node> is not compatible with &Node",
        "cannot store a reference in an object or struct literal",
    ]);
}
