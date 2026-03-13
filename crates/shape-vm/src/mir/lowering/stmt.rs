//! Statement lowering: AST statements -> MIR blocks.
//!
//! Handles variable declarations, assignments, control flow (if/while/for),
//! break/continue/return, and pattern destructuring.

use super::expr::*;
use super::helpers::*;
use super::MirBuilder;
use super::BindingMetadata;
use crate::mir::types::*;
use shape_ast::ast::{self, Expr, Span, Spanned, Statement};

// ---------------------------------------------------------------------------
// Statement dispatch
// ---------------------------------------------------------------------------

/// Lower a slice of statements into the current block.
pub(super) fn lower_statements(
    builder: &mut MirBuilder,
    stmts: &[Statement],
    exit_block: BasicBlockId,
) {
    for (idx, stmt) in stmts.iter().enumerate() {
        lower_statement(builder, stmt, exit_block, idx + 1 == stmts.len());
    }
}

/// Lower a single statement.
pub(super) fn lower_statement(
    builder: &mut MirBuilder,
    stmt: &Statement,
    exit_block: BasicBlockId,
    is_last: bool,
) {
    match stmt {
        Statement::VariableDecl(decl, span) => {
            lower_var_decl(builder, decl, *span);
        }
        Statement::Assignment(assign, span) => {
            lower_assignment(builder, assign, *span);
        }
        Statement::Return(value, span) => {
            lower_return_control_flow(builder, value.as_ref(), *span);
        }
        Statement::Expression(expr, span) => {
            if is_last {
                lower_return_control_flow(builder, Some(expr), *span);
            } else {
                // Expression statement — evaluate for side effects
                let _slot = lower_expr_to_temp(builder, expr);
                let _ = span; // span captured in sub-lowering
            }
        }
        Statement::Break(span) => {
            lower_break_control_flow(builder, None, *span);
        }
        Statement::Continue(span) => {
            lower_continue_control_flow(builder, *span);
        }
        Statement::If(if_stmt, span) => {
            lower_if(builder, if_stmt, *span, exit_block);
        }
        Statement::While(while_loop, span) => {
            lower_while(
                builder,
                &while_loop.condition,
                &while_loop.body,
                *span,
                exit_block,
            );
        }
        Statement::For(for_loop, span) => {
            lower_for_loop(builder, for_loop, *span, exit_block);
        }
        Statement::Extend(_, span)
        | Statement::RemoveTarget(span)
        | Statement::SetParamType { span, .. }
        | Statement::SetReturnType { span, .. }
        | Statement::ReplaceBody { span, .. } => {
            builder.push_stmt(StatementKind::Nop, *span);
        }
        Statement::SetParamValue {
            expression, span, ..
        }
        | Statement::SetReturnExpr { expression, span }
        | Statement::ReplaceBodyExpr { expression, span }
        | Statement::ReplaceModuleExpr { expression, span } => {
            let _ = lower_expr_to_temp(builder, expression);
            builder.push_stmt(StatementKind::Nop, *span);
        }
    }
}

// ---------------------------------------------------------------------------
// Variable declarations
// ---------------------------------------------------------------------------

