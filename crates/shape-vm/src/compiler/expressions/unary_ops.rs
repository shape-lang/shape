//! Unary operation expression compilation

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::type_tracking::NumericType;
use shape_ast::ast::{Expr, UnaryOp};
use shape_ast::error::Result;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a unary operation expression
    pub(super) fn compile_expr_unary_op(&mut self, op: &UnaryOp, operand: &Expr) -> Result<()> {
        self.compile_expr(operand)?;
        match op {
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
                        }),
                    ));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.last_expr_numeric_type = None;
                    return Ok(());
                }

                // Fallback for unknown types (BigInt, Decimal-as-heap, etc.):
                // emit the generic Neg opcode and let `exec_arithmetic` dispatch
                // by tag at runtime. Phase 2.6 / later phases narrow this further.
                self.emit(Instruction::simple(OpCode::Neg));
            }
            _ => {
                self.compile_unary_op(op)?;
            }
        }
        Ok(())
    }
}
