//! Type inference helpers for completions

use shape_ast::ast::{Expr, Item, Program, Statement};
use std::collections::HashMap;
use std::path::Path;

use super::methods::extract_generic_arg;
use super::types::resolve_property_type;

// Re-export from the canonical location in type_inference
pub use crate::type_inference::type_to_string;

pub fn infer_types(program: &Program) -> Option<HashMap<String, String>> {
    let types = crate::type_inference::infer_program_types(program);
    if types.is_empty() { None } else { Some(types) }
}

pub fn infer_types_with_context(
    program: &Program,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<HashMap<String, String>> {
    let types = crate::type_inference::infer_program_types_with_context(
        program,
        current_file,
        workspace_root,
        current_source,
    );
    if types.is_empty() { None } else { Some(types) }
}

pub fn infer_param_types(
    program: &Program,
    type_context: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut types = HashMap::new();
    for item in &program.items {
        collect_param_types_from_item(item, type_context, &mut types);
    }
    types
}

fn collect_param_types_from_item(
    item: &Item,
    type_context: &HashMap<String, String>,
    out: &mut HashMap<String, String>,
) {
    match item {
        Item::Statement(statement, _) => {
            collect_param_types_from_statement(statement, type_context, out);
        }
        Item::Function(func, _) => {
            for stmt in &func.body {
                collect_param_types_from_statement(stmt, type_context, out);
            }
        }
        Item::VariableDecl(var_decl, _) => {
            if let Some(value) = &var_decl.value {
                collect_param_types_from_expr(value, type_context, out);
            }
        }
        _ => {}
    }
}

fn collect_param_types_from_statement(
    statement: &Statement,
    type_context: &HashMap<String, String>,
    out: &mut HashMap<String, String>,
) {
    match statement {
        Statement::VariableDecl(var_decl, _) => {
            if let Some(value) = &var_decl.value {
                collect_param_types_from_expr(value, type_context, out);
            }
        }
        Statement::Assignment(assign, _) => {
            collect_param_types_from_expr(&assign.value, type_context, out);
        }
        Statement::Expression(expr, _) => {
            collect_param_types_from_expr(expr, type_context, out);
        }
        Statement::Return(Some(expr), _) => {
            collect_param_types_from_expr(expr, type_context, out);
        }
        Statement::For(for_loop, _) => {
            match &for_loop.init {
                shape_ast::ast::ForInit::ForIn { iter, .. } => {
                    collect_param_types_from_expr(iter, type_context, out);
                }
                shape_ast::ast::ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    collect_param_types_from_statement(init, type_context, out);
                    collect_param_types_from_expr(condition, type_context, out);
                    collect_param_types_from_expr(update, type_context, out);
                }
            }
            for stmt in &for_loop.body {
                collect_param_types_from_statement(stmt, type_context, out);
            }
        }
        Statement::While(while_loop, _) => {
            collect_param_types_from_expr(&while_loop.condition, type_context, out);
            for stmt in &while_loop.body {
                collect_param_types_from_statement(stmt, type_context, out);
            }
        }
        Statement::If(if_stmt, _) => {
            collect_param_types_from_expr(&if_stmt.condition, type_context, out);
            for stmt in &if_stmt.then_body {
                collect_param_types_from_statement(stmt, type_context, out);
            }
            if let Some(else_body) = &if_stmt.else_body {
                for stmt in else_body {
                    collect_param_types_from_statement(stmt, type_context, out);
                }
            }
        }
        _ => {}
    }
}

