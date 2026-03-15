//! Loop compilation (for, while, loop expressions)

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::type_tracking::NumericType;
use shape_ast::ast::{Expr, ForInit, RangeKind};
use shape_ast::error::{Result, ShapeError};

use super::{BytecodeCompiler, LoopContext};

/// State for a range counter loop specialization.
pub(super) struct RangeCounterLoopState {
    /// Local slot holding the loop counter (also the user's binding).
    pub counter_local: u16,
    /// Bytecode offset of the LoopStart instruction.
    pub loop_start: usize,
    /// Bytecode index of the exit JumpIfFalse (to be patched).
    pub exit_jump: usize,
    /// Whether both endpoints were proven int (typed opcodes) or not (generic).
    pub use_typed: bool,
}

impl BytecodeCompiler {
    // ===== Range counter loop specialization =====

    /// Try to begin a range counter loop specialization.
    ///
    /// If the iterator is a `Range { start, end }` with both endpoints present,
    /// emits a counter-based loop prologue and returns the state. The caller
    /// emits the body, then calls `end_range_counter_loop`.
    ///
    /// `var_name` is the simple identifier name for the loop variable.
    /// Pass `None` to signal that the pattern is not a simple identifier
    /// (returns `Ok(None)` immediately).
    ///
    /// Returns `Ok(None)` (no side effects) when specialization is not applicable.
    pub(super) fn try_begin_range_counter_loop(
        &mut self,
        var_name: Option<&str>,
        iter: &Expr,
    ) -> Result<Option<RangeCounterLoopState>> {
        // Only specialize simple identifier patterns
        let var_name = match var_name {
            Some(name) => name,
            None => return Ok(None),
        };

        // Only specialize Range with both endpoints present
        let (start_expr, end_expr, inclusive) = match iter {
            Expr::Range {
                start: Some(s),
                end: Some(e),
                kind,
                ..
            } => (s.as_ref(), e.as_ref(), *kind == RangeKind::Inclusive),
            _ => return Ok(None),
        };

        // === Point of no return: emit specialized bytecode ===

        // Declare loop variable (user binding = counter)
        let counter_local = self.declare_local(var_name)?;
        let end_local = self.declare_local("__range_end")?;

        // compile(start) → [NumberToInt if float] → StoreLocal(counter)
        self.compile_expr(start_expr)?;
        let start_nt = self.last_expr_numeric_type;
        if matches!(start_nt, Some(NumericType::Number)) {
            self.emit(Instruction::simple(OpCode::NumberToInt));
        }
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(counter_local)),
        ));

        // compile(end) → [NumberToInt if float] → StoreLocal(__end)
        self.compile_expr(end_expr)?;
        let end_nt = self.last_expr_numeric_type;
        if matches!(end_nt, Some(NumericType::Number)) {
            self.emit(Instruction::simple(OpCode::NumberToInt));
        }
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(end_local)),
        ));

        // Use typed opcodes when both endpoints are proven numeric
        // (Int directly, or Number after NumberToInt conversion → both are int)
        let use_typed = matches!(start_nt, Some(NumericType::Int) | Some(NumericType::Number))
            && matches!(end_nt, Some(NumericType::Int) | Some(NumericType::Number));

        // LoopStart
        let loop_start = self.program.current_offset();
        self.emit(Instruction::simple(OpCode::LoopStart));

        // LoadLocal(counter), LoadLocal(__end), LtInt/LteInt
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(counter_local)),
        ));
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(end_local)),
        ));
        if use_typed {
            self.emit(Instruction::simple(if inclusive {
                OpCode::LteInt
            } else {
                OpCode::LtInt
            }));
        } else {
            self.emit(Instruction::simple(if inclusive {
                OpCode::Lte
            } else {
                OpCode::Lt
            }));
        }

        // JumpIfFalse(exit)
        let exit_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

        Ok(Some(RangeCounterLoopState {
            counter_local,
            loop_start,
            exit_jump,
            use_typed,
        }))
    }

    /// End a range counter loop: patch continue jumps, emit increment,
    /// back-jump, LoopEnd, and patch exit jump.
    pub(super) fn end_range_counter_loop(&mut self, state: &RangeCounterLoopState) {
        // Patch deferred continue jumps to the increment block
        if let Some(loop_ctx) = self.loop_stack.last() {
            let continue_jumps: Vec<usize> = loop_ctx.continue_jumps.clone();
            for cj in continue_jumps {
                self.patch_jump(cj);
            }
        }

        // Increment: LoadLocal(counter), PushConst(1), AddInt, StoreLocal(counter)
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(state.counter_local)),
        ));
        if state.use_typed {
            let one_const = self.program.add_constant(Constant::Int(1));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(one_const)),
            ));
            self.emit(Instruction::simple(OpCode::AddInt));
        } else {
            let one_const = self.program.add_constant(Constant::Int(1));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(one_const)),
            ));
            self.emit(Instruction::simple(OpCode::Add));
        }
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(state.counter_local)),
        ));

        // Jump back to LoopStart
        let offset = state.loop_start as i32 - self.program.current_offset() as i32 - 1;
        self.emit(Instruction::new(
            OpCode::Jump,
            Some(Operand::Offset(offset)),
        ));

        // LoopEnd
        self.emit(Instruction::simple(OpCode::LoopEnd));

        // Patch exit jump (past LoopEnd)
        self.patch_jump(state.exit_jump);
    }
    pub(super) fn compile_while_loop(
        &mut self,
        while_loop: &shape_ast::ast::WhileLoop,
    ) -> Result<()> {
        // Emit LoopStart marker for JIT loop optimizations (LICM, GC safepoint, int unboxing)
        let loop_start = self.program.current_offset();
        self.emit(Instruction::simple(OpCode::LoopStart));

        // Create loop context — continue targets LoopStart so condition re-evaluates
        let loop_ctx = LoopContext {
            break_jumps: Vec::new(),
            continue_target: loop_start,
            break_value_local: None,
            iterator_on_stack: false,
            drop_scope_depth: self.drop_locals.len(),
            continue_jumps: Vec::new(),
        };

        // Compile condition
        self.compile_expr(&while_loop.condition)?;

        // Jump out if false
        let exit_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

        // Push loop context
        self.loop_stack.push(loop_ctx);

        // Compile body
        self.push_repeating_reference_release_barrier();
        let body_result = (|| -> Result<()> {
            for (idx, stmt) in while_loop.body.iter().enumerate() {
                let future_names = self.future_reference_use_names_for_remaining_statements(
                    &while_loop.body[idx + 1..],
                );
                self.push_future_reference_use_names(future_names);
                let compile_result = self.compile_statement(stmt);
                self.pop_future_reference_use_names();
                compile_result?;
                self.release_unused_local_reference_borrows_for_remaining_statements(
                    &while_loop.body[idx + 1..],
                );
                self.release_unused_module_reference_borrows_for_remaining_statements(
                    &while_loop.body[idx + 1..],
                );
            }
            Ok(())
        })();
        self.pop_repeating_reference_release_barrier();
        body_result?;

        // Jump back to LoopStart
        let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
        self.emit(Instruction::new(
            OpCode::Jump,
            Some(Operand::Offset(offset)),
        ));

        // Emit LoopEnd marker
        self.emit(Instruction::simple(OpCode::LoopEnd));

        // Patch exit jump (past LoopEnd)
        self.patch_jump(exit_jump);

        // Pop loop context and patch break jumps
        if let Some(loop_ctx) = self.loop_stack.pop() {
            for break_jump in loop_ctx.break_jumps {
                self.patch_jump(break_jump);
            }
        }

        Ok(())
    }

    pub(super) fn compile_while_expr(
        &mut self,
        while_expr: &shape_ast::ast::WhileExpr,
    ) -> Result<()> {
        self.push_scope();

        let result_local = self.declare_local("__while_result")?;
        self.emit(Instruction::simple(OpCode::PushNull));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        let loop_start = self.program.current_offset();
        self.emit(Instruction::simple(OpCode::LoopStart));

        self.compile_expr(&while_expr.condition)?;
        let exit_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

        self.loop_stack.push(LoopContext {
            break_jumps: Vec::new(),
            continue_target: loop_start,
            break_value_local: Some(result_local),
            iterator_on_stack: false,
            drop_scope_depth: self.drop_locals.len(),
            continue_jumps: Vec::new(),
        });

        self.push_repeating_reference_release_barrier();
        let body_result = self.compile_expr(&while_expr.body);
        self.pop_repeating_reference_release_barrier();
        body_result?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
        self.emit(Instruction::new(
            OpCode::Jump,
            Some(Operand::Offset(offset)),
        ));

        self.emit(Instruction::simple(OpCode::LoopEnd));
        self.patch_jump(exit_jump);

        if let Some(loop_ctx) = self.loop_stack.pop() {
            for break_jump in loop_ctx.break_jumps {
                self.patch_jump(break_jump);
            }
        }

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));

        self.pop_scope();
        Ok(())
    }

    /// Compile for loop
    pub(super) fn compile_for_loop(&mut self, for_loop: &shape_ast::ast::ForLoop) -> Result<()> {
        // Validate: `for await` requires async context
        if for_loop.is_async && !self.current_function_is_async {
            return Err(ShapeError::SemanticError {
                message: "'for await' can only be used inside an async function".to_string(),
                location: None,
            });
        }

        match &for_loop.init {
            ForInit::ForIn { pattern, iter } => {
                self.push_scope();

                // Try range counter loop specialization (non-async only)
                if !for_loop.is_async {
                    if let Some(rcl) = self.try_begin_range_counter_loop(
                        pattern.as_identifier(),
                        iter,
                    )? {
                        self.apply_binding_semantics_to_pattern_bindings(
                            pattern,
                            true,
                            Self::owned_mutable_binding_semantics(),
                        );

                        self.loop_stack.push(LoopContext {
                            break_jumps: Vec::new(),
                            continue_target: usize::MAX, // deferred
                            break_value_local: None,
                            iterator_on_stack: false,
                            drop_scope_depth: self.drop_locals.len(),
                            continue_jumps: Vec::new(),
                        });

                        // Compile body
                        self.push_repeating_reference_release_barrier();
                        let body_result = (|| -> Result<()> {
                            for (idx, stmt) in for_loop.body.iter().enumerate() {
                                let future_names = self
                                    .future_reference_use_names_for_remaining_statements(
                                        &for_loop.body[idx + 1..],
                                    );
                                self.push_future_reference_use_names(future_names);
                                let compile_result = self.compile_statement(stmt);
                                self.pop_future_reference_use_names();
                                compile_result?;
                                self.release_unused_local_reference_borrows_for_remaining_statements(
                                    &for_loop.body[idx + 1..],
                                );
                                self.release_unused_module_reference_borrows_for_remaining_statements(
                                    &for_loop.body[idx + 1..],
                                );
                            }
                            Ok(())
                        })();
                        self.pop_repeating_reference_release_barrier();
                        body_result?;

                        self.end_range_counter_loop(&rcl);

                        if let Some(loop_ctx) = self.loop_stack.pop() {
                            for break_jump in loop_ctx.break_jumps {
                                self.patch_jump(break_jump);
                            }
                        }

                        self.pop_scope();
                        return Ok(());
                    }
                }

                // === Generic iterator path (unchanged) ===

                // Compile iterator expression and leave it on stack
                self.compile_expr(iter)?;

                // Reserve local for index counter
                let idx_local = self.declare_local("__idx")?;

                // Initialize index to 0
                let zero_const = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(zero_const)),
                ));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(idx_local)),
                ));

                // Pre-declare locals for destructuring pattern
                // This ensures the locals are in scope for the entire loop
                for name in pattern.get_identifiers() {
                    self.declare_local(&name)?;
                }
                self.apply_binding_semantics_to_pattern_bindings(
                    pattern,
                    true,
                    Self::owned_mutable_binding_semantics(),
                );

                let loop_start = self.program.current_offset();
                self.emit(Instruction::simple(OpCode::LoopStart));
                let loop_ctx = LoopContext {
                    break_jumps: Vec::new(),
                    continue_target: loop_start,
                    break_value_local: None,
                    iterator_on_stack: true,
                    drop_scope_depth: self.drop_locals.len(),
                    continue_jumps: Vec::new(),
                };

                // Check if iterator is done (dup iterator and index, then IterDone)
                self.emit(Instruction::simple(OpCode::Dup)); // Dup iterator
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(idx_local)),
                ));
                self.emit(Instruction::simple(OpCode::IterDone));
                let exit_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

                // Get next element (dup iterator and index, then IterNext)
                self.emit(Instruction::simple(OpCode::Dup)); // Dup iterator
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(idx_local)),
                ));
                self.emit(Instruction::simple(OpCode::IterNext));

                // For `for await`, each element is a Future — await it before binding
                if for_loop.is_async {
                    self.emit(Instruction::simple(OpCode::Await));
                }

                // Destructure value into loop variable(s)
                self.compile_destructure_pattern(pattern)?;

                // Increment index before body so continue jumps advance correctly
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(idx_local)),
                ));
                let one_const = self.program.add_constant(Constant::Number(1.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(one_const)),
                ));
                self.emit(Instruction::simple(OpCode::Add));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(idx_local)),
                ));

                // Push loop context
                self.loop_stack.push(loop_ctx);

                // Compile body
                self.push_repeating_reference_release_barrier();
                let body_result = (|| -> Result<()> {
                    for (idx, stmt) in for_loop.body.iter().enumerate() {
                        let future_names = self
                            .future_reference_use_names_for_remaining_statements(
                                &for_loop.body[idx + 1..],
                            );
                        self.push_future_reference_use_names(future_names);
                        let compile_result = self.compile_statement(stmt);
                        self.pop_future_reference_use_names();
                        compile_result?;
                        self.release_unused_local_reference_borrows_for_remaining_statements(
                            &for_loop.body[idx + 1..],
                        );
                        self.release_unused_module_reference_borrows_for_remaining_statements(
                            &for_loop.body[idx + 1..],
                        );
                    }
                    Ok(())
                })();
                self.pop_repeating_reference_release_barrier();
                body_result?;

                // Jump back to LoopStart
                let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
                self.emit(Instruction::new(
                    OpCode::Jump,
                    Some(Operand::Offset(offset)),
                ));

                self.emit(Instruction::simple(OpCode::LoopEnd));

                // Patch exit jump (past LoopEnd)
                self.patch_jump(exit_jump);

                // Pop iterator from stack
                self.emit(Instruction::simple(OpCode::Pop));

                // Pop loop context and patch break jumps
                if let Some(loop_ctx) = self.loop_stack.pop() {
                    for break_jump in loop_ctx.break_jumps {
                        self.patch_jump(break_jump);
                    }
                }

                self.pop_scope();
            }
            ForInit::ForC {
                init,
                condition,
                update,
            } => {
                // Compile C-style for loop
                self.push_scope();

                // Initialize
                self.compile_statement(init)?;

                let loop_start = self.program.current_offset();
                self.emit(Instruction::simple(OpCode::LoopStart));

                // Create loop context
                let update_start = self.program.current_offset();
                let mut loop_ctx = LoopContext {
                    break_jumps: Vec::new(),
                    continue_target: update_start,
                    break_value_local: None,
                    iterator_on_stack: false,
                    drop_scope_depth: self.drop_locals.len(),
                    continue_jumps: Vec::new(),
                };

                // Check condition
                self.compile_expr(condition)?;
                let exit_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

                // Push loop context
                self.loop_stack.push(loop_ctx);

                // Compile body
                self.push_repeating_reference_release_barrier();
                let body_result = (|| -> Result<()> {
                    for (idx, stmt) in for_loop.body.iter().enumerate() {
                        let future_names = self
                            .future_reference_use_names_for_remaining_statements(
                                &for_loop.body[idx + 1..],
                            );
                        self.push_future_reference_use_names(future_names);
                        let compile_result = self.compile_statement(stmt);
                        self.pop_future_reference_use_names();
                        compile_result?;
                        self.release_unused_local_reference_borrows_for_remaining_statements(
                            &for_loop.body[idx + 1..],
                        );
                        self.release_unused_module_reference_borrows_for_remaining_statements(
                            &for_loop.body[idx + 1..],
                        );
                    }
                    Ok(())
                })();
                self.pop_repeating_reference_release_barrier();
                body_result?;

                // Update
                loop_ctx = self
                    .loop_stack
                    .pop()
                    .expect("loop context was pushed above");
                loop_ctx.continue_target = self.program.current_offset();
                self.loop_stack.push(loop_ctx);

                self.compile_expr(update)?;
                self.emit(Instruction::simple(OpCode::Pop));

                // Jump back to LoopStart
                let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
                self.emit(Instruction::new(
                    OpCode::Jump,
                    Some(Operand::Offset(offset)),
                ));

                self.emit(Instruction::simple(OpCode::LoopEnd));

                // Patch exit jump (past LoopEnd)
                self.patch_jump(exit_jump);

                // Pop loop context and patch break jumps
                if let Some(loop_ctx) = self.loop_stack.pop() {
                    for break_jump in loop_ctx.break_jumps {
                        self.patch_jump(break_jump);
                    }
                }

                self.pop_scope();
            }
        }

        Ok(())
    }

    pub(super) fn compile_for_expr(&mut self, for_expr: &shape_ast::ast::ForExpr) -> Result<()> {
        // Validate: `for await` requires async context
        if for_expr.is_async && !self.current_function_is_async {
            return Err(ShapeError::SemanticError {
                message: "'for await' can only be used inside an async function".to_string(),
                location: None,
            });
        }

        self.push_scope();

        // Try range counter specialization (non-async, simple identifier pattern)
        if !for_expr.is_async {
            let pattern_name = match &for_expr.pattern {
                    shape_ast::ast::Pattern::Identifier(name) => Some(name.as_str()),
                    _ => None,
                };
            if let Some(rcl) =
                self.try_begin_range_counter_loop(pattern_name, &for_expr.iterable)?
            {
                let result_local = self.declare_local("__for_result")?;
                self.emit(Instruction::simple(OpCode::PushNull));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));

                self.apply_binding_semantics_to_value_pattern_bindings(
                    &for_expr.pattern,
                    Self::owned_mutable_binding_semantics(),
                );

                self.loop_stack.push(LoopContext {
                    break_jumps: Vec::new(),
                    continue_target: usize::MAX,
                    break_value_local: Some(result_local),
                    iterator_on_stack: false,
                    drop_scope_depth: self.drop_locals.len(),
                    continue_jumps: Vec::new(),
                });

                self.push_repeating_reference_release_barrier();
                let body_result = self.compile_expr(&for_expr.body);
                self.pop_repeating_reference_release_barrier();
                body_result?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));

                self.end_range_counter_loop(&rcl);

                if let Some(loop_ctx) = self.loop_stack.pop() {
                    for break_jump in loop_ctx.break_jumps {
                        self.patch_jump(break_jump);
                    }
                }

                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));

                self.pop_scope();
                return Ok(());
            }
        }

        // === Generic iterator path (unchanged) ===

        // Determine binding pattern: simple identifier, object destructure, or array destructure.
        let elem_local;
        let mut destructure_fields: Vec<(String, u16)> = Vec::new();
        let mut array_destructure_locals: Vec<u16> = Vec::new();
        let is_object_destructure;
        let mut is_array_destructure = false;

        let result_local = self.declare_local("__for_result")?;
        self.emit(Instruction::simple(OpCode::PushNull));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        self.compile_expr(&for_expr.iterable)?;

        let idx_local = self.declare_local("__idx")?;
        let zero_const = self.program.add_constant(Constant::Number(0.0));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(zero_const)),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(idx_local)),
        ));

        match &for_expr.pattern {
            shape_ast::ast::Pattern::Identifier(name) => {
                elem_local = self.declare_local(name)?;
                is_object_destructure = false;
            }
            shape_ast::ast::Pattern::Object(fields) => {
                elem_local = self.declare_local("__elem")?;
                for (key, pat) in fields {
                    let field_name = match pat {
                        shape_ast::ast::Pattern::Identifier(n) => n.as_str(),
                        _ => key.as_str(),
                    };
                    let local = self.declare_local(field_name)?;
                    destructure_fields.push((key.clone(), local));
                }
                is_object_destructure = true;
            }
            shape_ast::ast::Pattern::Array(patterns) => {
                elem_local = self.declare_local("__elem")?;
                for pat in patterns {
                    let name = match pat {
                        shape_ast::ast::Pattern::Identifier(n) => n.clone(),
                        shape_ast::ast::Pattern::Wildcard => "__discard".to_string(),
                        _ => {
                            return Err(ShapeError::RuntimeError {
                                message:
                                    "Nested patterns in for-loop array destructure not supported"
                                        .to_string(),
                                location: None,
                            });
                        }
                    };
                    let local = self.declare_local(&name)?;
                    array_destructure_locals.push(local);
                }
                is_object_destructure = false;
                is_array_destructure = true;
            }
            shape_ast::ast::Pattern::Wildcard => {
                elem_local = self.declare_local("__discard")?;
                is_object_destructure = false;
            }
            _ => {
                return Err(ShapeError::RuntimeError {
                    message: "VM for-expr only supports identifier, object, and array destructure patterns"
                        .to_string(),
                    location: None,
                });
            }
        }
        self.apply_binding_semantics_to_value_pattern_bindings(
            &for_expr.pattern,
            Self::owned_mutable_binding_semantics(),
        );

        let loop_start = self.program.current_offset();
        self.emit(Instruction::simple(OpCode::LoopStart));

        self.emit(Instruction::simple(OpCode::Dup));
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(idx_local)),
        ));
        self.emit(Instruction::simple(OpCode::IterDone));
        let exit_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

        self.emit(Instruction::simple(OpCode::Dup));
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(idx_local)),
        ));
        self.emit(Instruction::simple(OpCode::IterNext));

        // For `for await`, each element is a Future — await it before binding
        if for_expr.is_async {
            self.emit(Instruction::simple(OpCode::Await));
        }

        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(elem_local)),
        ));

        // Object destructuring: extract fields from the element.
        if is_object_destructure {
            for (key, local) in &destructure_fields {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(elem_local)),
                ));
                let key_const = self.program.add_constant(Constant::String(key.to_string()));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(key_const)),
                ));
                self.emit(Instruction::simple(OpCode::GetProp));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(*local)),
                ));
            }
        }

        // Array destructuring: extract elements by index.
        if is_array_destructure {
            for (idx, local) in array_destructure_locals.iter().enumerate() {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(elem_local)),
                ));
                let idx_const = self.program.add_constant(Constant::Number(idx as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(idx_const)),
                ));
                self.emit(Instruction::simple(OpCode::GetProp));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(*local)),
                ));
            }
        }

        // Increment index before body so continue jumps advance correctly
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(idx_local)),
        ));
        let one_const = self.program.add_constant(Constant::Number(1.0));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(one_const)),
        ));
        self.emit(Instruction::simple(OpCode::Add));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(idx_local)),
        ));

        self.loop_stack.push(LoopContext {
            break_jumps: Vec::new(),
            continue_target: loop_start,
            break_value_local: Some(result_local),
            iterator_on_stack: true,
            drop_scope_depth: self.drop_locals.len(),
            continue_jumps: Vec::new(),
        });

        self.push_repeating_reference_release_barrier();
        let body_result = self.compile_expr(&for_expr.body);
        self.pop_repeating_reference_release_barrier();
        body_result?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
        self.emit(Instruction::new(
            OpCode::Jump,
            Some(Operand::Offset(offset)),
        ));

        self.emit(Instruction::simple(OpCode::LoopEnd));
        self.patch_jump(exit_jump);
        self.emit(Instruction::simple(OpCode::Pop));
        if let Some(loop_ctx) = self.loop_stack.pop() {
            for break_jump in loop_ctx.break_jumps {
                self.patch_jump(break_jump);
            }
        }
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));

        self.pop_scope();
        Ok(())
    }

    pub(super) fn compile_loop_expr(&mut self, loop_expr: &shape_ast::ast::LoopExpr) -> Result<()> {
        self.push_scope();

        let result_local = self.declare_local("__loop_result")?;
        self.emit(Instruction::simple(OpCode::PushNull));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        let loop_start = self.program.current_offset();
        self.emit(Instruction::simple(OpCode::LoopStart));
        self.loop_stack.push(LoopContext {
            break_jumps: Vec::new(),
            continue_target: loop_start,
            break_value_local: Some(result_local),
            iterator_on_stack: false,
            drop_scope_depth: self.drop_locals.len(),
            continue_jumps: Vec::new(),
        });

        self.push_repeating_reference_release_barrier();
        let body_result = self.compile_expr(&loop_expr.body);
        self.pop_repeating_reference_release_barrier();
        body_result?;
        // Discard the body value; break expressions store their values
        // to result_local themselves. We must Pop here so the stack
        // doesn't grow on each iteration.
        self.emit(Instruction::simple(OpCode::Pop));

        let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
        self.emit(Instruction::new(
            OpCode::Jump,
            Some(Operand::Offset(offset)),
        ));

        self.emit(Instruction::simple(OpCode::LoopEnd));

        if let Some(loop_ctx) = self.loop_stack.pop() {
            for break_jump in loop_ctx.break_jumps {
                self.patch_jump(break_jump);
            }
        }

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));

        self.pop_scope();
        Ok(())
    }

    pub(super) fn compile_list_comprehension(
        &mut self,
        comp: &shape_ast::ast::ListComprehension,
    ) -> Result<()> {
        self.push_scope();

        let result_local = self.declare_local("__comp_result")?;
        self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        self.compile_comprehension_clauses(&comp.element, &comp.clauses, result_local, 0)?;

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));

        self.pop_scope();
        Ok(())
    }

    pub(super) fn compile_comprehension_clauses(
        &mut self,
        element: &Expr,
        clauses: &[shape_ast::ast::ComprehensionClause],
        result_local: u16,
        depth: usize,
    ) -> Result<()> {
        if clauses.is_empty() {
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(result_local)),
            ));
            self.compile_expr(element)?;
            self.emit(Instruction::simple(OpCode::ArrayPush));
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(result_local)),
            ));
            return Ok(());
        }

        let clause = &clauses[0];

        // Try range counter specialization for this comprehension clause
        if let Some(rcl) = self.try_begin_range_counter_loop(
            clause.pattern.as_identifier(),
            &clause.iterable,
        )? {
            self.apply_binding_semantics_to_pattern_bindings(
                &clause.pattern,
                true,
                Self::owned_mutable_binding_semantics(),
            );

            if let Some(filter) = &clause.filter {
                self.compile_expr(filter)?;
                let skip_jump = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.compile_comprehension_clauses(
                    element,
                    &clauses[1..],
                    result_local,
                    depth + 1,
                )?;
                self.patch_jump(skip_jump);
            } else {
                self.compile_comprehension_clauses(
                    element,
                    &clauses[1..],
                    result_local,
                    depth + 1,
                )?;
            }

            // No LoopContext for comprehensions (no break/continue),
            // so end_range_counter_loop just emits increment + jump + patch.
            self.end_range_counter_loop(&rcl);

            return Ok(());
        }

        // === Generic iterator path (unchanged) ===

        self.compile_expr(&clause.iterable)?;
        let iter_local = self.declare_local(&format!("__comp_iter_{depth}"))?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(iter_local)),
        ));

        let idx_local = self.declare_local(&format!("__comp_idx_{depth}"))?;
        let zero_const = self.program.add_constant(Constant::Number(0.0));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(zero_const)),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(idx_local)),
        ));

        let loop_start = self.program.current_offset();

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(iter_local)),
        ));
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(idx_local)),
        ));
        self.emit(Instruction::simple(OpCode::IterDone));
        let exit_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(iter_local)),
        ));
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(idx_local)),
        ));
        self.emit(Instruction::simple(OpCode::IterNext));
        self.compile_destructure_pattern(&clause.pattern)?;
        self.apply_binding_semantics_to_pattern_bindings(
            &clause.pattern,
            true,
            Self::owned_mutable_binding_semantics(),
        );

        if let Some(filter) = &clause.filter {
            self.compile_expr(filter)?;
            let skip_jump = self.emit_jump(OpCode::JumpIfFalse, 0);
            self.compile_comprehension_clauses(element, &clauses[1..], result_local, depth + 1)?;
            self.patch_jump(skip_jump);
        } else {
            self.compile_comprehension_clauses(element, &clauses[1..], result_local, depth + 1)?;
        }

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(idx_local)),
        ));
        let one_const = self.program.add_constant(Constant::Number(1.0));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(one_const)),
        ));
        self.emit(Instruction::simple(OpCode::Add));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(idx_local)),
        ));

        let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
        self.emit(Instruction::new(
            OpCode::Jump,
            Some(Operand::Offset(offset)),
        ));

        self.patch_jump(exit_jump);

        Ok(())
    }

    pub(super) fn compile_array_with_spread(&mut self, elements: &[Expr]) -> Result<()> {
        self.push_scope();

        let result_local = self.declare_local("__array_result")?;
        self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        for (idx, elem) in elements.iter().enumerate() {
            match elem {
                Expr::Spread(inner, _) => {
                    // Try range counter specialization for spread-over-range
                    if let Expr::Range {
                        start: Some(start_expr),
                        end: Some(end_expr),
                        kind,
                        ..
                    } = inner.as_ref()
                    {
                        let inclusive = *kind == RangeKind::Inclusive;

                        let counter_local =
                            self.declare_local(&format!("__spread_counter_{idx}"))?;
                        let end_local = self.declare_local(&format!("__spread_end_{idx}"))?;

                        // Compile start → [NumberToInt if float] → store
                        self.compile_expr(start_expr)?;
                        let start_nt = self.last_expr_numeric_type;
                        if matches!(start_nt, Some(NumericType::Number)) {
                            self.emit(Instruction::simple(OpCode::NumberToInt));
                        }
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(counter_local)),
                        ));

                        // Compile end → [NumberToInt if float] → store
                        self.compile_expr(end_expr)?;
                        let end_nt = self.last_expr_numeric_type;
                        if matches!(end_nt, Some(NumericType::Number)) {
                            self.emit(Instruction::simple(OpCode::NumberToInt));
                        }
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(end_local)),
                        ));

                        let use_typed = matches!(
                            start_nt,
                            Some(NumericType::Int) | Some(NumericType::Number)
                        ) && matches!(
                            end_nt,
                            Some(NumericType::Int) | Some(NumericType::Number)
                        );

                        let loop_start = self.program.current_offset();

                        // counter < end (or <=)
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(counter_local)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(end_local)),
                        ));
                        if use_typed {
                            self.emit(Instruction::simple(if inclusive {
                                OpCode::LteInt
                            } else {
                                OpCode::LtInt
                            }));
                        } else {
                            self.emit(Instruction::simple(if inclusive {
                                OpCode::Lte
                            } else {
                                OpCode::Lt
                            }));
                        }
                        let exit_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

                        // Push counter value to result array
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(result_local)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(counter_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::ArrayPush));
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(result_local)),
                        ));

                        // Increment counter
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(counter_local)),
                        ));
                        if use_typed {
                            let one_const = self.program.add_constant(Constant::Int(1));
                            self.emit(Instruction::new(
                                OpCode::PushConst,
                                Some(Operand::Const(one_const)),
                            ));
                            self.emit(Instruction::simple(OpCode::AddInt));
                        } else {
                            let one_const = self.program.add_constant(Constant::Int(1));
                            self.emit(Instruction::new(
                                OpCode::PushConst,
                                Some(Operand::Const(one_const)),
                            ));
                            self.emit(Instruction::simple(OpCode::Add));
                        }
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(counter_local)),
                        ));

                        let offset =
                            loop_start as i32 - self.program.current_offset() as i32 - 1;
                        self.emit(Instruction::new(
                            OpCode::Jump,
                            Some(Operand::Offset(offset)),
                        ));

                        self.patch_jump(exit_jump);
                    } else {
                        // Generic iterator path for non-range spreads
                        self.plan_flexible_binding_escape_from_expr(inner);
                        self.compile_expr(inner)?;
                        let iter_local =
                            self.declare_local(&format!("__spread_iter_{idx}"))?;
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(iter_local)),
                        ));

                        let idx_local =
                            self.declare_local(&format!("__spread_idx_{idx}"))?;
                        let zero_const = self.program.add_constant(Constant::Number(0.0));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(zero_const)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(idx_local)),
                        ));

                        let loop_start = self.program.current_offset();

                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(iter_local)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(idx_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::IterDone));
                        let exit_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(result_local)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(iter_local)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(idx_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::IterNext));
                        self.emit(Instruction::simple(OpCode::ArrayPush));
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(result_local)),
                        ));

                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(idx_local)),
                        ));
                        let one_const = self.program.add_constant(Constant::Number(1.0));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(one_const)),
                        ));
                        self.emit(Instruction::simple(OpCode::Add));
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(idx_local)),
                        ));

                        let offset =
                            loop_start as i32 - self.program.current_offset() as i32 - 1;
                        self.emit(Instruction::new(
                            OpCode::Jump,
                            Some(Operand::Offset(offset)),
                        ));

                        self.patch_jump(exit_jump);
                    }
                }
                _ => {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(result_local)),
                    ));
                    self.plan_flexible_binding_escape_from_expr(elem);
                    self.compile_expr(elem)?;
                    self.emit(Instruction::simple(OpCode::ArrayPush));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(result_local)),
                    ));
                }
            }
        }

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));

        self.pop_scope();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::VMConfig;
    use crate::compiler::BytecodeCompiler;
    use crate::executor::VirtualMachine;
    use shape_ast::parser::parse_program;

    fn compile_and_run(code: &str) -> shape_value::ValueWord {
        let program = parse_program(code).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        let bytecode = compiler.compile(&program).unwrap();
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.execute(None).unwrap().clone()
    }

    #[test]
    fn test_range_loop_exclusive() {
        let result = compile_and_run(
            "fn t() { let mut s = 0; for i in 0..5 { s = s + i }; s } t()",
        );
        assert_eq!(result.as_i64(), Some(10));
    }

    #[test]
    fn test_range_loop_inclusive() {
        let result = compile_and_run(
            "fn t() { let mut s = 0; for i in 0..=5 { s = s + i }; s } t()",
        );
        assert_eq!(result.as_i64(), Some(15));
    }

    #[test]
    fn test_range_loop_empty() {
        let result = compile_and_run(
            "fn t() { let mut s = 0; for i in 5..0 { s = s + i }; s } t()",
        );
        assert_eq!(result.as_i64(), Some(0));
    }

    #[test]
    fn test_range_loop_break() {
        let result = compile_and_run(
            "fn t() { let mut s = 0; for i in 0..100 { if i == 5 { break }; s = s + i }; s } t()",
        );
        assert_eq!(result.as_i64(), Some(10));
    }

    #[test]
    fn test_range_loop_continue() {
        let result = compile_and_run(
            "fn t() { let mut s = 0; for i in 0..10 { if i % 2 == 0 { continue }; s = s + i }; s } t()",
        );
        assert_eq!(result.as_i64(), Some(25));
    }

    #[test]
    fn test_range_loop_no_makerange() {
        let code = "fn t() { let mut s = 0; for i in 0..10 { s = s + i }; s }";
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();
        assert!(
            !opcodes.contains(&crate::bytecode::OpCode::MakeRange),
            "Range counter loop must not emit MakeRange"
        );
        assert!(
            !opcodes.contains(&crate::bytecode::OpCode::IterDone),
            "Range counter loop must not emit IterDone"
        );
    }

    #[test]
    fn test_range_loop_for_expr() {
        let result = compile_and_run(
            "fn t() { let r = for i in 0..5 { i * 2 }; r } t()",
        );
        assert_eq!(result.as_i64(), Some(8));
    }

    #[test]
    fn test_range_loop_comprehension() {
        let result = compile_and_run(
            "fn t() { let a = [i * 2 for i in 0..5]; a.len() } t()",
        );
        assert_eq!(result.as_i64(), Some(5));
    }

    #[test]
    fn test_range_loop_spread() {
        let result = compile_and_run(
            "fn t() { let a = [...0..5]; a.len() } t()",
        );
        assert_eq!(result.as_i64(), Some(5));
    }

    #[test]
    fn test_non_range_fallback() {
        let result = compile_and_run(
            "fn t() { let mut s = 0; for x in [10, 20, 30] { s = s + x }; s } t()",
        );
        assert_eq!(result.as_i64(), Some(60));
    }
}
