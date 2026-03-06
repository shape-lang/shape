//! Control flow statement analysis
//!
//! This module handles analysis of control flow constructs like loops,
//! conditionals, and exception handling.

use shape_ast::ast::{ForInit, ForLoop, IfStatement, Spanned, Statement, WhileLoop};
use shape_ast::error::Result;

use super::types;

/// Implementation of control flow analysis methods for SemanticAnalyzer
impl super::SemanticAnalyzer {
    /// Analyze a statement
    pub(super) fn analyze_statement(&mut self, stmt: &Statement) -> Result<()> {
        match stmt {
            Statement::VariableDecl(decl, _) => {
                self.analyze_variable_decl(decl)?;
            }
            Statement::Assignment(assign, _) => {
                self.analyze_assignment(assign)?;
            }
            Statement::Expression(expr, _) => {
                self.check_expr_type(expr)?;
            }
            Statement::Return(expr_opt, _) => {
                if let Some(expr) = expr_opt {
                    self.check_expr_type(expr)?;
                }
            }
            Statement::For(for_loop, _) => {
                self.analyze_for_loop(for_loop)?;
            }
            Statement::While(while_loop, _) => {
                self.analyze_while_loop(while_loop)?;
            }
            Statement::If(if_stmt, _) => {
                self.analyze_if_statement(if_stmt)?;
            }
            Statement::Break(_) | Statement::Continue(_) => {
                // Break and continue are valid control flow, no analysis needed
            }
            Statement::Extend(ext, _) => {
                for method in &ext.methods {
                    for stmt in &method.body {
                        self.analyze_statement(stmt)?;
                    }
                }
            }
            Statement::RemoveTarget(_) => {}
            Statement::SetParamType { .. }
            | Statement::SetReturnType { .. }
            | Statement::SetReturnExpr { .. } => {}
            Statement::ReplaceModuleExpr { expression, .. } => {
                self.check_expr_type(expression)?;
            }
            Statement::ReplaceBodyExpr { expression, .. } => {
                self.check_expr_type(expression)?;
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    self.analyze_statement(stmt)?;
                }
            }
        }
        Ok(())
    }

    /// Analyze a for loop
    pub(super) fn analyze_for_loop(&mut self, for_loop: &ForLoop) -> Result<()> {
        match &for_loop.init {
            ForInit::ForIn { pattern, iter } => {
                // Check the iterator expression
                self.check_expr_type(iter)?;

                // Create a scope for the loop variable(s)
                self.symbol_table.push_scope();
                // Define all variables from the pattern
                for name in pattern.get_identifiers() {
                    self.symbol_table.define_variable(
                        &name,
                        types::Type::Unknown, // Element type from iterator
                        shape_ast::ast::VarKind::Const,
                        true,
                    )?;
                }

                // Analyze the loop body
                for stmt in &for_loop.body {
                    self.analyze_statement(stmt)?;
                }

                self.symbol_table.pop_scope();
            }
            ForInit::ForC {
                init,
                condition,
                update,
            } => {
                // Create a scope for the loop
                self.symbol_table.push_scope();

                // Analyze the initialization
                self.analyze_statement(init)?;

                // Check the condition is boolean
                let cond_type = self.check_expr_type(condition)?;
                if cond_type != types::Type::Bool && cond_type != types::Type::Unknown {
                    return Err(self.error_at(
                        condition.span(),
                        format!("For loop condition must be boolean, got {}", cond_type),
                    ));
                }

                // Check the update expression
                self.check_expr_type(update)?;

                // Analyze the loop body
                for stmt in &for_loop.body {
                    self.analyze_statement(stmt)?;
                }

                self.symbol_table.pop_scope();
            }
        }

        Ok(())
    }

    /// Analyze a while loop
    pub(super) fn analyze_while_loop(&mut self, while_loop: &WhileLoop) -> Result<()> {
        // Check the condition is boolean
        let cond_type = self.check_expr_type(&while_loop.condition)?;
        if cond_type != types::Type::Bool && cond_type != types::Type::Unknown {
            return Err(self.error_at(
                while_loop.condition.span(),
                format!("While loop condition must be boolean, got {}", cond_type),
            ));
        }

        // Create a scope for the loop body
        self.symbol_table.push_scope();
        for stmt in &while_loop.body {
            self.analyze_statement(stmt)?;
        }
        self.symbol_table.pop_scope();

        Ok(())
    }

    /// Analyze an if statement
    pub(super) fn analyze_if_statement(&mut self, if_stmt: &IfStatement) -> Result<()> {
        // Check the condition is boolean
        let cond_type = self.check_expr_type(&if_stmt.condition)?;
        if cond_type != types::Type::Bool && cond_type != types::Type::Unknown {
            return Err(self.error_at(
                if_stmt.condition.span(),
                format!("If condition must be boolean, got {}", cond_type),
            ));
        }

        // Analyze the then body
        self.symbol_table.push_scope();
        for stmt in &if_stmt.then_body {
            self.analyze_statement(stmt)?;
        }
        self.symbol_table.pop_scope();

        // Analyze the else body if present
        if let Some(else_body) = &if_stmt.else_body {
            self.symbol_table.push_scope();
            for stmt in else_body {
                self.analyze_statement(stmt)?;
            }
            self.symbol_table.pop_scope();
        }

        Ok(())
    }
}
