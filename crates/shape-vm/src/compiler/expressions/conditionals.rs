//! Conditional expression compilation

use crate::bytecode::OpCode;
use shape_ast::ast::Expr;
use shape_ast::error::Result;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a ternary conditional expression
    pub(super) fn compile_expr_conditional(
        &mut self,
        condition: &Expr,
        then_expr: &Expr,
        else_expr: &Option<Box<Expr>>,
    ) -> Result<()> {
        self.compile_expr(condition)?;

        let else_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

        self.compile_expr(then_expr)?;

        if let Some(else_e) = else_expr {
            let end_jump = self.emit_jump(OpCode::Jump, 0);
            self.patch_jump(else_jump);
            self.compile_expr(else_e)?;
            self.patch_jump(end_jump);
        } else {
            let end_jump = self.emit_jump(OpCode::Jump, 0);
            self.patch_jump(else_jump);
            self.emit_unit();
            self.patch_jump(end_jump);
        }
        Ok(())
    }

    /// Compile an if expression
    pub(super) fn compile_expr_if(&mut self, if_expr: &shape_ast::ast::IfExpr) -> Result<()> {
        self.compile_expr(&if_expr.condition)?;

        let else_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

        self.compile_expr(&if_expr.then_branch)?;

        if let Some(else_branch) = &if_expr.else_branch {
            let end_jump = self.emit_jump(OpCode::Jump, 0);
            self.patch_jump(else_jump);
            self.compile_expr(else_branch)?;
            self.patch_jump(end_jump);
        } else {
            let end_jump = self.emit_jump(OpCode::Jump, 0);
            self.patch_jump(else_jump);
            self.emit_unit();
            self.patch_jump(end_jump);
        }
        Ok(())
    }
}
