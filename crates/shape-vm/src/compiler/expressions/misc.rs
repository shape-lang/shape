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

            match item {
                shape_ast::ast::BlockItem::VariableDecl(var_decl) => {
                    if let Some(init_expr) = &var_decl.value {
                        let ref_borrow = self.compile_expr_for_reference_binding(init_expr)?;
                        // Use full destructure pattern support (array, object, identifier)
                        self.compile_destructure_pattern(&var_decl.pattern)?;

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
                            }
                        }
                    }
                }
                shape_ast::ast::BlockItem::Assignment(assignment) => 'block_assign: {
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
                                                break 'block_assign;
                                            }
                                        } else if let Some(&binding_idx) =
                                            self.module_bindings.get(name)
                                        {
                                            self.compile_expr(&args[0])?;
                                            self.emit(Instruction::new(
                                                OpCode::ArrayPushLocal,
                                                Some(Operand::ModuleBinding(binding_idx)),
                                            ));
                                            break 'block_assign;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let ref_borrow = self.compile_expr_for_reference_binding(&assignment.value)?;
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
                            }
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
                        }
                    }
                }
                shape_ast::ast::BlockItem::Statement(stmt) => {
                    self.compile_statement(stmt)?;
                    // Statements don't push anything to the stack
                }
                shape_ast::ast::BlockItem::Expression(expr) => {
                    self.compile_expr(expr)?;
                    if !is_last {
                        // Pop intermediate values
                        self.emit(Instruction::simple(OpCode::Pop));
                    } else {
                        has_value = true;
                    }
                }
            }

            self.release_unused_local_reference_borrows_for_remaining_block_items(
                &block.items[i + 1..],
            );
        }

        // If no value, push null
        if !has_value {
            self.emit(Instruction::simple(OpCode::PushNull));
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

        use shape_ast::ast::{Span, Statement};

        // Step 1: Evaluate the iterable expression at compile time.
        // Wrap it in a return statement so execute_comptime returns the value.
        let eval_stmts = vec![Statement::Return(Some((*cf.iterable).clone()), Span::DUMMY)];
        let extensions: Vec<_> = self
            .extension_registry
            .as_ref()
            .map(|r| r.as_ref().clone())
            .unwrap_or_default();
        let trait_impls = self.type_inference.env.trait_impl_keys();
        let known_type_symbols: std::collections::HashSet<String> = self
            .struct_types
            .keys()
            .chain(self.type_aliases.keys())
            .cloned()
            .collect();
        let comptime_helpers = self.collect_comptime_helpers();
        let loc = self.span_to_source_location(span);
        let execution = super::super::comptime::execute_comptime(
            &eval_stmts,
            &comptime_helpers,
            &extensions,
            trait_impls,
            known_type_symbols,
        )
        .map_err(|e| ShapeError::SemanticError {
            message: format!(
                "comptime for: iterable expression is not evaluable at compile time: {}",
                super::super::helpers::strip_error_prefix(&e)
            ),
            location: Some(loc.clone()),
        })?;
        self.process_comptime_directives(execution.directives, "")
            .map_err(|e| ShapeError::SemanticError {
                message: format!("comptime for: directive processing failed: {}", e),
                location: Some(loc.clone()),
            })?;

        // Step 2: The result must be an Array. Use ValueWord dispatch to extract elements.
        let iterable_nb = execution.value;
        let elements = match iterable_nb.as_any_array() {
            Some(view) => view.to_generic(),
            None => {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "comptime for: iterable must evaluate to an array, got {}",
                        iterable_nb.type_name()
                    ),
                    location: Some(loc),
                });
            }
        };

        // Step 3: If empty, push Unit.
        if elements.is_empty() {
            self.emit(Instruction::new(OpCode::PushNull, None));
            return Ok(());
        }

        // Step 4: Unroll — for each element, bind the loop variable and compile the body.
        // Snapshot immutable_locals and immutable_module_bindings, then selectively
        // unfreeze outer-scope variables that the comptime for body assigns to (but does
        // not redeclare), so that accumulation patterns like
        // `let total = 0.0; comptime for x in [...] { total = total + x }` work.
        let saved_immutable_locals = self.immutable_locals.clone();
        let saved_immutable_module = self.immutable_module_bindings.clone();
        {
            let mut assigned_names = HashSet::new();
            collect_assigned_names(&cf.body, &mut assigned_names);

            let mut body_declared = HashSet::new();
            collect_declared_names(&cf.body, &mut body_declared);

            for name in &assigned_names {
                if name == &cf.variable {
                    continue;
                }
                if body_declared.contains(name) {
                    continue;
                }
                // Unfreeze local if it exists
                if let Some(local_idx) = self.resolve_local(name) {
                    self.immutable_locals.remove(&local_idx);
                }
                // Unfreeze module binding if it exists
                let scoped_name = self
                    .resolve_scoped_module_binding_name(name)
                    .unwrap_or_else(|| name.to_string());
                if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                    self.immutable_module_bindings.remove(&binding_idx);
                }
            }
        }

        for (i, element) in elements.iter().enumerate() {
            // Pop previous iteration's value (except for the first).
            if i > 0 {
                self.emit(Instruction::simple(OpCode::Pop));
            }

            self.push_scope();

            // Bind the loop variable to the element value as a local.
            let lit = super::super::comptime::nb_to_literal(element);
            self.compile_literal(&lit)?;
            let local_idx = self.declare_local(&cf.variable)?;
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(local_idx)),
            ));

            // Compile body statements.
            // The last statement's value stays on the stack as the iteration result.
            let body_len = cf.body.len();
            if body_len == 0 {
                self.emit(Instruction::new(OpCode::PushNull, None));
            } else {
                for (j, stmt) in cf.body.iter().enumerate() {
                    let is_last_stmt = j + 1 == body_len;
                    match stmt {
                        Statement::Expression(expr, _) => {
                            self.compile_expr(expr)?;
                            if !is_last_stmt {
                                self.emit(Instruction::simple(OpCode::Pop));
                            }
                        }
                        _ => {
                            self.compile_statement(stmt)?;
                            if is_last_stmt {
                                // Non-expression final statements don't produce a loop value.
                                self.emit(Instruction::new(OpCode::PushNull, None));
                            }
                        }
                    }
                    // For non-final expression statements, pop the value to avoid stack buildup.
                    // The final statement's value is kept as the iteration result.
                }
            }

            self.pop_scope();
        }

        // Restore immutability tracking to pre-unroll state.
        self.immutable_locals = saved_immutable_locals;
        self.immutable_module_bindings = saved_immutable_module;

        Ok(())
    }
}

