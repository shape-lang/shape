//! MIR lowering: AST → MIR.
//!
//! Converts Shape AST function bodies into MIR basic blocks.
//! This is the bridge between parsing and borrow analysis.

use super::types::*;
use shape_ast::ast::{self, Expr, Span, Spanned, Statement};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
struct MirLoopContext {
    break_block: BasicBlockId,
    continue_block: BasicBlockId,
    break_value_slot: Option<SlotId>,
}

/// Builder for constructing a MIR function from AST.
pub struct MirBuilder {
    /// Name of the function being built.
    name: String,
    /// Completed basic blocks.
    blocks: Vec<BasicBlock>,
    /// Statements for the current (in-progress) basic block.
    current_stmts: Vec<MirStatement>,
    /// ID of the current basic block.
    current_block: BasicBlockId,
    /// Whether the current block has already been terminated and stored.
    current_block_finished: bool,
    /// Next block ID to allocate.
    next_block_id: u32,
    /// Next local slot to allocate.
    next_local: u16,
    /// Dedicated return slot used by explicit `return` statements.
    return_slot: SlotId,
    /// Next program point.
    next_point: u32,
    /// Next loan ID.
    next_loan: u32,
    /// Local variable name → slot mapping.
    locals: Vec<(String, SlotId, LocalTypeInfo)>,
    /// Active local name → slot mapping for place resolution.
    local_slots: HashMap<String, SlotId>,
    /// Stable field indices for property-place lowering.
    field_indices: HashMap<String, FieldIdx>,
    /// Next field index to allocate.
    next_field_idx: u16,
    /// Parameter slots.
    param_slots: Vec<SlotId>,
    /// Named-local shadowing stack for lexical scopes.
    scope_bindings: Vec<Vec<(String, Option<SlotId>)>>,
    /// Active loop control-flow targets.
    loop_contexts: Vec<MirLoopContext>,
    /// Exit block for the enclosing function.
    exit_block: Option<BasicBlockId>,
    /// Function span.
    span: Span,
    /// Whether lowering had to fall back to placeholder/Nop handling.
    had_fallbacks: bool,
}

#[derive(Debug)]
pub struct MirLoweringResult {
    pub mir: MirFunction,
    pub had_fallbacks: bool,
}

impl MirBuilder {
    pub fn new(name: String, span: Span) -> Self {
        let return_slot = SlotId(0);
        MirBuilder {
            name,
            blocks: Vec::new(),
            current_stmts: Vec::new(),
            current_block: BasicBlockId(0),
            current_block_finished: false,
            next_block_id: 1,
            next_local: 1,
            return_slot,
            next_point: 0,
            next_loan: 0,
            locals: vec![(
                "__mir_return".to_string(),
                return_slot,
                LocalTypeInfo::Unknown,
            )],
            local_slots: HashMap::new(),
            field_indices: HashMap::new(),
            next_field_idx: 0,
            param_slots: Vec::new(),
            scope_bindings: vec![Vec::new()],
            loop_contexts: Vec::new(),
            exit_block: None,
            span,
            had_fallbacks: false,
        }
    }

    /// Allocate a new local variable slot.
    pub fn alloc_local(&mut self, name: String, type_info: LocalTypeInfo) -> SlotId {
        let slot = SlotId(self.next_local);
        self.next_local += 1;
        self.locals.push((name, slot, type_info));
        if let Some((name, _, _)) = self.locals.last()
            && !name.starts_with("__mir_")
        {
            self.bind_named_local(name.clone(), slot);
        }
        slot
    }

    /// Allocate a temporary local slot that should not participate in name resolution.
    pub fn alloc_temp(&mut self, type_info: LocalTypeInfo) -> SlotId {
        let name = format!("__mir_tmp{}", self.next_local);
        self.alloc_local(name, type_info)
    }

    /// Register a parameter slot.
    pub fn add_param(&mut self, name: String, type_info: LocalTypeInfo) -> SlotId {
        let slot = self.alloc_local(name, type_info);
        self.param_slots.push(slot);
        slot
    }

