//! Loop compilation (for, while, loop expressions)

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use shape_ast::ast::{Expr, ForInit};
use shape_ast::error::{Result, ShapeError};

use super::{BytecodeCompiler, LoopContext};

impl BytecodeCompiler {
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
                // Compile for-in loop
                self.push_scope();

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

                let loop_start = self.program.current_offset();
                self.emit(Instruction::simple(OpCode::LoopStart));
                let loop_ctx = LoopContext {
                    break_jumps: Vec::new(),
                    continue_target: loop_start,
                    break_value_local: None,
                    iterator_on_stack: true,
                    drop_scope_depth: self.drop_locals.len(),
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

        // Determine binding pattern: simple identifier, object destructure, or array destructure.
        let elem_local;
        let mut destructure_fields: Vec<(String, u16)> = Vec::new();
        let mut array_destructure_locals: Vec<u16> = Vec::new();
        let is_object_destructure;
        let mut is_array_destructure = false;

        self.push_scope();

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
                    self.compile_expr(inner)?;
                    let iter_local = self.declare_local(&format!("__spread_iter_{idx}"))?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(iter_local)),
                    ));

                    let idx_local = self.declare_local(&format!("__spread_idx_{idx}"))?;
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

                    let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
                    self.emit(Instruction::new(
                        OpCode::Jump,
                        Some(Operand::Offset(offset)),
                    ));

                    self.patch_jump(exit_jump);
                }
                _ => {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(result_local)),
                    ));
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