/// Lower a variable declaration.
pub(super) fn lower_var_decl(builder: &mut MirBuilder, decl: &ast::VariableDecl, span: Span) {
    let binding_metadata = match decl.kind {
        ast::VarKind::Const => {
            Some(super::immutable_binding_metadata(span, false, true))
        }
        ast::VarKind::Let if !decl.is_mut => {
            Some(super::immutable_binding_metadata(span, true, false))
        }
        _ => None,
    };
    if let Some(name) = decl.pattern.as_identifier() {
        let type_info = decl
            .value
            .as_ref()
            .map(infer_local_type_from_expr)
            .unwrap_or(LocalTypeInfo::Unknown);
        let slot = if let Some(binding_metadata) = binding_metadata {
            builder.alloc_local_binding(name.to_string(), type_info, binding_metadata)
        } else {
            builder.alloc_local(name.to_string(), type_info)
        };

        if let Some(init_expr) = &decl.value {
            // Determine operand based on ownership modifier
            let operand = match decl.ownership {
                ast::OwnershipModifier::Move => {
                    lower_expr_to_explicit_move_operand(builder, init_expr)
                }
                ast::OwnershipModifier::Clone => {
                    lower_expr_to_operand(builder, init_expr, false)
                }
                ast::OwnershipModifier::Inferred => {
                    // For `var`: decision deferred to liveness analysis
                    // For `let`: default to Move
                    lower_expr_to_operand(builder, init_expr, true)
                }
            };
            let rvalue = match decl.ownership {
                ast::OwnershipModifier::Clone => Rvalue::Clone(operand),
                _ => Rvalue::Use(operand),
            };
            let point =
                builder.push_stmt(StatementKind::Assign(Place::Local(slot), rvalue), span);
            if binding_metadata.is_some() {
                builder.record_binding_initialization(slot, point);
            }
        }
        return;
    }

    let source_place = decl.value.as_ref().map(|init_expr| {
        let type_info = infer_local_type_from_expr(init_expr);
        let source_slot = builder.alloc_temp(type_info);
        let operand = match decl.ownership {
            ast::OwnershipModifier::Move => {
                lower_expr_to_explicit_move_operand(builder, init_expr)
            }
            ast::OwnershipModifier::Clone => lower_expr_to_operand(builder, init_expr, false),
            ast::OwnershipModifier::Inferred => lower_expr_to_operand(builder, init_expr, true),
        };
        let rvalue = match decl.ownership {
            ast::OwnershipModifier::Clone => Rvalue::Clone(operand),
            _ => Rvalue::Use(operand),
        };
        builder.push_stmt(
            StatementKind::Assign(Place::Local(source_slot), rvalue),
            span,
        );
        Place::Local(source_slot)
    });
    lower_destructure_bindings_from_place_opt(
        builder,
        &decl.pattern,
        source_place.as_ref(),
        span,
        binding_metadata,
    );
}

// ---------------------------------------------------------------------------
// Assignments
// ---------------------------------------------------------------------------

/// Lower an assignment statement.
pub(super) fn lower_assignment(builder: &mut MirBuilder, assign: &ast::Assignment, span: Span) {
    if let Some(name) = assign.pattern.as_identifier() {
        let Some(slot) = builder.lookup_local(name) else {
            builder.mark_fallback();
            builder.push_stmt(StatementKind::Nop, span);
            return;
        };
        let value = lower_expr_to_operand(builder, &assign.value, true);
        builder.push_stmt(
            StatementKind::Assign(Place::Local(slot), Rvalue::Use(value)),
            span,
        );
        return;
    }

    let source_slot = lower_expr_to_temp(builder, &assign.value);
    let source_place = Place::Local(source_slot);
    lower_destructure_assignment_from_place(builder, &assign.pattern, &source_place, span);
}

// ---------------------------------------------------------------------------
// Control flow helpers
// ---------------------------------------------------------------------------

pub(super) fn lower_return_control_flow(
    builder: &mut MirBuilder,
    value: Option<&Expr>,
    span: Span,
) {
    if let Some(expr) = value {
        let result = lower_expr_to_operand(builder, expr, true);
        builder.push_stmt(
            StatementKind::Assign(Place::Local(builder.return_slot()), Rvalue::Use(result)),
            expr.span(),
        );
    } else {
        builder.push_stmt(
            StatementKind::Assign(
                Place::Local(builder.return_slot()),
                Rvalue::Use(Operand::Constant(MirConstant::None)),
            ),
            span,
        );
    }
    builder.finish_block(TerminatorKind::Return, span);
    start_dead_block(builder);
}

