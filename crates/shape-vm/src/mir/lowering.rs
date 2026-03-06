//! MIR lowering: AST → MIR.
//!
//! Converts Shape AST function bodies into MIR basic blocks.
//! This is the bridge between parsing and borrow analysis.

use super::types::*;
use shape_ast::ast::{self, Expr, Span, Spanned, Statement};

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
    /// Next block ID to allocate.
    next_block_id: u32,
    /// Next local slot to allocate.
    next_local: u16,
    /// Next program point.
    next_point: u32,
    /// Next loan ID.
    next_loan: u32,
    /// Local variable name → slot mapping.
    locals: Vec<(String, SlotId, LocalTypeInfo)>,
    /// Parameter slots.
    param_slots: Vec<SlotId>,
    /// Function span.
    span: Span,
}

impl MirBuilder {
    pub fn new(name: String, span: Span) -> Self {
        MirBuilder {
            name,
            blocks: Vec::new(),
            current_stmts: Vec::new(),
            current_block: BasicBlockId(0),
            next_block_id: 1,
            next_local: 0,
            next_point: 0,
            next_loan: 0,
            locals: Vec::new(),
            param_slots: Vec::new(),
            span,
        }
    }

    /// Allocate a new local variable slot.
    pub fn alloc_local(&mut self, name: String, type_info: LocalTypeInfo) -> SlotId {
        let slot = SlotId(self.next_local);
        self.next_local += 1;
        self.locals.push((name, slot, type_info));
        slot
    }

    /// Register a parameter slot.
    pub fn add_param(&mut self, name: String, type_info: LocalTypeInfo) -> SlotId {
        let slot = self.alloc_local(name, type_info);
        self.param_slots.push(slot);
        slot
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
    }

    /// Start building a new block (after finishing the previous one).
    pub fn start_block(&mut self, id: BasicBlockId) {
        self.current_block = id;
        self.current_stmts.clear();
    }

    /// Finalize and produce the MIR function.
    pub fn build(self) -> MirFunction {
        let local_types = self.locals.iter().map(|(_, _, t)| t.clone()).collect();
        MirFunction {
            name: self.name,
            blocks: self.blocks,
            num_locals: self.next_local,
            param_slots: self.param_slots,
            local_types,
            span: self.span,
        }
    }
}

/// Lower a function body (list of statements) into MIR.
pub fn lower_function(
    name: &str,
    params: &[ast::FunctionParameter],
    body: &[Statement],
    span: Span,
) -> MirFunction {
    let mut builder = MirBuilder::new(name.to_string(), span);

    // Register parameters
    for param in params {
        let param_name = param.simple_name().unwrap_or("_").to_string();
        let type_info = if param.is_reference {
            LocalTypeInfo::NonCopy // references are always tracked
        } else {
            LocalTypeInfo::Unknown // will be resolved during analysis
        };
        builder.add_param(param_name, type_info);
    }

    // Create the exit block
    let exit_block = builder.new_block();

    // Lower body statements
    lower_statements(&mut builder, body, exit_block);

    // If current block hasn't been finished (no explicit return), emit goto exit
    if builder.current_stmts.len() > 0 || builder.blocks.len() == 0 {
        builder.finish_block(TerminatorKind::Goto(exit_block), span);
    }

    // Create exit block with Return terminator
    builder.start_block(exit_block);
    builder.finish_block(TerminatorKind::Return, span);

    builder.build()
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
            if let Some(expr) = value {
                let result_slot = lower_expr_to_temp(builder, expr);
                builder.push_stmt(
                    StatementKind::Assign(
                        Place::Local(SlotId(0)), // return slot convention
                        Rvalue::Use(Operand::Move(Place::Local(result_slot))),
                    ),
                    *span,
                );
            }
            builder.finish_block(TerminatorKind::Return, *span);
            // Start a new unreachable block for subsequent dead code
            let dead_block = builder.new_block();
            builder.start_block(dead_block);
        }
        Statement::Expression(expr, span) => {
            // Expression statement — evaluate for side effects
            let _slot = lower_expr_to_temp(builder, expr);
            let _ = span; // span captured in sub-lowering
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
            builder.push_stmt(StatementKind::Nop, span);
        }
    }
}

