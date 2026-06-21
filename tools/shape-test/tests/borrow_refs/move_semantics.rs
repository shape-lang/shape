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
// STAGE T2 — call-argument moves (full Rust-style; user 2026-06-21)
//
// A by-value (non-`&`) HEAP call-argument MOVES the source binding: an
// ownership-TAKING user fn `fn consume(p: P)` consumes its arg, so the caller
// cannot reuse it (B0005). EXCEPTIONS that BORROW (no move):
//   - read-only builtins (print / format / f-string / assert / range);
//   - method receivers/args (.len / .get / .map / ...);
//   - explicit `&p` / `&mut p` params (lower to a borrow temp, never the
//     binding) — `&mut p` is the loan-back path: caller-VISIBLE mutation.
//   - SCALARS stay Copy.
//
// Implementation: `solver::terminator_moved_arg_places` marks by-value heap
// Call-terminator args as moved unless `callee_borrows_all_args` /
// `arg_index_is_borrowed` (the `BorrowingParams` map = the EXPLICIT-`&`
// `inferred_ref_params`).
// =============================================================================
//
// CallArgConsume (user 2026-06-21): the WS-7 mutation-share-by-value
// convention is REVERSED. A by-value (non-`&`) heap param that MUTATES IN
// PLACE (`fn fill(arr){arr[i]=v}`) now CONSUMES its arg — caller-VISIBLE
// mutation requires an explicit `&mut p` param. See
// `borrowing_params_for_move_analysis`.