    /// Look up the current slot for a named local.
    pub fn lookup_local(&self, name: &str) -> Option<SlotId> {
        self.local_slots.get(name).copied()
    }

    /// Get or allocate a stable field index for a property name.
    pub fn field_idx(&mut self, property: &str) -> FieldIdx {
        if let Some(idx) = self.field_indices.get(property).copied() {
            return idx;
        }
        let idx = FieldIdx(self.next_field_idx);
        self.next_field_idx += 1;
        self.field_indices.insert(property.to_string(), idx);
        idx
    }

    pub fn return_slot(&self) -> SlotId {
        self.return_slot
    }

    pub fn set_exit_block(&mut self, block: BasicBlockId) {
        self.exit_block = Some(block);
    }

    pub fn exit_block(&self) -> BasicBlockId {
        self.exit_block
            .expect("MIR builder exit block should be initialized before lowering")
    }

    pub fn push_scope(&mut self) {
        self.scope_bindings.push(Vec::new());
    }

    pub fn pop_scope(&mut self) {
        if self.scope_bindings.len() <= 1 {
            return;
        }
        if let Some(bindings) = self.scope_bindings.pop() {
            for (name, previous_slot) in bindings.into_iter().rev() {
                if let Some(slot) = previous_slot {
                    self.local_slots.insert(name, slot);
                } else {
                    self.local_slots.remove(&name);
                }
            }
        }
    }

    fn bind_named_local(&mut self, name: String, slot: SlotId) {
        if let Some(scope) = self.scope_bindings.last_mut()
            && !scope.iter().any(|(existing, _)| existing == &name)
        {
            scope.push((name.clone(), self.local_slots.get(&name).copied()));
        }
        self.local_slots.insert(name, slot);
    }

    pub fn mark_fallback(&mut self) {
        self.had_fallbacks = true;
    }

    pub fn push_loop(
        &mut self,
        break_block: BasicBlockId,
        continue_block: BasicBlockId,
        break_value_slot: Option<SlotId>,
    ) {
        self.loop_contexts.push(MirLoopContext {
            break_block,
            continue_block,
            break_value_slot,
        });
    }

    pub fn pop_loop(&mut self) {
        self.loop_contexts.pop();
    }

    fn current_loop(&self) -> Option<MirLoopContext> {
        self.loop_contexts.last().copied()
    }

    /// Allocate a new program point.
    pub fn next_point(&mut self) -> Point {
        let p = Point(self.next_point);
        self.next_point += 1;
        p
    }

    /// Allocate a new loan ID.
    pub fn next_loan(&mut self) -> LoanId {
        let l = LoanId(self.next_loan);
        self.next_loan += 1;
        l
    }

    /// Create a new basic block and return its ID.
    pub fn new_block(&mut self) -> BasicBlockId {
        let id = BasicBlockId(self.next_block_id);
        self.next_block_id += 1;
        id
    }

    /// Push a statement into the current block.
    pub fn push_stmt(&mut self, kind: StatementKind, span: Span) {
        let point = self.next_point();
        self.current_stmts.push(MirStatement { kind, span, point });
    }

    /// Finish the current block with a terminator and switch to a new block.
    pub fn finish_block(&mut self, terminator_kind: TerminatorKind, span: Span) {
        let block = BasicBlock {
            id: self.current_block,
            statements: std::mem::take(&mut self.current_stmts),
            terminator: Terminator {
                kind: terminator_kind,
                span,
            },
        };
        self.blocks.push(block);
        self.current_block_finished = true;
    }

    /// Start building a new block (after finishing the previous one).
    pub fn start_block(&mut self, id: BasicBlockId) {
        self.current_block = id;
        self.current_stmts.clear();
        self.current_block_finished = false;
    }

    /// Finalize and produce the MIR function.
    pub fn build(self) -> MirLoweringResult {
        let local_types = self.locals.iter().map(|(_, _, t)| t.clone()).collect();
        MirLoweringResult {
            mir: MirFunction {
                name: self.name,
                blocks: self.blocks,
                num_locals: self.next_local,
                param_slots: self.param_slots,
                local_types,
                span: self.span,
            },
            had_fallbacks: self.had_fallbacks,
        }
    }
}

