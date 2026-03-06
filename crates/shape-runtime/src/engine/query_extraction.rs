//! Data query extraction for async prefetching

use shape_ast::ast::{Expr, Item, Program, Statement};
use shape_ast::error::Result;

impl super::ShapeEngine {
    /// Extract data queries from program (Phase 8)
    ///
    /// Legacy load() extraction is removed. This currently relies on context fallback.
    pub(super) fn extract_data_queries(
        &self,
        program: &Program,
    ) -> Result<Vec<crate::data::DataQuery>> {
        let mut queries = Vec::new();

        // Walk AST to keep traversal hooks aligned with program shape.
        for item in &program.items {
            self.extract_from_item(item, &mut queries)?;
        }

        // Fallback: check context for id/timeframe
        if queries.is_empty() {
            if let Some(ctx) = self.runtime.persistent_context() {
                if let (Ok(id), Ok(timeframe)) = (ctx.get_current_id(), ctx.get_current_timeframe())
                {
                    queries.push(crate::data::DataQuery::new(&id, timeframe).limit(1000));
                }
            }
        }

        Ok(queries)
    }

    /// Extract queries from a program item
    fn extract_from_item(
        &self,
        item: &Item,
        queries: &mut Vec<crate::data::DataQuery>,
    ) -> Result<()> {
        match item {
            Item::Expression(expr, _) => self.extract_from_expr(expr, queries)?,
            Item::Statement(stmt, _) => self.extract_from_statement(stmt, queries)?,
            Item::Function(func_def, _) => {
                // Search function bodies
                for stmt in &func_def.body {
                    self.extract_from_statement(stmt, queries)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Extract queries from a statement
    fn extract_from_statement(
        &self,
        stmt: &Statement,
        queries: &mut Vec<crate::data::DataQuery>,
    ) -> Result<()> {
        match stmt {
            Statement::Expression(expr, _) => self.extract_from_expr(expr, queries)?,
            Statement::VariableDecl(decl, _) => {
                if let Some(ref value_expr) = decl.value {
                    self.extract_from_expr(value_expr, queries)?;
                }
            }
            Statement::Assignment(assign, _) => {
                self.extract_from_expr(&assign.value, queries)?;
            }
            Statement::If(if_stmt, _) => {
                self.extract_from_expr(&if_stmt.condition, queries)?;
                for stmt in &if_stmt.then_body {
                    self.extract_from_statement(stmt, queries)?;
                }
                if let Some(ref else_stmts) = if_stmt.else_body {
                    for stmt in else_stmts {
                        self.extract_from_statement(stmt, queries)?;
                    }
                }
            }
            Statement::While(while_loop, _) => {
                self.extract_from_expr(&while_loop.condition, queries)?;
                for stmt in &while_loop.body {
                    self.extract_from_statement(stmt, queries)?;
                }
            }
            Statement::For(for_loop, _) => {
                if let shape_ast::ast::ForInit::ForIn { iter, .. } = &for_loop.init {
                    self.extract_from_expr(iter, queries)?;
                }
                for stmt in &for_loop.body {
                    self.extract_from_statement(stmt, queries)?;
                }
            }
            Statement::Return(Some(expr), _) => self.extract_from_expr(expr, queries)?,
            _ => {}
        }
        Ok(())
    }

    /// Extract queries from an expression
    fn extract_from_expr(
        &self,
        expr: &Expr,
        queries: &mut Vec<crate::data::DataQuery>,
    ) -> Result<()> {
        match expr {
            Expr::FunctionCall { args, .. } => {
                for arg in args {
                    self.extract_from_expr(arg, queries)?;
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.extract_from_expr(receiver, queries)?;
                for arg in args {
                    self.extract_from_expr(arg, queries)?;
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.extract_from_expr(left, queries)?;
                self.extract_from_expr(right, queries)?;
            }
            Expr::UnaryOp { operand, .. } => self.extract_from_expr(operand, queries)?,
            Expr::Array(elements, _) => {
                for element in elements {
                    self.extract_from_expr(element, queries)?;
                }
            }
            Expr::Object(entries, _) => {
                use shape_ast::ast::ObjectEntry;
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } => {
                            self.extract_from_expr(value, queries)?
                        }
                        ObjectEntry::Spread(spread_expr) => {
                            self.extract_from_expr(spread_expr, queries)?
                        }
                    }
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    self.extract_from_statement(stmt, queries)?;
                }
            }
            Expr::SimulationCall { params, .. } => {
                for (_, expr) in params {
                    self.extract_from_expr(expr, queries)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
