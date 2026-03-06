//! Call hierarchy provider for Shape
//!
//! Supports "prepare", "incoming calls", and "outgoing calls" for function symbols.

use crate::util::{get_word_at_position, offset_to_line_col, span_to_range};
use shape_ast::ast::{BlockItem, Expr, Item, Program, Statement};
use shape_ast::parser::parse_program;
use tower_lsp_server::ls_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, Position, Range,
    SymbolKind, Uri,
};

/// Parse with resilient fallback for broken code.
fn parse_resilient(text: &str) -> Option<Program> {
    match parse_program(text) {
        Ok(p) => Some(p),
        Err(_) => {
            let partial = shape_ast::parse_program_resilient(text);
            if partial.items.is_empty() {
                None
            } else {
                Some(partial.into_program())
            }
        }
    }
}

/// Prepare call hierarchy: return a CallHierarchyItem for the function at the cursor position.
pub fn prepare_call_hierarchy(
    text: &str,
    position: Position,
    uri: &Uri,
) -> Option<Vec<CallHierarchyItem>> {
    let word = get_word_at_position(text, position)?;
    let program = parse_resilient(text)?;

    // Find the function definition matching this word
    for item in &program.items {
        match item {
            Item::Function(func, span) if func.name == word => {
                let range = span_to_range(text, span);
                let sel_range = span_to_range(text, &func.name_span);
                return Some(vec![CallHierarchyItem {
                    name: func.name.clone(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    detail: Some(format!(
                        "({})",
                        func.params
                            .iter()
                            .flat_map(|p| p.get_identifiers())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    uri: uri.clone(),
                    range,
                    selection_range: sel_range,
                    data: None,
                }]);
            }
            Item::ForeignFunction(foreign_fn, span) if foreign_fn.name == word => {
                let range = span_to_range(text, span);
                let sel_range = span_to_range(text, &foreign_fn.name_span);
                return Some(vec![CallHierarchyItem {
                    name: foreign_fn.name.clone(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    detail: Some(format!(
                        "fn {} ({})",
                        foreign_fn.language,
                        foreign_fn
                            .params
                            .iter()
                            .flat_map(|p| p.get_identifiers())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    uri: uri.clone(),
                    range,
                    selection_range: sel_range,
                    data: None,
                }]);
            }
            _ => {}
        }
    }
    None
}

/// Find all incoming calls to the function identified by `item`.
///
/// Scans all functions in the program for call expressions matching the target name.
pub fn incoming_calls(
    text: &str,
    item: &CallHierarchyItem,
    uri: &Uri,
) -> Vec<CallHierarchyIncomingCall> {
    let target_name = &item.name;
    let program = match parse_resilient(text) {
        Some(p) => p,
        None => return vec![],
    };

    let mut results = Vec::new();

    for ast_item in &program.items {
        if let Item::Function(func, func_span) = ast_item {
            // Don't report self-references as incoming (unless recursive)
            let mut call_ranges = Vec::new();
            collect_call_sites_in_stmts(&func.body, target_name, text, &mut call_ranges);

            if !call_ranges.is_empty() {
                let from_range = span_to_range(text, func_span);
                let from_sel = span_to_range(text, &func.name_span);
                results.push(CallHierarchyIncomingCall {
                    from: CallHierarchyItem {
                        name: func.name.clone(),
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        detail: None,
                        uri: uri.clone(),
                        range: from_range,
                        selection_range: from_sel,
                        data: None,
                    },
                    from_ranges: call_ranges,
                });
            }
        }
    }

    // Also check top-level expressions/statements
    let mut top_level_ranges = Vec::new();
    for ast_item in &program.items {
        match ast_item {
            Item::Statement(stmt, _) => {
                collect_call_sites_in_stmt(stmt, target_name, text, &mut top_level_ranges);
            }
            Item::Expression(expr, _) => {
                collect_call_sites_in_expr(expr, target_name, text, &mut top_level_ranges);
            }
            Item::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    collect_call_sites_in_expr(value, target_name, text, &mut top_level_ranges);
                }
            }
            _ => {}
        }
    }

    if !top_level_ranges.is_empty() {
        results.push(CallHierarchyIncomingCall {
            from: CallHierarchyItem {
                name: "<module>".to_string(),
                kind: SymbolKind::MODULE,
                tags: None,
                detail: None,
                uri: uri.clone(),
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 0,
                    },
                },
                selection_range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 0,
                    },
                },
                data: None,
            },
            from_ranges: top_level_ranges,
        });
    }

    results
}