/// Lower a function body (list of statements) into MIR.
pub fn lower_function_detailed(
    name: &str,
    params: &[ast::FunctionParameter],
    body: &[Statement],
    span: Span,
) -> MirLoweringResult {
    let mut builder = MirBuilder::new(name.to_string(), span);

    // Register parameters
    for param in params {
        let Some(param_name) = param.simple_name() else {
            builder.mark_fallback();
            let fallback_name = format!("__mir_unsupported_param{}", builder.param_slots.len());
            builder.add_param(fallback_name, LocalTypeInfo::Unknown);
            continue;
        };
        let type_info = if param.is_reference {
            LocalTypeInfo::NonCopy // references are always tracked
        } else {
            LocalTypeInfo::Unknown // will be resolved during analysis
        };
        builder.add_param(param_name.to_string(), type_info);
    }

    // Create the exit block
    let exit_block = builder.new_block();
    builder.set_exit_block(exit_block);

    // Lower body statements
    lower_statements(&mut builder, body, exit_block);

    // If current block hasn't been finished (no explicit return), emit goto exit
    if !builder.current_block_finished {
        builder.finish_block(TerminatorKind::Goto(exit_block), span);
    }

    // Create exit block with Return terminator
    builder.start_block(exit_block);
    builder.finish_block(TerminatorKind::Return, span);

    builder.build()
}

/// Lower a function body (list of statements) into MIR.
pub fn lower_function(
    name: &str,
    params: &[ast::FunctionParameter],
    body: &[Statement],
    span: Span,
) -> MirFunction {
    lower_function_detailed(name, params, body, span).mir
}

/// Lower a slice of statements into the current block.
fn lower_statements(builder: &mut MirBuilder, stmts: &[Statement], exit_block: BasicBlockId) {
    for stmt in stmts {
        lower_statement(builder, stmt, exit_block);
    }
}

/// Lower a single statement.
fn lower_statement(builder: &mut MirBuilder, stmt: &Statement, exit_block: BasicBlockId) {
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
            // Expression statement — evaluate for side effects
            let _slot = lower_expr_to_temp(builder, expr);
            let _ = span; // span captured in sub-lowering
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
        _ => {
            // Other statement types: emit a Nop for now.
            // Will be expanded as more AST constructs get MIR support.
            let span = stmt.span().unwrap_or(Span::DUMMY);
            builder.mark_fallback();
            builder.push_stmt(StatementKind::Nop, span);
        }
    }
}

