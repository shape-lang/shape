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

// == LSP-N §D regression flow #7: prepareCallHierarchy returns [] =============
//
// Audit `v0.3-lsp-parity-audit.md` executive summary item #7: returns `[]`
// for a visible user fn — the entire call-hierarchy chain is dead in the
// editor. Currently red; LSP-I closes.

#[test]
fn lsp_n_prepare_call_hierarchy_on_visible_free_fn() {
    // §D #7 (free-fn leg): cursor on `hello` definition must surface a
    // CallHierarchyItem. PASSES today at HEAD 7813a652 — the
    // `Item::Function` arm in `call_hierarchy.rs:40` is wired.
    // Regression-prevention coverage.
    let code = "\
fn hello(name: string) -> string { name }
fn main() { hello(\"world\"); }
";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_call_hierarchy_prepare_ok();
}

#[test]
fn lsp_n_prepare_call_hierarchy_on_impl_method() {
    // §D #7 (impl-method leg): the §D regression was specifically
    // `fn hello (impl)` returning `[]` — i.e. the call_hierarchy dispatch
    // only matched `Item::Function` and `Item::ForeignFunction`; impl-block
    // methods were silently dropped. LSP-CH (r8w12-lsp-ch) extended
    // `prepare_call_hierarchy` with arms for `Item::Impl`, `Item::StructType`,
    // `Item::Extend`, and `Item::Trait` default methods.
    //
    // NB: the grammar requires `impl Trait for Type { ... }`; inherent
    // `impl Type { ... }` syntax does not parse (see
    // `crates/shape-ast/src/shape.pest:224` — `impl_block` requires `for`).
    // Test fixture uses the trait+impl form which exercises the same
    // dispatch-arm regression.
    let code = "\
type User { name: string }
trait Greet { fn hello(self) -> string; }
impl Greet for User {
    fn hello(self) -> string { self.name }
}
";
    ShapeTest::new(code)
        .at(pos(3, 7))
        .expect_call_hierarchy_prepare_ok();
}

#[test]
fn lsp_n_prepare_call_hierarchy_on_whitespace_is_empty() {
    // Positive-coverage regression: cursor on blank line must return empty.
    let code = "\
fn hello(name: string) -> string { name }

fn main() { hello(\"world\"); }
";
    ShapeTest::new(code)
        .at(pos(1, 0))
        .expect_call_hierarchy_prepare_empty();
}
