//! Folding range support for Shape LSP
//!
//! Provides foldable regions for functions, types, traits, impls, enums,
//! annotations, blocks, and import groups.

use shape_ast::ast::{Expr, Item, Program, Span, Statement};
use tower_lsp_server::ls_types::{FoldingRange, FoldingRangeKind};

/// Compute folding ranges for a Shape source document.
///
/// Walks the AST for multi-line constructs and also scans raw source
/// for comment blocks and consecutive import groups.
pub fn get_folding_ranges(source: &str, program: &Program) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();

    // Collect comment folding ranges from raw source
    collect_comment_folds(source, &mut ranges);

    // Collect import group folding ranges
    collect_import_folds(source, program, &mut ranges);

    // Walk AST items for structural folds
    for item in &program.items {
        collect_item_folds(source, item, &mut ranges);
    }

    ranges
}

/// Convert a byte-offset Span to (start_line, end_line). Returns None if single-line.
fn span_to_lines(source: &str, span: Span) -> Option<(u32, u32)> {
    if span.is_empty() || span.is_dummy() {
        return None;
    }
    let start_line = source[..span.start].matches('\n').count() as u32;
    let end_line = source[..span.end.min(source.len())].matches('\n').count() as u32;
    if end_line > start_line {
        Some((start_line, end_line))
    } else {
        None
    }
}

fn add_region_fold(ranges: &mut Vec<FoldingRange>, start_line: u32, end_line: u32) {
    ranges.push(FoldingRange {
        start_line,
        start_character: None,
        end_line,
        end_character: None,
        kind: Some(FoldingRangeKind::Region),
        collapsed_text: None,
    });
}

fn collect_item_folds(source: &str, item: &Item, ranges: &mut Vec<FoldingRange>) {
    match item {
        Item::Function(func, span) => {
            if let Some((start, end)) = span_to_lines(source, *span) {
                add_region_fold(ranges, start, end);
            }
            // Fold nested blocks in function body
            for stmt in &func.body {
                collect_stmt_folds(source, stmt, ranges);
            }
        }
        Item::ForeignFunction(_, span)
        | Item::StructType(_, span)
        | Item::Enum(_, span)
        | Item::Trait(_, span)
        | Item::Impl(_, span)
        | Item::Interface(_, span)
        | Item::Extend(_, span)
        | Item::AnnotationDef(_, span)
        | Item::DataSource(_, span)
        | Item::QueryDecl(_, span)
        | Item::Stream(_, span)
        | Item::Test(_, span)
        | Item::Optimize(_, span) => {
            if let Some((start, end)) = span_to_lines(source, *span) {
                add_region_fold(ranges, start, end);
            }
        }
        Item::Statement(stmt, _) => {
            collect_stmt_folds(source, stmt, ranges);
        }
        Item::Expression(expr, _) => {
            collect_expr_folds(source, expr, ranges);
        }
        // Single-line items: imports, exports, variable decls, assignments, comptime
        _ => {}
    }
}

fn collect_stmt_folds(source: &str, stmt: &Statement, ranges: &mut Vec<FoldingRange>) {
    match stmt {
        Statement::If(if_stmt, span) => {
            if let Some((start, end)) = span_to_lines(source, *span) {
                add_region_fold(ranges, start, end);
            }
            for s in &if_stmt.then_body {
                collect_stmt_folds(source, s, ranges);
            }
            if let Some(else_stmts) = &if_stmt.else_body {
                for s in else_stmts {
                    collect_stmt_folds(source, s, ranges);
                }
            }
        }
        Statement::For(_, span) | Statement::While(_, span) => {
            if let Some((start, end)) = span_to_lines(source, *span) {
                add_region_fold(ranges, start, end);
            }
        }
        Statement::Expression(expr, _) => {
            collect_expr_folds(source, expr, ranges);
        }
        _ => {}
    }
}

