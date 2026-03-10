//! Control flow expression compilation (break, continue, return)

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_ast::ast::Expr;
use shape_ast::error::{Result, ShapeError};

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a break expression
    pub(super) fn compile_expr_break(&mut self, value_expr: &Option<Box<Expr>>) -> Result<()> {
        if self.loop_stack.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: "break expression outside of loop".to_string(),
                location: None,
            });
        }

        let break_value_local = self.loop_stack.last().and_then(|ctx| ctx.break_value_local);

        if let Some(value) = value_expr {
            self.compile_expr(value)?;
        } else if break_value_local.is_some() {
            // Only push Null when there is a break-value local to store it in.
            // For statement-level loops (no break_value_local), pushing Null
            // would leave a stale value on the stack that corrupts the
            // iterator pop below.
            self.emit(Instruction::simple(OpCode::PushNull));
        }

        if let Some(local_idx) = break_value_local {
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(local_idx)),
            ));
        } else if value_expr.is_some() {
            self.emit(Instruction::simple(OpCode::Pop));
        }

        // Emit drops for drop scopes inside the loop before breaking
        let scopes_to_exit = self
            .loop_stack
            .last()
            .map(|ctx| self.drop_locals.len().saturating_sub(ctx.drop_scope_depth))
            .unwrap_or(0);
        if scopes_to_exit > 0 {
            self.emit_drops_for_early_exit(scopes_to_exit)?;
        }

        // If the current loop has an iterator on the stack (for-in loops),
        // pop it before jumping out so the stack stays balanced.
        if self
            .loop_stack
            .last()
            .map_or(false, |ctx| ctx.iterator_on_stack)
        {
            self.emit(Instruction::simple(OpCode::Pop));
        }

        let jump_idx = self.emit_jump(OpCode::Jump, 0);
        if let Some(loop_ctx) = self.loop_stack.last_mut() {
            loop_ctx.break_jumps.push(jump_idx);
        }
        Ok(())
    }

    /// Compile a continue expression
    pub(super) fn compile_expr_continue(&mut self) -> Result<()> {
        if let Some(loop_ctx) = self.loop_stack.last() {
            // Copy values we need before mutable borrow
            let scopes_to_exit = self
                .drop_locals
                .len()
                .saturating_sub(loop_ctx.drop_scope_depth);
            let continue_target = loop_ctx.continue_target;
            // Emit drops for drop scopes inside the loop before continuing
            if scopes_to_exit > 0 {
                self.emit_drops_for_early_exit(scopes_to_exit)?;
            }
            let offset = continue_target as i32 - self.program.current_offset() as i32 - 1;
            self.emit(Instruction::new(
                OpCode::Jump,
                Some(Operand::Offset(offset)),
            ));
        } else {
            return Err(ShapeError::RuntimeError {
                message: "continue expression outside of loop".to_string(),
                location: None,
            });
        }
        Ok(())
    }

    /// Compile a return expression
    pub(super) fn compile_expr_return(&mut self, value_expr: &Option<Box<Expr>>) -> Result<()> {
        if let Some(expr) = value_expr {
            self.compile_expr(expr)?;
        } else {
            self.emit_unit();
        }
        // Emit drops for all active drop scopes before returning
        let total_scopes = self.drop_locals.len();
        if total_scopes > 0 {
            self.emit_drops_for_early_exit(total_scopes)?;
        }
        self.emit(Instruction::simple(OpCode::ReturnValue));
        Ok(())
    }
}
