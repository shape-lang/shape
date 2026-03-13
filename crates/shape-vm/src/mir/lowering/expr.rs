//! Expression lowering: AST expressions -> MIR temporaries and places.
//!
//! The central function is `lower_expr_to_temp`, which dispatches on
//! `Expr` variants and produces a `SlotId` holding the result. Complex
//! expression forms (conditionals, blocks, match, loops) build their own
//! control-flow subgraphs.

use super::helpers::*;
use super::stmt::{lower_statement, lower_statements, lower_var_decl, StatementSpan};
use super::MirBuilder;
use super::immutable_binding_metadata;
use crate::mir::types::*;
use shape_ast::ast::{self, Expr, Span, Spanned, Statement};
use shape_runtime::closure::EnvironmentAnalyzer;

// ---------------------------------------------------------------------------
// Place resolution
// ---------------------------------------------------------------------------

/// Try to resolve an expression as a MIR place (lvalue).
pub(super) fn lower_expr_to_place(builder: &mut MirBuilder, expr: &Expr) -> Option<Place> {
    match expr {
        Expr::Identifier(name, _) | Expr::PatternRef(name, _) => {
            builder.lookup_local(name).map(Place::Local)
        }
        Expr::PropertyAccess {
            object, property, ..
        } => {
            let base = lower_expr_to_place(builder, object)?;
            Some(Place::Field(Box::new(base), builder.field_idx(property)))
        }
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            if end_index.is_some() {
                return None;
            }
            let base = lower_expr_to_place(builder, object)?;
            let index_operand = lower_expr_to_operand(builder, index, false);
            Some(Place::Index(Box::new(base), Box::new(index_operand)))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Operand lowering
// ---------------------------------------------------------------------------

pub(super) fn lower_expr_to_operand(
    builder: &mut MirBuilder,
    expr: &Expr,
    prefer_move: bool,
) -> Operand {
    if let Some(place) = lower_expr_to_place(builder, expr) {
        let operand = if prefer_move {
            Operand::Move(place)
        } else {
            Operand::Copy(place)
        };
        builder.record_task_boundary_operand(operand.clone());
        operand
    } else {
        let slot = lower_expr_to_temp(builder, expr);
        let place = Place::Local(slot);
        let operand = if prefer_move {
            Operand::Move(place)
        } else {
            Operand::Copy(place)
        };
        builder.record_task_boundary_operand(operand.clone());
        operand
    }
}

pub(super) fn lower_expr_to_explicit_move_operand(
    builder: &mut MirBuilder,
    expr: &Expr,
) -> Operand {
    if let Some(place) = lower_expr_to_place(builder, expr) {
        Operand::MoveExplicit(place)
    } else {
        let slot = lower_expr_to_temp(builder, expr);
        Operand::MoveExplicit(Place::Local(slot))
    }
}

pub(super) fn lower_expr_as_moved_operand(builder: &mut MirBuilder, expr: &Expr) -> Operand {
    if let Some(place) = lower_expr_to_place(builder, expr) {
        let operand = Operand::Move(place);
        builder.record_task_boundary_operand(operand.clone());
        operand
    } else {
        let operand = Operand::Move(Place::Local(lower_expr_to_temp(builder, expr)));
        builder.record_task_boundary_operand(operand.clone());
        operand
    }
}

pub(super) fn lower_exprs_to_aggregate<'a>(
    builder: &mut MirBuilder,
    temp: SlotId,
    exprs: impl IntoIterator<Item = &'a Expr>,
    span: Span,
) {
    let operands = exprs
        .into_iter()
        .map(|expr| lower_expr_as_moved_operand(builder, expr))
        .collect();
    builder.push_stmt(
        StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
        span,
    );
}

pub(super) fn lower_assign_target_place(
    builder: &mut MirBuilder,
    target: &Expr,
) -> Option<Place> {
    match target {
        Expr::Identifier(name, _) => builder.lookup_local(name).map(Place::Local),
        Expr::PropertyAccess { .. } | Expr::IndexAccess { .. } => {
            lower_expr_to_place(builder, target)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Closure / function expression helpers
// ---------------------------------------------------------------------------

fn collect_function_expr_capture_operands(
    builder: &MirBuilder,
    params: &[ast::FunctionParameter],
    body: &[Statement],
) -> Vec<Operand> {
    let proto_def = ast::FunctionDef {
        name: "__mir_closure".to_string(),
        name_span: Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        type_params: None,
        params: params.to_vec(),
        return_type: None,
        body: body.to_vec(),
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let mut captured_vars =
        EnvironmentAnalyzer::analyze_function(&proto_def, &builder.visible_named_locals());
    captured_vars.sort();
    captured_vars.dedup();

    let mut operands = Vec::new();
    for name in captured_vars {
        let Some(slot) = builder.lookup_local(&name) else {
            continue;
        };
        let operand = Operand::Copy(Place::Local(slot));
        if !operands.contains(&operand) {
            operands.push(operand);
        }
    }
    operands
}

fn lower_function_expr(
    builder: &mut MirBuilder,
    params: &[ast::FunctionParameter],
    body: &[Statement],
    temp: SlotId,
    span: Span,
) {
    let captures = collect_function_expr_capture_operands(builder, params, body);
    emit_container_store_if_needed(builder, ContainerStoreKind::Closure, temp, captures, span);
    assign_none(builder, temp, span);
}

// ---------------------------------------------------------------------------
// Specific expression lowering functions
// ---------------------------------------------------------------------------

fn lower_array_expr(builder: &mut MirBuilder, elements: &[Expr], temp: SlotId, span: Span) {
    let operands: Vec<_> = elements
        .iter()
        .map(|expr| lower_expr_as_moved_operand(builder, expr))
        .collect();
    builder.push_stmt(
        StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands.clone())),
        span,
    );
    emit_container_store_if_needed(builder, ContainerStoreKind::Array, temp, operands, span);
}

fn lower_window_function_operands(
    builder: &mut MirBuilder,
    func: &ast::windows::WindowFunction,
    operands: &mut Vec<Operand>,
) {
    use ast::windows::WindowFunction;
    match func {
        WindowFunction::Lag { expr, default, .. }
        | WindowFunction::Lead { expr, default, .. } => {
            operands.push(lower_expr_as_moved_operand(builder, expr));
            if let Some(d) = default {
                operands.push(lower_expr_as_moved_operand(builder, d));
            }
        }
        WindowFunction::FirstValue(e)
        | WindowFunction::LastValue(e)
        | WindowFunction::Sum(e)
        | WindowFunction::Avg(e)
        | WindowFunction::Min(e)
        | WindowFunction::Max(e) => {
            operands.push(lower_expr_as_moved_operand(builder, e));
        }
        WindowFunction::NthValue(e, _) => {
            operands.push(lower_expr_as_moved_operand(builder, e));
        }
        WindowFunction::Count(Some(e)) => {
            operands.push(lower_expr_as_moved_operand(builder, e));
        }
        WindowFunction::RowNumber
        | WindowFunction::Rank
        | WindowFunction::DenseRank
        | WindowFunction::Ntile(_)
        | WindowFunction::Count(None) => {}
    }
}

fn lower_await_expr(builder: &mut MirBuilder, inner: &Expr, temp: SlotId, span: Span) {
    let operand = lower_expr_to_operand(builder, inner, true);
    builder.push_stmt(
        StatementKind::Assign(Place::Local(temp), Rvalue::Use(operand)),
        span,
    );
}

fn lower_async_scope_expr(builder: &mut MirBuilder, inner: &Expr, temp: SlotId, span: Span) {
    builder.async_scope_depth += 1;
    let inner_slot = lower_expr_to_temp(builder, inner);
    builder.async_scope_depth -= 1;
    assign_copy_from_slot(builder, temp, inner_slot, span);
}

fn lower_async_let_expr(
    builder: &mut MirBuilder,
    async_let: &ast::AsyncLetExpr,
    temp: SlotId,
    span: Span,
) {
    builder.push_task_boundary_capture_scope();
    let _ = lower_expr_to_operand(builder, &async_let.expr, true);
    let captures = builder.pop_task_boundary_capture_scope();
    emit_task_boundary_if_needed(builder, captures, async_let.span);

    // async let bindings are immutable — the future must not be overwritten.
    let binding_metadata = immutable_binding_metadata(async_let.span, true, false);
    let future_slot = builder.alloc_local_binding(
        async_let.name.clone(),
        LocalTypeInfo::Unknown,
        binding_metadata,
    );
    let init_point = builder.push_stmt(
        StatementKind::Assign(
            Place::Local(future_slot),
            Rvalue::Use(Operand::Constant(crate::mir::types::MirConstant::None)),
        ),
        async_let.span,
    );
    builder.record_binding_initialization(future_slot, init_point);
    assign_copy_from_slot(builder, temp, future_slot, span);
}

fn lower_join_expr(
    builder: &mut MirBuilder,
    join_expr: &ast::JoinExpr,
    temp: SlotId,
    span: Span,
) {
    if join_expr.branches.is_empty() {
        assign_none(builder, temp, span);
        return;
    }

    // `join all/race/any/settle` is structured concurrency — all branches are
    // joined before the parent scope exits.
    builder.async_scope_depth += 1;
    let mut branch_operands = Vec::with_capacity(join_expr.branches.len());
    for branch in &join_expr.branches {
        builder.push_task_boundary_capture_scope();
        for annotation in &branch.annotations {
            for arg in &annotation.args {
                let _ = lower_expr_to_temp(builder, arg);
            }
        }
        let branch_operand = lower_expr_to_operand(builder, &branch.expr, true);
        let captures = builder.pop_task_boundary_capture_scope();
        emit_task_boundary_if_needed(builder, captures, branch.expr.span());
        branch_operands.push(branch_operand);
    }
    builder.async_scope_depth -= 1;

    builder.push_stmt(
        StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(branch_operands)),
        join_expr.span,
    );
}

fn lower_list_comprehension_expr(
    builder: &mut MirBuilder,
    comp: &ast::ListComprehension,
    temp: SlotId,
    span: Span,
) {
    builder.push_scope();
    for clause in &comp.clauses {
        let _ = lower_expr_to_temp(builder, &clause.iterable);
        let element_slot = builder.alloc_temp(LocalTypeInfo::Unknown);
        assign_none(builder, element_slot, clause.iterable.span());
        super::stmt::lower_destructure_bindings_from_place(
            builder,
            &clause.pattern,
            &Place::Local(element_slot),
            clause.iterable.span(),
            None,
        );
        if let Some(filter) = &clause.filter {
            let _ = lower_expr_to_temp(builder, filter);
        }
    }
    let element_slot = lower_expr_to_temp(builder, &comp.element);
    assign_copy_from_slot(builder, temp, element_slot, span);
    builder.pop_scope();
}

fn lower_from_query_expr(
    builder: &mut MirBuilder,
    from_query: &ast::FromQueryExpr,
    temp: SlotId,
    span: Span,
) {
    builder.push_scope();
    let _ = lower_expr_to_temp(builder, &from_query.source);
    let source_slot = builder.alloc_local(from_query.variable.clone(), LocalTypeInfo::Unknown);
    assign_none(builder, source_slot, from_query.source.span());

    for clause in &from_query.clauses {
        match clause {
            ast::QueryClause::Where(expr) => {
                let _ = lower_expr_to_temp(builder, expr);
            }
            ast::QueryClause::OrderBy(specs) => {
                for spec in specs {
                    let _ = lower_expr_to_temp(builder, &spec.key);
                }
            }
            ast::QueryClause::GroupBy {
                element,
                key,
                into_var,
            } => {
                let _ = lower_expr_to_temp(builder, element);
                let _ = lower_expr_to_temp(builder, key);
                if let Some(into_var) = into_var {
                    let group_slot =
                        builder.alloc_local(into_var.clone(), LocalTypeInfo::Unknown);
                    assign_none(builder, group_slot, key.span());
                }
            }
            ast::QueryClause::Join {
                variable,
                source,
                left_key,
                right_key,
                into_var,
            } => {
                let _ = lower_expr_to_temp(builder, source);
                let join_slot =
                    builder.alloc_local(variable.clone(), LocalTypeInfo::Unknown);
                assign_none(builder, join_slot, source.span());
                let _ = lower_expr_to_temp(builder, left_key);
                let _ = lower_expr_to_temp(builder, right_key);
                if let Some(into_var) = into_var {
                    let into_slot =
                        builder.alloc_local(into_var.clone(), LocalTypeInfo::Unknown);
                    assign_none(builder, into_slot, right_key.span());
                }
            }
            ast::QueryClause::Let { variable, value } => {
                let value_slot = lower_expr_to_temp(builder, value);
                let local_slot =
                    builder.alloc_local(variable.clone(), LocalTypeInfo::Unknown);
                assign_copy_from_slot(builder, local_slot, value_slot, value.span());
            }
        }
    }

    let select_slot = lower_expr_to_temp(builder, &from_query.select);
    assign_copy_from_slot(builder, temp, select_slot, span);
    builder.pop_scope();
}

fn lower_comptime_expr(
    builder: &mut MirBuilder,
    stmts: &[Statement],
    temp: SlotId,
    span: Span,
) {
    builder.push_scope();
    let exit_block = builder.exit_block();
    lower_statements(builder, stmts, exit_block);
    assign_none(builder, temp, span);
    builder.pop_scope();
}

fn lower_comptime_for_expr(
    builder: &mut MirBuilder,
    comptime_for: &ast::ComptimeForExpr,
    temp: SlotId,
    span: Span,
) {
    builder.push_scope();
    let _ = lower_expr_to_temp(builder, &comptime_for.iterable);
    let local_slot =
        builder.alloc_local(comptime_for.variable.clone(), LocalTypeInfo::Unknown);
    assign_none(builder, local_slot, comptime_for.iterable.span());
    let exit_block = builder.exit_block();
    lower_statements(builder, &comptime_for.body, exit_block);
    assign_none(builder, temp, span);
    builder.pop_scope();
}

// ---------------------------------------------------------------------------
// Complex expression lowering (control-flow subgraphs)
// ---------------------------------------------------------------------------

pub(super) fn lower_conditional_expr(
    builder: &mut MirBuilder,
    condition: &Expr,
    then_expr: &Expr,
    else_expr: Option<&Expr>,
    temp: SlotId,
    span: Span,
) {
    let cond_slot = lower_expr_to_temp(builder, condition);
    let then_block = builder.new_block();
    let else_block = builder.new_block();
    let merge_block = builder.new_block();

    builder.finish_block(
        TerminatorKind::SwitchBool {
            operand: Operand::Copy(Place::Local(cond_slot)),
            true_bb: then_block,
            false_bb: else_block,
        },
        span,
    );

    builder.start_block(then_block);
    let then_slot = lower_expr_to_temp(builder, then_expr);
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Copy(Place::Local(then_slot))),
        ),
        then_expr.span(),
    );
    builder.finish_block(TerminatorKind::Goto(merge_block), then_expr.span());

    builder.start_block(else_block);
    if let Some(else_expr) = else_expr {
        let else_slot = lower_expr_to_temp(builder, else_expr);
        builder.push_stmt(
            StatementKind::Assign(
                Place::Local(temp),
                Rvalue::Use(Operand::Copy(Place::Local(else_slot))),
            ),
            else_expr.span(),
        );
        builder.finish_block(TerminatorKind::Goto(merge_block), else_expr.span());
    } else {
        builder.push_stmt(
            StatementKind::Assign(
                Place::Local(temp),
                Rvalue::Use(Operand::Constant(MirConstant::None)),
            ),
            span,
        );
        builder.finish_block(TerminatorKind::Goto(merge_block), span);
    }

    builder.start_block(merge_block);
}

