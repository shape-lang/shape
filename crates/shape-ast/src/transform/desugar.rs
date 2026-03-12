//! AST desugaring pass
//!
//! Transforms high-level syntax into equivalent lower-level constructs.
//! Currently handles LINQ-style from queries → method chains.

use crate::ast::{
    DestructurePattern, Expr, FromQueryExpr, FunctionParameter, Item, Literal, ObjectEntry,
    Program, QueryClause, Span, Statement,
};

/// Desugar all high-level syntax in a program.
/// This should be called before compilation.
pub fn desugar_program(program: &mut Program) {
    for item in &mut program.items {
        desugar_item(item);
    }
}

fn desugar_item(item: &mut Item) {
    match item {
        Item::Function(func, _) => {
            for stmt in &mut func.body {
                desugar_statement(stmt);
            }
        }
        Item::VariableDecl(decl, _) => {
            if let Some(value) = &mut decl.value {
                desugar_expr(value);
            }
        }
        Item::Assignment(assign, _) => {
            desugar_expr(&mut assign.value);
        }
        Item::Expression(expr, _) => {
            desugar_expr(expr);
        }
        Item::Statement(stmt, _) => {
            desugar_statement(stmt);
        }
        Item::Export(export, _) => match &mut export.item {
            crate::ast::ExportItem::Function(func) => {
                for stmt in &mut func.body {
                    desugar_statement(stmt);
                }
            }
            crate::ast::ExportItem::TypeAlias(_)
            | crate::ast::ExportItem::Named(_)
            | crate::ast::ExportItem::Enum(_)
            | crate::ast::ExportItem::Struct(_)
            | crate::ast::ExportItem::Interface(_)
            | crate::ast::ExportItem::Trait(_)
            | crate::ast::ExportItem::BuiltinFunction(_)
            | crate::ast::ExportItem::BuiltinType(_)
            | crate::ast::ExportItem::Annotation(_)
            | crate::ast::ExportItem::ForeignFunction(_) => {}
        },
        Item::Module(module, _) => {
            for inner in &mut module.items {
                desugar_item(inner);
            }
        }
        Item::Extend(extend, _) => {
            for method in &mut extend.methods {
                for stmt in &mut method.body {
                    desugar_statement(stmt);
                }
            }
        }
        Item::Stream(stream, _) => {
            for decl in &mut stream.state {
                if let Some(value) = &mut decl.value {
                    desugar_expr(value);
                }
            }
            if let Some(stmts) = &mut stream.on_connect {
                for stmt in stmts {
                    desugar_statement(stmt);
                }
            }
            if let Some(stmts) = &mut stream.on_disconnect {
                for stmt in stmts {
                    desugar_statement(stmt);
                }
            }
            if let Some(on_event) = &mut stream.on_event {
                for stmt in &mut on_event.body {
                    desugar_statement(stmt);
                }
            }
            if let Some(on_window) = &mut stream.on_window {
                for stmt in &mut on_window.body {
                    desugar_statement(stmt);
                }
            }
            if let Some(on_error) = &mut stream.on_error {
                for stmt in &mut on_error.body {
                    desugar_statement(stmt);
                }
            }
        }
        // Other items don't need desugaring
        _ => {}
    }
}

fn desugar_statement(stmt: &mut Statement) {
    match stmt {
        Statement::Return(Some(expr), _) => desugar_expr(expr),
        Statement::VariableDecl(decl, _) => {
            if let Some(value) = &mut decl.value {
                desugar_expr(value);
            }
        }
        Statement::Assignment(assign, _) => desugar_expr(&mut assign.value),
        Statement::Expression(expr, _) => desugar_expr(expr),
        Statement::For(for_loop, _) => {
            match &mut for_loop.init {
                crate::ast::ForInit::ForIn { iter, .. } => desugar_expr(iter),
                crate::ast::ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    desugar_statement(init);
                    desugar_expr(condition);
                    desugar_expr(update);
                }
            }
            for s in &mut for_loop.body {
                desugar_statement(s);
            }
        }
        Statement::While(while_loop, _) => {
            desugar_expr(&mut while_loop.condition);
            for s in &mut while_loop.body {
                desugar_statement(s);
            }
        }
        Statement::If(if_stmt, _) => {
            desugar_expr(&mut if_stmt.condition);
            for s in &mut if_stmt.then_body {
                desugar_statement(s);
            }
            if let Some(else_body) = &mut if_stmt.else_body {
                for s in else_body {
                    desugar_statement(s);
                }
            }
        }
        _ => {}
    }
}

