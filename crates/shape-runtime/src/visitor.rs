//! AST Visitor trait and walk functions for Shape.
//!
//! This module provides a visitor pattern for traversing the AST.
//! All variants are explicitly handled - no wildcards.
//!
//! ## Per-Variant Expression Methods
//!
//! The `Visitor` trait provides fine-grained per-variant methods for expressions.
//! Each method has a default implementation that returns `true` (continue into
//! children). Override only the variants you care about.
//!
//! The visit order for each expression is:
//! 1. `visit_expr(expr)` — coarse pre-visit hook; return `false` to skip entirely
//! 2. `visit_<variant>(expr, span)` — per-variant hook; return `false` to skip children
//! 3. Walk children recursively
//! 4. `leave_expr(expr)` — post-visit hook

use shape_ast::ast::*;

/// A visitor trait for traversing Shape AST nodes.
///
/// All `visit_*` methods return `bool`:
/// - `true`: continue visiting children
/// - `false`: skip children
///
/// The `leave_*` methods are called after visiting all children.
///
/// ## Per-Variant Expression Methods
///
/// For finer granularity, override the per-variant expression methods
/// (e.g., `visit_identifier`, `visit_binary_op`, `visit_method_call`).
/// These are called from `walk_expr` after the coarse `visit_expr` hook.
/// Each receives the full `&Expr` node and its `Span`.
pub trait Visitor: Sized {
    // ===== Coarse-grained visitors (called on every node) =====

    /// Called before visiting any expression. Return `false` to skip entirely
    /// (neither per-variant method nor children will be visited).
    fn visit_expr(&mut self, _expr: &Expr) -> bool {
        true
    }
    /// Called after visiting an expression and all its children.
    fn leave_expr(&mut self, _expr: &Expr) {}

    // Statement visitors
    fn visit_stmt(&mut self, _stmt: &Statement) -> bool {
        true
    }
    fn leave_stmt(&mut self, _stmt: &Statement) {}

    // Item visitors
    fn visit_item(&mut self, _item: &Item) -> bool {
        true
    }
    fn leave_item(&mut self, _item: &Item) {}

    // Function definition visitors
    fn visit_function(&mut self, _func: &FunctionDef) -> bool {
        true
    }
    fn leave_function(&mut self, _func: &FunctionDef) {}

    // Literal visitors (kept for backward compat — also called from walk_expr)
    fn visit_literal(&mut self, _lit: &Literal) -> bool {
        true
    }
    fn leave_literal(&mut self, _lit: &Literal) {}

    // Block visitors (kept for backward compat — also called from walk_expr)
    fn visit_block(&mut self, _block: &BlockExpr) -> bool {
        true
    }
    fn leave_block(&mut self, _block: &BlockExpr) {}

    // ===== Per-variant expression visitors =====
    //
    // Each method receives the full &Expr and its Span. Return `true` to
    // continue walking children, `false` to skip children.
    //
    // Default implementations return `true` (walk children).