fn lower_block_expr(
    builder: &mut MirBuilder,
    block: &ast::BlockExpr,
    temp: SlotId,
    span: Span,
) {
    builder.push_scope();

    if block.items.is_empty() {
        builder.push_stmt(
            StatementKind::Assign(
                Place::Local(temp),
                Rvalue::Use(Operand::Constant(MirConstant::None)),
            ),
            span,
        );
        builder.pop_scope();
        return;
    }

    let last_idx = block.items.len() - 1;
    for (idx, item) in block.items.iter().enumerate() {
        let is_last = idx == last_idx;
        match item {
            ast::BlockItem::VariableDecl(decl) => {
                lower_var_decl(builder, decl, span);
                if is_last {
                    builder.push_stmt(
                        StatementKind::Assign(
                            Place::Local(temp),
                            Rvalue::Use(Operand::Constant(MirConstant::None)),
                        ),
                        span,
                    );
                }
            }
            ast::BlockItem::Assignment(assign) => {
                super::stmt::lower_assignment(builder, assign, span);
                if is_last {
                    builder.push_stmt(
                        StatementKind::Assign(
                            Place::Local(temp),
                            Rvalue::Use(Operand::Constant(MirConstant::None)),
                        ),
                        span,
                    );
                }
            }
            ast::BlockItem::Expression(expr) => {
                let expr_slot = lower_expr_to_temp(builder, expr);
                if is_last {
                    builder.push_stmt(
                        StatementKind::Assign(
                            Place::Local(temp),
                            Rvalue::Use(Operand::Copy(Place::Local(expr_slot))),
                        ),
                        expr.span(),
                    );
                }
            }
            ast::BlockItem::Statement(stmt) => {
                lower_statement(builder, stmt, builder.exit_block(), false);
                if is_last {
                    builder.push_stmt(
                        StatementKind::Assign(
                            Place::Local(temp),
                            Rvalue::Use(Operand::Constant(MirConstant::None)),
                        ),
                        stmt.span().unwrap_or(span),
                    );
                }
            }
        }
    }

    builder.pop_scope();
}

