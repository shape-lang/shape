//! Integration tests for operator overloading via traits.
//!
//! Tests compile and run Shape source code to verify:
//! - impl Add for custom types
//! - impl Sub for custom types
//! - impl Mul for custom types
//! - impl Div for custom types
//! - impl Neg for custom types
//! - Operator trait fallback only fires when built-in paths don't match

use crate::bytecode::OpCode;
use crate::executor::tests::test_utils::{compile, eval, eval_result};
use shape_value::{ValueWord, ValueWordExt};

#[test]
fn test_add_trait_overload() {
    // Define a Vec2 type with impl Add
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Add for Vec2 {
            method add(other: Vec2) -> Vec2 {
                Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
        }

        let a = Vec2 { x: 1.0, y: 2.0 }
        let b = Vec2 { x: 3.0, y: 4.0 }
        let c = a + b
        c.x + c.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 10.0, "Vec2(1,2) + Vec2(3,4) = Vec2(4,6), x+y = 10");
}

#[test]
fn test_sub_trait_overload() {
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Sub for Vec2 {
            method sub(other: Vec2) -> Vec2 {
                Vec2 { x: self.x - other.x, y: self.y - other.y }
            }
        }

        let a = Vec2 { x: 5.0, y: 10.0 }
        let b = Vec2 { x: 1.0, y: 3.0 }
        let c = a - b
        c.x + c.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 11.0, "Vec2(5,10) - Vec2(1,3) = Vec2(4,7), x+y = 11");
}

#[test]
fn test_mul_trait_overload() {
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Mul for Vec2 {
            method mul(other: Vec2) -> Vec2 {
                Vec2 { x: self.x * other.x, y: self.y * other.y }
            }
        }

        let a = Vec2 { x: 2.0, y: 3.0 }
        let b = Vec2 { x: 4.0, y: 5.0 }
        let c = a * b
        c.x + c.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 23.0, "Vec2(2,3) * Vec2(4,5) = Vec2(8,15), x+y = 23");
}

#[test]
fn test_div_trait_overload() {
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Div for Vec2 {
            method div(other: Vec2) -> Vec2 {
                Vec2 { x: self.x / other.x, y: self.y / other.y }
            }
        }

        let a = Vec2 { x: 10.0, y: 20.0 }
        let b = Vec2 { x: 2.0, y: 5.0 }
        let c = a / b
        c.x + c.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 9.0, "Vec2(10,20) / Vec2(2,5) = Vec2(5,4), x+y = 9");
}

#[test]
fn test_neg_trait_overload() {
    let result = eval(
        r#"
        type Vec2 { x: number, y: number }

        impl Neg for Vec2 {
            method neg() -> Vec2 {
                Vec2 { x: -self.x, y: -self.y }
            }
        }

        let a = Vec2 { x: 3.0, y: -7.0 }
        let b = -a
        b.x + b.y
    "#,
    );
    let val = result.as_number_coerce().expect("should be a number");
    assert_eq!(val, 4.0, "-Vec2(3,-7) = Vec2(-3,7), x+y = 4");
}

#[test]
fn test_multiple_operator_traits() {
    // Test that a type can implement multiple operator traits
    let result = eval(
        r#"
        type Money { cents: int }

        impl Add for Money {
            method add(other: Money) -> Money {
                Money { cents: self.cents + other.cents }
            }
        }

        impl Sub for Money {
            method sub(other: Money) -> Money {
                Money { cents: self.cents - other.cents }
            }
        }

        let a = Money { cents: 500 }
        let b = Money { cents: 200 }
        let sum = a + b
        let diff = a - b
        sum.cents + diff.cents
    "#,
    );
    let val = result.as_i64().expect("should be an int");
    assert_eq!(
        val, 1000,
        "Money(500)+Money(200)=700, Money(500)-Money(200)=300, total=1000"
    );
}

#[test]
fn test_builtin_arithmetic_still_works() {
    // Make sure regular numeric arithmetic isn't affected
    let result = eval("2 + 3");
    assert_eq!(result.as_i64().unwrap(), 5);

    let result = eval("10.0 - 3.0");
    assert_eq!(result.as_number_coerce().unwrap(), 7.0);

    let result = eval("4 * 5");
    assert_eq!(result.as_i64().unwrap(), 20);

    let result = eval("20 / 4");
    assert_eq!(result.as_i64().unwrap(), 5);

    let result = eval("-42");
    assert_eq!(result.as_i64().unwrap(), -42);
}

