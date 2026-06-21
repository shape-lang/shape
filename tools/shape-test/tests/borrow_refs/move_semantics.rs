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

// =============================================================================
// H1 — UNANNOTATED fn-return-sourced moves (strict REAL-MOVE close, 2026-06-21)
//
// `let p = mk()` where `mk` has a HEAP return type classifies `p` NonCopy via
// the compiler's `fn_return_types` seed (built from the type-checked function
// registry), even WITHOUT a binding annotation. A later `let q = p; <read p>`
// then moves and fires B0005. The MIR layer does not run inference; the seed is
// how the already-type-checked return type reaches the binding-classification
// site.
// =============================================================================

#[test]
fn use_after_move_unannotated_fn_return_string_is_compile_error() {
    // `fn mk()->string` — heap string return; unannotated `let p = mk()`.
    ShapeTest::new(
        r#"
        fn mk() -> string { "hi" }
        let p = mk()
        let q = p
        print(p)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn use_after_move_unannotated_fn_return_struct_is_compile_error() {
    // `fn mk()->P` — user-struct (heap) return; unannotated `let p = mk()`.
    // The struct return name `P` resolves via the unknown-named-type → NonCopy
    // arm of `classify_return_annotation` (NOT a generic param).
    ShapeTest::new(
        r#"
        type P { x: int }
        fn mk() -> P { P { x: 1 } }
        let p = mk()
        let q = p
        print(p.x)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn scalar_unannotated_fn_return_rebind_stays_copy() {
    // `fn mk()->int` — scalar return classifies the bind Copy; `let q = p` then
    // `print(p + q)` must NOT be a move (no spurious B0005).
    ShapeTest::new(
        r#"
        fn mk() -> int { 42 }
        let p = mk()
        let q = p
        print(p + q)
    "#,
    )
    .expect_output_contains("84");
}

#[test]
fn generic_fn_return_scalar_instantiation_not_flipped() {
    // A generic return `fn id<T>(x: T) -> T` is NOT seeded NonCopy (a generic
    // could instantiate to a scalar). The scalar instantiation `let n = id(5)`
    // stays Copy and a later rebind+read must not false-flip.
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { x }
        let n = id(5)
        let m = n
        print(n + m)
    "#,
    )
    .expect_output_contains("10");
}

// =============================================================================
// R1 — genuinely UNANNOTATED fn-return-sourced moves (strict REAL-MOVE close,
// 2026-06-21). Distinct from H1: there the FUNCTION carries a `-> string` /
// `-> P` return annotation (only the *binding* is unannotated). Here the
// function itself has NO return annotation — the type-checker INFERS the return
// type, and `build_fn_return_type_seed` threads that inferred hint
// (`type_tracker.function_return_types`) into the binding-classification site.
// A heap inferred return (string / struct / Array) classifies the bind NonCopy
// → MOVE → B0005; a scalar inferred return stays Copy.
//
// Was the last binding-move false-green: `fn make() { let s="hello"; s }
// let a=make(); let b=a; print(a)` RAN (a moved into b, not caught) because the
// MIR lowering left the unannotated-fn-return bind `Unknown`.
// =============================================================================

#[test]
fn use_after_move_inferred_string_fn_return_is_compile_error() {
    // No `-> T` on `make`; return type `string` is INFERRED. Heap → NonCopy.
    ShapeTest::new(
        r#"
        fn make() { let s = "hello"; s }
        let a = make()
        let b = a
        print(a)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn use_after_move_inferred_struct_fn_return_is_compile_error() {
    // No `-> T` on `mk`; return type `P` (user struct, heap) is INFERRED.
    ShapeTest::new(
        r#"
        type P { x: int }
        fn mk() { let p = P { x: 1 }; p }
        let a = mk()
        let b = a
        print(a.x)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn use_after_move_inferred_array_fn_return_is_compile_error() {
    // No `-> T` on `mka`; return type `Array<int>` is INFERRED. Heap → NonCopy.
    ShapeTest::new(
        r#"
        fn mka() { let a = [1, 2, 3]; a }
        let a = mka()
        let b = a
        print(a)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn scalar_inferred_fn_return_rebind_stays_copy() {
    // No `-> T` on `n`; return type `int` is INFERRED. Scalar → Copy, no move:
    // `let b = a` then `print(a + b)` must NOT raise B0005.
    ShapeTest::new(
        r#"
        fn n() { let x = 5; x }
        let a = n()
        let b = a
        print(a + b)
    "#,
    )
    .expect_output_contains("10");
}

#[test]
fn single_move_of_inferred_string_fn_return_runs() {
    // A LEGIT single move of an unannotated-inferred heap return: `let b = a`
    // then reading `b` (not `a`) must run — the move is not a use-after-move.
    ShapeTest::new(
        r#"
        fn make() { let s = "hello"; s }
        let a = make()
        let b = a
        print(b)
    "#,
    )
    .expect_output_contains("hello");
}

// =============================================================================
// H2 — cross-block / nested-scope moves (strict REAL-MOVE close, 2026-06-21)
//
// A move consuming an OUTER binding inside a nested block marks the outer
// binding moved at block exit. The move-error dataflow merges predecessor
// out-states by MAY-MOVE union (a value moved on ANY path is moved at the
// join), so the nested-block move propagates out to the outer read. The prior
// must-move intersection dropped it (false-green).
// =============================================================================

#[test]
fn use_after_move_array_in_nested_block_outer_read_is_compile_error() {
    // `let q = p` inside the `if` block moves the OUTER `p`; `print(p)` after
    // the block reads moved-from.
    ShapeTest::new(
        r#"
        let p = [1, 2, 3]
        if true { let q = p }
        print(p)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn use_after_move_struct_in_nested_block_outer_read_is_compile_error() {
    ShapeTest::new(
        r#"
        type P { x: int }
        let p = P { x: 1 }
        if true { let q = p }
        print(p.x)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn scalar_in_nested_block_outer_read_stays_copy() {
    // A SCALAR consumed by a nested-block rebind is COPIED, not moved — the
    // outer read must still work (no spurious cross-block B0005).
    ShapeTest::new(
        r#"
        let x = 5
        if true { let y = x }
        print(x + 1)
    "#,
    )
    .expect_output_contains("6");
}

#[test]
fn read_on_both_branches_no_move_no_false_error() {
    // Reading (not moving) `p` on both branches is a BORROW on each path; the
    // may-move union must not turn a non-consuming read into a move.
    ShapeTest::new(
        r#"
        let p = [1, 2, 3]
        if true { print(p) } else { print(p) }
        print(p)
    "#,
    )
    .expect_output_contains("[1, 2, 3]");
}

// =============================================================================
// var SMART-DEFAULT — AUTO-CLONE on still-live source (user 2026-06-21 reconcile)
//
// `let` / `let mut` are explicit MOVE bindings (B0005 on use-after-move).
// `var` is the ergonomic smart-default: it AUTO-CLONES on a STILL-LIVE source
// (clone-on-still-live / CoW), so `var copy = data; print(data)` keeps BOTH.
// The discriminator is the DESTINATION binding kind: a `var` destination keeps
// the non-consuming Clone; a `let` / `let mut` destination gets the REAL-MOVE
// flip. See `solver::compute_ownership_decisions` (`dest_is_var` gate) +
// `MirFunction.var_binding_slots` populated at `lower_var_decl`.
// =============================================================================

#[test]
fn var_array_autoclones_source_stays_usable() {
    // `var copy = data` on a still-live `data` AUTO-CLONES — both stay usable,
    // no B0005 (the documented `var` clone-on-still-live behavior).
    ShapeTest::new(
        r#"
        let data = [1, 2, 3]
        var copy = data
        print(data.len())
        print(copy.len())
    "#,
    )
    .expect_output_contains("3\n3");
}

#[test]
fn var_struct_autoclones_source_stays_usable() {
    ShapeTest::new(
        r#"
        type P { x: int }
        let p = P { x: 7 }
        var q = p
        print(p.x)
        print(q.x)
    "#,
    )
    .expect_output_contains("7\n7");
}

#[test]
fn var_string_autoclones_source_stays_usable() {
    ShapeTest::new(
        r#"
        let s = "hello"
        var t = s
        print(s)
        print(t)
    "#,
    )
    .expect_output_contains("hello\nhello");
}

#[test]
fn let_move_still_fires_b0005_while_var_autoclones() {
    // CONTRAST: the identical rebind under `let` MOVES and a still-live read of
    // the moved-from source is B0005 — `var` does NOT, `let` does.
    ShapeTest::new(
        r#"
        let p = [1, 2, 3]
        let q = p
        print(p)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn let_mut_move_still_fires_b0005() {
    // `let mut` is ALSO an explicit-move binding (not a `var` smart-default):
    // the moved-from source's later read is B0005.
    ShapeTest::new(
        r#"
        let p = [1, 2, 3]
        let mut q = p
        print(p)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn var_source_dead_after_still_runs() {
    // When the `var copy = data` source is DEAD afterward (the move-when-dead
    // half of the smart default), the program still runs — `copy` owns it.
    ShapeTest::new(
        r#"
        let data = [1, 2, 3]
        var copy = data
        print(copy.len())
    "#,
    )
    .expect_output_contains("3");
}

#[test]
fn var_scalar_stays_copy_both_usable() {
    // Scalars are Copy for `var` too — no move, both usable.
    ShapeTest::new(
        r#"
        let x = 5
        var y = x
        print(x)
        print(y)
    "#,
    )
    .expect_output_contains("5\n5");
}
