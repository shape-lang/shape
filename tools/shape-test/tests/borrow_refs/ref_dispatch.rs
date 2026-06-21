use shape_test::shape_test::ShapeTest;

// =============================================================================
// Method & index dispatch through a first-class reference (v0.3.3 RefDispatch).
//
// `let r = &a; r.len()` / `r[0]` dispatch the method / index access THROUGH the
// reference, mirroring the already-auto-derefing field access (`r.x`) and the
// operator auto-deref (`r + 1`). Before the fix:
//   - `r.len()` on `&Array<T>` rejected with "Array cannot have fields"
//     (method-call receiver resolution did not unwrap `Borrow{inner}`).
//   - `r[0]` rejected with "Borrow does not support index access"
//     (the `Indexable` constraint had no `Borrow{inner}` deref arm and
//     `infer_index_access` fell to the `_` wildcard).
// The referent type is forwarded verbatim — no int<->number coercion, the
// element family is preserved (CLAUDE.md §Type-System-Rules).
// =============================================================================

#[test]
fn ref_array_method_len_auto_derefs() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        let r = &a
        r.len()
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn ref_array_index_auto_derefs() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        let r = &a
        r[0]
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn ref_array_index_preserves_element_family() {
    // The element type flows through verbatim: an `int` array yields an `int`
    // element, usable in `int` arithmetic without coercion.
    ShapeTest::new(
        r#"
        fn test() -> int {
            let a = [10, 20, 30]
            let r = &a
            return r[1] + 5
        }
        test()
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn ref_string_length_property_auto_derefs() {
    // `.length` is a property, not a method — the property-access auto-deref
    // path. `&string` derefs to `string`.
    ShapeTest::new(
        r#"
        let s = "hi"
        let r = &s
        r.length
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn ref_string_method_len_auto_derefs() {
    ShapeTest::new(
        r#"
        let s = "hello"
        let r = &s
        r.len()
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn ref_mut_array_method_len_parity() {
    // `&mut` parity: an exclusive reference dispatches methods identically.
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut a = [1, 2, 3, 4]
            let r = &mut a
            return r.len()
        }
        test()
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn ref_mut_array_index_parity() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut a = [7, 8, 9]
            let r = &mut a
            return r[2]
        }
        test()
    "#,
    )
    .expect_number(9.0);
}

#[test]
fn ref_value_read_arithmetic_still_works() {
    // Regression guard: the operator auto-deref (`r + 1`) the new method/index
    // arms sit alongside must keep working on the same `&int` receiver shape.
    ShapeTest::new(
        r#"
        let x = 5
        let r = &x
        r + 1
    "#,
    )
    .expect_number(6.0);
}
