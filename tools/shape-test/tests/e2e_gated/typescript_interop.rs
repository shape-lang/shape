//! Compile-side checks for the `fn typescript` foreign vertical.
//!
//! Requires: `cargo test -p shape-test --features e2e-typescript`
//!
//! The real end-to-end EXECUTION tests (a `fn typescript` body actually
//! running through deno_core/V8 via the loaded extension, with scalar/container
//! args and the throw error channel) live in `bin/shape-cli/tests/ffi_e2e.rs`
//! and run in the `just test-ffi` CI tier — `ShapeTest` here drives the
//! in-process engine, which does NOT load the language-runtime `.so`, so it can
//! only validate the compile side (which needs no runtime).
//!
//! History: these tests previously asserted `fn typescript greet(...) ->
//! string` executing to a value. Both halves were broken (2026-07-04 audit):
//! the `-> string` signature is compiler-REJECTED (dynamic runtimes must
//! declare `Result<T>`, §3.6), and the execution assertion could never pass
//! in-process without a loaded runtime. They are rebuilt below against the
//! correct signature.

use shape_test::shape_test::ShapeTest;

/// A correctly-signed `fn typescript` declaration (`-> Result<int>`) compiles
/// and is non-fatal when never called (lazy linking, ffi-rebuild §4.2).
#[cfg(feature = "e2e-typescript")]
#[test]
fn typescript_result_signature_declaration_is_non_fatal() {
    ShapeTest::new(
        r#"
        fn typescript tadd(a: int, b: int) -> Result<int> {
            return a + b;
        }
        print("DECLARED")
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output("DECLARED");
}

/// The Result mandate is enforced at compile time for TypeScript too: a bare
/// `-> string` (the pre-audit signature) is rejected before any runtime is
/// consulted.
#[cfg(feature = "e2e-typescript")]
#[test]
fn typescript_dynamic_return_must_be_result() {
    ShapeTest::new(
        r#"
        fn typescript greet(name: string) -> string {
            return name;
        }
        print("unreachable")
    "#,
    )
    .with_stdlib()
    .expect_run_err();
}