#[cfg(test)]
mod comptime_for_tests {
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::ValueWord;

    fn eval(code: &str) -> ValueWord {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile failed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.execute(None).expect("execution failed").clone()
    }

    #[test]
    fn test_comptime_for_literal_array() {
        // Unroll over a literal array: each iteration yields the element.
        let result = eval(
            r#"
            let total = 0.0
            comptime for x in [1.0, 2.0, 3.0] {
                total = total + x
            }
            total
        "#,
        );
        assert_eq!(result, ValueWord::from_f64(6.0));
    }

    #[test]
    fn test_comptime_for_empty_array() {
        // Empty array: result is Unit (PushNull)
        let result = eval(
            r#"
            comptime for x in [] {
                x
            }
        "#,
        );
        assert_eq!(result, ValueWord::none());
    }

    #[test]
    fn test_comptime_for_string_array() {
        // Unroll over string array
        let result = eval(
            r#"
            let result = ""
            comptime for name in ["hello", "world"] {
                result = result + name + " "
            }
            result
        "#,
        );
        {
            let s = result.as_arc_string().expect("Expected String");
            assert_eq!(s.as_ref(), "hello world ");
        }
    }

    #[test]
    fn test_comptime_for_non_array_iterable_errors() {
        let code = r#"comptime for x in 42 { x }"#;
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        let result = compiler.compile(&program);
        assert!(result.is_err(), "comptime for with non-array should fail");
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("array"), "Error should mention array: {}", err);
    }
}
