//! Statement-level type inference
//!
//! Handles type inference for statements: if, while, for, return, etc.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{BinaryOp, Expr, Literal, Statement, TypeAnnotation};

impl TypeInferenceEngine {
    /// Infer type of statements
    pub(crate) fn infer_statements(&mut self, stmts: &[Statement]) -> TypeResult<Type> {
        let mut last_type = BuiltinTypes::void();

        for stmt in stmts {
            last_type = self.infer_statement(stmt)?;
        }

        Ok(last_type)
    }

    /// Infer return type for a callable body.
    ///
    /// - If the body contains explicit `return` statements, aggregate all
    ///   returned types (including from nested control-flow) into a single type.
    /// - If no explicit `return` exists, use the final statement type to support
    ///   expression-style bodies.
    pub(crate) fn infer_callable_return_type(
        &mut self,
        stmts: &[Statement],
        allow_unresolved_generic_args: bool,
    ) -> TypeResult<Type> {
        self.push_return_scope();
        self.push_implicit_return_scope();
        let body_result = self.infer_statements(stmts);
        let explicit_returns = self.pop_return_scope();
        let implicit_returns = self.pop_implicit_return_scope();
        let body_type = body_result?;

        if explicit_returns.is_empty() {
            let implicit_candidates: Vec<Type> = implicit_returns
                .into_iter()
                .filter(|ty| !self.is_void_type(ty))
                .collect();

            if implicit_candidates.is_empty() {
                Ok(body_type)
            } else {
                if allow_unresolved_generic_args {
                    self.combine_return_types_allow_unresolved(&implicit_candidates)
                } else {
                    self.combine_return_types(&implicit_candidates)
                }
            }
        } else if self.all_types_equal(&explicit_returns) {
            Ok(explicit_returns[0].clone())
        } else if let Some(base_var) = explicit_returns.iter().find_map(|ty| match ty {
            Type::Variable(var) => Some(var.clone()),
            _ => None,
        }) {
            // Preserve precision for mixed returns like:
            //   return c        // c: type variable resolved from call-sites
            //   return "hi"     // concrete
            // by materializing the union after call-site widening.
            let additional_members = explicit_returns
                .iter()
                .filter(|ty| !matches!(ty, Type::Variable(var) if *var == base_var))
                .cloned()
                .collect::<Vec<_>>();
            if additional_members.is_empty() {
                Ok(Type::Variable(base_var))
            } else {
                self.record_pending_return_union(base_var.clone(), additional_members);
                Ok(Type::Variable(base_var))
            }
        } else {
            if allow_unresolved_generic_args {
                self.combine_return_types_allow_unresolved(&explicit_returns)
            } else {
                self.combine_return_types(&explicit_returns)
            }
        }
    }