fn desugar_expr(expr: &mut Expr) {
    // First recursively desugar nested expressions
    match expr {
        Expr::FromQuery(from_query, span) => {
            // Desugar nested expressions in the query first
            desugar_expr(&mut from_query.source);
            for clause in &mut from_query.clauses {
                match clause {
                    QueryClause::Where(pred) => desugar_expr(pred),
                    QueryClause::OrderBy(specs) => {
                        for spec in specs {
                            desugar_expr(&mut spec.key);
                        }
                    }
                    QueryClause::GroupBy { element, key, .. } => {
                        desugar_expr(element);
                        desugar_expr(key);
                    }
                    QueryClause::Join {
                        source,
                        left_key,
                        right_key,
                        ..
                    } => {
                        desugar_expr(source);
                        desugar_expr(left_key);
                        desugar_expr(right_key);
                    }
                    QueryClause::Let { value, .. } => desugar_expr(value),
                }
            }
            desugar_expr(&mut from_query.select);

            // Now desugar the from query to method chains
            let desugared = desugar_from_query(from_query, *span);
            *expr = desugared;
        }
        // Recursively handle all other expression types
        Expr::PropertyAccess { object, .. } => desugar_expr(object),
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            desugar_expr(object);
            desugar_expr(index);
            if let Some(end) = end_index {
                desugar_expr(end);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            desugar_expr(left);
            desugar_expr(right);
        }
        Expr::FuzzyComparison { left, right, .. } => {
            desugar_expr(left);
            desugar_expr(right);
        }
        Expr::UnaryOp { operand, .. } => desugar_expr(operand),
        Expr::FunctionCall {
            args, named_args, ..
        } => {
            for arg in args {
                desugar_expr(arg);
            }
            for (_, val) in named_args {
                desugar_expr(val);
            }
        }
        Expr::QualifiedFunctionCall {
            args, named_args, ..
        } => {
            for arg in args {
                desugar_expr(arg);
            }
            for (_, val) in named_args {
                desugar_expr(val);
            }
        }
        Expr::MethodCall {
            receiver,
            args,
            named_args,
            ..
        } => {
            desugar_expr(receiver);
            for arg in args {
                desugar_expr(arg);
            }
            for (_, val) in named_args {
                desugar_expr(val);
            }
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            desugar_expr(condition);
            desugar_expr(then_expr);
            if let Some(e) = else_expr {
                desugar_expr(e);
            }
        }
        Expr::Object(entries, _) => {
            for entry in entries {
                match entry {
                    ObjectEntry::Field { value, .. } => desugar_expr(value),
                    ObjectEntry::Spread(e) => desugar_expr(e),
                }
            }
        }
        Expr::Array(elements, _) => {
            for elem in elements {
                desugar_expr(elem);
            }
        }
        Expr::ListComprehension(comp, _) => {
            desugar_expr(&mut comp.element);
            for clause in &mut comp.clauses {
                desugar_expr(&mut clause.iterable);
                if let Some(filter) = &mut clause.filter {
                    desugar_expr(filter);
                }
            }
        }
        Expr::Block(block, _) => {
            for item in &mut block.items {
                match item {
                    crate::ast::BlockItem::VariableDecl(decl) => {
                        if let Some(value) = &mut decl.value {
                            desugar_expr(value);
                        }
                    }
                    crate::ast::BlockItem::Assignment(assign) => {
                        desugar_expr(&mut assign.value);
                    }
                    crate::ast::BlockItem::Statement(stmt) => {
                        desugar_statement(stmt);
                    }
                    crate::ast::BlockItem::Expression(e) => {
                        desugar_expr(e);
                    }
                }
            }
        }
        Expr::TypeAssertion { expr: inner, .. } => desugar_expr(inner),
        Expr::InstanceOf { expr: inner, .. } => desugar_expr(inner),
        Expr::FunctionExpr { body, .. } => {
            for stmt in body {
                desugar_statement(stmt);
            }
        }
        Expr::Spread(inner, _) => desugar_expr(inner),
        Expr::If(if_expr, _) => {
            desugar_expr(&mut if_expr.condition);
            desugar_expr(&mut if_expr.then_branch);
            if let Some(e) = &mut if_expr.else_branch {
                desugar_expr(e);
            }
        }
        Expr::While(while_expr, _) => {
            desugar_expr(&mut while_expr.condition);
            desugar_expr(&mut while_expr.body);
        }
        Expr::For(for_expr, _) => {
            desugar_expr(&mut for_expr.iterable);
            desugar_expr(&mut for_expr.body);
        }
        Expr::Loop(loop_expr, _) => {
            desugar_expr(&mut loop_expr.body);
        }
        Expr::Let(let_expr, _) => {
            if let Some(val) = &mut let_expr.value {
                desugar_expr(val);
            }
            desugar_expr(&mut let_expr.body);
        }
        Expr::Assign(assign, _) => {
            desugar_expr(&mut assign.target);
            desugar_expr(&mut assign.value);
        }
        Expr::Break(Some(e), _) => desugar_expr(e),
        Expr::Return(Some(e), _) => desugar_expr(e),
        Expr::Match(match_expr, _) => {
            desugar_expr(&mut match_expr.scrutinee);
            for arm in &mut match_expr.arms {
                if let Some(guard) = &mut arm.guard {
                    desugar_expr(guard);
                }
                desugar_expr(&mut arm.body);
            }
        }
        Expr::Range { start, end, .. } => {
            if let Some(s) = start {
                desugar_expr(s);
            }
            if let Some(e) = end {
                desugar_expr(e);
            }
        }
        Expr::TimeframeContext { expr: inner, .. } => desugar_expr(inner),
        Expr::TryOperator(inner, _) => desugar_expr(inner),
        Expr::UsingImpl { expr: inner, .. } => desugar_expr(inner),
        Expr::Await(inner, _) => desugar_expr(inner),
        Expr::EnumConstructor { payload, .. } => match payload {
            crate::ast::EnumConstructorPayload::Tuple(args) => {
                for arg in args {
                    desugar_expr(arg);
                }
            }
            crate::ast::EnumConstructorPayload::Struct(fields) => {
                for (_, val) in fields {
                    desugar_expr(val);
                }
            }
            crate::ast::EnumConstructorPayload::Unit => {}
        },
        Expr::SimulationCall { params, .. } => {
            for (_, val) in params {
                desugar_expr(val);
            }
        }
        Expr::WindowExpr(window_expr, _) => {
            // Desugar window function arguments
            match &mut window_expr.function {
                crate::ast::WindowFunction::Lead { expr, default, .. }
                | crate::ast::WindowFunction::Lag { expr, default, .. } => {
                    desugar_expr(expr);
                    if let Some(d) = default {
                        desugar_expr(d);
                    }
                }
                crate::ast::WindowFunction::FirstValue(e)
                | crate::ast::WindowFunction::LastValue(e)
                | crate::ast::WindowFunction::Sum(e)
                | crate::ast::WindowFunction::Avg(e)
                | crate::ast::WindowFunction::Min(e)
                | crate::ast::WindowFunction::Max(e) => {
                    desugar_expr(e);
                }
                crate::ast::WindowFunction::NthValue(e, _) => {
                    desugar_expr(e);
                }
                crate::ast::WindowFunction::Count(Some(e)) => {
                    desugar_expr(e);
                }
                _ => {}
            }
            // Desugar partition_by and order_by expressions
            for e in &mut window_expr.over.partition_by {
                desugar_expr(e);
            }
            if let Some(order_by) = &mut window_expr.over.order_by {
                for (e, _) in &mut order_by.columns {
                    desugar_expr(e);
                }
            }
        }
        Expr::DataRef(data_ref, _) => match &mut data_ref.index {
            crate::ast::DataIndex::Expression(e) => desugar_expr(e),
            crate::ast::DataIndex::ExpressionRange(start, end) => {
                desugar_expr(start);
                desugar_expr(end);
            }
            _ => {}
        },
        Expr::DataRelativeAccess {
            reference, index, ..
        } => {
            desugar_expr(reference);
            match index {
                crate::ast::DataIndex::Expression(e) => desugar_expr(e),
                crate::ast::DataIndex::ExpressionRange(start, end) => {
                    desugar_expr(start);
                    desugar_expr(end);
                }
                _ => {}
            }
        }
        Expr::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                desugar_expr(value);
            }
        }
        // Leaf nodes - no recursion needed
        Expr::Literal(_, _)
        | Expr::Identifier(_, _)
        | Expr::DataDateTimeRef(_, _)
        | Expr::TimeRef(_, _)
        | Expr::DateTime(_, _)
        | Expr::PatternRef(_, _)
        | Expr::Duration(_, _)
        | Expr::Unit(_)
        | Expr::Continue(_)
        | Expr::Break(None, _)
        | Expr::Return(None, _)
        | Expr::Join(_, _) => {}
        Expr::Annotated { target, .. } => {
            desugar_expr(target);
        }
        Expr::AsyncLet(async_let, _) => {
            desugar_expr(&mut async_let.expr);
        }
        Expr::AsyncScope(inner, _) => {
            desugar_expr(inner);
        }
        Expr::Comptime(stmts, _) => {
            for stmt in stmts {
                desugar_statement(stmt);
            }
        }
        Expr::ComptimeFor(cf, _) => {
            desugar_expr(&mut cf.iterable);
            for stmt in &mut cf.body {
                desugar_statement(stmt);
            }
        }
        Expr::Reference { expr: inner, .. } => desugar_expr(inner),
        Expr::TableRows(rows, _) => {
            for row in rows {
                for elem in row {
                    desugar_expr(elem);
                }
            }
        }
    }
}