    fn visit_expr_literal(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_identifier(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_data_ref(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_data_datetime_ref(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_data_relative_access(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_property_access(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_index_access(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_binary_op(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_fuzzy_comparison(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_unary_op(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_function_call(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_enum_constructor(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_time_ref(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_datetime(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_pattern_ref(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_conditional(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_object(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_array(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_list_comprehension(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_block(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_type_assertion(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_instance_of(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_function_expr(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_duration(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_spread(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_if(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_while(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_for(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_loop(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_let(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_assign(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_break(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_continue(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_return(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_method_call(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_match(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_unit(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_range(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_timeframe_context(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_try_operator(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_using_impl(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_simulation_call(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_window_expr(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_from_query(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_struct_literal(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_await(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_join(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_annotated(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_async_let(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_async_scope(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_comptime(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_comptime_for(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
    fn visit_expr_reference(&mut self, _expr: &Expr, _span: Span) -> bool {
        true
    }
}

// ===== Walk Functions =====

/// Walk a program, visiting all items
pub fn walk_program<V: Visitor>(visitor: &mut V, program: &Program) {
    for item in &program.items {
        walk_item(visitor, item);
    }
}

/// Walk an item
pub fn walk_item<V: Visitor>(visitor: &mut V, item: &Item) {
    if !visitor.visit_item(item) {
        return;
    }

    match item {
        Item::Import(_, _) => {}
        Item::Module(module_def, _) => {
            for inner in &module_def.items {
                walk_item(visitor, inner);
            }
        }
        Item::Export(export, _) => match &export.item {
            ExportItem::Function(func) => walk_function(visitor, func),
            ExportItem::BuiltinFunction(_) => {}
            ExportItem::BuiltinType(_) => {}
            ExportItem::TypeAlias(_) => {}
            ExportItem::Named(_) => {}
            ExportItem::Enum(_) => {}
            ExportItem::Struct(_) => {}
            ExportItem::Interface(_) => {}
            ExportItem::Trait(_) => {}
            ExportItem::Annotation(annotation_def) => {
                for handler in &annotation_def.handlers {
                    walk_expr(visitor, &handler.body);
                }
            }
            ExportItem::ForeignFunction(_) => {} // foreign bodies are opaque
        },
        Item::TypeAlias(_, _) => {}
        Item::Interface(_, _) => {}
        Item::Trait(_, _) => {}
        Item::Enum(_, _) => {}
        Item::Extend(extend, _) => {
            for method in &extend.methods {
                for stmt in &method.body {
                    walk_stmt(visitor, stmt);
                }
            }
        }
        Item::Impl(impl_block, _) => {
            for method in &impl_block.methods {
                for stmt in &method.body {
                    walk_stmt(visitor, stmt);
                }
            }
        }
        Item::Function(func, _) => walk_function(visitor, func),
        Item::Query(query, _) => walk_query(visitor, query),
        Item::VariableDecl(decl, _) => {
            if let Some(value) = &decl.value {
                walk_expr(visitor, value);
            }
        }
        Item::Assignment(assign, _) => {
            walk_expr(visitor, &assign.value);
        }
        Item::Expression(expr, _) => walk_expr(visitor, expr),
        Item::Stream(stream, _) => {
            for decl in &stream.state {
                if let Some(value) = &decl.value {
                    walk_expr(visitor, value);
                }
            }
            if let Some(stmts) = &stream.on_connect {
                for stmt in stmts {
                    walk_stmt(visitor, stmt);
                }
            }
            if let Some(stmts) = &stream.on_disconnect {
                for stmt in stmts {
                    walk_stmt(visitor, stmt);
                }
            }
            if let Some(on_event) = &stream.on_event {
                for stmt in &on_event.body {
                    walk_stmt(visitor, stmt);
                }
            }
            if let Some(on_window) = &stream.on_window {
                for stmt in &on_window.body {
                    walk_stmt(visitor, stmt);
                }
            }
            if let Some(on_error) = &stream.on_error {
                for stmt in &on_error.body {
                    walk_stmt(visitor, stmt);
                }
            }
        }
        Item::Test(test, _) => {
            if let Some(setup) = &test.setup {
                for stmt in setup {
                    walk_stmt(visitor, stmt);
                }
            }
            if let Some(teardown) = &test.teardown {
                for stmt in teardown {
                    walk_stmt(visitor, stmt);
                }
            }
            for case in &test.test_cases {
                for test_stmt in &case.body {
                    walk_test_statement(visitor, test_stmt);
                }
            }
        }
        Item::Optimize(opt, _) => {
            walk_expr(visitor, &opt.range.0);
            walk_expr(visitor, &opt.range.1);
            if let OptimizationMetric::Custom(expr) = &opt.metric {
                walk_expr(visitor, expr);
            }
        }
        Item::Statement(stmt, _) => walk_stmt(visitor, stmt),
        Item::AnnotationDef(ann_def, _) => {
            // Walk the lifecycle handlers of the annotation definition
            for handler in &ann_def.handlers {
                walk_expr(visitor, &handler.body);
            }
        }
        Item::StructType(_, _) => {
            // No expressions to walk in struct type definitions
        }
        Item::DataSource(ds, _) => {
            walk_expr(visitor, &ds.provider_expr);
        }
        Item::QueryDecl(_, _) => {
            // Query declarations have no walkable expressions (SQL is a string literal)
        }
        Item::Comptime(stmts, _) => {
            for stmt in stmts {
                walk_stmt(visitor, stmt);
            }
        }
        Item::BuiltinTypeDecl(_, _) => {
            // Declaration-only intrinsic
        }
        Item::BuiltinFunctionDecl(_, _) => {
            // Declaration-only intrinsic
        }
        Item::ForeignFunction(_, _) => {
            // Foreign function bodies are opaque to the Shape visitor
        }
    }

    visitor.leave_item(item);
}

/// Walk a function definition
pub fn walk_function<V: Visitor>(visitor: &mut V, func: &FunctionDef) {
    if !visitor.visit_function(func) {
        return;
    }

    // Visit parameter default values
    for param in &func.params {
        if let Some(default) = &param.default_value {
            walk_expr(visitor, default);
        }
    }

    // Visit body statements
    for stmt in &func.body {
        walk_stmt(visitor, stmt);
    }

    visitor.leave_function(func);
}

/// Walk a query
pub fn walk_query<V: Visitor>(visitor: &mut V, query: &Query) {
    match query {
        Query::Backtest(backtest) => {
            for (_, expr) in &backtest.parameters {
                walk_expr(visitor, expr);
            }
        }
        Query::Alert(alert) => {
            walk_expr(visitor, &alert.condition);
        }
        Query::With(with_query) => {
            // Walk CTEs
            for cte in &with_query.ctes {
                walk_query(visitor, &cte.query);
            }
            // Walk main query
            walk_query(visitor, &with_query.query);
        }
    }
}

/// Walk a statement
pub fn walk_stmt<V: Visitor>(visitor: &mut V, stmt: &Statement) {
    if !visitor.visit_stmt(stmt) {
        return;
    }

    match stmt {
        Statement::Return(expr, _) => {
            if let Some(e) = expr {
                walk_expr(visitor, e);
            }
        }
        Statement::Break(_) => {}
        Statement::Continue(_) => {}
        Statement::VariableDecl(decl, _) => {
            if let Some(value) = &decl.value {
                walk_expr(visitor, value);
            }
        }
        Statement::Assignment(assign, _) => {
            walk_expr(visitor, &assign.value);
        }
        Statement::Expression(expr, _) => walk_expr(visitor, expr),
        Statement::For(for_loop, _) => {
            match &for_loop.init {
                ForInit::ForIn { iter, .. } => walk_expr(visitor, iter),
                ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    walk_stmt(visitor, init);
                    walk_expr(visitor, condition);
                    walk_expr(visitor, update);
                }
            }
            for stmt in &for_loop.body {
                walk_stmt(visitor, stmt);
            }
        }
        Statement::While(while_loop, _) => {
            walk_expr(visitor, &while_loop.condition);
            for stmt in &while_loop.body {
                walk_stmt(visitor, stmt);
            }
        }
        Statement::If(if_stmt, _) => {
            walk_expr(visitor, &if_stmt.condition);
            for stmt in &if_stmt.then_body {
                walk_stmt(visitor, stmt);
            }
            if let Some(else_body) = &if_stmt.else_body {
                for stmt in else_body {
                    walk_stmt(visitor, stmt);
                }
            }
        }
        Statement::Extend(ext, _) => {
            for method in &ext.methods {
                for stmt in &method.body {
                    walk_stmt(visitor, stmt);
                }
            }
        }
        Statement::RemoveTarget(_) => {}
        Statement::SetParamType { .. }
        | Statement::SetReturnType { .. }
        | Statement::SetReturnExpr { .. } => {}
        Statement::SetParamValue { expression, .. } => {
            walk_expr(visitor, expression);
        }
        Statement::ReplaceModuleExpr { expression, .. } => {
            walk_expr(visitor, expression);
        }
        Statement::ReplaceBodyExpr { expression, .. } => {
            walk_expr(visitor, expression);
        }
        Statement::ReplaceBody { body, .. } => {
            for stmt in body {
                walk_stmt(visitor, stmt);
            }
        }
    }

    visitor.leave_stmt(stmt);
}

/// Walk an expression - ALL VARIANTS HANDLED EXPLICITLY
///
/// Visit order:
/// 1. `visit_expr(expr)` — return `false` to skip entirely
/// 2. `visit_expr_<variant>(expr, span)` — return `false` to skip children
/// 3. Walk children recursively
/// 4. `leave_expr(expr)`
pub fn walk_expr<V: Visitor>(visitor: &mut V, expr: &Expr) {
    if !visitor.visit_expr(expr) {
        return;
    }

    match expr {
        // Leaf nodes (no children)
        Expr::Literal(lit, span) => {
            if visitor.visit_expr_literal(expr, *span) {
                visitor.visit_literal(lit);
                visitor.leave_literal(lit);
            }
        }
        Expr::Identifier(_, span) => {
            visitor.visit_expr_identifier(expr, *span);
        }
        Expr::DataRef(data_ref, span) => {
            if visitor.visit_expr_data_ref(expr, *span) {
                match &data_ref.index {
                    DataIndex::Expression(e) => walk_expr(visitor, e),
                    DataIndex::ExpressionRange(start, end) => {
                        walk_expr(visitor, start);
                        walk_expr(visitor, end);
                    }
                    DataIndex::Single(_) | DataIndex::Range(_, _) => {}
                }
            }
        }
        Expr::DataDateTimeRef(_, span) => {
            visitor.visit_expr_data_datetime_ref(expr, *span);
        }
        Expr::DataRelativeAccess {
            reference,
            index,
            span,
        } => {
            if visitor.visit_expr_data_relative_access(expr, *span) {
                walk_expr(visitor, reference);
                match index {
                    DataIndex::Expression(e) => walk_expr(visitor, e),
                    DataIndex::ExpressionRange(start, end) => {
                        walk_expr(visitor, start);
                        walk_expr(visitor, end);
                    }
                    DataIndex::Single(_) | DataIndex::Range(_, _) => {}
                }
            }
        }
        Expr::PropertyAccess { object, span, .. } => {
            if visitor.visit_expr_property_access(expr, *span) {
                walk_expr(visitor, object);
            }
        }
        Expr::IndexAccess {
            object,
            index,
            end_index,
            span,
        } => {
            if visitor.visit_expr_index_access(expr, *span) {
                walk_expr(visitor, object);
                walk_expr(visitor, index);
                if let Some(end) = end_index {
                    walk_expr(visitor, end);
                }
            }
        }
        Expr::BinaryOp {
            left, right, span, ..
        } => {
            if visitor.visit_expr_binary_op(expr, *span) {
                walk_expr(visitor, left);
                walk_expr(visitor, right);
            }
        }
        Expr::FuzzyComparison {
            left, right, span, ..
        } => {
            if visitor.visit_expr_fuzzy_comparison(expr, *span) {
                walk_expr(visitor, left);
                walk_expr(visitor, right);
            }
        }
        Expr::UnaryOp { operand, span, .. } => {
            if visitor.visit_expr_unary_op(expr, *span) {
                walk_expr(visitor, operand);
            }
        }
        Expr::FunctionCall {
            args,
            named_args,
            span,
            ..
        } => {
            if visitor.visit_expr_function_call(expr, *span) {
                for arg in args {
                    walk_expr(visitor, arg);
                }
                for (_, value) in named_args {
                    walk_expr(visitor, value);
                }
            }
        }
        Expr::QualifiedFunctionCall {
            args,
            named_args,
            span,
            ..
        } => {
            if visitor.visit_expr_function_call(expr, *span) {
                for arg in args {
                    walk_expr(visitor, arg);
                }
                for (_, value) in named_args {
                    walk_expr(visitor, value);
                }
            }
        }
        Expr::EnumConstructor { payload, span, .. } => {
            if visitor.visit_expr_enum_constructor(expr, *span) {
                match payload {
                    EnumConstructorPayload::Unit => {}
                    EnumConstructorPayload::Tuple(values) => {
                        for value in values {
                            walk_expr(visitor, value);
                        }
                    }
                    EnumConstructorPayload::Struct(fields) => {
                        for (_, value) in fields {
                            walk_expr(visitor, value);
                        }
                    }
                }
            }
        }
        Expr::TimeRef(_, span) => {
            visitor.visit_expr_time_ref(expr, *span);
        }
        Expr::DateTime(_, span) => {
            visitor.visit_expr_datetime(expr, *span);
        }
        Expr::PatternRef(_, span) => {
            visitor.visit_expr_pattern_ref(expr, *span);
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            span,
        } => {
            if visitor.visit_expr_conditional(expr, *span) {
                walk_expr(visitor, condition);
                walk_expr(visitor, then_expr);
                if let Some(else_e) = else_expr {
                    walk_expr(visitor, else_e);
                }
            }
        }
        Expr::Object(entries, span) => {
            if visitor.visit_expr_object(expr, *span) {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } => walk_expr(visitor, value),
                        ObjectEntry::Spread(spread_expr) => walk_expr(visitor, spread_expr),
                    }
                }
            }
        }
        Expr::Array(elements, span) => {
            if visitor.visit_expr_array(expr, *span) {
                for elem in elements {
                    walk_expr(visitor, elem);
                }
            }
        }
        Expr::TableRows(rows, _span) => {
            for row in rows {
                for elem in row {
                    walk_expr(visitor, elem);
                }
            }
        }
        Expr::ListComprehension(comp, span) => {
            if visitor.visit_expr_list_comprehension(expr, *span) {
                walk_expr(visitor, &comp.element);
                for clause in &comp.clauses {
                    walk_expr(visitor, &clause.iterable);
                    if let Some(filter) = &clause.filter {
                        walk_expr(visitor, filter);
                    }
                }
            }
        }
        Expr::Block(block, span) => {
            if visitor.visit_expr_block(expr, *span) {
                if visitor.visit_block(block) {
                    for item in &block.items {
                        match item {
                            BlockItem::VariableDecl(decl) => {
                                if let Some(value) = &decl.value {
                                    walk_expr(visitor, value);
                                }
                            }
                            BlockItem::Assignment(assign) => {
                                walk_expr(visitor, &assign.value);
                            }
                            BlockItem::Statement(stmt) => {
                                walk_stmt(visitor, stmt);
                            }
                            BlockItem::Expression(e) => walk_expr(visitor, e),
                        }
                    }
                    visitor.leave_block(block);
                }
            }
        }
        Expr::TypeAssertion {
            expr: inner, span, ..
        } => {
            if visitor.visit_expr_type_assertion(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
        Expr::InstanceOf {
            expr: inner, span, ..
        } => {
            if visitor.visit_expr_instance_of(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
        Expr::FunctionExpr {
            params, body, span, ..
        } => {
            if visitor.visit_expr_function_expr(expr, *span) {
                for param in params {
                    if let Some(default) = &param.default_value {
                        walk_expr(visitor, default);
                    }
                }
                for stmt in body {
                    walk_stmt(visitor, stmt);
                }
            }
        }
        Expr::Duration(_, span) => {
            visitor.visit_expr_duration(expr, *span);
        }
        Expr::Spread(inner, span) => {
            if visitor.visit_expr_spread(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
        Expr::If(if_expr, span) => {
            if visitor.visit_expr_if(expr, *span) {
                walk_expr(visitor, &if_expr.condition);
                walk_expr(visitor, &if_expr.then_branch);
                if let Some(else_branch) = &if_expr.else_branch {
                    walk_expr(visitor, else_branch);
                }
            }
        }
        Expr::While(while_expr, span) => {
            if visitor.visit_expr_while(expr, *span) {
                walk_expr(visitor, &while_expr.condition);
                walk_expr(visitor, &while_expr.body);
            }
        }
        Expr::For(for_expr, span) => {
            if visitor.visit_expr_for(expr, *span) {
                walk_expr(visitor, &for_expr.iterable);
                walk_expr(visitor, &for_expr.body);
            }
        }
        Expr::Loop(loop_expr, span) => {
            if visitor.visit_expr_loop(expr, *span) {
                walk_expr(visitor, &loop_expr.body);
            }
        }
        Expr::Let(let_expr, span) => {
            if visitor.visit_expr_let(expr, *span) {
                if let Some(value) = &let_expr.value {
                    walk_expr(visitor, value);
                }
                walk_expr(visitor, &let_expr.body);
            }
        }
        Expr::Assign(assign, span) => {
            if visitor.visit_expr_assign(expr, *span) {
                walk_expr(visitor, &assign.target);
                walk_expr(visitor, &assign.value);
            }
        }
        Expr::Break(inner, span) => {
            if visitor.visit_expr_break(expr, *span) {
                if let Some(e) = inner {
                    walk_expr(visitor, e);
                }
            }
        }
        Expr::Continue(span) => {
            visitor.visit_expr_continue(expr, *span);
        }
        Expr::Return(inner, span) => {
            if visitor.visit_expr_return(expr, *span) {
                if let Some(e) = inner {
                    walk_expr(visitor, e);
                }
            }
        }
        Expr::MethodCall {
            receiver,
            args,
            named_args,
            span,
            ..
        } => {
            if visitor.visit_expr_method_call(expr, *span) {
                walk_expr(visitor, receiver);
                for arg in args {
                    walk_expr(visitor, arg);
                }
                for (_, value) in named_args {
                    walk_expr(visitor, value);
                }
            }
        }
        Expr::Match(match_expr, span) => {
            if visitor.visit_expr_match(expr, *span) {
                walk_expr(visitor, &match_expr.scrutinee);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        walk_expr(visitor, guard);
                    }
                    walk_expr(visitor, &arm.body);
                }
            }
        }
        Expr::Unit(span) => {
            visitor.visit_expr_unit(expr, *span);
        }
        Expr::Range {
            start, end, span, ..
        } => {
            if visitor.visit_expr_range(expr, *span) {
                if let Some(s) = start {
                    walk_expr(visitor, s);
                }
                if let Some(e) = end {
                    walk_expr(visitor, e);
                }
            }
        }
        Expr::TimeframeContext {
            expr: inner, span, ..
        } => {
            if visitor.visit_expr_timeframe_context(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
        Expr::TryOperator(inner, span) => {
            if visitor.visit_expr_try_operator(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
        Expr::UsingImpl {
            expr: inner, span, ..
        } => {
            if visitor.visit_expr_using_impl(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
        Expr::SimulationCall { params, span, .. } => {
            if visitor.visit_expr_simulation_call(expr, *span) {
                for (_, value) in params {
                    walk_expr(visitor, value);
                }
            }
        }
        Expr::WindowExpr(window_expr, span) => {
            if visitor.visit_expr_window_expr(expr, *span) {
                // Walk function argument expressions
                match &window_expr.function {
                    WindowFunction::Lead { expr, default, .. }
                    | WindowFunction::Lag { expr, default, .. } => {
                        walk_expr(visitor, expr);
                        if let Some(d) = default {
                            walk_expr(visitor, d);
                        }
                    }
                    WindowFunction::FirstValue(e)
                    | WindowFunction::LastValue(e)
                    | WindowFunction::Sum(e)
                    | WindowFunction::Avg(e)
                    | WindowFunction::Min(e)
                    | WindowFunction::Max(e) => {
                        walk_expr(visitor, e);
                    }
                    WindowFunction::NthValue(e, _) => {
                        walk_expr(visitor, e);
                    }
                    WindowFunction::Count(opt_e) => {
                        if let Some(e) = opt_e {
                            walk_expr(visitor, e);
                        }
                    }
                    WindowFunction::RowNumber
                    | WindowFunction::Rank
                    | WindowFunction::DenseRank
                    | WindowFunction::Ntile(_) => {}
                }
                // Walk partition_by expressions
                for e in &window_expr.over.partition_by {
                    walk_expr(visitor, e);
                }
                // Walk order_by expressions
                if let Some(order_by) = &window_expr.over.order_by {
                    for (e, _) in &order_by.columns {
                        walk_expr(visitor, e);
                    }
                }
            }
        }
        Expr::FromQuery(from_query, span) => {
            if visitor.visit_expr_from_query(expr, *span) {
                // Walk source expression
                walk_expr(visitor, &from_query.source);
                // Walk each clause
                for clause in &from_query.clauses {
                    match clause {
                        QueryClause::Where(pred) => {
                            walk_expr(visitor, pred);
                        }
                        QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                walk_expr(visitor, &spec.key);
                            }
                        }
                        QueryClause::GroupBy { element, key, .. } => {
                            walk_expr(visitor, element);
                            walk_expr(visitor, key);
                        }
                        QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            walk_expr(visitor, source);
                            walk_expr(visitor, left_key);
                            walk_expr(visitor, right_key);
                        }
                        QueryClause::Let { value, .. } => {
                            walk_expr(visitor, value);
                        }
                    }
                }
                // Walk select expression
                walk_expr(visitor, &from_query.select);
            }
        }
        Expr::StructLiteral { fields, span, .. } => {
            if visitor.visit_expr_struct_literal(expr, *span) {
                for (_, value_expr) in fields {
                    walk_expr(visitor, value_expr);
                }
            }
        }
        Expr::Await(inner, span) => {
            if visitor.visit_expr_await(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
        Expr::Join(join_expr, span) => {
            if visitor.visit_expr_join(expr, *span) {
                for branch in &join_expr.branches {
                    walk_expr(visitor, &branch.expr);
                }
            }
        }
        Expr::Annotated { target, span, .. } => {
            if visitor.visit_expr_annotated(expr, *span) {
                walk_expr(visitor, target);
            }
        }
        Expr::AsyncLet(async_let, span) => {
            if visitor.visit_expr_async_let(expr, *span) {
                walk_expr(visitor, &async_let.expr);
            }
        }
        Expr::AsyncScope(inner, span) => {
            if visitor.visit_expr_async_scope(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
        Expr::Comptime(stmts, span) => {
            if visitor.visit_expr_comptime(expr, *span) {
                for stmt in stmts {
                    walk_stmt(visitor, stmt);
                }
            }
        }
        Expr::ComptimeFor(cf, span) => {
            if visitor.visit_expr_comptime_for(expr, *span) {
                walk_expr(visitor, &cf.iterable);
                for stmt in &cf.body {
                    walk_stmt(visitor, stmt);
                }
            }
        }
        Expr::Reference {
            expr: inner, span, ..
        } => {
            if visitor.visit_expr_reference(expr, *span) {
                walk_expr(visitor, inner);
            }
        }
    }

    visitor.leave_expr(expr);
}

/// Walk a test statement
fn walk_test_statement<V: Visitor>(visitor: &mut V, test_stmt: &TestStatement) {
    match test_stmt {
        TestStatement::Statement(stmt) => walk_stmt(visitor, stmt),
        TestStatement::Assert(assert) => {
            walk_expr(visitor, &assert.condition);
        }
        TestStatement::Expect(expect) => {
            walk_expr(visitor, &expect.actual);
            match &expect.matcher {
                ExpectationMatcher::ToBe(e) => walk_expr(visitor, e),
                ExpectationMatcher::ToEqual(e) => walk_expr(visitor, e),
                ExpectationMatcher::ToBeCloseTo { expected, .. } => walk_expr(visitor, expected),
                ExpectationMatcher::ToBeGreaterThan(e) => walk_expr(visitor, e),
                ExpectationMatcher::ToBeLessThan(e) => walk_expr(visitor, e),
                ExpectationMatcher::ToContain(e) => walk_expr(visitor, e),
                ExpectationMatcher::ToBeTruthy => {}
                ExpectationMatcher::ToBeFalsy => {}
                ExpectationMatcher::ToThrow(_) => {}
                ExpectationMatcher::ToMatchPattern { .. } => {}
            }
        }
        TestStatement::Should(should) => {
            walk_expr(visitor, &should.subject);
            match &should.matcher {
                ShouldMatcher::Be(e) => walk_expr(visitor, e),
                ShouldMatcher::Equal(e) => walk_expr(visitor, e),
                ShouldMatcher::Contain(e) => walk_expr(visitor, e),
                ShouldMatcher::Match(_) => {}
                ShouldMatcher::BeCloseTo { expected, .. } => walk_expr(visitor, expected),
            }
        }
        TestStatement::Fixture(fixture) => match fixture {
            TestFixture::WithData { data, body } => {
                walk_expr(visitor, data);
                for stmt in body {
                    walk_stmt(visitor, stmt);
                }
            }
            TestFixture::WithMock {
                mock_value, body, ..
            } => {
                if let Some(value) = mock_value {
                    walk_expr(visitor, value);
                }
                for stmt in body {
                    walk_stmt(visitor, stmt);
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simple visitor that counts expressions
    struct ExprCounter {
        count: usize,
    }

    impl Visitor for ExprCounter {
        fn visit_expr(&mut self, _expr: &Expr) -> bool {
            self.count += 1;
            true
        }
    }

    #[test]
    fn test_visitor_counts_expressions() {
        let program = Program {
            items: vec![Item::Expression(
                Expr::BinaryOp {
                    left: Box::new(Expr::Identifier("x".to_string(), Span::DUMMY)),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Literal(Literal::Number(1.0), Span::DUMMY)),
                    span: Span::DUMMY,
                },
                Span::DUMMY,
            )],
            docs: shape_ast::ast::ProgramDocs::default(),
        };

        let mut counter = ExprCounter { count: 0 };
        walk_program(&mut counter, &program);

        // Should count: BinaryOp, Identifier, Literal = 3
        assert_eq!(counter.count, 3);
    }

    #[test]
    fn test_visitor_handles_try_operator() {
        let program = Program {
            items: vec![Item::Expression(
                Expr::TryOperator(
                    Box::new(Expr::FunctionCall {
                        name: "some_function".to_string(),
                        args: vec![Expr::Literal(
                            Literal::String("arg".to_string()),
                            Span::DUMMY,
                        )],
                        named_args: vec![],
                        span: Span::DUMMY,
                    }),
                    Span::DUMMY,
                ),
                Span::DUMMY,
            )],
            docs: shape_ast::ast::ProgramDocs::default(),
        };

        let mut counter = ExprCounter { count: 0 };
        walk_program(&mut counter, &program);

        // Should count: TryOperator, FunctionCall, Literal = 3
        assert_eq!(counter.count, 3);
    }

    /// Test that per-variant visitor methods work
    struct IdentifierCollector {
        names: Vec<String>,
    }

    impl Visitor for IdentifierCollector {
        fn visit_expr_identifier(&mut self, expr: &Expr, _span: Span) -> bool {
            if let Expr::Identifier(name, _) = expr {
                self.names.push(name.clone());
            }
            true
        }
    }

    #[test]
    fn test_per_variant_visitor_identifier() {
        let program = Program {
            items: vec![Item::Expression(
                Expr::BinaryOp {
                    left: Box::new(Expr::Identifier("x".to_string(), Span::DUMMY)),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Identifier("y".to_string(), Span::DUMMY)),
                    span: Span::DUMMY,
                },
                Span::DUMMY,
            )],
            docs: shape_ast::ast::ProgramDocs::default(),
        };

        let mut collector = IdentifierCollector { names: vec![] };
        walk_program(&mut collector, &program);

        assert_eq!(collector.names, vec!["x", "y"]);
    }

    /// Test that per-variant method can skip children
    struct SkippingVisitor {
        count: usize,
    }

    impl Visitor for SkippingVisitor {
        fn visit_expr(&mut self, _expr: &Expr) -> bool {
            self.count += 1;
            true
        }
        // Skip children of BinaryOp
        fn visit_expr_binary_op(&mut self, _expr: &Expr, _span: Span) -> bool {
            false
        }
    }

    #[test]
    fn test_per_variant_skip_children() {
        let program = Program {
            items: vec![Item::Expression(
                Expr::BinaryOp {
                    left: Box::new(Expr::Identifier("x".to_string(), Span::DUMMY)),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Literal(Literal::Number(1.0), Span::DUMMY)),
                    span: Span::DUMMY,
                },
                Span::DUMMY,
            )],
            docs: shape_ast::ast::ProgramDocs::default(),
        };

        let mut v = SkippingVisitor { count: 0 };
        walk_program(&mut v, &program);

        // Only BinaryOp counted, children skipped
        assert_eq!(v.count, 1);
    }

    /// Test combined coarse + per-variant
    struct MatchCollector {
        match_count: usize,
        total_expr_count: usize,
    }

    impl Visitor for MatchCollector {
        fn visit_expr(&mut self, _expr: &Expr) -> bool {
            self.total_expr_count += 1;
            true
        }
        fn visit_expr_match(&mut self, _expr: &Expr, _span: Span) -> bool {
            self.match_count += 1;
            true
        }
    }

    #[test]
    fn test_coarse_and_per_variant_combined() {
        let program = Program {
            items: vec![Item::Expression(
                Expr::BinaryOp {
                    left: Box::new(Expr::Identifier("x".to_string(), Span::DUMMY)),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Identifier("y".to_string(), Span::DUMMY)),
                    span: Span::DUMMY,
                },
                Span::DUMMY,
            )],
            docs: shape_ast::ast::ProgramDocs::default(),
        };

        let mut mc = MatchCollector {
            match_count: 0,
            total_expr_count: 0,
        };
        walk_program(&mut mc, &program);

        assert_eq!(mc.total_expr_count, 3); // BinaryOp + x + y
        assert_eq!(mc.match_count, 0); // No Match expressions
    }
}
