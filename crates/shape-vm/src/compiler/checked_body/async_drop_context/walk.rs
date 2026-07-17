//! Suspension-point scan for the D6 async-drop-context gate
//! ([`super::BytecodeCompiler::reject_generated_drop_obligated_across_suspension`]).
//!
//! `true` iff a generated body contains any suspension point: `await`,
//! `async scope`, `async let`, `join`, or a `for await` loop. Drop-obligation
//! is NOT scanned here (it is read from the emission authority — see the parent
//! module); this is purely the suspension half.
//!
//! The walk is EXHAUSTIVE (no wildcard arm), mirroring
//! `transform::stamp_generated_closures`: a new `Expr`/`Statement` variant is a
//! compile failure here, so a future suspension-bearing node cannot silently
//! escape the fail-closed check. Note `await` lowers to no MIR task boundary
//! (`mir/lowering/expr.rs`), so the AST `Expr::Await` node — not the solver's
//! `task_boundary_loans` — is the authoritative suspension surface.

use shape_ast::ast::expr_helpers::{BlockItem, ComprehensionClause, QueryClause};
use shape_ast::ast::expressions::EnumConstructorPayload;
use shape_ast::ast::statements::ForInit;
use shape_ast::ast::windows::{WindowExpr, WindowFunction, WindowSpec};
use shape_ast::ast::{Expr, ObjectEntry, Statement};

pub(super) fn body_has_suspension_point(statements: &[Statement]) -> bool {
    statements.iter().any(statement_has_suspension)
}

fn any_expr(exprs: &[Expr]) -> bool {
    exprs.iter().any(expr_has_suspension)
}

fn any_named(named: &[(String, Expr)]) -> bool {
    named.iter().any(|(_, expr)| expr_has_suspension(expr))
}

fn statement_has_suspension(statement: &Statement) -> bool {
    match statement {
        Statement::Return(value, _) => value.as_ref().is_some_and(expr_has_suspension),
        Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => false,
        Statement::VariableDecl(decl, _) => decl.value.as_ref().is_some_and(expr_has_suspension),
        Statement::Assignment(assign, _) => expr_has_suspension(&assign.value),
        Statement::Expression(expr, _) => expr_has_suspension(expr),
        Statement::For(for_loop, _) => {
            for_loop.is_async
                || match &for_loop.init {
                    ForInit::ForIn { iter, .. } => expr_has_suspension(iter),
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        statement_has_suspension(init)
                            || expr_has_suspension(condition)
                            || expr_has_suspension(update)
                    }
                }
                || body_has_suspension_point(&for_loop.body)
        }
        Statement::While(while_loop, _) => {
            expr_has_suspension(&while_loop.condition)
                || body_has_suspension_point(&while_loop.body)
        }
        Statement::If(if_stmt, _) => {
            expr_has_suspension(&if_stmt.condition)
                || body_has_suspension_point(&if_stmt.then_body)
                || if_stmt
                    .else_body
                    .as_ref()
                    .is_some_and(|body| body_has_suspension_point(body))
        }
        Statement::Extend(extend, _) => extend.methods.iter().any(|method| {
            method
                .params
                .iter()
                .any(|param| param.default_value.as_ref().is_some_and(expr_has_suspension))
                || method.when_clause.as_ref().is_some_and(expr_has_suspension)
                || body_has_suspension_point(&method.body)
        }),
        Statement::SetParamType { .. } | Statement::SetReturnType { .. } => false,
        Statement::SetParamTypeExpr { expression, .. }
        | Statement::SetParamValue { expression, .. }
        | Statement::SetReturnExpr { expression, .. }
        | Statement::ReplaceBodyExpr { expression, .. }
        | Statement::ReplaceModuleExpr { expression, .. }
        | Statement::ExtendItemsExpr { expression, .. } => expr_has_suspension(expression),
        Statement::ReplaceBody { body, .. } => body_has_suspension_point(body),
    }
}

