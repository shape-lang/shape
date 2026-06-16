//! Unary operation expression compilation

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::type_tracking::NumericType;
use shape_ast::ast::{Expr, Span, UnaryOp};
use shape_ast::error::Result;

use super::super::BytecodeCompiler;
use super::numeric_ops::{inferred_type_to_numeric, is_strict_unary_bitwise};

impl BytecodeCompiler {
    /// Compile a unary operation expression.
    ///
    /// `op_span` is the source span of the parent `Expr::UnaryOp` node
    /// (W10 jit-call-method-user-trait-fix, 2026-05-17). Recorded in
    /// `BytecodeProgram.operator_trait_dispatch_sites` at the Neg/Not
    /// trait-dispatch branches so the JIT MIR consumer can re-emit the
    /// dispatch at the matching `Rvalue::UnaryOp` site (keyed by the
    /// same span the MIR lowering stamps via `expr.span()`).
    pub(super) fn compile_expr_unary_op(
        &mut self,
        op: &UnaryOp,
        operand: &Expr,
        op_span: Span,
    ) -> Result<()> {
        self.compile_expr(operand)?;
        match op {
            UnaryOp::BitNot => {
                // c5 Phase B (v0.3.3, 2026-05-28) — bitwise-strict-typing
                // gate (unary). When the operand isn't provably `int`,
                // refuse at compile time. Pre-fix, the fall-through
                // emitted a `BitNot` dynamic opcode whose executor
                // (`exec_dyn_bit_unary`) discarded the operand kind and
                // reinterpreted the slot bits as i64 — `~1.5` silently
                // returned `-4609434218613702657` (`!f64::to_bits(1.5)`).
                // See audit doc 05 §3 + audit doc 05a §c5 anchor sites.
                //
                // Producer-side polarity: refuse at the compiler rather
                // than preserve kinds at the consumer. Per CLAUDE.md
                // §Type-System-Rules: "NO runtime coercion".
                debug_assert!(
                    is_strict_unary_bitwise(op),
                    "unary `~` arm must classify as is_strict_unary_bitwise (gate site)"
                );
                let mut numeric = self.last_expr_numeric_type;
                if let Expr::Identifier(name, _) = operand {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.param_locals.contains(&local_idx) {
                            numeric = None;
                        }
                    }
                }
                if numeric.is_none() {
                    numeric = self
                        .infer_expr_type(operand)
                        .ok()
                        .and_then(|t| inferred_type_to_numeric(&t));
                }
                let is_int = matches!(numeric, Some(NumericType::Int));
                if !is_int {
                    let operand_desc = self
                        .infer_expr_type(operand)
                        .ok()
                        .map(|t| super::numeric_ops::type_display_name(&t))
                        .unwrap_or_else(|| "unknown".to_string());
                    return Err(shape_ast::error::ShapeError::SemanticError {
                        message: format!(
                            "Cannot apply '~' to {}. Bitwise NOT requires the operand to be \
                             `int` at compile time. Use an explicit cast (e.g. `~(x as int)`) \
                             when intentional.",
                            operand_desc,
                        ),
                        location: None,
                    });
                }
                self.emit(Instruction::simple(OpCode::BitNotInt));
                self.last_expr_schema = None;
                self.last_expr_type_info = None;
                self.last_expr_numeric_type = Some(NumericType::Int);
                return Ok(());
            }
            UnaryOp::Neg => {
                // Emit typed negation when the operand type is known
                let opcode = match self.last_expr_numeric_type {
                    Some(NumericType::Int) | Some(NumericType::IntWidth(_)) => Some(OpCode::NegInt),
                    Some(NumericType::Number) => Some(OpCode::NegNumber),
                    Some(NumericType::Decimal) => Some(OpCode::NegDecimal),
                    None => None,
                };
                if let Some(opcode) = opcode {
                    self.emit(Instruction::simple(opcode));
                    return Ok(());
                }

                // Phase 2.5: operator trait dispatch via CallMethod for `-x`
                // when `x` is a typed object that implements `Neg`. The operand
                // (receiver) is already on the stack from compile_expr above.
                let dispatches_via_neg_trait = self
                    .last_expr_schema
                    .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                    .is_some_and(|schema| {
                        self.type_inference
                            .env
                            .type_implements_trait(&schema.name, "Neg")
                    });
                if dispatches_via_neg_trait {
                    let method_id = shape_value::MethodId::from_name("neg");
                    let string_id = self.program.add_string("neg".to_string());
                    self.emit(Instruction::new(
                        OpCode::CallMethod,
                        Some(Operand::TypedMethodCall {
                            method_id: method_id.0,
                            arg_count: 0,
                            string_id,
                            receiver_type_tag: 0xFF,
                        }),
                    ));
                    // ADR-006 §2.7.5 W10 conduit: persist the bytecode-time
                    // unary-trait-dispatch decision so the JIT MIR consumer
                    // can lift `Rvalue::UnaryOp(Neg, _)` at the same source
                    // span to a method-call equivalent. arg_count = 0 for
                    // unary ops (only the receiver, no explicit args).
                    self.program
                        .operator_trait_dispatch_sites
                        .insert(op_span, ("neg".to_string(), 0));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.last_expr_numeric_type = None;
                    return Ok(());
                }

                // Second-chance: ask the type inferencer. If it resolves to
                // a concrete numeric type, emit the appropriate typed
                // opcode. If it resolves to a concrete non-numeric, non-Neg
                // type, error out — the caller genuinely wrote something
                // nonsensical like `-"foo"`.
                //
                // C.STDLIB-B: when the operand type cannot be proven at
                // compile time (unresolved numeric TypeVar, closure
                // parameter whose type the outer inferencer can't resolve,
                // or any other "not yet known" case), default to `number`.
                // `infer_unary_op` on the inference side already pushes a
                // constraint `operand == number` for untyped numeric
                // operands, so `number` is the type-system-consistent
                // default. This is a principled compile-time default, NOT
                // runtime coercion: the choice is made during bytecode
                // emission, before the program runs. The executor's
                // `NegNumber` handler coerces an `int` operand through
                // `number_operand` without silent precision loss for
                // i48 values.
                use shape_runtime::type_system::Type;
                match self.infer_expr_type(operand) {
                    Ok(inferred) => {
                        if let Some(nt) = inferred_type_to_numeric(&inferred) {
                            let opcode = match nt {
                                NumericType::Int | NumericType::IntWidth(_) => OpCode::NegInt,
                                NumericType::Number => OpCode::NegNumber,
                                NumericType::Decimal => OpCode::NegDecimal,
                            };
                            self.emit(Instruction::simple(opcode));
                            self.last_expr_numeric_type = Some(nt);
                            return Ok(());
                        }
                        // Unresolved TypeVar / Constrained / Function (not
                        // a concrete type) — default to `number`.
                        if matches!(
                            inferred,
                            Type::Variable(_) | Type::Constrained { .. } | Type::Function { .. }
                        ) {
                            self.emit(Instruction::simple(OpCode::NegNumber));
                            self.last_expr_numeric_type = Some(NumericType::Number);
                            return Ok(());
                        }
                        // Concrete non-numeric type with no Neg impl: fall
                        // through to error.
                    }
                    Err(_) => {
                        // Inferencer couldn't resolve the operand type
                        // (e.g. a closure parameter when the outer
                        // inference scope doesn't cover the closure body).
                        // Default to `number` — the only principled
                        // numeric choice for unary `-`.
                        self.emit(Instruction::simple(OpCode::NegNumber));
                        self.last_expr_numeric_type = Some(NumericType::Number);
                        return Ok(());
                    }
                }

                return Err(shape_ast::error::ShapeError::SemanticError {
                    message: "Cannot infer operand type for unary `-` — add type annotations"
                        .to_string(),
                    location: None,
                });
            }
            UnaryOp::Not => {
                // W1.6: operator trait dispatch via CallMethod for `!x`
                // when `x` is a typed object that implements `Not`. The
                // operand (receiver) is already on the stack from
                // compile_expr above. Sibling of the Neg dispatch above —
                // both unary traits route through a single-arg CallMethod.
                //
                // The built-in `OpCode::Not` handles the bool case (the
                // strict-typing compiler proves `bool` ahead of this path
                // for the typed bool form). User-type dispatch is the
                // exception that mirrors W1.5 Neg.
                let dispatches_via_not_trait = self
                    .last_expr_schema
                    .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                    .is_some_and(|schema| {
                        self.type_inference
                            .env
                            .type_implements_trait(&schema.name, "Not")
                    });
                if dispatches_via_not_trait {
                    let method_id = shape_value::MethodId::from_name("not");
                    let string_id = self.program.add_string("not".to_string());
                    self.emit(Instruction::new(
                        OpCode::CallMethod,
                        Some(Operand::TypedMethodCall {
                            method_id: method_id.0,
                            arg_count: 0,
                            string_id,
                            receiver_type_tag: 0xFF,
                        }),
                    ));
                    // ADR-006 §2.7.5 W10 conduit: persist the bytecode-time
                    // unary-trait-dispatch decision (Not sibling of Neg above).
                    self.program
                        .operator_trait_dispatch_sites
                        .insert(op_span, ("not".to_string(), 0));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.last_expr_numeric_type = None;
                    return Ok(());
                }

                // Fall through to the built-in `OpCode::Not` (boolean).
                self.compile_unary_op(op)?;
            }
        }
        Ok(())
    }
}
