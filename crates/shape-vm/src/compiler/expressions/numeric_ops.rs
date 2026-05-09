//! Numeric binary-op helpers shared by expression lowering.

use crate::bytecode::{Instruction, OpCode};
use crate::type_tracking::NumericType;
use shape_ast::ast::{BinaryOp, TypeAnnotation};
use shape_runtime::type_system::{BuiltinTypes, Type};

use super::super::BytecodeCompiler;

/// Check if a BinaryOp is strictly arithmetic (requires numeric operands).
/// Add is excluded because it also handles string concat, object merge, array concat.
pub(super) fn is_strict_arithmetic(op: &BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow
    )
}

/// Check if a BinaryOp is an ordered comparison (typed variants exist).
pub(super) fn is_ordered_comparison(op: &BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Greater | BinaryOp::Less | BinaryOp::GreaterEq | BinaryOp::LessEq
    )
}

/// Check if a Type from the inference engine is numeric.
pub(super) fn is_type_numeric(ty: &Type) -> bool {
    let name = match ty {
        Type::Concrete(TypeAnnotation::Basic(name)) => Some(name.as_str()),
        Type::Concrete(TypeAnnotation::Reference(name)) => Some(name.as_str()),
        _ => None,
    };
    if let Some(name) = name {
        BuiltinTypes::is_integer_type_name(name)
            || BuiltinTypes::is_number_type_name(name)
            || matches!(name, "decimal" | "Decimal")
    } else {
        false
    }
}

pub(super) fn is_function_type(ty: &Type) -> bool {
    matches!(ty, Type::Function { .. })
}

/// Map an inferred Type to a NumericType for typed opcode emission.
pub(super) fn inferred_type_to_numeric(ty: &Type) -> Option<NumericType> {
    let name = match ty {
        Type::Concrete(TypeAnnotation::Basic(name)) => Some(name.as_str()),
        Type::Concrete(TypeAnnotation::Reference(name)) => Some(name.as_str()),
        _ => None,
    };
    let name = name?;
    // Check width-specific integer types first
    if let Some(w) = shape_ast::IntWidth::from_name(name) {
        return Some(NumericType::IntWidth(w));
    }
    if BuiltinTypes::is_integer_type_name(name) {
        return Some(NumericType::Int);
    }
    if BuiltinTypes::is_number_type_name(name) {
        return Some(NumericType::Number);
    }
    match name {
        "decimal" | "Decimal" => Some(NumericType::Decimal),
        _ => None,
    }
}

/// Get a human-readable name for a Type (for error messages).
pub(super) fn type_display_name(ty: &Type) -> String {
    match ty {
        Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
        Type::Concrete(TypeAnnotation::Object(fields)) => {
            let field_strs: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
            format!("{{{}}}", field_strs.join(", "))
        }
        Type::Concrete(TypeAnnotation::Array(inner)) => {
            format!("{}[]", type_display_name(&Type::Concrete(*inner.clone())))
        }
        Type::Concrete(TypeAnnotation::Generic { name, .. }) => name.to_string(),
        Type::Variable(v) => format!("?T{}", v.0),
        _ => format!("{:?}", ty),
    }
}

/// Coercion direction for mixed-type operands.
#[derive(Debug, Clone, Copy)]
pub(super) enum CoercionPlan {
    /// Both types match - no coercion needed
    NoCoercion(NumericType),
    /// Left operand needs coercion (Int->Number). Stack: [left, right] -> Swap,IntToNumber,Swap
    CoerceLeft(NumericType),
    /// Right operand needs coercion (Int->Number). Stack: [left, right] -> IntToNumber
    CoerceRight(NumericType),
    /// Incompatible width types (u64 + signed) — must be a compile error
    IncompatibleWidths(shape_ast::IntWidth, shape_ast::IntWidth),
}

