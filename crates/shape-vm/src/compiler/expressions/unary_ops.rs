//! Unary operation expression compilation

use crate::bytecode::{Instruction, OpCode};
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
                    Some(NumericType::Int) | Some(NumericType::IntWidth(_)) => OpCode::NegInt,
                    Some(NumericType::Number) => OpCode::NegNumber,
                    Some(NumericType::Decimal) => OpCode::NegDecimal,
                    None => OpCode::Neg, // Fallback for unknown types (operator trait dispatch)
                };
                self.emit(Instruction::simple(opcode));
            }
            _ => {
                self.compile_unary_op(op)?;
            }
        }
        Ok(())
    }
}
