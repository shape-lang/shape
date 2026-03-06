//! LSP call hierarchy tests: incoming and outgoing call tracking.
//! TDD: ShapeTest does not expose call_hierarchy() — tests use go-to-definition
//! and find-references as proxies for basic call tracking.

use shape_test::shape_test::{ShapeTest, pos};

// == Incoming calls (proxy via find-references on definition) =================

#[test]
fn incoming_calls_function_called_from_multiple_sites() {
    // TDD: call_hierarchy not exposed; proxy via find-references on the definition
    let code = "\
function helper() { return 1; }
let a = helper();
let b = helper();
let c = helper();
";
    ShapeTest::new(code).at(pos(0, 10)).expect_references_min(3);
}

#[test]
fn incoming_calls_nested_function_calls() {
    // TDD: call_hierarchy not exposed; proxy via find-references
    let code = "\
function inner() { return 42; }
function outer() { return inner(); }
let x = outer();
";
    ShapeTest::new(code).at(pos(0, 10)).expect_references_min(2);
}

#[test]
fn incoming_calls_function_used_as_callback() {
    // TDD: call_hierarchy not exposed; proxy via find-references
    let code = "\
function transform(x) { return x * 2; }
let arr = [1, 2, 3];
let result = arr.map(transform);
";
    ShapeTest::new(code).at(pos(0, 10)).expect_references_min(2);
}

// == Outgoing calls (proxy via go-to-definition from call site) ===============

#[test]
fn outgoing_calls_goto_def_from_call_site() {
    // TDD: call_hierarchy not exposed; proxy via go-to-definition from call site
    let code = "\
function add(a, b) { return a + b; }
function mul(a, b) { return a * b; }
function combined(x, y) { return add(x, y) + mul(x, y); }
";
    ShapeTest::new(code).at(pos(2, 35)).expect_definition();
}

#[test]
fn outgoing_calls_goto_def_second_callee() {
    let code = "\
function add(a, b) { return a + b; }
function mul(a, b) { return a * b; }
function combined(x, y) { return add(x, y) + mul(x, y); }
";
    // "mul" starts at column 46 in "... + mul(x, y)"
    ShapeTest::new(code).at(pos(2, 46)).expect_definition();
}