/// Resolve mixed numeric pairs with coercion.
/// Returns the target numeric type and which operand needs coercion.
pub(super) fn plan_coercion(
    left: Option<NumericType>,
    right: Option<NumericType>,
) -> Option<CoercionPlan> {
    match (left, right) {
        (Some(l), Some(r)) if l == r => Some(CoercionPlan::NoCoercion(l)),
        // Int + Number -> coerce left Int to Number
        (Some(NumericType::Int), Some(NumericType::Number)) => {
            Some(CoercionPlan::CoerceLeft(NumericType::Number))
        }
        // Number + Int -> coerce right Int to Number
        (Some(NumericType::Number), Some(NumericType::Int)) => {
            Some(CoercionPlan::CoerceRight(NumericType::Number))
        }
        // IntWidth + Number → coerce left to Number
        (Some(NumericType::IntWidth(_)), Some(NumericType::Number)) => {
            Some(CoercionPlan::CoerceLeft(NumericType::Number))
        }
        // Number + IntWidth → coerce right to Number
        (Some(NumericType::Number), Some(NumericType::IntWidth(_))) => {
            Some(CoercionPlan::CoerceRight(NumericType::Number))
        }
        // IntWidth + Int → widen to Int (i64)
        (Some(NumericType::IntWidth(_)), Some(NumericType::Int)) => {
            Some(CoercionPlan::NoCoercion(NumericType::Int))
        }
        // Int + IntWidth → widen to Int (i64)
        (Some(NumericType::Int), Some(NumericType::IntWidth(_))) => {
            Some(CoercionPlan::NoCoercion(NumericType::Int))
        }
        // IntWidth(a) + IntWidth(b) → join widths
        (Some(NumericType::IntWidth(a)), Some(NumericType::IntWidth(b))) => {
            match shape_ast::IntWidth::join(a, b) {
                Ok(joined) => Some(CoercionPlan::NoCoercion(NumericType::IntWidth(joined))),
                Err(()) => {
                    // Only u64 + signed is truly incompatible (u64 can't fit in i64).
                    // Other mismatches (e.g. u32 + i8) safely promote to default int (i64).
                    let either_u64 = a == shape_ast::IntWidth::U64 || b == shape_ast::IntWidth::U64;
                    let mixed_sign = a.is_signed() != b.is_signed();
                    if either_u64 && mixed_sign {
                        Some(CoercionPlan::IncompatibleWidths(a, b))
                    } else {
                        // Promote to default int (i64) — both values fit
                        Some(CoercionPlan::NoCoercion(NumericType::Int))
                    }
                }
            }
        }
        _ => None,
    }
}

/// Apply the stack coercion plan and return the resulting numeric type.
pub(super) fn apply_coercion(compiler: &mut BytecodeCompiler, plan: CoercionPlan) -> NumericType {
    match plan {
        CoercionPlan::NoCoercion(t) => t,
        CoercionPlan::CoerceLeft(t) => {
            compiler.emit(Instruction::simple(OpCode::Swap));
            compiler.emit(Instruction::simple(OpCode::IntToNumber));
            compiler.emit(Instruction::simple(OpCode::Swap));
            t
        }
        CoercionPlan::CoerceRight(t) => {
            compiler.emit(Instruction::simple(OpCode::IntToNumber));
            t
        }
        CoercionPlan::IncompatibleWidths(_, _) => {
            unreachable!("IncompatibleWidths should be handled before apply_coercion")
        }
    }
}

/// Const dispatch table for typed arithmetic opcodes.
/// Indexed by [arith_op_index][numeric_type]: Add=0, Sub=1, Mul=2, Div=3, Mod=4, Pow=5
///                                            Int=0, Number=1, Decimal=2
const TYPED_ARITH: [[OpCode; 3]; 6] = [
    [OpCode::AddInt, OpCode::AddNumber, OpCode::AddDecimal],
    [OpCode::SubInt, OpCode::SubNumber, OpCode::SubDecimal],
    [OpCode::MulInt, OpCode::MulNumber, OpCode::MulDecimal],
    [OpCode::DivInt, OpCode::DivNumber, OpCode::DivDecimal],
    [OpCode::ModInt, OpCode::ModNumber, OpCode::ModDecimal],
    [OpCode::PowInt, OpCode::PowNumber, OpCode::PowDecimal],
];

