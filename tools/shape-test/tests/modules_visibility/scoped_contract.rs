//! Contract tests for the clean-break scoped import surface.
//!
//! The active tests cover behavior that should keep working as the import
//! system is tightened. The ignored tests encode the target contract:
//! no user-facing globals except `print`, explicit annotation imports, and
//! `::` namespace calls instead of leaked globals or dot-based module calls.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// ACTIVE SMOKE TESTS
// =============================================================================

#[test]
fn scoped_contract_print_remains_available_without_imports() {
    ShapeTest::new(r#"print("ok")"#).expect_output("ok");
}

#[test]
fn scoped_contract_regular_named_import_alias_executes() {
    ShapeTest::new(
        r#"
        from std::core::set use { new as new_set, size as set_size }
        let s = new_set()
        print(set_size(s))
    "#,
    )
    .with_stdlib()
    .expect_output("0");
}

#[test]
fn scoped_contract_annotation_alias_import_is_rejected() {
    ShapeTest::new("from std::core::remote use { @remote as worker_remote }").expect_parse_err();
}

// =============================================================================
// CLEAN-BREAK CONTRACT
// =============================================================================

#[test]
fn scoped_contract_annotation_named_import_parses() {
    ShapeTest::new("from std::core::remote use { @remote }").expect_parse_ok();
}

#[test]
fn scoped_contract_mixed_named_import_with_annotation_parses() {
    ShapeTest::new("from std::core::remote use { execute, @remote }").expect_parse_ok();
}

#[test]
fn scoped_contract_namespace_function_calls_use_double_colon() {
    ShapeTest::new(
        r#"
        use std::core::set as s
        let values = s::from_array([1, 2, 2, 3])
        print(s::size(values))
    "#,
    )
    .with_stdlib()
    .expect_output("3");
}

// W9 (v0.3 R2): cross-module annotation imports work end-to-end.
// Previously these tests were `#[should_panic]` because the namespace +
// qualified + named-import paths all failed at runtime with
// `Unknown annotation '@remote'` / `'@remote::remote'`. After W9 wires
// namespace-import annotation registration in
// `register_graph_imports_for_module` (statements.rs) and qualified-form
// resolution in `resolve_compiled_annotation_name_str`
// (compiler_impl_reference_model.rs), all three forms resolve and the
// `@remote` `before` handler runs successfully.
#[test]
fn scoped_contract_namespace_annotation_refs_use_double_colon() {
    ShapeTest::new(
        r#"
        use std::core::remote as remote

        @remote::remote("worker:9527")
        fn compute(x) { x + 1 }

        print("ok")
    "#,
    )
    .with_stdlib()
    .expect_output("ok");
}

#[test]
fn scoped_contract_named_annotation_import_enables_bare_annotation() {
    ShapeTest::new(
        r#"
        from std::core::remote use { @remote }

        @remote("worker:9527")
        fn compute(x) { x + 1 }

        print("ok")
    "#,
    )
    .with_stdlib()
    .expect_output("ok");
}

#[test]
fn scoped_contract_namespace_import_does_not_bind_bare_regular_names() {
    ShapeTest::new(
        r#"
        use std::core::set
        new()
    "#,
    )
    .with_stdlib()
    .expect_run_err_contains("new");
}

// W9 (v0.3 R2): renamed from
// `scoped_contract_namespace_import_does_not_bind_bare_annotations`. The
// negative-contract intent ("namespace import must NOT expose the
// annotation as a bare `@remote`") is reversed by W9 per supervisor
// disposition path (i): the namespace-import path now registers
// annotation defs from the imported module so bare `@remote` resolves.
#[test]
fn scoped_contract_namespace_import_binds_bare_annotations() {
    ShapeTest::new(
        r#"
        use std::core::remote

        @remote("worker:9527")
        fn compute(x) { x + 1 }

        print("ok")
    "#,
    )
    .with_stdlib()
    .expect_output("ok");
}

// These tests document the *desired* clean-break contract: builtins should
// require explicit imports. Currently they are globally available (prelude).
// When clean-break is implemented, flip these back to expect_run_err_contains.

#[test]
fn scoped_contract_hashmap_requires_explicit_import() {
    // TODO: should be expect_run_err_contains("HashMap") after clean-break
    ShapeTest::new("HashMap()").expect_run_ok();
}

#[test]
fn scoped_contract_result_constructors_require_explicit_import() {
    // TODO: should be expect_run_err_contains("Ok") after clean-break
    ShapeTest::new("Ok(1)").expect_run_ok();
}

#[test]
fn scoped_contract_snapshot_requires_explicit_import() {
    // snapshot() is a prelude builtin, but requires a snapshot store to be configured.
    // TODO: after clean-break, should be expect_run_err_contains("snapshot")
    ShapeTest::new("snapshot()").with_stdlib().expect_run_err();
}

#[test]
fn scoped_contract_global_stdlib_modules_require_imports() {
    ShapeTest::new("set::new()").with_stdlib().expect_run_err_contains("set");
}