fn expr_has_suspension(expr: &Expr) -> bool {
    match expr {
        // Suspension points — presence is the whole answer.
        Expr::Await(_, _) | Expr::AsyncScope(_, _) | Expr::AsyncLet(_, _) | Expr::Join(_, _) => true,
        // Leaves.
        Expr::Literal(_, _)
        | Expr::Identifier(_, _)
        | Expr::DataRef(_, _)
        | Expr::DataDateTimeRef(_, _)
        | Expr::TimeRef(_, _)
        | Expr::DateTime(_, _)
        | Expr::PatternRef(_, _)
        | Expr::TypeSyntax(_, _)
        | Expr::Duration(_, _)
        | Expr::Continue(_)
        | Expr::Unit(_) => false,
        // Generated closures: descend (conservative, whole-body).
        Expr::FunctionExpr { params, body, .. } => {
            params
                .iter()
                .any(|param| param.default_value.as_ref().is_some_and(expr_has_suspension))
                || body_has_suspension_point(body)
        }
        // Binding carrier.
        Expr::Let(let_expr, _) => {
            let_expr.value.as_deref().is_some_and(expr_has_suspension)
                || expr_has_suspension(&let_expr.body)
        }
        // Single-child carriers.
        Expr::DataRelativeAccess { reference, .. } => expr_has_suspension(reference),
        Expr::PropertyAccess { object, .. } => expr_has_suspension(object),
        Expr::UnaryOp { operand, .. } => expr_has_suspension(operand),
        Expr::Spread(inner, _)
        | Expr::TryOperator(inner, _)
        | Expr::UsingImpl { expr: inner, .. }
        | Expr::InstanceOf { expr: inner, .. }
        | Expr::TimeframeContext { expr: inner, .. }
        | Expr::Reference { expr: inner, .. } => expr_has_suspension(inner),
        Expr::TypeAssertion {
            expr: inner,
            meta_param_overrides,
            ..
        } => {
            expr_has_suspension(inner)
                || meta_param_overrides
                    .as_ref()
                    .is_some_and(|overrides| overrides.values().any(expr_has_suspension))
        }
        Expr::Annotated {
            annotation, target, ..
        } => any_expr(&annotation.args) || expr_has_suspension(target),
        Expr::Break(value, _) | Expr::Return(value, _) => {
            value.as_ref().is_some_and(expr_has_suspension)
        }
        // Multi-child carriers.
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            expr_has_suspension(object)
                || expr_has_suspension(index)
                || end_index.as_ref().is_some_and(|e| expr_has_suspension(e))
        }
        Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
            expr_has_suspension(left) || expr_has_suspension(right)
        }
        Expr::FunctionCall {
            const_args,
            args,
            named_args,
            ..
        }
        | Expr::QualifiedFunctionCall {
            const_args,
            args,
            named_args,
            ..
        } => any_expr(const_args) || any_expr(args) || any_named(named_args),
        Expr::MethodCall {
            receiver,
            args,
            named_args,
            ..
        } => expr_has_suspension(receiver) || any_expr(args) || any_named(named_args),
        Expr::EnumConstructor { payload, .. } => match payload {
            EnumConstructorPayload::Unit => false,
            EnumConstructorPayload::Tuple(values) => any_expr(values),
            EnumConstructorPayload::Struct(fields) => any_named(fields),
        },
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            expr_has_suspension(condition)
                || expr_has_suspension(then_expr)
                || else_expr.as_ref().is_some_and(|e| expr_has_suspension(e))
        }
        Expr::Object(entries, _) => entries.iter().any(|entry| match entry {
            ObjectEntry::Field { value, .. } => expr_has_suspension(value),
            ObjectEntry::Spread(value) => expr_has_suspension(value),
        }),
        Expr::Array(elements, _) => any_expr(elements),
        Expr::TableRows(rows, _) => rows.iter().any(|row| any_expr(row)),
        Expr::StructLiteral { fields, .. } => any_named(fields),
        Expr::SimulationCall { params, .. } => any_named(params),
        Expr::ListComprehension(comprehension, _) => {
            expr_has_suspension(&comprehension.element)
                || comprehension.clauses.iter().any(
                    |ComprehensionClause {
                         pattern: _,
                         iterable,
                         filter,
                     }| {
                        expr_has_suspension(iterable)
                            || filter.as_ref().is_some_and(|f| expr_has_suspension(f))
                    },
                )
        }
        Expr::Block(block, _) => block.items.iter().any(|item| match item {
            BlockItem::VariableDecl(decl) => decl.value.as_ref().is_some_and(expr_has_suspension),
            BlockItem::Assignment(assign) => expr_has_suspension(&assign.value),
            BlockItem::Statement(statement) => statement_has_suspension(statement),
            BlockItem::Expression(expr) => expr_has_suspension(expr),
        }),
        Expr::If(if_expr, _) => {
            expr_has_suspension(&if_expr.condition)
                || expr_has_suspension(&if_expr.then_branch)
                || if_expr
                    .else_branch
                    .as_ref()
                    .is_some_and(|e| expr_has_suspension(e))
        }
        Expr::While(while_expr, _) => {
            expr_has_suspension(&while_expr.condition) || expr_has_suspension(&while_expr.body)
        }
        Expr::For(for_expr, _) => {
            for_expr.is_async
                || expr_has_suspension(&for_expr.iterable)
                || expr_has_suspension(&for_expr.body)
        }
        Expr::Loop(loop_expr, _) => expr_has_suspension(&loop_expr.body),
        Expr::Assign(assign_expr, _) => {
            expr_has_suspension(&assign_expr.target) || expr_has_suspension(&assign_expr.value)
        }
        Expr::Match(match_expr, _) => {
            expr_has_suspension(&match_expr.scrutinee)
                || match_expr.arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(expr_has_suspension)
                        || expr_has_suspension(&arm.body)
                })
        }
        Expr::Range { start, end, .. } => {
            start.as_ref().is_some_and(|s| expr_has_suspension(s))
                || end.as_ref().is_some_and(|e| expr_has_suspension(e))
        }
        Expr::Comptime(body, _) => body_has_suspension_point(body),
        Expr::ComptimeFor(comptime_for, _) => {
            expr_has_suspension(&comptime_for.iterable)
                || body_has_suspension_point(&comptime_for.body)
        }
        Expr::FromQuery(query, _) => {
            expr_has_suspension(&query.source)
                || query.clauses.iter().any(|clause| match clause {
                    QueryClause::Where(condition) => expr_has_suspension(condition),
                    QueryClause::OrderBy(specs) => {
                        specs.iter().any(|spec| expr_has_suspension(&spec.key))
                    }
                    QueryClause::GroupBy { element, key, .. } => {
                        expr_has_suspension(element) || expr_has_suspension(key)
                    }
                    QueryClause::Join {
                        source,
                        left_key,
                        right_key,
                        ..
                    } => {
                        expr_has_suspension(source)
                            || expr_has_suspension(left_key)
                            || expr_has_suspension(right_key)
                    }
                    QueryClause::Let { value, .. } => expr_has_suspension(value),
                })
                || expr_has_suspension(&query.select)
        }
        Expr::WindowExpr(window, _) => window_has_suspension(window),
    }
}