/// Lower a variable declaration.
fn lower_var_decl(builder: &mut MirBuilder, decl: &ast::VariableDecl, span: Span) {
    let Some(name) = decl.pattern.as_identifier() else {
        builder.mark_fallback();
        if let Some(init_expr) = &decl.value {
            let _ = lower_expr_to_temp(builder, init_expr);
        }
        builder.push_stmt(StatementKind::Nop, span);
        return;
    };
    let type_info = decl
        .value
        .as_ref()
        .map(infer_local_type_from_expr)
        .unwrap_or(LocalTypeInfo::Unknown);
    let slot = builder.alloc_local(name.to_string(), type_info);

    if let Some(init_expr) = &decl.value {
        // Determine operand based on ownership modifier
        let operand = match decl.ownership {
            ast::OwnershipModifier::Move => lower_expr_to_explicit_move_operand(builder, init_expr),
            ast::OwnershipModifier::Clone => lower_expr_to_operand(builder, init_expr, false),
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
        builder.push_stmt(StatementKind::Assign(Place::Local(slot), rvalue), span);
    }
}

fn start_dead_block(builder: &mut MirBuilder) {
    let dead_block = builder.new_block();
    builder.start_block(dead_block);
}

fn lower_return_control_flow(builder: &mut MirBuilder, value: Option<&Expr>, span: Span) {
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

fn lower_break_control_flow(builder: &mut MirBuilder, value: Option<&Expr>, span: Span) {
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
        builder.push_stmt(StatementKind::Assign(Place::Local(result_slot), rvalue), span);
    } else if let Some(expr) = value {
        let _ = lower_expr_to_temp(builder, expr);
    }

    builder.finish_block(TerminatorKind::Goto(loop_ctx.break_block), span);
    start_dead_block(builder);
}

fn lower_continue_control_flow(builder: &mut MirBuilder, span: Span) {
    let Some(loop_ctx) = builder.current_loop() else {
        builder.mark_fallback();
        builder.push_stmt(StatementKind::Nop, span);
        return;
    };

    builder.finish_block(TerminatorKind::Goto(loop_ctx.continue_block), span);
    start_dead_block(builder);
}

fn lower_expr_to_explicit_move_operand(builder: &mut MirBuilder, expr: &Expr) -> Operand {
    if let Some(place) = lower_expr_to_place(builder, expr) {
        Operand::MoveExplicit(place)
    } else {
        let slot = lower_expr_to_temp(builder, expr);
        Operand::MoveExplicit(Place::Local(slot))
    }
}

fn infer_local_type_from_expr(expr: &Expr) -> LocalTypeInfo {
    match expr {
        Expr::Literal(literal, _) => match literal {
            ast::Literal::Int(_)
            | ast::Literal::UInt(_)
            | ast::Literal::TypedInt(_, _)
            | ast::Literal::Number(_)
            | ast::Literal::Decimal(_)
            | ast::Literal::Bool(_)
            | ast::Literal::Char(_)
            | ast::Literal::None
            | ast::Literal::Unit
            | ast::Literal::Timeframe(_) => LocalTypeInfo::Copy,
            ast::Literal::String(_)
            | ast::Literal::FormattedString { .. }
            | ast::Literal::ContentString { .. } => LocalTypeInfo::NonCopy,
        },
        Expr::Reference { .. } => LocalTypeInfo::NonCopy,
        _ => LocalTypeInfo::Unknown,
    }
}

/// Lower an assignment statement.
fn lower_assignment(builder: &mut MirBuilder, assign: &ast::Assignment, span: Span) {
    let Some(name) = assign.pattern.as_identifier() else {
        builder.mark_fallback();
        builder.push_stmt(StatementKind::Nop, span);
        return;
    };
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
}

fn lower_assign_target_place(builder: &mut MirBuilder, target: &Expr) -> Option<Place> {
    match target {
        Expr::Identifier(name, _) => builder.lookup_local(name).map(Place::Local),
        Expr::PropertyAccess { .. } | Expr::IndexAccess { .. } => {
            lower_expr_to_place(builder, target)
        }
        _ => None,
    }
}

/// Lower an expression and return the temp slot it was placed in.
/// This is a simplified version — full expression lowering will be more complex.
fn lower_expr_to_place(builder: &mut MirBuilder, expr: &Expr) -> Option<Place> {
    match expr {
        Expr::Identifier(name, _) => builder.lookup_local(name).map(Place::Local),
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
                builder.mark_fallback();
                return None;
            }
            let base = lower_expr_to_place(builder, object)?;
            let index_operand = lower_expr_to_operand(builder, index, false);
            Some(Place::Index(Box::new(base), Box::new(index_operand)))
        }
        _ => None,
    }
}

fn lower_expr_to_operand(builder: &mut MirBuilder, expr: &Expr, prefer_move: bool) -> Operand {
    if let Some(place) = lower_expr_to_place(builder, expr) {
        if prefer_move {
            Operand::Move(place)
        } else {
            Operand::Copy(place)
        }
    } else {
        let slot = lower_expr_to_temp(builder, expr);
        let place = Place::Local(slot);
        if prefer_move {
            Operand::Move(place)
        } else {
            Operand::Copy(place)
        }
    }
}