/// Desugar a from query expression into method chain calls.
///
/// Example transformation:
/// ```text
/// from t in trades where t.amount > 1000 order by t.date desc select t.price
/// ```
/// becomes:
/// ```text
/// trades.filter(|t| t.amount > 1000).orderBy(|t| t.date, "desc").map(|t| t.price)
/// ```
fn desugar_from_query(from_query: &FromQueryExpr, span: Span) -> Expr {
    let current_var = &from_query.variable;
    let mut result = (*from_query.source).clone();

    // Track the current iteration variable (changes after group by into)
    let mut iter_var = current_var.clone();

    for clause in &from_query.clauses {
        match clause {
            QueryClause::Where(pred) => {
                // source.filter(|var| predicate)
                // Uses "filter" (Queryable interface) instead of "where" for
                // consistency with trait-based dispatch (DbTable, DataTable, Array).
                result = method_call(
                    result,
                    "filter",
                    vec![make_lambda(&iter_var, pred, span)],
                    span,
                );
            }
            QueryClause::OrderBy(specs) => {
                // source.orderBy(|var| key, "dir").thenBy(|var| key2, "dir2")...
                for (i, spec) in specs.iter().enumerate() {
                    let method = if i == 0 { "orderBy" } else { "thenBy" };
                    let dir = if spec.descending { "desc" } else { "asc" };
                    result = method_call(
                        result,
                        method,
                        vec![
                            make_lambda(&iter_var, &spec.key, span),
                            string_lit(dir, span),
                        ],
                        span,
                    );
                }
            }
            QueryClause::GroupBy { key, into_var, .. } => {
                // source.groupBy(|var| key)
                result = method_call(
                    result,
                    "groupBy",
                    vec![make_lambda(&iter_var, key, span)],
                    span,
                );
                // After grouping, iteration variable changes to the group
                if let Some(var) = into_var {
                    iter_var = var.clone();
                }
            }
            QueryClause::Join {
                variable,
                source: join_source,
                left_key,
                right_key,
                into_var,
            } => {
                // source.innerJoin(other, |left| leftKey, |right| rightKey, |left, right| result)
                // or with into: source.leftJoin(...)
                let method = if into_var.is_some() {
                    "leftJoin"
                } else {
                    "innerJoin"
                };

                // Build result selector that creates object with both variables
                let result_selector = make_binary_lambda(
                    &iter_var,
                    variable,
                    &make_object(
                        vec![
                            (iter_var.clone(), Expr::Identifier(iter_var.clone(), span)),
                            (variable.clone(), Expr::Identifier(variable.clone(), span)),
                        ],
                        span,
                    ),
                    span,
                );

                result = method_call(
                    result,
                    method,
                    vec![
                        (**join_source).clone(),
                        make_lambda(&iter_var, left_key, span),
                        make_lambda(variable, right_key, span),
                        result_selector,
                    ],
                    span,
                );
            }
            QueryClause::Let {
                variable: let_var,
                value,
            } => {
                // Transform to intermediate select that adds the binding
                // source.select(|var| { __orig: var, let_var: value })
                let intermediate = make_object(
                    vec![
                        (
                            "__orig".to_string(),
                            Expr::Identifier(iter_var.clone(), span),
                        ),
                        (let_var.clone(), (**value).clone()),
                    ],
                    span,
                );
                result = method_call(
                    result,
                    "select",
                    vec![make_lambda(&iter_var, &intermediate, span)],
                    span,
                );
                // Update iter_var to access __orig for the original variable
                // This is a simplification; in a full implementation we'd rewrite references
                iter_var = "__x".to_string();
            }
        }
    }

    // Final select → desugars to .map() for typed table algebra
    result = method_call(
        result,
        "map",
        vec![make_lambda(&iter_var, &from_query.select, span)],
        span,
    );

    result
}