pub(super) fn lower_break_control_flow(
    builder: &mut MirBuilder,
    value: Option<&Expr>,
    span: Span,
) {
    let Some(loop_ctx) = builder.current_loop() else {
        builder.mark_fallback();
        builder.push_stmt(StatementKind::Nop, span);
        return;
    };

    if let Some(result_slot) = loop_ctx.break_value_slot {
        let rvalue = if let Some(expr) = value {
            Rvalue::Use(lower_expr_to_operand(builder, expr, true))
        } else {
            Rvalue::Use(Operand::Constant(MirConstant::None))
        };
        builder.push_stmt(
            StatementKind::Assign(Place::Local(result_slot), rvalue),
            span,
        );
    } else if let Some(expr) = value {
        let _ = lower_expr_to_temp(builder, expr);
    }

    builder.finish_block(TerminatorKind::Goto(loop_ctx.break_block), span);
    start_dead_block(builder);
}

pub(super) fn lower_continue_control_flow(builder: &mut MirBuilder, span: Span) {
    let Some(loop_ctx) = builder.current_loop() else {
        builder.mark_fallback();
        builder.push_stmt(StatementKind::Nop, span);
        return;
    };

    builder.finish_block(TerminatorKind::Goto(loop_ctx.continue_block), span);
    start_dead_block(builder);
}

// ---------------------------------------------------------------------------
// If statement
// ---------------------------------------------------------------------------

/// Lower an if statement.
fn lower_if(
    builder: &mut MirBuilder,
    if_stmt: &ast::IfStatement,
    span: Span,
    exit_block: BasicBlockId,
) {
    let cond_slot = lower_expr_to_temp(builder, &if_stmt.condition);

    let then_block = builder.new_block();
    let else_block = builder.new_block();
    let merge_block = builder.new_block();

    builder.finish_block(
        TerminatorKind::SwitchBool {
            operand: Operand::Copy(Place::Local(cond_slot)),
            true_bb: then_block,
            false_bb: if if_stmt.else_body.is_some() {
                else_block
            } else {
                merge_block
            },
        },
        span,
    );

    // Then branch
    builder.start_block(then_block);
    builder.push_scope();
    lower_statements(builder, &if_stmt.then_body, exit_block);
    builder.pop_scope();
    builder.finish_block(TerminatorKind::Goto(merge_block), span);

    // Else branch
    if let Some(else_body) = &if_stmt.else_body {
        builder.start_block(else_block);
        builder.push_scope();
        lower_statements(builder, else_body, exit_block);
        builder.pop_scope();
        builder.finish_block(TerminatorKind::Goto(merge_block), span);
    }

    // Continue in merge block
    builder.start_block(merge_block);
}

// ---------------------------------------------------------------------------
// While statement
// ---------------------------------------------------------------------------

/// Lower a while loop.
fn lower_while(
    builder: &mut MirBuilder,
    cond: &Expr,
    body: &[Statement],
    span: Span,
    exit_block: BasicBlockId,
) {
    let header = builder.new_block();
    let body_block = builder.new_block();
    let after = builder.new_block();

    builder.finish_block(TerminatorKind::Goto(header), span);

    // Loop header: evaluate condition
    builder.start_block(header);
    let cond_slot = lower_expr_to_temp(builder, cond);
    builder.finish_block(
        TerminatorKind::SwitchBool {
            operand: Operand::Copy(Place::Local(cond_slot)),
            true_bb: body_block,
            false_bb: after,
        },
        span,
    );

    // Loop body
    builder.start_block(body_block);
    builder.push_loop(after, header, None);
    builder.push_scope();
    lower_statements(builder, body, exit_block);
    builder.pop_scope();
    builder.pop_loop();
    builder.finish_block(TerminatorKind::Goto(header), span);

    // After loop
    builder.start_block(after);
}

// ---------------------------------------------------------------------------
// For loop statement
// ---------------------------------------------------------------------------

