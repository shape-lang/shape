//! Exhaustive read-only body traversal for the D6 async-drop-context scan
//! ([`super::AsyncDropContextScan`]). It mirrors
//! `transform::stamp_generated_closures`: a no-wildcard `Expr`/`Statement`
//! match makes a new AST variant a compile failure here, so a future
//! suspension- or binding-bearing node cannot silently escape the fail-closed
//! check. Drop-obligation is resolved through the SAME queries the emission
//! drop-plan uses (`tracked_type_name_from_annotation`,
//! `initializer_call_return_drop_type`, `drop_type_info`).

use shape_ast::ast::expr_helpers::{BlockItem, ComprehensionClause, QueryClause};
use shape_ast::ast::expressions::EnumConstructorPayload;
use shape_ast::ast::statements::ForInit;
use shape_ast::ast::windows::{WindowExpr, WindowFunction, WindowSpec};
use shape_ast::ast::{Expr, FunctionParameter, ObjectEntry, Statement, TypeAnnotation};

use super::AsyncDropContextScan;
use crate::compiler::BytecodeCompiler;

impl<'c> AsyncDropContextScan<'c> {
    pub(super) fn new(compiler: &'c BytecodeCompiler) -> Self {
        Self {
            compiler,
            saw_suspension: false,
            drop_obligated_type: None,
        }
    }

    /// Record a drop obligation if `type_name` carries a `Drop` impl (first hit
    /// wins; the name is diagnostic-only — any drop-obligated binding triggers
    /// the same conservative rejection).
    fn note_type_name(&mut self, type_name: &str) {
        if self.drop_obligated_type.is_none() && self.compiler.drop_type_info.contains_key(type_name)
        {
            self.drop_obligated_type = Some(type_name.to_string());
        }
    }

    fn note_annotation(&mut self, annotation: &TypeAnnotation) {
        if let Some(type_name) = BytecodeCompiler::tracked_type_name_from_annotation(annotation) {
            self.note_type_name(&type_name);
        }
    }

    /// Note the drop obligation implied by one binding's declared type or its
    /// initializer — the drop-plan's own static resolution sources: an explicit
    /// annotation, a struct-literal initializer (the local's inferred type is
    /// the struct name), or a call whose DECLARED return type is a `Drop` type.
    fn note_binding(&mut self, annotation: Option<&TypeAnnotation>, value: Option<&Expr>) {
        if self.drop_obligated_type.is_some() {
            return;
        }
        if let Some(annotation) = annotation {
            self.note_annotation(annotation);
        }
        if let Some(value) = value {
            if let Expr::StructLiteral { type_name, .. } = value {
                self.note_type_name(type_name.name());
            }
            if let Some((type_name, _kind)) = self.compiler.initializer_call_return_drop_type(value)
            {
                self.note_type_name(&type_name);
            }
        }
    }

    pub(super) fn note_params(&mut self, params: &[FunctionParameter]) {
        for param in params {
            if let Some(annotation) = param.type_annotation.as_ref() {
                self.note_annotation(annotation);
            }
            if let Some(default) = param.default_value.as_ref() {
                self.walk_expr(default);
            }
        }
    }

    pub(super) fn walk_statements(&mut self, statements: &[Statement]) {
        for statement in statements {
            self.walk_statement(statement);
        }
    }