fn collect_param_types_from_expr(
    expr: &Expr,
    type_context: &HashMap<String, String>,
    out: &mut HashMap<String, String>,
) {
    match expr {
        Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            // List of methods that take lambda parameters
            const LAMBDA_METHODS: &[&str] = &[
                "filter",
                "filter_by",
                "map",
                "forEach",
                "reduce",
                "find",
                "some",
                "every",
            ];

            // Check if this method takes a lambda parameter
            if LAMBDA_METHODS.contains(&method.as_str()) && !args.is_empty() {
                // Process first argument if it's a lambda function
                if let Expr::FunctionExpr { params, .. } = &args[0] {
                    // Infer type for each lambda parameter
                    for (param_index, param) in params.iter().enumerate() {
                        if let Some(param_type) =
                            infer_lambda_param_type(receiver, method, param_index, type_context)
                        {
                            for name in param.get_identifiers() {
                                out.insert(name, param_type.clone());
                            }
                        }
                    }
                }
            }

            collect_param_types_from_expr(receiver, type_context, out);
            for arg in args {
                collect_param_types_from_expr(arg, type_context, out);
            }
        }
        Expr::FunctionCall { args, .. } => {
            for arg in args {
                collect_param_types_from_expr(arg, type_context, out);
            }
        }
        Expr::PropertyAccess { object, .. } => {
            collect_param_types_from_expr(object, type_context, out);
        }
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            collect_param_types_from_expr(object, type_context, out);
            collect_param_types_from_expr(index, type_context, out);
            if let Some(end) = end_index {
                collect_param_types_from_expr(end, type_context, out);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_param_types_from_expr(left, type_context, out);
            collect_param_types_from_expr(right, type_context, out);
        }
        Expr::UnaryOp { operand, .. } => {
            collect_param_types_from_expr(operand, type_context, out);
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            collect_param_types_from_expr(condition, type_context, out);
            collect_param_types_from_expr(then_expr, type_context, out);
            if let Some(else_expr) = else_expr {
                collect_param_types_from_expr(else_expr, type_context, out);
            }
        }
        Expr::Object(entries, _) => {
            use shape_ast::ast::ObjectEntry;
            for entry in entries {
                match entry {
                    ObjectEntry::Field { value, .. } => {
                        collect_param_types_from_expr(value, type_context, out)
                    }
                    ObjectEntry::Spread(spread_expr) => {
                        collect_param_types_from_expr(spread_expr, type_context, out)
                    }
                }
            }
        }
        Expr::Array(elements, _) => {
            for element in elements {
                collect_param_types_from_expr(element, type_context, out);
            }
        }
        Expr::ListComprehension(comp, _) => {
            collect_param_types_from_expr(&comp.element, type_context, out);
            for clause in &comp.clauses {
                collect_param_types_from_expr(&clause.iterable, type_context, out);
                if let Some(filter) = &clause.filter {
                    collect_param_types_from_expr(filter, type_context, out);
                }
            }
        }
        Expr::Block(block, _) => {
            for item in &block.items {
                match item {
                    shape_ast::ast::BlockItem::VariableDecl(decl) => {
                        if let Some(value) = &decl.value {
                            collect_param_types_from_expr(value, type_context, out);
                        }
                    }
                    shape_ast::ast::BlockItem::Assignment(assign) => {
                        collect_param_types_from_expr(&assign.value, type_context, out);
                    }
                    shape_ast::ast::BlockItem::Expression(expr) => {
                        collect_param_types_from_expr(expr, type_context, out);
                    }
                    shape_ast::ast::BlockItem::Statement(stmt) => {
                        collect_param_types_from_statement(stmt, type_context, out);
                    }
                }
            }
        }
        Expr::FunctionExpr { body, .. } => {
            for stmt in body {
                collect_param_types_from_statement(stmt, type_context, out);
            }
        }
        Expr::If(if_expr, _) => {
            collect_param_types_from_expr(&if_expr.condition, type_context, out);
            collect_param_types_from_expr(&if_expr.then_branch, type_context, out);
            if let Some(else_branch) = &if_expr.else_branch {
                collect_param_types_from_expr(else_branch, type_context, out);
            }
        }
        Expr::While(while_expr, _) => {
            collect_param_types_from_expr(&while_expr.condition, type_context, out);
            collect_param_types_from_expr(&while_expr.body, type_context, out);
        }
        Expr::For(for_expr, _) => {
            collect_param_types_from_expr(&for_expr.iterable, type_context, out);
            collect_param_types_from_expr(&for_expr.body, type_context, out);
        }
        Expr::Loop(loop_expr, _) => {
            collect_param_types_from_expr(&loop_expr.body, type_context, out);
        }
        Expr::Let(let_expr, _) => {
            if let Some(value) = &let_expr.value {
                collect_param_types_from_expr(value, type_context, out);
            }
            collect_param_types_from_expr(&let_expr.body, type_context, out);
        }
        Expr::Assign(assign_expr, _) => {
            collect_param_types_from_expr(&assign_expr.value, type_context, out);
        }
        Expr::Range { start, end, .. } => {
            if let Some(s) = start {
                collect_param_types_from_expr(s, type_context, out);
            }
            if let Some(e) = end {
                collect_param_types_from_expr(e, type_context, out);
            }
        }
        Expr::TimeframeContext { expr, .. } => {
            collect_param_types_from_expr(expr, type_context, out);
        }
        _ => {}
    }
}

/// Infer the type of a lambda parameter based on receiver type and method
/// Supports: filter, map, forEach, reduce, find, some, every
/// For reduce: param_index 0 = accumulator (element type), param_index 1 = element
fn infer_lambda_param_type(
    receiver: &Expr,
    method: &str,
    param_index: usize,
    type_context: &HashMap<String, String>,
) -> Option<String> {
    let receiver_type = resolve_expr_type(receiver, type_context)?;

    // Special case: filter_by on BacktestResult returns Trade
    if method == "filter_by" && receiver_type.eq_ignore_ascii_case("backtestresult") {
        return Some("Trade".to_string());
    }

    // Extract element type from Table<T>, Array<T>, or similar containers
    let element_type = if let Some(elem) = series_element_type(&receiver_type) {
        elem
    } else if let Some(elem) = extract_generic_arg(&receiver_type, 0) {
        // Handle Array<T> and other generic types
        elem
    } else {
        return None;
    };

    // For most methods, all parameters are the element type
    // For reduce: param 0 = accumulator (same as element), param 1 = element
    match method {
        "reduce" => {
            // reduce((acc, item) => ...) - both params are element type
            // Note: In a more advanced implementation, accumulator could have different type
            Some(element_type)
        }
        "filter" | "map" | "forEach" | "find" | "some" | "every" | "filter_by" => {
            if param_index == 0 {
                Some(element_type)
            } else if param_index == 1 {
                // Second parameter is typically index (Number)
                Some("Number".to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn resolve_expr_type(expr: &Expr, type_context: &HashMap<String, String>) -> Option<String> {
    let empty_fields = HashMap::new();
    match expr {
        Expr::Identifier(name, _) => type_context.get(name).cloned(),
        Expr::PropertyAccess {
            object, property, ..
        } => {
            let base = resolve_expr_type(object, type_context)?;
            resolve_property_type(&base, property, &empty_fields)
        }
        Expr::MethodCall {
            receiver, method, ..
        } => {
            // Type-preserving methods return same type as receiver
            match method.as_str() {
                "filter" | "where" | "head" | "tail" | "slice" | "reverse" | "concat"
                | "orderBy" | "limit" | "sort" => resolve_expr_type(receiver, type_context),
                _ => None,
            }
        }
        _ => None,
    }
}

fn series_element_type(type_name: &str) -> Option<String> {
    let lower = type_name.to_lowercase();
    if !lower.starts_with("series<") {
        return None;
    }
    super::methods::parse_generic_type(type_name).and_then(|(base, args)| {
        if base.eq_ignore_ascii_case("series") {
            args.into_iter().next()
        } else {
            None
        }
    })
}
