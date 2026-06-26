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

        // Strict-typing-sweep (Cluster 4): the counter is always `int` (range
        // endpoints are coerced to int in the prologue). Without installing
        // this in the type tracker, binary ops on the IV (`i + 1`, `s + i`)
        // inside the loop body see the IV as `unknown` and fail strict-typing.
        self.set_local_type_info(counter_local, "int");
        self.set_local_type_info(end_local, "int");

        // compile(start) → [NumberToInt if float] → StoreLocal(counter)
        // U4-4: endpoint numeric kind derived from the one resolved Type.
        self.compile_expr(start_expr)?;
        let start_nt = self.numeric_type_of(start_expr);
        if matches!(start_nt, Some(NumericType::Number)) {
            self.emit(Instruction::simple(OpCode::NumberToInt));
        }
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(counter_local)),
        ));

        // compile(end) → [NumberToInt if float] → StoreLocal(__end)
        self.compile_expr(end_expr)?;
        let end_nt = self.numeric_type_of(end_expr);
        if matches!(end_nt, Some(NumericType::Number)) {
            self.emit(Instruction::simple(OpCode::NumberToInt));
        }
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(end_local)),
        ));

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
        // Both endpoints are always int at this point (Int directly, or
        // Number after NumberToInt conversion in the prologue).
        self.emit(Instruction::simple(if inclusive {
            OpCode::LteInt
        } else {
            OpCode::LtInt
        }));

        // JumpIfFalse(exit)
        let exit_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

        Ok(Some(RangeCounterLoopState {
            counter_local,
            loop_start,
            exit_jump,
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
        // Counter is always int (range endpoints coerced to int in prologue)
        let one_const = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(one_const)),
        ));
        self.emit(Instruction::simple(OpCode::AddInt));
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
                    if let Some(rcl) =
                        self.try_begin_range_counter_loop(pattern.as_identifier(), iter)?
                    {
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

                // Initialize index to 0 (int — internal counter, always integer)
                let zero_const = self.program.add_constant(Constant::Int(0));
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

                // Phase 3e: propagate iterator element type to the loop
                // variable. Without this, `for x in arr` (over an
                // Array<int>) declares `x` with no type info, so
                // `sum + x` can't emit AddInt and falls into trait
                // dispatch (which has no runtime handler for int.add).
                //
                // Only handles the common single-identifier pattern.
                // Complex destructuring patterns continue to leave the
                // loop var(s) untyped — bidirectional inference can
                // recover them when needed.
                if let Some(var_name) = pattern.as_identifier() {
                    if let Some(local_idx) = self.resolve_local(var_name) {
                        let name_from_string_path = self.iter_element_type_name(iter);
                        if let Some(ref elem_type) = name_from_string_path {
                            self.set_local_type_info(local_idx, elem_type);
                        }
                        // R3-subcase struct-array HOF (strict-flip, 2026-06-15):
                        // carry the element's full ConcreteType (struct/enum
                        // identity, not just the tracker name string) so a
                        // `for u in users { u.score }` field access — and the
                        // `result.push(item)` accumulator in the monomorphized
                        // `Vec.filter` body — resolves `u`/`item` to the named
                        // struct rather than `unknown`.
                        if let Some(elem_ct) = self.iter_element_concrete_type(iter) {
                            // ROOT-1 (strict-flip, 2026-06-18): the string-name
                            // path (`iter_element_type_name`) only resolves
                            // primitive-element literals + name-tracked bindings;
                            // an inline struct-array literal (`for p in [R{..}]`)
                            // or an inferred struct-array binding fell to the
                            // ConcreteType side-table ONLY, leaving the tracker
                            // NAME `unknown` so `p.age` failed to infer. Derive
                            // the tracker NAME from the proven element ConcreteType
                            // when the string path missed. ConcreteType IS the
                            // proof (ADR-006 §2.7.5); a no-stable-name shape stamps
                            // nothing (surface-and-stop preserved).
                            if name_from_string_path.is_none() {
                                if let Some(tn) =
                                    crate::compiler::patterns::binding::concrete_type_tracker_name(
                                        &elem_ct,
                                    )
                                {
                                    self.set_local_type_info(local_idx, &tn);
                                }
                            }
                            crate::compiler::monomorphization::type_resolution::record_binding_concrete_fact(
                                self,
                                crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::Local(local_idx),
                                elem_ct,
                                crate::compiler::BindingConcreteFactSource::IteratorElement,
                            );
                        }
                    }
                }

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

                // T1 sub-case (d) (strict-flip, 2026-06-20): the STATEMENT-form
                // for-in (`for {x, y} in points`) destructure-binds each field
                // via `compile_destructure_pattern` but — unlike the expression-
                // form `compile_for_expr` — never stamped the bound field
                // locals' tracker types, so a body `total + x + y` rejected the
                // destructured operands as `unknown`. Stamp each field from the
                // element's type: a NAMED struct via the schema field type, or an
                // ANONYMOUS object-literal element via its inferred field
                // annotation (the shared inference engine resolved it). The
                // element type IS the proof (ADR-006 §2.7.5); a field with no
                // scalar tracker name stamps nothing (surface-and-stop).
                if let shape_ast::ast::DestructurePattern::Object(fields) = pattern {
                    let mut named_done = false;
                    if let Some(shape_value::v2::ConcreteType::Struct(layout)) =
                        self.iter_element_concrete_type(iter)
                    {
                        if let Some(struct_name) = layout.name.as_ref().map(|n| n.to_string()) {
                            for f in fields {
                                let binder = f
                                    .pattern
                                    .as_identifier()
                                    .unwrap_or(f.key.as_str())
                                    .to_string();
                                if let Some(local_idx) = self.resolve_local(&binder) {
                                    if let Some(tn) =
                                        self.struct_field_tracker_type_name(&struct_name, &f.key)
                                    {
                                        self.set_local_type_info(local_idx, &tn);
                                    }
                                }
                            }
                            named_done = true;
                        }
                    }
                    if !named_done {
                        if let Some(elem_fields) = self.anonymous_object_element_fields(iter) {
                            for f in fields {
                                let binder = f
                                    .pattern
                                    .as_identifier()
                                    .unwrap_or(f.key.as_str())
                                    .to_string();
                                if let Some(local_idx) = self.resolve_local(&binder) {
                                    if let Some(tn) = elem_fields
                                        .iter()
                                        .find(|ef| ef.name == f.key)
                                        .and_then(|ef| {
                                            crate::compiler::loops::type_annotation_scalar_tracker_name(
                                                &ef.type_annotation,
                                            )
                                        })
                                    {
                                        self.set_local_type_info(local_idx, &tn);
                                    }
                                }
                            }
                        }
                    }
                }

                // Increment index before body so continue jumps advance correctly
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(idx_local)),
                ));
                let one_const = self.program.add_constant(Constant::Int(1));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(one_const)),
                ));
                self.emit(Instruction::simple(OpCode::AddInt));
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
                shape_ast::ast::Pattern::Identifier { name, .. } => Some(name.as_str()),
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
        let zero_const = self.program.add_constant(Constant::Int(0));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(zero_const)),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(idx_local)),
        ));

        match &for_expr.pattern {
            shape_ast::ast::Pattern::Identifier { name, .. } => {
                elem_local = self.declare_local(name)?;
                is_object_destructure = false;
            }
            shape_ast::ast::Pattern::Object(fields) => {
                elem_local = self.declare_local("__elem")?;
                for (key, pat) in fields {
                    let field_name = match pat {
                        shape_ast::ast::Pattern::Identifier { name, .. } => name.as_str(),
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
                        shape_ast::ast::Pattern::Identifier { name, .. } => name.clone(),
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

        // Phase 3e: propagate iterator element type to the loop variable
        // for the simple identifier-pattern form. Same fix as
        // `compile_for_loop`; `for x in arr` over `Array<int>` now
        // declares `x` with tracker type `int` so `sum + x` emits
        // `AddInt` rather than falling into trait dispatch.
        if let shape_ast::ast::Pattern::Identifier { .. } = &for_expr.pattern {
            let name_from_string_path = self.iter_element_type_name(&for_expr.iterable);
            if let Some(ref elem_type) = name_from_string_path {
                self.set_local_type_info(elem_local, elem_type);
            }
            // R3-subcase struct-array HOF (strict-flip, 2026-06-15): see the
            // matching site in `compile_for_loop` — carry the struct/enum
            // element identity to the loop variable's ConcreteType.
            if let Some(elem_ct) = self.iter_element_concrete_type(&for_expr.iterable) {
                // ROOT-1 (strict-flip, 2026-06-18): an inline struct-array
                // literal (`for p in [R{..}]`) leaves the string-name path
                // empty; derive the tracker NAME from the proven element
                // ConcreteType so `p.age` is field-accessible (the read sites
                // consult the tracker NAME, not the ConcreteType side-table).
                // ConcreteType IS the proof (ADR-006 §2.7.5). Mirror of the
                // `compile_for_loop` site.
                if name_from_string_path.is_none() {
                    if let Some(tn) =
                        crate::compiler::patterns::binding::concrete_type_tracker_name(&elem_ct)
                    {
                        self.set_local_type_info(elem_local, &tn);
                    }
                }
                crate::compiler::monomorphization::type_resolution::record_binding_concrete_fact(
                    self,
                    crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::Local(elem_local),
                    elem_ct,
                    crate::compiler::BindingConcreteFactSource::IteratorElement,
                );
            }
        }
        // ROOT-1 (strict-flip, 2026-06-18): destructuring for-in
        // (`for {x, y} in [P{..}]` / `for [a, b] in [[1,2]]`) previously left
        // EVERY bound field/element untyped — the prior code stamped only the
        // single-identifier loop var. Recover the element's named struct
        // ConcreteType and stamp each destructured field's tracker type from
        // the struct schema field types (object form) so `x + y` infers. The
        // element ConcreteType IS the proof (ADR-006 §2.7.5); a non-struct or
        // unresolvable element stamps nothing (surface-and-stop preserved).
        if is_object_destructure {
            let mut stamped_via_named = false;
            if let Some(shape_value::v2::ConcreteType::Struct(layout)) =
                self.iter_element_concrete_type(&for_expr.iterable)
            {
                if let Some(struct_name) = layout.name.as_ref().map(|n| n.to_string()) {
                    for (key, local) in &destructure_fields {
                        if let Some(tn) = self.struct_field_tracker_type_name(&struct_name, key) {
                            self.set_local_type_info(*local, &tn);
                        }
                    }
                    stamped_via_named = true;
                }
            }
            // T1 sub-case (d) (strict-flip, 2026-06-20): an ANONYMOUS
            // object-literal element (`for {x, y} in [{x: 1, y: 2}]` /
            // `for {x, y} in points` where `points = [{x:1,y:2}]`) has no
            // registered struct NAME, so the named path above misses and the
            // destructured fields erased — `x + y` rejected as
            // `unknown + unknown`. Recover each field's tracker type from a
            // representative element OBJECT LITERAL: the field-value expression's
            // proven ConcreteType IS the proof (ADR-006 §2.7.5). A field with no
            // statically-mappable kind stamps nothing (surface-and-stop). PER-
            // SITE-ARM, int != number preserved (the field value's own kind).
            if !stamped_via_named {
                let af = self.anonymous_object_element_fields(&for_expr.iterable);
                if let Some(elem_fields) = af {
                    for (key, local) in &destructure_fields {
                        if let Some(tn) =
                            elem_fields.iter().find(|f| f.name == *key).and_then(|f| {
                                crate::compiler::loops::type_annotation_scalar_tracker_name(
                                    &f.type_annotation,
                                )
                            })
                        {
                            self.set_local_type_info(*local, &tn);
                        }
                    }
                }
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
        let one_const = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(one_const)),
        ));
        self.emit(Instruction::simple(OpCode::AddInt));
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

    /// W16.2-C (Round 6 WS-1, 2026-05-21) — resolve the [`TypedArrayKind`]
    /// of a list-comprehension element expression that just compiled.
    ///
    /// Reads, in order:
    ///   1. The element's one resolved Type (`numeric_type_of` →
    ///      `infer_expr_type`) — covers `x * 2`, `x + 1`, range-counter loop
    ///      variables which the range specialization types as `int`. (U4-4:
    ///      this REPLACES the deleted ambient `last_expr_numeric_type`
    ///      register.)
    ///   2. `last_expr_type_info`'s `storage_hint` — covers the `bool`
    ///      case (a comparison result stamps `StorageHint::Bool`).
    ///   3. `concrete_type_for_expr` on the element AST — covers a bare
    ///      identifier loop variable bound by a generic-iterator clause
    ///      (`[x for x in src]`), where the kind lives in the type tracker.
    ///
    /// Per ADR-006 §2.7.5 every signal is a producer-side type proof (or a
    /// structural type-tracker fact) — never fabricated, never decoded from
    /// runtime bits. Returns `None` when no scalar kind is proven; the caller
    /// surfaces a clean compile error.
    fn resolve_pushed_element_typed_array_kind(
        &mut self,
        element: &Expr,
    ) -> Option<super::v2_typed_emission::TypedArrayKind> {
        use super::monomorphization::type_resolution::concrete_type_for_expr;
        use super::v2_typed_emission::{
            TypedArrayKind, should_use_typed_array, typed_array_kind_from_numeric_type,
        };
        if let Some(nt) = self.numeric_type_of(element) {
            return Some(typed_array_kind_from_numeric_type(nt));
        }
        if let Some(info) = &self.last_expr_type_info {
            if info.storage_hint == Some(crate::type_tracking::NativeKind::Bool) {
                return Some(TypedArrayKind::Bool);
            }
        }
        // Structural fallback — a bare identifier whose tracked type is a
        // scalar (the generic-iterator loop variable case).
        concrete_type_for_expr(self, element).and_then(|ct| should_use_typed_array(&ct))
    }

    pub(super) fn compile_list_comprehension(
        &mut self,
        comp: &shape_ast::ast::ListComprehension,
    ) -> Result<()> {
        self.push_scope();

        let result_local = self.declare_local("__comp_result")?;
        // W16.2-C (Round 6 WS-1, 2026-05-21): the result accumulator MUST be
        // a v2 typed array — `op_array_push` only accepts a
        // `Ptr(HeapKind::TypedArray)` receiver, and there is no untyped
        // runtime array carrier. The element kind is proven only AFTER the
        // body compiles, so emit a placeholder allocator here, record its
        // instruction index, then patch it once
        // `compile_comprehension_clauses` writes the proven
        // `comprehension_element_kind`. Per ADR-006 §2.7.5 the kind is
        // stamped at the producer site (the compiled element expression) —
        // never decoded from runtime bits, never Bool-defaulted.
        let alloc_instr_idx = self.program.instructions.len();
        self.emit(Instruction::new(OpCode::NewArray, Some(Operand::Count(0))));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        // Save/restore both comprehension-scoped fields so a nested
        // comprehension (`[[y for y in r] for x in ...]`) does not bleed
        // its element kind / push sites into the enclosing one.
        let saved_element_kind = self.comprehension_element_kind.take();
        let saved_push_sites = std::mem::take(&mut self.comprehension_push_sites);
        self.compile_comprehension_clauses(&comp.element, &comp.clauses, result_local, 0)?;
        let element_kind = self.comprehension_element_kind.take();
        let push_sites = std::mem::take(&mut self.comprehension_push_sites);
        self.comprehension_element_kind = saved_element_kind;
        self.comprehension_push_sites = saved_push_sites;

        match element_kind {
            Some(kind) => {
                // Patch the placeholder allocator with the resolved typed
                // allocator (capacity 0). Record the typed-array kind
                // against the result slot so downstream `.method()`
                // dispatch resolves the carrier.
                self.program.instructions[alloc_instr_idx] =
                    Instruction::new(kind.new_opcode(), Some(Operand::Count(0)));
                // Patch every element-push site to the matching typed
                // `TypedArrayPush*` opcode — the typed push unambiguously
                // identifies the v2 typed-array carrier for both the VM
                // and JIT (no generic-carrier slot-kind ambiguity).
                for &site in &push_sites {
                    self.program.instructions[site] = Instruction::simple(kind.push_opcode());
                }
                self.v2_typed_array_locals.insert(result_local, kind);
                // Signal the typed-array kind to the enclosing `let c = [...]`
                // binding path (`Statement::VarDecl`), which records it
                // against the destination slot via
                // `pending_variable_typed_array_kind` — the same hand-off
                // `compile_expr_array` uses for bare typed literals. Without
                // this the destination binding is untyped and `.len()` /
                // method dispatch on it falls to the generic carrier path.
                self.pending_variable_typed_array_kind = Some(kind);
            }
            None => {
                return Err(ShapeError::SemanticError {
                    message: "list comprehension element type could not be \
                              determined at compile time. Strict typing \
                              requires the element expression to have a \
                              proven scalar type (int / number / bool / \
                              decimal / sized integer). Annotate the \
                              comprehension's source so the element type \
                              resolves."
                        .to_string(),
                    location: None,
                });
            }
        }

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));

        // The comprehension body compiled the element expression, leaving
        // `last_expr_numeric_type` stamped with the ELEMENT's scalar type
        // (e.g. `Int` for `[x for x in 0..5]`). If left set, the enclosing
        // `let c = [...]` binding's `propagate_initializer_type_to_slot`
        // would record `c` as a bare `int`, mis-stamping the method-call
        // receiver tag. Reset to the array shape — mirrors the tail of
        // `compile_expr_array`.
        if let Some(kind) = element_kind {
            self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::named(
                super::v2_typed_emission::vec_type_name_for_typed_array_kind(kind).to_string(),
            ));
        } else {
            self.last_expr_type_info = None;
        }
        self.last_expr_schema = None;

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
            // W16.2-C (Round 6 WS-1): capture the proven element kind for
            // `compile_list_comprehension` to patch the accumulator
            // allocator with the matching typed `NewTypedArray*` opcode. The
            // element expression has just compiled — `last_expr_numeric_type`
            // / `last_expr_type_info` carry the producer-side type proof per
            // ADR-006 §2.7.5. A `bool`-typed element (comparison result)
            // surfaces via `last_expr_type_info.storage_hint == Bool`.
            let resolved_kind = self.resolve_pushed_element_typed_array_kind(element);
            // Every base-case visit compiles the SAME element expression, so
            // the resolved kind is identical across loop/filter clauses;
            // recording it once is sufficient. If a later visit somehow
            // disagrees, downgrade to `None` (un-provable) rather than
            // silently picking one.
            match (self.comprehension_element_kind, resolved_kind) {
                (None, k) => self.comprehension_element_kind = k,
                (Some(prev), Some(k)) if prev == k => {}
                (Some(_), _) => self.comprehension_element_kind = None,
            }
            // Emit a placeholder `ArrayPush` and record its index.
            // `compile_list_comprehension` patches it to the matching
            // `TypedArrayPush*` opcode once the element kind is resolved.
            // The typed push pops `[arr, val]` and pushes nothing back
            // (the `TypedArray<T>` struct pointer is stable across the
            // in-place `TypedArray::push` — only the inner data buffer
            // reallocs), so NO `StoreLocal` re-store is emitted: the
            // accumulator slot already holds the stable pointer. The
            // placeholder `ArrayPush` is never executed — it is always
            // patched before the program runs (or the comprehension
            // fails to compile).
            self.comprehension_push_sites
                .push(self.program.instructions.len());
            self.emit(Instruction::simple(OpCode::ArrayPush));
            return Ok(());
        }

        let clause = &clauses[0];

        // Try range counter specialization for this comprehension clause
        if let Some(rcl) =
            self.try_begin_range_counter_loop(clause.pattern.as_identifier(), &clause.iterable)?
        {
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
        let zero_const = self.program.add_constant(Constant::Int(0));
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

        // W16.2-C (Round 6 WS-1, 2026-05-21): type the comprehension loop
        // variable from the iterable's element type — mirrors the `for x in
        // iter` loop-variable typing (`compile_for_loop` `iter_element_type
        // _name` + `set_local_type_info`). Without this, `[x for x in arr]`
        // leaves `x` untyped and `resolve_pushed_element_typed_array_kind`
        // cannot prove the comprehension's element kind. Per ADR-006 §2.7.5
        // the iterable's tracked `Array<T>` element type IS the proof.
        if let (Some(loop_var), Some(elem_type)) = (
            clause.pattern.as_identifier(),
            self.iter_element_type_name(&clause.iterable),
        ) {
            if let Some(local_idx) = self.resolve_local(loop_var) {
                self.set_local_type_info(local_idx, &elem_type);
            }
        }

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
        let one_const = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(one_const)),
        ));
        self.emit(Instruction::simple(OpCode::AddInt));
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

    /// W16.2-C (Round 6 WS-1, 2026-05-21) — resolve the homogeneous
    /// [`TypedArrayKind`] of an array-spread literal's elements.
    ///
    /// Walks every element of `[...src, tail, ...]`: a `...src` spread
    /// contributes `src`'s array-element type (a `0..n` range spread
    /// contributes `int`); a bare element contributes its own type. The
    /// accumulator kind is the single kind every element agrees on. Returns
    /// `None` for a genuinely heterogeneous literal (e.g. `[...intArr,
    /// "str"]`) or an element whose type is not statically provable — the
    /// caller surfaces a clean compile error. Per ADR-006 §2.7.5 every kind
    /// is proven structurally at the producer site; no runtime inference.
    fn resolve_spread_accumulator_kind(
        &self,
        elements: &[Expr],
    ) -> Option<super::v2_typed_emission::TypedArrayKind> {
        use super::monomorphization::type_resolution::concrete_type_for_expr;
        use super::v2_typed_emission::should_use_typed_array;
        use shape_value::v2::ConcreteType;

        let mut acc: Option<super::v2_typed_emission::TypedArrayKind> = None;
        for elem in elements {
            let elem_kind = match elem {
                Expr::Spread(inner, _) => {
                    // A `0..n` / `0..=n` range spread yields `int` counters.
                    if let Expr::Range {
                        start: Some(_),
                        end: Some(_),
                        ..
                    } = inner.as_ref()
                    {
                        Some(super::v2_typed_emission::TypedArrayKind::I64)
                    } else {
                        // Any other spread source must be an `Array<T>`;
                        // its element type is the contributed kind.
                        match concrete_type_for_expr(self, inner) {
                            Some(ConcreteType::Array(inner_ct)) => {
                                should_use_typed_array(&inner_ct)
                            }
                            _ => None,
                        }
                    }
                }
                _ => concrete_type_for_expr(self, elem).and_then(|ct| should_use_typed_array(&ct)),
            };
            let elem_kind = elem_kind?;
            match acc {
                None => acc = Some(elem_kind),
                Some(prev) if prev == elem_kind => {}
                // Heterogeneous element kinds — not a typed-array literal.
                Some(_) => return None,
            }
        }
        acc
    }

    pub(super) fn compile_array_with_spread(&mut self, elements: &[Expr]) -> Result<()> {
        self.push_scope();

        // W16.2-C (Round 6 WS-1): the spread accumulator MUST be a v2 typed
        // array — `op_array_push` rejects any non-`Ptr(HeapKind::TypedArray)`
        // receiver. Resolve the homogeneous element kind structurally before
        // emission (per ADR-006 §2.7.5 the producer-side proof is the spread
        // sources' / tail elements' statically-known `ConcreteType`s).
        let accumulator_kind = self.resolve_spread_accumulator_kind(elements);
        let result_local = self.declare_local("__array_result")?;
        match accumulator_kind {
            Some(kind) => {
                self.emit(Instruction::new(kind.new_opcode(), Some(Operand::Count(0))));
                self.v2_typed_array_locals.insert(result_local, kind);
                // Signal the kind to the enclosing `let b = [...spread]`
                // binding path so the destination slot is recorded as a
                // typed array (mirrors `compile_expr_array`).
                self.pending_variable_typed_array_kind = Some(kind);
            }
            None => {
                return Err(ShapeError::SemanticError {
                    message: "array spread element types could not be \
                              reconciled at compile time. Strict typing \
                              requires every spread source and bare element \
                              to share one proven scalar element type \
                              (int / number / bool / decimal / sized \
                              integer). A heterogeneous spread literal is \
                              not supported."
                        .to_string(),
                    location: None,
                });
            }
        }
        let accumulator_kind =
            accumulator_kind.expect("spread accumulator kind was validated above");
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
                        // U4-4: endpoint numeric kind from the one resolved Type.
                        self.compile_expr(start_expr)?;
                        let start_nt = self.numeric_type_of(start_expr);
                        if matches!(start_nt, Some(NumericType::Number)) {
                            self.emit(Instruction::simple(OpCode::NumberToInt));
                        }
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(counter_local)),
                        ));

                        // Compile end → [NumberToInt if float] → store
                        self.compile_expr(end_expr)?;
                        let end_nt = self.numeric_type_of(end_expr);
                        if matches!(end_nt, Some(NumericType::Number)) {
                            self.emit(Instruction::simple(OpCode::NumberToInt));
                        }
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(end_local)),
                        ));

                        let loop_start = self.program.current_offset();

                        // counter < end (or <=) — always int after NumberToInt coercion
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(counter_local)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(end_local)),
                        ));
                        self.emit(Instruction::simple(if inclusive {
                            OpCode::LteInt
                        } else {
                            OpCode::LtInt
                        }));
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
                        self.emit(Instruction::simple(accumulator_kind.push_opcode()));

                        // Increment counter
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(counter_local)),
                        ));
                        // Counter is always int (range endpoints coerced to int)
                        let one_const = self.program.add_constant(Constant::Int(1));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(one_const)),
                        ));
                        self.emit(Instruction::simple(OpCode::AddInt));
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(counter_local)),
                        ));

                        let offset = loop_start as i32 - self.program.current_offset() as i32 - 1;
                        self.emit(Instruction::new(
                            OpCode::Jump,
                            Some(Operand::Offset(offset)),
                        ));

                        self.patch_jump(exit_jump);
                    } else {
                        // Generic iterator path for non-range spreads
                        self.plan_flexible_binding_escape_from_expr(inner);
                        self.compile_expr(inner)?;
                        let iter_local = self.declare_local(&format!("__spread_iter_{idx}"))?;
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(iter_local)),
                        ));

                        let idx_local = self.declare_local(&format!("__spread_idx_{idx}"))?;
                        let zero_const = self.program.add_constant(Constant::Int(0));
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
                        self.emit(Instruction::simple(accumulator_kind.push_opcode()));

                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(idx_local)),
                        ));
                        let one_const = self.program.add_constant(Constant::Int(1));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(one_const)),
                        ));
                        self.emit(Instruction::simple(OpCode::AddInt));
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
                }
                _ => {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(result_local)),
                    ));
                    self.plan_flexible_binding_escape_from_expr(elem);
                    self.compile_expr(elem)?;
                    self.emit(Instruction::simple(accumulator_kind.push_opcode()));
                }
            }
        }

        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));

        // Reset `last_expr_*` to the array shape so the enclosing
        // `let b = [...spread]` binding records `b` as an array, not as
        // the bare element scalar the last spread element left behind.
        self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::named(
            super::v2_typed_emission::vec_type_name_for_typed_array_kind(accumulator_kind)
                .to_string(),
        ));
        self.last_expr_schema = None;

        self.pop_scope();
        Ok(())
    }

    /// ROOT-1 (strict-flip, 2026-06-18): the type-tracker NAME for a named
    /// struct's field, used to type a destructuring for-in binding
    /// (`for {x, y} in [P{..}]`). Reads the field's `FieldType` off the
    /// registered struct schema and maps it to the tracker name the
    /// strict-typing binop emitter recognises. Returns `None` for an unknown
    /// struct / field or a field type with no stable scalar tracker name
    /// (surface-and-stop preserved — the binder then stays untyped). The
    /// schema field type IS the proof (ADR-006 §2.7.5).
    pub(super) fn struct_field_tracker_type_name(
        &self,
        struct_name: &str,
        field_name: &str,
    ) -> Option<String> {
        use shape_runtime::type_schema::FieldType;
        let schema = self.type_tracker.schema_registry().get(struct_name)?;
        let field = schema.get_field(field_name)?;
        let name = match &field.field_type {
            FieldType::I64 => "int",
            FieldType::F64 => "number",
            FieldType::Bool => "bool",
            FieldType::String => "string",
            FieldType::Decimal => "decimal",
            FieldType::I8 => "i8",
            FieldType::U8 => "u8",
            FieldType::I16 => "i16",
            FieldType::U16 => "u16",
            FieldType::I32 => "i32",
            FieldType::U32 => "u32",
            FieldType::U64 => "u64",
            FieldType::Object(name) => name.as_str(),
            // Array / Option / HashMap / Any / Timestamp / enum payloads:
            // no stable scalar tracker name — leave the binder untyped.
            _ => return None,
        };
        Some(name.to_string())
    }

    /// T1 sub-case (d) (strict-flip, 2026-06-20): resolve the iterable of a
    /// `for {x, y} in ITER` to its element's ANONYMOUS object field list, when
    /// the element is a structural object (object-literal array) rather than a
    /// named struct. Reads the SHARED inference engine's resolved iterable type
    /// (the same engine that ran the full program pass, so `points`'s element
    /// type `Object([{x:int},{y:int}])` is already solved) and unwraps one
    /// array layer. Returns `None` for a non-array / non-object-element iterable
    /// (the named-struct path handled those, or the binder stays untyped —
    /// surface-and-stop). The resolved element annotation IS the proof
    /// (ADR-006 §2.7.5); no fabrication.
    pub(super) fn anonymous_object_element_fields(
        &mut self,
        iterable: &Expr,
    ) -> Option<Vec<shape_ast::ast::ObjectTypeField>> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;
        // U4-6a: resolve the iterable's type from the inference engine's
        // per-expression span table via `infer_expr_type`. The engine (post-U4
        // span-table keystone) records the resolved type of the iterable
        // expression — including an IDENTIFIER iterable bound to an anonymous
        // object-literal array (`let points = [{x:1,y:2}]; for {x,y} in points`)
        // — keyed by the use-site span, so a fresh `infer_expr` is no longer
        // needed and no longer errors `UndefinedVariable`. The former
        // `binding_object_element_fields` side-table (a frozen projection
        // recorded at let-binding compile time to work around the empty-env
        // re-run) is deleted: the engine span-table is the single source of
        // truth for the element object's field annotations.
        let iter_ty = self.infer_expr_type(iterable).ok()?;
        let elem_ann = match &iter_ty {
            Type::Concrete(TypeAnnotation::Array(inner)) => (**inner).clone(),
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if args.len() == 1 && matches!(name.name(), "Array" | "Vec") =>
            {
                args[0].clone()
            }
            _ => return None,
        };
        // The element may still carry an unresolved structural object whose
        // field annotations are concrete — return it. A non-object element
        // (named struct handled elsewhere, or unresolved) yields None.
        match elem_ann {
            TypeAnnotation::Object(fields) => Some(fields),
            _ => None,
        }
    }

    /// Phase 3e helper: infer the element type name of a `for x in ITER`
    /// iterator expression.
    ///
    /// Currently handles:
    ///   - Identifier referring to a tracked typed array (`Vec<T>` /
    ///     `Array<T>` tracker name) — returns the inner type name.
    ///   - Array literal — peeks the first element and uses its
    ///     literal kind.
    ///
    /// Returns `None` for iterators whose element type can't be proven at
    /// compile time (HashMap iteration, custom iterables, untyped arrays).
    pub(super) fn iter_element_type_name(&self, iter: &Expr) -> Option<String> {
        match iter {
            Expr::Identifier(..) => {
                // U4-5b: recover the iterable's element type NAME STRUCTURALLY —
                // the element `ConcreteType` of the array binding, projected to a
                // tracker name. Replaces the deleted read of the binding's
                // `type_name` display string + `array_type_name_inner`
                // `strip_prefix("Vec<")` re-parse (the read half of the Rep-B
                // string round-trip). A non-array (or unresolved) iterable yields
                // `None` — the loop var stays unstamped and SURFACEs.
                self.iter_element_concrete_type(iter)
                    .as_ref()
                    .and_then(crate::compiler::patterns::binding::concrete_type_tracker_name)
            }
            Expr::Array(elems, _) => {
                let first = elems.first()?;
                match first {
                    Expr::Literal(shape_ast::ast::Literal::Int(_), _) => Some("int".to_string()),
                    Expr::Literal(shape_ast::ast::Literal::Number(_), _) => {
                        Some("number".to_string())
                    }
                    Expr::Literal(shape_ast::ast::Literal::Bool(_), _) => Some("bool".to_string()),
                    Expr::Literal(shape_ast::ast::Literal::String(_), _) => {
                        Some("string".to_string())
                    }
                    // Wave 1b FlattenReduce (2026-06-16): a nested-array literal
                    // element (`[[1,2],[3,4]]` → first = `[1,2]`) — the element
                    // type NAME is `Array<innerName>`. Recurse to resolve the
                    // inner element name; this lets `[[1,2],[3,4]].iter().flatten()`
                    // un-nest one level (the flatten arm strips the `Array<...>`
                    // wrapper to recover `int`). A nested literal whose inner
                    // element is itself unresolvable yields `None`.
                    Expr::Array(..) => {
                        let inner = self.iter_element_type_name(first)?;
                        Some(format!("Array<{inner}>"))
                    }
                    _ => None,
                }
            }
            // Wave 1b SEAM C (2026-06-15): `for x in arr.iter()` /
            // `for x in arr.iter().filter(..)`. Element-type-PRESERVING
            // iterator adapters yield the same element type as their
            // receiver, so recurse on the receiver — without this
            // propagation, `x` is untyped and `sum + x` falls out of
            // `AddInt` into trait dispatch (the same gap `for x in arr`
            // solves via the `Identifier` arm).
            //
            // Type-preserving adapters recursed here: `iter` (0 args),
            // `filter`/`take`/`skip` (the source element type is unchanged).
            // Type-CHANGING adapters (`map`/`flatMap`/`enumerate`) are NOT
            // recursed — their element type is the closure's return type /
            // an `[index, e]` pair, neither statically recoverable from the
            // receiver; the loop var is left untyped (bidirectional
            // inference recovers it where it can).
            Expr::MethodCall {
                receiver,
                method,
                args,
                ..
            } if matches!(method.as_str(), "iter" | "filter" | "take" | "skip")
                && (method != "iter" || args.is_empty()) =>
            {
                self.iter_element_type_name(receiver)
            }
            // Wave 1b FlattenReduce (2026-06-16): `flatten()` un-nests ONE
            // level, so its element type NAME is the INNER element of the
            // receiver's (nested-array) element. Recover the receiver's element
            // name (`Vec<int>` / `Array<int>`), then strip ONE more array
            // wrapper to yield `int`. Parallels the type-checker's
            // `ElementOf(ReceiverParam(0))` flatten signature. A receiver whose
            // element name is not an array (or unresolvable) yields `None` —
            // the closure param stays unstamped and SURFACEs.
            flatten @ Expr::MethodCall { method, args, .. }
                if method.as_str() == "flatten" && args.is_empty() =>
            {
                // U4-5b: `flatten()` un-nests one level — its element type is the
                // INNER element of the nested-array receiver. Derive that element
                // NAME STRUCTURALLY: `iter_element_concrete_type` already resolves
                // the flatten element `ConcreteType` (un-nesting one `Array`
                // layer), so project it to the tracker name. Replaces the deleted
                // `array_type_name_inner` `strip_prefix("Array<")` re-parse of the
                // receiver's element-name display string. A receiver whose element
                // is not an array (or unresolvable) yields `None` — the closure
                // param stays unstamped and SURFACEs.
                self.iter_element_concrete_type(flatten)
                    .as_ref()
                    .and_then(crate::compiler::patterns::binding::concrete_type_tracker_name)
            }
            _ => None,
        }
    }

    /// R3-subcase struct-array HOF (strict-flip, 2026-06-15): recover the loop
    /// variable's element [`ConcreteType`] from the iterable.
    ///
    /// Parallels [`iter_element_type_name`] but returns the full `ConcreteType`
    /// rather than a tracker name string. This is the load-bearing fix for
    /// `for u in users { ... }` / `users.iter().filter(|u| u.score > 85)` where
    /// `users: Array<User>`: the string-name path records the loop var's tracker
    /// type as `"User"`, but `concrete_type_for_expr(u)` →
    /// `identifier_concrete_type`'s `concrete_type_from_type_name` fallback only
    /// recognizes `Vec<...>` head-names — a bare struct name yields `None`, so
    /// `u.score` (and the `result.push(item)` accumulator in the monomorphized
    /// `Vec.filter` body) saw `u: unknown`. Seeding
    /// an explicit binding fact for `elem_local` with the element
    /// `ConcreteType::Struct(named)` carries the struct identity to field
    /// access / accumulator resolution.
    ///
    /// Derivation is type-proven (ADR-006 §2.7.5): the element type is the
    /// inner `T` of the iterable's already-resolved `ConcreteType::Array(T)`
    /// (recorded at the binding's let-statement / literal span). A non-array or
    /// unresolvable iterable yields `None` — the loop var stays unstamped and
    /// the existing string-name path / bidirectional inference is unchanged.
    pub(super) fn iter_element_concrete_type(
        &self,
        iter: &Expr,
    ) -> Option<shape_value::v2::ConcreteType> {
        use shape_value::v2::ConcreteType;
        // Type-preserving iterator adapters yield the receiver's element type;
        // recurse on the receiver (mirrors the `iter_element_type_name`
        // MethodCall arm). Type-CHANGING adapters (`map`/`flatMap`/`enumerate`)
        // are not recursed — their element type is not statically recoverable
        // from the receiver here.
        if let Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } = iter
        {
            if matches!(method.as_str(), "iter" | "filter" | "take" | "skip")
                && (method != "iter" || args.is_empty())
            {
                return self.iter_element_concrete_type(receiver);
            }
            // Wave 1b FlattenReduce (2026-06-16): `flatten()` removes ONE level
            // of nesting — its element type is the INNER element type of the
            // nested-array receiver (`Iterator<Array<T>>.flatten() ->
            // Iterator<T>`). Recover the receiver's element `ConcreteType`
            // (itself an `Array<T>` for a well-typed nested receiver), then
            // strip the inner `Array` to yield `T`. This parallels the
            // type-checker's `ElementOf(ReceiverParam(0))` flatten signature.
            // A receiver whose element is not an array (or unresolvable) yields
            // `None` — the closure param stays unstamped and SURFACEs. (Driven
            // off the same recursion as the type-preserving adapters; for an
            // inline nested literal the name-based `iter_element_type_name` path
            // resolves first, so this serves the let-bound `ConcreteType::Array`
            // receivers whose element ConcreteType is precisely tracked.)
            if method.as_str() == "flatten" && args.is_empty() {
                return match self.iter_element_concrete_type(receiver) {
                    Some(ConcreteType::Array(inner)) => Some(*inner),
                    _ => None,
                };
            }
        }
        match crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(self, iter)
        {
            Some(ConcreteType::Array(elem)) => Some(*elem),
            _ => None,
        }
    }
}