fn lower_let_expr(
    builder: &mut MirBuilder,
    let_expr: &ast::LetExpr,
    temp: SlotId,
    span: Span,
) {
    builder.push_scope();

    if let Some(name) = let_expr.pattern.as_simple_name() {
        let slot = builder.alloc_local(name.to_string(), LocalTypeInfo::Unknown);
        if let Some(value) = &let_expr.value {
            let operand = lower_expr_to_operand(builder, value, true);
            builder.push_stmt(
                StatementKind::Assign(Place::Local(slot), Rvalue::Use(operand)),
                value.span(),
            );
        } else {
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(slot),
                    Rvalue::Use(Operand::Constant(MirConstant::None)),
                ),
                span,
            );
        }
    } else {
        let source_place = if let Some(value) = &let_expr.value {
            let source_slot = lower_expr_to_temp(builder, value);
            Some(Place::Local(source_slot))
        } else {
            None
        };
        super::stmt::lower_pattern_bindings_from_place_opt(
            builder,
            &let_expr.pattern,
            source_place.as_ref(),
            span,
            Some(immutable_binding_metadata(span, false, false)),
        );
    }

    let body_slot = lower_expr_to_temp(builder, &let_expr.body);
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Copy(Place::Local(body_slot))),
        ),
        let_expr.body.span(),
    );

    builder.pop_scope();
}