/// Find all outgoing calls from the function identified by `item`.
///
/// Collects all function call expressions within the target function's body.
pub fn outgoing_calls(
    text: &str,
    item: &CallHierarchyItem,
    uri: &Uri,
) -> Vec<CallHierarchyOutgoingCall> {
    let program = match parse_resilient(text) {
        Some(p) => p,
        None => return vec![],
    };

    // Find the function matching the item
    for ast_item in &program.items {
        if let Item::Function(func, _) = ast_item {
            if func.name == item.name {
                let mut calls: Vec<(String, Range)> = Vec::new();
                collect_outgoing_calls_in_stmts(&func.body, text, &mut calls);

                // Group by callee name
                let mut grouped: std::collections::HashMap<String, Vec<Range>> =
                    std::collections::HashMap::new();
                for (name, range) in calls {
                    grouped.entry(name).or_default().push(range);
                }

                let mut results = Vec::new();
                for (callee_name, from_ranges) in grouped {
                    // Try to find the callee function definition for a proper item
                    let callee_item = find_function_item(&program, &callee_name, uri, text);
                    results.push(CallHierarchyOutgoingCall {
                        to: callee_item,
                        from_ranges,
                    });
                }
                return results;
            }
        }
    }

    vec![]
}

// --- Helpers ---

fn find_function_item(program: &Program, name: &str, uri: &Uri, text: &str) -> CallHierarchyItem {
    for item in &program.items {
        if let Item::Function(func, span) = item {
            if func.name == name {
                return CallHierarchyItem {
                    name: func.name.clone(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    detail: None,
                    uri: uri.clone(),
                    range: span_to_range(text, span),
                    selection_range: span_to_range(text, &func.name_span),
                    data: None,
                };
            }
        }
    }
    // Return a placeholder for unknown/builtin functions
    CallHierarchyItem {
        name: name.to_string(),
        kind: SymbolKind::FUNCTION,
        tags: None,
        detail: Some("(external)".to_string()),
        uri: uri.clone(),
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        },
        selection_range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        },
        data: None,
    }
}

fn collect_call_sites_in_stmts(
    stmts: &[Statement],
    target: &str,
    text: &str,
    out: &mut Vec<Range>,
) {
    for stmt in stmts {
        collect_call_sites_in_stmt(stmt, target, text, out);
    }
}

fn collect_call_sites_in_stmt(stmt: &Statement, target: &str, text: &str, out: &mut Vec<Range>) {
    match stmt {
        Statement::Expression(expr, _) => collect_call_sites_in_expr(expr, target, text, out),
        Statement::VariableDecl(decl, _) => {
            if let Some(value) = &decl.value {
                collect_call_sites_in_expr(value, target, text, out);
            }
        }
        Statement::Return(Some(expr), _) => collect_call_sites_in_expr(expr, target, text, out),
        Statement::If(if_stmt, _) => {
            collect_call_sites_in_expr(&if_stmt.condition, target, text, out);
            collect_call_sites_in_stmts(&if_stmt.then_body, target, text, out);
            if let Some(else_body) = &if_stmt.else_body {
                collect_call_sites_in_stmts(else_body, target, text, out);
            }
        }
        Statement::For(for_loop, _) => {
            collect_call_sites_in_stmts(&for_loop.body, target, text, out);
        }
        Statement::While(while_loop, _) => {
            collect_call_sites_in_expr(&while_loop.condition, target, text, out);
            collect_call_sites_in_stmts(&while_loop.body, target, text, out);
        }
        _ => {}
    }
}

