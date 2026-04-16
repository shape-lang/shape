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
    type_display_name, typed_opcode_for,
};

/// Map a strict arithmetic BinaryOp to its operator trait name, if one exists.
fn operator_trait_for_op(op: &BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Sub => Some("Sub"),
        BinaryOp::Mul => Some("Mul"),
        BinaryOp::Div => Some("Div"),
        BinaryOp::Greater | BinaryOp::Less | BinaryOp::GreaterEq | BinaryOp::LessEq => {
            Some("Ord")
        }
        _ => None, // Mod, Pow have no operator trait
    }
}

/// Map a binary op to the user-facing trait method name (lowercase).
/// Used by Phase 2.5 to emit `CallMethod("add"/"sub"/...)` for operator
/// overloading on user-defined types. The runtime dispatches via
/// `function_name_index["{Type}::{method}"]` (see `op_call_method` →
/// `handle_typed_object_method`).
fn operator_trait_method_for_op(op: &BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Add => Some("add"),
        BinaryOp::Sub => Some("sub"),
        BinaryOp::Mul => Some("mul"),
        BinaryOp::Div => Some("div"),
        BinaryOp::Greater | BinaryOp::Less | BinaryOp::GreaterEq | BinaryOp::LessEq => {
            Some("cmp")
        }
        _ => None,
    }
}

fn emit_cmp_result_comparison(compiler: &mut BytecodeCompiler, op: &BinaryOp) {
    use crate::bytecode::Constant;
    let zero_idx = compiler.program.add_constant(Constant::Int(0));
    compiler.emit(Instruction::new(OpCode::PushConst, Some(Operand::Const(zero_idx))));
    let cmp_op = match op {
        BinaryOp::Greater => OpCode::GtInt,
        BinaryOp::Less => OpCode::LtInt,
        BinaryOp::GreaterEq => OpCode::GteInt,
        BinaryOp::LessEq => OpCode::LteInt,
        _ => unreachable!(),
    };
    compiler.emit(Instruction::simple(cmp_op));
}

fn try_emit_trait_dispatch(compiler: &mut BytecodeCompiler, op: &BinaryOp, left_schema: Option<SchemaId>, left_expr: &Expr) -> bool {
    let trait_name = match operator_trait_for_op(op) { Some(t) => t, None => return false };
    let method_name = match operator_trait_method_for_op(op) { Some(m) => m, None => return false };
    let has_trait_via_schema = left_schema
        .and_then(|sid| compiler.type_tracker.schema_registry().get_by_id(sid))
        .is_some_and(|schema| compiler.type_inference.env.type_implements_trait(&schema.name, trait_name));
    let has_trait = has_trait_via_schema || compiler.infer_expr_type(left_expr).ok().is_some_and(|ty| {
        let name = type_display_name(&ty);
        compiler.type_inference.env.type_implements_trait(&name, trait_name)
    });
    if !has_trait { return false; }
    emit_operator_trait_call(compiler, method_name);
    if is_ordered_comparison(op) { emit_cmp_result_comparison(compiler, op); }
    true
}