/// Const dispatch table for typed comparison opcodes.
/// Indexed by [cmp_op_index][numeric_type]: Gt=0, Lt=1, Gte=2, Lte=3
const TYPED_CMP: [[OpCode; 3]; 4] = [
    [OpCode::GtInt, OpCode::GtNumber, OpCode::GtDecimal],
    [OpCode::LtInt, OpCode::LtNumber, OpCode::LtDecimal],
    [OpCode::GteInt, OpCode::GteNumber, OpCode::GteDecimal],
    [OpCode::LteInt, OpCode::LteNumber, OpCode::LteDecimal],
];

/// Map a BinaryOp to an arithmetic table index (0-5), or None if not arithmetic.
fn arith_op_index(op: &BinaryOp) -> Option<usize> {
    match op {
        BinaryOp::Add => Some(0),
        BinaryOp::Sub => Some(1),
        BinaryOp::Mul => Some(2),
        BinaryOp::Div => Some(3),
        BinaryOp::Mod => Some(4),
        BinaryOp::Pow => Some(5),
        _ => None,
    }
}

/// Map a BinaryOp to a comparison table index (0-3), or None if not an ordered comparison.
fn cmp_op_index(op: &BinaryOp) -> Option<usize> {
    match op {
        BinaryOp::Greater => Some(0),
        BinaryOp::Less => Some(1),
        BinaryOp::GreaterEq => Some(2),
        BinaryOp::LessEq => Some(3),
        _ => None,
    }
}

/// Map a NumericType to a table column index.
fn numeric_type_index(nt: NumericType) -> usize {
    match nt {
        NumericType::Int | NumericType::IntWidth(_) => 0,
        NumericType::Number => 1,
        NumericType::Decimal => 2,
    }
}

/// Map a (BinaryOp, NumericType) pair to a typed opcode.
/// For IntWidth types, returns the compact *Typed opcode (AddTyped, etc.)
/// that carries width information as an operand.
pub(super) fn typed_opcode_for(op: &BinaryOp, nt: NumericType) -> Option<OpCode> {
    // i32 gets direct v2 opcodes — no Width operand needed
    if matches!(nt, NumericType::IntWidth(shape_ast::IntWidth::I32)) {
        return match op {
            BinaryOp::Add => Some(OpCode::AddI32),
            BinaryOp::Sub => Some(OpCode::SubI32),
            BinaryOp::Mul => Some(OpCode::MulI32),
            BinaryOp::Div => Some(OpCode::DivI32),
            BinaryOp::Mod => Some(OpCode::ModI32),
            BinaryOp::Greater => Some(OpCode::GtI32),
            BinaryOp::Less => Some(OpCode::LtI32),
            BinaryOp::GreaterEq => Some(OpCode::GteI32),
            BinaryOp::LessEq => Some(OpCode::LteI32),
            BinaryOp::Equal => Some(OpCode::EqI32),
            BinaryOp::NotEqual => Some(OpCode::NeqI32),
            _ => None,
        };
    }

    // Other width-specific integers use compact typed opcodes with Width operand
    if let NumericType::IntWidth(_) = nt {
        return match op {
            BinaryOp::Add => Some(OpCode::AddTyped),
            BinaryOp::Sub => Some(OpCode::SubTyped),
            BinaryOp::Mul => Some(OpCode::MulTyped),
            BinaryOp::Div => Some(OpCode::DivTyped),
            BinaryOp::Mod => Some(OpCode::ModTyped),
            // Use regular int comparison opcodes for width types — they return
            // booleans (CmpTyped returns an ordering which callers don't expect).
            // Sub-64-bit unsigned values are non-negative in i64 so signed
            // comparison is correct for u8/u16/u32. u64 is handled separately.
            BinaryOp::Greater => Some(OpCode::GtInt),
            BinaryOp::Less => Some(OpCode::LtInt),
            BinaryOp::GreaterEq => Some(OpCode::GteInt),
            BinaryOp::LessEq => Some(OpCode::LteInt),
            BinaryOp::Equal => Some(OpCode::EqInt),
            BinaryOp::NotEqual => Some(OpCode::NeqInt),
            _ => None,
        };
    }

    let col = numeric_type_index(nt);
    if let Some(row) = arith_op_index(op) {
        Some(TYPED_ARITH[row][col])
    } else if let Some(row) = cmp_op_index(op) {
        Some(TYPED_CMP[row][col])
    } else if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
        match (op, nt) {
            (BinaryOp::Equal, NumericType::Int) => Some(OpCode::EqInt),
            (BinaryOp::Equal, NumericType::Number) => Some(OpCode::EqNumber),
            (BinaryOp::Equal, NumericType::Decimal) => Some(OpCode::EqDecimal),
            (BinaryOp::NotEqual, NumericType::Int) => Some(OpCode::NeqInt),
            (BinaryOp::NotEqual, NumericType::Number) => Some(OpCode::NeqNumber),
            _ => None,
        }
    } else {
        None
    }
}

