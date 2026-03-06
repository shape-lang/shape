//! Binary operation expression compilation

use crate::bytecode::{Instruction, NumericWidth, OpCode, Operand};
use crate::type_tracking::{NumericType, VariableTypeInfo};
use shape_ast::ast::operators::{FuzzyOp, FuzzyTolerance};
use shape_ast::ast::{BinaryOp, Expr, Literal, Span, Spanned, UnaryOp};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::{FieldType, SchemaId};

use super::super::BytecodeCompiler;
use super::numeric_ops::{
    CoercionPlan, apply_coercion, inferred_type_to_numeric, is_function_type,
    is_ordered_comparison, is_strict_arithmetic, is_type_numeric, plan_coercion,
    try_trusted_opcode, type_display_name, typed_opcode_for,
};

/// Map a strict arithmetic BinaryOp to its operator trait name, if one exists.
fn operator_trait_for_op(op: &BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Sub => Some("Sub"),
        BinaryOp::Mul => Some("Mul"),
        BinaryOp::Div => Some("Div"),
        _ => None, // Mod, Pow have no operator trait
    }
}

fn combined_span(left: &Expr, right: &Expr) -> Span {
    let ls = left.span();
    let rs = right.span();
    Span::new(ls.start.min(rs.start), ls.end.max(rs.end))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericEmitResult {
    EmittedTyped,
    CoercedNeedsGeneric,
    NoPlan,
}

impl BytecodeCompiler {
    fn infer_numeric_pair(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> (Option<NumericType>, Option<NumericType>) {
        let inferred_left = self
            .infer_expr_type(left)
            .ok()
            .and_then(|t| inferred_type_to_numeric(&t));
        let inferred_right = self
            .infer_expr_type(right)
            .ok()
            .and_then(|t| inferred_type_to_numeric(&t));
        (inferred_left, inferred_right)
    }

    fn adopt_missing_numeric_operand_hint(
        &mut self,
        left: &Expr,
        right: &Expr,
        left_numeric: &mut Option<NumericType>,
        right_numeric: &mut Option<NumericType>,
    ) {
        if let (Some(known), None) = (*left_numeric, *right_numeric)
            && matches!(right, Expr::Identifier(..) | Expr::IndexAccess { .. })
            && self.last_expr_schema.is_none()
        {
            // Adopt Int only if the identifier has a confirmed Int type.
            // Otherwise promote to Number to avoid misclassifying floats as ints.
            let safe = self.safe_adopt_numeric_hint(right, known);
            // Only adopt if the type didn't change (confirmed match).
            // If safe != known, skip adoption — let the operation fall through
            // to inference or generic opcodes that handle mixed types at runtime.
            if safe == known {
                *right_numeric = Some(safe);
                self.seed_numeric_hint_from_expr(right, safe);
            }
            return;
        }

        if let (None, Some(known)) = (*left_numeric, *right_numeric)
            && matches!(left, Expr::Identifier(..) | Expr::IndexAccess { .. })
        {
            // Do not adopt a numeric hint for identifiers that are typed objects.
            let has_object_schema = if let Expr::Identifier(name, _) = left {
                self.resolve_local(name)
                    .and_then(|idx| self.type_tracker.get_local_type(idx))
                    .and_then(|info| info.schema_id)
                    .is_some()
            } else {
                false
            };
            if !has_object_schema {
                let safe = self.safe_adopt_numeric_hint(left, known);
                // Only adopt if the type didn't change (confirmed match).
                if safe == known {
                    *left_numeric = Some(safe);
                    self.seed_numeric_hint_from_expr(left, safe);
                }
            }
        }
    }

    /// When adopting a numeric hint from one operand to another, check if adopting
    /// Int is safe. If the target identifier has no confirmed Int type from the
    /// type tracker, promote to Number to avoid emitting Int-typed opcodes for
    /// values that may actually be floats at runtime.
    fn safe_adopt_numeric_hint(&self, expr: &Expr, hint: NumericType) -> NumericType {
        if hint != NumericType::Int {
            return hint;
        }
        // Check if the identifier has a confirmed numeric type
        if let Expr::Identifier(name, _) = expr {
            if let Some(local_idx) = self.resolve_local(name) {
                if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                    if info.storage_hint == crate::type_tracking::StorageHint::Int64 {
                        return NumericType::Int;
                    }
                }
            }
        }
        // For unconfirmed types, use Number (safe for both int and float values)
        NumericType::Number
    }

    /// Returns `true` when the expression is syntactically guaranteed to be numeric.
    /// This does NOT consult the type tracker — it only looks at the AST node itself.
    fn is_expr_confirmed_numeric(expr: &Expr) -> bool {
        match expr {
            Expr::Literal(Literal::Int(_), _) | Expr::Literal(Literal::Number(_), _) => true,
            Expr::UnaryOp { op: UnaryOp::Neg, operand, .. } => {
                Self::is_expr_confirmed_numeric(operand)
            }
            _ => false,
        }
    }

    /// Get the compile-time StorageHint for an expression, if it can be determined.
    ///
    /// Only returns a hint for identifiers that are immutable (`let` bindings),
    /// since mutable variables (`var`) can be modified through reference parameters
    /// (DerefStore) and their runtime type may diverge from the tracker's static view.
    fn storage_hint_for_expr(&self, expr: &Expr) -> Option<crate::type_tracking::StorageHint> {
        match expr {
            Expr::Identifier(name, _) => {
                let local_idx = self.resolve_local(name)?;
                // Don't trust storage hints for function parameters with no explicit
                // type annotation — their inferred types (from inferred_param_type_hints)
                // can be wrong (e.g., a string param inferred as numeric → B19).
                if self.param_locals.contains(&local_idx) {
                    return None;
                }
                let info = self.type_tracker.get_local_type(local_idx)?;
                if info.storage_hint != crate::type_tracking::StorageHint::Unknown {
                    Some(info.storage_hint)
                } else {
                    None
                }
            }
            Expr::Literal(Literal::Int(_), _) => Some(crate::type_tracking::StorageHint::Int64),
            Expr::Literal(Literal::Number(_), _) => {
                Some(crate::type_tracking::StorageHint::Float64)
            }
            _ => None,
        }
    }

    #[allow(dead_code)]
    fn emit_numeric_binary_with_coercion(
        &mut self,
        op: &BinaryOp,
        left_numeric: Option<NumericType>,
        right_numeric: Option<NumericType>,
        is_comparison: bool,
    ) -> NumericEmitResult {
        self.emit_numeric_binary_with_coercion_inner(
            op,
            left_numeric,
            right_numeric,
            is_comparison,
            None,
            None,
        )
    }

    fn emit_numeric_binary_with_coercion_trusted(
        &mut self,
        op: &BinaryOp,
        left_numeric: Option<NumericType>,
        right_numeric: Option<NumericType>,
        is_comparison: bool,
        left_expr: &Expr,
        right_expr: &Expr,
    ) -> NumericEmitResult {
        let lhs_hint = self.storage_hint_for_expr(left_expr);
        let rhs_hint = self.storage_hint_for_expr(right_expr);
        self.emit_numeric_binary_with_coercion_inner(
            op,
            left_numeric,
            right_numeric,
            is_comparison,
            lhs_hint,
            rhs_hint,
        )
    }

    fn emit_numeric_binary_with_coercion_inner(
        &mut self,
        op: &BinaryOp,
        left_numeric: Option<NumericType>,
        right_numeric: Option<NumericType>,
        is_comparison: bool,
        lhs_hint: Option<crate::type_tracking::StorageHint>,
        rhs_hint: Option<crate::type_tracking::StorageHint>,
    ) -> NumericEmitResult {
        let Some(plan) = plan_coercion(left_numeric, right_numeric) else {
            return NumericEmitResult::NoPlan;
        };

        // u64 + signed is a compile error — must use explicit `as` cast
        if let CoercionPlan::IncompatibleWidths(a, b) = plan {
            self.errors
                .push(shape_ast::error::ShapeError::SemanticError {
                    message: format!(
                        "cannot mix `{}` and `{}` in arithmetic — use an explicit `as` cast",
                        a.type_name(),
                        b.type_name()
                    ),
                    location: None,
                });
            return NumericEmitResult::NoPlan;
        }

        let result_type = apply_coercion(self, plan);
        if let Some(guarded_opcode) = typed_opcode_for(op, result_type) {
            // Try to upgrade to trusted variant if both operand hints are known
            let opcode = if let (Some(lh), Some(rh)) = (lhs_hint, rhs_hint) {
                try_trusted_opcode(op, result_type, lh, rh).unwrap_or(guarded_opcode)
            } else {
                guarded_opcode
            };
            // Compact typed opcodes (AddTyped, etc.) need Width operand
            if let NumericType::IntWidth(w) = result_type {
                self.emit(Instruction::new(
                    opcode,
                    Some(Operand::Width(NumericWidth::from_int_width(w))),
                ));
            } else {
                self.emit(Instruction::simple(opcode));
            }
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = if is_comparison {
                None
            } else {
                Some(result_type)
            };
            NumericEmitResult::EmittedTyped
        } else {
            NumericEmitResult::CoercedNeedsGeneric
        }
    }

    /// Compile a binary operation expression
    pub(super) fn compile_expr_binary_op(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
    ) -> Result<()> {
        match op {
            BinaryOp::And => {
                self.compile_expr(left)?;
                let false_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

                self.compile_expr(right)?;
                self.emit(Instruction::simple(OpCode::Not));
                self.emit(Instruction::simple(OpCode::Not));
                let end_jump = self.emit_jump(OpCode::Jump, 0);

                self.patch_jump(false_jump);
                self.emit_bool(false);
                self.patch_jump(end_jump);
                // Boolean result — not a TypedObject or numeric
                self.last_expr_schema = None;
                self.last_expr_numeric_type = None;
            }
            BinaryOp::Or => {
                self.compile_expr(left)?;
                let true_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

                self.compile_expr(right)?;
                self.emit(Instruction::simple(OpCode::Not));
                self.emit(Instruction::simple(OpCode::Not));
                let end_jump = self.emit_jump(OpCode::Jump, 0);

                self.patch_jump(true_jump);
                self.emit_bool(true);
                self.patch_jump(end_jump);
                // Boolean result — not a TypedObject or numeric
                self.last_expr_schema = None;
                self.last_expr_numeric_type = None;
            }
            BinaryOp::NullCoalesce => {
                // Short-circuit null coalescing: a ?? b
                // Only evaluate RHS if LHS is None.
                //
                // Stack discipline:
                //   1. compile LHS          -> [lhs]
                //   2. Dup                   -> [lhs, lhs]
                //   3. PushNull              -> [lhs, lhs, null]
                //   4. Eq                    -> [lhs, is_none]
                //   5. JumpIfFalse use_lhs   -> [lhs]  (lhs is not None)
                //   6. Pop                   -> []      (discard None lhs)
                //   7. compile RHS           -> [rhs]
                //   8. Jump end
                //   use_lhs:                 -> [lhs]   (already on stack)
                //   end:
                self.compile_expr(left)?;
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::simple(OpCode::PushNull));
                self.emit(Instruction::simple(OpCode::Eq));
                let use_lhs_jump = self.emit_jump(OpCode::JumpIfFalse, 0);
                // LHS was None — pop it, compile RHS
                self.emit(Instruction::simple(OpCode::Pop));
                self.compile_expr(right)?;
                let end_jump = self.emit_jump(OpCode::Jump, 0);
                // LHS was not None — it's already on the stack
                self.patch_jump(use_lhs_jump);
                self.patch_jump(end_jump);
                self.last_expr_schema = None;
                self.last_expr_numeric_type = None;
            }
            BinaryOp::Pipe => {
                // Pipe operator: a |> f transforms to f(a)
                // a |> f(x) transforms to f(a, x)
                match right {
                    Expr::FunctionCall {
                        name,
                        args,
                        named_args,
                        span,
                    } => {
                        // a |> f(x, y) -> f(a, x, y)
                        let mut new_args = vec![left.clone()];
                        new_args.extend(args.iter().cloned());
                        let new_call = Expr::FunctionCall {
                            name: name.clone(),
                            args: new_args,
                            named_args: named_args.clone(),
                            span: *span,
                        };
                        self.compile_expr(&new_call)?;
                    }
                    Expr::MethodCall {
                        receiver,
                        method,
                        args,
                        named_args,
                        span,
                    } => {
                        // a |> obj.method(x) -> obj.method(a, x)
                        let mut new_args = vec![left.clone()];
                        new_args.extend(args.iter().cloned());
                        let new_call = Expr::MethodCall {
                            receiver: receiver.clone(),
                            method: method.clone(),
                            args: new_args,
                            named_args: named_args.clone(),
                            span: *span,
                        };
                        self.compile_expr(&new_call)?;
                    }
                    Expr::Identifier(name, span) => {
                        // a |> f -> f(a)
                        let new_call = Expr::FunctionCall {
                            name: name.clone(),
                            args: vec![left.clone()],
                            named_args: vec![],
                            span: *span,
                        };
                        self.compile_expr(&new_call)?;
                    }
                    _ => {
                        return Err(ShapeError::RuntimeError {
                            message:
                                "Pipe operator requires a function or method call on the right side"
                                    .to_string(),
                            location: None,
                        });
                    }
                }
            }
            BinaryOp::Add => {
                // For Add, check if we can do typed merge optimization
                self.compile_expr(left)?;
                let left_schema = self.last_expr_schema.take();
                let mut left_numeric = self.last_expr_numeric_type;

                self.compile_expr(right)?;
                let right_schema = self.last_expr_schema.take();
                let mut right_numeric = self.last_expr_numeric_type;

                // If one side is numeric and the other is an identifier/index read with no
                // hint yet, adopt the known numeric kind and seed slot hints.
                self.adopt_missing_numeric_operand_hint(
                    left,
                    right,
                    &mut left_numeric,
                    &mut right_numeric,
                );

                // Priority 1: typed object merge (both operands are TypedObjects)
                // Exception: if the left type implements Add, skip merge and emit
                // generic Add so the executor's operator trait dispatch handles it.
                if let (Some(left_id), Some(right_id)) = (left_schema, right_schema) {
                    let left_has_add = self
                        .type_tracker
                        .schema_registry()
                        .get_by_id(left_id)
                        .is_some_and(|schema| {
                            self.type_inference
                                .env
                                .type_implements_trait(&schema.name, "Add")
                        });
                    if left_has_add {
                        // Operator trait: emit generic Add for executor dispatch
                        self.emit(Instruction::simple(OpCode::Add));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = None;
                    } else {
                        self.compile_typed_merge(left_id, right_id)?;
                        self.last_expr_numeric_type = None;
                    }
                }
                // Priority 2: typed numeric add (same types or mixed Int/Number with coercion)
                //
                // Add is overloaded (numeric add, string concat, array concat,
                // object merge).  Only emit typed numeric opcodes when we have
                // *direct* evidence that both operands are numeric — i.e. each
                // is either a numeric literal or an immutable local whose
                // storage hint is a numeric family.  Without that evidence the
                // `last_expr_numeric_type` values may come from speculative
                // inference hints (inferred_param_type_hints) which can be wrong
                // when a param is actually a string.
                else {
                    // Confirm each operand is numeric via one of three paths:
                    // 1. Syntactic: it's a numeric literal
                    // 2. Storage hint: it's a local with a known numeric hint
                    //    (excludes untyped function params — see param_locals)
                    // 3. Non-identifier with type tracker info: for expressions
                    //    like a[0], foo.bar, x*y — the type tracker is reliable
                    //    because the B19 mistyping only affects bare param identifiers
                    let lhs_confirmed = Self::is_expr_confirmed_numeric(left)
                        || self.storage_hint_for_expr(left)
                            .is_some_and(|h| h.is_default_int_family() || h.is_float_family())
                        || (!matches!(left, Expr::Identifier(..)) && left_numeric.is_some());
                    let rhs_confirmed = Self::is_expr_confirmed_numeric(right)
                        || self.storage_hint_for_expr(right)
                            .is_some_and(|h| h.is_default_int_family() || h.is_float_family())
                        || (!matches!(right, Expr::Identifier(..)) && right_numeric.is_some());

                    let primary = if lhs_confirmed && rhs_confirmed {
                        self.emit_numeric_binary_with_coercion_trusted(
                            &BinaryOp::Add,
                            left_numeric,
                            right_numeric,
                            false,
                            left,
                            right,
                        )
                    } else {
                        NumericEmitResult::NoPlan
                    };
                    match primary {
                        NumericEmitResult::EmittedTyped => {
                            self.last_expr_schema = None;
                        }
                        NumericEmitResult::CoercedNeedsGeneric | NumericEmitResult::NoPlan => {
                            // Generic Add (string concat, array concat, etc.)
                            self.emit(Instruction::simple(OpCode::Add));
                            self.last_expr_schema = None;
                            self.last_expr_type_info = None;
                            self.last_expr_numeric_type = None;
                        }
                    }
                }
            }
            _ => {
                // Typed matrix kernels: Mat<number> * Vec<number>/Mat<number>.
                // Lower before generic strict-arithmetic checks so typed matrix
                // paths never fall back to scalar arithmetic dispatch.
                if matches!(op, BinaryOp::Mul) && self.try_compile_typed_matrix_mul(left, right)? {
                    return Ok(());
                }

                // ── Compile-time type safety for strict arithmetic ──
                // Sub, Mul, Div, Mod, Pow require numeric operands.
                // If both types are known and either is non-numeric → compile error.
                if is_strict_arithmetic(op) {
                    if let (Ok(lt), Ok(rt)) =
                        (self.infer_expr_type(left), self.infer_expr_type(right))
                    {
                        // `infer_expr_type` runs outside the compiler's local-slot context.
                        // For identifiers that are currently bound in bytecode locals/module_bindings,
                        // an inferred Function type may be a shadowed builtin (e.g. `len`).
                        // In that case, defer to slot-based tracking below to avoid false errors.
                        let left_shadowed_builtin = matches!(left, Expr::Identifier(name, _)
                            if (self.resolve_local(name).is_some() || self.module_bindings.contains_key(name))
                                && is_function_type(&lt));
                        let right_shadowed_builtin = matches!(right, Expr::Identifier(name, _)
                            if (self.resolve_local(name).is_some() || self.module_bindings.contains_key(name))
                                && is_function_type(&rt));

                        if left_shadowed_builtin || right_shadowed_builtin {
                            // Skip this early semantic gate for shadowed identifiers.
                            // The typed/local tracking pass below will still enforce arithmetic safety.
                        } else if !is_type_numeric(&lt) || !is_type_numeric(&rt) {
                            // Check if the left operand's type implements an operator trait
                            // for this operation (e.g. impl Mul for Vec2). If so, allow the
                            // generic opcode through to the executor's trait dispatch.
                            let has_operator_trait = operator_trait_for_op(op)
                                .and_then(|trait_name| {
                                    let type_name = type_display_name(&lt);
                                    if self
                                        .type_inference
                                        .env
                                        .type_implements_trait(&type_name, trait_name)
                                    {
                                        Some(())
                                    } else {
                                        None
                                    }
                                })
                                .is_some();

                            if !has_operator_trait {
                                let op_symbol = match op {
                                    BinaryOp::Sub => "-",
                                    BinaryOp::Mul => "*",
                                    BinaryOp::Div => "/",
                                    BinaryOp::Mod => "%",
                                    BinaryOp::Pow => "**",
                                    _ => "?",
                                };
                                return Err(ShapeError::SemanticError {
                                    message: format!(
                                        "Cannot apply '{}' to {} and {}. Both operands must be numeric (int, number, or decimal).",
                                        op_symbol,
                                        type_display_name(&lt),
                                        type_display_name(&rt),
                                    ),
                                    location: Some(
                                        self.span_to_source_location(combined_span(left, right)),
                                    ),
                                });
                            }
                        }
                    }
                }

                // ── Compile operands, capture numeric types and schemas ──
                self.compile_expr(left)?;
                let mut left_numeric = self.last_expr_numeric_type;
                let left_schema = self.last_expr_schema;
                self.compile_expr(right)?;
                let mut right_numeric = self.last_expr_numeric_type;
                let right_schema = self.last_expr_schema;


                // Don't trust inferred numeric types for untyped function parameters.
                // Their inferred_param_type_hints can be wrong (same rationale as the
                // param_locals guard in storage_hint_for_expr for Add).  Without an
                // explicit type annotation the parameter may receive values of any
                // type at runtime, so fall back to generic opcodes.
                if let Expr::Identifier(name, _) = left {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.param_locals.contains(&local_idx) {
                            left_numeric = None;
                        }
                    }
                }
                if let Expr::Identifier(name, _) = right {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.param_locals.contains(&local_idx) {
                            right_numeric = None;
                        }
                    }
                }

                // Strict arithmetic requires numeric operands. If an indexed access
                // lacks a concrete hint, treat it as numeric to enable typed lowering.
                if is_strict_arithmetic(op) {
                    if left_numeric.is_none() && matches!(left, Expr::IndexAccess { .. }) {
                        left_numeric = Some(NumericType::Number);
                    }
                    if right_numeric.is_none() && matches!(right, Expr::IndexAccess { .. }) {
                        right_numeric = Some(NumericType::Number);
                    }
                }

                if is_strict_arithmetic(op) || is_ordered_comparison(op) {
                    self.adopt_missing_numeric_operand_hint(
                        left,
                        right,
                        &mut left_numeric,
                        &mut right_numeric,
                    );
                }

                // ── Schema-based type safety (catches objects in arithmetic) ──
                // If an operand has a schema (it's a TypedObject) but no numeric type,
                // it's an object being used in arithmetic → compile error.
                // Exception: if the left type implements an operator trait for this op.
                if is_strict_arithmetic(op) {
                    let left_is_object = left_schema.is_some() && left_numeric.is_none();
                    let right_is_object = right_schema.is_some() && right_numeric.is_none();
                    if left_is_object || right_is_object {
                        // Check if the left operand's type implements an operator trait
                        let has_operator_trait = left_schema
                            .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                            .and_then(|schema| {
                                operator_trait_for_op(op).filter(|trait_name| {
                                    self.type_inference
                                        .env
                                        .type_implements_trait(&schema.name, trait_name)
                                })
                            })
                            .is_some();

                        if !has_operator_trait {
                            let op_symbol = match op {
                                BinaryOp::Sub => "-",
                                BinaryOp::Mul => "*",
                                BinaryOp::Div => "/",
                                BinaryOp::Mod => "%",
                                BinaryOp::Pow => "**",
                                _ => "?",
                            };
                            let left_desc = if left_is_object { "object" } else { "numeric" };
                            let right_desc = if right_is_object { "object" } else { "numeric" };
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "Cannot apply '{}' to {} and {}. Both operands must be numeric (int, number, or decimal).",
                                    op_symbol, left_desc, right_desc,
                                ),
                                location: Some(
                                    self.span_to_source_location(combined_span(left, right)),
                                ),
                            });
                        }
                    }
                }

                // ── Emit typed opcode (with coercion for mixed Int/Number) ──
                let is_comparison = is_ordered_comparison(op);
                let emit_result = self.emit_numeric_binary_with_coercion_trusted(
                    op,
                    left_numeric,
                    right_numeric,
                    is_comparison,
                    left,
                    right,
                );
                match emit_result {
                    NumericEmitResult::EmittedTyped => {}
                    NumericEmitResult::CoercedNeedsGeneric => {
                        // Op has no typed variant (e.g., Eq, Neq, bitwise).
                        self.compile_binary_op(op)?;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = None;
                    }
                    NumericEmitResult::NoPlan => {
                        // Types unknown from slot tracking — try inference engine.
                        let (inferred_left, inferred_right) = self.infer_numeric_pair(left, right);
                        match self.emit_numeric_binary_with_coercion_trusted(
                            op,
                            inferred_left,
                            inferred_right,
                            is_comparison,
                            left,
                            right,
                        ) {
                            NumericEmitResult::EmittedTyped => {}
                            _ => {
                                self.compile_binary_op(op)?;
                                self.last_expr_type_info = None;
                                self.last_expr_numeric_type = None;
                            }
                        }
                    }
                }
                self.last_expr_schema = None;
            }
        }
        Ok(())
    }

    /// Compile a fuzzy comparison expression with tolerance.
    /// Desugars to arithmetic operations — no dedicated fuzzy VM opcodes needed.
    pub(super) fn compile_expr_fuzzy_comparison(
        &mut self,
        left: &Expr,
        op: &FuzzyOp,
        right: &Expr,
        tolerance: &FuzzyTolerance,
    ) -> Result<()> {
        use crate::bytecode::{Constant, Operand};

        // Store left and right in temp locals to avoid re-evaluation
        let temp_a = self.declare_temp_local("__fuzzy_a")?;
        let temp_b = self.declare_temp_local("__fuzzy_b")?;

        self.compile_expr(left)?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(temp_a)),
        ));
        self.compile_expr(right)?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(temp_b)),
        ));

        // Helper: emit abs(a - b) → load a, load b, Sub, Dup, push 0, Lt, JumpIfFalse(skip), Neg, skip:
        // This computes abs(top-of-stack) inline
        let emit_abs_diff = |compiler: &mut BytecodeCompiler| {
            compiler.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(temp_a)),
            ));
            compiler.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(temp_b)),
            ));
            compiler.emit(Instruction::simple(OpCode::Sub));
            // abs: dup, push 0, Lt, JumpIfFalse(skip), Neg
            compiler.emit(Instruction::simple(OpCode::Dup));
            let zero_idx = compiler.program.add_constant(Constant::Number(0.0));
            compiler.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(zero_idx)),
            ));
            compiler.emit(Instruction::simple(OpCode::Lt));
            let skip = compiler.emit_jump(OpCode::JumpIfFalse, 0);
            compiler.emit(Instruction::simple(OpCode::Neg));
            compiler.patch_jump(skip);
        };

        match (op, tolerance) {
            (FuzzyOp::Equal, FuzzyTolerance::Absolute(tol)) => {
                // abs(a - b) <= tol
                emit_abs_diff(self);
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::Lte));
            }
            (FuzzyOp::Equal, FuzzyTolerance::Percentage(tol)) => {
                // abs(a - b) / ((abs(a) + abs(b)) / 2) <= tol
                // numerator: abs(a - b)
                emit_abs_diff(self);
                // denominator: (abs(a) + abs(b)) / 2
                // abs(a)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let zero_idx2 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(zero_idx2)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let skip_a = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::Neg));
                self.patch_jump(skip_a);
                // abs(b)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let zero_idx3 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(zero_idx3)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let skip_b = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::Neg));
                self.patch_jump(skip_b);
                // (abs(a) + abs(b)) / 2
                self.emit(Instruction::simple(OpCode::Add));
                let two_idx = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two_idx)),
                ));
                self.emit(Instruction::simple(OpCode::Div));
                // numerator / denominator <= tol
                self.emit(Instruction::simple(OpCode::Div));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::Lte));
            }
            (FuzzyOp::Greater, FuzzyTolerance::Absolute(tol)) => {
                // a > b || abs(a - b) <= tol
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Gt));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                emit_abs_diff(self);
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::Lte));
                let end2 = self.emit_jump(OpCode::Jump, 0);
                self.patch_jump(end);
                self.emit_bool(true);
                self.patch_jump(end2);
            }
            (FuzzyOp::Greater, FuzzyTolerance::Percentage(tol)) => {
                // a > b || (percentage within tolerance)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Gt));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                // Reuse percentage tolerance check
                emit_abs_diff(self);
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let z1 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(z1)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let sa = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::Neg));
                self.patch_jump(sa);
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let z2 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(z2)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let sb = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::Neg));
                self.patch_jump(sb);
                self.emit(Instruction::simple(OpCode::Add));
                let two = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two)),
                ));
                self.emit(Instruction::simple(OpCode::Div));
                self.emit(Instruction::simple(OpCode::Div));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::Lte));
                let end2 = self.emit_jump(OpCode::Jump, 0);
                self.patch_jump(end);
                self.emit_bool(true);
                self.patch_jump(end2);
            }
            (FuzzyOp::Less, FuzzyTolerance::Absolute(tol)) => {
                // a < b || abs(a - b) <= tol
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                emit_abs_diff(self);
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::Lte));
                let end2 = self.emit_jump(OpCode::Jump, 0);
                self.patch_jump(end);
                self.emit_bool(true);
                self.patch_jump(end2);
            }
            (FuzzyOp::Less, FuzzyTolerance::Percentage(tol)) => {
                // a < b || (percentage within tolerance)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                emit_abs_diff(self);
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let z1 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(z1)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let sa = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::Neg));
                self.patch_jump(sa);
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let z2 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(z2)),
                ));
                self.emit(Instruction::simple(OpCode::Lt));
                let sb = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::Neg));
                self.patch_jump(sb);
                self.emit(Instruction::simple(OpCode::Add));
                let two = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two)),
                ));
                self.emit(Instruction::simple(OpCode::Div));
                self.emit(Instruction::simple(OpCode::Div));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::Lte));
                let end2 = self.emit_jump(OpCode::Jump, 0);
                self.patch_jump(end);
                self.emit_bool(true);
                self.patch_jump(end2);
            }
        }

        Ok(())
    }

    /// Compile a typed object merge (a + b where both are TypedObjects)
    ///
    /// This registers the intersection schema at compile time and emits
    /// TypedMergeObject for O(1) memcpy-based merge.
    fn compile_typed_merge(&mut self, left_id: SchemaId, right_id: SchemaId) -> Result<()> {
        let registry = self.type_tracker.schema_registry();

        let left_schema = registry
            .get_by_id(left_id)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Unknown left schema ID: {}", left_id),
                location: None,
            })?;
        let right_schema =
            registry
                .get_by_id(right_id)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!("Unknown right schema ID: {}", right_id),
                    location: None,
                })?;

        // Calculate sizes (8 bytes per field)
        let left_size = left_schema.fields.len() * 8;
        let right_size = right_schema.fields.len() * 8;

        // Build merged field list
        let mut merged_fields: Vec<(String, FieldType)> = Vec::new();
        for f in &left_schema.fields {
            merged_fields.push((f.name.clone(), f.field_type.clone()));
        }
        for f in &right_schema.fields {
            merged_fields.push((f.name.clone(), f.field_type.clone()));
        }

        // Register intersection schema
        let merged_name = format!("__intersection_{}_{}", left_id, right_id);
        let target_id = self
            .type_tracker
            .schema_registry_mut()
            .register_type(merged_name, merged_fields);

        // Emit TypedMergeObject
        self.emit(Instruction::new(
            OpCode::TypedMergeObject,
            Some(Operand::TypedMerge {
                target_schema_id: target_id as u16,
                left_size: left_size as u16,
                right_size: right_size as u16,
            }),
        ));

        // Track result schema for chained operations (e.g., a + b + c)
        self.last_expr_schema = Some(target_id);
        self.last_expr_type_info = Some(VariableTypeInfo::known(
            target_id,
            format!("__intersection_{}_{}", left_id, right_id),
        ));

        Ok(())
    }
}