fn lower_while_expr(
    builder: &mut MirBuilder,
    while_expr: &ast::WhileExpr,
    temp: SlotId,
    span: Span,
) {
    let header = builder.new_block();
    let body_block = builder.new_block();
    let after = builder.new_block();

    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Constant(MirConstant::None)),
        ),
        span,
    );
    builder.finish_block(TerminatorKind::Goto(header), span);

    builder.start_block(header);
    let cond_slot = lower_expr_to_temp(builder, &while_expr.condition);
    builder.finish_block(
        TerminatorKind::SwitchBool {
            operand: Operand::Copy(Place::Local(cond_slot)),
            true_bb: body_block,
            false_bb: after,
        },
        span,
    );

    builder.start_block(body_block);
    builder.push_loop(after, header, Some(temp));
    let body_slot = lower_expr_to_temp(builder, &while_expr.body);
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Copy(Place::Local(body_slot))),
        ),
        while_expr.body.span(),
    );
    builder.pop_loop();
    builder.finish_block(TerminatorKind::Goto(header), span);

    builder.start_block(after);
}

fn lower_for_expr(
    builder: &mut MirBuilder,
    for_expr: &ast::ForExpr,
    temp: SlotId,
    span: Span,
) {
    builder.push_scope();

    let iter_slot = lower_expr_to_temp(builder, &for_expr.iterable);
    let elem_slot = builder.alloc_temp(LocalTypeInfo::Unknown);
    let header = builder.new_block();
    let body_block = builder.new_block();
    let after = builder.new_block();

    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Constant(MirConstant::None)),
        ),
        span,
    );
    builder.finish_block(TerminatorKind::Goto(header), span);

    builder.start_block(header);
    builder.finish_block(
        TerminatorKind::SwitchBool {
            operand: Operand::Copy(Place::Local(iter_slot)),
            true_bb: body_block,
            false_bb: after,
        },
        span,
    );

    builder.start_block(body_block);
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(elem_slot),
            Rvalue::Use(Operand::Constant(MirConstant::None)),
        ),
        span,
    );
    super::stmt::lower_pattern_bindings_from_place(
        builder,
        &for_expr.pattern,
        &Place::Local(elem_slot),
        span,
        None,
    );
    builder.push_loop(after, header, Some(temp));
    let body_slot = lower_expr_to_temp(builder, &for_expr.body);
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Copy(Place::Local(body_slot))),
        ),
        for_expr.body.span(),
    );
    builder.pop_loop();
    builder.finish_block(TerminatorKind::Goto(header), span);

    builder.start_block(after);
    builder.pop_scope();
}