    /// Infer type of a single statement
    pub(crate) fn infer_statement(&mut self, stmt: &Statement) -> TypeResult<Type> {
        match stmt {
            Statement::Return(expr_opt, _) => {
                let return_type = if let Some(expr) = expr_opt {
                    self.infer_expr(expr)?
                } else {
                    BuiltinTypes::void()
                };
                self.record_return_type(return_type.clone());
                Ok(return_type)
            }
            Statement::VariableDecl(decl, _) => {
                self.infer_variable_decl(decl)?;
                Ok(BuiltinTypes::void())
            }
            Statement::Assignment(assign, span) => {
                let value_type = self.infer_expr(&assign.value)?;
                if let Some(name) = assign.pattern.as_identifier() {
                    let target_type = self
                        .env
                        .lookup(name)
                        .map(|scheme| scheme.instantiate())
                        .ok_or_else(|| {
                            self.register_undefined_variable_origin(name, *span);
                            TypeError::UndefinedVariable(name.to_string())
                        })?;
                    self.constraints.push((target_type, value_type));
                } else {
                    // Destructuring assignment: conservatively constrain each bound name
                    // to the assigned value until full pattern assignment inference lands.
                    for name in assign.pattern.get_identifiers() {
                        let target_type = self
                            .env
                            .lookup(&name)
                            .map(|scheme| scheme.instantiate())
                            .ok_or_else(|| {
                                self.register_undefined_variable_origin(&name, *span);
                                TypeError::UndefinedVariable(name.clone())
                            })?;
                        self.constraints.push((target_type, value_type.clone()));
                    }
                }
                Ok(BuiltinTypes::void())
            }
            Statement::Expression(expr, _) => {
                let expr_type = self.infer_expr(expr)?;
                self.record_implicit_return_type(expr_type.clone());
                Ok(expr_type)
            }
            Statement::If(if_stmt, _) => {
                self.infer_expr(&if_stmt.condition)?;

                // Extract flow-sensitive narrowing info from the condition
                let narrowings = self.extract_narrowings(&if_stmt.condition);

                // Enter conditional context for field evolution tracking
                self.env.enter_conditional();
                self.env.push_scope();
                // Push narrowed types for then-branch (e.g. x != null → x: T)
                for (var_name, narrowed_type) in &narrowings {
                    self.env.define(var_name, TypeScheme::mono(narrowed_type.clone()));
                }
                let then_type = self.infer_statements(&if_stmt.then_body)?;
                self.env.pop_scope();
                self.env.exit_conditional();

                if let Some(else_body) = &if_stmt.else_body {
                    // Compute inverse narrowings for else-branch
                    let inverse_narrowings =
                        self.extract_inverse_narrowings(&if_stmt.condition);
                    self.env.enter_conditional();
                    self.env.push_scope();
                    for (var_name, narrowed_type) in &inverse_narrowings {
                        self.env
                            .define(var_name, TypeScheme::mono(narrowed_type.clone()));
                    }
                    let else_type = self.infer_statements(else_body)?;
                    self.env.pop_scope();
                    self.env.exit_conditional();
                    // Both branches should have compatible types
                    self.constraints.push((then_type.clone(), else_type));
                }

                Ok(then_type)
            }
            Statement::For(for_loop, _) => {
                self.env.push_scope();

                // Handle different for loop types
                match &for_loop.init {
                    shape_ast::ast::ForInit::ForIn { pattern, iter } => {
                        let iter_type = self.infer_expr(iter)?;

                        // Infer element type from iterator
                        let element_type = self.infer_iterator_element_type(&iter_type)?;
                        // Define all variables from the pattern
                        for name in pattern.get_identifiers() {
                            self.env
                                .define(&name, TypeScheme::mono(element_type.clone()));
                        }
                    }
                    shape_ast::ast::ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.infer_statement(init)?;
                        let cond_type = self.infer_expr(condition)?;
                        self.constraints.push((cond_type, BuiltinTypes::boolean()));
                        self.infer_expr(update)?;
                    }
                }

                // Enter loop context for field evolution tracking
                self.env.enter_loop();
                self.infer_statements(&for_loop.body)?;
                self.env.exit_loop();
                self.env.pop_scope();

                Ok(BuiltinTypes::void())
            }
            Statement::While(while_loop, _) => {
                self.infer_expr(&while_loop.condition)?;
                // Enter loop context for field evolution tracking
                self.env.enter_loop();
                self.infer_statements(&while_loop.body)?;
                self.env.exit_loop();
                Ok(BuiltinTypes::void())
            }
            _ => Ok(BuiltinTypes::void()),
        }
    }

    /// Extract narrowing info from a condition expression.
    /// For `x != null`, returns `[(x, T)]` where the original type of x is `T?`.
    fn extract_narrowings(&self, condition: &Expr) -> Vec<(String, Type)> {
        match condition {
            // x != null  or  x != undefined  →  narrow x from T? to T
            Expr::BinaryOp {
                left,
                op: BinaryOp::NotEqual,
                right,
                ..
            } => {
                if Self::is_null_literal(right) {
                    self.try_null_narrowing(left)
                } else if Self::is_null_literal(left) {
                    self.try_null_narrowing(right)
                } else {
                    vec![]
                }
            }
            // x == null  →  no narrowing in then-branch (narrowing in else-branch)
            _ => vec![],
        }
    }

    /// Extract inverse narrowings for else-branch.
    /// For `x == null`, returns `[(x, T)]` (else means x is not null).
    /// For `x != null`, no narrowing in else-branch.
    fn extract_inverse_narrowings(&self, condition: &Expr) -> Vec<(String, Type)> {
        match condition {
            // x == null  →  in the else-branch, x is not null → narrow T? to T
            Expr::BinaryOp {
                left,
                op: BinaryOp::Equal,
                right,
                ..
            } => {
                if Self::is_null_literal(right) {
                    self.try_null_narrowing(left)
                } else if Self::is_null_literal(left) {
                    self.try_null_narrowing(right)
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    /// Check if an expression is a null/none literal.
    fn is_null_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Literal(Literal::None, _) => true,
            Expr::Identifier(name, _) => {
                name == "null" || name == "undefined" || name == "none"
            }
            _ => false,
        }
    }

    /// Try to narrow a variable from T? to T.
    /// Returns narrowing if the expression is a variable with an Optional type.
    fn try_null_narrowing(&self, expr: &Expr) -> Vec<(String, Type)> {
        if let Expr::Identifier(name, _) = expr {
            if let Some(scheme) = self.env.lookup(name) {
                let ty = scheme.instantiate();
                if let Some(inner) = Self::unwrap_optional_type(&ty) {
                    return vec![(name.clone(), inner)];
                }
            }
        }
        vec![]
    }

    /// Unwrap T? / Option<T> to T.
    fn unwrap_optional_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Option" && args.len() == 1 =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            Type::Generic { base, args } => {
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    if name == "Option" && args.len() == 1 {
                        return Some(args[0].clone());
                    }
                }
                None
            }
            _ => None,
        }
    }
}
