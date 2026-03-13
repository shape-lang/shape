//! MIR lowering: AST -> MIR.
//!
//! Converts Shape AST function bodies into MIR basic blocks.
//! This is the bridge between parsing and borrow analysis.
//!
//! ## Module structure
//!
//! - [`mod.rs`](self) -- Public API (`lower_function`, `lower_function_detailed`,
//!   `compute_mutability_errors`), `MirBuilder` struct and its state machine.
//! - [`expr`] -- Expression lowering (`lower_expr_to_temp` and its many helpers).
//! - [`stmt`] -- Statement lowering (variable decls, assignments, control flow,
//!   pattern destructuring).
//! - [`helpers`] -- Shared utilities: generic container store emission, operand
//!   collection, place projection, type inference from expressions.

mod expr;
mod helpers;
mod stmt;

use super::types::*;
use crate::mir::analysis::MutabilityError;
use shape_ast::ast::{self, Span, Statement};
use std::collections::{HashMap, HashSet};


#[derive(Debug, Clone, Copy)]
pub(super) struct MirLoopContext {
    pub(super) break_block: BasicBlockId,
    pub(super) continue_block: BasicBlockId,
    pub(super) break_value_slot: Option<SlotId>,
}

#[derive(Debug, Clone)]
struct TaskBoundaryCaptureScope {
    outer_locals_cutoff: u16,
    operands: Vec<Operand>,
}

