//! Numeric-conversion LITERAL ADOPTION — value-correct float lowering
//! (numeric-conversion-spec §4, THE RULE user 2026-06-01).
//!
//! When the type checker accepts a bare integer literal in a `number`(f64)
//! context (`let n: number = 5`, `f(5)` where `f(x: number)`, `-> number { 5 }`,
//! `Array<number> = [1, 2, 3]`, `x: number = 5` default), the literal `5` IS
//! the number literal `5.0` — it must lower to a FLOAT constant carrying
//! `NativeKind::Float64`, NOT an `Int64` constant. The checker only *accepts*
//! the adoption (`adopt_int_literal_in_context`); it leaves the AST node a
//! `Literal::Int`, so without this lowering the bytecode emits an i64 constant
//! into a Float64-stamped slot — a bit-reinterpret soundness hole
//! (`f(5)` reading raw i64 `5` as f64 bits → `2.5e-323`; `n / 2` doing int
//! division → `2`).
//!
//! This is COMPILE-TIME literal re-typing, NOT a runtime coercion opcode: the
//! literal `5` is exactly `5.0` in a number context, so we rebuild the AST node
//! as `Literal::Number(5.0)` and let the normal float-literal emission path
//! push a `Constant::Number`. No `IntToNumber`/`Convert<X>To<Y>` opcode, no
//! W4-δ defection — the established pattern at
//! `expressions/assignment.rs:765` (field-assignment widening).
//!
//! SOUNDNESS: fires ONLY for a bare `Int`/`UInt` literal (a `42u8` typed-int
//! literal, a float/decimal literal, and any non-literal expression are NOT
//! rewritten) whose value losslessly fits the f64 exact-integer range
//! `[-2^53, 2^53]`, AND only when the target type is `number`/`f64`. A
//! non-literal `int` value never adopts — `let m: int = 5; let n: number = m`
//! stays a COMPILE ERROR (the p-var rejection is upstream in the type checker
//! and is not weakened here). `int` and `number` never unify.

use shape_ast::ast::{Expr, Literal, Spanned, TypeAnnotation};
use shape_runtime::type_schema::FieldType;

/// The f64 exact-integer lossless range `[-2^53, 2^53]`.
fn fits_f64_lossless(v: i128) -> bool {
    v >= -(1i128 << 53) && v <= (1i128 << 53)
}

/// The bare-int literal value of `expr` as `i128`, if it is an adoptable
/// integer literal (`Int`/`UInt`). A typed-int literal (`42u8`), a float, a
/// decimal, or any non-literal expression returns `None` — it does not adopt.
fn adoptable_int_literal_value(expr: &Expr) -> Option<i128> {
    match expr {
        Expr::Literal(Literal::Int(v), _) => Some(*v as i128),
        Expr::Literal(Literal::UInt(v), _) => Some(*v as i128),
        _ => None,
    }
}

/// Whether `name` denotes the `number`/`f64` floating-point family. The literal
/// adoption to a FLOAT constant only fires for this family — width-int targets
/// (`u8`, `i32`, …) and `decimal` keep their natural integer-literal lowering
/// (the Int64 / Decimal bits are the correct slot payload there).
fn is_number_type_name(name: &str) -> bool {
    matches!(name, "number" | "f64" | "float")
}

/// Rewrite a bare integer literal `expr` to a `Number` literal when it adopts a
/// `number`/`f64` context type given as an AST `TypeAnnotation` (the
/// let-annotation / param-annotation / return-annotation site shape). Returns
/// `None` when no rewrite applies (the caller compiles the original `expr`).
pub(crate) fn widen_int_literal_for_annotation(
    expr: &Expr,
    annotation: &TypeAnnotation,
) -> Option<Expr> {
    let name = annotation.as_simple_name()?;
    if !is_number_type_name(name) {
        return None;
    }
    widen_int_literal_to_number(expr)
}

/// Rewrite a bare integer literal `expr` to a `Number` literal when the target
/// `FieldType` is `F64` (the struct/object-field / array-element site shape).
/// Returns `None` when no rewrite applies.
pub(crate) fn widen_int_literal_for_field_type(
    expr: &Expr,
    field_type: &FieldType,
) -> Option<Expr> {
    if !matches!(field_type, FieldType::F64) {
        return None;
    }
    widen_int_literal_to_number(expr)
}

/// Core rewrite: a bare `Int`/`UInt` literal whose value losslessly fits the
/// f64 exact-integer range becomes the equivalent `Literal::Number`. The span
/// is preserved so diagnostics still point at the source literal. Out-of-range
/// literals are NOT rewritten (they keep their natural int literal, which then
/// fails the §2 lossless lattice against the float target downstream — the
/// adoption is gated on losslessness exactly like the checker's
/// `int_value_fits_numeric` F64 arm).
pub(crate) fn widen_int_literal_to_number(expr: &Expr) -> Option<Expr> {
    let v = adoptable_int_literal_value(expr)?;
    if !fits_f64_lossless(v) {
        return None;
    }
    let span = expr.span();
    Some(Expr::Literal(Literal::Number(v as f64), span))
}