fn lower_loop_expr(
    builder: &mut MirBuilder,
    loop_expr: &ast::LoopExpr,
    temp: SlotId,
    span: Span,
) {
    let body_block = builder.new_block();
    let after = builder.new_block();

    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Constant(MirConstant::None)),
        ),
        span,
    );
    builder.finish_block(TerminatorKind::Goto(body_block), span);

    builder.start_block(body_block);
    builder.push_loop(after, body_block, Some(temp));
    let body_slot = lower_expr_to_temp(builder, &loop_expr.body);
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Copy(Place::Local(body_slot))),
        ),
        loop_expr.body.span(),
    );
    builder.pop_loop();
    builder.finish_block(TerminatorKind::Goto(body_block), span);

    builder.start_block(after);
}

pub(super) fn lower_match_expr(
    builder: &mut MirBuilder,
    match_expr: &ast::MatchExpr,
    temp: SlotId,
    span: Span,
) {
    if match_expr.arms.is_empty() {
        builder.push_stmt(
            StatementKind::Assign(
                Place::Local(temp),
                Rvalue::Use(Operand::Constant(MirConstant::None)),
            ),
            span,
        );
        return;
    }

    let scrutinee_slot = lower_expr_to_temp(builder, &match_expr.scrutinee);
    let merge_block = builder.new_block();
    let no_match_block = builder.new_block();
    let mut next_test_block = builder.current_block;

    for (idx, arm) in match_expr.arms.iter().enumerate() {
        if idx > 0 {
            builder.start_block(next_test_block);
        }

        let body_block = builder.new_block();
        let next_block = if idx + 1 < match_expr.arms.len() {
            builder.new_block()
        } else {
            no_match_block
        };
        let pattern_span = arm.pattern_span.unwrap_or(span);
        let mut binding_scope_active = false;
        if super::stmt::pattern_has_bindings(&arm.pattern) {
            builder.push_scope();
            binding_scope_active = true;
            super::stmt::lower_pattern_bindings_from_place(
                builder,
                &arm.pattern,
                &Place::Local(scrutinee_slot),
                pattern_span,
                Some(immutable_binding_metadata(pattern_span, false, false)),
            );
        }

        if let Some(pattern_operand) = lower_match_pattern_condition_operand(
            builder,
            &arm.pattern,
            scrutinee_slot,
            pattern_span,
        ) {
            if let Some(guard) = &arm.guard {
                let guard_block = builder.new_block();
                builder.finish_block(
                    TerminatorKind::SwitchBool {
                        operand: pattern_operand,
                        true_bb: guard_block,
                        false_bb: next_block,
                    },
                    pattern_span,
                );
                builder.start_block(guard_block);
                let guard_slot = lower_expr_to_temp(builder, guard);
                builder.finish_block(
                    TerminatorKind::SwitchBool {
                        operand: Operand::Copy(Place::Local(guard_slot)),
                        true_bb: body_block,
                        false_bb: next_block,
                    },
                    guard.span(),
                );
            } else {
                builder.finish_block(
                    TerminatorKind::SwitchBool {
                        operand: pattern_operand,
                        true_bb: body_block,
                        false_bb: next_block,
                    },
                    pattern_span,
                );
            }
        } else if let Some(guard) = &arm.guard {
            let guard_slot = lower_expr_to_temp(builder, guard);
            builder.finish_block(
                TerminatorKind::SwitchBool {
                    operand: Operand::Copy(Place::Local(guard_slot)),
                    true_bb: body_block,
                    false_bb: next_block,
                },
                guard.span(),
            );
        } else {
            builder.finish_block(TerminatorKind::Goto(body_block), pattern_span);
        }

        builder.start_block(body_block);
        let body_slot = lower_expr_to_temp(builder, &arm.body);
        builder.push_stmt(
            StatementKind::Assign(
                Place::Local(temp),
                Rvalue::Use(Operand::Copy(Place::Local(body_slot))),
            ),
            arm.body.span(),
        );
        builder.finish_block(TerminatorKind::Goto(merge_block), arm.body.span());

        if binding_scope_active {
            builder.pop_scope();
        }
        next_test_block = next_block;
    }

    builder.start_block(no_match_block);
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(temp),
            Rvalue::Use(Operand::Constant(MirConstant::None)),
        ),
        span,
    );
    builder.finish_block(TerminatorKind::Goto(merge_block), span);

    builder.start_block(merge_block);
}

