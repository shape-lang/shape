//! ADR-009 generated-closure provenance stamping.
//! The capture gate reads compiler-issued provenance on each closure node,
//! replacing the former generated-function name predicate that failed across
//! monomorphization, replacement bodies, and nested closures. The exhaustive
//! walk makes new AST variants compile failures and stamps deterministic
//! structural paths (`extend:Type/method:name/closure:N`). Capture spelling,
//! source locations, owner prose, and traversal/file order are not identity.

use crate::ast::expr_helpers::{BlockItem, ComprehensionClause, QueryClause};
use crate::ast::patterns::DestructurePattern;
use crate::ast::provenance::GeneratedNodeOrigin;
use crate::ast::statements::ForInit;
use crate::ast::windows::{WindowExpr, WindowFunction, WindowSpec};
use crate::ast::{Expr, ObjectEntry, Statement};

mod path_cursor;
mod source_paths;
use path_cursor::GeneratedClosurePathCursor;
pub use source_paths::{GeneratedClosureSourcePath, generated_closure_source_paths};

/// Stamp every closure literal in a generated body (and every closure nested
/// inside those) with its provenance.
///
/// Re-stamping is idempotent because path indices are traversal-derived.
pub fn stamp_generated_closures(body: &mut [Statement], origin: &GeneratedNodeOrigin) {
    let mut walker = Stamper {
        origin,
        paths: GeneratedClosurePathCursor::new(origin.path().clone()),
    };
    walker.statements(body);
}

