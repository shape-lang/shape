//! ADR-009 E3 (slice S3, legacy class U11): rewrite the `ctx.original(...)`
//! capability access inside a `replace body { … }` replacement into a direct
//! typed call to the compiler-issued HYGIENIC shadow function that holds the
//! pre-annotation body.
//!
//! `ctx.original` is the TYPED capability that replaces the deleted
//! name-encoded `__original__` alias (the `function_aliases["__original__"]`
//! map + the `__original__{fn}` shadow spelling). The role is bound by the
//! capability member (`.original` on the annotation handler's `ctx`), never by
//! a user-guessable global spelling: a user local/function named `__original__`
//! no longer resolves to the shadow (rejection-matrix row 1), and no magic
//! spelling enters a symbol table (row 2 — the shadow's registry name is the
//! unspellable [`crate::compiler::HygienicSymbol`] descriptor).
//!
//! The rewrite runs at directive-application time (inside the `ReplaceBody`
//! comptime directive handler), BEFORE the swapped body reaches MIR lowering
//! and the MIR-derived type-inference pass, so the pre-annotation call is a
//! fully typed `FunctionCall` to the shadow everywhere downstream (the shadow
//! carries the original's exact signature, so `ctx.original(5) + 100` types as
//! ordinary `int` arithmetic — no trait-dispatch fallback).
//!
//! Exhaustive by construction: adding a new `Expr`/`Statement` variant forces a
//! compile error here (the CLAUDE.md Exhaustive Match Rule), so a
//! `ctx.original(...)` embedded in any position is always rewritten — never
//! silently left to fail as an "undefined ctx" reference.

use std::collections::HashSet;

use shape_ast::ast::expr_helpers::{
    AssignExpr, AsyncLetExpr, BlockExpr, BlockItem, ComprehensionClause, ComptimeForExpr, ForExpr,
    FromQueryExpr, IfExpr, JoinBranch, JoinExpr, LetExpr, ListComprehension, LoopExpr, MatchArm,
    MatchExpr, QueryClause, WhileExpr,
};
use shape_ast::ast::expressions::{EnumConstructorPayload, Expr, ObjectEntry};
use shape_ast::ast::patterns::Pattern;
use shape_ast::ast::statements::{ForInit, IfStatement, Statement, WhileLoop};

mod function_expr;

/// True when a `MethodCall` receiver names the annotation handler's `ctx`
/// capability (any bare identifier that is NOT a real binding in the
/// replacement body's own scope) and the member is `original`. The role is
/// bound by the member + the receiver being an ambient capability, never by a
/// fixed magic spelling — the user chooses the handler's `ctx` parameter name.
fn is_original_capability_call(
    receiver: &Expr,
    method: &str,
    named_args: &[(String, Expr)],
    optional: bool,
    bound_receivers: &HashSet<String>,
) -> bool {
    if method != "original" || optional || !named_args.is_empty() {
        return false;
    }
    match receiver {
        // A real binding in scope (a function parameter or `self`) with an
        // `.original()` method is an ordinary method call, never the capability.
        Expr::Identifier(name, _) => !bound_receivers.contains(name),
        _ => false,
    }
}

/// Rewrite every `ctx.original(...)` capability call in `stmts` into a direct
/// call to the hygienic `shadow` function. `bound_receivers` is the set of
/// identifiers that name real bindings in the replacement body's own scope
/// (the target function's parameter names + `self`); a receiver in that set is
/// left as an ordinary method call.
///
/// Scope is threaded lexically: bindings the body itself introduces (`let`
/// locals, closure params, `for`/`match`/comprehension pattern binders, …)
/// extend the set within their region, so a `<local>.original()` is an ordinary
/// method call — never hijacked into the shadow. A local that shadows the
/// ambient `ctx` capability's spelling therefore correctly suppresses the
/// rewrite (role by position/scope, not spelling), and the receiver is never
/// silently dropped.
pub(crate) fn rewrite_original_calls_in_statements(
    stmts: &[Statement],
    bound_receivers: &HashSet<String>,
    shadow: &str,
) -> Vec<Statement> {
    rewrite_statement_seq(stmts, bound_receivers, shadow)
}

/// Rewrite a statement sequence, threading lexical scope through it: a binding
/// introduced by one statement (a `let` local, an `async let` name) is in scope
/// for the statements that follow it, and shadows the ambient `ctx` capability
/// role for those siblings.
fn rewrite_statement_seq(
    stmts: &[Statement],
    bound: &HashSet<String>,
    shadow: &str,
) -> Vec<Statement> {
    let mut scope = bound.clone();
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        // The statement's own initializers are rewritten in the scope BEFORE it
        // binds (`let x = x.original()` — the RHS `x` is the outer binding),
        // then the new binding extends scope for later siblings.
        out.push(rewrite_in_statement(s, &scope, shadow));
        add_statement_bindings(&mut scope, s);
    }
    out
}