fn lower_expr_to_temp(builder: &mut MirBuilder, expr: &Expr) -> SlotId {
    let span = expr.span();
    let temp = builder.alloc_temp(LocalTypeInfo::Unknown);

    match expr {
        Expr::Literal(_, _) => {
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
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Use(operand)),
                span,
            );
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
                StatementKind::Assign(Place::Local(temp), Rvalue::Borrow(kind, borrowed_place)),
                *ref_span,
            );
        }
        Expr::Assign(assign, _) => {
            let Some(target_place) = lower_assign_target_place(builder, &assign.target) else {
                builder.mark_fallback();
                builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(temp),
                        Rvalue::Use(Operand::Constant(MirConstant::None)),
                    ),
                    span,
                );
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
        Expr::BinaryOp { left, right, .. } => {
            let l = lower_expr_to_operand(builder, left, false);
            let r = lower_expr_to_operand(builder, right, false);
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::BinaryOp(BinOp::Add, l, r)),
                span,
            );
        }
        Expr::Break(value, _) => {
            lower_break_control_flow(builder, value.as_deref(), span);
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::Use(Operand::Constant(MirConstant::None)),
                ),
                span,
            );
        }
        Expr::Continue(_) => {
            lower_continue_control_flow(builder, span);
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::Use(Operand::Constant(MirConstant::None)),
                ),
                span,
            );
        }
        Expr::Return(value, _) => {
            lower_return_control_flow(builder, value.as_deref(), span);
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::Use(Operand::Constant(MirConstant::None)),
                ),
                span,
            );
        }
        Expr::FunctionCall { args, .. } => {
            // Lower function calls — simplified for now
            let arg_ops: Vec<Operand> = args
                .iter()
                .map(|a| {
                    let s = lower_expr_to_temp(builder, a);
                    Operand::Move(Place::Local(s))
                })
                .collect();
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(arg_ops)),
                span,
            );
        }
        _ => {
            // Fallback: emit a Nop + assign from constant
            builder.mark_fallback();
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::Use(Operand::Constant(MirConstant::None)),
                ),
                span,
            );
        }
    }

    temp
}