struct Stamper<'origin> {
    origin: &'origin GeneratedNodeOrigin,
    paths: GeneratedClosurePathCursor,
}
impl Stamper<'_> {
    fn statements(&mut self, stmts: &mut [Statement]) {
        for stmt in stmts {
            self.statement(stmt);
        }
    }

    fn statement(&mut self, stmt: &mut Statement) {
        match stmt {
            Statement::Return(value, _) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => {}
            Statement::VariableDecl(decl, _) => {
                self.destructure_pattern(&mut decl.pattern);
                if let Some(value) = decl.value.as_mut() {
                    self.expr(value);
                }
            }
            Statement::Assignment(assign, _) => {
                self.destructure_pattern(&mut assign.pattern);
                self.expr(&mut assign.value);
            }
            Statement::Expression(expr, _) => self.expr(expr),
            Statement::For(for_loop, _) => {
                match &mut for_loop.init {
                    ForInit::ForIn { pattern, iter } => {
                        self.destructure_pattern(pattern);
                        self.expr(iter);
                    }
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.statement(init);
                        self.expr(condition);
                        self.expr(update);
                    }
                }
                self.statements(&mut for_loop.body);
            }
            Statement::While(while_loop, _) => {
                self.expr(&mut while_loop.condition);
                self.statements(&mut while_loop.body);
            }
            Statement::If(if_stmt, _) => {
                self.expr(&mut if_stmt.condition);
                self.statements(&mut if_stmt.then_body);
                if let Some(else_body) = if_stmt.else_body.as_mut() {
                    self.statements(else_body);
                }
            }
            Statement::Extend(extend, _) => {
                for method in &mut extend.methods {
                    for param in &mut method.params {
                        if let Some(default) = param.default_value.as_mut() {
                            self.expr(default);
                        }
                    }
                    if let Some(when_clause) = method.when_clause.as_mut() {
                        self.expr(when_clause);
                    }
                    self.statements(&mut method.body);
                }
            }
            Statement::SetParamType { .. } | Statement::SetReturnType { .. } => {}
            Statement::SetParamTypeExpr { expression, .. }
            | Statement::SetParamValue { expression, .. }
            | Statement::SetReturnExpr { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. }
            | Statement::ExtendItemsExpr { expression, .. } => self.expr(expression),
            Statement::ReplaceBody { body, .. } => self.statements(body),
        }
    }

    /// Exhaustive even though binding patterns carry no expressions today.
    fn destructure_pattern(&mut self, pattern: &mut DestructurePattern) {
        match pattern {
            DestructurePattern::Identifier(_, _) | DestructurePattern::Decomposition(_) => {}
            DestructurePattern::Array(elements) => {
                for element in elements {
                    self.destructure_pattern(element);
                }
            }
            DestructurePattern::Object(fields) => {
                for field in fields {
                    self.destructure_pattern(&mut field.pattern);
                }
            }
            DestructurePattern::Rest(inner) => self.destructure_pattern(inner),
        }
    }

    fn exprs(&mut self, exprs: &mut [Expr]) {
        for expr in exprs {
            self.expr(expr);
        }
    }

    fn named(&mut self, named: &mut [(String, Expr)]) {
        for (_, expr) in named {
            self.expr(expr);
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        match expr {
            // ── the node the whole module exists for ────────────────────────
            Expr::FunctionExpr {
                params,
                return_type: _,
                body,
                generated_origin,
                // Capture clauses are authored by the generator; stamping only
                // attaches provenance and never rewrites the declaration.
                captures: _,
                span: _,
            } => {
                let closure_path = self.paths.next_closure();
                // Parameter defaults are evaluated in the ENCLOSING scope, so
                // they belong to the enclosing level's sibling numbering.
                for param in params.iter_mut() {
                    if let Some(default) = param.default_value.as_mut() {
                        self.expr(default);
                    }
                }
                let closure_origin = self.origin.child(closure_path.segment());
                debug_assert_eq!(closure_origin.path(), closure_path.path());
                *generated_origin = Some(closure_origin.clone());
                // Closures nested in this body hang off THIS closure's path.
                let mut nested = Stamper {
                    origin: &closure_origin,
                    paths: closure_path.nested_cursor(),
                };
                nested.statements(body);
            }

            // ── leaves ──────────────────────────────────────────────────────
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
            | Expr::Unit(_) => {}

            // ── single-child carriers ───────────────────────────────────────
            Expr::DataRelativeAccess { reference, .. } => self.expr(reference),
            Expr::PropertyAccess { object, .. } => self.expr(object),
            Expr::UnaryOp { operand, .. } => self.expr(operand),
            Expr::Spread(inner, _) => self.expr(inner),
            Expr::TryOperator(inner, _) => self.expr(inner),
            Expr::UsingImpl { expr: inner, .. } => self.expr(inner),
            Expr::Await(inner, _) => self.expr(inner),
            Expr::AsyncScope(inner, _) => self.expr(inner),
            Expr::TypeAssertion {
                expr: inner,
                meta_param_overrides,
                ..
            } => {
                self.expr(inner);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values_mut() {
                        self.expr(value);
                    }
                }
            }
            Expr::InstanceOf { expr: inner, .. } => self.expr(inner),
            Expr::TimeframeContext { expr: inner, .. } => self.expr(inner),
            Expr::Reference { expr: inner, .. } => self.expr(inner),
            Expr::Annotated {
                annotation, target, ..
            } => {
                self.exprs(&mut annotation.args);
                self.expr(target);
            }
            Expr::Break(value, _) | Expr::Return(value, _) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }

            // ── multi-child carriers ────────────────────────────────────────
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                self.expr(object);
                self.expr(index);
                if let Some(end_index) = end_index {
                    self.expr(end_index);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                self.expr(left);
                self.expr(right);
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
            } => {
                self.exprs(const_args);
                self.exprs(args);
                self.named(named_args);
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                self.expr(receiver);
                self.exprs(args);
                self.named(named_args);
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                crate::ast::expressions::EnumConstructorPayload::Unit => {}
                crate::ast::expressions::EnumConstructorPayload::Tuple(values) => {
                    self.exprs(values)
                }
                crate::ast::expressions::EnumConstructorPayload::Struct(fields) => {
                    self.named(fields)
                }
            },
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.expr(condition);
                self.expr(then_expr);
                if let Some(else_expr) = else_expr {
                    self.expr(else_expr);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } => self.expr(value),
                        ObjectEntry::Spread(value) => self.expr(value),
                    }
                }
            }
            Expr::Array(elements, _) => self.exprs(elements),
            Expr::TableRows(rows, _) => {
                for row in rows {
                    self.exprs(row);
                }
            }
            Expr::StructLiteral { fields, .. } => self.named(fields),
            Expr::SimulationCall { params, .. } => self.named(params),
            Expr::ListComprehension(comprehension, _) => {
                self.expr(&mut comprehension.element);
                for ComprehensionClause {
                    pattern,
                    iterable,
                    filter,
                } in &mut comprehension.clauses
                {
                    self.destructure_pattern(pattern);
                    self.expr(iterable);
                    if let Some(filter) = filter {
                        self.expr(filter);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &mut block.items {
                    match item {
                        BlockItem::VariableDecl(decl) => {
                            self.destructure_pattern(&mut decl.pattern);
                            if let Some(value) = decl.value.as_mut() {
                                self.expr(value);
                            }
                        }
                        BlockItem::Assignment(assign) => {
                            self.destructure_pattern(&mut assign.pattern);
                            self.expr(&mut assign.value);
                        }
                        BlockItem::Statement(stmt) => self.statement(stmt),
                        BlockItem::Expression(expr) => self.expr(expr),
                    }
                }
            }
            Expr::If(if_expr, _) => {
                self.expr(&mut if_expr.condition);
                self.expr(&mut if_expr.then_branch);
                if let Some(else_branch) = if_expr.else_branch.as_mut() {
                    self.expr(else_branch);
                }
            }
            Expr::While(while_expr, _) => {
                self.expr(&mut while_expr.condition);
                self.expr(&mut while_expr.body);
            }
            Expr::For(for_expr, _) => {
                self.expr(&mut for_expr.iterable);
                self.expr(&mut for_expr.body);
            }
            Expr::Loop(loop_expr, _) => self.expr(&mut loop_expr.body),
            Expr::Let(let_expr, _) => {
                if let Some(value) = let_expr.value.as_mut() {
                    self.expr(value);
                }
                self.expr(&mut let_expr.body);
            }
            Expr::Assign(assign_expr, _) => {
                self.expr(&mut assign_expr.target);
                self.expr(&mut assign_expr.value);
            }
            Expr::Match(match_expr, _) => {
                self.expr(&mut match_expr.scrutinee);
                for arm in &mut match_expr.arms {
                    if let Some(guard) = arm.guard.as_mut() {
                        self.expr(guard);
                    }
                    self.expr(&mut arm.body);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    self.expr(start);
                }
                if let Some(end) = end {
                    self.expr(end);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &mut join_expr.branches {
                    for annotation in &mut branch.annotations {
                        self.exprs(&mut annotation.args);
                    }
                    self.expr(&mut branch.expr);
                }
            }
            Expr::AsyncLet(async_let, _) => self.expr(&mut async_let.expr),
            Expr::Comptime(body, _) => self.statements(body),
            Expr::ComptimeFor(comptime_for, _) => {
                self.expr(&mut comptime_for.iterable);
                self.statements(&mut comptime_for.body);
            }
            Expr::FromQuery(query, _) => {
                self.expr(&mut query.source);
                for clause in &mut query.clauses {
                    match clause {
                        QueryClause::Where(condition) => self.expr(condition),
                        QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                self.expr(&mut spec.key);
                            }
                        }
                        QueryClause::GroupBy { element, key, .. } => {
                            self.expr(element);
                            self.expr(key);
                        }
                        QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            self.expr(source);
                            self.expr(left_key);
                            self.expr(right_key);
                        }
                        QueryClause::Let { value, .. } => self.expr(value),
                    }
                }
                self.expr(&mut query.select);
            }
            Expr::WindowExpr(window, _) => self.window_expr(window),
        }
    }

    fn window_expr(&mut self, window: &mut WindowExpr) {
        let WindowExpr { function, over } = window;
        match function {
            WindowFunction::Lag {
                expr,
                default,
                offset: _,
            }
            | WindowFunction::Lead {
                expr,
                default,
                offset: _,
            } => {
                self.expr(expr);
                if let Some(default) = default {
                    self.expr(default);
                }
            }
            WindowFunction::RowNumber
            | WindowFunction::Rank
            | WindowFunction::DenseRank
            | WindowFunction::Ntile(_) => {}
            WindowFunction::FirstValue(expr)
            | WindowFunction::LastValue(expr)
            | WindowFunction::NthValue(expr, _)
            | WindowFunction::Sum(expr)
            | WindowFunction::Avg(expr)
            | WindowFunction::Min(expr)
            | WindowFunction::Max(expr) => self.expr(expr),
            WindowFunction::Count(expr) => {
                if let Some(expr) = expr {
                    self.expr(expr);
                }
            }
        }
        let WindowSpec {
            partition_by,
            order_by,
            frame: _,
        } = over;
        self.exprs(partition_by);
        if let Some(order_by) = order_by {
            for (key, _direction) in &mut order_by.columns {
                self.expr(key);
            }
        }
    }
}

#[cfg(test)]
mod tests;