/// T1 sub-case (d) (strict-flip, 2026-06-20): map a scalar `TypeAnnotation`
/// (an anonymous object field's declared/inferred type) to the type-tracker
/// name the strict-typing binop emitter recognises. Returns `None` for a
/// non-scalar annotation (array / object / unknown / type-var) — the
/// destructured binder then stays untyped (surface-and-stop). Mirrors the
/// scalar arms of `struct_field_tracker_type_name`; int != number preserved.
pub(crate) fn type_annotation_scalar_tracker_name(
    ann: &shape_ast::ast::TypeAnnotation,
) -> Option<String> {
    use shape_ast::ast::TypeAnnotation;
    let name = match ann {
        TypeAnnotation::Basic(n) => match n.as_str() {
            "int" | "number" | "bool" | "string" | "decimal" | "bigint" | "i8" | "u8" | "i16"
            | "u16" | "i32" | "u32" | "u64" | "DateTime" => n.as_str(),
            _ => return None,
        },
        TypeAnnotation::Reference(path) if !path.is_qualified() => match path.name() {
            "int" | "number" | "bool" | "string" | "decimal" | "bigint" => path.name(),
            _ => return None,
        },
        _ => return None,
    };
    Some(name.to_string())
}

// ADR-006 §2.7.4 — Phase 2c rebuild (R8 C1-temporal-lowering, 2026-05-23).
//
// The original suite asserted via `shape_value::ValueWordExt::as_i64`
// against a `vm.execute(None) -> ValueWord` return. Post-strict-typing,
// `VirtualMachine::execute(None) -> Result<KindedSlot, VMError>` and
// `KindedSlot` exposes intrinsic per-kind accessors (`as_i64`, `as_f64`,
// `as_bool`, `as_str`) per §2.7.6 / Q8. The migration drops the deleted
// `ValueWordExt` trait import; every accessor body is byte-identical to
// the pre-W-series shape because the kinded API was designed to be a
// drop-in replacement for the deleted tagged-bits accessors.
#[cfg(test)]
mod tests {
    use crate::VMConfig;
    use crate::compiler::BytecodeCompiler;
    use crate::executor::VirtualMachine;
    use shape_ast::parser::parse_program;

