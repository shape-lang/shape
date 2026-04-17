//! Closure (function expression) compilation

use crate::bytecode::{Function, Instruction, OpCode, Operand};
use crate::compiler::monomorphization::type_resolution::concrete_type_for_expr;
use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};
use shape_ast::ast::{Expr, FunctionDef, Span};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::closure::EnvironmentAnalyzer;
use shape_value::v2::concrete_type::{ClosureTypeId, ConcreteType};
use std::collections::BTreeSet;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a function expression (closure)
    pub(super) fn compile_expr_closure(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[shape_ast::ast::Statement],
    ) -> Result<()> {
        let closure_name = format!("__closure_{}", self.closure_counter);
        self.closure_counter += 1;

        let proto_def = FunctionDef {
            name: closure_name.clone(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let outer_vars = self.collect_outer_scope_vars();
        let (mut captured_vars, mutated_captures) =
            EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
        captured_vars.sort();
        let param_names: BTreeSet<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();
        captured_vars.retain(|name| !param_names.contains(name));

        // Inside function bodies the MIR solver detects reference-capture errors
        // via `closure_capture_loans` facts, producing `ReferenceEscapeIntoClosure`.
        // For top-level code (no MIR), we still reject at the front-end.
        // Exception: inferred-ref locals (params passed by reference for performance)
        // are owned values and CAN be captured — the value is dereferenced at capture time.
        if self.current_function.is_none() {
            for captured in &captured_vars {
                if let Some(local_idx) = self.resolve_local(captured) {
                    let escapes_direct_borrow = self.ref_locals.contains(&local_idx)
                        && !self.inferred_ref_locals.contains(&local_idx);
                    let escapes_reference_value = self.reference_value_locals.contains(&local_idx);
                    if escapes_direct_borrow || escapes_reference_value {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "[B0003] reference '{}' cannot escape into a closure; capture a value instead",
                                captured
                            ),
                            location: None,
                        });
                    }
                }

                if let Some(scoped_name) = self.resolve_scoped_module_binding_name(captured)
                    && let Some(&binding_idx) = self.module_bindings.get(&scoped_name)
                    && self.reference_value_module_bindings.contains(&binding_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "[B0003] reference '{}' cannot escape into a closure; capture a value instead",
                            captured
                        ),
                        location: None,
                    });
                }
            }
        }

        // Build per-capture mutability flags (aligned with captured_vars order).
        // A capture is mutable if the closure itself mutates it OR if a previous
        // closure in the same scope already boxed it into a SharedCell.
        let mutable_flags: Vec<bool> = captured_vars
            .iter()
            .map(|name| mutated_captures.contains(name) || self.boxed_locals.contains(name))
            .collect();

        // Build closure parameters: only immutable captures become leading params.
        // Mutable captures are accessed via LoadClosure/StoreClosure opcodes.
        let mut closure_params = Vec::with_capacity(captured_vars.len() + params.len());
        for name in &captured_vars {
            closure_params.push(shape_ast::ast::FunctionParameter {
                pattern: shape_ast::ast::DestructurePattern::Identifier(name.clone(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            });
        }
        closure_params.extend(params.to_vec());

        let closure_def = FunctionDef {
            name: closure_name.clone(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: closure_params,
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        let user_pass_modes = self.effective_function_like_pass_modes(None, params, Some(body));
        let mut closure_pass_modes =
            vec![crate::compiler::ParamPassMode::ByValue; captured_vars.len()];
        closure_pass_modes.extend(user_pass_modes);
        let ref_params: Vec<_> = closure_pass_modes
            .iter()
            .map(|mode| mode.is_reference())
            .collect();
        let ref_mutates: Vec<_> = closure_pass_modes
            .iter()
            .map(|mode| mode.is_exclusive())
            .collect();
        self.inferred_param_pass_modes
            .insert(closure_name.clone(), closure_pass_modes);

        // Phase A: mint a ClosureTypeId keyed on the capture signature.
        //
        // Resolves each captured name to a `ConcreteType` via the monomorphizer
        // helpers; unresolved captures fall back to `Pointer(Void)` (opaque
        // 8-byte slot, conservatively treated as a heap-refcounted pointer by
        // the layout's `heap_capture_mask`). This records layout metadata in
        // `closure_registry` that Phase C consumes to extend the monomorphization
        // cache key. Emission is unchanged.
        let closure_type_id = self.mint_closure_type_id(&captured_vars);

        let func_idx = self.program.functions.len();
        self.program.functions.push(Function {
            name: closure_name.clone(),
            arity: closure_def.params.len() as u16,
            param_names: closure_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: true,
            captures_count: captured_vars.len() as u16,
            is_async: false,
            ref_params,
            ref_mutates,
            mutable_captures: mutable_flags.clone(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: None,
        });

        // Record closure function_id for MIR back-patching (ClosurePlaceholder → Function)
        self.closure_function_ids.push((closure_name.clone(), func_idx as u16));
        // Phase A: record the closure's ClosureTypeId against its function index.
        self.closure_type_ids
            .push((func_idx as u16, closure_type_id));

        // Set up mutable_closure_captures so that during body compilation,
        // variable accesses for mutable captures emit LoadClosure/StoreClosure.
        let saved_mutable_captures = std::mem::take(&mut self.mutable_closure_captures);
        for (i, name) in captured_vars.iter().enumerate() {
            if mutable_flags.get(i).copied().unwrap_or(false) {
                self.mutable_closure_captures.insert(name.clone(), i as u16);
            }
        }

        let jump_over = self.emit_jump(OpCode::Jump, 0);
        let saved_closure_ids = self.closure_function_ids.clone();
        self.compile_function(&closure_def)?;
        self.closure_function_ids = saved_closure_ids;
        self.patch_jump(jump_over);

        // Restore mutable_closure_captures
        self.mutable_closure_captures = saved_mutable_captures;

        // Capture boxing decisions
        // ────────────────────────
        // The storage planner assigns each binding a BindingStorageClass that
        // determines whether the variable needs heap indirection:
        //
        //   Direct     → LoadLocal / StoreLocal (no indirection needed)
        //   Deferred   → plan not yet resolved; fall back to legacy boxing
        //   UniqueHeap → currently: BoxLocal + Arc<RwLock<>> (SharedCell).
        //                Future: unique Box without RwLock overhead.
        //   SharedCow  → currently: BoxLocal + Arc<RwLock<>> (SharedCell).
        //                Future: COW wrapper.
        //   Reference  → DerefLoad / DerefStore (already handled above)
        //
        // We emit BoxLocal when the storage plan says the binding needs heap
        // indirection (UniqueHeap, SharedCow, Direct, or Deferred). Only
        // Reference bindings skip boxing — they are handled separately by the
        // escape check above. In the future, the planner may introduce a
        // dedicated "no-sharing" class to skip boxing for Direct bindings.
        for (i, captured) in captured_vars.iter().enumerate() {
            if matches!(
                self.binding_semantics_for_name(captured),
                Some((_, _, semantics))
                    if semantics.ownership_class == BindingOwnershipClass::Flexible
            ) {
                let storage = if mutable_flags.get(i).copied().unwrap_or(false) {
                    BindingStorageClass::SharedCow
                } else {
                    BindingStorageClass::UniqueHeap
                };
                self.promote_flexible_binding_storage_for_name(captured, storage);
            }
            if mutable_flags.get(i).copied().unwrap_or(false) {
                // Consult the storage plan to decide whether boxing is needed.
                // Currently, Direct and Deferred bindings are both boxed for
                // mutable captures because the storage plan runs before closure
                // compilation and these are the default states. Reference
                // bindings are already handled by the escape check above, so
                // the only class that could skip boxing is one where the
                // planner explicitly marks "no sharing needed" — a future
                // optimization.
                // Consult the MIR storage plan first (authoritative when available),
                // then fall back to type-tracker binding semantics.
                let mir_plan_class = self
                    .resolve_local(captured)
                    .and_then(|idx| self.mir_storage_class_for_slot(idx));
                let should_box = if let Some(plan_class) = mir_plan_class {
                    // MIR plan is authoritative: box when UniqueHeap/SharedCow,
                    // skip for Reference (handled above), box for Direct/Deferred
                    // since mutable capture needs heap indirection.
                    !matches!(plan_class, BindingStorageClass::Reference)
                } else if let Some((_, _, semantics)) = self.binding_semantics_for_name(captured) {
                    // Fallback to type-tracker semantics
                    !matches!(semantics.storage_class, BindingStorageClass::Reference)
                } else {
                    true // no plan available, use legacy behavior (always box)
                };

                if should_box {
                    // Mutable capture: emit BoxLocal/BoxModuleBinding to convert the
                    // variable to a SharedCell and push the cell onto the stack.
                    // MakeClosure will extract the Arc so the closure and enclosing
                    // scope share the same mutable cell.
                    // Track that this variable has been boxed so subsequent closures
                    // in the same scope also use the SharedCell path.
                    self.boxed_locals.insert(captured.clone());
                    self.set_binding_storage_class_for_name(
                        captured,
                        BindingStorageClass::SharedCow,
                    );
                    if let Some(local_idx) = self.resolve_local(captured) {
                        self.emit(Instruction::new(
                            OpCode::BoxLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else if let Some(scoped_name) =
                        self.resolve_scoped_module_binding_name(captured)
                    {
                        let mb_idx = self.get_or_create_module_binding(&scoped_name);
                        self.emit(Instruction::new(
                            OpCode::BoxModuleBinding,
                            Some(Operand::ModuleBinding(mb_idx)),
                        ));
                    } else if self.module_bindings.contains_key(captured) {
                        let mb_idx = self.get_or_create_module_binding(captured);
                        self.emit(Instruction::new(
                            OpCode::BoxModuleBinding,
                            Some(Operand::ModuleBinding(mb_idx)),
                        ));
                    } else {
                        // Last resort fallback — just load the value
                        let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                        self.compile_expr(&temp)?;
                    }
                } else {
                    // Storage plan says Direct — no boxing needed, just load the value.
                    let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                    self.compile_expr(&temp)?;
                }
            } else {
                let temp = Expr::Identifier(captured.clone(), Span::DUMMY);
                self.compile_expr(&temp)?;
            }
        }

        self.emit(Instruction::new(
            OpCode::MakeClosure,
            Some(Operand::Function(shape_value::FunctionId(func_idx as u16))),
        ));
        // Closures don't produce TypedObjects
        self.last_expr_schema = None;
        Ok(())
    }

    /// Read-only access to the compiler's closure registry.
    /// Populated by each closure literal during lowering (Phase A).
    pub fn closure_registry(&self) -> &shape_value::v2::closure_layout::ClosureRegistry {
        &self.closure_registry
    }

    /// `(function_id, ClosureTypeId)` pairs, one per closure literal lowered
    /// during compilation. Phase C consumes this to key the monomorphization
    /// cache by closure layout.
    pub fn closure_type_ids(&self) -> &[(u16, ClosureTypeId)] {
        &self.closure_type_ids
    }

    /// Mint a `ClosureTypeId` for a closure literal by resolving each capture
    /// name to a `ConcreteType` and interning the resulting signature in
    /// `closure_registry` (Phase A).
    ///
    /// Unresolved captures fall back to `Pointer(Void)` — an opaque 8-byte
    /// slot that the layout treats as heap-refcounted. This keeps semantics
    /// conservative (no missed Drop glue) while Phase B/C/D grow the
    /// resolution coverage.
    pub(crate) fn mint_closure_type_id(
        &mut self,
        captured_vars: &[String],
    ) -> ClosureTypeId {
        let capture_types: Vec<ConcreteType> = captured_vars
            .iter()
            .map(|name| {
                let ident = Expr::Identifier(name.clone(), Span::DUMMY);
                concrete_type_for_expr(self, &ident)
                    .unwrap_or_else(|| ConcreteType::Pointer(Box::new(ConcreteType::Void)))
            })
            .collect();
        self.closure_registry.intern(capture_types)
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use crate::type_tracking::BindingStorageClass;
    use shape_ast::ast::{Expr, Item, Span, Statement, VarKind, VariableDecl};
    use shape_ast::parser::parse_program;

    #[test]
    fn test_mutable_closure_capture_marks_binding_as_shared_storage() {
        let program =
            parse_program("let inc = || { counter = counter + 1; counter }").expect("parse failed");
        let var_decl = match &program.items[0] {
            Item::VariableDecl(var_decl, _) => var_decl,
            Item::Statement(Statement::VariableDecl(var_decl, _), _) => var_decl,
            _ => panic!("expected variable declaration"),
        };
        let Some(Expr::FunctionExpr { params, body, .. }) = var_decl.value.as_ref() else {
            panic!("expected closure initializer");
        };

        let mut compiler = BytecodeCompiler::new();
        let counter_idx = compiler.get_or_create_module_binding("counter");
        let counter_decl = VariableDecl {
            kind: VarKind::Let,
            is_mut: false,
            pattern: shape_ast::ast::DestructurePattern::Identifier(
                "counter".to_string(),
                Span::DUMMY,
            ),
            type_annotation: None,
            value: None,
            ownership: Default::default(),
        };
        compiler.apply_binding_semantics_to_pattern_bindings(
            &counter_decl.pattern,
            false,
            BytecodeCompiler::binding_semantics_for_var_decl(&counter_decl),
        );

        compiler
            .compile_expr_closure(params, body)
            .expect("closure should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(counter_idx)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::SharedCow)
        );
    }

    #[test]
    fn test_flexible_closure_capture_marks_binding_as_unique_heap_storage() {
        let program = parse_program("let read = || counter").expect("parse failed");
        let var_decl = match &program.items[0] {
            Item::VariableDecl(var_decl, _) => var_decl,
            Item::Statement(Statement::VariableDecl(var_decl, _), _) => var_decl,
            _ => panic!("expected variable declaration"),
        };
        let Some(Expr::FunctionExpr { params, body, .. }) = var_decl.value.as_ref() else {
            panic!("expected closure initializer");
        };

        let mut compiler = BytecodeCompiler::new();
        let counter_idx = compiler.get_or_create_module_binding("counter");
        let counter_decl = VariableDecl {
            kind: VarKind::Var,
            is_mut: false,
            pattern: shape_ast::ast::DestructurePattern::Identifier(
                "counter".to_string(),
                Span::DUMMY,
            ),
            type_annotation: None,
            value: None,
            ownership: Default::default(),
        };
        compiler.apply_binding_semantics_to_pattern_bindings(
            &counter_decl.pattern,
            false,
            BytecodeCompiler::binding_semantics_for_var_decl(&counter_decl),
        );

        compiler
            .compile_expr_closure(params, body)
            .expect("closure should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(counter_idx)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    // ---- Phase A: ClosureTypeId minting & ClosureRegistry population ----

    use shape_value::v2::concrete_type::ClosureTypeId;

    fn compile_closure_literal(source: &str, compiler: &mut BytecodeCompiler) {
        let program = parse_program(source).expect("parse failed");
        let var_decl = match &program.items[0] {
            Item::VariableDecl(var_decl, _) => var_decl,
            Item::Statement(Statement::VariableDecl(var_decl, _), _) => var_decl,
            _ => panic!("expected variable declaration"),
        };
        let Some(Expr::FunctionExpr { params, body, .. }) = var_decl.value.as_ref() else {
            panic!("expected closure initializer");
        };
        compiler
            .compile_expr_closure(params, body)
            .expect("closure should compile");
    }

    #[test]
    fn test_no_capture_closure_mints_closure_type_id_zero() {
        let mut compiler = BytecodeCompiler::new();
        compile_closure_literal("let f = || 42", &mut compiler);

        // Exactly one closure was lowered.
        assert_eq!(compiler.closure_type_ids().len(), 1);
        let (func_id, type_id) = compiler.closure_type_ids()[0];
        assert_eq!(type_id, ClosureTypeId(0));
        // func_id refers to a real entry in the program's function table.
        assert!(
            (func_id as usize) < compiler.program.functions.len(),
            "func_id must index the program's function table"
        );

        let layout = compiler.closure_registry().get(type_id).expect("layout");
        assert_eq!(layout.capture_count(), 0);
        assert_eq!(layout.total_heap_size(), 16);
        assert_eq!(layout.total_stack_size(), 8);
    }

    #[test]
    fn test_two_no_capture_closures_share_type_id() {
        let mut compiler = BytecodeCompiler::new();
        compile_closure_literal("let f = || 1", &mut compiler);
        compile_closure_literal("let g = || 2", &mut compiler);

        let ids = compiler.closure_type_ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].1, ids[1].1, "identical capture signatures share id");
        // Distinct function indices — the bodies are separate functions even
        // though the capture layout is shared.
        assert_ne!(ids[0].0, ids[1].0);

        // Registry contains exactly one layout (the shared signature).
        assert_eq!(compiler.closure_registry().len(), 1);
    }

    #[test]
    fn test_closure_counter_advances_independently_of_type_id() {
        let mut compiler = BytecodeCompiler::new();
        // Two closure literals, no captures → shared ClosureTypeId(0),
        // but closure_counter should still be 2 (two function names minted).
        compile_closure_literal("let f = || 1", &mut compiler);
        compile_closure_literal("let g = || 2", &mut compiler);
        assert_eq!(compiler.closure_counter, 2);
        assert_eq!(compiler.closure_registry().len(), 1);
    }

    #[test]
    fn test_captures_with_unresolved_types_fallback_to_opaque_pointer() {
        // When a capture's concrete type is unresolved, the registry records
        // it as `Pointer(Void)` — an opaque 8-byte heap-refcounted slot. This
        // conservative fallback ensures Drop glue is safe.
        use shape_value::v2::struct_layout::FieldKind;

        let mut compiler = BytecodeCompiler::new();
        // Create a module binding with no type information.
        compiler.get_or_create_module_binding("x");
        compile_closure_literal("let f = || x + 1", &mut compiler);

        let ids = compiler.closure_type_ids();
        assert_eq!(ids.len(), 1);
        let layout = compiler.closure_registry().get(ids[0].1).expect("layout");
        assert_eq!(layout.capture_count(), 1);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.heap_capture_mask, 0b1, "opaque pointer is heap-refcounted");
    }

    #[test]
    fn test_three_closures_record_three_entries_in_type_id_map() {
        let mut compiler = BytecodeCompiler::new();
        // Three module bindings, one captured per closure.
        compiler.get_or_create_module_binding("a");
        compiler.get_or_create_module_binding("b");
        compiler.get_or_create_module_binding("c");

        compile_closure_literal("let f = || a", &mut compiler);
        compile_closure_literal("let g = || b", &mut compiler);
        compile_closure_literal("let h = || c", &mut compiler);

        let ids = compiler.closure_type_ids();
        assert_eq!(ids.len(), 3, "three closure literals → three map entries");

        // All three have the same capture shape (one opaque-pointer module
        // binding), so they should share a ClosureTypeId.
        assert_eq!(ids[0].1, ids[1].1);
        assert_eq!(ids[1].1, ids[2].1);
        assert_eq!(compiler.closure_registry().len(), 1);

        // But each has a distinct function id.
        assert_ne!(ids[0].0, ids[1].0);
        assert_ne!(ids[1].0, ids[2].0);
    }

    #[test]
    fn test_closure_type_ids_reference_distinct_function_indices() {
        // The (function_id, ClosureTypeId) pairs must point at real,
        // distinct entries in the compiled program's function table.
        let mut compiler = BytecodeCompiler::new();
        compile_closure_literal("let f = || 1", &mut compiler);
        compile_closure_literal("let g = || 2", &mut compiler);
        compile_closure_literal("let h = || 3", &mut compiler);

        let ids = compiler.closure_type_ids();
        assert_eq!(ids.len(), 3);
        let funcs = &compiler.program.functions;
        for (fid, _) in ids {
            let idx = *fid as usize;
            assert!(idx < funcs.len(), "function id {idx} out of range");
            assert!(funcs[idx].is_closure);
        }
        // Function ids are distinct.
        let mut seen = std::collections::HashSet::new();
        for (fid, _) in ids {
            assert!(seen.insert(*fid), "duplicate function id {fid}");
        }
    }
}
