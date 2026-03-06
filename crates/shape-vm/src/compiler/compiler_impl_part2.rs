use super::*;

impl BytecodeCompiler {
    pub(super) fn infer_reference_params_from_types(
        program: &Program,
        inferred_types: &HashMap<String, Type>,
    ) -> HashMap<String, Vec<bool>> {
        let funcs = Self::collect_program_functions(program);
        let mut inferred = HashMap::new();

        for (name, func) in funcs {
            let mut inferred_flags = vec![false; func.params.len()];
            let Some(Type::Function { params, .. }) = inferred_types.get(&name) else {
                inferred.insert(name, inferred_flags);
                continue;
            };

            for (idx, param) in func.params.iter().enumerate() {
                if param.type_annotation.is_some()
                    || param.is_reference
                    || param.simple_name().is_none()
                {
                    continue;
                }
                if let Some(inferred_param_ty) = params.get(idx)
                    && Self::type_is_heap_like(inferred_param_ty)
                {
                    inferred_flags[idx] = true;
                }
            }
            inferred.insert(name, inferred_flags);
        }

        inferred
    }

    pub(super) fn analyze_statement_for_ref_mutation(
        stmt: &shape_ast::ast::Statement,
        caller_name: &str,
        param_index_by_name: &HashMap<String, usize>,
        caller_ref_params: &[bool],
        callee_ref_params: &HashMap<String, Vec<bool>>,
        direct_mutates: &mut [bool],
        edges: &mut Vec<(String, usize, String, usize)>,
    ) {
        use shape_ast::ast::{ForInit, Statement};

        match stmt {
            Statement::Return(Some(expr), _) | Statement::Expression(expr, _) => {
                Self::analyze_expr_for_ref_mutation(
                    expr,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    Self::analyze_expr_for_ref_mutation(
                        value,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
            }
            Statement::Assignment(assign, _) => {
                if let Some(name) = assign.pattern.as_identifier()
                    && let Some(&idx) = param_index_by_name.get(name)
                    && caller_ref_params.get(idx).copied().unwrap_or(false)
                {
                    direct_mutates[idx] = true;
                }
                Self::analyze_expr_for_ref_mutation(
                    &assign.value,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::If(if_stmt, _) => {
                Self::analyze_expr_for_ref_mutation(
                    &if_stmt.condition,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
                for stmt in &if_stmt.then_body {
                    Self::analyze_statement_for_ref_mutation(
                        stmt,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for stmt in else_body {
                        Self::analyze_statement_for_ref_mutation(
                            stmt,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                    }
                }
            }
            Statement::While(while_loop, _) => {
                Self::analyze_expr_for_ref_mutation(
                    &while_loop.condition,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
                for stmt in &while_loop.body {
                    Self::analyze_statement_for_ref_mutation(
                        stmt,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
            }
            Statement::For(for_loop, _) => {
                match &for_loop.init {
                    ForInit::ForIn { iter, .. } => {
                        Self::analyze_expr_for_ref_mutation(
                            iter,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                    }
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        Self::analyze_statement_for_ref_mutation(
                            init,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                        Self::analyze_expr_for_ref_mutation(
                            condition,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                        Self::analyze_expr_for_ref_mutation(
                            update,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                    }
                }
                for stmt in &for_loop.body {
                    Self::analyze_statement_for_ref_mutation(
                        stmt,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
            }
            Statement::Extend(ext, _) => {
                for method in &ext.methods {
                    for stmt in &method.body {
                        Self::analyze_statement_for_ref_mutation(
                            stmt,
                            caller_name,
                            param_index_by_name,
                            caller_ref_params,
                            callee_ref_params,
                            direct_mutates,
                            edges,
                        );
                    }
                }
            }
            Statement::SetReturnExpr { expression, .. } => {
                Self::analyze_expr_for_ref_mutation(
                    expression,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::ReplaceBodyExpr { expression, .. } => {
                Self::analyze_expr_for_ref_mutation(
                    expression,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::ReplaceModuleExpr { expression, .. } => {
                Self::analyze_expr_for_ref_mutation(
                    expression,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                );
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    Self::analyze_statement_for_ref_mutation(
                        stmt,
                        caller_name,
                        param_index_by_name,
                        caller_ref_params,
                        callee_ref_params,
                        direct_mutates,
                        edges,
                    );
                }
            }
            Statement::Break(_)
            | Statement::Continue(_)
            | Statement::Return(None, _)
            | Statement::RemoveTarget(_)
            | Statement::SetParamType { .. }
            | Statement::SetReturnType { .. } => {}
        }
    }

    pub(super) fn ref_param_index_from_arg(
        arg: &shape_ast::ast::Expr,
        param_index_by_name: &HashMap<String, usize>,
        caller_ref_params: &[bool],
    ) -> Option<usize> {
        match arg {
            shape_ast::ast::Expr::Reference { expr: inner, .. } => match inner.as_ref() {
                shape_ast::ast::Expr::Identifier(name, _) => param_index_by_name
                    .get(name)
                    .copied()
                    .filter(|idx| caller_ref_params.get(*idx).copied().unwrap_or(false)),
                _ => None,
            },
            shape_ast::ast::Expr::Identifier(name, _) => param_index_by_name
                .get(name)
                .copied()
                .filter(|idx| caller_ref_params.get(*idx).copied().unwrap_or(false)),
            _ => None,
        }
    }
}