/// Extend `scope` with every identifier a statement binds for its later
/// siblings in the same block.
fn add_statement_bindings(scope: &mut HashSet<String>, stmt: &Statement) {
    match stmt {
        Statement::VariableDecl(decl, _) => scope.extend(decl.pattern.get_identifiers()),
        Statement::Expression(expr, _) => add_expr_bindings(scope, expr),
        _ => {}
    }
}

/// Extend `scope` with an identifier an expression-statement binds outward
/// (currently only `async let name = …`, whose handle is visible to siblings).
fn add_expr_bindings(scope: &mut HashSet<String>, expr: &Expr) {
    if let Expr::AsyncLet(async_let, _) = expr {
        scope.insert(async_let.name.clone());
    }
}

/// `bound` extended with `names` — the scope inside a nested binding construct.
fn extended(bound: &HashSet<String>, names: impl IntoIterator<Item = String>) -> HashSet<String> {
    let mut scope = bound.clone();
    scope.extend(names);
    scope
}

/// Every identifier a match/for/let `Pattern` binds (recursively through
/// destructuring), in the order they appear.
fn pattern_binding_names(pattern: &Pattern) -> Vec<String> {
    pattern.get_bindings().into_iter().map(|(n, _)| n).collect()
}

fn rewrite_in_statement(stmt: &Statement, bound: &HashSet<String>, shadow: &str) -> Statement {
    match stmt {
        Statement::Return(expr, span) => Statement::Return(
            expr.as_ref().map(|e| rewrite_in_expr(e, bound, shadow)),
            *span,
        ),
        Statement::Break(span) => Statement::Break(*span),
        Statement::Continue(span) => Statement::Continue(*span),

        Statement::VariableDecl(decl, span) => {
            let mut new_decl = decl.clone();
            new_decl.value = decl.value.as_ref().map(|e| rewrite_in_expr(e, bound, shadow));
            Statement::VariableDecl(new_decl, *span)
        }

        Statement::Assignment(assign, span) => {
            let mut new_assign = assign.clone();
            new_assign.value = rewrite_in_expr(&assign.value, bound, shadow);
            Statement::Assignment(new_assign, *span)
        }

        Statement::Expression(expr, span) => {
            Statement::Expression(rewrite_in_expr(expr, bound, shadow), *span)
        }

        Statement::For(for_loop, span) => {
            let mut new_loop = for_loop.clone();
            // The scope visible inside the loop body: the enclosing scope plus
            // whatever the loop header binds (the `for x in` pattern, or the
            // `for (let i = 0; …)` C-init binding).
            let body_scope;
            new_loop.init = match &for_loop.init {
                ForInit::ForIn { pattern, iter } => {
                    body_scope = extended(bound, pattern.get_identifiers());
                    ForInit::ForIn {
                        pattern: pattern.clone(),
                        iter: rewrite_in_expr(iter, bound, shadow),
                    }
                }
                ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    let new_init = rewrite_in_statement(init, bound, shadow);
                    let mut scope = bound.clone();
                    add_statement_bindings(&mut scope, init);
                    let new_condition = rewrite_in_expr(condition, &scope, shadow);
                    let new_update = rewrite_in_expr(update, &scope, shadow);
                    body_scope = scope;
                    ForInit::ForC {
                        init: Box::new(new_init),
                        condition: new_condition,
                        update: new_update,
                    }
                }
            };
            new_loop.body = rewrite_statement_seq(&for_loop.body, &body_scope, shadow);
            Statement::For(new_loop, *span)
        }

        Statement::While(while_loop, span) => Statement::While(
            WhileLoop {
                condition: rewrite_in_expr(&while_loop.condition, bound, shadow),
                body: rewrite_statement_seq(&while_loop.body, bound, shadow),
            },
            *span,
        ),

        Statement::If(if_stmt, span) => Statement::If(
            IfStatement {
                condition: rewrite_in_expr(&if_stmt.condition, bound, shadow),
                then_body: rewrite_statement_seq(&if_stmt.then_body, bound, shadow),
                else_body: if_stmt
                    .else_body
                    .as_ref()
                    .map(|body| rewrite_statement_seq(body, bound, shadow)),
            },
            *span,
        ),

        // A comptime directive statement cannot appear inside a runtime
        // replacement body (they are only valid in `comptime {}` context), so
        // there is no `ctx.original` to rewrite inside them. Clone verbatim.
        Statement::Extend(_, _)
        | Statement::RemoveTarget(_)
        | Statement::SetParamType { .. }
        | Statement::SetParamTypeExpr { .. }
        | Statement::SetParamValue { .. }
        | Statement::SetReturnType { .. }
        | Statement::SetReturnExpr { .. }
        | Statement::ReplaceBody { .. }
        | Statement::ReplaceBodyExpr { .. }
        | Statement::ReplaceModuleExpr { .. }
        | Statement::ExtendItemsExpr { .. } => stmt.clone(),
    }
}

