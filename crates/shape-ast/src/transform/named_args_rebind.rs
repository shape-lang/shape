//! Named-argument rebinding pass (STAGE T4, 2026-06-22).
//!
//! `functions.mdx` documents named call arguments (`box_vol(w: 2, h: 3, d:
//! 4)`). They parse into `Expr::FunctionCall { args, named_args, .. }` with the
//! `name: value` pairs kept in `named_args`. Every downstream consumer
//! (type-inference call-shape constraint, bytecode compiler call-lowering, MIR
//! lowering) reads only the POSITIONAL `args` slice — so a named-only call was
//! seen as zero positional arguments (`expects between 3 and 3 arguments, got
//! 0`) and, with default params, silently dropped the named values.
//!
//! This pass runs once, right after desugaring, BEFORE inference and codegen.
//! It collects user function signatures (parameter names + default
//! expressions) from the program and rewrites each `FunctionCall` that carries
//! named arguments into a fully positional call: `args[i]` becomes the value
//! supplied for parameter `i` (positionally or by name), or that parameter's
//! declared default when omitted. The single rewritten AST then feeds every
//! downstream pass unchanged.
//!
//! Clean compile-errors (ADR-006 surface-and-stop — never a silent
//! miscompute):
//!   - a named argument whose name matches no parameter;
//!   - a parameter supplied twice (positional + named, or duplicate named);
//!   - an interior parameter left without a value or default.
//!
//! Named args on a call whose name is NOT a known user function (builtin /
//! enum constructor / local callable value) are left untouched here; the
//! bytecode compiler's `resolve_named_function_args` rejects those at the call
//! site (where local-binding resolution is available).

use std::collections::HashMap;

use crate::ast::{Expr, ForInit, Item, ObjectEntry, Program, QueryClause, Statement};
use crate::error::{Result, ShapeError};

/// One parameter's name and (optional) default expression.
#[derive(Clone)]
struct ParamInfo {
    name: Option<String>,
    default: Option<Expr>,
}

type Signatures = HashMap<String, Vec<ParamInfo>>;

