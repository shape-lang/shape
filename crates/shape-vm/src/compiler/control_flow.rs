//! Control flow compilation (if statements, try-catch)

use crate::bytecode::OpCode;
use shape_ast::error::Result;

use super::BytecodeCompiler;

impl BytecodeCompiler {
    pub(super) fn compile_if_statement(
        &mut self,
        if_stmt: &shape_ast::ast::IfStatement,
    ) -> Result<()> {
        // Compile condition
        self.compile_expr(&if_stmt.condition)?;

        // Jump to else if false
        let else_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

        // Compile then body
        for stmt in &if_stmt.then_body {
            self.compile_statement(stmt)?;
        }

        if let Some(else_body) = &if_stmt.else_body {
            // Jump over else
            let end_jump = self.emit_jump(OpCode::Jump, 0);

            // Patch else jump
            self.patch_jump(else_jump);

            // Compile else body
            for stmt in else_body {
                self.compile_statement(stmt)?;
            }

            // Patch end jump
            self.patch_jump(end_jump);
        } else {
            // Patch else jump
            self.patch_jump(else_jump);
        }

        Ok(())
    }

    // ===== Helper Methods =====
}