/// Lower a for loop (simplified — treats as while with iterator).
fn lower_for_loop(
    builder: &mut MirBuilder,
    for_loop: &ast::ForLoop,
    span: Span,
    exit_block: BasicBlockId,
) {
    match &for_loop.init {
        ast::ForInit::ForIn { pattern, iter } => {
            builder.push_scope();

            let iter_slot = lower_expr_to_temp(builder, iter);
            let pattern_slot = builder.alloc_temp(LocalTypeInfo::Unknown);
            let header = builder.new_block();
            let body_block = builder.new_block();
            let after = builder.new_block();

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
                    Place::Local(pattern_slot),
                    Rvalue::Use(Operand::Constant(MirConstant::None)),
                ),
                span,
            );
            lower_destructure_bindings_from_place(
                builder,
                pattern,
                &Place::Local(pattern_slot),
                span,
                None,
            );
            builder.push_loop(after, header, None);
            builder.push_scope();
            lower_statements(builder, &for_loop.body, exit_block);
            builder.pop_scope();
            builder.pop_loop();
            builder.finish_block(TerminatorKind::Goto(header), span);

            builder.start_block(after);
            builder.pop_scope();
        }
        ast::ForInit::ForC {
            init,
            condition,
            update,
        } => {
            builder.push_scope();
            lower_statement(builder, init, exit_block, false);

            let header = builder.new_block();
            let body_block = builder.new_block();
            let update_block = builder.new_block();
            let after = builder.new_block();

            builder.finish_block(TerminatorKind::Goto(header), span);

            builder.start_block(header);
            let cond_slot = lower_expr_to_temp(builder, condition);
            builder.finish_block(
                TerminatorKind::SwitchBool {
                    operand: Operand::Copy(Place::Local(cond_slot)),
                    true_bb: body_block,
                    false_bb: after,
                },
                span,
            );

            builder.start_block(body_block);
            builder.push_loop(after, update_block, None);
            builder.push_scope();
            lower_statements(builder, &for_loop.body, exit_block);
            builder.pop_scope();
            builder.pop_loop();
            builder.finish_block(TerminatorKind::Goto(update_block), span);

            builder.start_block(update_block);
            let _ = lower_expr_to_temp(builder, update);
            builder.finish_block(TerminatorKind::Goto(header), span);

            builder.start_block(after);
            builder.pop_scope();
        }
    }
}

// ---------------------------------------------------------------------------
// Pattern destructuring
// ---------------------------------------------------------------------------

pub(super) fn pattern_has_bindings(pattern: &ast::Pattern) -> bool {
    match pattern {
        ast::Pattern::Identifier(_) | ast::Pattern::Typed { .. } => true,
        ast::Pattern::Array(patterns) => patterns.iter().any(pattern_has_bindings),
        ast::Pattern::Object(fields) => fields
            .iter()
            .any(|(_, pattern)| pattern_has_bindings(pattern)),
        ast::Pattern::Constructor { fields, .. } => match fields {
            ast::PatternConstructorFields::Unit => false,
            ast::PatternConstructorFields::Tuple(patterns) => {
                patterns.iter().any(pattern_has_bindings)
            }
            ast::PatternConstructorFields::Struct(fields) => fields
                .iter()
                .any(|(_, pattern)| pattern_has_bindings(pattern)),
        },
        ast::Pattern::Literal(_) | ast::Pattern::Wildcard => false,
    }
}

fn lower_constructor_bindings_from_place_opt(
    builder: &mut MirBuilder,
    fields: &ast::PatternConstructorFields,
    source_place: Option<&Place>,
    span: Span,
    binding_metadata: Option<BindingMetadata>,
) {
    match fields {
        ast::PatternConstructorFields::Unit => {}
        ast::PatternConstructorFields::Tuple(patterns) => {
            for (index, pattern) in patterns.iter().enumerate() {
                let projected_place =
                    source_place.map(|source_place| projected_index_place(source_place, index));
                lower_pattern_bindings_from_place_opt(
                    builder,
                    pattern,
                    projected_place.as_ref(),
                    span,
                    binding_metadata,
                );
            }
        }
        ast::PatternConstructorFields::Struct(fields) => {
            for (field_name, pattern) in fields {
                let projected_place = source_place
                    .map(|source_place| projected_field_place(builder, source_place, field_name));
                lower_pattern_bindings_from_place_opt(
                    builder,
                    pattern,
                    projected_place.as_ref(),
                    span,
                    binding_metadata,
                );
            }
        }
    }
}