    fn compile_and_run_i64(code: &str) -> i64 {
        let program = parse_program(code).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        let bytecode = compiler.compile(&program).unwrap();
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.execute(None)
            .unwrap()
            .as_i64()
            .expect("expected i64 top-level return")
    }

    #[test]
    fn test_range_loop_exclusive() {
        let result =
            compile_and_run_i64("fn t() { let mut s = 0; for i in 0..5 { s = s + i }; s } t()");
        assert_eq!(result, 10);
    }

    #[test]
    fn test_range_loop_inclusive() {
        let result =
            compile_and_run_i64("fn t() { let mut s = 0; for i in 0..=5 { s = s + i }; s } t()");
        assert_eq!(result, 15);
    }

    #[test]
    fn test_range_loop_empty() {
        let result =
            compile_and_run_i64("fn t() { let mut s = 0; for i in 5..0 { s = s + i }; s } t()");
        assert_eq!(result, 0);
    }

    #[test]
    fn test_range_loop_break() {
        let result = compile_and_run_i64(
            "fn t() { let mut s = 0; for i in 0..100 { if i == 5 { break }; s = s + i }; s } t()",
        );
        assert_eq!(result, 10);
    }

    #[test]
    fn test_range_loop_continue() {
        let result = compile_and_run_i64(
            "fn t() { let mut s = 0; for i in 0..10 { if i % 2 == 0 { continue }; s = s + i }; s } t()",
        );
        assert_eq!(result, 25);
    }