/// Create a method call expression
fn method_call(receiver: Expr, method: &str, args: Vec<Expr>, span: Span) -> Expr {
    Expr::MethodCall {
        receiver: Box::new(receiver),
        method: method.to_string(),
        args,
        named_args: vec![],
        span,
    }
}

/// Create a lambda expression: |param| body
fn make_lambda(param: &str, body: &Expr, span: Span) -> Expr {
    Expr::FunctionExpr {
        params: vec![FunctionParameter {
            pattern: DestructurePattern::Identifier(param.to_string(), span),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: None,
            default_value: None,
        }],
        return_type: None,
        body: vec![Statement::Return(Some(body.clone()), span)],
        span,
    }
}

/// Create a binary lambda expression: |param1, param2| body
fn make_binary_lambda(param1: &str, param2: &str, body: &Expr, span: Span) -> Expr {
    Expr::FunctionExpr {
        params: vec![
            FunctionParameter {
                pattern: DestructurePattern::Identifier(param1.to_string(), span),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            },
            FunctionParameter {
                pattern: DestructurePattern::Identifier(param2.to_string(), span),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            },
        ],
        return_type: None,
        body: vec![Statement::Return(Some(body.clone()), span)],
        span,
    }
}

/// Create a string literal expression
fn string_lit(s: &str, span: Span) -> Expr {
    Expr::Literal(Literal::String(s.to_string()), span)
}