pub(super) fn lower_destructure_bindings_from_place_opt(
    builder: &mut MirBuilder,
    pattern: &ast::DestructurePattern,
    source_place: Option<&Place>,
    span: Span,
    binding_metadata: Option<BindingMetadata>,
) {
    match pattern {
        ast::DestructurePattern::Identifier(name, _) => {
            let slot = if let Some(binding_metadata) = binding_metadata {
                builder.alloc_local_binding(name.clone(), LocalTypeInfo::Unknown, binding_metadata)
            } else {
                builder.alloc_local(name.clone(), LocalTypeInfo::Unknown)
            };
            if let Some(source_place) = source_place {
                let point = builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(slot),
                        Rvalue::Use(Operand::Copy(source_place.clone())),
                    ),
                    span,
                );
                if binding_metadata.is_some() {
                    builder.record_binding_initialization(slot, point);
                }
            }
        }
        ast::DestructurePattern::Array(patterns) => {
            for (index, pattern) in patterns.iter().enumerate() {
                let projected_place =
                    source_place.map(|source_place| projected_index_place(source_place, index));
                lower_destructure_bindings_from_place_opt(
                    builder,
                    pattern,
                    projected_place.as_ref(),
                    span,
                    binding_metadata,
                );
            }
        }
        ast::DestructurePattern::Object(fields) => {
            for field in fields {
                let projected_place = source_place
                    .map(|source_place| projected_field_place(builder, source_place, &field.key));
                lower_destructure_bindings_from_place_opt(
                    builder,
                    &field.pattern,
                    projected_place.as_ref(),
                    span,
                    binding_metadata,
                );
            }
        }
        ast::DestructurePattern::Rest(pattern) => {
            lower_destructure_bindings_from_place_opt(
                builder,
                pattern,
                source_place,
                span,
                binding_metadata,
            );
        }
        ast::DestructurePattern::Decomposition(bindings) => {
            for binding in bindings {
                let slot = if let Some(binding_metadata) = binding_metadata {
                    builder.alloc_local_binding(
                        binding.name.clone(),
                        LocalTypeInfo::Unknown,
                        binding_metadata,
                    )
                } else {
                    builder.alloc_local(binding.name.clone(), LocalTypeInfo::Unknown)
                };
                if let Some(source_place) = source_place {
                    let point = builder.push_stmt(
                        StatementKind::Assign(
                            Place::Local(slot),
                            Rvalue::Use(Operand::Copy(source_place.clone())),
                        ),
                        span,
                    );
                    if binding_metadata.is_some() {
                        builder.record_binding_initialization(slot, point);
                    }
                }
            }
        }
    }
}

pub(super) fn lower_destructure_bindings_from_place(
    builder: &mut MirBuilder,
    pattern: &ast::DestructurePattern,
    source_place: &Place,
    span: Span,
    binding_metadata: Option<BindingMetadata>,
) {
    lower_destructure_bindings_from_place_opt(
        builder,
        pattern,
        Some(source_place),
        span,
        binding_metadata,
    );
}