    #[test]
    fn test_range_loop_no_makerange() {
        // Range-counter loops compile to a direct increment-and-compare
        // pattern; they must NOT emit MakeRange/IterDone (which would
        // allocate a Range heap value).
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
        let result = compile_and_run_i64("fn t() { let r = for i in 0..5 { i * 2 }; r } t()");
        assert_eq!(result, 8);
    }

    #[test]
    fn test_range_loop_comprehension() {
        let result = compile_and_run_i64("fn t() { let a = [i * 2 for i in 0..5]; a.len() } t()");
        assert_eq!(result, 5);
    }

    #[test]
    fn test_range_loop_spread() {
        let result = compile_and_run_i64("fn t() { let a = [...0..5]; a.len() } t()");
        assert_eq!(result, 5);
    }

    #[test]
    fn test_non_range_fallback() {
        let result = compile_and_run_i64(
            "fn t() { let mut s = 0; for x in [10, 20, 30] { s = s + x }; s } t()",
        );
        assert_eq!(result, 60);
    }

    // ROOT-1 (strict-flip, 2026-06-18): the derived-read element-type flow.

    #[test]
    fn root1_for_in_let_bound_struct_array_field_add() {
        // `let ps = [R{..}]` (no annotation) must type the loop var as `R` so
        // `p.age + 10` infers under strict typing (pre-fix: `unknown + int`).
        let result = compile_and_run_i64(
            "type R { age: int } \
             fn t() { let ps = [R{age:1}, R{age:2}]; let mut s = 0; \
                      for p in ps { s = s + p.age + 10 }; s } t()",
        );
        assert_eq!(result, 23);
    }