fn lower_match_pattern_condition_operand(
    builder: &mut MirBuilder,
    pattern: &ast::Pattern,
    scrutinee_slot: SlotId,
    pattern_span: Span,
) -> Option<Operand> {
    match pattern {
        ast::Pattern::Identifier(_) | ast::Pattern::Typed { .. } | ast::Pattern::Wildcard => None,
        ast::Pattern::Literal(literal) => {
            let literal_expr = Expr::Literal(literal.clone(), pattern_span);
            let literal_operand = lower_expr_to_operand(builder, &literal_expr, false);
            let matches_slot = builder.alloc_temp(LocalTypeInfo::Copy);
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(matches_slot),
                    Rvalue::BinaryOp(
                        BinOp::Eq,
                        Operand::Copy(Place::Local(scrutinee_slot)),
                        literal_operand,
                    ),
                ),
                pattern_span,
            );
            Some(Operand::Copy(Place::Local(matches_slot)))
        }
        ast::Pattern::Array(_) | ast::Pattern::Object(_) | ast::Pattern::Constructor { .. } => {
            Some(Operand::Copy(Place::Local(scrutinee_slot)))
        }
    }
}

// ---------------------------------------------------------------------------
// Main expression dispatch
// ---------------------------------------------------------------------------

/// Lower an expression into a temporary slot.
///
/// This is the main expression dispatch: each `Expr` variant is matched and
/// lowered into one or more MIR statements, with the result placed into a
/// freshly-allocated temporary slot.
pub(crate) fn lower_expr_to_temp(builder: &mut MirBuilder, expr: &Expr) -> SlotId {
    let span = expr.span();
    let temp = builder.alloc_temp(LocalTypeInfo::Unknown);

    match expr {
        Expr::Literal(_, _)
        | Expr::DataRef(_, _)
        | Expr::DataDateTimeRef(_, _)
        | Expr::TimeRef(_, _)
        | Expr::DateTime(_, _)
        | Expr::Duration(_, _)
        | Expr::Unit(_) => {
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
                ),
                span,
            );
        }
        Expr::Identifier(name, _) => {
            let operand = builder
                .lookup_local(name)
                .map(Place::Local)
                .map(Operand::Copy)
                .unwrap_or(Operand::Constant(MirConstant::None));
            builder.record_task_boundary_operand(operand.clone());
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Use(operand)),
                span,
            );
        }
        Expr::PatternRef(name, _) => {
            let operand = builder
                .lookup_local(name)
                .map(Place::Local)
                .map(Operand::Copy)
                .unwrap_or(Operand::Constant(MirConstant::None));
            builder.record_task_boundary_operand(operand.clone());
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Use(operand)),
                span,
            );
        }
        Expr::PropertyAccess { object, .. } => {
            if let Some(place) = lower_expr_to_place(builder, expr) {
                builder.record_task_boundary_operand(Operand::Copy(place.clone()));
                assign_copy_from_place(builder, temp, place, span);
            } else {
                lower_exprs_to_aggregate(builder, temp, [object.as_ref()], span);
            }
        }
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            if let Some(place) = lower_expr_to_place(builder, expr) {
                builder.record_task_boundary_operand(Operand::Copy(place.clone()));
                assign_copy_from_place(builder, temp, place, span);
            } else {
                let mut operands = vec![
                    lower_expr_as_moved_operand(builder, object),
                    lower_expr_as_moved_operand(builder, index),
                ];
                if let Some(end_index) = end_index {
                    operands.push(lower_expr_as_moved_operand(builder, end_index));
                }
                builder.push_stmt(
                    StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
                    span,
                );
            }
        }
        Expr::DataRelativeAccess { reference, .. } => {
            lower_exprs_to_aggregate(builder, temp, [reference.as_ref()], span);
        }
        Expr::Reference {
            expr: inner,
            is_mutable,
            span: ref_span,
        } => {
            let kind = if *is_mutable {
                BorrowKind::Exclusive
            } else {
                BorrowKind::Shared
            };
            let borrowed_place = if let Some(place) = lower_expr_to_place(builder, inner) {
                place
            } else {
                builder.mark_fallback();
                Place::Local(lower_expr_to_temp(builder, inner))
            };
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::Borrow(kind, borrowed_place.clone()),
                ),
                *ref_span,
            );
            builder.record_task_boundary_reference_capture(temp, &borrowed_place);
        }
        Expr::UnaryOp { op, operand, .. } => {
            let operand = lower_expr_to_operand(builder, operand, false);
            if let Some(op) = lower_unary_op(*op) {
                builder.push_stmt(
                    StatementKind::Assign(Place::Local(temp), Rvalue::UnaryOp(op, operand)),
                    span,
                );
            } else {
                builder.push_stmt(
                    StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(vec![operand])),
                    span,
                );
            }
        }
        Expr::Assign(assign, _) => {
            let Some(target_place) = lower_assign_target_place(builder, &assign.target) else {
                builder.mark_fallback();
                assign_none(builder, temp, span);
                return temp;
            };
            let value_slot = lower_expr_to_temp(builder, &assign.value);
            builder.push_stmt(
                StatementKind::Assign(
                    target_place.clone(),
                    Rvalue::Use(Operand::Move(Place::Local(value_slot))),
                ),
                span,
            );
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Use(Operand::Copy(target_place))),
                span,
            );
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            lower_conditional_expr(
                builder,
                condition,
                then_expr,
                else_expr.as_deref(),
                temp,
                span,
            );
        }
        Expr::If(if_expr, _) => {
            lower_conditional_expr(
                builder,
                &if_expr.condition,
                &if_expr.then_branch,
                if_expr.else_branch.as_deref(),
                temp,
                span,
            );
        }
        Expr::Block(block, _) => {
            lower_block_expr(builder, block, temp, span);
        }
        Expr::Let(let_expr, _) => {
            lower_let_expr(builder, let_expr, temp, span);
        }
        Expr::While(while_expr, _) => {
            lower_while_expr(builder, while_expr, temp, span);
        }
        Expr::For(for_expr, _) => {
            lower_for_expr(builder, for_expr, temp, span);
        }
        Expr::Loop(loop_expr, _) => {
            lower_loop_expr(builder, loop_expr, temp, span);
        }
        Expr::Match(match_expr, _) => {
            lower_match_expr(builder, match_expr, temp, span);
        }
        Expr::BinaryOp {
            left, op, right, ..
        } => {
            let l = lower_expr_to_operand(builder, left, false);
            let r = lower_expr_to_operand(builder, right, false);
            if let Some(op) = lower_binary_op(*op) {
                builder.push_stmt(
                    StatementKind::Assign(Place::Local(temp), Rvalue::BinaryOp(op, l, r)),
                    span,
                );
            } else {
                builder.push_stmt(
                    StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(vec![l, r])),
                    span,
                );
            }
        }
        Expr::FuzzyComparison {
            left, op, right, ..
        } => {
            let l = lower_expr_to_operand(builder, left, false);
            let r = lower_expr_to_operand(builder, right, false);
            let mir_op = match op {
                ast::operators::FuzzyOp::Equal => BinOp::Eq,
                ast::operators::FuzzyOp::Greater => BinOp::Gt,
                ast::operators::FuzzyOp::Less => BinOp::Lt,
            };
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::BinaryOp(mir_op, l, r)),
                span,
            );
        }
        Expr::Break(value, _) => {
            super::stmt::lower_break_control_flow(builder, value.as_deref(), span);
            assign_none(builder, temp, span);
        }
        Expr::Continue(_) => {
            super::stmt::lower_continue_control_flow(builder, span);
            assign_none(builder, temp, span);
        }
        Expr::Return(value, _) => {
            super::stmt::lower_return_control_flow(builder, value.as_deref(), span);
            assign_none(builder, temp, span);
        }
        Expr::FunctionCall {
            name,
            args,
            named_args,
            ..
        } => {
            let mut arg_ops = Vec::with_capacity(args.len() + named_args.len());
            arg_ops.extend(
                args.iter()
                    .map(|arg| lower_expr_as_moved_operand(builder, arg)),
            );
            arg_ops.extend(
                named_args
                    .iter()
                    .map(|(_, expr)| lower_expr_as_moved_operand(builder, expr)),
            );
            let func_op = Operand::Constant(MirConstant::Function(name.clone()));
            builder.emit_call(func_op, arg_ops, Place::Local(temp), span);
        }
        Expr::QualifiedFunctionCall {
            namespace,
            function,
            args,
            named_args,
            ..
        } => {
            let mut arg_ops = Vec::with_capacity(args.len() + named_args.len());
            arg_ops.extend(
                args.iter()
                    .map(|arg| lower_expr_as_moved_operand(builder, arg)),
            );
            arg_ops.extend(
                named_args
                    .iter()
                    .map(|(_, expr)| lower_expr_as_moved_operand(builder, expr)),
            );
            let func_op = Operand::Constant(MirConstant::Function(format!(
                "{}::{}",
                namespace, function
            )));
            builder.emit_call(func_op, arg_ops, Place::Local(temp), span);
        }
        Expr::EnumConstructor { payload, .. } => match payload {
            ast::EnumConstructorPayload::Unit => {
                assign_none(builder, temp, span);
            }
            ast::EnumConstructorPayload::Tuple(values) => {
                let operands: Vec<_> = values
                    .iter()
                    .map(|expr| lower_expr_as_moved_operand(builder, expr))
                    .collect();
                builder.push_stmt(
                    StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands.clone())),
                    span,
                );
                emit_container_store_if_needed(
                    builder,
                    ContainerStoreKind::Enum,
                    temp,
                    operands,
                    span,
                );
            }
            ast::EnumConstructorPayload::Struct(fields) => {
                let operands: Vec<_> = fields
                    .iter()
                    .map(|(_, expr)| lower_expr_as_moved_operand(builder, expr))
                    .collect();
                builder.push_stmt(
                    StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands.clone())),
                    span,
                );
                emit_container_store_if_needed(
                    builder,
                    ContainerStoreKind::Enum,
                    temp,
                    operands,
                    span,
                );
            }
        },
        Expr::Object(entries, _) => {
            let mut operands = Vec::new();
            for entry in entries {
                match entry {
                    ast::ObjectEntry::Field { value, .. } => {
                        operands.push(lower_expr_as_moved_operand(builder, value));
                    }
                    ast::ObjectEntry::Spread(expr) => {
                        operands.push(lower_expr_as_moved_operand(builder, expr));
                    }
                }
            }
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands.clone())),
                span,
            );
            emit_container_store_if_needed(
                builder,
                ContainerStoreKind::Object,
                temp,
                operands,
                span,
            );
        }
        Expr::Array(elements, _) => {
            lower_array_expr(builder, elements, temp, span);
        }
        Expr::ListComprehension(comp, _) => {
            lower_list_comprehension_expr(builder, comp, temp, span);
        }
        Expr::TypeAssertion {
            expr,
            meta_param_overrides,
            ..
        } => {
            let mut operands = vec![lower_expr_as_moved_operand(builder, expr)];
            if let Some(overrides) = meta_param_overrides {
                let mut keys: Vec<_> = overrides.keys().cloned().collect();
                keys.sort();
                for key in keys {
                    if let Some(value) = overrides.get(&key) {
                        operands.push(lower_expr_as_moved_operand(builder, value));
                    }
                }
            }
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
                span,
            );
        }
        Expr::InstanceOf { expr, .. } => {
            lower_exprs_to_aggregate(builder, temp, [expr.as_ref()], span);
        }
        Expr::FunctionExpr { params, body, .. } => {
            lower_function_expr(builder, params, body, temp, span);
        }
        Expr::Spread(expr, _) => {
            let expr_slot = lower_expr_to_temp(builder, expr);
            assign_copy_from_slot(builder, temp, expr_slot, span);
        }
        Expr::MethodCall {
            receiver,
            method,
            args,
            named_args,
            ..
        } => {
            let receiver_op = lower_expr_as_moved_operand(builder, receiver);
            let mut arg_ops = Vec::with_capacity(1 + args.len() + named_args.len());
            arg_ops.push(receiver_op);
            arg_ops.extend(
                args.iter()
                    .map(|arg| lower_expr_as_moved_operand(builder, arg)),
            );
            arg_ops.extend(
                named_args
                    .iter()
                    .map(|(_, expr)| lower_expr_as_moved_operand(builder, expr)),
            );
            let func_op = Operand::Constant(MirConstant::Method(method.clone()));
            builder.emit_call(func_op, arg_ops, Place::Local(temp), span);
        }
        Expr::Range { start, end, .. } => {
            let mut operands = Vec::new();
            if let Some(start) = start {
                operands.push(lower_expr_as_moved_operand(builder, start));
            }
            if let Some(end) = end {
                operands.push(lower_expr_as_moved_operand(builder, end));
            }
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
                span,
            );
        }
        Expr::TimeframeContext { expr, .. }
        | Expr::TryOperator(expr, _)
        | Expr::UsingImpl { expr, .. } => {
            let expr_slot = lower_expr_to_temp(builder, expr);
            assign_copy_from_slot(builder, temp, expr_slot, span);
        }
        Expr::SimulationCall { params, .. } => {
            lower_exprs_to_aggregate(builder, temp, params.iter().map(|(_, expr)| expr), span);
        }
        Expr::StructLiteral { fields, .. } => {
            let operands: Vec<_> = fields
                .iter()
                .map(|(_, expr)| lower_expr_as_moved_operand(builder, expr))
                .collect();
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands.clone())),
                span,
            );
            emit_container_store_if_needed(
                builder,
                ContainerStoreKind::Object,
                temp,
                operands,
                span,
            );
        }
        Expr::Annotated {
            annotation, target, ..
        } => {
            let mut operands = Vec::with_capacity(annotation.args.len() + 1);
            operands.extend(
                annotation
                    .args
                    .iter()
                    .map(|expr| lower_expr_as_moved_operand(builder, expr)),
            );
            operands.push(lower_expr_as_moved_operand(builder, target));
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
                span,
            );
        }
        Expr::TableRows(rows, _) => {
            let mut operands = Vec::new();
            for row in rows {
                operands.extend(
                    row.iter()
                        .map(|expr| lower_expr_as_moved_operand(builder, expr)),
                );
            }
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
                span,
            );
        }
        Expr::Await(inner, _) => {
            lower_await_expr(builder, inner, temp, span);
        }
        Expr::Join(join_expr, _) => {
            lower_join_expr(builder, join_expr, temp, span);
        }
        Expr::AsyncLet(async_let, _) => {
            lower_async_let_expr(builder, async_let, temp, span);
        }
        Expr::AsyncScope(inner, _) => {
            lower_async_scope_expr(builder, inner, temp, span);
        }
        Expr::FromQuery(from_query, _) => {
            lower_from_query_expr(builder, from_query, temp, span);
        }
        Expr::Comptime(stmts, _) => {
            lower_comptime_expr(builder, stmts, temp, span);
        }
        Expr::ComptimeFor(comptime_for, _) => {
            lower_comptime_for_expr(builder, comptime_for, temp, span);
        }
        Expr::WindowExpr(window_expr, _) => {
            // Lower window expressions as an aggregate of their sub-expressions.
            // The borrow solver only needs to track which slots are read.
            let mut operands = Vec::new();
            lower_window_function_operands(builder, &window_expr.function, &mut operands);
            for expr in &window_expr.over.partition_by {
                operands.push(lower_expr_as_moved_operand(builder, expr));
            }
            if let Some(order_by) = &window_expr.over.order_by {
                for (expr, _) in &order_by.columns {
                    operands.push(lower_expr_as_moved_operand(builder, expr));
                }
            }
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
                span,
            );
        }
    }

    temp
}
