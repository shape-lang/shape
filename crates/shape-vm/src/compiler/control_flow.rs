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
        for (idx, stmt) in if_stmt.then_body.iter().enumerate() {
            let future_names = self
                .future_reference_use_names_for_remaining_statements(&if_stmt.then_body[idx + 1..]);
            self.push_future_reference_use_names(future_names);
            let compile_result = self.compile_statement(stmt);
            self.pop_future_reference_use_names();
            compile_result?;
            self.release_unused_local_reference_borrows_for_remaining_statements(
                &if_stmt.then_body[idx + 1..],
            );
            self.release_unused_module_reference_borrows_for_remaining_statements(
                &if_stmt.then_body[idx + 1..],
            );
        }

        if let Some(else_body) = &if_stmt.else_body {
            // Jump over else
            let end_jump = self.emit_jump(OpCode::Jump, 0);

            // Patch else jump
            self.patch_jump(else_jump);

            // Compile else body
            for (idx, stmt) in else_body.iter().enumerate() {
                let future_names =
                    self.future_reference_use_names_for_remaining_statements(&else_body[idx + 1..]);
                self.push_future_reference_use_names(future_names);
                let compile_result = self.compile_statement(stmt);
                self.pop_future_reference_use_names();
                compile_result?;
                self.release_unused_local_reference_borrows_for_remaining_statements(
                    &else_body[idx + 1..],
                );
                self.release_unused_module_reference_borrows_for_remaining_statements(
                    &else_body[idx + 1..],
                );
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