/// Rebind named function-call arguments to positional form across the program.
pub fn rebind_named_args(program: &mut Program) -> Result<()> {
    let sigs = collect_signatures(program);
    let mut err: Option<ShapeError> = None;
    for item in &mut program.items {
        rewrite_item(item, &sigs, &mut err);
    }
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn param_infos(params: &[crate::ast::FunctionParameter]) -> Vec<ParamInfo> {
    params
        .iter()
        .map(|p| ParamInfo {
            name: p.simple_name().map(|s| s.to_string()),
            default: p.default_value.clone(),
        })
        .collect()
}

fn collect_signatures(program: &Program) -> Signatures {
    let mut sigs = Signatures::new();
    fn from_items(items: &[Item], sigs: &mut Signatures) {
        for item in items {
            match item {
                Item::Function(func, _) => {
                    sigs.entry(func.name.clone())
                        .or_insert_with(|| param_infos(&func.params));
                }
                Item::Export(export, _) => {
                    if let crate::ast::ExportItem::Function(func) = &export.item {
                        sigs.entry(func.name.clone())
                            .or_insert_with(|| param_infos(&func.params));
                    }
                }
                Item::Module(module, _) => from_items(&module.items, sigs),
                _ => {}
            }
        }
    }
    from_items(&program.items, &mut sigs);
    sigs
}

/// Rebind a single `FunctionCall`'s named args into positional form, in place.
/// Records the first error encountered into `err` (and leaves the node as-is).
fn rebind_call(
    name: &str,
    args: &mut Vec<Expr>,
    named_args: &mut Vec<(String, Expr)>,
    sigs: &Signatures,
    err: &mut Option<ShapeError>,
) {
    if named_args.is_empty() {
        return;
    }
    // Only user functions with statically-known parameter names are rebound
    // here. Anything else is left for the call-site compiler to reject.
    let Some(params) = sigs.get(name) else {
        return;
    };
    let n_params = params.len();

    if args.len() > n_params {
        set_err(
            err,
            format!(
                "Function '{}' expects at most {} positional argument(s), got {}",
                name,
                n_params,
                args.len()
            ),
        );
        return;
    }

    let mut slots: Vec<Option<Expr>> = vec![None; n_params];
    for (i, arg) in args.drain(..).enumerate() {
        slots[i] = Some(arg);
    }

    for (arg_name, arg_expr) in named_args.drain(..) {
        let Some(idx) = params
            .iter()
            .position(|p| p.name.as_deref() == Some(arg_name.as_str()))
        else {
            set_err(
                err,
                format!("Function '{name}' has no parameter named '{arg_name}'"),
            );
            return;
        };
        if slots[idx].is_some() {
            set_err(
                err,
                format!(
                    "Argument for parameter '{arg_name}' of function '{name}' was \
                     supplied more than once (positional and/or named)"
                ),
            );
            return;
        }
        slots[idx] = Some(arg_expr);
    }

    // Fill omitted parameters carrying a default.
    for (idx, slot) in slots.iter_mut().enumerate() {
        if slot.is_none() {
            if let Some(default_expr) = params[idx].default.clone() {
                *slot = Some(default_expr);
            }
        }
    }

    // Emit a dense positional vec up to the highest filled slot. A trailing
    // run of unfilled-without-default slots is left off so the downstream
    // arity check reports the missing required arguments; an INTERIOR hole is
    // a clean error here (the positional form cannot express it).
    let last_filled = slots.iter().rposition(|s| s.is_some());
    let mut positional: Vec<Expr> = Vec::with_capacity(n_params);
    if let Some(last) = last_filled {
        for (idx, slot) in slots.into_iter().enumerate().take(last + 1) {
            match slot {
                Some(expr) => positional.push(expr),
                None => {
                    let pname = params
                        .get(idx)
                        .and_then(|p| p.name.clone())
                        .unwrap_or_else(|| format!("#{}", idx + 1));
                    set_err(
                        err,
                        format!(
                            "Function '{name}' is missing a value for parameter \
                             '{pname}' (it has no default and was not supplied \
                             positionally or by name)"
                        ),
                    );
                    return;
                }
            }
        }
    }

    *args = positional;
    // named_args already drained to empty.
}

fn set_err(err: &mut Option<ShapeError>, message: String) {
    if err.is_none() {
        *err = Some(ShapeError::SemanticError {
            message,
            location: None,
        });
    }
}

// ===== Exhaustive traversal =====
//
// The matches below are exhaustive on `Expr` / `Statement` so the compiler
// flags any new variant (per the project's exhaustive-match rule). Every arm
// recurses into all child expressions and statements; `FunctionCall` is the
// only node that is also rewritten.

fn rewrite_item(item: &mut Item, sigs: &Signatures, err: &mut Option<ShapeError>) {
    match item {
        Item::Function(func, _) => stmts(&mut func.body, sigs, err),
        Item::Export(export, _) => {
            if let crate::ast::ExportItem::Function(func) = &mut export.item {
                stmts(&mut func.body, sigs, err);
            }
        }
        Item::VariableDecl(decl, _) => {
            if let Some(value) = &mut decl.value {
                expr(value, sigs, err);
            }
        }
        Item::Assignment(assign, _) => expr(&mut assign.value, sigs, err),
        Item::Expression(e, _) => expr(e, sigs, err),
        Item::Statement(stmt, _) => statement(stmt, sigs, err),
        Item::Module(module, _) => {
            for inner in &mut module.items {
                rewrite_item(inner, sigs, err);
            }
        }
        Item::Extend(extend, _) => {
            for method in &mut extend.methods {
                stmts(&mut method.body, sigs, err);
            }
        }
        Item::Impl(impl_block, _) => {
            for method in &mut impl_block.methods {
                stmts(&mut method.body, sigs, err);
            }
        }
        Item::Comptime(body, _) => stmts(body, sigs, err),
        // Item shapes whose bodies are outside the named-arg gate scope, or
        // that carry no embedded executable expressions, are not traversed.
        Item::Import(_, _)
        | Item::TypeAlias(_, _)
        | Item::Trait(_, _)
        | Item::Enum(_, _)
        | Item::Query(_, _)
        | Item::Test(_, _)
        | Item::Stream(_, _)
        | Item::Optimize(_, _)
        | Item::AnnotationDef(_, _)
        | Item::StructType(_, _)
        | Item::DataSource(_, _)
        | Item::QueryDecl(_, _)
        | Item::BuiltinTypeDecl(_, _)
        | Item::BuiltinFunctionDecl(_, _)
        | Item::ForeignFunction(_, _) => {}
    }
}

fn stmts(list: &mut [Statement], sigs: &Signatures, err: &mut Option<ShapeError>) {
    for s in list {
        statement(s, sigs, err);
    }
}

fn statement(stmt: &mut Statement, sigs: &Signatures, err: &mut Option<ShapeError>) {
    match stmt {
        Statement::Return(Some(e), _) => expr(e, sigs, err),
        Statement::Return(None, _) => {}
        Statement::Break(_) | Statement::Continue(_) => {}
        Statement::VariableDecl(decl, _) => {
            if let Some(value) = &mut decl.value {
                expr(value, sigs, err);
            }
        }
        Statement::Assignment(assign, _) => expr(&mut assign.value, sigs, err),
        Statement::Expression(e, _) => expr(e, sigs, err),
        Statement::For(for_loop, _) => {
            match &mut for_loop.init {
                ForInit::ForIn { iter, .. } => expr(iter, sigs, err),
                ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    statement(init, sigs, err);
                    expr(condition, sigs, err);
                    expr(update, sigs, err);
                }
            }
            stmts(&mut for_loop.body, sigs, err);
        }
        Statement::While(while_loop, _) => {
            expr(&mut while_loop.condition, sigs, err);
            stmts(&mut while_loop.body, sigs, err);
        }
        Statement::If(if_stmt, _) => {
            expr(&mut if_stmt.condition, sigs, err);
            stmts(&mut if_stmt.then_body, sigs, err);
            if let Some(else_body) = &mut if_stmt.else_body {
                stmts(else_body, sigs, err);
            }
        }
        Statement::Extend(extend, _) => {
            for method in &mut extend.methods {
                stmts(&mut method.body, sigs, err);
            }
        }
        // Annotation-handler mutation statements — recurse into embedded exprs.
        Statement::SetParamValue { expression, .. }
        | Statement::SetParamTypeExpr { expression, .. }
        | Statement::SetReturnExpr { expression, .. }
        | Statement::ReplaceBodyExpr { expression, .. }
        | Statement::ReplaceModuleExpr { expression, .. }
        | Statement::ExtendItemsExpr { expression, .. } => expr(expression, sigs, err),
        Statement::ReplaceBody { body, .. } => stmts(body, sigs, err),
        // No embedded exprs to rewrite.
        Statement::RemoveTarget(_)
        | Statement::SetParamType { .. }
        | Statement::SetReturnType { .. } => {}
    }
}

