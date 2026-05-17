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

        // ADR-006 §2.7.5 stamp-at-compile-time — V3-S6e-jit-specialized-
        // vec-map-aggregate-classify (Phase 3 cluster-0+1 Wave 3, 2026-
        // 05-16; V3-S6 multi-session chain checkpoint-final).
        //
        // For `let mut <name>: Array<C> = []` bindings (where the
        // annotation resolves to `ConcreteType::Array(elem)`), record the
        // empty-typed-array element ConcreteType against the binding slot
        // so the conduit producer at `compiler/helpers.rs::infer_top_
        // level_concrete_types_from_mir_with_resolvers` can stamp
        // `concrete_types[slot] = Array(elem)`. Empty array literals
        // short-circuit at `mir/lowering/helpers.rs::emit_container_
        // store_if_needed` (line 128-130) — no `StatementKind::ArrayStore`
        // is emitted, so the conduit producer's ArrayStore walker at
        // `compiler/helpers.rs:687` has no operand source to infer the
        // element kind from. Without an explicit binding-slot stamp the
        // slot's `concrete_types[slot]` stays `Void`, the JIT-MIR v2-
        // fast-path at `mir_compiler/statements.rs::v2_typed_array_
        // elem_kind` returns `None`, and the kind-blind Aggregate fall-
        // back fires per Route A `W11-jit-new-array` SURFACE.
        //
        // V3-S6a's `synthesize_empty_array_result_annotation` writes the
        // `Array<C>` annotation onto the specialized `Vec.map<U>` /
        // `Vec.filter<U>` body's `let mut result = []` after generic
        // substitution concretizes the return type. The conduit chain:
        // V3-S6a annotation → MIR `local_typed_array_element_types` →
        // conduit producer stamps `concrete_types[result_slot] =
        // Array(elem)` → JIT-MIR v2-fast-path activates →
        // `emit_v2_array_aggregate` succeeds → specialized body JIT-
        // compiles → V3-S6c routing's direct FuncRef call returns
        // correct raw `*const TypedArray<i64>` bits per V3-S5.
        //
        // cluster-2-closure-wave-B-class-bc-coverage (Phase 3 cluster-2
        // Round 2, 2026-05-16): widen the empty-literal gate to ALL
        // typed-array-annotated bindings (`let doubled: Array<int> =
        // xs.map(...)` etc.). The empty-literal-only gate covered the
        // V3-S6a-synthesized `let mut result = []` inside specialized
        // `Vec.map<U>` bodies; widening to non-empty initializers covers
        // any user-written `let x: Array<C> = <expr>` whose RHS is a
        // method call, function call, or other non-literal producing
        // a typed-array. The annotation IS the proof per §2.7.5 stamp-
        // at-compile-time discipline — no inference, no decode, no
        // fabricated default. Initializer-shape-independent: stamps from
        // the annotation directly, regardless of what the RHS expression
        // is. Honors §2.7.7 #9 — if no annotation exists the slot stays
        // unstamped (surface-and-stop at the JIT-MIR consumer for Class B
        // and Class C inferred cases; tracked as gap below).
        let annotated_array_elem: Option<shape_value::v2::ConcreteType> =
            decl.type_annotation.as_ref().and_then(|annotation| {
                crate::compiler::v2_map_emission::concrete_type_from_annotation(annotation)
                    .and_then(|ct| match ct {
                        shape_value::v2::ConcreteType::Array(elem) => Some(*elem),
                        _ => None,
                    })
            });
        let empty_array_elem: Option<shape_value::v2::ConcreteType> =
            if let (Some(_annotation), Some(Expr::Array(items, _))) =
                (decl.type_annotation.as_ref(), decl.value.as_ref())
            {
                if items.is_empty() {
                    annotated_array_elem.clone()
                } else {
                    None
                }
            } else {
                None
            };
        if let Some(elem) = annotated_array_elem.clone() {
            builder.record_local_typed_array_element_type(slot, elem);
        }

        if let Some(init_expr) = &decl.value {
            // ADR-006 §2.7.27 / W17-mutation-writeback: when the initializer
            // is a recognized COW-container ctor (`Set()` / `HashMap()` /
            // `Deque()` / `PriorityQueue()`), record the binding slot's
            // container kind so a subsequent `s.add(...)` method call can
            // emit the `Assign(receiver, Use(Move(temp)))` write-back. This
            // mirrors the bytecode compiler's `mut_self_container_locals`
            // tracking (see `compiler/statements.rs:4707` and
            // `compiler/expressions/function_calls.rs:967`).
            if let Expr::FunctionCall { name: ctor_name, .. } = init_expr {
                if let Some(kind) =
                    crate::compiler::mutation_writeback::ContainerKind::from_ctor_name(
                        ctor_name,
                    )
                {
                    builder.record_mut_self_container_local(slot, kind);
                }
            }
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
            // ADR-006 §2.7.5 stamp-at-compile-time — V3-S6e classification
            // chain (continued from the `empty_array_elem` capture above).
            // When the initializer was an empty `Array<C>` literal,
            // `lower_expr_to_operand` returned `Move(Local(temp))` where
            // `temp` is the scratch slot the empty-Aggregate Assign was
            // emitted against. Stamp the temp slot's element type so the
            // conduit producer reaches BOTH the binding slot AND its
            // source temp — the JIT v2-fast-path at
            // `mir_compiler/statements.rs:24` consumes
            // `concrete_types[place]` on the EMPTY-AGGREGATE `Assign(temp,
            // Aggregate(vec![]))` site (the temp is the destination there;
            // the binding slot only becomes the destination at the next
            // `Assign(binding, Use(Move(temp)))` Move stmt).
            //
            // Scoped to the exact temp-slot operand shape (`Move|Copy|
            // MoveExplicit of Place::Local(temp)`) so it does not over-
            // stamp unrelated temps. This pattern is the canonical
            // `lower_expr_to_operand` -> `lower_expr_to_temp` output for
            // non-place expressions.
            if let Some(elem) = empty_array_elem.clone() {
                use crate::mir::types::{Place as MirPlace, Operand as MirOperand};
                if let MirOperand::Move(MirPlace::Local(temp))
                | MirOperand::Copy(MirPlace::Local(temp))
                | MirOperand::MoveExplicit(MirPlace::Local(temp)) = &operand
                {
                    builder.record_local_typed_array_element_type(*temp, elem);
                }
            }
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
            // Per cluster-2-closure-wave-1-iter-statemachine (2026-05-16):
            // index-counter state machine mirror of the `lower_for_expr`
            // generic-iterator branch at `expr.rs:1035`. The
            // `Statement::For` arm is reachable for `for x in iter { ... }`
            // statement-form for-loops (parser emits `Rule::for_loop` →
            // `Statement::For` per `crates/shape-ast/src/parser/statements
            // .rs:63`); the expression-form `for x in iter { ... }`
            // routes through `Statement::Expression(Expr::For(_, _))` →
            // `lower_for_expr`. Both shapes share the same broken-stub
            // class per empirical-verification §9 Q1, so the fix lands at
            // both sites to forestall the parallel-implementation
            // defection-attractor pattern (CLAUDE.md §Parallel-
            // implementation across producer/consumer carrier-shape
            // boundaries).
            //
            // Lowering shape: see `expr.rs::lower_for_expr` generic-
            // iterator branch for the full design comment. The
            // statement-form path differs only in that the for-loop
            // produces no result value (no `temp` slot), so we omit the
            // `temp = none` / per-iteration `temp = body_slot` writes.
            builder.push_scope();

            let iter_slot = lower_expr_to_temp(builder, iter);

            let idx_slot = builder.alloc_temp(LocalTypeInfo::Copy);
            let len_slot = builder.alloc_temp(LocalTypeInfo::Copy);

            // __idx = 0
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(idx_slot),
                    Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
                ),
                span,
            );

            // __len = iter_slot.len()
            let len_call_func =
                Operand::Constant(MirConstant::Method("len".to_string()));
            builder.emit_call(
                len_call_func,
                vec![Operand::Copy(Place::Local(iter_slot))],
                Place::Local(len_slot),
                span,
            );

            let header = builder.new_block();
            let body_block = builder.new_block();
            let after = builder.new_block();

            builder.finish_block(TerminatorKind::Goto(header), span);

            // Loop header: __cond = __idx < __len; switchbool __cond.
            builder.start_block(header);
            let cond_slot = builder.alloc_temp(LocalTypeInfo::Copy);
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(cond_slot),
                    Rvalue::BinaryOp(
                        BinOp::Lt,
                        Operand::Copy(Place::Local(idx_slot)),
                        Operand::Copy(Place::Local(len_slot)),
                    ),
                ),
                span,
            );
            builder.finish_block(
                TerminatorKind::SwitchBool {
                    operand: Operand::Copy(Place::Local(cond_slot)),
                    true_bb: body_block,
                    false_bb: after,
                },
                span,
            );

            // Loop body: read iter_slot[__idx] into a destructure-source
            // slot (named for single-identifier patterns, anonymous temp
            // otherwise), then bind the pattern, lower the body, and
            // increment the counter.
            builder.start_block(body_block);

            let pattern_slot = match pattern {
                ast::DestructurePattern::Identifier(name, _) => {
                    builder.alloc_local(name.clone(), LocalTypeInfo::Unknown)
                }
                _ => builder.alloc_temp(LocalTypeInfo::Unknown),
            };

            let index_place = Place::Index(
                Box::new(Place::Local(iter_slot)),
                Box::new(Operand::Copy(Place::Local(idx_slot))),
            );
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(pattern_slot),
                    Rvalue::Use(Operand::Copy(index_place)),
                ),
                span,
            );

            if !matches!(pattern, ast::DestructurePattern::Identifier(_, _)) {
                lower_destructure_bindings_from_place(
                    builder,
                    pattern,
                    &Place::Local(pattern_slot),
                    span,
                    None,
                );
            }

            builder.push_loop(after, header, None);
            builder.push_scope();
            lower_statements(builder, &for_loop.body, exit_block);
            builder.pop_scope();
            builder.pop_loop();

            // __idx = __idx + 1
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(idx_slot),
                    Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(Place::Local(idx_slot)),
                        Operand::Constant(MirConstant::Int(1)),
                    ),
                ),
                span,
            );

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
        ast::Pattern::Constructor { variant, fields, .. } => {
            // W12-jit-result-option-trinity (Phase 3 cluster-0 Round 7A,
            // 2026-05-12). For `Ok(v)` / `Err(e)` / `Some(x)` / `None`
            // bindings (per ADR-006 §2.7.17 / Q18 — kinded
            // `Arc<ResultData>` / `Arc<OptionData>` carrier), the binding
            // payload is NOT `Place::Index(scrutinee, 0)` — that would
            // read the `is_ok` byte of the ResultData struct interpreted
            // as an array element, returning garbage. Instead we emit an
            // explicit `Rvalue::EnumPayload { operand, variant: <tag> }`
            // into a fresh slot, then bind the inner pattern to that
            // slot. The JIT consumer reads from
            // `r.payload.kind`/`o.payload.kind` via `jit_arc_result_payload`
            // / `jit_arc_option_payload` per the §2.7.17 receiver-
            // recovery soundness rule.
            //
            // None has no payload (and the surrounding match codegen
            // ensures this arm is only entered when the variant matches —
            // so we don't emit EnumPayload for None_).
            if let Some(tag) = VariantTag::from_name(variant) {
                if matches!(tag, VariantTag::None_) {
                    // None: no inner bindings (parser only allows
                    // Unit-shape constructor fields for None).
                    return;
                }
                if let Some(source_place) = source_place {
                    // Allocate a fresh payload slot; emit
                    // EnumPayload into it; recurse with the new slot as
                    // the source_place. The fields are tuple-shape for
                    // Ok / Err / Some — single-element tuple containing
                    // the binding pattern.
                    match fields {
                        ast::PatternConstructorFields::Tuple(patterns) if patterns.len() == 1 => {
                            let payload_slot = builder.alloc_temp(LocalTypeInfo::Unknown);
                            builder.push_stmt(
                                StatementKind::Assign(
                                    Place::Local(payload_slot),
                                    Rvalue::EnumPayload {
                                        operand: Operand::Copy(source_place.clone()),
                                        variant: tag,
                                    },
                                ),
                                span,
                            );
                            lower_pattern_bindings_from_place_opt(
                                builder,
                                &patterns[0],
                                Some(&Place::Local(payload_slot)),
                                span,
                                binding_metadata,
                            );
                            return;
                        }
                        ast::PatternConstructorFields::Unit => {
                            // No payload to bind — `Ok` / `Err` / `Some`
                            // with no payload pattern is an unusual but
                            // valid form (e.g. just to test the variant
                            // without naming the payload). No bindings to
                            // emit.
                            return;
                        }
                        _ => {
                            // Fall through to the general path for non-
                            // tuple/non-unit shapes — this preserves
                            // backward-compat for user-defined struct
                            // variants that happen to be named Ok/Err/Some
                            // (rare but legal).
                        }
                    }
                }
            }

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