    #[test]
    fn root1_for_in_inline_struct_array_literal_field_add() {
        // Inline struct-array literal in for-in: the element ConcreteType is
        // recovered from the literal and stamped onto the loop var's tracker.
        let result = compile_and_run_i64(
            "type R { age: int } \
             fn t() { let mut s = 0; for p in [R{age:5}] { s = s + p.age }; s } t()",
        );
        assert_eq!(result, 5);
    }

    #[test]
    fn root1_for_in_object_destructure_struct_fields() {
        // `for {x, y} in [P{..}]` must type each destructured field from the
        // struct schema so `x + y` infers (pre-fix: `unknown + unknown`).
        let result = compile_and_run_i64(
            "type P { x: int, y: int } \
             fn t() { let mut s = 0; for {x, y} in [P{x:3, y:4}] { s = s + x + y }; s } t()",
        );
        assert_eq!(result, 7);
    }

    #[test]
    fn root1_derived_index_read_struct_field_add() {
        // `let p = ps[0]` (derived read) must carry the struct identity so
        // `p.age + 10` infers.
        let result = compile_and_run_i64(
            "type R { age: int } \
             fn t() { let ps = [R{age:8}, R{age:9}]; let p = ps[0]; p.age + 10 } t()",
        );
        assert_eq!(result, 18);
    }