// NOTE: Trusted arithmetic/comparison opcodes (TRUSTED_ARITH, TRUSTED_CMP,
// try_trusted_opcode) have been removed. The typed opcodes (AddInt, GtInt, etc.)
// are sufficient — they already provide zero-dispatch execution.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_aware_basic_types_map_to_numeric_hints() {
        use shape_ast::IntWidth;
        let int_ty = Type::Concrete(TypeAnnotation::Basic("i16".to_string()));
        let uint_ty = Type::Concrete(TypeAnnotation::Basic("u8".to_string()));
        let float_ty = Type::Concrete(TypeAnnotation::Basic("f32".to_string()));
        let double_ty = Type::Concrete(TypeAnnotation::Basic("f64".to_string()));
        let default_int = Type::Concrete(TypeAnnotation::Basic("int".to_string()));

        assert_eq!(
            inferred_type_to_numeric(&int_ty),
            Some(NumericType::IntWidth(IntWidth::I16))
        );
        assert_eq!(
            inferred_type_to_numeric(&uint_ty),
            Some(NumericType::IntWidth(IntWidth::U8))
        );
        assert_eq!(
            inferred_type_to_numeric(&float_ty),
            Some(NumericType::Number)
        );
        assert_eq!(
            inferred_type_to_numeric(&double_ty),
            Some(NumericType::Number)
        );
        // Default `int` stays as NumericType::Int (not IntWidth)
        assert_eq!(
            inferred_type_to_numeric(&default_int),
            Some(NumericType::Int)
        );
    }

    #[test]
    fn width_aware_reference_types_map_to_numeric_hints() {
        use shape_ast::IntWidth;
        let int_ref = Type::Concrete(TypeAnnotation::Reference("i32".into()));
        let float_ref = Type::Concrete(TypeAnnotation::Reference("f32".into()));

        assert_eq!(
            inferred_type_to_numeric(&int_ref),
            Some(NumericType::IntWidth(IntWidth::I32))
        );
        assert_eq!(
            inferred_type_to_numeric(&float_ref),
            Some(NumericType::Number)
        );
    }

    #[test]
    fn coercion_u64_plus_signed_is_incompatible() {
        use shape_ast::IntWidth;
        // u64 + i8 should be IncompatibleWidths (compile error)
        let plan = plan_coercion(
            Some(NumericType::IntWidth(IntWidth::U64)),
            Some(NumericType::IntWidth(IntWidth::I8)),
        );
        assert!(
            matches!(plan, Some(CoercionPlan::IncompatibleWidths(_, _))),
            "u64 + i8 should be IncompatibleWidths, got {:?}",
            plan
        );

        // i32 + u64 should also be IncompatibleWidths
        let plan = plan_coercion(
            Some(NumericType::IntWidth(IntWidth::I32)),
            Some(NumericType::IntWidth(IntWidth::U64)),
        );
        assert!(
            matches!(plan, Some(CoercionPlan::IncompatibleWidths(_, _))),
            "i32 + u64 should be IncompatibleWidths, got {:?}",
            plan
        );
    }

    #[test]
    fn coercion_u32_plus_signed_promotes_to_int() {
        use shape_ast::IntWidth;
        // u32 + i8 should promote to default Int (i64), not IncompatibleWidths
        let plan = plan_coercion(
            Some(NumericType::IntWidth(IntWidth::U32)),
            Some(NumericType::IntWidth(IntWidth::I8)),
        );
        assert!(
            matches!(plan, Some(CoercionPlan::NoCoercion(NumericType::Int))),
            "u32 + i8 should promote to Int (i64), got {:?}",
            plan
        );

        // i8 + u32 should also promote to default Int (i64)
        let plan = plan_coercion(
            Some(NumericType::IntWidth(IntWidth::I8)),
            Some(NumericType::IntWidth(IntWidth::U32)),
        );
        assert!(
            matches!(plan, Some(CoercionPlan::NoCoercion(NumericType::Int))),
            "i8 + u32 should promote to Int (i64), got {:?}",
            plan
        );
    }

    #[test]
    fn coercion_same_width_types_no_coercion() {
        use shape_ast::IntWidth;
        // u8 + u8 should be NoCoercion(IntWidth(U8))
        let plan = plan_coercion(
            Some(NumericType::IntWidth(IntWidth::U8)),
            Some(NumericType::IntWidth(IntWidth::U8)),
        );
        assert!(
            matches!(
                plan,
                Some(CoercionPlan::NoCoercion(NumericType::IntWidth(
                    IntWidth::U8
                )))
            ),
            "u8 + u8 should be NoCoercion(U8), got {:?}",
            plan
        );
    }

    #[test]
    fn i32_gets_direct_opcodes_not_typed() {
        use shape_ast::IntWidth;
        // i32 should get direct AddI32, not AddTyped
        assert_eq!(
            typed_opcode_for(&BinaryOp::Add, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::AddI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::Sub, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::SubI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::Mul, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::MulI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::Div, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::DivI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::Mod, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::ModI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::Equal, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::EqI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::NotEqual, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::NeqI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::Greater, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::GtI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::Less, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::LtI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::GreaterEq, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::GteI32)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::LessEq, NumericType::IntWidth(IntWidth::I32)),
            Some(OpCode::LteI32)
        );

        // Other widths should still use AddTyped
        assert_eq!(
            typed_opcode_for(&BinaryOp::Add, NumericType::IntWidth(IntWidth::U8)),
            Some(OpCode::AddTyped)
        );
        assert_eq!(
            typed_opcode_for(&BinaryOp::Add, NumericType::IntWidth(IntWidth::I16)),
            Some(OpCode::AddTyped)
        );
    }

    // --- End-to-end tests: compile and execute Shape code ---
    //
    // Wave-β C-expressions: the `eval_fn` harness returned the deleted
    // `ValueWord` carrier and every end-to-end test asserted on it via
    // `.as_i64()` / `.as_bool()` (deleted `ValueWordExt` accessors).
    // Width-arith semantic coverage is restored together with the
    // phase-2c carrier shape (ADR-006 §2.4); the compile-side check
    // below stays here because it doesn't touch the carrier.

    fn compile_should_fail(code: &str) -> bool {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let compiler = super::super::super::BytecodeCompiler::new();
        compiler.compile(&program).is_err()
    }

    // HIGH-1 / MED-2 / MED-3 width-arith semantic tests deleted with
    // `eval_fn`; see comment above. Restore alongside the phase-2c
    // carrier shape (ADR-006 §2.4).

    // MED-4: u64 + signed types should be a compile error
    #[test]
    fn u64_plus_signed_is_compile_error() {
        assert!(
            compile_should_fail(
                r#"
                function test() -> int {
                    let a: u64 = 100
                    let b: i8 = 10
                    return a + b
                }
                "#
            ),
            "u64 + i8 should be a compile error"
        );
    }

    // v2 direct i32 opcodes (i32_add_uses_direct_opcode and friends)
    // and `u32_plus_signed_promotes_to_i64` deleted with `eval_fn`;
    // restore alongside the phase-2c carrier shape (ADR-006 §2.4).
}
