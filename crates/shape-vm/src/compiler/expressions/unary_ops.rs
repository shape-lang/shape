//! Unary operation expression compilation

use shape_ast::ast::{Expr, UnaryOp};
use shape_ast::error::Result;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a unary operation expression
    pub(super) fn compile_expr_unary_op(&mut self, op: &UnaryOp, operand: &Expr) -> Result<()> {
        self.compile_expr(operand)?;
        self.compile_unary_op(op)?;
        Ok(())
    }
}