/// Create an object literal expression
fn make_object(fields: Vec<(String, Expr)>, span: Span) -> Expr {
    let entries = fields
        .into_iter()
        .map(|(key, value)| ObjectEntry::Field {
            key,
            value,
            type_annotation: None,
        })
        .collect();
    Expr::Object(entries, span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_program;

    /// Helper to extract expression from a program item
    fn get_expr(item: &Item) -> Option<&Expr> {
        match item {
            Item::Expression(expr, _) => Some(expr),
            Item::Statement(Statement::Expression(expr, _), _) => Some(expr),
            _ => None,
        }
    }

    #[test]
    fn test_basic_from_query_desugaring() {
        let code = "from x in [1, 2, 3] where x > 1 select x * 2";
        let mut program = parse_program(code).unwrap();
        desugar_program(&mut program);

        // The desugared form should be a method chain
        if let Some(expr) = get_expr(&program.items[0]) {
            // Should be: [1,2,3].filter(|x| x > 1).map(|x| x * 2)
            assert!(matches!(expr, Expr::MethodCall { method, .. } if method == "map"));
        } else {
            panic!("Expected expression item");
        }
    }

    #[test]
    fn test_order_by_desugaring() {
        let code = "from x in arr order by x.value desc select x";
        let mut program = parse_program(code).unwrap();
        desugar_program(&mut program);

        if let Some(expr) = get_expr(&program.items[0]) {
            // Final call should be select
            assert!(matches!(expr, Expr::MethodCall { method, .. } if method == "map"));
        } else {
            panic!("Expected expression item");
        }
    }
}