    fn walk_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Return(value, _) => {
                if let Some(value) = value {
                    self.walk_expr(value);
                }
            }
            Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => {}
            Statement::VariableDecl(decl, _) => {
                self.note_binding(decl.type_annotation.as_ref(), decl.value.as_ref());
                if let Some(value) = decl.value.as_ref() {
                    self.walk_expr(value);
                }
            }
            Statement::Assignment(assign, _) => self.walk_expr(&assign.value),
            Statement::Expression(expr, _) => self.walk_expr(expr),
            Statement::For(for_loop, _) => {
                if for_loop.is_async {
                    self.saw_suspension = true;
                }
                match &for_loop.init {
                    ForInit::ForIn { pattern: _, iter } => self.walk_expr(iter),
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.walk_statement(init);
                        self.walk_expr(condition);
                        self.walk_expr(update);
                    }
                }
                self.walk_statements(&for_loop.body);
            }
            Statement::While(while_loop, _) => {
                self.walk_expr(&while_loop.condition);
                self.walk_statements(&while_loop.body);
            }
            Statement::If(if_stmt, _) => {
                self.walk_expr(&if_stmt.condition);
                self.walk_statements(&if_stmt.then_body);
                if let Some(else_body) = if_stmt.else_body.as_ref() {
                    self.walk_statements(else_body);
                }
            }
            Statement::Extend(extend, _) => {
                for method in &extend.methods {
                    for param in &method.params {
                        if let Some(default) = param.default_value.as_ref() {
                            self.walk_expr(default);
                        }
                    }
                    if let Some(when_clause) = method.when_clause.as_ref() {
                        self.walk_expr(when_clause);
                    }
                    self.walk_statements(&method.body);
                }
            }
            Statement::SetParamType { .. } | Statement::SetReturnType { .. } => {}
            Statement::SetParamTypeExpr { expression, .. }
            | Statement::SetParamValue { expression, .. }
            | Statement::SetReturnExpr { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. }
            | Statement::ExtendItemsExpr { expression, .. } => self.walk_expr(expression),
            Statement::ReplaceBody { body, .. } => self.walk_statements(body),
        }
    }

    fn walk_exprs(&mut self, exprs: &[Expr]) {
        for expr in exprs {
            self.walk_expr(expr);
        }
    }

    fn walk_named(&mut self, named: &[(String, Expr)]) {
        for (_, expr) in named {
            self.walk_expr(expr);
        }
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            // Suspension points.
            Expr::Await(inner, _) | Expr::AsyncScope(inner, _) => {
                self.saw_suspension = true;
                self.walk_expr(inner);
            }
            Expr::AsyncLet(async_let, _) => {
                self.saw_suspension = true;
                self.walk_expr(&async_let.expr);
            }
            Expr::Join(join_expr, _) => {
                self.saw_suspension = true;
                for branch in &join_expr.branches {
                    for annotation in &branch.annotations {
                        self.walk_exprs(&annotation.args);
                    }
                    self.walk_expr(&branch.expr);
                }
            }
            // Binding carrier (drop-obligation site).
            Expr::Let(let_expr, _) => {
                self.note_binding(let_expr.type_annotation.as_ref(), let_expr.value.as_deref());
                if let Some(value) = let_expr.value.as_deref() {
                    self.walk_expr(value);
                }
                self.walk_expr(&let_expr.body);
            }
            // Generated closures: descend (conservative, whole-body).
            Expr::FunctionExpr { params, body, .. } => {
                for param in params.iter() {
                    if let Some(default) = param.default_value.as_ref() {
                        self.walk_expr(default);
                    }
                }
                self.walk_statements(body);
            }
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
            | Expr::Unit(_) => {}
            // Single-child carriers.
            Expr::DataRelativeAccess { reference, .. } => self.walk_expr(reference),
            Expr::PropertyAccess { object, .. } => self.walk_expr(object),
            Expr::UnaryOp { operand, .. } => self.walk_expr(operand),
            Expr::Spread(inner, _) => self.walk_expr(inner),
            Expr::TryOperator(inner, _) => self.walk_expr(inner),
            Expr::UsingImpl { expr: inner, .. } => self.walk_expr(inner),
            Expr::TypeAssertion {
                expr: inner,
                meta_param_overrides,
                ..
            } => {
                self.walk_expr(inner);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values() {
                        self.walk_expr(value);
                    }
                }
            }
            Expr::InstanceOf { expr: inner, .. } => self.walk_expr(inner),
            Expr::TimeframeContext { expr: inner, .. } => self.walk_expr(inner),
            Expr::Reference { expr: inner, .. } => self.walk_expr(inner),
            Expr::Annotated {
                annotation, target, ..
            } => {
                self.walk_exprs(&annotation.args);
                self.walk_expr(target);
            }
            Expr::Break(value, _) | Expr::Return(value, _) => {
                if let Some(value) = value {
                    self.walk_expr(value);
                }
            }
            // Multi-child carriers.
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                self.walk_expr(object);
                self.walk_expr(index);
                if let Some(end_index) = end_index {
                    self.walk_expr(end_index);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
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
                self.walk_exprs(const_args);
                self.walk_exprs(args);
                self.walk_named(named_args);
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                self.walk_expr(receiver);
                self.walk_exprs(args);
                self.walk_named(named_args);
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                EnumConstructorPayload::Unit => {}
                EnumConstructorPayload::Tuple(values) => self.walk_exprs(values),
                EnumConstructorPayload::Struct(fields) => self.walk_named(fields),
            },
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.walk_expr(condition);
                self.walk_expr(then_expr);
                if let Some(else_expr) = else_expr {
                    self.walk_expr(else_expr);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } => self.walk_expr(value),
                        ObjectEntry::Spread(value) => self.walk_expr(value),
                    }
                }
            }
            Expr::Array(elements, _) => self.walk_exprs(elements),
            Expr::TableRows(rows, _) => {
                for row in rows {
                    self.walk_exprs(row);
                }
            }
            Expr::StructLiteral { fields, .. } => self.walk_named(fields),
            Expr::SimulationCall { params, .. } => self.walk_named(params),
            Expr::ListComprehension(comprehension, _) => {
                self.walk_expr(&comprehension.element);
                for ComprehensionClause {
                    pattern: _,
                    iterable,
                    filter,
                } in &comprehension.clauses
                {
                    self.walk_expr(iterable);
                    if let Some(filter) = filter {
                        self.walk_expr(filter);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        BlockItem::VariableDecl(decl) => {
                            self.note_binding(decl.type_annotation.as_ref(), decl.value.as_ref());
                            if let Some(value) = decl.value.as_ref() {
                                self.walk_expr(value);
                            }
                        }
                        BlockItem::Assignment(assign) => self.walk_expr(&assign.value),
                        BlockItem::Statement(statement) => self.walk_statement(statement),
                        BlockItem::Expression(expr) => self.walk_expr(expr),
                    }
                }
            }
            Expr::If(if_expr, _) => {
                self.walk_expr(&if_expr.condition);
                self.walk_expr(&if_expr.then_branch);
                if let Some(else_branch) = if_expr.else_branch.as_ref() {
                    self.walk_expr(else_branch);
                }
            }
            Expr::While(while_expr, _) => {
                self.walk_expr(&while_expr.condition);
                self.walk_expr(&while_expr.body);
            }
            Expr::For(for_expr, _) => {
                if for_expr.is_async {
                    self.saw_suspension = true;
                }
                self.walk_expr(&for_expr.iterable);
                self.walk_expr(&for_expr.body);
            }
            Expr::Loop(loop_expr, _) => self.walk_expr(&loop_expr.body),
            Expr::Assign(assign_expr, _) => {
                self.walk_expr(&assign_expr.target);
                self.walk_expr(&assign_expr.value);
            }
            Expr::Match(match_expr, _) => {
                self.walk_expr(&match_expr.scrutinee);
                for arm in &match_expr.arms {
                    if let Some(guard) = arm.guard.as_ref() {
                        self.walk_expr(guard);
                    }
                    self.walk_expr(&arm.body);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    self.walk_expr(start);
                }
                if let Some(end) = end {
                    self.walk_expr(end);
                }
            }
            Expr::Comptime(body, _) => self.walk_statements(body),
            Expr::ComptimeFor(comptime_for, _) => {
                self.walk_expr(&comptime_for.iterable);
                self.walk_statements(&comptime_for.body);
            }
            Expr::FromQuery(query, _) => {
                self.walk_expr(&query.source);
                for clause in &query.clauses {
                    match clause {
                        QueryClause::Where(condition) => self.walk_expr(condition),
                        QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                self.walk_expr(&spec.key);
                            }
                        }
                        QueryClause::GroupBy { element, key, .. } => {
                            self.walk_expr(element);
                            self.walk_expr(key);
                        }
                        QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            self.walk_expr(source);
                            self.walk_expr(left_key);
                            self.walk_expr(right_key);
                        }
                        QueryClause::Let { value, .. } => self.walk_expr(value),
                    }
                }
                self.walk_expr(&query.select);
            }
            Expr::WindowExpr(window, _) => self.walk_window(window),
        }
    }

    fn walk_window(&mut self, window: &WindowExpr) {
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
                self.walk_expr(expr);
                if let Some(default) = default {
                    self.walk_expr(default);
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
            | WindowFunction::Max(expr) => self.walk_expr(expr),
            WindowFunction::Count(expr) => {
                if let Some(expr) = expr {
                    self.walk_expr(expr);
                }
            }
        }
        let WindowSpec {
            partition_by,
            order_by,
            frame: _,
        } = over;
        self.walk_exprs(partition_by);
        if let Some(order_by) = order_by {
            for (key, _direction) in &order_by.columns {
                self.walk_expr(key);
            }
        }
    }
}