fn collect_expr_folds(source: &str, expr: &Expr, ranges: &mut Vec<FoldingRange>) {
    match expr {
        Expr::Block(block, span) => {
            if let Some((start, end)) = span_to_lines(source, *span) {
                add_region_fold(ranges, start, end);
            }
            for item in &block.items {
                match item {
                    shape_ast::ast::BlockItem::Statement(s) => {
                        collect_stmt_folds(source, s, ranges)
                    }
                    shape_ast::ast::BlockItem::Expression(e) => {
                        collect_expr_folds(source, e, ranges)
                    }
                    _ => {}
                }
            }
        }
        Expr::If(if_expr, span) => {
            if let Some((start, end)) = span_to_lines(source, *span) {
                add_region_fold(ranges, start, end);
            }
            collect_expr_folds(source, &if_expr.then_branch, ranges);
            if let Some(else_br) = &if_expr.else_branch {
                collect_expr_folds(source, else_br, ranges);
            }
        }
        Expr::Conditional {
            span,
            then_expr,
            else_expr,
            ..
        } => {
            if let Some((start, end)) = span_to_lines(source, *span) {
                add_region_fold(ranges, start, end);
            }
            collect_expr_folds(source, then_expr, ranges);
            if let Some(else_br) = else_expr {
                collect_expr_folds(source, else_br, ranges);
            }
        }
        Expr::For(_, span) | Expr::While(_, span) | Expr::Loop(_, span) | Expr::Match(_, span) => {
            if let Some((start, end)) = span_to_lines(source, *span) {
                add_region_fold(ranges, start, end);
            }
        }
        Expr::FunctionExpr { body, .. } => {
            for stmt in body {
                collect_stmt_folds(source, stmt, ranges);
            }
        }
        _ => {}
    }
}

/// Scan source for consecutive line comments (// ...) and block comments (/* ... */).
fn collect_comment_folds(source: &str, ranges: &mut Vec<FoldingRange>) {
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        // Consecutive line comments
        if trimmed.starts_with("//") {
            let start = i;
            while i < lines.len() && lines[i].trim_start().starts_with("//") {
                i += 1;
            }
            let end = i - 1;
            if end > start {
                ranges.push(FoldingRange {
                    start_line: start as u32,
                    start_character: None,
                    end_line: end as u32,
                    end_character: None,
                    kind: Some(FoldingRangeKind::Comment),
                    collapsed_text: None,
                });
            }
            continue;
        }
        // Block comments: find /* and scan to */
        if trimmed.starts_with("/*") {
            let start = i;
            let mut depth = 0u32;
            let mut found_end = false;
            while i < lines.len() {
                let line = lines[i];
                for (idx, _) in line.char_indices() {
                    if line[idx..].starts_with("/*") {
                        depth += 1;
                    } else if line[idx..].starts_with("*/") {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            found_end = true;
                            break;
                        }
                    }
                }
                if found_end {
                    break;
                }
                i += 1;
            }
            let end = i;
            if end > start {
                ranges.push(FoldingRange {
                    start_line: start as u32,
                    start_character: None,
                    end_line: end as u32,
                    end_character: None,
                    kind: Some(FoldingRangeKind::Comment),
                    collapsed_text: None,
                });
            }
            i += 1;
            continue;
        }
        i += 1;
    }
}