/// Emit a `CallMethod` instruction targeting an operator trait method
/// (e.g. `Vec2::add`). Both operands must already be on the stack: receiver
/// first, then the right-hand-side argument.
fn emit_operator_trait_call(compiler: &mut BytecodeCompiler, method_name: &'static str) {
    let method_id = shape_value::MethodId::from_name(method_name);
    let string_id = compiler.program.add_string(method_name.to_string());
    compiler.emit(Instruction::new(
        OpCode::CallMethod,
        Some(Operand::TypedMethodCall {
            method_id: method_id.0,
            arg_count: 1,
            string_id,
         receiver_type_tag: 0xFF, }),
    ));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

fn combined_span(left: &Expr, right: &Expr) -> Span {
    let ls = left.span();
    let rs = right.span();
    Span::new(ls.start.min(rs.start), ls.end.max(rs.end))
}

/// Emit the appropriate runtime-dispatched opcode for the given binary
/// operation, routing through the centralized helpers in `helpers.rs`.
/// Returns `true` if a helper was used, `false` if the caller should
/// fall through to `compile_binary_op()` for remaining ops (And/Or/BitOps/
/// NullCoalesce/ErrorContext).
fn emit_generic_via_helper(compiler: &mut BytecodeCompiler, op: &BinaryOp) -> bool {
    use crate::compiler::helpers;
    match op {
        BinaryOp::Add => { helpers::emit_dynamic_add(compiler); true }
        BinaryOp::Sub => { helpers::emit_dynamic_sub(compiler); true }
        BinaryOp::Mul => { helpers::emit_dynamic_mul(compiler); true }
        BinaryOp::Div => { helpers::emit_dynamic_div(compiler); true }
        BinaryOp::Mod => { helpers::emit_dynamic_mod(compiler); true }
        BinaryOp::Pow => { helpers::emit_dynamic_pow(compiler); true }
        BinaryOp::Greater => { helpers::emit_dynamic_gt(compiler); true }
        BinaryOp::Less => { helpers::emit_dynamic_lt(compiler); true }
        BinaryOp::GreaterEq => { helpers::emit_dynamic_gte(compiler); true }
        BinaryOp::LessEq => { helpers::emit_dynamic_lte(compiler); true }
        BinaryOp::Equal => { helpers::emit_dynamic_eq(compiler); true }
        BinaryOp::NotEqual => { helpers::emit_dynamic_neq(compiler); true }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericEmitResult {
    EmittedTyped,
    CoercedNeedsGeneric,
    NoPlan,
}

/// Simplified type category for equality dispatch.
/// Collapses int-width variants to `Int` and char to `String` (EqString
/// handles both heap-boxed string and char values via `as_str()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EqOperandType {
    Int,
    Number,
    Decimal,
    String,
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
            Expr::Literal(Literal::Int(_), _)
            | Expr::Literal(Literal::Number(_), _)
            | Expr::Literal(Literal::TypedInt(..), _)
            | Expr::Literal(Literal::UInt(_), _)
            | Expr::Literal(Literal::Decimal(_), _) => true,
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                operand,
                ..
            } => Self::is_expr_confirmed_numeric(operand),
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
        _lhs_hint: Option<crate::type_tracking::StorageHint>,
        _rhs_hint: Option<crate::type_tracking::StorageHint>,
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
        if let Some(opcode) = typed_opcode_for(op, result_type) {
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

    /// Phase 2.6.5.3/4: inference-driven typed equality dispatch.
    ///
    /// Architectural shift: resolve operand types from multiple sources
    /// BEFORE compiling them, then pick the typed `Eq*`/`Neq*` opcode.
    ///
    /// Type resolution priority:
    /// 1. Type inference engine (`infer_expr_type`)
    /// 2. AST literal type (for `Literal::Int`, `Literal::String`, etc.)
    /// 3. Asymmetric propagation: if one side is typed and the other is not,
    ///    assume both sides have the same type. This is safe because typed
    ///    comparison opcodes return false for mismatched runtime types.
    ///
    /// Returns `Ok(true)` if a typed opcode was emitted, `Ok(false)` to
    /// fall through to the legacy slot-tracker dispatch.
    fn compile_typed_equality(
        &mut self,
        op: &BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<bool> {
        if !matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
            return Ok(false);
        }

        let is_neq = matches!(op, BinaryOp::NotEqual);

        // Desugar `x == None` / `None == x` to IsNull(x).
        // This covers Option<T> comparisons and any None-literal equality.
        if matches!(right, Expr::Literal(Literal::None, _)) {
            self.compile_expr(left)?;
            self.emit(Instruction::simple(OpCode::IsNull));
            if is_neq {
                self.emit(Instruction::simple(OpCode::Not));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = None;
            return Ok(true);
        }
        if matches!(left, Expr::Literal(Literal::None, _)) {
            self.compile_expr(right)?;
            self.emit(Instruction::simple(OpCode::IsNull));
            if is_neq {
                self.emit(Instruction::simple(OpCode::Not));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = None;
            return Ok(true);
        }

        // Resolve operand types from inference + literal fallback.
        let mut lhs_eq = self.resolve_eq_type(left);
        let mut rhs_eq = self.resolve_eq_type(right);

        // Asymmetric propagation: if one side is typed and the other is not,
        // propagate the known type. For `x == 5` where x is an untracked
        // loop counter, the literal 5 tells us to use EqInt.
        if lhs_eq.is_none() && rhs_eq.is_some() {
            lhs_eq = rhs_eq;
        } else if rhs_eq.is_none() && lhs_eq.is_some() {
            rhs_eq = lhs_eq;
        }

        // Pick the typed opcode and whether to negate after.
        // EqString/EqDecimal have no Neq variants → emit Eq + Not for NotEqual.
        // EqInt/EqNumber have NeqInt/NeqNumber variants → use them directly.
        let emission = match (lhs_eq, rhs_eq) {
            (Some(EqOperandType::Int), Some(EqOperandType::Int)) => Some(if is_neq {
                (OpCode::NeqInt, false)
            } else {
                (OpCode::EqInt, false)
            }),
            (Some(EqOperandType::Number), Some(EqOperandType::Number)) => Some(if is_neq {
                (OpCode::NeqNumber, false)
            } else {
                (OpCode::EqNumber, false)
            }),
            (Some(EqOperandType::Decimal), Some(EqOperandType::Decimal)) => {
                Some((OpCode::EqDecimal, is_neq))
            }
            (Some(EqOperandType::String), Some(EqOperandType::String)) => {
                Some((OpCode::EqString, is_neq))
            }
            _ => None,
        };

        if let Some((opcode, needs_negate)) = emission {
            self.compile_expr(left)?;
            self.compile_expr(right)?;
            self.emit(Instruction::simple(opcode));
            if needs_negate {
                self.emit(Instruction::simple(OpCode::Not));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = None;
            return Ok(true);
        }

        // Fallback: both operand types are unresolved. Compile operands and
        // emit a runtime-dispatched equality (vw_equals in the executor).
        // This covers generic stdlib code, untyped function params, etc.
        self.compile_expr(left)?;
        self.compile_expr(right)?;
        if is_neq {
            crate::compiler::helpers::emit_dynamic_neq(self);
        } else {
            crate::compiler::helpers::emit_dynamic_eq(self);
        }
        Ok(true)
    }

    /// Resolve the equality-relevant type of an expression from multiple
    /// sources: inference engine, then AST literal kind.
    fn resolve_eq_type(&mut self, expr: &Expr) -> Option<EqOperandType> {
        // Source 1: type inference engine
        if let Ok(ty) = self.infer_expr_type(expr) {
            if let Some(nt) = inferred_type_to_numeric(&ty) {
                return Some(match nt {
                    NumericType::Int | NumericType::IntWidth(_) => EqOperandType::Int,
                    NumericType::Number => EqOperandType::Number,
                    NumericType::Decimal => EqOperandType::Decimal,
                });
            }
            let name = type_display_name(&ty);
            match name.as_str() {
                "string" | "char" => return Some(EqOperandType::String),
                _ => {}
            }
        }

        // Source 2: AST literal type
        match expr {
            Expr::Literal(Literal::Int(_) | Literal::UInt(_) | Literal::TypedInt(..), _) => {
                Some(EqOperandType::Int)
            }
            Expr::Literal(Literal::Number(_), _) => Some(EqOperandType::Number),
            Expr::Literal(Literal::Decimal(_), _) => Some(EqOperandType::Decimal),
            Expr::Literal(Literal::String(_), _) => Some(EqOperandType::String),
            _ => None,
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
                // Stack discipline (Stage 2.6.5.2: typed IsNull replaces PushNull;Eq):
                //   1. compile LHS          -> [lhs]
                //   2. Dup                   -> [lhs, lhs]
                //   3. IsNull                -> [lhs, is_none]
                //   4. JumpIfFalse use_lhs   -> [lhs]  (lhs is not None)
                //   5. Pop                   -> []      (discard None lhs)
                //   6. compile RHS           -> [rhs]
                //   7. Jump end
                //   use_lhs:                 -> [lhs]   (already on stack)
                //   end:
                self.compile_expr(left)?;
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::simple(OpCode::IsNull));
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
                        optional,
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
                            optional: *optional,
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
                        // Phase 2.5: operator trait dispatch via CallMethod.
                        // The left operand (receiver) and right operand (arg)
                        // are already on the stack from compile_expr above.
                        emit_operator_trait_call(self, "add");
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
                    // Priority 1.5: dedicated StringConcat / ArrayConcat for
                    // built-in heap types whose operand kinds the compiler can
                    // prove. These replace the heap-heap arms in `exec_arithmetic`
                    // (Phase 2.3 / 2.4) without going through the generic Add
                    // dispatch.
                    let inferred_lhs = self.infer_expr_type(left).ok();
                    let inferred_rhs = self.infer_expr_type(right).ok();
                    let lhs_name = inferred_lhs.as_ref().map(type_display_name);
                    let rhs_name = inferred_rhs.as_ref().map(type_display_name);

                    // String / Char concat: any combination of string + char,
                    // as long as at least one operand is a string. Char + Char
                    // also produces a string (matches the legacy heap-heap arms).
                    let is_strish =
                        |n: &Option<String>| matches!(n.as_deref(), Some("string") | Some("char"));
                    let either_is_string = matches!(lhs_name.as_deref(), Some("string"))
                        || matches!(rhs_name.as_deref(), Some("string"));
                    if is_strish(&lhs_name) && is_strish(&rhs_name) && either_is_string {
                        // Use the typed string concatenation opcode when both
                        // operands are proven strings/chars.
                        self.emit(Instruction::simple(OpCode::StringConcatTyped));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = None;
                        return Ok(());
                    }

                    // Array concat: both operands proven to be arrays. We
                    // intentionally only fire for the generic `Array<T>` shape,
                    // not for `Vec<number>`-style FloatArray/IntArray/BoolArray
                    // (which use element-wise SIMD broadcast for `+`, not concat).
                    // Display name comes from `type_display_name`: a generic
                    // `Array<T>` formats as "Array", and a legacy `T[]` formats
                    // as "T[]".
                    let is_arrayish = |n: &Option<String>| match n.as_deref() {
                        Some("Array") => true,
                        Some(s) if s.ends_with("[]") => true,
                        _ => false,
                    };
                    if is_arrayish(&lhs_name) && is_arrayish(&rhs_name) {
                        self.emit(Instruction::simple(OpCode::ArrayConcat));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = None;
                        return Ok(());
                    }

                    // DateTime/Duration addition: at least one side is
                    // DateTime or Duration (TimeSpan). Dispatch via
                    // CallMethod("add") so the executor's PHF-backed
                    // datetime/timespan method registry handles the
                    // type combinations. Replaces the generic Add path.
                    let is_temporal = |n: &Option<String>| {
                        matches!(
                            n.as_deref(),
                            Some("DateTime") | Some("Duration") | Some("TimeSpan")
                        )
                    };
                    if is_temporal(&lhs_name) || is_temporal(&rhs_name) {
                        let method_id = shape_value::MethodId::from_name("add");
                        let string_id = self.program.add_string("add".to_string());
                        self.emit(Instruction::new(OpCode::CallMethod, Some(Operand::TypedMethodCall {
                            method_id: method_id.0, arg_count: 1, string_id,
                         receiver_type_tag: 0xFF, })));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = None;
                        return Ok(());
                    }

                    // Path 4: if infer_expr_type resolved a numeric type
                    // name, fill in missing NumericType for the coercion
                    // planner.
                    let inferred_numeric = |n: &Option<String>| -> Option<NumericType> {
                        match n.as_deref() {
                            Some("int") => Some(NumericType::Int),
                            Some("number") => Some(NumericType::Number),
                            Some("decimal") => Some(NumericType::Decimal),
                            _ => None,
                        }
                    };
                    let lhs_inferred_num = inferred_numeric(&lhs_name);
                    let rhs_inferred_num = inferred_numeric(&rhs_name);
                    if left_numeric.is_none() && lhs_inferred_num.is_some() {
                        left_numeric = lhs_inferred_num;
                    }
                    if right_numeric.is_none() && rhs_inferred_num.is_some() {
                        right_numeric = rhs_inferred_num;
                    }

                    // Confirm each operand is numeric via one of four paths:
                    // 1. Syntactic: it's a numeric literal
                    // 2. Storage hint: it's a local with a known numeric hint
                    //    (excludes untyped function params — see param_locals)
                    // 3. Type tracker info, excluding only untyped function
                    //    params (param_locals) whose inferred hints can be
                    //    wrong (B19). Non-param identifiers (locals, for-loop
                    //    vars, module bindings) have reliable tracker info.
                    // 4. infer_expr_type resolved a numeric type name
                    let is_untyped_param = |e: &Expr| -> bool {
                        if let Expr::Identifier(name, _) = e {
                            if let Some(idx) = self.resolve_local(name) {
                                return self.param_locals.contains(&idx);
                            }
                        }
                        false
                    };
                    // Path 5: check if identifier resolves to a local whose
                    // type_name in the type tracker is numeric. This covers
                    // locals and for-loop variables that have a known type
                    // name but whose storage_hint is Unknown (not yet
                    // propagated).
                    let local_has_numeric_type_name = |e: &Expr| -> Option<NumericType> {
                        if let Expr::Identifier(name, _) = e {
                            if let Some(idx) = self.resolve_local(name) {
                                if self.param_locals.contains(&idx) {
                                    return None;
                                }
                                if let Some(info) = self.type_tracker.get_local_type(idx) {
                                    if let Some(ref tn) = info.type_name {
                                        return match tn.as_str() {
                                            "int" | "Int" | "Integer" | "i64" => Some(NumericType::Int),
                                            "number" | "Number" | "Float" | "f64" => Some(NumericType::Number),
                                            "decimal" | "Decimal" => Some(NumericType::Decimal),
                                            _ => None,
                                        };
                                    }
                                }
                            }
                        }
                        None
                    };
                    let lhs_local_num = local_has_numeric_type_name(left);
                    let rhs_local_num = local_has_numeric_type_name(right);
                    if left_numeric.is_none() && lhs_local_num.is_some() {
                        left_numeric = lhs_local_num;
                    }
                    if right_numeric.is_none() && rhs_local_num.is_some() {
                        right_numeric = rhs_local_num;
                    }
                    let lhs_confirmed = Self::is_expr_confirmed_numeric(left)
                        || self
                            .storage_hint_for_expr(left)
                            .is_some_and(|h| h.is_numeric_family())
                        || (!is_untyped_param(left) && left_numeric.is_some())
                        || lhs_inferred_num.is_some()
                        || lhs_local_num.is_some();
                    let rhs_confirmed = Self::is_expr_confirmed_numeric(right)
                        || self
                            .storage_hint_for_expr(right)
                            .is_some_and(|h| h.is_numeric_family())
                        || (!is_untyped_param(right) && right_numeric.is_some())
                        || rhs_inferred_num.is_some()
                        || rhs_local_num.is_some();

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
                            // Runtime-dispatched fallback for unresolvable operand
                            // types. The VM's exec_arithmetic handles type dispatch
                            // (numeric, string concat, DateTime, etc.) at runtime.
                            crate::compiler::helpers::emit_dynamic_add(self);
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

                // Phase 2.6.5.3: inference-driven typed Eq/Neq dispatch.
                // Queries the inference engine for both operand types BEFORE
                // compiling them and emits the typed opcode directly. This is
                // the PRIMARY path for Equal/NotEqual; the legacy slot-tracker
                // dispatch below is the secondary fallback for cases inference
                // can't resolve.
                if self.compile_typed_equality(op, left, right)? {
                    return Ok(());
                }

                // Stage 4.2: typed string ordered comparison.
                // When both operands are proven strings and the op is an
                // ordered comparison (>, <, >=, <=), emit the specialized
                // string comparison opcode for zero-dispatch execution.
                if is_ordered_comparison(op) {
                    if let (Ok(lt), Ok(rt)) = (self.infer_expr_type(left), self.infer_expr_type(right)) {
                        let lt_name = type_display_name(&lt);
                        let rt_name = type_display_name(&rt);
                        let is_strish = |n: &str| matches!(n, "string" | "char");
                        if is_strish(&lt_name) && is_strish(&rt_name) {
                            let string_cmp_op = match op {
                                BinaryOp::Greater => OpCode::GtString,
                                BinaryOp::Less => OpCode::LtString,
                                BinaryOp::GreaterEq => OpCode::GteString,
                                BinaryOp::LessEq => OpCode::LteString,
                                _ => unreachable!(),
                            };
                            self.compile_expr(left)?;
                            self.compile_expr(right)?;
                            self.emit(Instruction::simple(string_cmp_op));
                            self.last_expr_schema = None;
                            self.last_expr_type_info = None;
                            self.last_expr_numeric_type = None;
                            return Ok(());
                        }
                    }
                }

                // Stage 4.2: temporal Sub dispatch via CallMethod("sub").
                // When one operand is DateTime or Duration/TimeSpan, emit
                // CallMethod instead of falling through to the strict
                // arithmetic check which would reject non-numeric types.
                if matches!(op, BinaryOp::Sub) {
                    if let (Ok(lt), Ok(rt)) = (self.infer_expr_type(left), self.infer_expr_type(right)) {
                        let lt_name = type_display_name(&lt);
                        let rt_name = type_display_name(&rt);
                        let is_temporal = |n: &str| matches!(n, "DateTime" | "Duration" | "TimeSpan");
                        if is_temporal(&lt_name) || is_temporal(&rt_name) {
                            self.compile_expr(left)?;
                            self.compile_expr(right)?;
                            let method_id = shape_value::MethodId::from_name("sub");
                            let string_id = self.program.add_string("sub".to_string());
                            self.emit(Instruction::new(OpCode::CallMethod, Some(Operand::TypedMethodCall {
                                method_id: method_id.0, arg_count: 1, string_id,
                             receiver_type_tag: 0xFF, })));
                            self.last_expr_schema = None;
                            self.last_expr_type_info = None;
                            self.last_expr_numeric_type = None;
                            return Ok(());
                        }
                    }
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

                // ── Phase 2.5: operator trait dispatch via CallMethod ──
                // If the left operand is a typed object whose schema implements
                // the operator trait (Sub/Mul/Div), emit a method call instead
                // of falling through to a generic arithmetic opcode. The receiver
                // and the right-hand-side operand are already on the stack.
                if let Some(trait_name) = operator_trait_for_op(op) {
                    let dispatches_via_trait = left_schema
                        .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                        .is_some_and(|schema| {
                            self.type_inference
                                .env
                                .type_implements_trait(&schema.name, trait_name)
                        });
                    if dispatches_via_trait {
                        if let Some(method_name) = operator_trait_method_for_op(op) {
                            emit_operator_trait_call(self, method_name);
                            return Ok(());
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
                        // Op has no typed variant for this type combination.
                        if !try_emit_trait_dispatch(self, op, left_schema, left) {
                            if !emit_generic_via_helper(self, op) {
                                self.compile_binary_op(op)?;
                            }
                        }
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
                                if !try_emit_trait_dispatch(self, op, left_schema, left) {
                                    if !emit_generic_via_helper(self, op) {
                                        self.compile_binary_op(op)?;
                                    }
                                }
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

        // Helper: emit abs(a - b) → load a, load b, SubNumber, Dup, push 0, LtNumber, JumpIfFalse(skip), NegNumber, skip:
        // This computes abs(top-of-stack) inline.  All fuzzy comparison operands are f64.
        let emit_abs_diff = |compiler: &mut BytecodeCompiler| {
            compiler.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(temp_a)),
            ));
            compiler.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(temp_b)),
            ));
            compiler.emit(Instruction::simple(OpCode::SubNumber));
            // abs: dup, push 0, LtNumber, JumpIfFalse(skip), NegNumber
            compiler.emit(Instruction::simple(OpCode::Dup));
            let zero_idx = compiler.program.add_constant(Constant::Number(0.0));
            compiler.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(zero_idx)),
            ));
            compiler.emit(Instruction::simple(OpCode::LtNumber));
            let skip = compiler.emit_jump(OpCode::JumpIfFalse, 0);
            compiler.emit(Instruction::simple(OpCode::NegNumber));
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
                self.emit(Instruction::simple(OpCode::LteNumber));
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
                self.emit(Instruction::simple(OpCode::LtNumber));
                let skip_a = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
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
                self.emit(Instruction::simple(OpCode::LtNumber));
                let skip_b = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(skip_b);
                // (abs(a) + abs(b)) / 2
                self.emit(Instruction::simple(OpCode::AddNumber));
                let two_idx = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two_idx)),
                ));
                self.emit(Instruction::simple(OpCode::DivNumber));
                // numerator / denominator <= tol
                self.emit(Instruction::simple(OpCode::DivNumber));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
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
                self.emit(Instruction::simple(OpCode::GtNumber));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                emit_abs_diff(self);
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
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
                self.emit(Instruction::simple(OpCode::GtNumber));
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
                self.emit(Instruction::simple(OpCode::LtNumber));
                let sa = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
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
                self.emit(Instruction::simple(OpCode::LtNumber));
                let sb = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(sb);
                self.emit(Instruction::simple(OpCode::AddNumber));
                let two = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two)),
                ));
                self.emit(Instruction::simple(OpCode::DivNumber));
                self.emit(Instruction::simple(OpCode::DivNumber));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
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
                self.emit(Instruction::simple(OpCode::LtNumber));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                emit_abs_diff(self);
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
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
                self.emit(Instruction::simple(OpCode::LtNumber));
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
                self.emit(Instruction::simple(OpCode::LtNumber));
                let sa = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
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
                self.emit(Instruction::simple(OpCode::LtNumber));
                let sb = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(sb);
                self.emit(Instruction::simple(OpCode::AddNumber));
                let two = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two)),
                ));
                self.emit(Instruction::simple(OpCode::DivNumber));
                self.emit(Instruction::simple(OpCode::DivNumber));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
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