fn rewrite_in_expr(expr: &Expr, bound: &HashSet<String>, shadow: &str) -> Expr {
    match expr {
        // The capability transform: `<ctx>.original(args)` → `shadow(args)`.
        Expr::MethodCall {
            receiver,
            method,
            args,
            named_args,
            optional,
            span,
        } if is_original_capability_call(receiver, method, named_args, *optional, bound) => {
            Expr::FunctionCall {
                name: shadow.to_string(),
                const_args: Vec::new(),
                args: args.iter().map(|a| rewrite_in_expr(a, bound, shadow)).collect(),
                named_args: Vec::new(),
                span: *span,
            }
        }

        // Leaves with no sub-expressions.
        Expr::Identifier(_, _)
        | Expr::Literal(_, _)
        | Expr::DataRef(_, _)
        | Expr::DataDateTimeRef(_, _)
        | Expr::TimeRef(_, _)
        | Expr::DateTime(_, _)
        | Expr::PatternRef(_, _)
        | Expr::Duration(_, _)
        | Expr::Continue(_)
        | Expr::Unit(_)
        | Expr::TypeSyntax(_, _)
        | Expr::TableRows(_, _)
        | Expr::WindowExpr(_, _) => expr.clone(),

        Expr::DataRelativeAccess {
            reference,
            index,
            span,
        } => Expr::DataRelativeAccess {
            reference: Box::new(rewrite_in_expr(reference, bound, shadow)),
            index: index.clone(),
            span: *span,
        },

        Expr::PropertyAccess {
            object,
            property,
            optional,
            span,
        } => Expr::PropertyAccess {
            object: Box::new(rewrite_in_expr(object, bound, shadow)),
            property: property.clone(),
            optional: *optional,
            span: *span,
        },

        Expr::IndexAccess {
            object,
            index,
            end_index,
            span,
        } => Expr::IndexAccess {
            object: Box::new(rewrite_in_expr(object, bound, shadow)),
            index: Box::new(rewrite_in_expr(index, bound, shadow)),
            end_index: end_index
                .as_ref()
                .map(|e| Box::new(rewrite_in_expr(e, bound, shadow))),
            span: *span,
        },

        Expr::BinaryOp {
            left,
            op,
            right,
            span,
        } => Expr::BinaryOp {
            left: Box::new(rewrite_in_expr(left, bound, shadow)),
            op: op.clone(),
            right: Box::new(rewrite_in_expr(right, bound, shadow)),
            span: *span,
        },

        Expr::FuzzyComparison {
            left,
            op,
            right,
            tolerance,
            span,
        } => Expr::FuzzyComparison {
            left: Box::new(rewrite_in_expr(left, bound, shadow)),
            op: op.clone(),
            right: Box::new(rewrite_in_expr(right, bound, shadow)),
            tolerance: tolerance.clone(),
            span: *span,
        },

        Expr::UnaryOp { op, operand, span } => Expr::UnaryOp {
            op: op.clone(),
            operand: Box::new(rewrite_in_expr(operand, bound, shadow)),
            span: *span,
        },

        Expr::FunctionCall {
            name,
            const_args,
            args,
            named_args,
            span,
        } => Expr::FunctionCall {
            name: name.clone(),
            const_args: const_args
                .iter()
                .map(|a| rewrite_in_expr(a, bound, shadow))
                .collect(),
            args: args.iter().map(|a| rewrite_in_expr(a, bound, shadow)).collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), rewrite_in_expr(v, bound, shadow)))
                .collect(),
            span: *span,
        },

        Expr::QualifiedFunctionCall {
            namespace,
            function,
            const_args,
            args,
            named_args,
            span,
        } => Expr::QualifiedFunctionCall {
            namespace: namespace.clone(),
            function: function.clone(),
            const_args: const_args
                .iter()
                .map(|a| rewrite_in_expr(a, bound, shadow))
                .collect(),
            args: args.iter().map(|a| rewrite_in_expr(a, bound, shadow)).collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), rewrite_in_expr(v, bound, shadow)))
                .collect(),
            span: *span,
        },

        Expr::EnumConstructor {
            enum_name,
            variant,
            payload,
            span,
        } => Expr::EnumConstructor {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            payload: match payload {
                EnumConstructorPayload::Unit => EnumConstructorPayload::Unit,
                EnumConstructorPayload::Tuple(args) => EnumConstructorPayload::Tuple(
                    args.iter().map(|a| rewrite_in_expr(a, bound, shadow)).collect(),
                ),
                EnumConstructorPayload::Struct(fields) => EnumConstructorPayload::Struct(
                    fields
                        .iter()
                        .map(|(k, v)| (k.clone(), rewrite_in_expr(v, bound, shadow)))
                        .collect(),
                ),
            },
            span: *span,
        },

        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            span,
        } => Expr::Conditional {
            condition: Box::new(rewrite_in_expr(condition, bound, shadow)),
            then_expr: Box::new(rewrite_in_expr(then_expr, bound, shadow)),
            else_expr: else_expr
                .as_ref()
                .map(|e| Box::new(rewrite_in_expr(e, bound, shadow))),
            span: *span,
        },

        Expr::Object(entries, span) => Expr::Object(
            entries
                .iter()
                .map(|e| match e {
                    ObjectEntry::Field {
                        key,
                        value,
                        type_annotation,
                    } => ObjectEntry::Field {
                        key: key.clone(),
                        value: rewrite_in_expr(value, bound, shadow),
                        type_annotation: type_annotation.clone(),
                    },
                    ObjectEntry::Spread(inner) => {
                        ObjectEntry::Spread(rewrite_in_expr(inner, bound, shadow))
                    }
                })
                .collect(),
            *span,
        ),

        Expr::Array(items, span) => Expr::Array(
            items.iter().map(|i| rewrite_in_expr(i, bound, shadow)).collect(),
            *span,
        ),

        Expr::ListComprehension(comp, span) => {
            // Clauses bind sequentially: a clause pattern is in scope for its own
            // filter, for later clauses' iterables, and for the element.
            let mut scope = bound.clone();
            let mut clauses = Vec::with_capacity(comp.clauses.len());
            for c in &comp.clauses {
                let iterable = Box::new(rewrite_in_expr(&c.iterable, &scope, shadow));
                scope.extend(c.pattern.get_identifiers());
                let filter = c
                    .filter
                    .as_ref()
                    .map(|f| Box::new(rewrite_in_expr(f, &scope, shadow)));
                clauses.push(ComprehensionClause {
                    pattern: c.pattern.clone(),
                    iterable,
                    filter,
                });
            }
            let element = Box::new(rewrite_in_expr(&comp.element, &scope, shadow));
            Expr::ListComprehension(Box::new(ListComprehension { element, clauses }), *span)
        }

        Expr::Block(block, span) => {
            // Block items are sequential: a `let` local binds for the items that
            // follow it, so thread the scope through the item list.
            let mut scope = bound.clone();
            let mut new_items = Vec::with_capacity(block.items.len());
            for item in &block.items {
                let rewritten = match item {
                    BlockItem::VariableDecl(decl) => {
                        let mut new_decl = decl.clone();
                        new_decl.value =
                            decl.value.as_ref().map(|e| rewrite_in_expr(e, &scope, shadow));
                        BlockItem::VariableDecl(new_decl)
                    }
                    BlockItem::Assignment(assign) => {
                        let mut new_assign = assign.clone();
                        new_assign.value = rewrite_in_expr(&assign.value, &scope, shadow);
                        BlockItem::Assignment(new_assign)
                    }
                    BlockItem::Statement(s) => {
                        BlockItem::Statement(rewrite_in_statement(s, &scope, shadow))
                    }
                    BlockItem::Expression(e) => {
                        BlockItem::Expression(rewrite_in_expr(e, &scope, shadow))
                    }
                };
                match item {
                    BlockItem::VariableDecl(decl) => {
                        scope.extend(decl.pattern.get_identifiers())
                    }
                    BlockItem::Statement(s) => add_statement_bindings(&mut scope, s),
                    BlockItem::Expression(e) => add_expr_bindings(&mut scope, e),
                    BlockItem::Assignment(_) => {}
                }
                new_items.push(rewritten);
            }
            Expr::Block(BlockExpr { items: new_items }, *span)
        }

        Expr::TypeAssertion {
            expr,
            type_annotation,
            meta_param_overrides,
            span,
        } => Expr::TypeAssertion {
            expr: Box::new(rewrite_in_expr(expr, bound, shadow)),
            type_annotation: type_annotation.clone(),
            meta_param_overrides: meta_param_overrides.clone(),
            span: *span,
        },

        Expr::InstanceOf {
            expr,
            type_annotation,
            span,
        } => Expr::InstanceOf {
            expr: Box::new(rewrite_in_expr(expr, bound, shadow)),
            type_annotation: type_annotation.clone(),
            span: *span,
        },

        Expr::FunctionExpr { .. } => function_expr::rewrite(expr, bound, shadow),

        Expr::Spread(inner, span) => {
            Expr::Spread(Box::new(rewrite_in_expr(inner, bound, shadow)), *span)
        }

        Expr::If(if_expr, span) => Expr::If(
            Box::new(IfExpr {
                condition: Box::new(rewrite_in_expr(&if_expr.condition, bound, shadow)),
                then_branch: Box::new(rewrite_in_expr(&if_expr.then_branch, bound, shadow)),
                else_branch: if_expr
                    .else_branch
                    .as_ref()
                    .map(|e| Box::new(rewrite_in_expr(e, bound, shadow))),
            }),
            *span,
        ),

        Expr::While(while_expr, span) => Expr::While(
            Box::new(WhileExpr {
                condition: Box::new(rewrite_in_expr(&while_expr.condition, bound, shadow)),
                body: Box::new(rewrite_in_expr(&while_expr.body, bound, shadow)),
            }),
            *span,
        ),

        Expr::For(for_expr, span) => {
            let body_scope = extended(bound, pattern_binding_names(&for_expr.pattern));
            Expr::For(
                Box::new(ForExpr {
                    pattern: for_expr.pattern.clone(),
                    iterable: Box::new(rewrite_in_expr(&for_expr.iterable, bound, shadow)),
                    body: Box::new(rewrite_in_expr(&for_expr.body, &body_scope, shadow)),
                    is_async: for_expr.is_async,
                }),
                *span,
            )
        }

        Expr::Loop(loop_expr, span) => Expr::Loop(
            Box::new(LoopExpr {
                body: Box::new(rewrite_in_expr(&loop_expr.body, bound, shadow)),
            }),
            *span,
        ),

        Expr::Let(let_expr, span) => {
            // The bound value is evaluated in the enclosing scope; the pattern's
            // binders are in scope only for the `let` body.
            let body_scope = extended(bound, pattern_binding_names(&let_expr.pattern));
            Expr::Let(
                Box::new(LetExpr {
                    pattern: let_expr.pattern.clone(),
                    type_annotation: let_expr.type_annotation.clone(),
                    value: let_expr
                        .value
                        .as_ref()
                        .map(|v| Box::new(rewrite_in_expr(v, bound, shadow))),
                    body: Box::new(rewrite_in_expr(&let_expr.body, &body_scope, shadow)),
                }),
                *span,
            )
        }

        Expr::Assign(assign_expr, span) => Expr::Assign(
            Box::new(AssignExpr {
                target: Box::new(rewrite_in_expr(&assign_expr.target, bound, shadow)),
                value: Box::new(rewrite_in_expr(&assign_expr.value, bound, shadow)),
            }),
            *span,
        ),

        Expr::Break(value, span) => Expr::Break(
            value
                .as_ref()
                .map(|e| Box::new(rewrite_in_expr(e, bound, shadow))),
            *span,
        ),

        Expr::Return(value, span) => Expr::Return(
            value
                .as_ref()
                .map(|e| Box::new(rewrite_in_expr(e, bound, shadow))),
            *span,
        ),

        // Non-capability method call: recurse into receiver + args, keep shape.
        Expr::MethodCall {
            receiver,
            method,
            args,
            named_args,
            optional,
            span,
        } => Expr::MethodCall {
            receiver: Box::new(rewrite_in_expr(receiver, bound, shadow)),
            method: method.clone(),
            args: args.iter().map(|a| rewrite_in_expr(a, bound, shadow)).collect(),
            named_args: named_args
                .iter()
                .map(|(k, v)| (k.clone(), rewrite_in_expr(v, bound, shadow)))
                .collect(),
            optional: *optional,
            span: *span,
        },

        Expr::Match(match_expr, span) => Expr::Match(
            Box::new(MatchExpr {
                scrutinee: Box::new(rewrite_in_expr(&match_expr.scrutinee, bound, shadow)),
                arms: match_expr
                    .arms
                    .iter()
                    .map(|arm| {
                        // The arm pattern's binders are in scope for that arm's
                        // guard and body only.
                        let arm_scope = extended(bound, pattern_binding_names(&arm.pattern));
                        MatchArm {
                            pattern: arm.pattern.clone(),
                            guard: arm
                                .guard
                                .as_ref()
                                .map(|g| Box::new(rewrite_in_expr(g, &arm_scope, shadow))),
                            body: Box::new(rewrite_in_expr(&arm.body, &arm_scope, shadow)),
                            pattern_span: arm.pattern_span,
                        }
                    })
                    .collect(),
            }),
            *span,
        ),

        Expr::Range {
            start,
            end,
            kind,
            span,
        } => Expr::Range {
            start: start
                .as_ref()
                .map(|e| Box::new(rewrite_in_expr(e, bound, shadow))),
            end: end
                .as_ref()
                .map(|e| Box::new(rewrite_in_expr(e, bound, shadow))),
            kind: *kind,
            span: *span,
        },

        Expr::TimeframeContext {
            timeframe,
            expr,
            span,
        } => Expr::TimeframeContext {
            timeframe: timeframe.clone(),
            expr: Box::new(rewrite_in_expr(expr, bound, shadow)),
            span: *span,
        },

        Expr::TryOperator(inner, span) => {
            Expr::TryOperator(Box::new(rewrite_in_expr(inner, bound, shadow)), *span)
        }

        Expr::UsingImpl {
            expr,
            impl_name,
            span,
        } => Expr::UsingImpl {
            expr: Box::new(rewrite_in_expr(expr, bound, shadow)),
            impl_name: impl_name.clone(),
            span: *span,
        },

        Expr::SimulationCall { name, params, span } => Expr::SimulationCall {
            name: name.clone(),
            params: params
                .iter()
                .map(|(k, v)| (k.clone(), rewrite_in_expr(v, bound, shadow)))
                .collect(),
            span: *span,
        },

        Expr::FromQuery(q, span) => {
            let source = Box::new(rewrite_in_expr(&q.source, bound, shadow));
            // The `from <variable> in …` binder is in scope across the clauses
            // and the select; clauses (`let`, `join … into`, `group … into`)
            // introduce further binders sequentially.
            let mut scope = bound.clone();
            scope.insert(q.variable.clone());
            let mut clauses = Vec::with_capacity(q.clauses.len());
            for clause in &q.clauses {
                let new_clause = match clause {
                    QueryClause::Where(e) => {
                        QueryClause::Where(Box::new(rewrite_in_expr(e, &scope, shadow)))
                    }
                    QueryClause::OrderBy(specs) => QueryClause::OrderBy(specs.clone()),
                    QueryClause::GroupBy {
                        element,
                        key,
                        into_var,
                    } => {
                        let new_group = QueryClause::GroupBy {
                            element: Box::new(rewrite_in_expr(element, &scope, shadow)),
                            key: Box::new(rewrite_in_expr(key, &scope, shadow)),
                            into_var: into_var.clone(),
                        };
                        if let Some(v) = into_var {
                            scope.insert(v.clone());
                        }
                        new_group
                    }
                    QueryClause::Join {
                        variable,
                        source,
                        left_key,
                        right_key,
                        into_var,
                    } => {
                        let new_source = Box::new(rewrite_in_expr(source, &scope, shadow));
                        // The join variable is in scope for the key expressions.
                        let key_scope = extended(&scope, std::iter::once(variable.clone()));
                        let new_left = Box::new(rewrite_in_expr(left_key, &key_scope, shadow));
                        let new_right = Box::new(rewrite_in_expr(right_key, &key_scope, shadow));
                        // After the join, the `into` group (or the join variable
                        // itself, for a plain join) is in scope.
                        match into_var {
                            Some(v) => scope.insert(v.clone()),
                            None => scope.insert(variable.clone()),
                        };
                        QueryClause::Join {
                            variable: variable.clone(),
                            source: new_source,
                            left_key: new_left,
                            right_key: new_right,
                            into_var: into_var.clone(),
                        }
                    }
                    QueryClause::Let { variable, value } => {
                        let new_value = Box::new(rewrite_in_expr(value, &scope, shadow));
                        scope.insert(variable.clone());
                        QueryClause::Let {
                            variable: variable.clone(),
                            value: new_value,
                        }
                    }
                };
                clauses.push(new_clause);
            }
            let select = Box::new(rewrite_in_expr(&q.select, &scope, shadow));
            Expr::FromQuery(
                Box::new(FromQueryExpr {
                    variable: q.variable.clone(),
                    source,
                    clauses,
                    select,
                }),
                *span,
            )
        }

        Expr::StructLiteral {
            type_name,
            fields,
            span,
        } => Expr::StructLiteral {
            type_name: type_name.clone(),
            fields: fields
                .iter()
                .map(|(k, v)| (k.clone(), rewrite_in_expr(v, bound, shadow)))
                .collect(),
            span: *span,
        },

        Expr::Await(inner, span) => {
            Expr::Await(Box::new(rewrite_in_expr(inner, bound, shadow)), *span)
        }

        Expr::Join(join, span) => Expr::Join(
            Box::new(JoinExpr {
                kind: join.kind,
                branches: join
                    .branches
                    .iter()
                    .map(|b| JoinBranch {
                        label: b.label.clone(),
                        expr: rewrite_in_expr(&b.expr, bound, shadow),
                        annotations: b.annotations.clone(),
                    })
                    .collect(),
                span: join.span,
            }),
            *span,
        ),

        Expr::Annotated {
            annotation,
            target,
            span,
        } => Expr::Annotated {
            annotation: annotation.clone(),
            target: Box::new(rewrite_in_expr(target, bound, shadow)),
            span: *span,
        },

        Expr::AsyncLet(async_let, span) => Expr::AsyncLet(
            Box::new(AsyncLetExpr {
                name: async_let.name.clone(),
                expr: Box::new(rewrite_in_expr(&async_let.expr, bound, shadow)),
                span: async_let.span,
            }),
            *span,
        ),

        Expr::AsyncScope(inner, span) => {
            Expr::AsyncScope(Box::new(rewrite_in_expr(inner, bound, shadow)), *span)
        }

        Expr::Comptime(stmts, span) => {
            Expr::Comptime(rewrite_statement_seq(stmts, bound, shadow), *span)
        }

        Expr::ComptimeFor(comp_for, span) => {
            // The loop variable and any `some<W…>` witnesses bind in the body.
            let mut names = comp_for.witnesses.clone();
            names.push(comp_for.variable.clone());
            let body_scope = extended(bound, names);
            Expr::ComptimeFor(
                Box::new(ComptimeForExpr {
                    witnesses: comp_for.witnesses.clone(),
                    variable: comp_for.variable.clone(),
                    iterable: Box::new(rewrite_in_expr(&comp_for.iterable, bound, shadow)),
                    body: rewrite_statement_seq(&comp_for.body, &body_scope, shadow),
                }),
                *span,
            )
        }

        Expr::Reference {
            expr,
            is_mutable,
            span,
        } => Expr::Reference {
            expr: Box::new(rewrite_in_expr(expr, bound, shadow)),
            is_mutable: *is_mutable,
            span: *span,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    fn rewrite_body(src: &str, bound: &[&str], shadow: &str) -> Vec<Statement> {
        // Parse a program whose single function's body carries the replacement.
        let program = parse_program(src).expect("parse");
        let body = program
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(f, _) => Some(f.body),
                _ => None,
            })
            .expect("function");
        let bound: HashSet<String> = bound.iter().map(|s| s.to_string()).collect();
        rewrite_original_calls_in_statements(&body, &bound, shadow)
    }

    fn contains_shadow_call(stmts: &[Statement], shadow: &str) -> bool {
        fn expr_has(e: &Expr, shadow: &str) -> bool {
            match e {
                Expr::FunctionCall { name, args, .. } => {
                    name == shadow || args.iter().any(|a| expr_has(a, shadow))
                }
                Expr::BinaryOp { left, right, .. } => {
                    expr_has(left, shadow) || expr_has(right, shadow)
                }
                Expr::Return(Some(inner), _) => expr_has(inner, shadow),
                Expr::MethodCall { receiver, args, .. } => {
                    expr_has(receiver, shadow) || args.iter().any(|a| expr_has(a, shadow))
                }
                _ => false,
            }
        }
        fn stmt_has(s: &Statement, shadow: &str) -> bool {
            match s {
                Statement::Return(Some(e), _) | Statement::Expression(e, _) => expr_has(e, shadow),
                Statement::VariableDecl(d, _) => {
                    d.value.as_ref().is_some_and(|e| expr_has(e, shadow))
                }
                _ => false,
            }
        }
        stmts.iter().any(|s| stmt_has(s, shadow))
    }

    fn contains_original_method(stmts: &[Statement]) -> bool {
        fn expr_has(e: &Expr) -> bool {
            match e {
                Expr::MethodCall {
                    receiver,
                    method,
                    args,
                    ..
                } => {
                    (method == "original" && matches!(receiver.as_ref(), Expr::Identifier(_, _)))
                        || expr_has(receiver)
                        || args.iter().any(expr_has)
                }
                Expr::BinaryOp { left, right, .. } => expr_has(left) || expr_has(right),
                Expr::FunctionCall { args, .. } => args.iter().any(expr_has),
                Expr::Return(Some(inner), _) => expr_has(inner),
                _ => false,
            }
        }
        fn stmt_has(s: &Statement) -> bool {
            match s {
                Statement::Return(Some(e), _) | Statement::Expression(e, _) => expr_has(e),
                Statement::VariableDecl(d, _) => d.value.as_ref().is_some_and(expr_has),
                _ => false,
            }
        }
        stmts.iter().any(stmt_has)
    }

    #[test]
    fn rewrites_ctx_original_inside_binary_op_return() {
        let out = rewrite_body(
            "fn f(x: int) -> int { return ctx.original(5) + 100 }",
            &["x"],
            "\u{1}shadow",
        );
        assert!(contains_shadow_call(&out, "\u{1}shadow"));
        assert!(!contains_original_method(&out));
    }

    #[test]
    fn rewrites_zero_arg_ctx_original() {
        let out = rewrite_body(
            "fn f() -> int { return ctx.original() + 1 }",
            &[],
            "\u{1}shadow",
        );
        assert!(contains_shadow_call(&out, "\u{1}shadow"));
        assert!(!contains_original_method(&out));
    }

    #[test]
    fn rewrite_is_receiver_name_agnostic() {
        // The handler's ctx parameter may be spelled anything; the role is the
        // `.original` member, not a fixed spelling.
        let out = rewrite_body(
            "fn f(x: int) -> int { return c.original(x) }",
            &["x"],
            "\u{1}shadow",
        );
        assert!(contains_shadow_call(&out, "\u{1}shadow"));
    }

    #[test]
    fn does_not_rewrite_original_on_a_real_binding() {
        // `x` is a real parameter binding; `x.original()` is an ordinary method
        // call and must NOT be hijacked into the shadow.
        let out = rewrite_body(
            "fn f(x: int) -> int { return x.original() }",
            &["x"],
            "\u{1}shadow",
        );
        assert!(!contains_shadow_call(&out, "\u{1}shadow"));
        assert!(contains_original_method(&out));
    }

    // Debug-serialization detectors: recurse through EVERY AST form (closures,
    // for/match/comprehension bodies, blocks) that the hand-written helpers
    // above do not, by scanning the derived `Debug` output. The hygienic shadow
    // only ever appears as a `FunctionCall { name: "<shadow>", .. }`, and a
    // surviving capability-shaped call as `MethodCall { .. method: "original" }`.
    fn debug_has_shadow_call(stmts: &[Statement], shadow: &str) -> bool {
        format!("{stmts:?}").contains(&format!("name: {shadow:?}"))
    }
    fn debug_has_original_method(stmts: &[Statement]) -> bool {
        format!("{stmts:?}").contains("method: \"original\"")
    }

    #[test]
    fn does_not_rewrite_original_on_a_body_local_let() {
        // Review finding (E3, major): a body-local `let` binding shadows the
        // ambient `ctx` capability role. `doc.original()` where `doc` is a local
        // is an ordinary method call and must NOT be hijacked into the shadow —
        // the receiver `doc` would be silently dropped otherwise.
        let out = rewrite_body(
            "fn f() -> int { let doc = make(); return doc.original() }",
            &[],
            "\u{1}shadow",
        );
        assert!(!debug_has_shadow_call(&out, "\u{1}shadow"));
        assert!(debug_has_original_method(&out));
    }

    #[test]
    fn does_not_rewrite_original_on_a_closure_param() {
        // A closure parameter is a real binding in the closure body's scope.
        let out = rewrite_body(
            "fn f() { let g = |doc| { return doc.original() }; }",
            &[],
            "\u{1}shadow",
        );
        assert!(!debug_has_shadow_call(&out, "\u{1}shadow"));
        assert!(debug_has_original_method(&out));
    }

    #[test]
    fn does_not_rewrite_original_on_a_for_pattern_binding() {
        // `doc` is bound by the for-loop pattern within the loop body.
        let out = rewrite_body(
            "fn f() { for doc in items { doc.original(); } }",
            &[],
            "\u{1}shadow",
        );
        assert!(!debug_has_shadow_call(&out, "\u{1}shadow"));
        assert!(debug_has_original_method(&out));
    }

    #[test]
    fn does_not_rewrite_original_on_a_match_arm_binding() {
        // `doc` is bound by the match-arm pattern within that arm's body.
        let out = rewrite_body(
            "fn f() -> int { return match v { doc => doc.original() } }",
            &[],
            "\u{1}shadow",
        );
        assert!(!debug_has_shadow_call(&out, "\u{1}shadow"));
        assert!(debug_has_original_method(&out));
    }

    #[test]
    fn binding_scope_does_not_leak_to_a_sibling_statement() {
        // The closure param `doc` is scoped to the closure body only. Once that
        // scope closes, a sibling `doc.original()` sees no `doc` binding, so it
        // IS the ambient capability and MUST rewrite to the shadow — proving the
        // scope threading is lexically precise, not a global accumulation.
        let out = rewrite_body(
            "fn f() -> int { let g = |doc| doc; return doc.original() }",
            &[],
            "\u{1}shadow",
        );
        assert!(debug_has_shadow_call(&out, "\u{1}shadow"));
    }
}

#[cfg(test)]
mod generated_origin_tests;