fn exprs(list: &mut [Expr], sigs: &Signatures, err: &mut Option<ShapeError>) {
    for e in list {
        expr(e, sigs, err);
    }
}

fn expr(e: &mut Expr, sigs: &Signatures, err: &mut Option<ShapeError>) {
    match e {
        Expr::FunctionCall {
            name,
            const_args,
            args,
            named_args,
            ..
        } => {
            exprs(const_args, sigs, err);
            exprs(args, sigs, err);
            for (_, v) in named_args.iter_mut() {
                expr(v, sigs, err);
            }
            rebind_call(name, args, named_args, sigs, err);
        }
        Expr::QualifiedFunctionCall {
            const_args,
            args,
            named_args,
            ..
        } => {
            exprs(const_args, sigs, err);
            exprs(args, sigs, err);
            for (_, v) in named_args.iter_mut() {
                expr(v, sigs, err);
            }
        }
        Expr::MethodCall {
            receiver,
            args,
            named_args,
            ..
        } => {
            expr(receiver, sigs, err);
            exprs(args, sigs, err);
            for (_, v) in named_args.iter_mut() {
                expr(v, sigs, err);
            }
        }
        Expr::EnumConstructor { payload, .. } => match payload {
            crate::ast::EnumConstructorPayload::Unit => {}
            crate::ast::EnumConstructorPayload::Tuple(items) => exprs(items, sigs, err),
            crate::ast::EnumConstructorPayload::Struct(fields) => {
                for (_, v) in fields.iter_mut() {
                    expr(v, sigs, err);
                }
            }
        },
        Expr::BinaryOp { left, right, .. } => {
            expr(left, sigs, err);
            expr(right, sigs, err);
        }
        Expr::FuzzyComparison { left, right, .. } => {
            expr(left, sigs, err);
            expr(right, sigs, err);
        }
        Expr::UnaryOp { operand, .. } => expr(operand, sigs, err),
        Expr::DataRelativeAccess { reference, .. } => expr(reference, sigs, err),
        Expr::PropertyAccess { object, .. } => expr(object, sigs, err),
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            expr(object, sigs, err);
            expr(index, sigs, err);
            if let Some(end) = end_index {
                expr(end, sigs, err);
            }
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            expr(condition, sigs, err);
            expr(then_expr, sigs, err);
            if let Some(else_e) = else_expr {
                expr(else_e, sigs, err);
            }
        }
        Expr::Object(entries, _) => {
            for entry in entries {
                match entry {
                    ObjectEntry::Field { value, .. } => expr(value, sigs, err),
                    ObjectEntry::Spread(inner) => expr(inner, sigs, err),
                }
            }
        }
        Expr::Array(elements, _) => exprs(elements, sigs, err),
        Expr::TableRows(rows, _) => {
            for row in rows {
                exprs(row, sigs, err);
            }
        }
        Expr::ListComprehension(comp, _) => {
            expr(&mut comp.element, sigs, err);
            for clause in &mut comp.clauses {
                expr(&mut clause.iterable, sigs, err);
                if let Some(filter) = &mut clause.filter {
                    expr(filter, sigs, err);
                }
            }
        }
        Expr::Block(block, _) => {
            for bi in &mut block.items {
                match bi {
                    crate::ast::BlockItem::VariableDecl(decl) => {
                        if let Some(value) = &mut decl.value {
                            expr(value, sigs, err);
                        }
                    }
                    crate::ast::BlockItem::Assignment(assign) => expr(&mut assign.value, sigs, err),
                    crate::ast::BlockItem::Statement(stmt) => statement(stmt, sigs, err),
                    crate::ast::BlockItem::Expression(e) => expr(e, sigs, err),
                }
            }
        }
        // ADR-009 A2: type syntax has no child expressions and no named
        // args to rebind.
        Expr::TypeSyntax(_, _) => {}
        Expr::TypeAssertion {
            expr: inner,
            meta_param_overrides,
            ..
        } => {
            expr(inner, sigs, err);
            if let Some(overrides) = meta_param_overrides {
                for v in overrides.values_mut() {
                    expr(v, sigs, err);
                }
            }
        }
        Expr::InstanceOf { expr: inner, .. } => expr(inner, sigs, err),
        Expr::FunctionExpr { body, .. } => stmts(body, sigs, err),
        Expr::Spread(inner, _) => expr(inner, sigs, err),
        Expr::If(if_expr, _) => {
            expr(&mut if_expr.condition, sigs, err);
            expr(&mut if_expr.then_branch, sigs, err);
            if let Some(else_branch) = &mut if_expr.else_branch {
                expr(else_branch, sigs, err);
            }
        }
        Expr::While(while_expr, _) => {
            expr(&mut while_expr.condition, sigs, err);
            expr(&mut while_expr.body, sigs, err);
        }
        Expr::For(for_expr, _) => {
            expr(&mut for_expr.iterable, sigs, err);
            expr(&mut for_expr.body, sigs, err);
        }
        Expr::Loop(loop_expr, _) => expr(&mut loop_expr.body, sigs, err),
        Expr::Let(let_expr, _) => {
            if let Some(value) = &mut let_expr.value {
                expr(value, sigs, err);
            }
            expr(&mut let_expr.body, sigs, err);
        }
        Expr::Assign(assign_expr, _) => {
            expr(&mut assign_expr.target, sigs, err);
            expr(&mut assign_expr.value, sigs, err);
        }
        Expr::Break(Some(inner), _) => expr(inner, sigs, err),
        Expr::Break(None, _) => {}
        Expr::Return(Some(inner), _) => expr(inner, sigs, err),
        Expr::Return(None, _) => {}
        Expr::Continue(_) => {}
        Expr::Match(match_expr, _) => {
            expr(&mut match_expr.scrutinee, sigs, err);
            for arm in &mut match_expr.arms {
                if let Some(guard) = &mut arm.guard {
                    expr(guard, sigs, err);
                }
                expr(&mut arm.body, sigs, err);
            }
        }
        Expr::Range { start, end, .. } => {
            if let Some(s) = start {
                expr(s, sigs, err);
            }
            if let Some(en) = end {
                expr(en, sigs, err);
            }
        }
        Expr::TimeframeContext { expr: inner, .. } => expr(inner, sigs, err),
        Expr::TryOperator(inner, _) => expr(inner, sigs, err),
        Expr::UsingImpl { expr: inner, .. } => expr(inner, sigs, err),
        Expr::SimulationCall { params, .. } => {
            for (_, v) in params.iter_mut() {
                expr(v, sigs, err);
            }
        }
        Expr::WindowExpr(_, _) => {}
        Expr::FromQuery(from_query, _) => {
            expr(&mut from_query.source, sigs, err);
            for clause in &mut from_query.clauses {
                match clause {
                    QueryClause::Where(pred) => expr(pred, sigs, err),
                    QueryClause::OrderBy(specs) => {
                        for spec in specs {
                            expr(&mut spec.key, sigs, err);
                        }
                    }
                    QueryClause::GroupBy { element, key, .. } => {
                        expr(element, sigs, err);
                        expr(key, sigs, err);
                    }
                    QueryClause::Let { value, .. } => expr(value, sigs, err),
                    QueryClause::Join {
                        source,
                        left_key,
                        right_key,
                        ..
                    } => {
                        expr(source, sigs, err);
                        expr(left_key, sigs, err);
                        expr(right_key, sigs, err);
                    }
                }
            }
            expr(&mut from_query.select, sigs, err);
        }
        Expr::StructLiteral { fields, .. } => {
            for (_, v) in fields.iter_mut() {
                expr(v, sigs, err);
            }
        }
        Expr::Await(inner, _) => expr(inner, sigs, err),
        Expr::Join(join_expr, _) => {
            for branch in &mut join_expr.branches {
                expr(&mut branch.expr, sigs, err);
            }
        }
        Expr::Annotated { target, .. } => expr(target, sigs, err),
        Expr::AsyncLet(async_let, _) => expr(&mut async_let.expr, sigs, err),
        Expr::AsyncScope(inner, _) => expr(inner, sigs, err),
        Expr::Comptime(body, _) => stmts(body, sigs, err),
        Expr::ComptimeFor(comptime_for, _) => {
            expr(&mut comptime_for.iterable, sigs, err);
            stmts(&mut comptime_for.body, sigs, err);
        }
        Expr::Reference { expr: inner, .. } => expr(inner, sigs, err),
        // Leaves — no child expressions.
        Expr::Literal(_, _)
        | Expr::Identifier(_, _)
        | Expr::DataRef(_, _)
        | Expr::DataDateTimeRef(_, _)
        | Expr::TimeRef(_, _)
        | Expr::DateTime(_, _)
        | Expr::PatternRef(_, _)
        | Expr::Duration(_, _)
        | Expr::Unit(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_program;

    /// Pull a top-level call expression out of an `Item` (whether it parsed as
    /// `Item::Expression` or `Item::Statement(Statement::Expression)`).
    fn top_call(item: &Item) -> Option<&Expr> {
        match item {
            Item::Expression(e @ Expr::FunctionCall { .. }, _) => Some(e),
            Item::Statement(Statement::Expression(e @ Expr::FunctionCall { .. }, _), _) => Some(e),
            _ => None,
        }
    }

    /// Rebind, then return the first top-level call expression's positional args.
    fn rebound_call_args(src: &str) -> Result<Vec<Expr>> {
        let mut program = parse_program(src).expect("parse");
        rebind_named_args(&mut program)?;
        for item in &program.items {
            if let Some(Expr::FunctionCall {
                args, named_args, ..
            }) = top_call(item)
            {
                assert!(named_args.is_empty(), "named_args must be drained");
                return Ok(args.clone());
            }
        }
        panic!("no top-level call expression found");
    }

    fn as_int(e: &Expr) -> i64 {
        match e {
            Expr::Literal(crate::ast::Literal::Int(n), _) => *n,
            other => panic!("expected int literal, got {other:?}"),
        }
    }

    #[test]
    fn all_named_rebinds_in_param_order() {
        let args =
            rebound_call_args("fn bv(w: int, h: int, d: int) -> int { w }\nbv(w: 2, h: 3, d: 4)")
                .expect("ok");
        assert_eq!(args.len(), 3);
        assert_eq!(as_int(&args[0]), 2);
        assert_eq!(as_int(&args[1]), 3);
        assert_eq!(as_int(&args[2]), 4);
    }

    #[test]
    fn out_of_order_named_rebinds_in_param_order() {
        let args =
            rebound_call_args("fn bv(w: int, h: int, d: int) -> int { w }\nbv(d: 4, w: 2, h: 3)")
                .expect("ok");
        assert_eq!(as_int(&args[0]), 2);
        assert_eq!(as_int(&args[1]), 3);
        assert_eq!(as_int(&args[2]), 4);
    }

    #[test]
    fn leading_positional_then_named() {
        let args =
            rebound_call_args("fn bv(w: int, h: int, d: int) -> int { w }\nbv(2, d: 4, h: 3)")
                .expect("ok");
        assert_eq!(as_int(&args[0]), 2);
        assert_eq!(as_int(&args[1]), 3);
        assert_eq!(as_int(&args[2]), 4);
    }

    #[test]
    fn omitted_trailing_default_is_filled() {
        let args =
            rebound_call_args("fn f(x: int, y: int = 10) -> int { x }\nf(x: 5)").expect("ok");
        assert_eq!(args.len(), 2);
        assert_eq!(as_int(&args[0]), 5);
        assert_eq!(as_int(&args[1]), 10);
    }

    #[test]
    fn unknown_named_arg_errors() {
        assert!(rebound_call_args("fn bv(w: int, h: int) -> int { w }\nbv(w: 2, bad: 3)").is_err());
    }

    #[test]
    fn duplicate_positional_and_named_errors() {
        assert!(
            rebound_call_args("fn bv(w: int, h: int) -> int { w }\nbv(2, w: 5, h: 3)").is_err()
        );
    }

    #[test]
    fn nested_named_call_in_argument_is_rebound() {
        let mut program = parse_program(
            "fn inner(a: int, b: int) -> int { a }\nfn outer(x: int) -> int { x }\n\
             outer(inner(b: 2, a: 1))",
        )
        .expect("parse");
        rebind_named_args(&mut program).expect("ok");
        for item in &program.items {
            if let Some(Expr::FunctionCall { args, .. }) = top_call(item) {
                if let Expr::FunctionCall {
                    args: inner_args,
                    named_args,
                    ..
                } = &args[0]
                {
                    assert!(named_args.is_empty());
                    assert_eq!(as_int(&inner_args[0]), 1);
                    assert_eq!(as_int(&inner_args[1]), 2);
                    return;
                }
            }
        }
        panic!("nested call not found");
    }

    #[test]
    fn no_named_args_is_untouched() {
        let args = rebound_call_args("fn bv(w: int, h: int) -> int { w }\nbv(2, 3)").expect("ok");
        assert_eq!(as_int(&args[0]), 2);
        assert_eq!(as_int(&args[1]), 3);
    }
}