pub(super) fn lower_pattern_bindings_from_place_opt(
    builder: &mut MirBuilder,
    pattern: &ast::Pattern,
    source_place: Option<&Place>,
    span: Span,
    binding_metadata: Option<BindingMetadata>,
) {
    match pattern {
        ast::Pattern::Identifier(name) | ast::Pattern::Typed { name, .. } => {
            let slot = if let Some(binding_metadata) = binding_metadata {
                builder.alloc_local_binding(name.clone(), LocalTypeInfo::Unknown, binding_metadata)
            } else {
                builder.alloc_local(name.clone(), LocalTypeInfo::Unknown)
            };
            if let Some(source_place) = source_place {
                let point = builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(slot),
                        Rvalue::Use(Operand::Copy(source_place.clone())),
                    ),
                    span,
                );
                if binding_metadata.is_some() {
                    builder.record_binding_initialization(slot, point);
                }
            }
        }
        ast::Pattern::Array(patterns) => {
            for (index, pattern) in patterns.iter().enumerate() {
                let projected_place =
                    source_place.map(|source_place| projected_index_place(source_place, index));
                lower_pattern_bindings_from_place_opt(
                    builder,
                    pattern,
                    projected_place.as_ref(),
                    span,
                    binding_metadata,
                );
            }
        }
        ast::Pattern::Object(fields) => {
            for (field_name, pattern) in fields {
                let projected_place = source_place
                    .map(|source_place| projected_field_place(builder, source_place, field_name));
                lower_pattern_bindings_from_place_opt(
                    builder,
                    pattern,
                    projected_place.as_ref(),
                    span,
                    binding_metadata,
                );
            }
        }
        ast::Pattern::Constructor { fields, .. } => {
            lower_constructor_bindings_from_place_opt(
                builder,
                fields,
                source_place,
                span,
                binding_metadata,
            );
        }
        ast::Pattern::Wildcard => {}
        ast::Pattern::Literal(_) => {}
    }
}

pub(super) fn lower_pattern_bindings_from_place(
    builder: &mut MirBuilder,
    pattern: &ast::Pattern,
    source_place: &Place,
    span: Span,
    binding_metadata: Option<BindingMetadata>,
) {
    lower_pattern_bindings_from_place_opt(
        builder,
        pattern,
        Some(source_place),
        span,
        binding_metadata,
    );
}

fn lower_destructure_assignment_from_place(
    builder: &mut MirBuilder,
    pattern: &ast::DestructurePattern,
    source_place: &Place,
    span: Span,
) {
    match pattern {
        ast::DestructurePattern::Identifier(name, _) => {
            let Some(slot) = builder.lookup_local(name) else {
                builder.mark_fallback();
                return;
            };
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(slot),
                    Rvalue::Use(Operand::Copy(source_place.clone())),
                ),
                span,
            );
        }
        ast::DestructurePattern::Array(patterns) => {
            for (index, pattern) in patterns.iter().enumerate() {
                let projected_place = projected_index_place(source_place, index);
                lower_destructure_assignment_from_place(builder, pattern, &projected_place, span);
            }
        }
        ast::DestructurePattern::Object(fields) => {
            for field in fields {
                let projected_place = projected_field_place(builder, source_place, &field.key);
                lower_destructure_assignment_from_place(
                    builder,
                    &field.pattern,
                    &projected_place,
                    span,
                );
            }
        }
        ast::DestructurePattern::Rest(pattern) => {
            lower_destructure_assignment_from_place(builder, pattern, source_place, span);
        }
        ast::DestructurePattern::Decomposition(bindings) => {
            for binding in bindings {
                let Some(slot) = builder.lookup_local(&binding.name) else {
                    builder.mark_fallback();
                    return;
                };
                builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(slot),
                        Rvalue::Use(Operand::Copy(source_place.clone())),
                    ),
                    span,
                );
            }
        }
    }
}

// Helper to get span from Statement
pub(super) trait StatementSpan {
    fn span(&self) -> Option<Span>;
}

impl StatementSpan for Statement {
    fn span(&self) -> Option<Span> {
        match self {
            Statement::VariableDecl(_, span) => Some(*span),
            Statement::Assignment(_, span) => Some(*span),
            Statement::Expression(_, span) => Some(*span),
            Statement::Return(_, span) => Some(*span),
            Statement::If(_, span) => Some(*span),
            Statement::While(_, span) => Some(*span),
            Statement::For(_, span) => Some(*span),
            _ => None,
        }
    }
}
