//! Miscellaneous expression compilation (unit, spread, block, range, strategy, window, loops, join, comptime for)

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use shape_ast::ast::Expr;
use shape_ast::error::{Result, ShapeError};

use std::collections::HashSet;

use super::super::BytecodeCompiler;
use shape_ast::ast::expr_helpers::ComptimeForExpr;
use shape_ast::ast::statements::Statement as AstStatement;
use shape_ast::ast::{JoinExpr, JoinKind};

/// Recursively collect all assigned variable names in a list of statements.
///
/// Currently dead code: the only consumer was `compile_comptime_for`,
/// stubbed pending phase-2c (ADR-006 §2.4). Kept here so the comptime-for
/// unroll can be restored without re-deriving the helper.
#[allow(dead_code)]
fn collect_assigned_names(stmts: &[AstStatement], names: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            AstStatement::Assignment(assign, _) => {
                if let Some(name) = assign.pattern.as_identifier() {
                    names.insert(name.to_string());
                }
            }
            AstStatement::If(if_stmt, _) => {
                collect_assigned_names(&if_stmt.then_body, names);
                if let Some(ref else_body) = if_stmt.else_body {
                    collect_assigned_names(else_body, names);
                }
            }
            AstStatement::For(for_stmt, _) => {
                collect_assigned_names(&for_stmt.body, names);
            }
            AstStatement::While(while_stmt, _) => {
                collect_assigned_names(&while_stmt.body, names);
            }
            _ => {}
        }
    }
}

/// Recursively collect all declared variable names in a list of statements.
///
/// Currently dead code: companion to `collect_assigned_names`; consumer
/// is the stubbed `compile_comptime_for` (phase-2c).
#[allow(dead_code)]
fn collect_declared_names(stmts: &[AstStatement], names: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            AstStatement::VariableDecl(decl, _) => {
                if let Some(name) = decl.pattern.as_identifier() {
                    names.insert(name.to_string());
                }
            }
            AstStatement::If(if_stmt, _) => {
                collect_declared_names(&if_stmt.then_body, names);
                if let Some(ref else_body) = if_stmt.else_body {
                    collect_declared_names(else_body, names);
                }
            }
            AstStatement::For(for_stmt, _) => {
                collect_declared_names(&for_stmt.body, names);
            }
            AstStatement::While(while_stmt, _) => {
                collect_declared_names(&while_stmt.body, names);
            }
            _ => {}
        }
    }
}

impl BytecodeCompiler {
    /// Compile a unit expression
    pub(super) fn compile_expr_unit(&mut self) -> Result<()> {
        self.emit_unit();
        Ok(())
    }

    /// Compile a spread expression (error - only valid in array/object context)
    pub(super) fn compile_expr_spread(&mut self) -> Result<()> {
        Err(ShapeError::RuntimeError {
            message: "Spread operator can only be used in array or object context".to_string(),
            location: None,
        })
    }

