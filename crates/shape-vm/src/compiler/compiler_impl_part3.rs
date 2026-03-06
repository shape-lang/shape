use super::*;

impl BytecodeCompiler {
    pub(super) fn analyze_expr_for_ref_mutation(
        expr: &shape_ast::ast::Expr,
        caller_name: &str,
        param_index_by_name: &HashMap<String, usize>,
        caller_ref_params: &[bool],
        callee_ref_params: &HashMap<String, Vec<bool>>,
        direct_mutates: &mut [bool],
        edges: &mut Vec<(String, usize, String, usize)>,
    ) {
        use shape_ast::ast::Expr;
        macro_rules! visit_expr {
            ($e:expr) => {
                Self::analyze_expr_for_ref_mutation(
                    $e,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                )
            };
        }
        macro_rules! visit_stmt {
            ($s:expr) => {
                Self::analyze_statement_for_ref_mutation(
                    $s,
                    caller_name,
                    param_index_by_name,
                    caller_ref_params,
                    callee_ref_params,
                    direct_mutates,
                    edges,
                )
            };
        }

        match expr {
            Expr::Assign(assign, _) => {
                match assign.target.as_ref() {
                    Expr::Identifier(name, _) => {
                        if let Some(&idx) = param_index_by_name.get(name)
                            && caller_ref_params.get(idx).copied().unwrap_or(false)
                        {
                            direct_mutates[idx] = true;
                        }
                    }
                    Expr::IndexAccess { object, .. } | Expr::PropertyAccess { object, .. } => {
                        if let Expr::Identifier(name, _) = object.as_ref()
                            && let Some(&idx) = param_index_by_name.get(name)
                            && caller_ref_params.get(idx).copied().unwrap_or(false)
                        {
                            direct_mutates[idx] = true;
                        }
                    }
                    _ => {}
                }
                visit_expr!(&assign.value);
            }
            Expr::FunctionCall {
                name,
                args,
                named_args,
                ..
            } => {
                if let Some(callee_params) = callee_ref_params.get(name) {
                    for (arg_idx, arg) in args.iter().enumerate() {
                        if !callee_params.get(arg_idx).copied().unwrap_or(false) {
                            continue;
                        }
                        if let Some(caller_param_idx) = Self::ref_param_index_from_arg(
                            arg,
                            param_index_by_name,
                            caller_ref_params,
                        ) {
                            edges.push((
                                caller_name.to_string(),
                                caller_param_idx,
                                name.clone(),
                                arg_idx,
                            ));
                        }
                    }
                }
                // For callees not in the known function set (builtins, intrinsics,
                // imported functions), assume they do NOT mutate reference parameters.
                // Being too conservative here causes false B0004 errors when passing
                // non-identifier expressions (like object literals) to functions whose
                // parameters are inferred as references.

                for arg in args {
                    visit_expr!(arg);
                }

                for (_, arg) in named_args {
                    if let Some(idx) =
                        Self::ref_param_index_from_arg(arg, param_index_by_name, caller_ref_params)
                    {
                        direct_mutates[idx] = true;
                    }
                    visit_expr!(arg);
                }
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                visit_expr!(receiver);
                for arg in args {
                    visit_expr!(arg);
                }
                for (_, arg) in named_args {
                    visit_expr!(arg);
                }
            }
            Expr::UnaryOp { operand, .. }
            | Expr::Spread(operand, _)
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::TimeframeContext { expr: operand, .. }
            | Expr::UsingImpl { expr: operand, .. }
            | Expr::Reference { expr: operand, .. } => {
                visit_expr!(operand);
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                visit_expr!(left);
                visit_expr!(right);
            }
            Expr::PropertyAccess { object, .. } => {
                visit_expr!(object);
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                visit_expr!(object);
                visit_expr!(index);
                if let Some(end) = end_index {
                    visit_expr!(end);
                }
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                visit_expr!(condition);
                visit_expr!(then_expr);
                if let Some(else_expr) = else_expr {
                    visit_expr!(else_expr);
                }
            }
            Expr::Array(items, _) => {
                for item in items {
                    visit_expr!(item);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        shape_ast::ast::ObjectEntry::Field { value, .. } => {
                            visit_expr!(value);
                        }
                        shape_ast::ast::ObjectEntry::Spread(spread) => {
                            visit_expr!(spread);
                        }
                    }
                }
            }
            Expr::ListComprehension(comp, _) => {
                visit_expr!(&comp.element);
                for clause in &comp.clauses {
                    visit_expr!(&clause.iterable);
                    if let Some(filter) = &clause.filter {
                        visit_expr!(filter);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            if let Some(value) = &decl.value {
                                visit_expr!(value);
                            }
                        }
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            if let Some(name) = assign.pattern.as_identifier()
                                && let Some(&idx) = param_index_by_name.get(name)
                                && caller_ref_params.get(idx).copied().unwrap_or(false)
                            {
                                direct_mutates[idx] = true;
                            }
                            visit_expr!(&assign.value);
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            visit_stmt!(stmt);
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => {
                            visit_expr!(expr);
                        }
                    }
                }
            }
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    visit_stmt!(stmt);
                }
            }
            Expr::If(if_expr, _) => {
                visit_expr!(&if_expr.condition);
                visit_expr!(&if_expr.then_branch);
                if let Some(else_branch) = &if_expr.else_branch {
                    visit_expr!(else_branch);
                }
            }
            Expr::While(while_expr, _) => {
                visit_expr!(&while_expr.condition);
                visit_expr!(&while_expr.body);
            }
            Expr::For(for_expr, _) => {
                visit_expr!(&for_expr.iterable);
                visit_expr!(&for_expr.body);
            }
            Expr::Loop(loop_expr, _) => {
                visit_expr!(&loop_expr.body);
            }
            Expr::Let(let_expr, _) => {
                if let Some(value) = &let_expr.value {
                    visit_expr!(value);
                }
                visit_expr!(&let_expr.body);
            }
            Expr::Match(match_expr, _) => {
                visit_expr!(&match_expr.scrutinee);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        visit_expr!(guard);
                    }
                    visit_expr!(&arm.body);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    visit_expr!(&branch.expr);
                }
            }
            Expr::Annotated { target, .. } => {
                visit_expr!(target);
            }
            Expr::AsyncLet(async_let, _) => {
                visit_expr!(&async_let.expr);
            }
            Expr::AsyncScope(inner, _) => {
                visit_expr!(inner);
            }
            Expr::Comptime(stmts, _) => {
                for stmt in stmts {
                    visit_stmt!(stmt);
                }
            }
            Expr::ComptimeFor(cf, _) => {
                visit_expr!(&cf.iterable);
                for stmt in &cf.body {
                    visit_stmt!(stmt);
                }
            }
            Expr::SimulationCall { params, .. } => {
                for (_, value) in params {
                    visit_expr!(value);
                }
            }
            Expr::WindowExpr(window_expr, _) => {
                match &window_expr.function {
                    shape_ast::ast::WindowFunction::Lag { expr, default, .. }
                    | shape_ast::ast::WindowFunction::Lead { expr, default, .. } => {
                        visit_expr!(expr);
                        if let Some(default) = default {
                            visit_expr!(default);
                        }
                    }
                    shape_ast::ast::WindowFunction::FirstValue(expr)
                    | shape_ast::ast::WindowFunction::LastValue(expr)
                    | shape_ast::ast::WindowFunction::NthValue(expr, _)
                    | shape_ast::ast::WindowFunction::Sum(expr)
                    | shape_ast::ast::WindowFunction::Avg(expr)
                    | shape_ast::ast::WindowFunction::Min(expr)
                    | shape_ast::ast::WindowFunction::Max(expr) => {
                        visit_expr!(expr);
                    }
                    shape_ast::ast::WindowFunction::Count(expr) => {
                        if let Some(expr) = expr {
                            visit_expr!(expr);
                        }
                    }
                    shape_ast::ast::WindowFunction::RowNumber
                    | shape_ast::ast::WindowFunction::Rank
                    | shape_ast::ast::WindowFunction::DenseRank
                    | shape_ast::ast::WindowFunction::Ntile(_) => {}
                }

                for partition_expr in &window_expr.over.partition_by {
                    visit_expr!(partition_expr);
                }
                if let Some(order_by) = &window_expr.over.order_by {
                    for (order_expr, _) in &order_by.columns {
                        visit_expr!(order_expr);
                    }
                }
            }
            Expr::FromQuery(fq, _) => {
                visit_expr!(&fq.source);
                for clause in &fq.clauses {
                    match clause {
                        shape_ast::ast::QueryClause::Where(expr) => {
                            visit_expr!(expr);
                        }
                        shape_ast::ast::QueryClause::OrderBy(items) => {
                            for item in items {
                                visit_expr!(&item.key);
                            }
                        }
                        shape_ast::ast::QueryClause::GroupBy { element, key, .. } => {
                            visit_expr!(element);
                            visit_expr!(key);
                        }
                        shape_ast::ast::QueryClause::Let { value, .. } => {
                            visit_expr!(value);
                        }
                        shape_ast::ast::QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            visit_expr!(source);
                            visit_expr!(left_key);
                            visit_expr!(right_key);
                        }
                    }
                }
                visit_expr!(&fq.select);
            }
            Expr::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    visit_expr!(value);
                }
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                shape_ast::ast::EnumConstructorPayload::Unit => {}
                shape_ast::ast::EnumConstructorPayload::Tuple(values) => {
                    for value in values {
                        visit_expr!(value);
                    }
                }
                shape_ast::ast::EnumConstructorPayload::Struct(fields) => {
                    for (_, value) in fields {
                        visit_expr!(value);
                    }
                }
            },
            Expr::TypeAssertion {
                expr,
                meta_param_overrides,
                ..
            } => {
                visit_expr!(expr);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values() {
                        visit_expr!(value);
                    }
                }
            }
            Expr::InstanceOf { expr, .. } => {
                visit_expr!(expr);
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    visit_expr!(start);
                }
                if let Some(end) = end {
                    visit_expr!(end);
                }
            }
            Expr::DataRelativeAccess { reference, .. } => {
                visit_expr!(reference);
            }
            Expr::Break(Some(expr), _) | Expr::Return(Some(expr), _) => {
                visit_expr!(expr);
            }
            Expr::Literal(..)
            | Expr::Identifier(..)
            | Expr::DataRef(..)
            | Expr::DataDateTimeRef(..)
            | Expr::TimeRef(..)
            | Expr::DateTime(..)
            | Expr::PatternRef(..)
            | Expr::Unit(..)
            | Expr::Duration(..)
            | Expr::Continue(..)
            | Expr::Break(None, _)
            | Expr::Return(None, _) => {}
        }
    }
}