    // ROOT-2 (strict-flip, 2026-06-18): inline method-call return-type stamp.

    #[test]
    fn root2_inline_datetime_method_int_return_in_binop() {
        // `d.hour() + 1` inline must resolve `d.hour()` to `int` (parity with
        // the `let h = d.hour(); h + 1` form) — pre-fix: `unknown + int`.
        let result = compile_and_run_i64(
            "fn t() { let d = DateTime.parse(\"2024-01-15T08:30:00Z\"); d.hour() + 1 } t()",
        );
        assert_eq!(result, 9);
    }

    #[test]
    fn root2_inline_user_fn_inferred_return_in_binop() {
        // An unannotated user function's inferred return type flows into an
        // inline binop operand (`dbl(n) + 1`) without an intervening `let`.
        let result =
            compile_and_run_i64("fn dbl(x: int) { x * 2 } fn t() { let n = 5; dbl(n) + 1 } t()");
        assert_eq!(result, 11);
    }

    // T1 (strict-flip, 2026-06-20): type-erasure residuals (a)+(c)+(d).

    fn compile_expect_err(code: &str) -> String {
        let program = parse_program(code).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        match compiler.compile(&program) {
            Ok(_) => panic!("expected compile error, but compilation succeeded"),
            Err(e) => format!("{e:?}"),
        }
    }