/// Group consecutive import statements into a single Imports fold.
fn collect_import_folds(source: &str, program: &Program, ranges: &mut Vec<FoldingRange>) {
    let mut import_lines: Vec<u32> = Vec::new();
    for item in &program.items {
        if let Item::Import(_, span) = item {
            if !span.is_dummy() {
                let line = source[..span.start].matches('\n').count() as u32;
                import_lines.push(line);
            }
        }
    }
    if import_lines.len() < 2 {
        return;
    }
    import_lines.sort();

    // Group consecutive lines (allowing gaps of 1 blank line)
    let mut group_start = import_lines[0];
    let mut group_end = import_lines[0];
    for &line in &import_lines[1..] {
        if line <= group_end + 2 {
            group_end = line;
        } else {
            if group_end > group_start {
                ranges.push(FoldingRange {
                    start_line: group_start,
                    start_character: None,
                    end_line: group_end,
                    end_character: None,
                    kind: Some(FoldingRangeKind::Imports),
                    collapsed_text: None,
                });
            }
            group_start = line;
            group_end = line;
        }
    }
    if group_end > group_start {
        ranges.push(FoldingRange {
            start_line: group_start,
            start_character: None,
            end_line: group_end,
            end_character: None,
            kind: Some(FoldingRangeKind::Imports),
            collapsed_text: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    fn fold_kinds(source: &str) -> Vec<(u32, u32, Option<FoldingRangeKind>)> {
        let program = parse_program(source).expect("parse should succeed");
        let ranges = get_folding_ranges(source, &program);
        ranges
            .into_iter()
            .map(|r| (r.start_line, r.end_line, r.kind))
            .collect()
    }

    #[test]
    fn test_function_fold() {
        let source = "fn foo(a) {\n  return a\n}";
        let folds = fold_kinds(source);
        assert!(
            folds
                .iter()
                .any(|(s, e, k)| *s == 0 && *e == 2 && *k == Some(FoldingRangeKind::Region)),
            "expected function fold 0..2, got: {:?}",
            folds
        );
    }

    #[test]
    fn test_enum_fold() {
        let source = "enum Color {\n  Red,\n  Green,\n  Blue\n}";
        let folds = fold_kinds(source);
        assert!(
            folds
                .iter()
                .any(|(s, e, k)| *s == 0 && *e == 4 && *k == Some(FoldingRangeKind::Region)),
            "expected enum fold 0..4, got: {:?}",
            folds
        );
    }

    #[test]
    fn test_trait_fold() {
        let source = "trait Printable {\n  method to_string() -> string {\n    return \"\"\n  }\n}";
        let folds = fold_kinds(source);
        assert!(
            folds
                .iter()
                .any(|(s, _e, k)| *s == 0 && *k == Some(FoldingRangeKind::Region)),
            "expected trait fold starting at line 0, got: {:?}",
            folds
        );
    }

    #[test]
    fn test_comment_fold() {
        let source = "// line 1\n// line 2\n// line 3\nlet x = 1";
        let program = parse_program(source).expect("parse");
        let ranges = get_folding_ranges(source, &program);
        assert!(
            ranges.iter().any(|r| r.start_line == 0
                && r.end_line == 2
                && r.kind == Some(FoldingRangeKind::Comment)),
            "expected comment fold 0..2, got: {:?}",
            ranges
        );
    }

    #[test]
    fn test_import_fold() {
        let source = "from a use { a }\nfrom b use { b }\nfrom c use { c }\nlet x = 1";
        let program = parse_program(source).expect("parse");
        let ranges = get_folding_ranges(source, &program);
        assert!(
            ranges
                .iter()
                .any(|r| r.kind == Some(FoldingRangeKind::Imports)),
            "expected import fold, got: {:?}",
            ranges
        );
    }

    #[test]
    fn test_single_line_no_fold() {
        let source = "let x = 1";
        let program = parse_program(source).expect("parse");
        let ranges = get_folding_ranges(source, &program);
        // Should have no region folds for single-line content
        assert!(
            !ranges
                .iter()
                .any(|r| r.kind == Some(FoldingRangeKind::Region)),
            "single line should not produce region folds, got: {:?}",
            ranges
        );
    }

    #[test]
    fn test_nested_folds() {
        let source = "fn foo() {\n  if true {\n    let x = 1\n  }\n}";
        let folds = fold_kinds(source);
        // Should have at least 2 region folds (function + if block)
        let region_folds: Vec<_> = folds
            .iter()
            .filter(|(_, _, k)| *k == Some(FoldingRangeKind::Region))
            .collect();
        assert!(
            region_folds.len() >= 2,
            "expected at least 2 nested folds, got: {:?}",
            region_folds
        );
    }
}
