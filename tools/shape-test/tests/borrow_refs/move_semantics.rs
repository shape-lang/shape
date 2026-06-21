use shape_test::shape_test::ShapeTest;

// =============================================================================
// Strict REAL-MOVE semantics (user 2026-06-21)
//
// THE MODEL (Rust-style): `let q = p` MOVES a HEAP value `p`; reading the
// moved-from `p` afterward is a COMPILE-TIME use-after-move (B0005). SCALARS
// (int/number/bool) STAY COPY — they never move.
//
// Implementation: `solver::compute_ownership_decisions` flips the NonCopy
// still-live arm from Clone to Move; `solver::actual_move_places` enters the
// moved source into the moved-set (Move-operand directly, Copy-operand when
// the bind destination is a real user binding); `compute_use_after_move_errors`
// raises B0005. See `crates/shape-vm/src/mir/solver.rs`.
// =============================================================================

// --- Move-OK: the new owner is used, the moved-from source is NOT re-read ----

#[test]
fn move_struct_into_new_binding_then_use_new() {
    // `let q = p` moves the struct into `q`; `p` is never read again → OK.
    ShapeTest::new(
        r#"
        type P { x: int }
        let p = P { x: 1 }
        let q = p
        print(q.x)
    "#,
    )
    .expect_output_contains("1");
}

#[test]
fn move_array_into_new_binding_then_use_new() {
    ShapeTest::new(
        r#"
        let a = [10, 20, 30]
        let b = a
        print(b[1])
    "#,
    )
    .expect_output_contains("20");
}

// --- Use-after-move: reading the moved-from source is a compile error --------

#[test]
fn use_after_move_struct_is_compile_error() {
    // `let mut q = p` moves the struct; `print(p.x)` then reads moved-from `p`.
    ShapeTest::new(
        r#"
        type P { x: int }
        let p = P { x: 1 }
        let mut q = p
        q.x = 99
        print(p.x)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn use_after_move_array_is_compile_error() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        let b = a
        print(a[0])
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

// --- Scalar Copy: scalars NEVER move — both bindings stay usable -------------

#[test]
fn scalar_int_stays_copy_both_usable() {
    // `let y = x` COPIES the int; reading `x` afterward is NOT an error.
    ShapeTest::new(
        r#"
        let x = 5
        let y = x
        print(x)
        print(y)
    "#,
    )
    .expect_output_contains("5\n5");
}

#[test]
fn scalar_loop_var_copy_into_local_no_move() {
    // A loop variable is a scalar; `let v = i` must NOT move it — `sum + v`
    // (and the loop-back read of `i`) must keep working.
    ShapeTest::new(
        r#"
        fn f() {
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                let v = i
                sum = sum + v
            }
            return sum
        }
        f()
    "#,
    )
    .expect_number(15.0);
}

// --- Non-consuming reads: borrowing reads do NOT count as a move ------------

#[test]
fn repeated_index_reads_do_not_move_array() {
    // Reading elements via `a[i]` is a borrowing read, not a move.
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        print(a[0])
        print(a[1])
        print(a[2])
    "#,
    )
    .expect_output_contains("1\n2\n3");
}

#[test]
fn fstring_read_does_not_move_binding() {
    // The `f"{s}"` interpolation reads `s` to build a derived string — this is
    // NON-consuming; a later `print(s)` must still work (no spurious B0005).
    ShapeTest::new(
        r#"
        let s = "hi"
        print(f"{s}")
        print(s)
    "#,
    )
    .expect_output_contains("hi\nhi");
}

// --- Identifier-sourced moves (CloseFalseGreen 2026-06-21) -------------------
//
// A `let p = a` bind whose initializer is a bare IDENTIFIER (not a heap
// literal) used to type `p` as `LocalTypeInfo::Unknown` — so a transitive
// `let q = p` was kept as a non-consuming Clone and reading the moved-from `p`
// was SILENTLY NOT caught (rc=0). `infer_local_type_from_expr_with_builder`
// now propagates the source binding's classification, so an identifier-sourced
// HEAP rebind moves (B0005) while a SCALAR identifier-sourced rebind stays
// Copy. See `crates/shape-vm/src/mir/lowering/helpers.rs`.

#[test]
fn use_after_move_array_identifier_sourced_is_compile_error() {
    // `let p = a` (identifier source) then `let q = p` moves the array out of
    // `p`; `print(p)` reads moved-from `p`.
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        let p = a
        let q = p
        print(p)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn use_after_move_struct_identifier_sourced_is_compile_error() {
    ShapeTest::new(
        r#"
        type P { x: int }
        let a = P { x: 1 }
        let p = a
        let q = p
        print(p.x)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn use_after_move_string_identifier_sourced_is_compile_error() {
    ShapeTest::new(
        r#"
        let s = "hello"
        let p = s
        let q = p
        print(p)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn use_after_move_annotated_fn_return_heap_is_compile_error() {
    // An annotated heap-returning bind classifies the slot NonCopy even though
    // the fn-call initializer is not a literal — `let q = p` moves, `print(p)`
    // reads moved-from.
    ShapeTest::new(
        r#"
        fn foo() -> Array<int> { [1, 2, 3] }
        let p: Array<int> = foo()
        let q = p
        print(p)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn scalar_identifier_sourced_rebind_stays_copy() {
    // `let p = x` (identifier source, SCALAR) must NOT move — a transitive
    // `let q = p` then `print(p)` must still work (no spurious B0005).
    ShapeTest::new(
        r#"
        let x = 5
        let p = x
        let q = p
        print(p)
    "#,
    )
    .expect_output_contains("5");
}

#[test]
fn scalar_identifier_sourced_rebind_reused_no_false_move() {
    // The scalar source `x` flows through two identifier rebinds and is still
    // read at the end — none of these are moves.
    ShapeTest::new(
        r#"
        let x = 10
        let a = x
        let b = a
        print(a)
        print(b)
        print(x)
    "#,
    )
    .expect_output_contains("10\n10\n10");
}