    #[test]
    fn stage_f1_unannotated_empty_push_accumulator_field_read_rejected() {
        // STAGE F1 (strict-flip, 2026-06-20): re-tighten the T1 any-sink.
        // `let mut rs = []; rs = rs.push(Run{..})` then `rs[0].len` reads a
        // field off an element whose type is known ONLY from the push into an
        // UNANNOTATED empty array. Per the no-untyped-array / no-`any` rule the
        // element field type is unprovable WITHOUT an annotation, so the field
        // read is a CLEAN compile-error (NOT an `any`-typed result that would
        // wrongly accept an ill-typed program). Previously this any-sinked:
        // `rs[0].len + 1` compiled and `let x: bool = rs[0].len` was accepted.
        let err = compile_expect_err(
            "type Run { value: int, len: int } \
             fn t() { let mut rs = []; rs = rs.push(Run { value: 0, len: 4 }); \
                      rs[0].len + 1 } t()",
        );
        assert!(
            err.contains("annotate the array") || err.contains("cannot infer the type of field"),
            "expected the annotate-the-array compile error, got: {err}"
        );
    }

    #[test]
    fn stage_f1_annotated_empty_push_accumulator_field_read_works() {
        // STAGE F1: the SAME accumulator with a DECLARED `Array<Run>`
        // annotation has a proven element type — the field read resolves and
        // arithmetic works.
        let result = compile_and_run_i64(
            "type Run { value: int, len: int } \
             fn t() { let mut rs: Array<Run> = []; rs = rs.push(Run { value: 0, len: 4 }); \
                      rs[0].len + 1 } t()",
        );
        assert_eq!(result, 5);
    }

