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

#[derive(Debug, Clone)]
struct TaskBoundaryCaptureScope {
    outer_locals_cutoff: u16,
    operands: Vec<Operand>,
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
    /// Active task-boundary capture scopes for async lowering.
    task_boundary_capture_scopes: Vec<TaskBoundaryCaptureScope>,
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
            task_boundary_capture_scopes: Vec::new(),
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

    pub fn push_task_boundary_capture_scope(&mut self) {
        self.task_boundary_capture_scopes
            .push(TaskBoundaryCaptureScope {
                outer_locals_cutoff: self.next_local,
                operands: Vec::new(),
            });
    }

    pub fn pop_task_boundary_capture_scope(&mut self) -> Vec<Operand> {
        self.task_boundary_capture_scopes
            .pop()
            .map(|scope| scope.operands)
            .unwrap_or_default()
    }

    pub fn record_task_boundary_operand(&mut self, operand: Operand) {
        for scope in &mut self.task_boundary_capture_scopes {
            if !operand_crosses_task_boundary(scope.outer_locals_cutoff, &operand) {
                continue;
            }
            if !scope.operands.contains(&operand) {
                scope.operands.push(operand.clone());
            }
        }
    }

    pub fn record_task_boundary_reference_capture(
        &mut self,
        reference_slot: SlotId,
        borrowed_place: &Place,
    ) {
        let reference_operand = Operand::Copy(Place::Local(reference_slot));
        for scope in &mut self.task_boundary_capture_scopes {
            if borrowed_place.root_local().0 >= scope.outer_locals_cutoff {
                continue;
            }
            if !scope.operands.contains(&reference_operand) {
                scope.operands.push(reference_operand.clone());
            }
        }
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
        let type_info = if param.is_reference {
            LocalTypeInfo::NonCopy // references are always tracked
        } else {
            LocalTypeInfo::Unknown // will be resolved during analysis
        };
        if let Some(param_name) = param.simple_name() {
            builder.add_param(param_name.to_string(), type_info);
        } else {
            let fallback_name = format!("__mir_param{}", builder.param_slots.len());
            let slot = builder.add_param(fallback_name, type_info);
            lower_destructure_bindings_from_place(
                &mut builder,
                &param.pattern,
                &Place::Local(slot),
                param.span(),
            );
        }
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

/// Lower a variable declaration.
fn lower_var_decl(builder: &mut MirBuilder, decl: &ast::VariableDecl, span: Span) {
    if let Some(name) = decl.pattern.as_identifier() {
        let type_info = decl
            .value
            .as_ref()
            .map(infer_local_type_from_expr)
            .unwrap_or(LocalTypeInfo::Unknown);
        let slot = builder.alloc_local(name.to_string(), type_info);

        if let Some(init_expr) = &decl.value {
            // Determine operand based on ownership modifier
            let operand = match decl.ownership {
                ast::OwnershipModifier::Move => {
                    lower_expr_to_explicit_move_operand(builder, init_expr)
                }
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
        return;
    }

    let source_place = decl.value.as_ref().map(|init_expr| {
        let type_info = infer_local_type_from_expr(init_expr);
        let source_slot = builder.alloc_temp(type_info);
        let operand = match decl.ownership {
            ast::OwnershipModifier::Move => lower_expr_to_explicit_move_operand(builder, init_expr),
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
    lower_destructure_bindings_from_place_opt(builder, &decl.pattern, source_place.as_ref(), span);
}

fn projected_field_place(builder: &mut MirBuilder, base: &Place, property: &str) -> Place {
    Place::Field(Box::new(base.clone()), builder.field_idx(property))
}

fn projected_index_place(base: &Place, index: usize) -> Place {
    Place::Index(
        Box::new(base.clone()),
        Box::new(Operand::Constant(MirConstant::Int(index as i64))),
    )
}

fn assign_none(builder: &mut MirBuilder, destination: SlotId, span: Span) {
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(destination),
            Rvalue::Use(Operand::Constant(MirConstant::None)),
        ),
        span,
    );
}

fn assign_copy_from_place(builder: &mut MirBuilder, destination: SlotId, place: Place, span: Span) {
    builder.push_stmt(
        StatementKind::Assign(Place::Local(destination), Rvalue::Use(Operand::Copy(place))),
        span,
    );
}

fn assign_copy_from_slot(
    builder: &mut MirBuilder,
    destination: SlotId,
    source: SlotId,
    span: Span,
) {
    assign_copy_from_place(builder, destination, Place::Local(source), span);
}

fn lower_expr_as_moved_operand(builder: &mut MirBuilder, expr: &Expr) -> Operand {
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

fn lower_exprs_to_aggregate<'a>(
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

fn lower_binary_op(op: ast::BinaryOp) -> Option<BinOp> {
    match op {
        ast::BinaryOp::Add => Some(BinOp::Add),
        ast::BinaryOp::Sub => Some(BinOp::Sub),
        ast::BinaryOp::Mul => Some(BinOp::Mul),
        ast::BinaryOp::Div => Some(BinOp::Div),
        ast::BinaryOp::Mod => Some(BinOp::Mod),
        ast::BinaryOp::Greater => Some(BinOp::Gt),
        ast::BinaryOp::Less => Some(BinOp::Lt),
        ast::BinaryOp::GreaterEq => Some(BinOp::Ge),
        ast::BinaryOp::LessEq => Some(BinOp::Le),
        ast::BinaryOp::Equal => Some(BinOp::Eq),
        ast::BinaryOp::NotEqual => Some(BinOp::Ne),
        ast::BinaryOp::And => Some(BinOp::And),
        ast::BinaryOp::Or => Some(BinOp::Or),
        ast::BinaryOp::Pow
        | ast::BinaryOp::FuzzyEqual
        | ast::BinaryOp::FuzzyGreater
        | ast::BinaryOp::FuzzyLess
        | ast::BinaryOp::BitAnd
        | ast::BinaryOp::BitOr
        | ast::BinaryOp::BitXor
        | ast::BinaryOp::BitShl
        | ast::BinaryOp::BitShr
        | ast::BinaryOp::NullCoalesce
        | ast::BinaryOp::ErrorContext
        | ast::BinaryOp::Pipe => None,
    }
}

fn lower_unary_op(op: ast::UnaryOp) -> Option<UnOp> {
    match op {
        ast::UnaryOp::Neg => Some(UnOp::Neg),
        ast::UnaryOp::Not => Some(UnOp::Not),
        ast::UnaryOp::BitNot => None,
    }
}

fn lower_constructor_bindings_from_place_opt(
    builder: &mut MirBuilder,
    fields: &ast::PatternConstructorFields,
    source_place: Option<&Place>,
    span: Span,
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
                );
            }
        }
    }
}

fn pattern_has_bindings(pattern: &ast::Pattern) -> bool {
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

fn lower_destructure_bindings_from_place_opt(
    builder: &mut MirBuilder,
    pattern: &ast::DestructurePattern,
    source_place: Option<&Place>,
    span: Span,
) {
    match pattern {
        ast::DestructurePattern::Identifier(name, _) => {
            let slot = builder.alloc_local(name.clone(), LocalTypeInfo::Unknown);
            if let Some(source_place) = source_place {
                builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(slot),
                        Rvalue::Use(Operand::Copy(source_place.clone())),
                    ),
                    span,
                );
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
                );
            }
        }
        ast::DestructurePattern::Rest(pattern) => {
            lower_destructure_bindings_from_place_opt(builder, pattern, source_place, span);
        }
        ast::DestructurePattern::Decomposition(bindings) => {
            for binding in bindings {
                let slot = builder.alloc_local(binding.name.clone(), LocalTypeInfo::Unknown);
                if let Some(source_place) = source_place {
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
}

fn lower_destructure_bindings_from_place(
    builder: &mut MirBuilder,
    pattern: &ast::DestructurePattern,
    source_place: &Place,
    span: Span,
) {
    lower_destructure_bindings_from_place_opt(builder, pattern, Some(source_place), span);
}

fn lower_pattern_bindings_from_place_opt(
    builder: &mut MirBuilder,
    pattern: &ast::Pattern,
    source_place: Option<&Place>,
    span: Span,
) {
    match pattern {
        ast::Pattern::Identifier(name) | ast::Pattern::Typed { name, .. } => {
            let slot = builder.alloc_local(name.clone(), LocalTypeInfo::Unknown);
            if let Some(source_place) = source_place {
                builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(slot),
                        Rvalue::Use(Operand::Copy(source_place.clone())),
                    ),
                    span,
                );
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
                );
            }
        }
        ast::Pattern::Constructor { fields, .. } => {
            lower_constructor_bindings_from_place_opt(builder, fields, source_place, span);
        }
        ast::Pattern::Wildcard => {}
        ast::Pattern::Literal(_) => {}
    }
}

fn lower_pattern_bindings_from_place(
    builder: &mut MirBuilder,
    pattern: &ast::Pattern,
    source_place: &Place,
    span: Span,
) {
    lower_pattern_bindings_from_place_opt(builder, pattern, Some(source_place), span);
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

fn lower_expr_to_operand(builder: &mut MirBuilder, expr: &Expr, prefer_move: bool) -> Operand {
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

fn emit_task_boundary_if_needed(builder: &mut MirBuilder, operands: Vec<Operand>, span: Span) {
    if operands.is_empty() {
        return;
    }
    builder.push_stmt(StatementKind::TaskBoundary(operands), span);
}

fn lower_await_expr(builder: &mut MirBuilder, inner: &Expr, temp: SlotId, span: Span) {
    let operand = lower_expr_to_operand(builder, inner, true);
    builder.push_stmt(
        StatementKind::Assign(Place::Local(temp), Rvalue::Use(operand)),
        span,
    );
}

fn lower_async_scope_expr(builder: &mut MirBuilder, inner: &Expr, temp: SlotId, span: Span) {
    let inner_slot = lower_expr_to_temp(builder, inner);
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

    let future_slot = builder.alloc_local(async_let.name.clone(), LocalTypeInfo::Unknown);
    assign_none(builder, future_slot, async_let.span);
    assign_copy_from_slot(builder, temp, future_slot, span);
}

fn lower_join_expr(builder: &mut MirBuilder, join_expr: &ast::JoinExpr, temp: SlotId, span: Span) {
    if join_expr.branches.is_empty() {
        assign_none(builder, temp, span);
        return;
    }

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

    builder.push_stmt(
        StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(branch_operands)),
        join_expr.span,
    );
}

fn operand_crosses_task_boundary(outer_locals_cutoff: u16, operand: &Operand) -> bool {
    match operand {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            place.root_local().0 < outer_locals_cutoff
        }
        Operand::Constant(_) => false,
    }
}

fn lower_expr_to_temp(builder: &mut MirBuilder, expr: &Expr) -> SlotId {
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
            lower_break_control_flow(builder, value.as_deref(), span);
            assign_none(builder, temp, span);
        }
        Expr::Continue(_) => {
            lower_continue_control_flow(builder, span);
            assign_none(builder, temp, span);
        }
        Expr::Return(value, _) => {
            lower_return_control_flow(builder, value.as_deref(), span);
            assign_none(builder, temp, span);
        }
        Expr::FunctionCall {
            name,
            args,
            named_args,
            ..
        } => {
            let mut arg_ops = Vec::with_capacity(1 + args.len() + named_args.len());
            arg_ops.push(Operand::Constant(MirConstant::Function(name.clone())));
            arg_ops.extend(
                args.iter()
                    .map(|arg| lower_expr_as_moved_operand(builder, arg)),
            );
            arg_ops.extend(
                named_args
                    .iter()
                    .map(|(_, expr)| lower_expr_as_moved_operand(builder, expr)),
            );
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(arg_ops)),
                span,
            );
        }
        Expr::EnumConstructor { payload, .. } => match payload {
            ast::EnumConstructorPayload::Unit => {
                assign_none(builder, temp, span);
            }
            ast::EnumConstructorPayload::Tuple(values) => {
                lower_exprs_to_aggregate(builder, temp, values.iter(), span);
            }
            ast::EnumConstructorPayload::Struct(fields) => {
                lower_exprs_to_aggregate(builder, temp, fields.iter().map(|(_, expr)| expr), span);
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
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
                span,
            );
        }
        Expr::Array(elements, _) => {
            lower_exprs_to_aggregate(builder, temp, elements.iter(), span);
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
        Expr::Spread(expr, _) => {
            let expr_slot = lower_expr_to_temp(builder, expr);
            assign_copy_from_slot(builder, temp, expr_slot, span);
        }
        Expr::MethodCall {
            receiver,
            args,
            named_args,
            ..
        } => {
            let mut operands = Vec::with_capacity(1 + args.len() + named_args.len());
            operands.push(lower_expr_as_moved_operand(builder, receiver));
            operands.extend(
                args.iter()
                    .map(|arg| lower_expr_as_moved_operand(builder, arg)),
            );
            operands.extend(
                named_args
                    .iter()
                    .map(|(_, expr)| lower_expr_as_moved_operand(builder, expr)),
            );
            builder.push_stmt(
                StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands)),
                span,
            );
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
            lower_exprs_to_aggregate(builder, temp, fields.iter().map(|(_, expr)| expr), span);
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
        _ => {
            // Remaining fallbacks are the intentionally deferred step-4 forms:
            // closures/captures, queries/comprehensions,
            // and comptime-evaluated expression families.
            builder.mark_fallback();
            assign_none(builder, temp, span);
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
        lower_pattern_bindings_from_place_opt(
            builder,
            &let_expr.pattern,
            source_place.as_ref(),
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
    lower_pattern_bindings_from_place(builder, &for_expr.pattern, &Place::Local(elem_slot), span);
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
        if pattern_has_bindings(&arm.pattern) {
            builder.push_scope();
            binding_scope_active = true;
            lower_pattern_bindings_from_place(
                builder,
                &arm.pattern,
                &Place::Local(scrutinee_slot),
                pattern_span,
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
    fn test_lowered_match_expression_array_pattern_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(pair) {
                    let mut x = 1
                    let y = match pair {
                        [left, right] => {
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
            "array-pattern match lowering should stay supported"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected array-pattern match write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_match_expression_object_pattern_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(obj) {
                    let mut x = 1
                    let y = match obj {
                        { left: l, right: r } => {
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
            "object-pattern match lowering should stay supported"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected object-pattern match write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_match_expression_constructor_pattern_write_while_borrowed_is_visible_to_solver()
    {
        let lowering = lower_parsed_function(
            r#"
                function test(opt) {
                    let mut x = 1
                    let y = match opt {
                        Some(v) => {
                            let shared = &x
                            x = 2
                        }
                        None => 0
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "constructor-pattern match lowering should stay supported"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected constructor-pattern match write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_destructure_var_decl_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(pair) {
                    let [left, right] = pair
                    let shared = &left
                    left = 2
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "array destructuring declarations should stay supported"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected destructuring declaration write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_destructure_param_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test([left, right]) {
                    let shared = &left
                    left = 2
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "array destructuring parameters should stay supported"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected destructured-parameter write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_destructure_assignment_stays_supported() {
        let pair_param = ast::FunctionParameter {
            pattern: DestructurePattern::Identifier("pair".to_string(), span()),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: None,
            default_value: None,
        };
        let body = vec![
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("left".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: true,
                    pattern: DestructurePattern::Identifier("right".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Literal(ast::Literal::Int(2), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::Assignment(
                ast::Assignment {
                    pattern: DestructurePattern::Array(vec![
                        DestructurePattern::Identifier("left".to_string(), span()),
                        DestructurePattern::Identifier("right".to_string(), span()),
                    ]),
                    value: Expr::Identifier("pair".to_string(), span()),
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
                        expr: Box::new(Expr::Identifier("left".to_string(), span())),
                        is_mutable: false,
                        span: span(),
                    }),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
            Statement::Assignment(
                ast::Assignment {
                    pattern: DestructurePattern::Identifier("left".to_string(), span()),
                    value: Expr::Literal(ast::Literal::Int(3), span()),
                },
                span(),
            ),
        ];
        let lowering = lower_function_detailed("test", &[pair_param], &body, span());
        assert!(
            !lowering.had_fallbacks,
            "destructuring assignments should stay supported"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected destructuring assignment write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_destructure_rest_pattern_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(items) {
                    let [head, ...tail] = items
                    let shared = &tail
                    tail = items
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "rest destructuring should stay supported"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected rest destructuring write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_decomposition_pattern_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(merged) {
                    let (left: {x}, right: {y}) = merged
                    let shared = &left
                    left = merged
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "decomposition patterns should stay supported"
        );
        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected decomposition-pattern write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_supported_runtime_opaque_expressions_stay_supported() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "digits".to_string(),
            Expr::Literal(ast::Literal::Int(2), span()),
        );
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
                    pattern: DestructurePattern::Identifier("arr".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Array(
                        vec![
                            Expr::Identifier("x".to_string(), span()),
                            Expr::Literal(ast::Literal::Int(2), span()),
                        ],
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
                    pattern: DestructurePattern::Identifier("obj".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Object(
                        vec![
                            ast::ObjectEntry::Field {
                                key: "left".to_string(),
                                value: Expr::Identifier("x".to_string(), span()),
                                type_annotation: None,
                            },
                            ast::ObjectEntry::Spread(Expr::Identifier("arr".to_string(), span())),
                        ],
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
                    pattern: DestructurePattern::Identifier("unary".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::UnaryOp {
                        op: ast::UnaryOp::Neg,
                        operand: Box::new(Expr::Identifier("x".to_string(), span())),
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
                    pattern: DestructurePattern::Identifier("fuzzy".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::FuzzyComparison {
                        left: Box::new(Expr::Identifier("x".to_string(), span())),
                        op: ast::operators::FuzzyOp::Equal,
                        right: Box::new(Expr::Literal(ast::Literal::Int(1), span())),
                        tolerance: ast::operators::FuzzyTolerance::Percentage(0.02),
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
                    pattern: DestructurePattern::Identifier("slice".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::IndexAccess {
                        object: Box::new(Expr::Identifier("arr".to_string(), span())),
                        index: Box::new(Expr::Literal(ast::Literal::Int(0), span())),
                        end_index: Some(Box::new(Expr::Literal(ast::Literal::Int(1), span()))),
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
                    pattern: DestructurePattern::Identifier("asserted".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::TypeAssertion {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        type_annotation: ast::TypeAnnotation::Basic("int".to_string()),
                        meta_param_overrides: Some(overrides),
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
                    pattern: DestructurePattern::Identifier("instance".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::InstanceOf {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        type_annotation: ast::TypeAnnotation::Basic("int".to_string()),
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
                    pattern: DestructurePattern::Identifier("variant".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::EnumConstructor {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        payload: ast::EnumConstructorPayload::Tuple(vec![Expr::Identifier(
                            "x".to_string(),
                            span(),
                        )]),
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
                    pattern: DestructurePattern::Identifier("call".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::MethodCall {
                        receiver: Box::new(Expr::Identifier("obj".to_string(), span())),
                        method: "touch".to_string(),
                        args: vec![Expr::Identifier("x".to_string(), span())],
                        named_args: vec![(
                            "tail".to_string(),
                            Expr::IndexAccess {
                                object: Box::new(Expr::Identifier("arr".to_string(), span())),
                                index: Box::new(Expr::Literal(ast::Literal::Int(0), span())),
                                end_index: Some(Box::new(Expr::Literal(
                                    ast::Literal::Int(1),
                                    span(),
                                ))),
                                span: span(),
                            },
                        )],
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
                    pattern: DestructurePattern::Identifier("range".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Range {
                        start: Some(Box::new(Expr::Literal(ast::Literal::Int(0), span()))),
                        end: Some(Box::new(Expr::Identifier("x".to_string(), span()))),
                        kind: ast::RangeKind::Exclusive,
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
                    pattern: DestructurePattern::Identifier("contextual".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::TimeframeContext {
                        timeframe: ast::Timeframe::new(5, ast::TimeframeUnit::Minute),
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
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
                    pattern: DestructurePattern::Identifier("using_impl".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::UsingImpl {
                        expr: Box::new(Expr::Identifier("x".to_string(), span())),
                        impl_name: "Tracked".to_string(),
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
                    pattern: DestructurePattern::Identifier("simulation".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::SimulationCall {
                        name: "sim".to_string(),
                        params: vec![(
                            "value".to_string(),
                            Expr::Identifier("x".to_string(), span()),
                        )],
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
                    pattern: DestructurePattern::Identifier("struct_lit".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::StructLiteral {
                        type_name: "Point".to_string(),
                        fields: vec![("x".to_string(), Expr::Identifier("x".to_string(), span()))],
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
                    pattern: DestructurePattern::Identifier("annotated".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Annotated {
                        annotation: ast::Annotation {
                            name: "trace".to_string(),
                            args: vec![Expr::Identifier("x".to_string(), span())],
                            span: span(),
                        },
                        target: Box::new(Expr::Identifier("x".to_string(), span())),
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
                    pattern: DestructurePattern::Identifier("rows".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::TableRows(
                        vec![
                            vec![
                                Expr::Identifier("x".to_string(), span()),
                                Expr::Literal(ast::Literal::Int(2), span()),
                            ],
                            vec![
                                Expr::Literal(ast::Literal::Int(3), span()),
                                Expr::Literal(ast::Literal::Int(4), span()),
                            ],
                        ],
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
            "supported runtime expression families should stay on the MIR path"
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

    #[test]
    fn test_lowered_async_let_exclusive_ref_task_boundary_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                async function test() {
                    let mut x = 1
                    async let fut = &mut x
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "async let with direct ref capture should stay in the supported MIR subset"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected task-boundary exclusive-ref error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_async_let_nested_ref_binding_task_boundary_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                async function test() {
                    let mut x = 1
                    async let fut = {
                        let r = &mut x
                        r
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "async let block bodies should stay in the supported MIR subset"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected nested task-boundary exclusive-ref error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_async_let_shared_ref_task_boundary_stays_clean() {
        let lowering = lower_parsed_function(
            r#"
                async function test() {
                    let x = 1
                    async let fut = &x
                    await fut
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "shared-ref async let should stay in the supported MIR subset"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            !analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "shared refs should not trigger task-boundary exclusivity errors, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_join_exclusive_ref_task_boundary_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                async function test() {
                    let mut x = 1
                    await join all {
                        &mut x,
                        2,
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "join branches should stay in the supported MIR subset"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected join task-boundary exclusive-ref error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_lowered_async_scope_with_async_let_stays_supported() {
        let lowering = lower_parsed_function(
            r#"
                async function test() {
                    let x = 1
                    async scope {
                        async let fut = &x
                        await fut
                    }
                }
            "#,
        );
        assert!(
            !lowering.had_fallbacks,
            "async scope with supported async forms should stay in the MIR subset"
        );

        let analysis = solver::analyze(&lowering.mir);
        assert!(
            analysis.errors.is_empty(),
            "shared async-scope captures should stay borrow-clean, got {:?}",
            analysis.errors
        );
    }
}
