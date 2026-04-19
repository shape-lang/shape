//! Unary operation expression compilation

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::type_tracking::NumericType;
use shape_ast::ast::{Expr, UnaryOp};
use shape_ast::error::Result;

use super::super::BytecodeCompiler;
use super::numeric_ops::inferred_type_to_numeric;

impl BytecodeCompiler {
    /// Compile a unary operation expression
    pub(super) fn compile_expr_unary_op(&mut self, op: &UnaryOp, operand: &Expr) -> Result<()> {
        self.compile_expr(operand)?;
        match op {
            UnaryOp::BitNot => {
                // Phase R5.1C: emit `BitNotInt` when the operand type is
                // provably `int` at compile time. Otherwise fall through
                // to the Dynamic `BitNot` opcode via `compile_unary_op`.
                //
                // Semantics match the Dynamic variant exactly — i48
                // payload truncation applies. Gate:
                // `SHAPE_V2_TYPED_BITWISE` (default ON via
                // `typed_bitwise_enabled()`, shared with the binary
                // bitwise ops).
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
                let emit_typed =
                    is_int && crate::compiler::helpers::typed_bitwise_enabled();
                if emit_typed {
                    self.emit(Instruction::simple(OpCode::BitNotInt));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.last_expr_numeric_type = Some(NumericType::Int);
                    return Ok(());
                }
                // Fall through to Dynamic `BitNot` — preserves pre-R5.1C
                // emission byte-identically.
                self.compile_unary_op(op)?;
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
                         receiver_type_tag: 0xFF, }),
                    ));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.last_expr_numeric_type = None;
                    return Ok(());
                }

                return Err(shape_ast::error::ShapeError::SemanticError {
                    message: "Cannot infer operand type for unary `-` — add type annotations".to_string(),
                    location: None,
                });
            }
            _ => {
                self.compile_unary_op(op)?;
            }
        }
        Ok(())
    }
}