#[derive(Debug, Clone)]
struct MirLocalRecord {
    name: String,
    type_info: LocalTypeInfo,
    binding_info: Option<LoweredBindingInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredBindingInfo {
    pub slot: SlotId,
    pub name: String,
    pub declaration_span: Span,
    pub enforce_immutable_assignment: bool,
    pub is_explicit_let: bool,
    pub is_const: bool,
    pub initialization_point: Option<Point>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BindingMetadata {
    declaration_span: Span,
    enforce_immutable_assignment: bool,
    is_explicit_let: bool,
    is_const: bool,
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
    pub(super) current_block: BasicBlockId,
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
    /// Local variable name -> slot mapping.
    locals: Vec<MirLocalRecord>,
    /// Active local name -> slot mapping for place resolution.
    local_slots: HashMap<String, SlotId>,
    /// Stable field indices for property-place lowering.
    field_indices: HashMap<String, FieldIdx>,
    /// Next field index to allocate.
    next_field_idx: u16,
    /// Parameter slots.
    param_slots: Vec<SlotId>,
    /// Per-parameter reference kind, aligned with `param_slots`.
    param_reference_kinds: Vec<Option<BorrowKind>>,
    /// Named-local shadowing stack for lexical scopes.
    scope_bindings: Vec<Vec<(String, Option<SlotId>)>>,
    /// Active loop control-flow targets.
    loop_contexts: Vec<MirLoopContext>,
    /// Active task-boundary capture scopes for async lowering.
    task_boundary_capture_scopes: Vec<TaskBoundaryCaptureScope>,
    /// Nesting depth of `async scope` blocks -- nonzero means structured concurrency.
    pub(super) async_scope_depth: u32,
    /// Exit block for the enclosing function.
    exit_block: Option<BasicBlockId>,
    /// Function span.
    span: Span,
    /// Spans where lowering had to fall back to placeholder/Nop handling.
    /// Empty means clean lowering with no fallbacks.
    fallback_spans: Vec<Span>,
}

#[derive(Debug)]
pub struct MirLoweringResult {
    pub mir: MirFunction,
    pub had_fallbacks: bool,
    /// Spans where lowering fell back to placeholder handling.
    /// Used for span-granular error filtering in partial-authority mode.
    pub fallback_spans: Vec<Span>,
    pub binding_infos: Vec<LoweredBindingInfo>,
    /// Reverse map from field index -> field name (inverted from `field_indices`).
    pub field_names: HashMap<FieldIdx, String>,
    /// All named locals (params + bindings), excluding `__mir_*` temporaries.
    /// Used by callee summary filtering to detect local-name shadows.
    pub all_local_names: HashSet<String>,
}

// ---------------------------------------------------------------------------
// MirBuilder -- block and state machine management
// ---------------------------------------------------------------------------

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
            locals: vec![MirLocalRecord {
                name: "__mir_return".to_string(),
                type_info: LocalTypeInfo::Unknown,
                binding_info: None,
            }],
            local_slots: HashMap::new(),
            field_indices: HashMap::new(),
            next_field_idx: 0,
            param_slots: Vec::new(),
            param_reference_kinds: Vec::new(),
            scope_bindings: vec![Vec::new()],
            loop_contexts: Vec::new(),
            task_boundary_capture_scopes: Vec::new(),
            async_scope_depth: 0,
            exit_block: None,
            span,
            fallback_spans: Vec::new(),
        }
    }

    /// Allocate a new local variable slot.
    pub fn alloc_local(&mut self, name: String, type_info: LocalTypeInfo) -> SlotId {
        self.alloc_local_with_binding(name, type_info, None)
    }

    pub(super) fn alloc_local_binding(
        &mut self,
        name: String,
        type_info: LocalTypeInfo,
        binding_metadata: BindingMetadata,
    ) -> SlotId {
        self.alloc_local_with_binding(name, type_info, Some(binding_metadata))
    }

    fn alloc_local_with_binding(
        &mut self,
        name: String,
        type_info: LocalTypeInfo,
        binding_metadata: Option<BindingMetadata>,
    ) -> SlotId {
        let slot = SlotId(self.next_local);
        self.next_local += 1;
        let binding_info = binding_metadata.map(|binding_metadata| LoweredBindingInfo {
            slot,
            name: name.clone(),
            declaration_span: binding_metadata.declaration_span,
            enforce_immutable_assignment: binding_metadata.enforce_immutable_assignment,
            is_explicit_let: binding_metadata.is_explicit_let,
            is_const: binding_metadata.is_const,
            initialization_point: None,
        });
        self.locals.push(MirLocalRecord {
            name,
            type_info,
            binding_info,
        });
        if let Some(local) = self.locals.last()
            && !local.name.starts_with("__mir_")
        {
            self.bind_named_local(local.name.clone(), slot);
        }
        slot
    }

    /// Allocate a temporary local slot that should not participate in name resolution.
    pub fn alloc_temp(&mut self, type_info: LocalTypeInfo) -> SlotId {
        let name = format!("__mir_tmp{}", self.next_local);
        self.alloc_local(name, type_info)
    }

    /// Register a parameter slot.
    fn add_param(
        &mut self,
        name: String,
        type_info: LocalTypeInfo,
        reference_kind: Option<BorrowKind>,
        binding_metadata: Option<BindingMetadata>,
    ) -> SlotId {
        let slot = self.alloc_local_with_binding(name, type_info, binding_metadata);
        self.param_slots.push(slot);
        self.param_reference_kinds.push(reference_kind);
        slot
    }

    /// Look up the current slot for a named local.
    pub fn lookup_local(&self, name: &str) -> Option<SlotId> {
        self.local_slots.get(name).copied()
    }

    pub fn visible_named_locals(&self) -> Vec<String> {
        self.local_slots
            .keys()
            .filter(|name| !name.starts_with("__mir_"))
            .cloned()
            .collect()
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
        // Legacy: called without a span. Use the current function span as fallback.
        self.fallback_spans.push(self.span);
    }

    pub fn mark_fallback_at(&mut self, span: Span) {
        self.fallback_spans.push(span);
    }

    pub fn had_fallbacks(&self) -> bool {
        !self.fallback_spans.is_empty()
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

    pub(super) fn current_loop(&self) -> Option<MirLoopContext> {
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
            if !helpers::operand_crosses_task_boundary(scope.outer_locals_cutoff, &operand) {
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
    pub fn push_stmt(&mut self, kind: StatementKind, span: Span) -> Point {
        let point = self.next_point();
        self.current_stmts.push(MirStatement { kind, span, point });
        point
    }

    pub fn record_binding_initialization(&mut self, slot: SlotId, point: Point) {
        if let Some(local) = self.locals.get_mut(slot.0 as usize)
            && let Some(binding_info) = local.binding_info.as_mut()
        {
            binding_info.initialization_point = Some(point);
        }
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

    /// Emit a function call as a block terminator. Finishes current block
    /// with TerminatorKind::Call and starts a continuation block.
    pub fn emit_call(
        &mut self,
        func: Operand,
        args: Vec<Operand>,
        destination: Place,
        span: Span,
    ) {
        let next_bb = self.new_block();
        self.finish_block(
            TerminatorKind::Call {
                func,
                args,
                destination,
                next: next_bb,
            },
            span,
        );
        self.start_block(next_bb);
    }

    /// Finalize and produce the MIR function.
    pub fn build(self) -> MirLoweringResult {
        let local_types = self
            .locals
            .iter()
            .map(|local| local.type_info.clone())
            .collect();
        let binding_infos = self
            .locals
            .iter()
            .filter_map(|local| local.binding_info.clone())
            .collect();
        let field_names: HashMap<FieldIdx, String> = self
            .field_indices
            .iter()
            .map(|(name, &idx)| (idx, name.clone()))
            .collect();
        // Sort blocks by ID so that MirFunction::block(id) can index by id.0
        let mut blocks = self.blocks;
        blocks.sort_by_key(|b| b.id.0);

        let had_fallbacks = !self.fallback_spans.is_empty();
        let fallback_spans = self.fallback_spans;
        let all_local_names: HashSet<String> = self
            .locals
            .iter()
            .filter(|l| !l.name.starts_with("__mir_"))
            .map(|l| l.name.clone())
            .collect();

        MirLoweringResult {
            mir: MirFunction {
                name: self.name,
                blocks,
                num_locals: self.next_local,
                param_slots: self.param_slots,
                param_reference_kinds: self.param_reference_kinds,
                local_types,
                span: self.span,
            },
            had_fallbacks,
            fallback_spans,
            binding_infos,
            field_names,
            all_local_names,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub(super) fn immutable_binding_metadata(
    declaration_span: Span,
    is_explicit_let: bool,
    is_const: bool,
) -> BindingMetadata {
    BindingMetadata {
        declaration_span,
        enforce_immutable_assignment: true,
        is_explicit_let,
        is_const,
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
        let reference_kind = if param.is_mut_reference {
            Some(BorrowKind::Exclusive)
        } else if param.is_reference {
            Some(BorrowKind::Shared)
        } else {
            None
        };
        let binding_metadata = if param.is_const {
            Some(immutable_binding_metadata(param.span(), false, true))
        } else if matches!(reference_kind, Some(BorrowKind::Shared)) {
            Some(immutable_binding_metadata(param.span(), false, false))
        } else {
            None
        };
        if let Some(param_name) = param.simple_name() {
            builder.add_param(
                param_name.to_string(),
                type_info,
                reference_kind,
                binding_metadata,
            );
        } else {
            let slot = builder.add_param(
                format!("__mir_param{}", builder.param_slots.len()),
                type_info,
                reference_kind,
                None,
            );
            stmt::lower_destructure_bindings_from_place(
                &mut builder,
                &param.pattern,
                &Place::Local(slot),
                param.span(),
                binding_metadata,
            );
        }
    }

    // Create the exit block
    let exit_block = builder.new_block();
    builder.set_exit_block(exit_block);

    // Lower body statements
    stmt::lower_statements(&mut builder, body, exit_block);

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

pub fn compute_mutability_errors(lowering: &MirLoweringResult) -> Vec<MutabilityError> {
    let tracked_bindings: HashMap<SlotId, &LoweredBindingInfo> = lowering
        .binding_infos
        .iter()
        .filter(|binding| binding.enforce_immutable_assignment)
        .map(|binding| (binding.slot, binding))
        .collect();
    let mut errors = Vec::new();

    for block in &lowering.mir.blocks {
        for stmt in &block.statements {
            let StatementKind::Assign(place, _) = &stmt.kind else {
                continue;
            };
            let root = place.root_local();
            let Some(binding) = tracked_bindings.get(&root) else {
                continue;
            };
            let is_declaration_init = matches!(place, Place::Local(slot) if *slot == root)
                && binding.initialization_point == Some(stmt.point);
            if is_declaration_init {
                continue;
            }
            errors.push(MutabilityError {
                span: stmt.span,
                variable_name: binding.name.clone(),
                declaration_span: binding.declaration_span,
                is_explicit_let: binding.is_explicit_let,
                is_const: binding.is_const,
            });
        }
    }

    errors
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::analysis::BorrowErrorKind;
    use crate::mir::cfg::ControlFlowGraph;
    use crate::mir::liveness;
    use crate::mir::solver;
    use shape_ast::ast::{self, DestructurePattern, Expr, OwnershipModifier, VarKind};

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
    fn test_compute_mutability_errors_ignores_binding_initializer() {
        let lowering = lower_parsed_function(
            r#"
                function keep() {
                    let x = 1
                    x
                }
            "#,
        );
        let errors = compute_mutability_errors(&lowering);
        assert!(
            errors.is_empty(),
            "declaration initializer should not be reported as a mutability error: {:?}",
            errors
        );
    }

    #[test]
    fn test_compute_mutability_errors_flags_immutable_let_reassignment() {
        let lowering = lower_parsed_function(
            r#"
                function mutate() {
                    let x = 1
                    x = 2
                    x
                }
            "#,
        );
        let errors = compute_mutability_errors(&lowering);
        assert_eq!(
            errors.len(),
            1,
            "expected one mutability error, got {errors:?}"
        );
        assert_eq!(errors[0].variable_name, "x");
        assert!(errors[0].is_explicit_let);
    }

    #[test]
    fn test_compute_mutability_errors_flags_const_reassignment() {
        let lowering = lower_parsed_function(
            r#"
                function mutate() {
                    const x = 1
                    x = 2
                    x
                }
            "#,
        );
        let errors = compute_mutability_errors(&lowering);
        assert_eq!(
            errors.len(),
            1,
            "expected one mutability error, got {errors:?}"
        );
        assert_eq!(errors[0].variable_name, "x");
        assert!(errors[0].is_const);
    }

    #[test]
    fn test_compute_mutability_errors_flags_shared_ref_param_write() {
        let lowering = lower_parsed_function(
            r#"
                function mutate(&x) {
                    x = 2
                    x
                }
            "#,
        );
        let errors = compute_mutability_errors(&lowering);
        assert_eq!(
            errors.len(),
            1,
            "expected one mutability error, got {errors:?}"
        );
        assert_eq!(errors[0].variable_name, "x");
        assert!(!errors[0].is_explicit_let);
    }

    #[test]
    fn test_compute_mutability_errors_flags_const_param_write() {
        let lowering = lower_parsed_function(
            r#"
                function mutate(const x) {
                    x = 2
                    x
                }
            "#,
        );
        let errors = compute_mutability_errors(&lowering);
        assert_eq!(
            errors.len(),
            1,
            "expected one mutability error, got {errors:?}"
        );
        assert_eq!(errors[0].variable_name, "x");
        assert!(errors[0].is_const);
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
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("kept".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Identifier("shared".to_string(), span())),
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
            Statement::Return(Some(Expr::Identifier("shared".to_string(), span())), span()),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir, &Default::default());
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
            Statement::VariableDecl(
                ast::VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("kept".to_string(), span()),
                    type_annotation: None,
                    value: Some(Expr::Identifier("shared".to_string(), span())),
                    ownership: OwnershipModifier::Inferred,
                },
                span(),
            ),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir, &Default::default());
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
            Statement::Expression(Expr::Identifier("shared".to_string(), span()), span()),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir, &Default::default());
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
            Statement::Expression(Expr::Identifier("exclusive".to_string(), span()), span()),
        ];
        let mir = lower_function("test", &[], &body, span());
        let analysis = solver::analyze(&mir, &Default::default());
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
        let analysis = solver::analyze(&mir, &Default::default());
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
    fn test_lowered_array_direct_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let arr = [&x]
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_array_indirect_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let r = &x
                    let arr = [r]
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_object_direct_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let obj = { value: &x }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_object_indirect_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let r = &x
                    let obj = { value: r }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_struct_direct_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let point = Point { value: &x }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_struct_indirect_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let r = &x
                    let point = Point { value: r }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_enum_tuple_direct_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let value = Maybe::Some(&x)
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_enum_tuple_indirect_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let r = &x
                    let value = Maybe::Some(r)
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_enum_struct_direct_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let value = Maybe::Err { code: &x }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_enum_struct_indirect_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let x = 1
                    let r = &x
                    let value = Maybe::Err { code: r }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
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
        let analysis = solver::analyze(&mir, &Default::default());
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
                        shared
                        0
                    }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
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
                        shared
                        0
                    }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_loop_expr_break_value_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test() {
                    let mut x = 1
                    let y = loop {
                        let shared = &x
                        x = 2
                        shared
                        break 0
                    }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
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
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
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
                            shared
                            0
                        }
                        _ => 0
                    }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
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
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
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
                            shared
                            0
                        }
                        _ => 0
                    }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
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
                            shared
                            0
                        }
                        _ => 0
                    }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
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
                            shared
                            0
                        }
                        None => 0
                    }
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_destructure_var_decl_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(pair) {
                    var [left, right] = pair
                    let shared = &left
                    left = 2
                    shared
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_destructure_param_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test([left, right]) {
                    let mut left_copy = left
                    let shared = &left_copy
                    left_copy = 2
                    shared
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
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
            Statement::Expression(Expr::Identifier("shared".to_string(), span()), span()),
        ];
        let lowering = lower_function_detailed("test", &[pair_param], &body, span());
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_destructure_rest_pattern_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(items) {
                    var [head, ...tail] = items
                    let shared = &tail
                    tail = items
                    shared
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_decomposition_pattern_write_while_borrowed_is_visible_to_solver() {
        let lowering = lower_parsed_function(
            r#"
                function test(merged) {
                    var (left: {x}, right: {y}) = merged
                    let shared = &left
                    left = merged
                    shared
                }
            "#,
        );
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_supported_runtime_opaque_expressions_stay_supported() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "digits".to_string(),
            Expr::Literal(ast::Literal::Int(2), span()),
        );
        let body = vec![
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("x".to_string(), span()), type_annotation: None, value: Some(Expr::Literal(ast::Literal::Int(1), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("arr".to_string(), span()), type_annotation: None, value: Some(Expr::Array(vec![Expr::Identifier("x".to_string(), span()), Expr::Literal(ast::Literal::Int(2), span())], span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("obj".to_string(), span()), type_annotation: None, value: Some(Expr::Object(vec![ast::ObjectEntry::Field { key: "left".to_string(), value: Expr::Identifier("x".to_string(), span()), type_annotation: None }, ast::ObjectEntry::Spread(Expr::Identifier("arr".to_string(), span()))], span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("unary".to_string(), span()), type_annotation: None, value: Some(Expr::UnaryOp { op: ast::UnaryOp::Neg, operand: Box::new(Expr::Identifier("x".to_string(), span())), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("fuzzy".to_string(), span()), type_annotation: None, value: Some(Expr::FuzzyComparison { left: Box::new(Expr::Identifier("x".to_string(), span())), op: ast::operators::FuzzyOp::Equal, right: Box::new(Expr::Literal(ast::Literal::Int(1), span())), tolerance: ast::operators::FuzzyTolerance::Percentage(0.02), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("slice".to_string(), span()), type_annotation: None, value: Some(Expr::IndexAccess { object: Box::new(Expr::Identifier("arr".to_string(), span())), index: Box::new(Expr::Literal(ast::Literal::Int(0), span())), end_index: Some(Box::new(Expr::Literal(ast::Literal::Int(1), span()))), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("asserted".to_string(), span()), type_annotation: None, value: Some(Expr::TypeAssertion { expr: Box::new(Expr::Identifier("x".to_string(), span())), type_annotation: ast::TypeAnnotation::Basic("int".to_string()), meta_param_overrides: Some(overrides), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("instance".to_string(), span()), type_annotation: None, value: Some(Expr::InstanceOf { expr: Box::new(Expr::Identifier("x".to_string(), span())), type_annotation: ast::TypeAnnotation::Basic("int".to_string()), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("variant".to_string(), span()), type_annotation: None, value: Some(Expr::EnumConstructor { enum_name: "Option".to_string(), variant: "Some".to_string(), payload: ast::EnumConstructorPayload::Tuple(vec![Expr::Identifier("x".to_string(), span())]), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("call".to_string(), span()), type_annotation: None, value: Some(Expr::MethodCall { receiver: Box::new(Expr::Identifier("obj".to_string(), span())), method: "touch".to_string(), args: vec![Expr::Identifier("x".to_string(), span())], named_args: vec![("tail".to_string(), Expr::IndexAccess { object: Box::new(Expr::Identifier("arr".to_string(), span())), index: Box::new(Expr::Literal(ast::Literal::Int(0), span())), end_index: Some(Box::new(Expr::Literal(ast::Literal::Int(1), span()))), span: span() })], optional: false, span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("range".to_string(), span()), type_annotation: None, value: Some(Expr::Range { start: Some(Box::new(Expr::Literal(ast::Literal::Int(0), span()))), end: Some(Box::new(Expr::Identifier("x".to_string(), span()))), kind: ast::RangeKind::Exclusive, span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("contextual".to_string(), span()), type_annotation: None, value: Some(Expr::TimeframeContext { timeframe: ast::Timeframe::new(5, ast::TimeframeUnit::Minute), expr: Box::new(Expr::Identifier("x".to_string(), span())), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("using_impl".to_string(), span()), type_annotation: None, value: Some(Expr::UsingImpl { expr: Box::new(Expr::Identifier("x".to_string(), span())), impl_name: "Tracked".to_string(), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("simulation".to_string(), span()), type_annotation: None, value: Some(Expr::SimulationCall { name: "sim".to_string(), params: vec![("value".to_string(), Expr::Identifier("x".to_string(), span()))], span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("struct_lit".to_string(), span()), type_annotation: None, value: Some(Expr::StructLiteral { type_name: "Point".to_string(), fields: vec![("x".to_string(), Expr::Identifier("x".to_string(), span()))], span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("annotated".to_string(), span()), type_annotation: None, value: Some(Expr::Annotated { annotation: ast::Annotation { name: "trace".to_string(), args: vec![Expr::Identifier("x".to_string(), span())], span: span() }, target: Box::new(Expr::Identifier("x".to_string(), span())), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("rows".to_string(), span()), type_annotation: None, value: Some(Expr::TableRows(vec![vec![Expr::Identifier("x".to_string(), span()), Expr::Literal(ast::Literal::Int(2), span())], vec![Expr::Literal(ast::Literal::Int(3), span()), Expr::Literal(ast::Literal::Int(4), span())]], span())), ownership: OwnershipModifier::Inferred }, span()),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(!lowering.had_fallbacks);
    }

    #[test]
    fn test_lowered_assignment_expr_write_while_borrowed_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: true, pattern: DestructurePattern::Identifier("x".to_string(), span()), type_annotation: None, value: Some(Expr::Literal(ast::Literal::Int(1), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("shared".to_string(), span()), type_annotation: None, value: Some(Expr::Reference { expr: Box::new(Expr::Identifier("x".to_string(), span())), is_mutable: false, span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("y".to_string(), span()), type_annotation: None, value: Some(Expr::Assign(Box::new(ast::AssignExpr { target: Box::new(Expr::Identifier("x".to_string(), span())), value: Box::new(Expr::Literal(ast::Literal::Int(2), span())) }), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::Return(Some(Expr::Identifier("shared".to_string(), span())), span()),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_property_assignment_expr_preserves_disjoint_places() {
        let body = vec![
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: true, pattern: DestructurePattern::Identifier("pair".to_string(), span()), type_annotation: None, value: Some(Expr::Literal(ast::Literal::String("pair".to_string()), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("left".to_string(), span()), type_annotation: None, value: Some(Expr::Reference { expr: Box::new(Expr::PropertyAccess { object: Box::new(Expr::Identifier("pair".to_string(), span())), property: "left".to_string(), optional: false, span: span() }), is_mutable: false, span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::Expression(Expr::Assign(Box::new(ast::AssignExpr { target: Box::new(Expr::PropertyAccess { object: Box::new(Expr::Identifier("pair".to_string(), span())), property: "right".to_string(), optional: false, span: span() }), value: Box::new(Expr::Literal(ast::Literal::String("updated".to_string()), span())) }), span()), span()),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_property_assignment_direct_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            function test() {
                var obj = { value: 0 }
                let x = 1
                obj.value = &x
                0
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_property_assignment_indirect_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            function test() {
                var obj = { value: 0 }
                let x = 1
                let r = &x
                obj.value = r
                0
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_index_assignment_direct_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            function test() {
                var arr = [0]
                let x = 1
                arr[0] = &x
                0
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_index_assignment_indirect_ref_escape_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            function test() {
                var arr = [0]
                let x = 1
                let r = &x
                arr[0] = r
                0
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_block_expr_write_while_borrowed_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: true, pattern: DestructurePattern::Identifier("x".to_string(), span()), type_annotation: None, value: Some(Expr::Literal(ast::Literal::Int(1), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("shared".to_string(), span()), type_annotation: None, value: Some(Expr::Block(ast::BlockExpr { items: vec![ast::BlockItem::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("inner".to_string(), span()), type_annotation: None, value: Some(Expr::Reference { expr: Box::new(Expr::Identifier("x".to_string(), span())), is_mutable: false, span: span() }), ownership: OwnershipModifier::Inferred }), ast::BlockItem::Expression(Expr::Identifier("inner".to_string(), span()))] }, span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::Assignment(ast::Assignment { pattern: DestructurePattern::Identifier("x".to_string(), span()), value: Expr::Literal(ast::Literal::Int(2), span()) }, span()),
            Statement::Expression(Expr::Identifier("shared".to_string(), span()), span()),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_let_expr_write_while_borrowed_is_visible_to_solver() {
        let body = vec![
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: true, pattern: DestructurePattern::Identifier("x".to_string(), span()), type_annotation: None, value: Some(Expr::Literal(ast::Literal::Int(1), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("shared".to_string(), span()), type_annotation: None, value: Some(Expr::Let(Box::new(ast::LetExpr { pattern: ast::Pattern::Identifier("inner".to_string()), type_annotation: None, value: Some(Box::new(Expr::Reference { expr: Box::new(Expr::Identifier("x".to_string(), span())), is_mutable: false, span: span() })), body: Box::new(Expr::Identifier("inner".to_string(), span())) }), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::Assignment(ast::Assignment { pattern: DestructurePattern::Identifier("x".to_string(), span()), value: Expr::Literal(ast::Literal::Int(2), span()) }, span()),
            Statement::Expression(Expr::Identifier("shared".to_string(), span()), span()),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_if_expression_with_block_branches_stays_supported() {
        let block_branch = |borrow_name: &str| {
            Expr::Block(ast::BlockExpr { items: vec![ast::BlockItem::Expression(Expr::Reference { expr: Box::new(Expr::Identifier(borrow_name.to_string(), span())), is_mutable: false, span: span() })] }, span())
        };
        let body = vec![
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: true, pattern: DestructurePattern::Identifier("x".to_string(), span()), type_annotation: None, value: Some(Expr::Literal(ast::Literal::Int(1), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("flag".to_string(), span()), type_annotation: None, value: Some(Expr::Literal(ast::Literal::Bool(true), span())), ownership: OwnershipModifier::Inferred }, span()),
            Statement::VariableDecl(ast::VariableDecl { kind: VarKind::Let, is_mut: false, pattern: DestructurePattern::Identifier("shared".to_string(), span()), type_annotation: None, value: Some(Expr::Conditional { condition: Box::new(Expr::Identifier("flag".to_string(), span())), then_expr: Box::new(block_branch("x")), else_expr: Some(Box::new(block_branch("x"))), span: span() }), ownership: OwnershipModifier::Inferred }, span()),
            Statement::Assignment(ast::Assignment { pattern: DestructurePattern::Identifier("x".to_string(), span()), value: Expr::Literal(ast::Literal::Int(2), span()) }, span()),
        ];
        let lowering = lower_function_detailed("test", &[], &body, span());
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_async_let_exclusive_ref_task_boundary_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            async function test() {
                let mut x = 1
                async let fut = &mut x
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary));
    }

    #[test]
    fn test_lowered_async_let_nested_ref_binding_task_boundary_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            async function test() {
                let mut x = 1
                async let fut = {
                    let r = &mut x
                    r
                }
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary));
    }

    #[test]
    fn test_lowered_async_let_shared_ref_task_boundary_stays_clean() {
        let lowering = lower_parsed_function(r#"
            async function test() {
                let x = 1
                async let fut = &x
                await fut
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(!analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary));
    }

    #[test]
    fn test_lowered_join_exclusive_ref_task_boundary_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            async function test() {
                let mut x = 1
                await join all {
                    &mut x,
                    2,
                }
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary));
    }

    #[test]
    fn test_lowered_async_scope_with_async_let_stays_supported() {
        let lowering = lower_parsed_function(r#"
            async function test() {
                let x = 1
                async scope {
                    async let fut = &x
                    await fut
                }
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_closure_capture_of_reference_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            function test() {
                let x = 1
                let r = &x
                let f = || r
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_returned_array_with_ref_still_errors() {
        let lowering = lower_parsed_function(r#"
            function test() {
                let x = 1
                let arr = [&x]
                return arr
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::ReferenceStoredInArray));
    }

    #[test]
    fn test_lowered_returned_closure_with_ref_still_errors() {
        let lowering = lower_parsed_function(r#"
            function test() {
                let x = 1
                let r = &x
                let f = || r
                return f
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::ReferenceEscapeIntoClosure));
    }

    #[test]
    fn test_lowered_closure_capture_of_owned_value_stays_clean() {
        let lowering = lower_parsed_function(r#"
            function test() {
                let x = 1
                let f = || x
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.is_empty());
    }

    #[test]
    fn test_lowered_list_comprehension_write_conflict_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            function test() {
                let mut x = 1
                let r = &x
                let xs = [(x = 2) for y in [1]]
                r
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_from_query_write_conflict_is_visible_to_solver() {
        let lowering = lower_parsed_function(r#"
            function test() {
                let mut x = 1
                let r = &x
                let rows = from y in [1] where (x = 2) > 0 select y
                r
            }
        "#);
        assert!(!lowering.had_fallbacks);
        let analysis = solver::analyze(&lowering.mir, &Default::default());
        assert!(analysis.errors.iter().any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed));
    }

    #[test]
    fn test_lowered_comptime_expr_stays_supported() {
        let lowering = lower_parsed_function(r#"
            function test() {
                let generated = comptime {
                    let x = 1
                }
            }
        "#);
        assert!(!lowering.had_fallbacks);
    }

    #[test]
    fn test_lowered_comptime_for_expr_stays_supported() {
        let lowering = lower_parsed_function(r#"
            function test() {
                let generated = comptime for f in [1, 2] {
                    let y = f
                }
            }
        "#);
        assert!(!lowering.had_fallbacks);
    }
}