fn window_has_suspension(window: &WindowExpr) -> bool {
    let WindowExpr { function, over } = window;
    let function_suspends = match function {
        WindowFunction::Lag {
            expr,
            default,
            offset: _,
        }
        | WindowFunction::Lead {
            expr,
            default,
            offset: _,
        } => expr_has_suspension(expr) || default.as_ref().is_some_and(|d| expr_has_suspension(d)),
        WindowFunction::RowNumber
        | WindowFunction::Rank
        | WindowFunction::DenseRank
        | WindowFunction::Ntile(_) => false,
        WindowFunction::FirstValue(expr)
        | WindowFunction::LastValue(expr)
        | WindowFunction::NthValue(expr, _)
        | WindowFunction::Sum(expr)
        | WindowFunction::Avg(expr)
        | WindowFunction::Min(expr)
        | WindowFunction::Max(expr) => expr_has_suspension(expr),
        WindowFunction::Count(expr) => expr.as_ref().is_some_and(|e| expr_has_suspension(e)),
    };
    let WindowSpec {
        partition_by,
        order_by,
        frame: _,
    } = over;
    function_suspends
        || any_expr(partition_by)
        || order_by
            .as_ref()
            .is_some_and(|order| order.columns.iter().any(|(key, _)| expr_has_suspension(key)))
}