#[test]
fn call_arg_struct_move_then_use_is_compile_error() {
    // `consume(x)` takes ownership of the struct; `print(x.x)` reads moved-from.
    ShapeTest::new(
        r#"
        type P { x: int }
        fn consume(p: P) {}
        let x = P { x: 1 }
        consume(x)
        print(x.x)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn call_arg_array_move_then_index_is_compile_error() {
    ShapeTest::new(
        r#"
        fn consume(a: Array<int>) {}
        let arr = [1, 2, 3]
        consume(arr)
        print(arr[0])
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn call_arg_array_move_then_method_is_compile_error() {
    ShapeTest::new(
        r#"
        fn consume(a: Array<int>) {}
        let arr = [1, 2, 3]
        consume(arr)
        print(arr.len())
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn call_arg_two_heap_args_first_moved_then_read_is_compile_error() {
    ShapeTest::new(
        r#"
        type P { x: int }
        fn consume2(a: P, b: P) {}
        let p = P { x: 1 }
        let q = P { x: 2 }
        consume2(p, q)
        print(p.x)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn print_borrows_arg_reusable() {
    // print is a read-only builtin — it BORROWS, so the same value prints twice.
    ShapeTest::new(
        r#"
        let s = "hello"
        print(s)
        print(s)
    "#,
    )
    .expect_output_contains("hello\nhello");
}

#[test]
fn format_borrows_arg_reusable() {
    ShapeTest::new(
        r#"
        let s = "x"
        let m = f"v={s}"
        print(s)
        print(m)
    "#,
    )
    .expect_output_contains("x");
}

#[test]
fn method_receiver_borrows_reusable() {
    // `.len()` / `.map()` borrow the receiver — `arr` stays usable.
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let n = arr.len()
        let b = arr.map(|e| e + 1)
        print(arr[0])
        print(n)
    "#,
    )
    .expect_output_contains("1\n3");
}

#[test]
fn clone_keeps_source_across_consuming_call() {
    // `clone x` produces an independent value to consume; `x` stays live.
    ShapeTest::new(
        r#"
        type P { x: int }
        fn consume(p: P) {}
        let x = P { x: 1 }
        let c = clone x
        consume(c)
        print(x.x)
    "#,
    )
    .expect_output_contains("1");
}

#[test]
fn scalar_arg_stays_copy_across_calls() {
    // SCALAR args are Copy — passing `a` to two consuming calls is fine.
    ShapeTest::new(
        r#"
        fn dbl(n: int) -> int { n + n }
        let a = 4
        let r1 = dbl(a)
        let r2 = dbl(a)
        print(r1 + r2)
    "#,
    )
    .expect_output_contains("16");
}

#[test]
fn fn_returning_moved_arg_is_ok() {
    // A fn that returns its moved arg; the caller binds the returned owner.
    ShapeTest::new(
        r#"
        type P { x: int }
        fn passthru(p: P) -> P { p }
        let x = P { x: 7 }
        let y = passthru(x)
        print(y.x)
    "#,
    )
    .expect_output_contains("7");
}

#[test]
fn explicit_ref_arg_does_not_move() {
    // An explicit `&arr` reference param BORROWS — the binding stays usable.
    ShapeTest::new(
        r#"
        fn read_first(&arr) { arr[0] }
        let xs = [9]
        let a = read_first(&xs)
        print(xs[0])
    "#,
    )
    .expect_output_contains("9");
}

#[test]
fn in_place_mutating_by_value_param_consumes_then_reuse_is_compile_error() {
    // CallArgConsume reversal: a by-value (non-`&`) heap param that mutates in
    // place CONSUMES its arg. `fill(xs)` moves `xs`; reusing `xs[0]` after is
    // B0005. Caller-visible mutation now requires `&mut` (next test).
    ShapeTest::new(
        r#"
        fn fill(arr, val) {
            let mut i = 0
            while i < arr.len() {
                arr[i] = val
                i = i + 1
            }
        }
        let xs = [0, 0, 0, 0]
        fill(xs, 7)
        print(xs[0])
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn mut_ref_param_fill_is_caller_visible() {
    // The loan-back path: an explicit `&mut Array<int>` param BORROWS and its
    // in-place mutation is VISIBLE to the caller. `xs` stays usable and sees
    // the writes.
    ShapeTest::new(
        r#"
        fn fill(arr: &mut Array<int>, val: int) {
            let mut i = 0
            while i < arr.len() {
                arr[i] = val
                i = i + 1
            }
        }
        let mut xs = [0, 0, 0, 0]
        fill(&mut xs, 7)
        print(xs[0] + xs[1] + xs[2] + xs[3])
    "#,
    )
    .expect_output_contains("28");
}

#[test]
fn mut_ref_param_struct_field_assign_is_caller_visible() {
    // `fn modify(p: &mut P){p.x=9}` — caller-visible mutation through `&mut`.
    ShapeTest::new(
        r#"
        type P { x: int }
        fn modify(p: &mut P) { p.x = 9 }
        let mut x = P { x: 1 }
        modify(&mut x)
        print(x.x)
    "#,
    )
    .expect_output_contains("9");
}

#[test]
fn in_place_mutating_method_by_value_param_consumes_then_reuse_is_compile_error() {
    // A by-value param mutated via a mutating METHOD (`.push`) also CONSUMES.
    ShapeTest::new(
        r#"
        fn grow(arr: Array<int>) { arr.push(9) }
        let xs = [1, 2]
        grow(xs)
        print(xs.len())
    "#,
    )
    .expect_run_err_contains("after it was moved");
}

#[test]
fn fn_returning_moved_arg_threads_ownership_out() {
    // A consuming fn that THREADS its moved arg out as the return value: the
    // caller rebinds the returned owner and reads it; the original `x` is
    // moved (reading it would be B0005, exercised elsewhere).
    ShapeTest::new(
        r#"
        type P { x: int }
        fn passthru(p: P) -> P { p }
        let x = P { x: 7 }
        let y = passthru(x)
        print(y.x)
    "#,
    )
    .expect_output_contains("7");
}

#[test]
fn fn_returning_moved_arg_then_use_original_is_compile_error() {
    // After `let y = passthru(x)` threads `x` out, reading the moved-from `x`
    // is a use-after-move.
    ShapeTest::new(
        r#"
        type P { x: int }
        fn passthru(p: P) -> P { p }
        let x = P { x: 7 }
        let y = passthru(x)
        print(x.x)
    "#,
    )
    .expect_run_err_contains("after it was moved");
}