fn lower_conditional_expr(
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

fn lower_block_expr(builder: &mut MirBuilder, block: &ast::BlockExpr, temp: SlotId, span: Span) {
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
                lower_assignment(builder, assign, span);
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
                lower_statement(builder, stmt, builder.exit_block());
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

fn lower_let_expr(builder: &mut MirBuilder, let_expr: &ast::LetExpr, temp: SlotId, span: Span) {
    builder.push_scope();

    let Some(name) = let_expr.pattern.as_simple_name() else {
        builder.mark_fallback();
        if let Some(value) = &let_expr.value {
            let _ = lower_expr_to_temp(builder, value);
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
        return;
    };

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

fn lower_for_expr(builder: &mut MirBuilder, for_expr: &ast::ForExpr, temp: SlotId, span: Span) {
    builder.push_scope();

    let iter_slot = lower_expr_to_temp(builder, &for_expr.iterable);
    let elem_slot = match &for_expr.pattern {
        ast::Pattern::Identifier(name) | ast::Pattern::Typed { name, .. } => {
            Some(builder.alloc_local(name.clone(), LocalTypeInfo::Unknown))
        }
        ast::Pattern::Wildcard => Some(builder.alloc_temp(LocalTypeInfo::Unknown)),
        _ => {
            builder.mark_fallback();
            None
        }
    };
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
    if let Some(elem_slot) = elem_slot {
        builder.push_stmt(
            StatementKind::Assign(
                Place::Local(elem_slot),
                Rvalue::Use(Operand::Constant(MirConstant::None)),
            ),
            span,
        );
    }
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

fn lower_loop_expr(builder: &mut MirBuilder, loop_expr: &ast::LoopExpr, temp: SlotId, span: Span) {
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

fn lower_match_expr(
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

        match &arm.pattern {
            ast::Pattern::Identifier(name) | ast::Pattern::Typed { name, .. } => {
                builder.push_scope();
                binding_scope_active = true;
                let binding_slot = builder.alloc_local(name.clone(), LocalTypeInfo::Unknown);
                builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(binding_slot),
                        Rvalue::Use(Operand::Copy(Place::Local(scrutinee_slot))),
                    ),
                    pattern_span,
                );
                if let Some(guard) = &arm.guard {
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
            }
            ast::Pattern::Wildcard => {
                if let Some(guard) = &arm.guard {
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
            }
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
                if let Some(guard) = &arm.guard {
                    let guard_block = builder.new_block();
                    builder.finish_block(
                        TerminatorKind::SwitchBool {
                            operand: Operand::Copy(Place::Local(matches_slot)),
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
                            operand: Operand::Copy(Place::Local(matches_slot)),
                            true_bb: body_block,
                            false_bb: next_block,
                        },
                        pattern_span,
                    );
                }
            }
            _ => {
                builder.mark_fallback();
                if let Some(guard) = &arm.guard {
                    let _ = lower_expr_to_temp(builder, guard);
                }
                builder.finish_block(TerminatorKind::Goto(body_block), pattern_span);
            }
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
            let pattern_slot = match pattern {
                ast::DestructurePattern::Identifier(name, _) => {
                    Some(builder.alloc_local(name.clone(), LocalTypeInfo::Unknown))
                }
                _ => {
                    builder.mark_fallback();
                    None
                }
            };
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
            if let Some(pattern_slot) = pattern_slot {
                builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(pattern_slot),
                        Rvalue::Use(Operand::Constant(MirConstant::None)),
                    ),
                    span,
                );
            }
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
            lower_statement(builder, init, exit_block);

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

// Helper to get span from Statement
trait StatementSpan {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::analysis::BorrowErrorKind;
    use crate::mir::cfg::ControlFlowGraph;
    use crate::mir::liveness;
    use crate::mir::solver;
    use shape_ast::ast::{self, DestructurePattern, OwnershipModifier, VarKind};

    fn span() -> Span {
        Span { start: 0, end: 1 }
    }

    fn lower_parsed_function(code: &str) -> MirLoweringResult {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let func = match &program.items[0] {
            ast::Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };
        lower_function_detailed(&func.name, &func.params, &func.body, func.name_span)
    }

    #[test]
    fn test_lower_empty_function() {
        let mir = lower_function("empty", &[], &[], span());
        assert_eq!(mir.name, "empty");
        assert!(mir.blocks.len() >= 2); // entry + exit
        assert_eq!(mir.num_locals, 1);
    }

    #[test]
    fn test_lower_simple_var_decl() {
        let body = vec![Statement::VariableDecl(
            ast::VariableDecl {
                kind: VarKind::Let,
                is_mut: false,
                pattern: DestructurePattern::Identifier("x".to_string(), span()),
                type_annotation: None,
                value: Some(Expr::Literal(ast::Literal::Int(42), span())),
                ownership: OwnershipModifier::Inferred,
            },
            span(),
        )];
        let mir = lower_function("test", &[], &body, span());
        assert!(mir.num_locals >= 1); // at least x + temp
        // Should have at least 2 blocks (entry + exit)
        assert!(mir.blocks.len() >= 2);
    }

    #[test]
    fn test_lower_with_liveness() {
        // let x = 1; let y = x; (x live after first stmt, dead after second)
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(
                        ast::Literal::String("hi".to_string()),
                        span(),
                    )),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("y".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Identifier("x".to_string(), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
        ];
        let mir = lower_function("test", &[], &body, span());
        let cfg = ControlFlowGraph::build(&mir);
        let _liveness = liveness::compute_liveness(&mir, &cfg);
        // The MIR lowers and liveness computes without panic
    }

    #[test]
    fn test_lower_reference_to_identifier_borrows_original_local() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(
                        ast::Literal::String("hi".to_string()),
                        span(),
                    )),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("r".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        is_mutable: false,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
        ];
        let mir = lower_function("test", &[], &body, span());
        let borrow_place = mir
            .blocks
            .iter()
            .flat_map(|block| block.statements.iter())
            .find_map(|stmt| match &stmt.kind {
                StatementKind::Assign(_, Rvalue::Borrow(_, place)) => Some(place.clone()),
                _ => None,
            })
            .expect("expected borrow statement");
        assert_eq!(borrow_place, Place::Local(SlotId(1)));
    }

    #[test]
    fn test_lowered_local_borrow_conflict_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("shared".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        is_mutable: false,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("exclusive".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        is_mutable: true,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ConflictSharedExclusive),
            "expected shared/exclusive conflict, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_property_borrows_preserve_disjoint_places() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("pair".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(0), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("left".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::PropertyAccess {
                            object: Box::new(Expr::Identifier("pair".to_string(), span())),
                            property: "left".to_string(),
                            optional: false,
                            span: span(),
                        }),
                        is_mutable: true,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("right".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::PropertyAccess {
                            object: Box::new(Expr::Identifier("pair".to_string(), span())),
                            property: "right".to_string(),
                            optional: false,
                            span: span(),
                        }),
                        is_mutable: true,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir);
        assert!(
            analysis.errors.is_empty(),
            "disjoint field borrows should not conflict, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_write_while_borrowed_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("shared".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        is_mutable: false,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::Assignment(
                ast::Assignment {
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    value: Expr::Literal(ast::Literal::Int(2), span()),
                },
                span(),
            ),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_read_while_exclusive_borrow_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("exclusive".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        is_mutable: true,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("copy".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Identifier("x".to_string(), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReadWhileExclusivelyBorrowed),
            "expected read-while-exclusive error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_returned_ref_alias_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("r".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        is_mutable: false,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("alias".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Identifier("r".to_string(), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::Return(Some(Expr::Identifier("alias".to_string(), span())), span()),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceEscape),
            "expected reference-escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_use_after_explicit_move_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(
                        ast::Literal::String("hi".to_string()),
                        span(),
                    )),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("y".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Identifier("x".to_string(), span())),
                    ownership: OwnershipModifier::Move,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("z".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Identifier("x".to_string(), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::UseAfterMove),
            "expected use-after-move error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_while_expr_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let mut x = 1
                    let y = while true {
                        let shared = &x
                        x = 2
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "while-expression lowering should stay in the supported subset"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected while-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_for_expr_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(items) {
                    let mut x = 1
                    let y = for item in items {
                        let shared = &x
                        x = 2
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "for-expression lowering should stay in the supported subset"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected for-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_loop_expr_break_value_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let mut x = 1
                    let y = loop {
                        let shared = &x
                        break (x = 2)
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "loop-expression break lowering should stay in the supported subset"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected loop-expression break write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_continue_expression_in_while_body_stays_supported() {
        let lowering = lower_parsed_function(
            r#"
                function test(flag) {
                    let mut x = 1
                    let y = while flag {
                        if flag { continue } else { x }
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "continue inside a while-expression body should stay supported"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis.errors.is_empty(),
            "continue-only control flow should not introduce borrow errors, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_match_expression_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(flag) {
                    let mut x = 1
                    let y = match flag {
                        true => {
                            let shared = &x
                            x = 2
                        }
                        _ => 0
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "simple literal/wildcard match lowering should stay supported"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected match-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_match_expression_identifier_guard_stays_supported() {
        let lowering = lower_parsed_function(
            r#"
                function test(v) {
                    let y = match v {
                        x where x > 0 => x
                        _ => 0
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "identifier/guard match lowering should stay supported"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis.errors.is_empty(),
            "simple guarded identifier matches should stay clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lower_destructure_var_decl_marks_fallback() {
        let body = vec![Statement::VariableDecl(
            ast::VariableDecl {
                kind: VarKind::Let,
                is_mut: false,
                pattern: DestructurePattern::Array(vec![
                    DestructurePattern::Identifier("left".to_string(), span()),
                    DestructurePattern::Identifier("right".to_string(), span()),
                ]),
                type_annotation: None,
                value: Some(Expr::Literal(
                    ast::Literal::String("hi".to_string()),
                    span(),
                )),
                ownership: OwnershipModifier::Inferred,
            },
            span(),
        )];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(
            lowering.had_fallbacks,
            "destructuring declarations should keep MIR in fallback mode"
        );
    }

    #[test]
    fn test_lowered_assignment_expr_write_while_borrowed_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("shared".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        is_mutable: false,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("y".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Assign(
                        Box::new(ast::AssignExpr {
                            target: Box::new(Expr::Identifier("x".to_string(), span())),
                            value: Box::new(Expr::Literal(ast::Literal::Int(2), span())),
                        }),
                        span(),
                    )),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(
            !lowering.had_fallbacks,
            "simple assignment expressions should stay in the supported MIR subset"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_property_assignment_expr_preserves_disjoint_places() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("pair".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(
                        ast::Literal::String("pair".to_string()),
                        span(),
                    )),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("left".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Reference {
                        expr: Box::new(Expr::PropertyAccess {
                            object: Box::new(Expr::Identifier("pair".to_string(), span())),
                            property: "left".to_string(),
                            optional: false,
                            span: span(),
                        }),
                        is_mutable: false,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::Expression(
                Expr::Assign(
                    Box::new(ast::AssignExpr {
                        target: Box::new(Expr::PropertyAccess {
                            object: Box::new(Expr::Identifier("pair".to_string(), span())),
                            property: "right".to_string(),
                            optional: false,
                            span: span(),
                        }),
                        value: Box::new(Expr::Literal(
                            ast::Literal::String("updated".to_string()),
                            span(),
                        )),
                    }),
                    span(),
                ),
                span(),
            ),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(
            !lowering.had_fallbacks,
            "property assignment expressions should stay in the supported MIR subset"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis.errors.is_empty(),
            "disjoint property assignment should stay borrow-clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_block_expr_write_while_borrowed_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("shared".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Block(
                        ast::BlockExpr {
                            items: vec![
                                ast::BlockItem::VariableDecl(ast::VariableDecl {
                                    kind: VarKind::Let,
                                    is_mut: false,
                                    pattern: DestructurePattern::Identifier(
                                        "inner".to_string(),
                                        span(),
                                    ),
                                    type_annotation: None,
                                    value: Some(Expr::Reference {
                                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                                        is_mutable: false,
                                        span: span(),
                                    }),
                                    ownership: OwnershipModifier::Inferred,
                                }),
                                ast::BlockItem::Expression(Expr::Identifier(
                                    "inner".to_string(),
                                    span(),
                                )),
                            ],
                        },
                        span(),
                    )),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::Assignment(
                ast::Assignment {
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    value: Expr::Literal(ast::Literal::Int(2), span()),
                },
                span(),
            ),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(
            !lowering.had_fallbacks,
            "block expressions with simple local bindings should stay in the supported MIR subset"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_let_expr_write_while_borrowed_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("shared".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Let(
                        Box::new(ast::LetExpr {
                            pattern: ast::Pattern::Identifier("inner".to_string()),
                            type_annotation: None,
                            value: Some(Box::new(Expr::Reference {
                                expr: Box::new(Expr::Identifier("x".to_string(), span())),
                                is_mutable: false,
                                span: span(),
                            })),
                            body: Box::new(Expr::Identifier("inner".to_string(), span())),
                        }),
                        span(),
                    )),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::Assignment(
                ast::Assignment {
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    value: Expr::Literal(ast::Literal::Int(2), span()),
                },
                span(),
            ),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(
            !lowering.had_fallbacks,
            "let expressions with simple bindings should stay in the supported MIR subset"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_if_expression_with_block_branches_stays_supported() {
        let block_branch = |borrow_name: &str| {
            Expr::Block(
                ast::BlockExpr {
                    items: vec![ast::BlockItem::Expression(Expr::Reference {
                        expr: Box::new(Expr::Identifier(borrow_name.to_string(), span())),
                        is_mutable: false,
                        span: span(),
                    })],
                },
                span(),
            )
        };
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("flag".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Bool(true), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("shared".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Conditional {
                        condition: Box::new(Expr::Identifier("flag".to_string(), span())),
                        then_expr: Box::new(block_branch("x")),
                        else_expr: Some(Box::new(block_branch("x"))),
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::Assignment(
                ast::Assignment {
                    pattern: DestructurePattern::Identifier("x".to_string(), span()),
                    value: Expr::Literal(ast::Literal::Int(2), span()),
                },
                span(),
            ),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(
            !lowering.had_fallbacks,
            "if expressions with simple block branches should stay in the supported MIR subset"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis.errors.is_empty(),
            "simple branch-local borrows should stay borrow-clean here, got {:?}",
            analysis.errors
        );
    }
}