    #[test]
    fn t1a_struct_array_literal_element_field_arith() {
        // (a) the literal-element form: `rs[0].len + 1` over `[Run{..}]` — the
        // element type is PROVEN by the non-empty literal (structural Object
        // path), so it stays accepted under STAGE F1.
        let result = compile_and_run_i64(
            "type Run { value: int, len: int } \
             fn t() { let rs = [Run { value: 0, len: 4 }]; rs[0].len + 1 } t()",
        );
        assert_eq!(result, 5);
    }

    #[test]
    fn t1c_datetime_param_method_int_into_let_arith() {
        // (c) an int-returning DateTime method on a DateTime PARAMETER flows
        // into a let then arithmetic (ROOT-2 extended to a param receiver).
        let result = compile_and_run_i64(
            "fn days(d1: DateTime, d2: DateTime) { \
                let s1 = d1.unix_timestamp(); let s2 = d2.unix_timestamp(); \
                let diff = s2 - s1; diff / 86400 } \
             fn t() { let a = DateTime.parse(\"2024-06-10T00:00:00Z\"); \
                      let b = DateTime.parse(\"2024-06-15T00:00:00Z\"); days(a, b) } t()",
        );
        assert_eq!(result, 5);
    }

    #[test]
    fn t1d_object_literal_array_destructure_field_arith() {
        // (d) `for {x, y} in [{x:1, y:2}]` (ANONYMOUS object element) types each
        // destructured field so `x + y` infers (pre-fix: `unknown + unknown`).
        let result = compile_and_run_i64(
            "fn t() { let mut s = 0; for {x, y} in [{x: 1, y: 2}, {x: 3, y: 4}] \
                      { s = s + x + y }; s } t()",
        );
        assert_eq!(result, 10);
    }

    #[test]
    fn t1d_object_literal_array_binding_destructure_field_arith() {
        // (d) the identifier-bound form: `let pts = [{..}]; for {x,y} in pts`.
        let result = compile_and_run_i64(
            "fn t() { let pts = [{x: 1, y: 2}, {x: 3, y: 4}]; let mut s = 0; \
                      for {x, y} in pts { s = s + x + y }; s } t()",
        );
        assert_eq!(result, 10);
    }
}