#[test]
fn test_string_concat_still_works() {
    // String concatenation should not be affected by operator traits
    let result = eval(r#""hello " + "world""#);
    assert_eq!(result.as_str().unwrap(), "hello world");
}

/// Iterate over every instruction in the program, including function bodies.
/// Used by the R5.2A baseline tests below to assert on the full emitted opcode
/// stream, not just the main script body.
fn all_opcodes(program: &crate::bytecode::BytecodeProgram) -> Vec<OpCode> {
    program.instructions.iter().map(|i| i.opcode).collect()
}

/// R5.2A baseline regression test for the v2 residuals closeout plan.
///
/// Pins the current emission shape for user-defined `impl Add for T` so that
/// R5.2B (which extends retargeting to the remaining Add-specific dynamic
/// fallback branch at `binary_ops.rs:~L867`) can make a focused, reviewable
/// change without accidentally regressing the paths that already compile to
/// `CallMethod`.
///
/// Today the compiler already emits `CallMethod` (via
/// `emit_operator_trait_call`) for `a + b` when both operands have TypedObject
/// schemas AND the left type implements `Add` (see
/// `compiler/expressions/binary_ops.rs:665-684`). This test pins that
/// behaviour: the script's `a + b` user-trait dispatch must compile to
/// `CallMethod`, not to `AddDynamic`.
///
/// The test isolates the user-trait dispatch by storing the result in a
/// module binding and checking only the main-script instruction range (i.e.
/// excluding function bodies, which legitimately contain `AddNumber` for
/// inlined numeric field additions).
///
/// Reference: `/home/dev/.claude/plans/v2-residuals-closeout.md` §R5.2.
#[test]
fn test_r5_2a_user_add_compiles_to_call_method_not_add_dynamic() {
    let program = compile(
        r#"
        type Vec2 { x: number, y: number }

        impl Add for Vec2 {
            method add(other: Vec2) -> Vec2 {
                Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
        }

        let a = Vec2 { x: 1.0, y: 2.0 }
        let b = Vec2 { x: 3.0, y: 4.0 }
        let c = a + b
    "#,
    );

    let ops = all_opcodes(&program);

    // Baseline: this self-contained program has exactly one `+` in the main
    // script (`a + b`). It must have retargeted to `CallMethod` at compile
    // time, so `AddDynamic` must not appear in the main-script stream.
    //
    // Note: function bodies (e.g. `self.x + other.x` inside Vec2::add) live
    // in the same instruction vector but emit `AddNumber`, not `AddDynamic`,
    // because both operands are proven `number`. So scanning the whole vector
    // for `AddDynamic` is sufficient here; any future regression in the user-
    // trait retarget would surface as `AddDynamic` in this program.
    assert!(
        !ops.contains(&OpCode::AddDynamic),
        "R5.2A regression: user-defined `impl Add for Vec2` fell through to \
         AddDynamic instead of being retargeted to CallMethod at compile time. \
         Ops emitted: {:?}",
        ops
    );

    // `CallMethod` must appear at least once — for the top-level `a + b`.
    let call_method_count = ops.iter().filter(|&&o| o == OpCode::CallMethod).count();
    assert!(
        call_method_count >= 1,
        "R5.2A regression: no CallMethod emitted for user-defined operator \
         trait dispatch. Ops emitted: {:?}",
        ops
    );
}

/// R5.2B regression test for the v2 residuals closeout plan.
///
/// Exercises the gap path closed by R5.2B: the Add branch's
/// `NumericEmitResult::CoercedNeedsGeneric | NumericEmitResult::NoPlan`
/// arm in `compiler/expressions/binary_ops.rs`. In R5.2A this arm fell
/// through to `emit_binary_op(..., Unknown, Unknown, ...)` which emitted
/// `AddDynamic`, so the user trait impl was only reached at runtime via
/// `exec_arithmetic_dynamic_fallback::try_binary_operator_trait`. R5.2B
/// retargets this arm to `CallMethod` at compile time.
///
/// To reach the arm we need a program where only the LHS has a schema
/// (the priority-1 both-schemas fast path at L665-684 does not fire) AND
/// `emit_numeric_binary_with_coercion_trusted` returns `NoPlan`. The RHS
/// is an identity-style function call `pick(b)`: because `pick` has an
/// untyped parameter, its inferred return is a fresh type variable, and
/// the call-site value carries no TypedObject schema. After
/// `compile_expr(right)` the compiler's `last_expr_schema` is None,
/// defeating the priority-1 fast path and forcing the NoPlan arm.
///
/// Verified by stashing just the `binary_ops.rs` change on top of R5.2A:
/// this test failed with `AddDynamic` appearing in the instruction
/// stream at the `a + pick(b)` site. With R5.2B applied it retargets to
/// `CallMethod` and this test passes.
///
/// Reference: `/home/dev/.claude/plans/v2-residuals-closeout.md` §R5.2.
#[test]
fn test_r5_2b_user_add_retargets_single_schema_fallback() {
    let program = compile(
        r#"
        type Vec2 { x: number, y: number }

        impl Add for Vec2 {
            method add(other: Vec2) -> Vec2 {
                Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
        }

        // Identity-style function with an untyped parameter: its inferred
        // return type is a free type variable (not a concrete schema), so
        // the call-site value has no TypedObject schema attached.
        fn pick(x) { return x }

        let a = Vec2 { x: 1.0, y: 2.0 }
        let b = Vec2 { x: 3.0, y: 4.0 }
        // `pick(b)` returns a schema-less value at compile time: the
        // identity return type is a fresh type variable, so after
        // `compile_expr(right)` the compiler's `last_expr_schema` is None.
        // This defeats the priority-1 both-schemas fast path at L665-684
        // and forces the Add branch into the `CoercedNeedsGeneric | NoPlan`
        // arm — the gap R5.2B closes. The R5.2B-inserted
        // `try_emit_trait_dispatch` call then picks up `left_schema = Vec2`
        // and retargets to `CallMethod("add")` at compile time.
        let r = a + pick(b)
    "#,
    );

    let ops = all_opcodes(&program);

    // R5.2B: the single-schema fallback must now retarget to `CallMethod`
    // at compile time, not fall through to `AddDynamic`. Scanning the
    // whole instruction vector is safe: every `+` inside `Vec2::add`
    // operates on proven-numeric operands and emits `AddNumber`; any
    // `AddDynamic` would be the R5.2B gap regressing.
    assert!(
        !ops.contains(&OpCode::AddDynamic),
        "R5.2B regression: Add branch's CoercedNeedsGeneric | NoPlan arm \
         fell through to AddDynamic instead of retargeting to CallMethod. \
         Ops emitted: {:?}",
        ops
    );

    // `CallMethod` must appear at least once — for the top-level
    // `a + { ... }` retargeted to `a.add(rhs)`.
    let call_method_count = ops.iter().filter(|&&o| o == OpCode::CallMethod).count();
    assert!(
        call_method_count >= 1,
        "R5.2B regression: no CallMethod emitted for the single-schema \
         user-op Add fallback. Ops emitted: {:?}",
        ops
    );
}

/// R5.3B retarget regression test for the v2 residuals closeout plan.
///
/// Replaces `test_r5_3a_datetime_arithmetic_fallback_baseline` (the R5.3A
/// baseline that pinned the pre-fix `AddDynamic` / `SubDynamic` emission).
/// After R5.3B the compiler's temporal retarget fires uniformly:
///
///   - `compile_expr_datetime` and `compile_expr_duration` now publish the
///     temporal display name on `last_expr_type_info`, so
///     `propagate_assignment_type_to_slot` records `"DateTime"` or
///     `"Duration"` on let-locals bound to temporal literals.
///   - `infer_expr_type` consults the compiler's `type_tracker` for
///     `Expr::Identifier` and returns the temporal display name whenever
///     the local/module-binding tracker records it, covering both typed
///     function parameters (populated via
///     `tracked_type_name_from_annotation`) and let-locals (populated via
///     the new `propagate_assignment_type_to_slot` arm).
///
/// With those two fixes the retarget guards at
/// `compiler/expressions/binary_ops.rs:750-771` (Add) and `:1049-1072`
/// (Sub) fire for every reachable DateTime / Duration arithmetic shape
/// and emit `CallMethod("add")` / `CallMethod("sub")`. The five temporal
/// arms in `executor/arithmetic/mod.rs::try_heap_arithmetic` are now
/// unreachable for any program these assertions can reach.
///
/// The three cases mirror the R5.3A baseline shapes: let-locals +
/// literals, typed params with `DateTime - Duration`, typed params with
/// `DateTime - DateTime`. Each asserts `CallMethod` is emitted and that
/// the dynamic-fallback opcode is absent.
///
/// Reference: `/home/dev/.claude/plans/v2-residuals-closeout.md` §R5.3.

#[test]
fn test_r5_3b_datetime_arithmetic_retargets_to_call_method() {
    // Case 1: DateTime + Duration via let-locals. After R5.3B the
    // temporal-literal initializers populate the local tracker with
    // `"DateTime"` / `"Duration"`, and `infer_expr_type` reads them back
    // through the tracker for the `dt + dur` identifier references so the
    // retarget fires.
    {
        let program = compile(
            r#"
            fn test() {
                let dt = @"2024-01-15"
                let dur = 3d
                dt + dur
            }
            test()
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            !ops.contains(&OpCode::AddDynamic),
            "R5.3B regression: let-local DateTime + Duration fell through \
             to AddDynamic instead of retargeting to CallMethod. \
             Ops emitted: {:?}",
            ops
        );
        assert!(
            ops.contains(&OpCode::CallMethod),
            "R5.3B regression: let-local DateTime + Duration did not emit \
             CallMethod. Ops emitted: {:?}",
            ops
        );
    }

    // Case 2: DateTime - Duration via typed function parameters. Param
    // annotations seed the tracker via `tracked_type_name_from_annotation`
    // in `compile_function`, so `infer_expr_type` finds `"DateTime"` /
    // `"Duration"` for the identifiers and the Sub retarget fires.
    {
        let program = compile(
            r#"
            fn test(dt: DateTime, dur: Duration) {
                dt - dur
            }
            test(@"2024-01-15", 1d)
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            !ops.contains(&OpCode::SubDynamic),
            "R5.3B regression: typed-param DateTime - Duration fell through \
             to SubDynamic instead of retargeting to CallMethod. \
             Ops emitted: {:?}",
            ops
        );
        assert!(
            ops.contains(&OpCode::CallMethod),
            "R5.3B regression: typed-param DateTime - Duration did not emit \
             CallMethod. Ops emitted: {:?}",
            ops
        );
    }

    // Case 3: DateTime - DateTime via typed function parameters. Yields a
    // Duration/TimeSpan via `datetime_sub_v2` through CallMethod.
    {
        let program = compile(
            r#"
            fn test(a: DateTime, b: DateTime) {
                a - b
            }
            test(@"2024-01-15", @"2024-01-10")
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            !ops.contains(&OpCode::SubDynamic),
            "R5.3B regression: typed-param DateTime - DateTime fell through \
             to SubDynamic instead of retargeting to CallMethod. \
             Ops emitted: {:?}",
            ops
        );
        assert!(
            ops.contains(&OpCode::CallMethod),
            "R5.3B regression: typed-param DateTime - DateTime did not emit \
             CallMethod. Ops emitted: {:?}",
            ops
        );
    }
}

#[test]
fn test_operator_overload_without_trait_fails() {
    // Without implementing Sub, - on custom types should fail at compile time
    let result = eval_result(
        r#"
        type Foo { x: int }
        let a = Foo { x: 1 }
        let b = Foo { x: 2 }
        a - b
    "#,
    );
    assert!(
        result.is_err(),
        "Subtracting two Foo without impl Sub should fail"
    );
}

/// R5.4A baseline regression test for the v2 residuals closeout plan.
///
/// Pins the current emission shape for `Matrix + Matrix`, `Matrix - Matrix`,
/// `Vec<number> ± * / Vec<number>`, and `Vec<int> + Vec<int>` so that R5.4B
/// can flip the assertions to the retargeted intrinsic form in a single,
/// reviewable commit — mirroring the R5.3A→R5.3B pattern for DateTime.
///
/// Today, `Mat<number> + Mat<number>`, `Mat<number> - Mat<number>`,
/// `Vec<number> ± * / Vec<number>`, and `Vec<int> + Vec<int>` all fall
/// through to `AddDynamic` / `SubDynamic` / `MulDynamic` / `DivDynamic`
/// and dispatch via
/// `executor/arithmetic/mod.rs::try_heap_arithmetic`'s Matrix /
/// `TypedArrayData::F64` / `TypedArrayData::I64` arms. The compile-time
/// retarget does not fire because:
///
///   - The inferred operand type names are `"Vec"` / `"Mat"` (from
///     `type_display_name(Type::Concrete(TypeAnnotation::Generic { name, .. }))`),
///     which are neither numeric (so `is_type_numeric` returns false and
///     the strict-arithmetic error gate is sidestepped via the
///     `Expr::IndexAccess` / `adopt_missing_numeric_operand_hint` paths
///     or the operand-trait-check escape) nor registered in the Add-arm's
///     typed-heap retargets at `binary_ops.rs:~L703-771`
///     (String / Array / DateTime / Duration).
///   - There is no schema attached (Vec / Mat aren't TypedObjects), so
///     `try_emit_trait_dispatch` in the `CoercedNeedsGeneric | NoPlan`
///     arm returns false and the shim emits `AddDynamic` for Add and
///     dispatches the `_ => {}` strict-arithmetic branch for Sub / Mul /
///     Div to `emit_generic_via_helper` / `compile_binary_op`.
///
/// The arms in `try_heap_arithmetic` corresponding to these operand
/// shapes are currently the live execution path — they are not yet
/// marked dead. R5.4B will (1) resolve the Mat*Mat representation bug
/// and missing intrinsics, (2) retarget at compile time, and (3) flip
/// each assertion below to require the retargeted lowering. See the
/// expanded doc comment at `try_heap_arithmetic` in
/// `executor/arithmetic/mod.rs` for the three unblocking preconditions.
///
/// Reference: /home/dev/.claude/plans/v2-residuals-closeout.md §R5.4.
#[test]
fn test_r5_4a_matrix_vec_arithmetic_fallback_baseline() {
    // Case 1: `Mat<number> + Mat<number>` via typed function parameters.
    // Expected emission today: AddDynamic (via emit_binary_op shim).
    {
        let program = compile(
            r#"
            fn test(a: Mat<number>, b: Mat<number>) { a + b }
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            ops.contains(&OpCode::AddDynamic),
            "R5.4A baseline regression: Mat+Mat must emit AddDynamic \
             until R5.4B lands the retarget. Ops emitted: {:?}",
            ops
        );
    }

    // Case 2: `Mat<number> - Mat<number>` via typed function parameters.
    // Expected emission today: SubDynamic (via emit_generic_via_helper
    // / compile_binary_op in the `_ => {}` strict-arithmetic arm).
    {
        let program = compile(
            r#"
            fn test(a: Mat<number>, b: Mat<number>) { a - b }
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            ops.contains(&OpCode::SubDynamic),
            "R5.4A baseline regression: Mat-Mat must emit SubDynamic \
             until R5.4B lands the retarget. Ops emitted: {:?}",
            ops
        );
    }

    // Case 3: `Vec<number> + Vec<number>` via typed function parameters.
    // Expected emission today: AddDynamic (via emit_binary_op shim). At
    // runtime this reaches the SIMD arm in try_heap_arithmetic keyed on
    // `(TypedArrayData::F64, TypedArrayData::F64)`.
    {
        let program = compile(
            r#"
            fn test(a: Vec<number>, b: Vec<number>) { a + b }
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            ops.contains(&OpCode::AddDynamic),
            "R5.4A baseline regression: Vec<number>+Vec<number> must \
             emit AddDynamic until R5.4B lands the retarget. \
             Ops emitted: {:?}",
            ops
        );
    }

    // Case 4: `Vec<number> - Vec<number>` via typed function parameters.
    {
        let program = compile(
            r#"
            fn test(a: Vec<number>, b: Vec<number>) { a - b }
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            ops.contains(&OpCode::SubDynamic),
            "R5.4A baseline regression: Vec<number>-Vec<number> must \
             emit SubDynamic until R5.4B lands the retarget. \
             Ops emitted: {:?}",
            ops
        );
    }

    // Case 5: `Vec<number> * Vec<number>` via typed function parameters.
    // Elementwise SIMD multiply at the fallback. Does NOT route through
    // `try_compile_typed_matrix_mul` (which only fires for Mat*Vec and
    // Mat*Mat).
    {
        let program = compile(
            r#"
            fn test(a: Vec<number>, b: Vec<number>) { a * b }
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            ops.contains(&OpCode::MulDynamic),
            "R5.4A baseline regression: Vec<number>*Vec<number> must \
             emit MulDynamic until R5.4B lands the retarget. \
             Ops emitted: {:?}",
            ops
        );
    }

    // Case 6: `Vec<number> / Vec<number>` via typed function parameters.
    {
        let program = compile(
            r#"
            fn test(a: Vec<number>, b: Vec<number>) { a / b }
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            ops.contains(&OpCode::DivDynamic),
            "R5.4A baseline regression: Vec<number>/Vec<number> must \
             emit DivDynamic until R5.4B lands the retarget. \
             Ops emitted: {:?}",
            ops
        );
    }

    // Case 7: `Vec<int> + Vec<int>` via typed function parameters.
    // At runtime this reaches the integer-preserving arm in
    // try_heap_arithmetic keyed on `(TypedArrayData::I64,
    // TypedArrayData::I64)` which calls `simd_vec_add_i64` and returns
    // overflow errors as `VMError::RuntimeError`.
    {
        let program = compile(
            r#"
            fn test(a: Vec<int>, b: Vec<int>) { a + b }
            "#,
        );
        let ops = all_opcodes(&program);
        assert!(
            ops.contains(&OpCode::AddDynamic),
            "R5.4A baseline regression: Vec<int>+Vec<int> must emit \
             AddDynamic until R5.4B lands the retarget (and adds the \
             IntrinsicVecAddI64 builtin to preserve overflow semantics). \
             Ops emitted: {:?}",
            ops
        );
    }
}