    /// Compile a block expression
    pub(super) fn compile_expr_block(&mut self, block: &shape_ast::ast::BlockExpr) -> Result<()> {
        self.push_scope();
        self.push_drop_scope();

        let mut has_value = false;
        for (i, item) in block.items.iter().enumerate() {
            let is_last = i == block.items.len() - 1;
            let mut future_names =
                self.future_reference_use_names_for_remaining_block_items(&block.items[i + 1..]);
            if self.current_expr_result_mode() == crate::compiler::ExprResultMode::PreserveRef
                && i + 1 < block.items.len()
                && let Some(shape_ast::ast::BlockItem::Expression(expr)) = block.items.last()
            {
                self.collect_reference_use_names_from_expr(expr, true, &mut future_names);
            }
            self.push_future_reference_use_names(future_names);

            let compile_result: Result<()> = match item {
                shape_ast::ast::BlockItem::VariableDecl(var_decl) => {
                    if let Some(init_expr) = &var_decl.value {
                        let saved_pending_variable_name = self.pending_variable_name.clone();
                        self.pending_variable_name = var_decl
                            .pattern
                            .as_identifier()
                            .map(|name| name.to_string());
                        let compile_result = self.compile_expr_for_reference_binding(init_expr);
                        self.pending_variable_name = saved_pending_variable_name;
                        let ref_borrow = compile_result?;
                        // Use full destructure pattern support (array, object, identifier)
                        self.compile_destructure_pattern(&var_decl.pattern)?;
                        for (binding_name, _) in var_decl.pattern.get_bindings() {
                            if let Some(local_idx) = self.resolve_local(&binding_name) {
                                if var_decl.kind == shape_ast::ast::VarKind::Const {
                                    self.const_locals.insert(local_idx);
                                }
                                if var_decl.kind == shape_ast::ast::VarKind::Let && !var_decl.is_mut
                                {
                                    self.immutable_locals.insert(local_idx);
                                }
                            }
                        }
                        self.apply_binding_semantics_to_pattern_bindings(
                            &var_decl.pattern,
                            true,
                            Self::binding_semantics_for_var_decl(var_decl),
                        );
                        self.plan_flexible_binding_storage_for_pattern_initializer(
                            &var_decl.pattern,
                            true,
                            Some(init_expr),
                        );

                        // For simple identifier patterns, track type and drop info
                        if let shape_ast::ast::DestructurePattern::Identifier(name, _) =
                            &var_decl.pattern
                        {
                            if let Some(local_idx) = self.resolve_local(name) {
                                // Track type annotation (so drop tracking can resolve the type)
                                if let Some(ref type_ann) = var_decl.type_annotation {
                                    if let Some(type_name) =
                                        Self::tracked_type_name_from_annotation(type_ann)
                                    {
                                        self.set_local_type_info(local_idx, &type_name);
                                    }
                                    // Handle Table<T> generic annotation
                                    self.try_track_datatable_type(type_ann, local_idx, true)?;
                                } else {
                                    // Propagate initializer type (e.g., var x = 0 → Int64 hint)
                                    // so typed opcodes can be emitted for operations on this variable.
                                    let is_mutable = var_decl.kind == shape_ast::ast::VarKind::Var;
                                    self.propagate_initializer_type_to_slot(
                                        local_idx, true, is_mutable,
                                    );
                                }
                                // Track for auto-drop at scope exit
                                let drop_kind = self.local_drop_kind(local_idx).or_else(|| {
                                    var_decl
                                        .type_annotation
                                        .as_ref()
                                        .and_then(|ann| self.annotation_drop_kind(ann))
                                });
                                let is_async = match drop_kind {
                                    Some(super::super::DropKind::AsyncOnly) => {
                                        self.current_function_is_async
                                    }
                                    Some(super::super::DropKind::Both) => {
                                        self.current_function_is_async
                                    }
                                    Some(super::super::DropKind::SyncOnly) | None => false,
                                };
                                self.track_drop_local(local_idx, is_async);
                                self.finish_reference_binding_from_expr(
                                    local_idx, true, name, init_expr, ref_borrow,
                                );
                                self.update_callable_binding_from_expr(local_idx, true, init_expr);
                            }
                        }
                    }
                    Ok::<(), ShapeError>(())
                }
                shape_ast::ast::BlockItem::Assignment(assignment) => 'block_assign: {
                    if let Some(name) = assignment.pattern.as_identifier() {
                        self.check_named_binding_write_allowed(name, None)?;
                    }
                    // Optimization: x = x.push(val) → ArrayPushLocal (O(1) in-place)
                    if let Some(name) = assignment.pattern.as_identifier() {
                        if let Expr::MethodCall {
                            receiver,
                            method,
                            args,
                            ..
                        } = &assignment.value
                        {
                            if method == "push" && args.len() == 1 {
                                if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                                    if recv_name == name {
                                        if let Some(local_idx) = self.resolve_local(name) {
                                            if !self.ref_locals.contains(&local_idx) {
                                                self.compile_expr(&args[0])?;
                                                self.emit(Instruction::new(
                                                    OpCode::ArrayPushLocal,
                                                    Some(Operand::Local(local_idx)),
                                                ));
                                                self.plan_flexible_binding_storage_from_expr(
                                                    local_idx,
                                                    true,
                                                    &assignment.value,
                                                );
                                                break 'block_assign Ok::<(), ShapeError>(());
                                            }
                                        } else if let Some(&binding_idx) =
                                            self.module_bindings.get(name)
                                        {
                                            self.compile_expr(&args[0])?;
                                            self.emit(Instruction::new(
                                                OpCode::ArrayPushLocal,
                                                Some(Operand::ModuleBinding(binding_idx)),
                                            ));
                                            self.plan_flexible_binding_storage_from_expr(
                                                binding_idx,
                                                false,
                                                &assignment.value,
                                            );
                                            break 'block_assign Ok::<(), ShapeError>(());
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let saved_pending_variable_name = self.pending_variable_name.clone();
                    self.pending_variable_name = assignment
                        .pattern
                        .as_identifier()
                        .map(|name| name.to_string());
                    let compile_result = self.compile_expr_for_reference_binding(&assignment.value);
                    self.pending_variable_name = saved_pending_variable_name;
                    let ref_borrow = compile_result?;
                    // Store in local/module_binding/closure variable
                    self.compile_destructure_assignment(&assignment.pattern)?;
                    if let Some(name) = assignment.pattern.as_identifier() {
                        if let Some(local_idx) = self.resolve_local(name) {
                            if !self.ref_locals.contains(&local_idx) {
                                self.finish_reference_binding_from_expr(
                                    local_idx,
                                    true,
                                    name,
                                    &assignment.value,
                                    ref_borrow,
                                );
                                self.update_callable_binding_from_expr(
                                    local_idx,
                                    true,
                                    &assignment.value,
                                );
                            }
                            self.plan_flexible_binding_storage_from_expr(
                                local_idx,
                                true,
                                &assignment.value,
                            );
                        } else if let Some(scoped_name) =
                            self.resolve_scoped_module_binding_name(name)
                            && let Some(&binding_idx) = self.module_bindings.get(&scoped_name)
                        {
                            self.finish_reference_binding_from_expr(
                                binding_idx,
                                false,
                                name,
                                &assignment.value,
                                ref_borrow,
                            );
                            self.update_callable_binding_from_expr(
                                binding_idx,
                                false,
                                &assignment.value,
                            );
                            self.plan_flexible_binding_storage_from_expr(
                                binding_idx,
                                false,
                                &assignment.value,
                            );
                        }
                    }
                    Ok::<(), ShapeError>(())
                }
                shape_ast::ast::BlockItem::Statement(stmt) => {
                    self.compile_statement(stmt)?;
                    // Statements don't push anything to the stack
                    Ok::<(), ShapeError>(())
                }
                shape_ast::ast::BlockItem::Expression(expr) => {
                    if is_last
                        && self.current_expr_result_mode()
                            == crate::compiler::ExprResultMode::PreserveRef
                    {
                        self.compile_expr_preserving_refs(expr)?;
                    } else {
                        self.compile_expr(expr)?;
                    }
                    if !is_last {
                        // Pop intermediate values
                        self.emit(Instruction::simple(OpCode::Pop));
                    } else {
                        has_value = true;
                    }
                    Ok::<(), ShapeError>(())
                }
            };
            self.pop_future_reference_use_names();
            compile_result?;

            self.release_unused_local_reference_borrows_for_remaining_block_items(
                &block.items[i + 1..],
            );
            self.release_unused_module_reference_borrows_for_remaining_block_items(
                &block.items[i + 1..],
            );
        }

        // If no value expression, the block evaluates to unit
        if !has_value {
            self.emit_unit();
            self.clear_last_expr_reference_result();
        }

        self.pop_drop_scope()?;
        self.pop_scope();
        Ok(())
    }

    /// Compile a range expression
    pub(super) fn compile_expr_range(
        &mut self,
        start: &Option<Box<Expr>>,
        end: &Option<Box<Expr>>,
        kind: &shape_ast::ast::RangeKind,
    ) -> Result<()> {
        // Push start value or null if not present
        if let Some(s) = start {
            self.compile_expr(s)?;
        } else {
            self.emit(Instruction::simple(OpCode::PushNull));
        }
        // Push end value or null if not present
        if let Some(e) = end {
            self.compile_expr(e)?;
        } else {
            self.emit(Instruction::simple(OpCode::PushNull));
        }
        // Push inclusive flag
        let inclusive = *kind == shape_ast::ast::RangeKind::Inclusive;
        let const_idx = self.program.add_constant(Constant::Bool(inclusive));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
        self.emit(Instruction::simple(OpCode::MakeRange));
        Ok(())
    }

    /// Compile a window expression to bytecode.
    ///
    /// Window functions operate on a DataTable that must already be on the stack
    /// (or in scope as a variable). The compilation strategy:
    /// 1. Compile the inner expression (column accessor) if any
    /// 2. Encode the window spec (partition_by, order_by, frame) as a constant string
    /// 3. Push arg count
    /// 4. Emit BuiltinCall with the appropriate Window* builtin
    pub(super) fn compile_expr_window(
        &mut self,
        window_expr: &shape_ast::ast::windows::WindowExpr,
    ) -> Result<()> {
        use crate::bytecode::{BuiltinFunction, Constant};
        use shape_ast::ast::windows::WindowFunction;

        // Encode the window spec as a JSON-like string constant for the executor.
        // The executor will parse this to determine partitioning, ordering, and framing.
        let spec_str = self.encode_window_spec(&window_expr.over);
        let spec_const = self.program.add_constant(Constant::String(spec_str));

        // Determine the builtin variant and compile any inner expression arguments.
        let (builtin, arg_count) = match &window_expr.function {
            WindowFunction::RowNumber => {
                // No inner expr — push spec only
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowRowNumber, 1)
            }
            WindowFunction::Rank => {
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowRank, 1)
            }
            WindowFunction::DenseRank => {
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowDenseRank, 1)
            }
            WindowFunction::Ntile(n) => {
                let n_const = self.program.add_constant(Constant::Number(*n as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(n_const)),
                ));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowNtile, 2)
            }
            WindowFunction::Lag {
                expr,
                offset,
                default,
            } => {
                self.compile_expr(expr)?;
                let offset_const = self.program.add_constant(Constant::Number(*offset as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(offset_const)),
                ));
                if let Some(def) = default {
                    self.compile_expr(def)?;
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                }
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowLag, 4)
            }
            WindowFunction::Lead {
                expr,
                offset,
                default,
            } => {
                self.compile_expr(expr)?;
                let offset_const = self.program.add_constant(Constant::Number(*offset as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(offset_const)),
                ));
                if let Some(def) = default {
                    self.compile_expr(def)?;
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                }
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowLead, 4)
            }
            WindowFunction::FirstValue(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowFirstValue, 2)
            }
            WindowFunction::LastValue(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowLastValue, 2)
            }
            WindowFunction::NthValue(expr, n) => {
                self.compile_expr(expr)?;
                let n_const = self.program.add_constant(Constant::Number(*n as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(n_const)),
                ));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowNthValue, 3)
            }
            WindowFunction::Sum(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowSum, 2)
            }
            WindowFunction::Avg(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowAvg, 2)
            }
            WindowFunction::Min(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowMin, 2)
            }
            WindowFunction::Max(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowMax, 2)
            }
            WindowFunction::Count(expr_opt) => {
                if let Some(expr) = expr_opt {
                    self.compile_expr(expr)?;
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                }
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(spec_const)),
                ));
                (BuiltinFunction::WindowCount, 2)
            }
        };

        // Push arg count and emit the builtin call
        let count_const = self
            .program
            .add_constant(Constant::Number(arg_count as f64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(count_const)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(builtin)),
        ));
        Ok(())
    }

    /// Encode a WindowSpec as a compact string for the executor to parse.
    fn encode_window_spec(&mut self, spec: &shape_ast::ast::windows::WindowSpec) -> String {
        use shape_ast::ast::windows::{SortDirection, WindowBound, WindowFrameType};
        let mut parts = Vec::new();

        if !spec.partition_by.is_empty() {
            let partition_strs: Vec<String> = spec
                .partition_by
                .iter()
                .map(|e| format!("{:?}", e))
                .collect();
            parts.push(format!("partition:{}", partition_strs.join(",")));
        }

        if let Some(ref order_by) = spec.order_by {
            let order_strs: Vec<String> = order_by
                .columns
                .iter()
                .map(|(e, dir)| {
                    let dir_str = match dir {
                        SortDirection::Ascending => "asc",
                        SortDirection::Descending => "desc",
                    };
                    format!("{:?}:{}", e, dir_str)
                })
                .collect();
            parts.push(format!("order:{}", order_strs.join(",")));
        }

        if let Some(ref frame) = spec.frame {
            let frame_type = match frame.frame_type {
                WindowFrameType::Rows => "rows",
                WindowFrameType::Range => "range",
            };
            let start = match &frame.start {
                WindowBound::UnboundedPreceding => "unbounded_preceding".to_string(),
                WindowBound::CurrentRow => "current_row".to_string(),
                WindowBound::Preceding(n) => format!("preceding:{}", n),
                WindowBound::Following(n) => format!("following:{}", n),
                WindowBound::UnboundedFollowing => "unbounded_following".to_string(),
            };
            let end = match &frame.end {
                WindowBound::UnboundedPreceding => "unbounded_preceding".to_string(),
                WindowBound::CurrentRow => "current_row".to_string(),
                WindowBound::Preceding(n) => format!("preceding:{}", n),
                WindowBound::Following(n) => format!("following:{}", n),
                WindowBound::UnboundedFollowing => "unbounded_following".to_string(),
            };
            parts.push(format!("frame:{}:{}:{}", frame_type, start, end));
        }

        if parts.is_empty() {
            "default".to_string()
        } else {
            parts.join(";")
        }
    }

    /// Compile a while expression
    pub(super) fn compile_expr_while(
        &mut self,
        while_expr: &shape_ast::ast::WhileExpr,
    ) -> Result<()> {
        self.compile_while_expr(while_expr)
    }

    /// Compile a for expression
    pub(super) fn compile_expr_for(&mut self, for_expr: &shape_ast::ast::ForExpr) -> Result<()> {
        self.compile_for_expr(for_expr)
    }

    /// Compile a loop expression
    pub(super) fn compile_expr_loop(&mut self, loop_expr: &shape_ast::ast::LoopExpr) -> Result<()> {
        self.compile_loop_expr(loop_expr)
    }

    /// Compile a join expression: `join all|race|any|settle { branch1, branch2, ... }`
    ///
    /// Compilation strategy:
    /// 1. For each branch, wrap the expression in a closure and emit SpawnTask
    ///    to get a Future(task_id) on the stack.
    /// 2. Emit JoinInit(packed_u16) where high 2 bits = kind, low 14 bits = arity.
    ///    This pops all futures and pushes a TaskGroup.
    /// 3. Emit JoinAwait which pops the TaskGroup and suspends until
    ///    the host resolves according to the join strategy.
    ///
    /// Note: This is always wrapped by Expr::Await in the parser,
    /// so the caller already checks current_function_is_async.
    pub(super) fn compile_join_expr(&mut self, join_expr: &JoinExpr) -> Result<()> {
        if self.current_function.is_some() && !self.current_function_is_async {
            return Err(ShapeError::SemanticError {
                message: "'await join' can only be used inside an async function".to_string(),
                location: None,
            });
        }

        let arity = join_expr.branches.len();
        if arity == 0 {
            return Err(ShapeError::SemanticError {
                message: "Join expression requires at least one branch".to_string(),
                location: None,
            });
        }
        if arity > 0x3FFF {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Join expression has too many branches (max 16383, got {})",
                    arity
                ),
                location: None,
            });
        }

        // Compile each branch: wrap in closure, spawn task
        for branch in &join_expr.branches {
            // Compile the branch expression as-is.
            // In a full implementation, each branch would be wrapped in a closure
            // and SpawnTask would schedule it. For now, we compile the expression
            // and emit SpawnTask which creates a Future from the top-of-stack value.
            self.plan_flexible_binding_escape_from_expr(&branch.expr);
            self.compile_expr(&branch.expr)?;
            self.emit(Instruction::simple(OpCode::SpawnTask));
        }

        // Pack kind and arity into a u16: [kind:2][arity:14]
        let kind_bits = match join_expr.kind {
            JoinKind::All => 0u16,
            JoinKind::Race => 1u16,
            JoinKind::Any => 2u16,
            JoinKind::Settle => 3u16,
        };
        let packed = (kind_bits << 14) | (arity as u16);

        // JoinInit: pop N futures, push TaskGroup
        self.emit(Instruction::new(
            OpCode::JoinInit,
            Some(Operand::Count(packed)),
        ));

        // JoinAwait: pop TaskGroup, suspend, host pushes result on resume
        self.emit(Instruction::simple(OpCode::JoinAwait));

        Ok(())
    }

    /// Compile a comptime for expression by evaluating the iterable at compile time
    /// and unrolling the body once per element.
    pub(super) fn compile_comptime_for(
        &mut self,
        cf: &ComptimeForExpr,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        // Inside comptime mini-programs (e.g. comptime annotation handlers), we
        // execute comptime-for directly as a runtime for-expression so lexical
        // bindings like `target` are visible.
        if self.comptime_mode {
            let mut items = Vec::with_capacity(cf.body.len());
            for (idx, stmt) in cf.body.iter().enumerate() {
                let is_last = idx + 1 == cf.body.len();
                if is_last {
                    if let shape_ast::ast::Statement::Expression(expr, _) = stmt {
                        items.push(shape_ast::ast::BlockItem::Expression(expr.clone()));
                        continue;
                    }
                }
                items.push(shape_ast::ast::BlockItem::Statement(stmt.clone()));
            }

            let for_expr = shape_ast::ast::ForExpr {
                pattern: shape_ast::ast::Pattern::Identifier(cf.variable.clone()),
                iterable: cf.iterable.clone(),
                body: Box::new(Expr::Block(
                    shape_ast::ast::BlockExpr { items },
                    shape_ast::ast::Span::DUMMY,
                )),
                is_async: false,
            };
            return self.compile_expr_for(&for_expr);
        }

        // The comptime evaluator's `ComptimeExecutionResult.value` is
        // shaped as the deleted `ValueWord` carrier (see
        // `compiler/comptime.rs`); `nb_to_literal` and `as_any_array` /
        // `type_name` projections used by the unroll path below also
        // ride that carrier. The kinded carrier shape lands in phase-2c
        // (ADR-006 §2.4); until then this site surfaces rather than
        // routes through the deleted accessors.
        let _ = (cf, span);
        todo!("phase-2c — see ADR-006 §2.4");
    }
}

// Wave-β C-expressions: the `comptime_for_tests` and `block_expr_tests`
// modules previously declared here asserted against the deleted
// carrier and pulled `eval` whose return type rides the same shape.
// The assertions are deleted to keep this territory free of the
// deleted carrier; restoring the coverage lands together with the
// phase-2c carrier shape (ADR-006 §2.4) and the test harness sweep on
// `crate::test_utils::eval`.