fn collect_call_sites_in_expr(expr: &Expr, target: &str, text: &str, out: &mut Vec<Range>) {
    match expr {
        Expr::FunctionCall {
            name, args, span, ..
        } => {
            if name == target && !span.is_dummy() {
                // The call-site range covers just the function name portion
                let name_end = span.start + name.len();
                out.push(range_from_offsets(text, span.start, name_end));
            }
            for arg in args {
                collect_call_sites_in_expr(arg, target, text, out);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            collect_call_sites_in_expr(receiver, target, text, out);
            for arg in args {
                collect_call_sites_in_expr(arg, target, text, out);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_call_sites_in_expr(left, target, text, out);
            collect_call_sites_in_expr(right, target, text, out);
        }
        Expr::UnaryOp { operand, .. } => {
            collect_call_sites_in_expr(operand, target, text, out);
        }
        Expr::If(if_expr, _) => {
            collect_call_sites_in_expr(&if_expr.condition, target, text, out);
            collect_call_sites_in_expr(&if_expr.then_branch, target, text, out);
            if let Some(else_br) = &if_expr.else_branch {
                collect_call_sites_in_expr(else_br, target, text, out);
            }
        }
        Expr::Block(block, _) => {
            for item in &block.items {
                match item {
                    BlockItem::Statement(s) => {
                        collect_call_sites_in_stmt(s, target, text, out);
                    }
                    BlockItem::Expression(e) => {
                        collect_call_sites_in_expr(e, target, text, out);
                    }
                    BlockItem::VariableDecl(decl) => {
                        if let Some(value) = &decl.value {
                            collect_call_sites_in_expr(value, target, text, out);
                        }
                    }
                    BlockItem::Assignment(assign) => {
                        collect_call_sites_in_expr(&assign.value, target, text, out);
                    }
                }
            }
        }
        Expr::Array(elements, _) => {
            for el in elements {
                collect_call_sites_in_expr(el, target, text, out);
            }
        }
        Expr::PropertyAccess { object, .. } => {
            collect_call_sites_in_expr(object, target, text, out);
        }
        Expr::IndexAccess { object, index, .. } => {
            collect_call_sites_in_expr(object, target, text, out);
            collect_call_sites_in_expr(index, target, text, out);
        }
        Expr::Assign(assign, _) => {
            collect_call_sites_in_expr(&assign.target, target, text, out);
            collect_call_sites_in_expr(&assign.value, target, text, out);
        }
        Expr::Return(Some(inner), _) => {
            collect_call_sites_in_expr(inner, target, text, out);
        }
        Expr::Await(inner, _) | Expr::TryOperator(inner, _) | Expr::Spread(inner, _) => {
            collect_call_sites_in_expr(inner, target, text, out);
        }
        _ => {}
    }
}

/// Collect (callee_name, call_range) pairs from statements
fn collect_outgoing_calls_in_stmts(
    stmts: &[Statement],
    text: &str,
    out: &mut Vec<(String, Range)>,
) {
    for stmt in stmts {
        collect_outgoing_calls_in_stmt(stmt, text, out);
    }
}

fn collect_outgoing_calls_in_stmt(stmt: &Statement, text: &str, out: &mut Vec<(String, Range)>) {
    match stmt {
        Statement::Expression(expr, _) => collect_outgoing_calls_in_expr(expr, text, out),
        Statement::VariableDecl(decl, _) => {
            if let Some(value) = &decl.value {
                collect_outgoing_calls_in_expr(value, text, out);
            }
        }
        Statement::Return(Some(expr), _) => collect_outgoing_calls_in_expr(expr, text, out),
        Statement::If(if_stmt, _) => {
            collect_outgoing_calls_in_expr(&if_stmt.condition, text, out);
            collect_outgoing_calls_in_stmts(&if_stmt.then_body, text, out);
            if let Some(else_body) = &if_stmt.else_body {
                collect_outgoing_calls_in_stmts(else_body, text, out);
            }
        }
        Statement::For(for_loop, _) => {
            collect_outgoing_calls_in_stmts(&for_loop.body, text, out);
        }
        Statement::While(while_loop, _) => {
            collect_outgoing_calls_in_expr(&while_loop.condition, text, out);
            collect_outgoing_calls_in_stmts(&while_loop.body, text, out);
        }
        _ => {}
    }
}

fn collect_outgoing_calls_in_expr(expr: &Expr, text: &str, out: &mut Vec<(String, Range)>) {
    match expr {
        Expr::FunctionCall {
            name, args, span, ..
        } => {
            if !span.is_dummy() {
                let name_end = span.start + name.len();
                out.push((name.clone(), range_from_offsets(text, span.start, name_end)));
            }
            for arg in args {
                collect_outgoing_calls_in_expr(arg, text, out);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            collect_outgoing_calls_in_expr(receiver, text, out);
            for arg in args {
                collect_outgoing_calls_in_expr(arg, text, out);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_outgoing_calls_in_expr(left, text, out);
            collect_outgoing_calls_in_expr(right, text, out);
        }
        Expr::UnaryOp { operand, .. } => {
            collect_outgoing_calls_in_expr(operand, text, out);
        }
        Expr::If(if_expr, _) => {
            collect_outgoing_calls_in_expr(&if_expr.condition, text, out);
            collect_outgoing_calls_in_expr(&if_expr.then_branch, text, out);
            if let Some(else_br) = &if_expr.else_branch {
                collect_outgoing_calls_in_expr(else_br, text, out);
            }
        }
        Expr::Block(block, _) => {
            for item in &block.items {
                match item {
                    BlockItem::Statement(s) => collect_outgoing_calls_in_stmt(s, text, out),
                    BlockItem::Expression(e) => collect_outgoing_calls_in_expr(e, text, out),
                    BlockItem::VariableDecl(decl) => {
                        if let Some(value) = &decl.value {
                            collect_outgoing_calls_in_expr(value, text, out);
                        }
                    }
                    BlockItem::Assignment(assign) => {
                        collect_outgoing_calls_in_expr(&assign.value, text, out);
                    }
                }
            }
        }
        Expr::Array(elements, _) => {
            for el in elements {
                collect_outgoing_calls_in_expr(el, text, out);
            }
        }
        Expr::PropertyAccess { object, .. } => {
            collect_outgoing_calls_in_expr(object, text, out);
        }
        Expr::IndexAccess { object, index, .. } => {
            collect_outgoing_calls_in_expr(object, text, out);
            collect_outgoing_calls_in_expr(index, text, out);
        }
        Expr::Assign(assign, _) => {
            collect_outgoing_calls_in_expr(&assign.target, text, out);
            collect_outgoing_calls_in_expr(&assign.value, text, out);
        }
        Expr::Return(Some(inner), _) => {
            collect_outgoing_calls_in_expr(inner, text, out);
        }
        Expr::Await(inner, _) | Expr::TryOperator(inner, _) | Expr::Spread(inner, _) => {
            collect_outgoing_calls_in_expr(inner, text, out);
        }
        _ => {}
    }
}

// --- Utility ---

fn range_from_offsets(text: &str, start: usize, end: usize) -> Range {
    let (sl, sc) = offset_to_line_col(text, start);
    let (el, ec) = offset_to_line_col(text, end);
    Range {
        start: Position {
            line: sl,
            character: sc,
        },
        end: Position {
            line: el,
            character: ec,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepare_call_hierarchy() {
        let code = "fn foo(x) {\n  return x + 1\n}\nfn bar() {\n  return foo(42)\n}";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        // Position on "foo" in the definition
        let result = prepare_call_hierarchy(
            code,
            Position {
                line: 0,
                character: 3,
            },
            &uri,
        );
        assert!(result.is_some(), "Should find function at cursor");
        let items = result.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "foo");
    }

    #[test]
    fn test_incoming_calls() {
        let code = "fn foo(x) {\n  return x + 1\n}\nfn bar() {\n  return foo(42)\n}";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let item = CallHierarchyItem {
            name: "foo".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri: uri.clone(),
            range: Range::default(),
            selection_range: Range::default(),
            data: None,
        };

        let calls = incoming_calls(code, &item, &uri);
        assert!(!calls.is_empty(), "foo should have incoming calls from bar");
        assert_eq!(calls[0].from.name, "bar");
    }

    #[test]
    fn test_outgoing_calls() {
        let code = "fn add(a, b) {\n  return a + b\n}\nfn mul(a, b) {\n  return a * b\n}\nfn compute(x) {\n  return add(x, mul(x, 2))\n}";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let item = CallHierarchyItem {
            name: "compute".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri: uri.clone(),
            range: Range::default(),
            selection_range: Range::default(),
            data: None,
        };

        let calls = outgoing_calls(code, &item, &uri);
        let callee_names: Vec<&str> = calls.iter().map(|c| c.to.name.as_str()).collect();
        assert!(
            callee_names.contains(&"add"),
            "compute should call add, got {:?}",
            callee_names
        );
        assert!(
            callee_names.contains(&"mul"),
            "compute should call mul, got {:?}",
            callee_names
        );
    }

    #[test]
    fn test_no_incoming_calls() {
        let code = "fn unused() {\n  return 42\n}";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let item = CallHierarchyItem {
            name: "unused".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri: uri.clone(),
            range: Range::default(),
            selection_range: Range::default(),
            data: None,
        };

        let calls = incoming_calls(code, &item, &uri);
        assert!(calls.is_empty(), "unused should have no incoming calls");
    }

    #[test]
    fn test_top_level_incoming_call() {
        let code = "fn foo() {\n  return 1\n}\nlet x = foo()";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let item = CallHierarchyItem {
            name: "foo".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri: uri.clone(),
            range: Range::default(),
            selection_range: Range::default(),
            data: None,
        };

        let calls = incoming_calls(code, &item, &uri);
        assert!(
            !calls.is_empty(),
            "foo should have incoming call from top-level"
        );
        // Top-level calls come from the <module> item
        assert!(
            calls.iter().any(|c| c.from.name == "<module>"),
            "Should have module-level caller"
        );
    }
}