/// Lower a variable declaration.
fn lower_var_decl(builder: &mut MirBuilder, decl: &ast::VariableDecl, span: Span) {
    let name = decl.pattern.as_identifier().unwrap_or("_").to_string();
    let type_info = LocalTypeInfo::Unknown; // resolved during analysis
    let slot = builder.alloc_local(name, type_info);

    if let Some(init_expr) = &decl.value {
        let init_slot = lower_expr_to_temp(builder, init_expr);
        // Determine operand based on ownership modifier
        let operand = match decl.ownership {
            ast::OwnershipModifier::Move => Operand::Move(Place::Local(init_slot)),
            ast::OwnershipModifier::Clone => Operand::Copy(Place::Local(init_slot)),
            ast::OwnershipModifier::Inferred => {
                // For `var`: decision deferred to liveness analysis
                // For `let`: default to Move
                Operand::Move(Place::Local(init_slot))
            }
        };
        let rvalue = match decl.ownership {
            ast::OwnershipModifier::Clone => Rvalue::Clone(operand),
            _ => Rvalue::Use(operand),
        };
        builder.push_stmt(StatementKind::Assign(Place::Local(slot), rvalue), span);
    }
}

/// Lower an assignment statement.
fn lower_assignment(builder: &mut MirBuilder, assign: &ast::Assignment, span: Span) {
    let value_slot = lower_expr_to_temp(builder, &assign.value);
    // Simplified: assume LHS is a simple identifier for now
    // Full place resolution will be added for field/index assignments
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(SlotId(0)), // placeholder - real resolution TBD
            Rvalue::Use(Operand::Move(Place::Local(value_slot))),
        ),
        span,
    );
}

/// Lower an expression and return the temp slot it was placed in.
/// This is a simplified version — full expression lowering will be more complex.
fn lower_expr_to_temp(builder: &mut MirBuilder, expr: &Expr) -> SlotId {
    let span = expr.span();
    let temp = builder.alloc_local("_tmp".to_string(), LocalTypeInfo::Unknown);

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
        Expr::Identifier(_, _) => {
            // Reference to a local — would resolve to actual slot
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::Use(Operand::Copy(Place::Local(SlotId(0)))),
                ),
                span,
            );
        }
        Expr::Reference {
            expr: inner,
            is_mutable,
            span: ref_span,
        } => {
            let inner_slot = lower_expr_to_temp(builder, inner);
            let kind = if *is_mutable {
                BorrowKind::Exclusive
            } else {
                BorrowKind::Shared
            };
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::Borrow(kind, Place::Local(inner_slot)),
                ),
                *ref_span,
            );
        }
        Expr::BinaryOp { left, right, .. } => {
            let l = lower_expr_to_temp(builder, left);
            let r = lower_expr_to_temp(builder, right);
            builder.push_stmt(
                StatementKind::Assign(
                    Place::Local(temp),
                    Rvalue::BinaryOp(
                        BinOp::Add, // simplified — real op from AST
                        Operand::Copy(Place::Local(l)),
                        Operand::Copy(Place::Local(r)),
                    ),
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
    lower_statements(builder, &if_stmt.then_body, exit_block);
    builder.finish_block(TerminatorKind::Goto(merge_block), span);

    // Else branch
    if let Some(else_body) = &if_stmt.else_body {
        builder.start_block(else_block);
        lower_statements(builder, else_body, exit_block);
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
    lower_statements(builder, body, exit_block);
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
    // Extract the iterable expression
    let iter_expr = match &for_loop.init {
        ast::ForInit::ForIn { iter, .. } => iter,
        ast::ForInit::ForC { condition, .. } => condition,
    };

    let _iter_slot = lower_expr_to_temp(builder, iter_expr);
    let header = builder.new_block();
    let body_block = builder.new_block();
    let after = builder.new_block();

    builder.finish_block(TerminatorKind::Goto(header), span);

    builder.start_block(header);
    builder.finish_block(
        TerminatorKind::SwitchBool {
            operand: Operand::Constant(MirConstant::Bool(true)),
            true_bb: body_block,
            false_bb: after,
        },
        span,
    );

    builder.start_block(body_block);
    lower_statements(builder, &for_loop.body, exit_block);
    builder.finish_block(TerminatorKind::Goto(header), span);

    builder.start_block(after);
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
    use crate::mir::cfg::ControlFlowGraph;
    use crate::mir::liveness;
    use shape_ast::ast::{self, DestructurePattern, OwnershipModifier, VarKind};

    fn span() -> Span {
        Span { start: 0, end: 1 }
    }

    #[test]
    fn test_lower_empty_function() {
        let mir = lower_function("empty", &[], &[], span());
        assert_eq!(mir.name, "empty");
        assert!(mir.blocks.len() >= 2); // entry + exit
        assert_eq!(mir.num_locals, 0);
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
                    value: Some(Expr::Literal(ast::Literal::Int(1), span())),
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
}
