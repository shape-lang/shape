//! Function and closure compilation

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_ast::ast::{FunctionDef, Item, Span, Statement};
use shape_ast::error::{ErrorNote, Result, ShapeError};
use std::collections::HashMap;

use super::{BytecodeCompiler, ParamPassMode};

impl BytecodeCompiler {
    pub(super) fn explicit_param_pass_modes(
        params: &[shape_ast::ast::FunctionParameter],
    ) -> Vec<ParamPassMode> {
        params
            .iter()
            .map(|param| {
                if param.is_mut_reference {
                    ParamPassMode::ByRefExclusive
                } else if param.is_reference {
                    ParamPassMode::ByRefShared
                } else {
                    ParamPassMode::ByValue
                }
            })
            .collect()
    }

    pub(super) fn effective_function_like_pass_modes(
        &self,
        name: Option<&str>,
        params: &[shape_ast::ast::FunctionParameter],
        body: Option<&[shape_ast::ast::Statement]>,
    ) -> Vec<ParamPassMode> {
        if let Some(name) = name {
            if let Some(inferred_modes) = self.inferred_param_pass_modes.get(name) {
                let fallback_modes = Self::explicit_param_pass_modes(params);
                return fallback_modes
                    .into_iter()
                    .enumerate()
                    .map(|(idx, fallback)| inferred_modes.get(idx).copied().unwrap_or(fallback))
                    .collect();
            }
            if let Some(func_idx) = self.find_function(name)
                && let Some(func) = self.program.functions.get(func_idx)
            {
                let fallback_modes = Self::explicit_param_pass_modes(params);
                let registered_modes =
                    Self::pass_modes_from_ref_flags(&func.ref_params, &func.ref_mutates);
                return fallback_modes
                    .into_iter()
                    .enumerate()
                    .map(|(idx, fallback)| registered_modes.get(idx).copied().unwrap_or(fallback))
                    .collect();
            }
        }

        let mut modes = Self::explicit_param_pass_modes(params);
        let Some(body) = body else {
            return modes;
        };

        let caller_ref_params: Vec<_> = modes.iter().map(|mode| mode.is_reference()).collect();
        if !caller_ref_params.iter().any(|is_ref| *is_ref) {
            return modes;
        }

        let mut known_callable_modes: HashMap<String, Vec<ParamPassMode>> = self
            .program
            .functions
            .iter()
            .map(|func| {
                (
                    func.name.clone(),
                    Self::pass_modes_from_ref_flags(&func.ref_params, &func.ref_mutates),
                )
            })
            .collect();
        for scope in &self.locals {
            for (binding_name, local_idx) in scope {
                if let Some(pass_modes) = self.local_callable_pass_modes.get(local_idx) {
                    known_callable_modes.insert(binding_name.clone(), pass_modes.clone());
                }
            }
        }
        for (binding_name, binding_idx) in &self.module_bindings {
            if let Some(pass_modes) = self.module_binding_callable_pass_modes.get(binding_idx) {
                known_callable_modes.insert(binding_name.clone(), pass_modes.clone());
            }
        }

        let callee_ref_params: HashMap<String, Vec<bool>> = known_callable_modes
            .iter()
            .map(|(callee_name, pass_modes)| {
                (
                    callee_name.clone(),
                    pass_modes.iter().map(|mode| mode.is_reference()).collect(),
                )
            })
            .collect();
        let caller_name = name.unwrap_or("__function_expr__");
        let mut direct_mutates = vec![false; params.len()];
        let mut edges = Vec::new();
        let mut param_index_by_name = HashMap::new();
        for (idx, param) in params.iter().enumerate() {
            for param_name in param.get_identifiers() {
                param_index_by_name.insert(param_name, idx);
            }
        }
        for stmt in body {
            Self::analyze_statement_for_ref_mutation(
                stmt,
                caller_name,
                &param_index_by_name,
                &caller_ref_params,
                &callee_ref_params,
                &mut direct_mutates,
                &mut edges,
            );
        }
        for (_, caller_idx, callee_name, callee_idx) in edges {
            if known_callable_modes
                .get(&callee_name)
                .and_then(|modes| modes.get(callee_idx))
                .is_some_and(|mode| mode.is_exclusive())
                && let Some(flag) = direct_mutates.get_mut(caller_idx)
            {
                *flag = true;
            }
        }
        for (idx, direct_mutates) in direct_mutates.into_iter().enumerate() {
            if direct_mutates && modes.get(idx).is_some_and(|mode| mode.is_reference()) {
                modes[idx] = ParamPassMode::ByRefExclusive;
            }
        }

        modes
    }

    pub(super) fn compile_function(&mut self, func_def: &FunctionDef) -> Result<()> {
        // Validate annotation target kinds before compilation
        self.validate_annotation_targets(func_def)?;

        // In non-comptime mode (i.e., the outer/runtime compiler), `comptime fn`
        // helpers are only needed as AST in `function_defs` (for
        // collect_comptime_helpers). Skip compiling their bodies into the runtime
        // bytecode — doing so wastes space and leaks comptime-only code into the
        // runtime program where it can collide with runtime names.
        // In comptime mode (inside the mini-VM compiler), we DO compile them
        // because the mini-VM actually needs to execute them.
        if func_def.is_comptime && !self.comptime_mode {
            return Ok(());
        }

        let mut effective_def = func_def.clone();
        let effective_pass_modes = self.effective_function_like_pass_modes(
            Some(&effective_def.name),
            &effective_def.params,
            Some(&effective_def.body),
        );
        for (idx, param) in effective_def.params.iter_mut().enumerate() {
            let effective_mode = effective_pass_modes
                .get(idx)
                .copied()
                .unwrap_or(ParamPassMode::ByValue);
            if param.type_annotation.is_none()
                && param.simple_name().is_some()
                && effective_mode.is_reference()
            {
                param.is_reference = true;
            }
            if effective_mode.is_exclusive() {
                param.is_mut_reference = true;
            }
        }
        let has_const_template_params = effective_def.params.iter().any(|p| p.is_const);
        let has_specialization_bindings = self
            .specialization_const_bindings
            .contains_key(&effective_def.name);

        // Execute comptime annotation handlers before compilation.
        // These run at compile time and can inspect/modify the target.
        // Template bases (functions with const parameters) skip comptime handler
        // execution until a concrete call-site specialization binds those consts.
        if !(has_const_template_params && !has_specialization_bindings)
            && self.execute_comptime_handlers(&mut effective_def)?
        {
            // Track removed functions so call sites produce a clear error
            // instead of jumping to an invalid entry point (stack overflow).
            self.removed_functions.insert(effective_def.name.clone());
            self.function_defs.remove(&effective_def.name);
            return Ok(());
        }

        // Keep the registry synchronized with the final mutated function shape.
        // This is used by expansion/inspection tooling.
        self.function_defs
            .insert(effective_def.name.clone(), effective_def.clone());

        // Lower every compiled function to MIR and run the shared borrow analysis.
        // MIR borrow analysis is the primary authority for functions with clean
        // lowering (no fallbacks). When authoritative, the lexical borrow checker
        // calls in helpers.rs are skipped. For functions where MIR lowering had
        // fallbacks, the lexical checker remains the active fallback.
        let mir_lowering = crate::mir::lowering::lower_function_detailed(
            &effective_def.name,
            &effective_def.params,
            &effective_def.body,
            effective_def.name_span,
        );
        let callee_summaries =
            self.build_callee_summaries(Some(&effective_def.name), &mir_lowering.all_local_names);
        let mut mir_analysis = crate::mir::solver::analyze(&mir_lowering.mir, &callee_summaries);
        mir_analysis.mutability_errors =
            crate::mir::lowering::compute_mutability_errors(&mir_lowering);
        crate::mir::repair::attach_repairs(&mut mir_analysis, &mir_lowering.mir);
        // MIR is the sole authority for borrow checking. Span-granular error
        // filtering: when lowering had fallbacks, only suppress errors whose span
        // overlaps a fallback span. Errors in cleanly-lowered regions pass through.
        let first_mutability_error = if mir_lowering.fallback_spans.is_empty() {
            mir_analysis.mutability_errors.first().cloned()
        } else {
            mir_analysis
                .mutability_errors
                .iter()
                .find(|e| !Self::span_overlaps_any(&e.span, &mir_lowering.fallback_spans))
                .cloned()
        };
        let first_mir_error = if mir_lowering.fallback_spans.is_empty() {
            mir_analysis.errors.first().cloned()
        } else {
            mir_analysis
                .errors
                .iter()
                .find(|e| !Self::span_overlaps_any(&e.span, &mir_lowering.fallback_spans))
                .cloned()
        };
        if let Some(summary) = mir_analysis.return_reference_summary.clone() {
            self.function_return_reference_summaries
                .insert(effective_def.name.clone(), summary.into());
        } else {
            self.function_return_reference_summaries
                .remove(&effective_def.name);
        }
        // Run storage planning pass: decide Direct / UniqueHeap / SharedCow / Reference
        // for each MIR slot based on closure captures, aliasing, and mutation.
        {
            let (closure_captures, mutable_captures) =
                crate::mir::storage_planning::collect_closure_captures(&mir_lowering.mir);

            // Gather binding semantics from the type tracker for each slot.
            let mut binding_semantics = std::collections::HashMap::new();
            for slot_idx in 0..mir_lowering.mir.num_locals {
                if let Some(sem) = self.type_tracker.get_local_binding_semantics(slot_idx) {
                    binding_semantics.insert(slot_idx, *sem);
                }
            }

            let planner_input = crate::mir::storage_planning::StoragePlannerInput {
                mir: &mir_lowering.mir,
                analysis: &mir_analysis,
                binding_semantics: &binding_semantics,
                closure_captures: &closure_captures,
                mutable_captures: &mutable_captures,
                had_fallbacks: mir_lowering.had_fallbacks,
            };

            let storage_plan = crate::mir::storage_planning::plan_storage(&planner_input);

            self.mir_storage_plans
                .insert(effective_def.name.clone(), storage_plan);
        }

        // Run field-level definite-initialization and liveness analysis.
        // This is Phase 2 of the two-phase TypedObject hoisting design:
        // the AST pre-pass (Phase 1, in compiler_impl_part4.rs) collects
        // property assignments for initial schema sizing, and this MIR
        // analysis validates initialization flow and detects dead fields.
        let field_cfg = crate::mir::cfg::ControlFlowGraph::build(&mir_lowering.mir);
        let mut field_analysis = crate::mir::field_analysis::analyze_fields(
            &crate::mir::field_analysis::FieldAnalysisInput {
                mir: &mir_lowering.mir,
                cfg: &field_cfg,
            },
        );

        // Populate hoisting_recommendations from field names (MIR-authoritative).
        // Dead fields are pruned: if a field is written but never read, it's
        // excluded from the recommendation (schema compaction).
        for (slot_id, field_indices) in &field_analysis.hoisted_fields {
            let recommendations: Vec<(crate::mir::FieldIdx, String)> = field_indices
                .iter()
                .filter(|idx| !field_analysis.dead_fields.contains(&(*slot_id, **idx)))
                .filter_map(|idx| {
                    mir_lowering
                        .field_names
                        .get(idx)
                        .map(|name| (*idx, name.clone()))
                })
                .collect();
            if !recommendations.is_empty() {
                field_analysis
                    .hoisting_recommendations
                    .insert(*slot_id, recommendations);
            }
        }

        // Merge MIR-derived hoisted fields into the AST pre-pass hoisted_fields map.
        // MIR field analysis is authoritative per-function: it refines the AST
        // pre-pass (which over-hoists conservatively). Dead fields detected by MIR
        // are excluded from the hoisted list.
        for (slot_id, field_indices) in &field_analysis.hoisted_fields {
            if let Some(binding) = mir_lowering
                .binding_infos
                .iter()
                .find(|b| b.slot == *slot_id)
            {
                let var_name = &binding.name;
                let field_names: Vec<String> = field_indices
                    .iter()
                    // Prune dead fields from hoisted list (schema compaction)
                    .filter(|idx| !field_analysis.dead_fields.contains(&(*slot_id, **idx)))
                    .filter_map(|idx| mir_lowering.field_names.get(idx))
                    .cloned()
                    .collect();
                if !field_names.is_empty() {
                    // For function scope, use MIR list as authoritative (replace, don't merge)
                    self.hoisted_fields.insert(var_name.clone(), field_names);
                }
            }
        }

        self.mir_field_analyses
            .insert(effective_def.name.clone(), field_analysis);

        // Build span→point mapping for ownership decision lookups.
        // This lets the bytecode compiler translate AST spans (which it knows
        // at expression compile time) into MIR Points (which the ownership
        // decision API expects).
        {
            let mut span_to_point = HashMap::new();
            for block in mir_lowering.mir.iter_blocks() {
                for stmt in &block.statements {
                    span_to_point.entry(stmt.span).or_insert(stmt.point);
                }
            }
            self.mir_span_to_point
                .insert(effective_def.name.clone(), span_to_point);
        }

        // Extract and store borrow summary for interprocedural alias checking.
        let borrow_summary = crate::mir::solver::extract_borrow_summary(
            &mir_lowering.mir,
            mir_analysis.return_reference_summary.clone(),
        );
        if !borrow_summary.conflict_pairs.is_empty() || borrow_summary.return_summary.is_some() {
            self.function_borrow_summaries
                .insert(effective_def.name.clone(), borrow_summary);
        } else {
            self.function_borrow_summaries.remove(&effective_def.name);
        }

        // Interprocedural alias checking: scan call sites in this function's MIR
        // for argument aliasing conflicts using stored callee summaries.
        let alias_errors =
            self.check_call_site_aliasing(&mir_lowering.mir, &mir_lowering.fallback_spans);
        let first_alias_error = alias_errors.first().cloned();
        mir_analysis.errors.extend(alias_errors);

        self.mir_functions
            .insert(effective_def.name.clone(), mir_lowering.mir);
        self.mir_borrow_analyses
            .insert(effective_def.name.clone(), mir_analysis);
        if let Some(error) = first_mutability_error.as_ref() {
            return Err(self.mir_mutability_error(error));
        }
        if let Some(error) = first_mir_error.as_ref() {
            return Err(self.mir_borrow_error(error));
        }
        if let Some(error) = first_alias_error.as_ref() {
            return Err(self.mir_borrow_error(error));
        }

        // Track whether __original__ alias is active so we can clean it up.
        let has_original_alias = self.function_aliases.contains_key("__original__");

        // Enable __* builtin access only for stdlib-origin functions
        // (or preserve if an enclosing stdlib compilation context already enabled it).
        let saved_allow_internal = self.allow_internal_builtins;
        self.allow_internal_builtins = saved_allow_internal
            || effective_def
                .declaring_module_path
                .as_deref()
                .is_some_and(|module_path| module_path.starts_with("std::"));

        // Check for annotation-based wrapping BEFORE compiling
        let annotations = self.find_compiled_annotations(&effective_def);
        if annotations.len() == 1 {
            // SAFETY invariant: the `len() == 1` guard above guarantees
            // `.next()` yields `Some`. This is a compile-time structural
            // invariant, not a runtime condition, so `expect` is appropriate.
            self.compile_wrapped_function(
                &effective_def,
                annotations.into_iter().next().expect("checked len == 1"),
            )?;
        } else if annotations.len() > 1 {
            self.compile_chained_annotations(&effective_def, annotations)?;
        } else {
            self.compile_function_body(&effective_def)?;
        }

        // Restore previous flag (safe for nested compilation).
        self.allow_internal_builtins = saved_allow_internal;

        // Cache MIR data in the Function struct for JIT v2 (MirToIR compilation).
        // Clone from the compiler's caches so both the compiler (for diagnostics/LSP)
        // and the JIT (via Function.mir_data) have access.
        if let Some(func_idx) = self.find_function(&effective_def.name) {
            let mir_opt = self.mir_functions.get(&effective_def.name).cloned();
            let borrow_opt = self.mir_borrow_analyses.get(&effective_def.name).cloned();
            let storage_opt = self.mir_storage_plans.get(&effective_def.name).cloned();
            if let (Some(mut mir), Some(borrow_analysis), Some(storage_plan)) =
                (mir_opt, borrow_opt, storage_opt)
            {
                // Back-patch MIR: replace ClosurePlaceholder and ClosureCapture
                // with resolved closure function names/IDs from bytecode compilation.
                if !self.closure_function_ids.is_empty() {
                    let mut closure_idx = 0;
                    let closure_ids = self.closure_function_ids.clone();
                    for block in &mut mir.blocks {
                        for stmt in &mut block.statements {
                            // Check for ClosurePlaceholder in Assign
                            let is_placeholder = matches!(
                                &stmt.kind,
                                crate::mir::types::StatementKind::Assign(
                                    _,
                                    crate::mir::types::Rvalue::Use(
                                        crate::mir::types::Operand::Constant(
                                            crate::mir::types::MirConstant::ClosurePlaceholder,
                                        ),
                                    ),
                                )
                            );
                            if is_placeholder {
                                if closure_idx < closure_ids.len() {
                                    let (ref name, _) = closure_ids[closure_idx];
                                    stmt.kind = crate::mir::types::StatementKind::Assign(
                                        crate::mir::types::Place::Local(
                                            // Extract the place from the current statement
                                            match &stmt.kind {
                                                crate::mir::types::StatementKind::Assign(p, _) => p.root_local(),
                                                _ => unreachable!(),
                                            }
                                        ),
                                        crate::mir::types::Rvalue::Use(
                                            crate::mir::types::Operand::Constant(
                                                crate::mir::types::MirConstant::Function(name.clone()),
                                            ),
                                        ),
                                    );
                                    closure_idx += 1;
                                }
                                continue;
                            }
                            // Patch ClosureCapture with function_id
                            if let crate::mir::types::StatementKind::ClosureCapture {
                                function_id,
                                ..
                            } = &mut stmt.kind {
                                if closure_idx < closure_ids.len() {
                                    let (_, idx) = closure_ids[closure_idx];
                                    *function_id = Some(idx);
                                }
                            }
                        }
                    }
                }
                // Clear closure_function_ids for the next function
                self.closure_function_ids.clear();

                self.program.functions[func_idx].mir_data =
                    Some(std::sync::Arc::new(crate::bytecode::MirFunctionData {
                        mir,
                        storage_plan,
                        borrow_analysis,
                    }));
            }
        }

        // Clean up __original__ alias after the replacement body is compiled.
        if has_original_alias {
            self.function_aliases.remove("__original__");
        }

        // Runtime lifecycle hooks (`on_define`, `metadata`) are invoked at
        // definition time by emitting top-level calls after function compilation.
        self.emit_annotation_lifecycle_calls(&effective_def)
    }

    /// Return the (message_body, hint) pair for a MIR borrow error kind.
    ///
    /// The message body does NOT include the `[B00XX]` prefix — that is
    /// prepended by `mir_borrow_error` using `BorrowErrorKind::code()` so
    /// the code mapping is defined in exactly one place.
    fn mir_borrow_error_message(
        &self,
        kind: crate::mir::analysis::BorrowErrorKind,
    ) -> (&'static str, &'static str) {
        match kind {
            crate::mir::analysis::BorrowErrorKind::ConflictSharedExclusive => (
                "cannot mutably borrow this value while shared borrows are active",
                "move the mutable borrow later, or end the shared borrow sooner",
            ),
            crate::mir::analysis::BorrowErrorKind::ConflictExclusiveExclusive => (
                "cannot mutably borrow this value because it is already borrowed",
                "end the previous mutable borrow before creating another one",
            ),
            crate::mir::analysis::BorrowErrorKind::ReadWhileExclusivelyBorrowed => (
                "cannot read this value while it is mutably borrowed",
                "read through the existing reference, or move the read after the borrow ends",
            ),
            crate::mir::analysis::BorrowErrorKind::WriteWhileBorrowed => (
                "cannot write to this value while it is borrowed",
                "move this write after the borrow ends",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceEscape => (
                "cannot return or store a reference that outlives its owner",
                "return an owned value instead of a reference",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceStoredInArray => (
                "cannot store a reference in an array — references are scoped borrows that cannot escape into collections. Use owned values instead",
                "store owned values in the array instead of references",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceStoredInObject => (
                "cannot store a reference in an object or struct literal — references are scoped borrows that cannot escape into aggregate values. Use owned values instead",
                "store owned values in the object or struct instead of references",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceStoredInEnum => (
                "cannot store a reference in an enum payload — references are scoped borrows that cannot escape into aggregate values. Use owned values instead",
                "store owned values in the enum payload instead of references",
            ),
            crate::mir::analysis::BorrowErrorKind::ReferenceEscapeIntoClosure => (
                "reference cannot escape into a closure",
                "capture an owned value instead of a reference",
            ),
            crate::mir::analysis::BorrowErrorKind::UseAfterMove => (
                "cannot use this value after it was moved",
                "clone the value before moving it, or stop using the original after the move",
            ),
            crate::mir::analysis::BorrowErrorKind::ExclusiveRefAcrossTaskBoundary => (
                "cannot move an exclusive reference across a task boundary",
                "keep the mutable reference within the current task or pass an owned value instead",
            ),
            crate::mir::analysis::BorrowErrorKind::SharedRefAcrossDetachedTask => (
                "cannot send a shared reference across a detached task boundary",
                "clone the value before sending it to a detached task, or use a structured task instead",
            ),
            crate::mir::analysis::BorrowErrorKind::InconsistentReferenceReturn => (
                "reference-returning functions must return a reference on every path from the same borrowed origin and borrow kind",
                "return a reference from the same borrowed origin on every path, or return owned values instead",
            ),
            crate::mir::analysis::BorrowErrorKind::CallSiteAliasConflict => (
                "cannot pass the same variable to multiple parameters that require non-aliased access",
                "use separate variables or clone one of the arguments",
            ),
            crate::mir::analysis::BorrowErrorKind::NonSendableAcrossTaskBoundary => (
                "cannot send a closure with mutable captures across a detached task boundary",
                "clone the captured values before spawning the task",
            ),
        }
    }

    fn mir_borrow_origin_note(&self, kind: crate::mir::analysis::BorrowErrorKind) -> &'static str {
        match kind {
            crate::mir::analysis::BorrowErrorKind::ConflictSharedExclusive
            | crate::mir::analysis::BorrowErrorKind::ConflictExclusiveExclusive
            | crate::mir::analysis::BorrowErrorKind::ReadWhileExclusivelyBorrowed
            | crate::mir::analysis::BorrowErrorKind::WriteWhileBorrowed => {
                "conflicting borrow originates here"
            }
            crate::mir::analysis::BorrowErrorKind::ReferenceEscape
            | crate::mir::analysis::BorrowErrorKind::ReferenceStoredInArray
            | crate::mir::analysis::BorrowErrorKind::ReferenceStoredInObject
            | crate::mir::analysis::BorrowErrorKind::ReferenceStoredInEnum
            | crate::mir::analysis::BorrowErrorKind::ReferenceEscapeIntoClosure => {
                "reference originates here"
            }
            crate::mir::analysis::BorrowErrorKind::UseAfterMove => "value was moved here",
            crate::mir::analysis::BorrowErrorKind::ExclusiveRefAcrossTaskBoundary
            | crate::mir::analysis::BorrowErrorKind::SharedRefAcrossDetachedTask => {
                "reference originates here"
            }
            crate::mir::analysis::BorrowErrorKind::InconsistentReferenceReturn => {
                "borrowed origin on another return path originates here"
            }
            crate::mir::analysis::BorrowErrorKind::CallSiteAliasConflict => {
                "conflicting argument originates here"
            }
            crate::mir::analysis::BorrowErrorKind::NonSendableAcrossTaskBoundary => {
                "closure with mutable captures originates here"
            }
        }
    }

    fn mir_borrow_error(&self, error: &crate::mir::analysis::BorrowError) -> ShapeError {
        let (body, default_hint) = self.mir_borrow_error_message(error.kind.clone());
        let code = error.kind.code();
        let message = format!("[{}] {}", code, body);
        let mut location = self.span_to_source_location(error.span);
        location.hints.push(default_hint.to_string());
        if let Some(repair) = error.repairs.first() {
            location.hints.push(repair.description.clone());
        }
        location.notes.push(ErrorNote {
            message: self.mir_borrow_origin_note(error.kind.clone()).to_string(),
            location: Some(self.span_to_source_location(error.loan_span)),
        });
        if let Some(last_use_span) = error.last_use_span {
            location.notes.push(ErrorNote {
                message: "borrow is still needed here".to_string(),
                location: Some(self.span_to_source_location(last_use_span)),
            });
        }
        ShapeError::SemanticError {
            message,
            location: Some(location),
        }
    }

    fn mir_mutability_error(&self, error: &crate::mir::analysis::MutabilityError) -> ShapeError {
        let mut location = self.span_to_source_location(error.span);
        if error.is_const {
            location
                .hints
                .push("const bindings cannot be reassigned".to_string());
        } else if error.is_explicit_let {
            location
                .hints
                .push("declare it as `let mut` if mutation is intended".to_string());
        } else {
            location
                .hints
                .push("this binding is immutable in this context".to_string());
        }
        location.notes.push(ErrorNote {
            message: "binding declared here".to_string(),
            location: Some(self.span_to_source_location(error.declaration_span)),
        });
        ShapeError::SemanticError {
            message: if error.is_const {
                format!("cannot assign to const binding '{}'", error.variable_name)
            } else {
                format!(
                    "cannot assign to immutable binding '{}'",
                    error.variable_name
                )
            },
            location: Some(location),
        }
    }

    /// Build callee return-reference summaries for interprocedural composition.
    ///
    /// Only includes names that are confirmed direct global function calls.
    /// Excludes names that shadow globals: locals, captures, module bindings,
    /// and the function being compiled (prevents stale self-summary).
    ///
    /// This mirrors the bytecode compiler's call resolution order
    /// (function_calls.rs:514-516): locals → captures → module bindings → globals.
    pub(crate) fn build_callee_summaries(
        &self,
        exclude_name: Option<&str>,
        mir_local_names: &std::collections::HashSet<String>,
    ) -> crate::mir::solver::CalleeSummaries {
        self.function_borrow_summaries
            .iter()
            .filter_map(|(name, summary)| {
                if exclude_name == Some(name.as_str()) {
                    return None;
                }
                // Mirror call resolution: locals → captures → module bindings → globals
                if mir_local_names.contains(name.as_str()) {
                    return None;
                }
                if self.mutable_closure_captures.contains_key(name.as_str()) {
                    return None;
                }
                if self.resolve_scoped_module_binding_name(name).is_some() {
                    return None;
                }
                summary
                    .return_summary
                    .as_ref()
                    .map(|s| (name.clone(), s.clone()))
            })
            .collect()
    }

    /// Check call sites in a function's MIR for interprocedural alias conflicts.
    /// Returns errors for each call where the same variable is passed to multiple
    /// parameters that the callee's borrow summary says must not alias.
    fn check_call_site_aliasing(
        &self,
        mir: &crate::mir::types::MirFunction,
        fallback_spans: &[Span],
    ) -> Vec<crate::mir::analysis::BorrowError> {
        use crate::mir::analysis::{BorrowError, BorrowErrorKind};
        use crate::mir::types::*;
        let mut errors = Vec::new();

        for block in mir.iter_blocks() {
            if let TerminatorKind::Call { func, args, .. } = &block.terminator.kind {
                // Determine callee name from the func operand
                let callee_name = match func {
                    Operand::Constant(MirConstant::Function(name)) => Some(name.as_str()),
                    _ => None,
                };

                let Some(callee_name) = callee_name else {
                    continue;
                };

                // Look up the callee's borrow summary
                let Some(summary) = self.function_borrow_summaries.get(callee_name) else {
                    continue;
                };

                // For each conflict pair, check if the corresponding args share root slots
                for &(i, j) in &summary.conflict_pairs {
                    if i >= args.len() || j >= args.len() {
                        continue;
                    }
                    let root_i = arg_root_slot(block, &args[i]);
                    let root_j = arg_root_slot(block, &args[j]);
                    if let (Some(ri), Some(rj)) = (root_i, root_j) {
                        if ri == rj {
                            let span = block.terminator.span;
                            // Skip if this span overlaps a fallback region
                            if !fallback_spans.is_empty()
                                && Self::span_overlaps_any(&span, fallback_spans)
                            {
                                continue;
                            }
                            errors.push(BorrowError {
                                kind: BorrowErrorKind::CallSiteAliasConflict,
                                span,
                                conflicting_loan: LoanId(0),
                                loan_span: span,
                                last_use_span: None,
                                repairs: Vec::new(),
                            });
                            break; // one error per call site
                        }
                    }
                }
            }
        }

        errors
    }

    /// Check if a span overlaps with any span in a list.
    /// Used for span-granular error filtering: errors in fallback regions are suppressed.
    fn span_overlaps_any(span: &Span, fallback_spans: &[Span]) -> bool {
        fallback_spans.iter().any(|fb| {
            // Overlap: not (span ends before fb starts || span starts after fb ends)
            !(span.end <= fb.start || span.start >= fb.end)
        })
    }

    fn synthetic_item_sequence_span(items: &[Item]) -> Span {
        items
            .first()
            .map(|item| match item {
                Item::Import(_, span)
                | Item::Export(_, span)
                | Item::Module(_, span)
                | Item::TypeAlias(_, span)
                | Item::Interface(_, span)
                | Item::Trait(_, span)
                | Item::Enum(_, span)
                | Item::Extend(_, span)
                | Item::Impl(_, span)
                | Item::Function(_, span)
                | Item::Query(_, span)
                | Item::VariableDecl(_, span)
                | Item::Assignment(_, span)
                | Item::Expression(_, span)
                | Item::Stream(_, span)
                | Item::Test(_, span)
                | Item::Optimize(_, span)
                | Item::AnnotationDef(_, span)
                | Item::StructType(_, span)
                | Item::DataSource(_, span)
                | Item::QueryDecl(_, span)
                | Item::Statement(_, span)
                | Item::Comptime(_, span)
                | Item::BuiltinTypeDecl(_, span)
                | Item::BuiltinFunctionDecl(_, span)
                | Item::ForeignFunction(_, span) => *span,
            })
            .unwrap_or(Span::DUMMY)
    }

    fn synthetic_mir_statements_for_items(items: &[Item]) -> Vec<Statement> {
        let mut body = Vec::new();
        for item in items {
            match item {
                Item::VariableDecl(var_decl, span) => {
                    body.push(Statement::VariableDecl(var_decl.clone(), *span));
                }
                Item::Assignment(assign, span) => {
                    body.push(Statement::Assignment(assign.clone(), *span));
                }
                Item::Expression(expr, span) => {
                    body.push(Statement::Expression(expr.clone(), *span));
                }
                Item::Statement(stmt, _) => body.push(stmt.clone()),
                Item::Export(export, span) => {
                    if let Some(source_decl) = &export.source_decl {
                        body.push(Statement::VariableDecl(source_decl.clone(), *span));
                    }
                }
                Item::Comptime(..)
                | Item::Function(..)
                | Item::Module(..)
                | Item::Import(..)
                | Item::TypeAlias(..)
                | Item::Interface(..)
                | Item::Trait(..)
                | Item::Enum(..)
                | Item::Extend(..)
                | Item::Impl(..)
                | Item::Query(..)
                | Item::Stream(..)
                | Item::Test(..)
                | Item::Optimize(..)
                | Item::AnnotationDef(..)
                | Item::StructType(..)
                | Item::DataSource(..)
                | Item::QueryDecl(..)
                | Item::BuiltinTypeDecl(..)
                | Item::BuiltinFunctionDecl(..)
                | Item::ForeignFunction(..) => {}
            }
        }
        body
    }

    pub(super) fn analyze_non_function_items_with_mir(
        &mut self,
        context_name: &str,
        items: &[Item],
    ) -> Result<()> {
        let body = Self::synthetic_mir_statements_for_items(items);
        if body.is_empty() {
            return Ok(());
        }

        let lowering = crate::mir::lowering::lower_function_detailed(
            context_name,
            &[],
            &body,
            Self::synthetic_item_sequence_span(items),
        );
        let callee_summaries = self.build_callee_summaries(None, &lowering.all_local_names);
        let mut analysis = crate::mir::solver::analyze(&lowering.mir, &callee_summaries);
        analysis.mutability_errors = crate::mir::lowering::compute_mutability_errors(&lowering);
        crate::mir::repair::attach_repairs(&mut analysis, &lowering.mir);

        // Span-granular error filtering: when lowering had fallbacks, only
        // suppress errors whose span overlaps a fallback span. Errors in
        // cleanly-lowered regions pass through even when other regions fell back.
        let first_mutability_error = if lowering.fallback_spans.is_empty() {
            analysis.mutability_errors.first().cloned()
        } else {
            analysis
                .mutability_errors
                .iter()
                .find(|e| !Self::span_overlaps_any(&e.span, &lowering.fallback_spans))
                .cloned()
        };
        let first_borrow_error = if lowering.fallback_spans.is_empty() {
            analysis.errors.first().cloned()
        } else {
            analysis
                .errors
                .iter()
                .find(|e| !Self::span_overlaps_any(&e.span, &lowering.fallback_spans))
                .cloned()
        };

        // Build span→point mapping for non-function MIR contexts too.
        {
            let mut span_to_point = HashMap::new();
            for block in lowering.mir.iter_blocks() {
                for stmt in &block.statements {
                    span_to_point.entry(stmt.span).or_insert(stmt.point);
                }
            }
            self.mir_span_to_point
                .insert(context_name.to_string(), span_to_point);
        }

        self.mir_functions
            .insert(context_name.to_string(), lowering.mir);
        self.mir_borrow_analyses
            .insert(context_name.to_string(), analysis);

        if let Some(error) = first_mutability_error.as_ref() {
            return Err(self.mir_mutability_error(error));
        }
        if let Some(error) = first_borrow_error.as_ref() {
            return Err(self.mir_borrow_error(error));
        }

        Ok(())
    }

    /// Core function body compilation (shared by normal functions and ___impl functions)
    pub(super) fn compile_function_body(&mut self, func_def: &FunctionDef) -> Result<()> {
        // Find function index
        let func_idx = self
            .program
            .functions
            .iter()
            .position(|f| f.name == func_def.name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Function not found: {}", func_def.name),
                location: None,
            })?;

        // If compiling at top-level (not inside another function), emit a jump over the function body
        // This prevents the VM from falling through into function code during normal execution
        let jump_over = if self.current_function.is_none() {
            Some(self.emit_jump(OpCode::Jump, 0))
        } else {
            None
        };

        // Save current state
        let saved_function = self.current_function;
        let saved_next_local = self.next_local;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_is_async = self.current_function_is_async;
        let saved_ref_locals = std::mem::take(&mut self.ref_locals);
        let saved_exclusive_ref_locals = std::mem::take(&mut self.exclusive_ref_locals);
        let saved_inferred_ref_locals = std::mem::take(&mut self.inferred_ref_locals);
        let saved_local_callable_pass_modes = std::mem::take(&mut self.local_callable_pass_modes);
        let saved_local_callable_return_reference_summaries =
            std::mem::take(&mut self.local_callable_return_reference_summaries);
        let saved_reference_value_locals = std::mem::take(&mut self.reference_value_locals);
        let saved_exclusive_reference_value_locals =
            std::mem::take(&mut self.exclusive_reference_value_locals);
        let saved_reference_value_module_bindings = self.reference_value_module_bindings.clone();
        let saved_exclusive_reference_value_module_bindings =
            self.exclusive_reference_value_module_bindings.clone();
        let saved_comptime_mode = self.comptime_mode;
        let saved_drop_locals = std::mem::take(&mut self.drop_locals);
        let saved_boxed_locals = std::mem::take(&mut self.boxed_locals);
        let saved_param_locals = std::mem::take(&mut self.param_locals);
        let saved_function_params =
            std::mem::replace(&mut self.current_function_params, func_def.params.clone());
        let saved_current_function_return_reference_summary =
            self.current_function_return_reference_summary.clone();

        // Set up isolated locals for function compilation
        self.current_function = Some(func_idx);
        self.current_function_is_async = func_def.is_async;
        self.current_function_return_reference_summary = self
            .function_return_reference_summaries
            .get(&func_def.name)
            .cloned();

        // If this is a `comptime fn`, mark the compilation context as comptime
        // so that calls to other `comptime fn` functions within the body are allowed.
        if func_def.is_comptime {
            self.comptime_mode = true;
        }
        self.locals = vec![HashMap::new()];
        self.type_tracker.clear_locals(); // Clear local type info for new function
        self.ref_locals.clear();
        self.exclusive_ref_locals.clear();
        self.inferred_ref_locals.clear();
        self.local_callable_pass_modes.clear();
        self.local_callable_return_reference_summaries.clear();
        self.reference_value_locals.clear();
        self.exclusive_reference_value_locals.clear();
        self.immutable_locals.clear();
        self.param_locals.clear();
        self.push_scope();
        self.push_drop_scope();
        self.next_local = 0;

        // Reset expression-level tracking to prevent stale values from previous
        // function compilations leaking into parameter binding
        self.last_expr_schema = None;
        self.last_expr_numeric_type = None;
        self.last_expr_type_info = None;

        // Set function entry point (AFTER the jump instruction)
        self.program.functions[func_idx].entry_point = self.program.current_offset();

        // Start blob builder for this function (snapshot global pool sizes).
        let saved_blob_builder = self.current_blob_builder.take();
        self.current_blob_builder = Some(super::FunctionBlobBuilder::new(
            func_def.name.clone(),
            self.program.current_offset(),
            self.program.constants.len(),
            self.program.strings.len(),
        ));

        let inferred_modes = self.inferred_param_pass_modes.get(&func_def.name).cloned();

        // Bind parameters as locals - destructure each parameter value
        // Parameters arrive in local slots 0, 1, 2, ... from caller
        for (idx, param) in func_def.params.iter().enumerate() {
            let effective_pass_mode = inferred_modes
                .as_ref()
                .and_then(|modes| modes.get(idx))
                .copied()
                .unwrap_or_else(|| {
                    if param.is_mut_reference {
                        ParamPassMode::ByRefExclusive
                    } else if param.is_reference {
                        ParamPassMode::ByRefShared
                    } else {
                        ParamPassMode::ByValue
                    }
                });

            // Load parameter value from its slot
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(idx as u16)),
            ));
            // Destructure into bindings (self declares locals and binds them)
            self.compile_destructure_pattern(&param.pattern)?;
            self.apply_binding_semantics_to_pattern_bindings(
                &param.pattern,
                true,
                Self::binding_semantics_for_param(param, effective_pass_mode),
            );
            for (binding_name, _) in param.pattern.get_bindings() {
                if let Some(local_idx) = self.resolve_local(&binding_name) {
                    if param.is_const {
                        self.const_locals.insert(local_idx);
                        self.immutable_locals.insert(local_idx);
                    } else if matches!(effective_pass_mode, ParamPassMode::ByRefShared) {
                        self.immutable_locals.insert(local_idx);
                    }
                }
            }

            // Propagate parameter type annotations into local type tracker so
            // dot-access compiles to typed field ops (no runtime property fallback).
            if let Some(name) = param.pattern.as_identifier() {
                if let Some(local_idx) = self.resolve_local(name) {
                    if let Some(type_ann) = &param.type_annotation {
                        match type_ann {
                            shape_ast::ast::TypeAnnotation::Object(fields) => {
                                let field_refs: Vec<&str> =
                                    fields.iter().map(|f| f.name.as_str()).collect();
                                let schema_id =
                                    self.type_tracker.register_inline_object_schema(&field_refs);
                                let schema_name = self
                                    .type_tracker
                                    .schema_registry()
                                    .get_by_id(schema_id)
                                    .map(|s| s.name.clone())
                                    .unwrap_or_else(|| format!("__anon_{}", schema_id));
                                let info = crate::type_tracking::VariableTypeInfo::known(
                                    schema_id,
                                    schema_name,
                                );
                                self.type_tracker.set_local_type(local_idx, info);
                            }
                            _ => {
                                if let Some(type_name) =
                                    Self::tracked_type_name_from_annotation(type_ann)
                                {
                                    self.set_local_type_info(local_idx, &type_name);
                                }
                            }
                        }
                        self.try_track_datatable_type(type_ann, local_idx, true)?;
                    } else {
                        // Mark as a param local with inferred type (no explicit annotation).
                        // storage_hint_for_expr will not trust these for typed Add emission.
                        self.param_locals.insert(local_idx);
                        let inferred_type_name = self
                            .inferred_param_type_hints
                            .get(&func_def.name)
                            .and_then(|hints| hints.get(idx))
                            .and_then(|hint| hint.clone());
                        if let Some(type_name) = inferred_type_name {
                            self.set_local_type_info(local_idx, &type_name);
                        }
                    }
                }
            }
        }

        // Mark reference parameters in ref_locals so identifier/assignment compilation
        // emits DerefLoad/DerefStore/SetIndexRef instead of LoadLocal/StoreLocal/SetLocalIndex.
        // Also track which ref_locals were INFERRED (not explicitly declared) so that
        // closure capture can distinguish true borrows from pass-by-ref optimizations.
        for (idx, param) in func_def.params.iter().enumerate() {
            if param.is_reference {
                self.ref_locals.insert(idx as u16);
                if param.is_mut_reference {
                    self.exclusive_ref_locals.insert(idx as u16);
                }
                // A param is "inferred ref" if it has no type annotation and no explicit
                // mut reference — the compiler's pass-mode inference set is_reference.
                let was_inferred = param.type_annotation.is_none()
                    && !param.is_mut_reference
                    && inferred_modes
                        .as_ref()
                        .and_then(|modes| modes.get(idx))
                        .map_or(false, |mode| mode.is_reference());
                if was_inferred {
                    self.inferred_ref_locals.insert(idx as u16);
                }
            }
        }

        // If self is a DataTable closure, tag the first user parameter as RowView
        if let Some((schema_id, type_name)) = self.closure_row_schema.take() {
            let row_param_slot = func_def
                .params
                .first()
                .and_then(|param| param.pattern.as_identifier())
                .and_then(|name| self.resolve_local(name))
                .unwrap_or_else(|| self.program.functions[func_idx].captures_count);

            self.type_tracker.set_local_type(
                row_param_slot,
                crate::type_tracking::VariableTypeInfo::row_view(schema_id, type_name),
            );
        }

        // Parameter defaults: only check parameters that have a default value.
        // Required parameters are guaranteed to have a real value from the caller
        // (arity is enforced at call sites), so no unit-check is needed for them.
        for (idx, param) in func_def.params.iter().enumerate() {
            if let Some(default_expr) = &param.default_value {
                // Check if the caller omitted this argument (sent unit sentinel)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(idx as u16)),
                ));
                self.emit_unit();
                self.emit(Instruction::simple(OpCode::Eq));

                let skip_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

                // Caller omitted this arg — fill in the default value
                self.compile_expr(default_expr)?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(idx as u16)),
                ));

                self.patch_jump(skip_jump);
            }
        }

        // Compile function body with implicit return support
        let body_len = func_def.body.len();
        for (idx, stmt) in func_def.body.iter().enumerate() {
            let is_last = idx == body_len - 1;

            // Check if the last statement is an expression - if so, use implicit return
            if is_last {
                match stmt {
                    Statement::Expression(expr, _) => {
                        // Compile expression and keep value on stack for implicit return.
                        if self.current_function_return_reference_summary.is_some() {
                            self.compile_expr_preserving_refs(expr)?;
                        } else {
                            self.compile_expr(expr)?;
                        }
                        // Emit drops for function-level locals before returning
                        let total_scopes = self.drop_locals.len();
                        if total_scopes > 0 {
                            self.emit_drops_for_early_exit(total_scopes)?;
                        }
                        self.emit(Instruction::simple(OpCode::ReturnValue));
                        // Skip the fallback return below since we've already returned
                        // Update function locals count
                        self.program.functions[func_idx].locals_count = self.next_local;
                        self.capture_function_local_storage_hints(func_idx);
                        // Finalize blob builder and store completed blob
                        self.finalize_current_blob(func_idx);
                        self.current_blob_builder = saved_blob_builder;
                        // Restore state
                        self.drop_locals = saved_drop_locals;
                        self.boxed_locals = saved_boxed_locals;
                        self.param_locals = saved_param_locals;
                        self.current_function_params = saved_function_params;
                        self.pop_scope();
                        self.locals = saved_locals;
                        self.current_function = saved_function;
                        self.current_function_is_async = saved_is_async;
                        self.next_local = saved_next_local;
                        self.ref_locals = saved_ref_locals;
                        self.exclusive_ref_locals = saved_exclusive_ref_locals.clone();
                        self.inferred_ref_locals = saved_inferred_ref_locals.clone();
                        self.local_callable_pass_modes = saved_local_callable_pass_modes.clone();
                        self.local_callable_return_reference_summaries =
                            saved_local_callable_return_reference_summaries.clone();
                        self.reference_value_locals = saved_reference_value_locals;
                        self.exclusive_reference_value_locals =
                            saved_exclusive_reference_value_locals;
                        self.reference_value_module_bindings =
                            saved_reference_value_module_bindings;
                        self.exclusive_reference_value_module_bindings =
                            saved_exclusive_reference_value_module_bindings;
                        self.comptime_mode = saved_comptime_mode;
                        self.current_function_return_reference_summary =
                            saved_current_function_return_reference_summary;
                        // Patch the jump-over instruction if we emitted one
                        if let Some(jump_addr) = jump_over {
                            self.patch_jump(jump_addr);
                        }
                        return Ok(());
                    }
                    Statement::Return(_, _) => {
                        // Explicit return - compile normally, it will handle its own return
                        let future_names = self
                            .future_reference_use_names_for_remaining_statements(
                                &func_def.body[idx + 1..],
                            );
                        self.push_future_reference_use_names(future_names);
                        let compile_result = self.compile_statement(stmt);
                        self.pop_future_reference_use_names();
                        compile_result?;
                        // After an explicit return, we still need the fallback below for
                        // control flow that might skip the return (though rare)
                    }
                    _ => {
                        // Other statement types - compile normally
                        let future_names = self
                            .future_reference_use_names_for_remaining_statements(
                                &func_def.body[idx + 1..],
                            );
                        self.push_future_reference_use_names(future_names);
                        let compile_result = self.compile_statement(stmt);
                        self.pop_future_reference_use_names();
                        compile_result?;
                        self.release_unused_local_reference_borrows_for_remaining_statements(
                            &func_def.body[idx + 1..],
                        );
                    }
                }
            } else {
                let mut future_names = self
                    .future_reference_use_names_for_remaining_statements(&func_def.body[idx + 1..]);
                if self.current_function_return_reference_summary.is_some()
                    && idx + 1 < body_len
                    && let Some(Statement::Expression(expr, _)) = func_def.body.last()
                {
                    self.collect_reference_use_names_from_expr(expr, true, &mut future_names);
                }
                self.push_future_reference_use_names(future_names);
                let compile_result = self.compile_statement(stmt);
                self.pop_future_reference_use_names();
                compile_result?;
                self.release_unused_local_reference_borrows_for_remaining_statements(
                    &func_def.body[idx + 1..],
                );
            }
        }

        // Emit drops for function-level locals before implicit null return
        let total_scopes = self.drop_locals.len();
        if total_scopes > 0 {
            self.emit_drops_for_early_exit(total_scopes)?;
        }

        // Implicit return null if no explicit return and last stmt wasn't an expression
        self.emit(Instruction::simple(OpCode::PushNull));
        self.emit(Instruction::simple(OpCode::ReturnValue));

        // Update function locals count
        self.program.functions[func_idx].locals_count = self.next_local;
        self.capture_function_local_storage_hints(func_idx);

        // Finalize blob builder and store completed blob
        self.finalize_current_blob(func_idx);
        self.current_blob_builder = saved_blob_builder;

        // Restore state
        self.drop_locals = saved_drop_locals;
        self.boxed_locals = saved_boxed_locals;
        self.current_function_params = saved_function_params;
        self.pop_scope();
        self.locals = saved_locals;
        self.current_function = saved_function;
        self.current_function_is_async = saved_is_async;
        self.next_local = saved_next_local;
        self.ref_locals = saved_ref_locals;
        self.exclusive_ref_locals = saved_exclusive_ref_locals;
        self.inferred_ref_locals = saved_inferred_ref_locals;
        self.local_callable_pass_modes = saved_local_callable_pass_modes;
        self.local_callable_return_reference_summaries =
            saved_local_callable_return_reference_summaries;
        self.reference_value_locals = saved_reference_value_locals;
        self.exclusive_reference_value_locals = saved_exclusive_reference_value_locals;
        self.reference_value_module_bindings = saved_reference_value_module_bindings;
        self.exclusive_reference_value_module_bindings =
            saved_exclusive_reference_value_module_bindings;
        self.comptime_mode = saved_comptime_mode;
        self.current_function_return_reference_summary =
            saved_current_function_return_reference_summary;

        // Patch the jump-over instruction if we emitted one
        if let Some(jump_addr) = jump_over {
            self.patch_jump(jump_addr);
        }

        Ok(())
    }

    // Compile a statement
}

/// Extract the root SlotId from a MIR operand if it references a local.
fn arg_root_slot(
    block: &crate::mir::types::BasicBlock,
    op: &crate::mir::types::Operand,
) -> Option<crate::mir::types::SlotId> {
    use crate::mir::types::{Operand, Place, Rvalue, StatementKind};
    use std::collections::{HashMap, HashSet};

    fn resolve_slot_root(
        slot: crate::mir::types::SlotId,
        alias_roots: &HashMap<crate::mir::types::SlotId, crate::mir::types::SlotId>,
    ) -> crate::mir::types::SlotId {
        let mut current = slot;
        let mut seen = HashSet::new();
        while seen.insert(current) {
            let Some(next) = alias_roots.get(&current).copied() else {
                break;
            };
            current = next;
        }
        current
    }

    fn operand_root_slot(
        op: &Operand,
        alias_roots: &HashMap<crate::mir::types::SlotId, crate::mir::types::SlotId>,
    ) -> Option<crate::mir::types::SlotId> {
        match op {
            Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
                Some(resolve_slot_root(place.root_local(), alias_roots))
            }
            Operand::Constant(_) => None,
        }
    }

    let mut alias_roots = HashMap::new();
    for stmt in &block.statements {
        let StatementKind::Assign(Place::Local(dst), rvalue) = &stmt.kind else {
            continue;
        };

        match rvalue {
            Rvalue::Borrow(_, place) => {
                alias_roots.insert(*dst, resolve_slot_root(place.root_local(), &alias_roots));
            }
            Rvalue::Use(inner) | Rvalue::Clone(inner) | Rvalue::UnaryOp(_, inner) => {
                if let Some(root) = operand_root_slot(inner, &alias_roots) {
                    alias_roots.insert(*dst, root);
                } else {
                    alias_roots.remove(dst);
                }
            }
            _ => {
                alias_roots.remove(dst);
            }
        }
    }

    operand_root_slot(op, &alias_roots)
}

#[cfg(test)]
mod tests {
    use crate::bytecode::Constant;
    use crate::compiler::{BytecodeCompiler, ParamPassMode};
    use crate::executor::{VMConfig, VirtualMachine};
    use crate::mir::analysis::BorrowErrorKind;
    use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};
    use shape_ast::ast::{DestructurePattern, FunctionParameter, Item, Span};
    use shape_value::ValueWord;

    fn eval(code: &str) -> ValueWord {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        let bytecode = compiler.compile(&program).expect("compile failed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.execute(None).expect("execution failed").clone()
    }

    fn compiles(code: &str) -> Result<crate::bytecode::BytecodeProgram, String> {
        let program =
            shape_ast::parser::parse_program(code).map_err(|e| format!("parse: {}", e))?;
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        compiler
            .compile(&program)
            .map_err(|e| format!("compile: {}", e))
    }

    fn test_param(is_const: bool, is_reference: bool, is_mut_reference: bool) -> FunctionParameter {
        FunctionParameter {
            pattern: DestructurePattern::Identifier("value".to_string(), Span::DUMMY),
            is_const,
            is_reference,
            is_mut_reference,
            is_out: false,
            type_annotation: None,
            default_value: None,
        }
    }

    #[test]
    fn test_binding_semantics_for_param_modes() {
        let by_value = BytecodeCompiler::binding_semantics_for_param(
            &test_param(false, false, false),
            ParamPassMode::ByValue,
        );
        assert_eq!(
            by_value.ownership_class,
            BindingOwnershipClass::OwnedMutable
        );
        assert_eq!(by_value.storage_class, BindingStorageClass::Direct);

        let const_value = BytecodeCompiler::binding_semantics_for_param(
            &test_param(true, false, false),
            ParamPassMode::ByValue,
        );
        assert_eq!(
            const_value.ownership_class,
            BindingOwnershipClass::OwnedImmutable
        );
        assert_eq!(const_value.storage_class, BindingStorageClass::Direct);

        let shared_ref = BytecodeCompiler::binding_semantics_for_param(
            &test_param(false, true, false),
            ParamPassMode::ByRefShared,
        );
        assert_eq!(
            shared_ref.ownership_class,
            BindingOwnershipClass::OwnedImmutable
        );
        assert_eq!(shared_ref.storage_class, BindingStorageClass::Reference);

        let exclusive_ref = BytecodeCompiler::binding_semantics_for_param(
            &test_param(false, true, true),
            ParamPassMode::ByRefExclusive,
        );
        assert_eq!(
            exclusive_ref.ownership_class,
            BindingOwnershipClass::OwnedMutable
        );
        assert_eq!(exclusive_ref.storage_class, BindingStorageClass::Reference);
    }

    #[test]
    fn test_block_expr_destructured_binding_still_runs() {
        let code = r#"
            let value = {
                let [a, b] = [1, 2]
                a + b
            }
            value
        "#;
        let result = eval(code);
        assert_eq!(result.as_number_coerce().unwrap(), 3.0);
    }

    #[test]
    fn test_const_param_requires_compile_time_constant_argument() {
        let code = r#"
            function connect(const conn_str: string) {
                conn_str
            }
            let value = "duckdb://local.db"
            connect(value)
        "#;
        let err = compiles(code).expect_err("non-constant argument for const param should fail");
        assert!(
            err.contains("declared `const` and requires a compile-time constant argument"),
            "Expected const argument diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_const_template_skips_comptime_until_specialized() {
        let code = r#"
            annotation schema_connect() {
                comptime post(target, ctx) {
                    // `uri` is a const template parameter and is only bound on specialization.
                    if uri == "duckdb://analytics.db" {
                        set return int
                    } else {
                        set return int
                    }
                }
            }

            @schema_connect()
            function connect(const uri) {
                1
            }
        "#;
        let _ = compiles(code).expect("template base should compile without specialization");
    }

    #[test]
    fn test_const_template_specialization_binds_const_values() {
        let code = r#"
            annotation schema_connect() {
                comptime post(target, ctx) {
                    if uri == "duckdb://analytics.db" {
                        set return int
                    } else {
                        set return int
                    }
                }
            }

            @schema_connect()
            function connect(const uri) {
                1
            }

            let a = connect("duckdb://analytics.db")
            let b = connect("duckdb://other.db")
        "#;
        let bytecode = compiles(code).expect("const specialization should compile");
        let specialization_count = bytecode
            .functions
            .iter()
            .filter(|f| f.name.starts_with("connect__const_"))
            .count();
        assert_eq!(
            specialization_count, 2,
            "expected one specialization per distinct const argument"
        );
    }

    #[test]
    fn test_comptime_before_cannot_override_explicit_param_type() {
        let code = r#"
            annotation force_string() {
                comptime pre(target, ctx) {
                    set param x: string
                }
            }
            @force_string()
            function foo(x: int) {
                x
            }
        "#;
        let err = compiles(code).expect_err("explicit param type override should fail");
        assert!(
            err.contains("cannot override explicit type of parameter 'x'"),
            "Expected explicit param override error, got: {}",
            err
        );
    }

    #[test]
    fn test_comptime_after_cannot_override_explicit_return_type() {
        let code = r#"
            annotation force_string_return() {
                comptime post(target, ctx) {
                    set return string
                }
            }
            @force_string_return()
            function foo() -> int {
                1
            }
        "#;
        let err = compiles(code).expect_err("explicit return type override should fail");
        assert!(
            err.contains("cannot override explicit function return type annotation"),
            "Expected explicit return override error, got: {}",
            err
        );
    }

    #[test]
    fn test_comptime_after_receives_annotation_args() {
        let code = r#"
            annotation set_return_type_from_annotation(type_name) {
                comptime post(target, ctx, ty) {
                    if ty == "int" {
                        set return int
                    } else {
                        set return string
                    }
                }
            }
            @set_return_type_from_annotation("int")
            fn foo() {
                1
            }
            foo()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected numeric result"),
            1.0
        );
    }

    #[test]
    fn test_comptime_after_variadic_annotation_args() {
        let code = r#"
            annotation variadic_schema() {
                comptime post(target, ctx, ...config) {
                    set return int
                }
            }
            @variadic_schema(1, "x", true)
            fn foo() {
                1
            }
            foo()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected numeric result"),
            1.0
        );
    }

    #[test]
    fn test_comptime_after_arg_arity_errors() {
        let missing_arg = r#"
            annotation needs_arg() {
                comptime post(target, ctx, config) {
                    target.name
                }
            }
            @needs_arg()
            fn foo() { 1 }
        "#;
        let err = compiles(missing_arg).expect_err("missing annotation arg should fail");
        assert!(
            err.contains("missing annotation argument for comptime handler parameter 'config'"),
            "unexpected error: {}",
            err
        );

        let too_many = r#"
            annotation one_arg() {
                comptime post(target, ctx, config) {
                    target.name
                }
            }
            @one_arg(1, 2)
            fn foo() { 1 }
        "#;
        let err = compiles(too_many).expect_err("too many annotation args should fail");
        assert!(
            err.contains("too many annotation arguments"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_comptime_after_can_replace_function_body() {
        let code = r#"
            annotation synthesize_body() {
                comptime post(target, ctx) {
                    replace body {
                        return 42
                    }
                }
            }
            @synthesize_body()
            function foo() {
            }
            foo()
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 42 from synthesized body"),
            42.0
        );
    }

    #[test]
    fn test_comptime_after_can_replace_function_body_from_expr() {
        let code = r#"
            comptime fn body_src() {
                "return 7"
            }

            annotation synthesize_body_expr() {
                comptime post(target, ctx) {
                    replace body (body_src())
                }
            }
            @synthesize_body_expr()
            function foo() {
            }
            foo()
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 7 from synthesized body"),
            7.0
        );
    }

    #[test]
    fn test_comptime_handler_extend_generates_method() {
        // A comptime handler using direct `extend` should register generated methods.
        let code = r#"
            annotation add_method() {
                targets: [type]
                comptime post(target, ctx) {
                    extend Number {
                        method doubled() { self * 2.0 }
                    }
                }
            }

            @add_method()
            type Marker { x: int }

            (5.0).doubled()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(10.0)"),
            10.0
        );
    }

    #[test]
    fn test_comptime_handler_extend_method_executes() {
        // Verify the generated extend method actually runs correctly
        let code = r#"
            annotation auto_extend() {
                targets: [type]
                comptime post(target, ctx) {
                    extend Number {
                        method tripled() { self * 3.0 }
                    }
                }
            }
            @auto_extend()
            type Marker { x: int }
            (10.0).tripled()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(30.0)"),
            30.0
        );
    }

    #[test]
    fn test_comptime_handler_non_object_result_ignored() {
        // Handler values are ignored unless explicit directives are emitted.
        let code = r#"
            annotation no_op() {
                comptime post(target, ctx) {
                    "just a string"
                }
            }
            @no_op()
            function my_func(x) {
                return x + 1.0
            }
            my_func(5.0)
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(6.0)"),
            6.0
        );
    }

    #[test]
    fn test_legacy_action_object_not_processed() {
        // Legacy action-object return values are intentionally ignored.
        let code = r#"
            annotation legacy() {
                comptime post(target, ctx) {
                    { action: "extend", source: "method doubled() { return self * 2.0 }", type: "Number" }
                }
            }
            @legacy()
            function placeholder() { 0 }
            (5.0).doubled()
        "#;
        let result = compiles(code).expect("legacy action object should not fail compilation");
        let has_doubled = result
            .functions
            .iter()
            .any(|f| f.name.ends_with("::doubled"));
        assert!(
            !has_doubled,
            "Legacy action-object return should not generate methods"
        );
    }

    #[test]
    fn test_comptime_handler_extend_multiple_methods() {
        // A comptime handler can emit multiple methods in one extend block.
        let code = r#"
            annotation math_ops() {
                targets: [type]
                comptime post(target, ctx) {
                    extend Number {
                        method add_ten() { self + 10.0 }
                        method sub_ten() { self - 10.0 }
                    }
                }
            }
            @math_ops()
            type Marker { x: int }
            let a = (25.0).add_ten()
            let b = (25.0).sub_ten()
            a + b
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(50.0)"),
            50.0
        );
    }

    #[test]
    fn test_expression_annotation_comptime_handler_executes() {
        // Expression-level annotation should run comptime handler and process extend directives.
        let code = r#"
            annotation expr_extend() {
                targets: [expression]
                comptime post(target, ctx) {
                    extend Number {
                        method quadrupled() { self * 4.0 }
                    }
                }
            }

            let x = @expr_extend() 2.0
            x.quadrupled()
        "#;
        let result = eval(code);
        assert_eq!(
            result.as_number_coerce().expect("Expected Number(8.0)"),
            8.0
        );
    }

    #[test]
    fn test_expression_annotation_target_validation() {
        // Type-only annotation applied to an expression should fail with a target error.
        let code = r#"
            annotation only_type() {
                targets: [type]
                comptime post(target, ctx) {
                    target.kind
                }
            }

            let x = @only_type() 1
        "#;
        let err = compiles(code).expect_err("type-only annotation on expression should fail");
        assert!(
            err.contains("cannot be applied to a expression"),
            "Expected expression target error, got: {}",
            err
        );
    }

    #[test]
    fn test_expression_annotation_rejects_definition_lifecycle_hooks() {
        let code = r#"
            annotation info() {
                metadata(target, ctx) {
                    target.kind
                }
            }

            let x = @info() 1
        "#;
        let err =
            compiles(code).expect_err("definition-time lifecycle hooks on expression should fail");
        assert!(
            err.contains("definition-time lifecycle hooks"),
            "Expected definition-time lifecycle target error, got: {}",
            err
        );
    }

    #[test]
    fn test_await_annotation_target_validation() {
        // Await-only annotation should compile in await context.
        let ok_code = r#"
            annotation only_await() {
                targets: [await_expr]
                comptime post(target, ctx) {
                    target.kind
                }
            }

            async function ready() {
                return 1
            }

            async function run() {
                await @only_await() ready()
                return 1
            }
        "#;
        assert!(
            compiles(ok_code).is_ok(),
            "await annotation should be accepted in await context"
        );

        // The same await-only annotation on a plain expression must fail.
        let bad_code = r#"
            annotation only_await() {
                targets: [await_expr]
                comptime post(target, ctx) {
                    target.kind
                }
            }

            let x = @only_await() 1
        "#;
        let err = compiles(bad_code).expect_err("await-only annotation on expression should fail");
        assert!(
            err.contains("cannot be applied to a expression"),
            "Expected expression target error, got: {}",
            err
        );
    }

    #[test]
    fn test_direct_extend_target_on_type_via_comptime_handler() {
        // Direct `extend target { ... }` should work without action-object indirection.
        let code = r#"
            annotation add_sum() {
                targets: [type]
                comptime post(target, ctx) {
                    extend target {
                        method sum() {
                            self.x + self.y
                        }
                    }
                }
            }

            @add_sum()
            type Point { x: int, y: int }

            Point { x: 2, y: 3 }.sum()
        "#;
        let result = eval(code);
        assert_eq!(result.as_number_coerce().expect("Expected 5"), 5.0);
    }

    #[test]
    fn test_direct_remove_target_on_expression() {
        // `remove target` on an expression target should replace the expression with null.
        let code = r#"
            annotation drop_expr() {
                targets: [expression]
                comptime post(target, ctx) {
                    remove target
                }
            }

            let x = @drop_expr() 123
            x
        "#;
        let result = eval(code);
        assert!(
            result.is_none(),
            "Expected None after remove target, got {:?}",
            result
        );
    }

    #[test]
    fn test_replace_body_original_calls_original_function() {
        // __original__ should call the original function body from a replacement body.
        let code = r#"
            annotation wrap() {
                comptime post(target, ctx) {
                    replace body {
                        return __original__(5) + 100
                    }
                }
            }
            @wrap()
            function add_ten(x) {
                return x + 10
            }
            add_ten(0)
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 115 from __original__ call"),
            115.0,
        );
    }

    #[test]
    fn test_replace_body_args_contains_function_parameters() {
        // `args` should be an array of the function's parameters in the replacement body.
        let code = r#"
            annotation with_args() {
                comptime post(target, ctx) {
                    replace body {
                        return args.len()
                    }
                }
            }
            @with_args()
            function three_params(a, b, c) {
                return 0
            }
            three_params(10, 20, 30)
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 3 from args.len()"),
            3.0,
        );
    }

    #[test]
    fn test_replace_body_original_with_no_params() {
        // __original__ should work even with zero-parameter functions.
        let code = r#"
            annotation add_one() {
                comptime post(target, ctx) {
                    replace body {
                        return __original__() + 1
                    }
                }
            }
            @add_one()
            function get_value() {
                return 41
            }
            get_value()
        "#;
        let result = eval(code);
        assert_eq!(
            result
                .as_number_coerce()
                .expect("Expected 42 from __original__() + 1"),
            42.0,
        );
    }

    #[test]
    fn test_content_addressed_program_has_main_and_functions() {
        let code = r#"
            function add(a, b) { a + b }
            function mul(a, b) { a * b }
            let x = add(2, 3)
            mul(x, 4)
        "#;
        let bytecode = compiles(code).expect("should compile");
        let ca = bytecode
            .content_addressed
            .expect("content_addressed program should be Some");

        // Should have at least __main__, add, and mul blobs
        assert!(
            ca.function_store.len() >= 3,
            "Expected at least 3 blobs (__main__, add, mul), got {}",
            ca.function_store.len()
        );

        // Entry should be set (non-zero hash)
        assert_ne!(
            ca.entry,
            crate::bytecode::FunctionHash::ZERO,
            "Entry hash should not be zero"
        );

        // Entry should be in the function store
        assert!(
            ca.function_store.contains_key(&ca.entry),
            "Entry hash should be present in function_store"
        );

        // Check that each blob has a non-zero content hash
        for (hash, blob) in &ca.function_store {
            assert_ne!(
                *hash,
                crate::bytecode::FunctionHash::ZERO,
                "Blob '{}' should have non-zero hash",
                blob.name
            );
            assert_eq!(
                *hash, blob.content_hash,
                "Blob '{}' key should match its content_hash",
                blob.name
            );
            assert!(
                !blob.instructions.is_empty(),
                "Blob '{}' should have instructions",
                blob.name
            );
        }
    }

    #[test]
    fn test_content_addressed_blob_has_local_pools() {
        let code = r#"
            function greet(name) { "hello " + name }
            greet("world")
        "#;
        let bytecode = compiles(code).expect("should compile");
        let ca = bytecode
            .content_addressed
            .expect("content_addressed program should be Some");

        // Find the greet blob
        let greet_blob = ca
            .function_store
            .values()
            .find(|b| b.name == "greet")
            .expect("greet blob should exist");

        assert_eq!(greet_blob.arity, 1);
        assert_eq!(greet_blob.param_names, vec!["name".to_string()]);
        // Should have at least one string in its local pool ("hello ")
        assert!(
            !greet_blob.strings.is_empty() || !greet_blob.constants.is_empty(),
            "greet blob should have local constants or strings"
        );
    }

    #[test]
    fn test_content_addressed_stable_hash() {
        // Compiling the same code twice should produce the same content hashes
        let code = r#"
            function double(x) { x * 2 }
            double(21)
        "#;
        let bytecode1 = compiles(code).expect("should compile");
        let bytecode2 = compiles(code).expect("should compile");

        let ca1 = bytecode1.content_addressed.expect("should have ca1");
        let ca2 = bytecode2.content_addressed.expect("should have ca2");

        // Find the double blob in both
        let double1 = ca1
            .function_store
            .values()
            .find(|b| b.name == "double")
            .expect("double blob in ca1");
        let double2 = ca2
            .function_store
            .values()
            .find(|b| b.name == "double")
            .expect("double blob in ca2");

        assert_eq!(
            double1.content_hash, double2.content_hash,
            "Same code should produce same content hash"
        );
    }

    #[test]
    fn test_extern_c_signature_supports_callback_and_nullable_cstring() {
        let code = r#"
            extern C fn walk(
                root: Option<string>,
                on_entry: (path: ptr, data: ptr) => i32
            ) -> Option<string> from "libwalk";
        "#;
        let bytecode = compiles(code).expect("should compile");
        assert_eq!(bytecode.foreign_functions.len(), 1);
        let entry = &bytecode.foreign_functions[0];
        let native = entry
            .native_abi
            .as_ref()
            .expect("extern C binding should carry native ABI metadata");
        assert_eq!(
            native.signature,
            "fn(cstring?, callback(fn(ptr, ptr) -> i32)) -> cstring?"
        );
    }

    #[test]
    fn test_extern_c_signature_maps_vec_to_native_slice() {
        let code = r#"
            extern C fn hash_bytes(data: Vec<byte>) -> u64 from "libhash";
            extern C fn split_words(data: Vec<Option<string>>) -> Vec<Option<string>> from "libhash";
        "#;
        let bytecode = compiles(code).expect("should compile");
        assert_eq!(bytecode.foreign_functions.len(), 2);
        let hash = bytecode.foreign_functions[0]
            .native_abi
            .as_ref()
            .expect("extern C function should carry native ABI metadata");
        assert_eq!(hash.signature, "fn(cslice<u8>) -> u64");
        let split = bytecode.foreign_functions[1]
            .native_abi
            .as_ref()
            .expect("extern C function should carry native ABI metadata");
        assert_eq!(split.signature, "fn(cslice<cstring?>) -> cslice<cstring?>");
    }

    #[test]
    fn test_extern_c_cmut_slice_param_marks_ref_mutate_contract() {
        let code = r#"
            extern C fn hash_bytes(data: Vec<byte>) -> u64 from "libhash";
            extern C fn mutate_bytes(data: CMutSlice<byte>) -> void from "libhash";
        "#;
        let bytecode = compiles(code).expect("should compile");
        let hash_fn = bytecode
            .functions
            .iter()
            .find(|func| func.name == "hash_bytes")
            .expect("hash_bytes function should exist");
        assert_eq!(hash_fn.ref_params, vec![false]);
        assert_eq!(hash_fn.ref_mutates, vec![false]);

        let mutate_fn = bytecode
            .functions
            .iter()
            .find(|func| func.name == "mutate_bytes")
            .expect("mutate_bytes function should exist");
        assert_eq!(mutate_fn.ref_params, vec![true]);
        assert_eq!(mutate_fn.ref_mutates, vec![true]);
    }

    #[test]
    fn test_extern_c_signature_rejects_nested_vec_type() {
        let code = r#"
            extern C fn bad(data: Vec<Vec<byte>>) -> i32 from "libbad";
        "#;
        let err = compiles(code).expect_err("nested Vec native slice should be rejected");
        assert!(err.contains("unsupported parameter type 'Vec<Vec<byte>>'"));
    }

    #[test]
    fn test_extern_c_call_targets_stub_then_call_foreign() {
        let code = r#"
            extern C fn cos_c(x: f64) -> f64 from "libm.so.6" as "cos";
            let value = cos_c(0.0)
            value
        "#;
        let bytecode = compiles(code).expect("should compile");
        let cos_idx = bytecode
            .functions
            .iter()
            .position(|f| f.name == "cos_c")
            .expect("cos_c function should exist") as u16;
        let mut saw_call_value = false;
        for ip in 0..bytecode.instructions.len() {
            let instr = bytecode.instructions[ip];
            if instr.opcode == crate::bytecode::OpCode::CallValue {
                saw_call_value = true;
            }
        }
        assert!(
            saw_call_value,
            "top-level should invoke function values through CallValue"
        );

        let cos = &bytecode.functions[cos_idx as usize];
        let stub_instrs = &bytecode.instructions[cos.entry_point..];
        assert!(
            stub_instrs
                .iter()
                .take(8)
                .any(|i| i.opcode == crate::bytecode::OpCode::CallForeign),
            "foreign stub should contain CallForeign opcode near its entry"
        );
        let ca = bytecode
            .content_addressed
            .as_ref()
            .expect("content-addressed program should exist");
        let cos_hash = *ca
            .function_store
            .iter()
            .find(|(_, blob)| blob.name == "cos_c")
            .map(|(hash, _)| hash)
            .expect("cos_c blob should exist");
        let main_blob = ca
            .function_store
            .values()
            .find(|blob| blob.name == "__main__")
            .expect("__main__ blob should exist");
        assert!(
            main_blob.dependencies.contains(&cos_hash),
            "__main__ blob must depend on cos_c hash so function constants remap correctly"
        );
        let has_dep_function_constant = main_blob
            .constants
            .iter()
            .any(|c| matches!(c, Constant::Function(0)));
        assert!(
            has_dep_function_constant,
            "__main__ constants should store function references as dependency-local indices"
        );
    }

    #[test]
    fn test_duckdb_package_style_arrow_import_compiles() {
        let code = r#"
            extern C fn duckdb_query_arrow(conn: ptr, sql: string, out_result: ptr) -> i32 from "duckdb";
            extern C fn duckdb_query_arrow_schema(result: ptr, out_schema: ptr) -> i32 from "duckdb";
            extern C fn duckdb_query_arrow_array(result: ptr, out_array: ptr) -> i32 from "duckdb";
            extern C fn duckdb_destroy_arrow(result_p: ptr) -> void from "duckdb" as "duckdb_destroy_arrow";

            type CandleRow {
                ts: i64,
                close: f64,
            }

            fn query_typed(conn: ptr, sql: string) -> Result<Table<CandleRow>, AnyError> {
                let result_cell = __native_ptr_new_cell()
                __native_ptr_write_ptr(result_cell, 0)
                duckdb_query_arrow(conn, sql, result_cell)
                let arrow_result = __native_ptr_read_ptr(result_cell)

                let schema_cell = __native_ptr_new_cell()
                __native_ptr_write_ptr(schema_cell, 0)
                duckdb_query_arrow_schema(arrow_result, schema_cell)
                let schema_handle = __native_ptr_read_ptr(schema_cell)
                let schema_ptr = __native_ptr_read_ptr(schema_handle)

                let array_cell = __native_ptr_new_cell()
                __native_ptr_write_ptr(array_cell, 0)
                duckdb_query_arrow_array(arrow_result, array_cell)
                let array_handle = __native_ptr_read_ptr(array_cell)
                let array_ptr = __native_ptr_read_ptr(array_handle)

                let typed: Result<Table<CandleRow>, AnyError> =
                    __native_table_from_arrow_c_typed(schema_ptr, array_ptr, "CandleRow")

                duckdb_destroy_arrow(result_cell)
                __native_ptr_free_cell(array_cell)
                __native_ptr_free_cell(schema_cell)
                __native_ptr_free_cell(result_cell)

                typed
            }
        "#;
        compiles(code).expect("duckdb package-style native code should compile");
    }

    #[test]
    fn test_extern_c_resolution_is_package_scoped_not_global() {
        let code = r#"
            extern C fn dep_a_call() -> i32 from "shared";
            extern C fn dep_b_call() -> i32 from "shared";
        "#;
        let mut program = shape_ast::parser::parse_program(code).expect("parse failed");
        for item in &mut program.items {
            if let shape_ast::ast::Item::ForeignFunction(def, _) = item
                && let Some(native) = def.native_abi.as_mut()
            {
                native.package_key = Some(match def.name.as_str() {
                    "dep_a_call" => "dep_a@1.0.0".to_string(),
                    "dep_b_call" => "dep_b@1.0.0".to_string(),
                    other => panic!("unexpected foreign function '{}'", other),
                });
            }
        }

        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;

        let mut resolutions = shape_runtime::native_resolution::NativeResolutionSet::default();
        resolutions.insert(shape_runtime::native_resolution::ResolvedNativeDependency {
            package_name: "dep_a".to_string(),
            package_version: "1.0.0".to_string(),
            package_key: "dep_a@1.0.0".to_string(),
            alias: "shared".to_string(),
            target: shape_runtime::project::NativeTarget::current(),
            provider: shape_runtime::project::NativeDependencyProvider::System,
            resolved_value: "libdep_a_shared.so".to_string(),
            load_target: "/tmp/libdep_a_shared.so".to_string(),
            fingerprint: "test-a".to_string(),
            declared_version: Some("1.0.0".to_string()),
            cache_key: None,
            provenance: shape_runtime::native_resolution::NativeProvenance::UpdateResolved,
        });
        resolutions.insert(shape_runtime::native_resolution::ResolvedNativeDependency {
            package_name: "dep_b".to_string(),
            package_version: "1.0.0".to_string(),
            package_key: "dep_b@1.0.0".to_string(),
            alias: "shared".to_string(),
            target: shape_runtime::project::NativeTarget::current(),
            provider: shape_runtime::project::NativeDependencyProvider::System,
            resolved_value: "libdep_b_shared.so".to_string(),
            load_target: "/tmp/libdep_b_shared.so".to_string(),
            fingerprint: "test-b".to_string(),
            declared_version: Some("1.0.0".to_string()),
            cache_key: None,
            provenance: shape_runtime::native_resolution::NativeProvenance::UpdateResolved,
        });
        compiler.native_resolution_context = Some(resolutions);

        let bytecode = compiler.compile(&program).expect("compile should succeed");
        let dep_a = bytecode.foreign_functions[0]
            .native_abi
            .as_ref()
            .expect("dep_a native ABI");
        let dep_b = bytecode.foreign_functions[1]
            .native_abi
            .as_ref()
            .expect("dep_b native ABI");

        assert_eq!(dep_a.library, "/tmp/libdep_a_shared.so");
        assert_eq!(dep_b.library, "/tmp/libdep_b_shared.so");
    }

    #[test]
    fn test_out_param_extern_c_compiles() {
        let code = r#"
            extern C fn duckdb_open(path: string, out out_db: ptr) -> i32 from "duckdb";
            extern C fn duckdb_connect(db: ptr, out out_conn: ptr) -> i32 from "duckdb";

            fn test() {
                let [status, db] = duckdb_open("test.db")
                let [s2, conn] = duckdb_connect(db)
                conn
            }
        "#;
        compiles(code).expect("out param extern C should compile");
    }

    #[test]
    fn test_out_param_void_return_single_out() {
        let code = r#"
            extern C fn duckdb_close(out db_p: ptr) -> void from "duckdb";

            fn test() {
                let db = duckdb_close()
                db
            }
        "#;
        // Single out + void return → return type is out value directly
        compiles(code).expect("single out param with void return should compile");
    }

    #[test]
    fn test_out_param_not_allowed_on_non_extern_c() {
        let code = r#"
            fn python test(out x: ptr) -> i32 { "pass" }
        "#;
        let err = compiles(code).expect_err("out params should not work on non-extern-C");
        assert!(
            err.contains("`out` parameter") && err.contains("only valid on `extern C`"),
            "Expected out-param validation error, got: {}",
            err
        );
    }

    #[test]
    fn test_out_param_must_be_ptr_type() {
        let code = r#"
            extern C fn foo(out x: i32) -> void from "lib";
        "#;
        let err = compiles(code).expect_err("out params must be ptr type");
        assert!(
            err.contains("must have type `ptr`"),
            "Expected ptr type error, got: {}",
            err
        );
    }

    #[test]
    fn test_native_builtin_blocked_from_user_code() {
        // Verify that __native_ptr_new_cell is not accessible from user code.
        let code = r#"
            fn test() {
                let cell = __native_ptr_new_cell()
                cell
            }
        "#;
        let compiler = BytecodeCompiler::new();
        // Do NOT set allow_internal_builtins — simulates user code
        let program = shape_ast::parser::parse_program(code).unwrap();
        let err = compiler
            .compile(&program)
            .expect_err("__native_* should be blocked from user code");
        let msg = format!("{}", err);
        assert!(
            msg.contains("'__native_ptr_new_cell' resolves to internal intrinsic scope")
                && msg.contains("not available from ordinary user code"),
            "Expected internal-only intrinsic error, got: {}",
            msg
        );
    }

    #[test]
    fn test_intrinsic_builtin_blocked_from_user_code() {
        // Verify that __intrinsic_* and __json_* builtins are gated from user code.
        // Note: __into_*/__try_into_* are NOT gated (compiler-generated for type assertions).
        for intrinsic in &["__intrinsic_sum", "__intrinsic_mean", "__json_object_get"] {
            let code = format!(
                r#"
                fn test() {{
                    let x = {}([1, 2, 3])
                    x
                }}
            "#,
                intrinsic
            );
            let compiler = BytecodeCompiler::new();
            let program = shape_ast::parser::parse_program(&code).unwrap();
            let err = compiler
                .compile(&program)
                .expect_err(&format!("{} should be blocked from user code", intrinsic));
            let msg = format!("{}", err);
            assert!(
                msg.contains(&format!(
                    "'{}' resolves to internal intrinsic scope",
                    intrinsic
                )) && msg.contains("not available from ordinary user code"),
                "Expected internal-only intrinsic error for {}, got: {}",
                intrinsic,
                msg
            );
        }
    }

    #[test]
    fn test_intrinsic_builtin_method_syntax_blocked_from_user_code() {
        let code = r#"
            fn test() {
                [1, 2, 3].__intrinsic_sum()
            }
        "#;
        let compiler = BytecodeCompiler::new();
        let program = shape_ast::parser::parse_program(code).unwrap();
        let err = compiler
            .compile(&program)
            .expect_err("__intrinsic_* method syntax should be blocked from user code");
        let msg = format!("{}", err);
        assert!(
            msg.contains("'__intrinsic_sum' resolves to internal intrinsic scope")
                && msg.contains("not available from ordinary user code"),
            "Expected internal-only intrinsic method error, got: {}",
            msg
        );
    }

    #[test]
    fn test_unknown_function_message_mentions_resolution_scopes() {
        let code = r#"
            fn test() {
                totally_unknown_function()
            }
        "#;
        let program = shape_ast::parser::parse_program(code).unwrap();
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("unknown function should fail");
        let msg = format!("{}", err);
        assert!(
            msg.contains(
                "Function names resolve from module scope, explicit imports, type-associated scope, and the implicit prelude."
            ),
            "Expected function scope guidance, got: {}",
            msg
        );
    }

    #[test]
    fn test_undefined_variable_message_mentions_resolution_scopes() {
        let code = r#"
            fn test() {
                missing_value
            }
        "#;
        let program = shape_ast::parser::parse_program(code).unwrap();
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("unknown variable should fail");
        let msg = format!("{}", err);
        assert!(
            msg.contains("Variable names resolve from local scope and module scope."),
            "Expected variable scope guidance, got: {}",
            msg
        );
    }

    #[test]
    fn test_internal_builtin_not_unlocked_by_stdlib_name_collision() {
        let code = r#"
            type Json { payload: any }

            extend Json {
                method get(key: string) -> any {
                    __json_object_get(self.payload, key)
                }
            }
        "#;
        let mut compiler = BytecodeCompiler::new();
        compiler
            .stdlib_function_names
            .insert("Json.get".to_string());
        let program = shape_ast::parser::parse_program(code).unwrap();
        let err = compiler
            .compile(&program)
            .expect_err("user-defined Json.get must not gain __* access");
        let msg = format!("{}", err);
        assert!(
            msg.contains("'__json_object_get' resolves to internal intrinsic scope")
                && msg.contains("not available from ordinary user code"),
            "Expected internal-only intrinsic error, got: {}",
            msg
        );
    }

    #[test]
    fn test_compile_function_records_mir_analysis() {
        let program = shape_ast::parser::parse_program(
            r#"
                function choose(flag, left, right) {
                    if flag { left } else { right }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("function should compile");

        let mir = compiler
            .mir_functions
            .get("choose")
            .expect("mir should be recorded");
        assert_eq!(mir.name, "choose");
        assert!(mir.num_locals >= 3, "params should appear in MIR locals");

        let analysis = compiler
            .mir_borrow_analyses
            .get("choose")
            .expect("borrow analysis should be recorded");
        assert_eq!(analysis.loans.len(), 0);
        assert!(analysis.errors.is_empty(), "analysis should be clean");

        // JIT v2: verify mir_data is cached on the Function struct.
        let func_entry = compiler
            .program
            .functions
            .iter()
            .find(|f| f.name == "choose")
            .expect("choose should be in program.functions");
        assert!(
            func_entry.mir_data.is_some(),
            "mir_data should be populated on the Function struct"
        );
        let mir_data = func_entry.mir_data.as_ref().unwrap();
        assert_eq!(mir_data.mir.name, "choose");
    }

    #[test]
    fn test_compile_function_records_return_reference_summary() {
        let program = shape_ast::parser::parse_program(
            r#"
                function borrow_id(&x) {
                    x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("reference-returning function should compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("borrow_id")
            .expect("borrow analysis should be recorded");
        assert_eq!(
            analysis.return_reference_summary,
            Some(crate::mir::analysis::ReturnReferenceSummary {
                param_index: 0,
                kind: crate::mir::types::BorrowKind::Shared,
                projection: Some(Vec::new()),
            })
        );
    }

    #[test]
    fn test_compile_function_allows_expression_return_reference_with_summary() {
        let program = shape_ast::parser::parse_program(
            r#"
                function borrow_id(&x) {
                    let ignored = {
                        return &x
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("expression-form reference return should compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("borrow_id")
            .expect("borrow analysis should be recorded");
        assert_eq!(
            analysis.return_reference_summary,
            Some(crate::mir::analysis::ReturnReferenceSummary {
                param_index: 0,
                kind: crate::mir::types::BorrowKind::Shared,
                projection: Some(Vec::new()),
            })
        );
    }

    #[test]
    fn test_compile_function_rejects_inconsistent_return_reference_summary() {
        let program = shape_ast::parser::parse_program(
            r#"
                function borrow_id(flag, &x) {
                    if flag {
                        return x
                    }
                    return 1
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("mixed ref/value returns should be rejected");
        assert!(
            format!("{}", err).contains("same borrowed origin and borrow kind"),
            "expected inconsistent-ref-return error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("borrow_id")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::InconsistentReferenceReturn),
            "expected inconsistent reference return error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_borrow_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function clash() {
                    let mut x = 1
                    let shared = &x
                    let exclusive = &mut x
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR borrow conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0001"),
            "expected B0001-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("clash")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ConflictSharedExclusive),
            "expected MIR borrow conflict, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_mutability_error() {
        let program = shape_ast::parser::parse_program(
            r#"
                function reassign() {
                    let x = 1
                    x = 2
                    x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("immutable reassignment should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot assign to immutable binding 'x'"),
            "expected immutable binding error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("reassign")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .mutability_errors
                .iter()
                .any(|error| error.variable_name == "x"),
            "expected MIR mutability error, got {:?}",
            analysis.mutability_errors
        );
    }

    // Tests for MIR authority tracking removed: MIR is now the sole authority,
    // there is no longer a lexical fallback mechanism.

    #[test]
    fn test_compile_function_records_mir_const_mutability_error() {
        let program = shape_ast::parser::parse_program(
            r#"
                function reassign() {
                    const x = 1
                    x = 2
                    x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("const reassignment should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot assign to const binding 'x'"),
            "expected const binding error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("reassign")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .mutability_errors
                .iter()
                .any(|error| error.variable_name == "x" && error.is_const),
            "expected MIR const mutability error, got {:?}",
            analysis.mutability_errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_const_param_mutability_error() {
        let program = shape_ast::parser::parse_program(
            r#"
                function reassign(const x) {
                    x = 2
                    x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("const parameter reassignment should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot assign to const binding 'x'"),
            "expected const parameter binding error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("reassign")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .mutability_errors
                .iter()
                .any(|error| error.variable_name == "x" && error.is_const),
            "expected MIR const parameter mutability error, got {:?}",
            analysis.mutability_errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_write_while_borrowed() {
        let program = shape_ast::parser::parse_program(
            r#"
                function reassign() {
                    let mut x = 1
                    let shared = &x
                    x = 2
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR write-while-borrowed should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("reassign")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_read_while_exclusive_borrow() {
        let program = shape_ast::parser::parse_program(
            r#"
                function read_owner() {
                    let mut x = 1
                    let exclusive = &mut x
                    let copy = x
                    exclusive
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR read-while-exclusive should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0001"),
            "expected B0001-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("read_owner")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReadWhileExclusivelyBorrowed),
            "expected MIR read-while-exclusive error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function escape_ref() {
                    let x = 1
                    let r = &x
                    let alias = r
                    return alias
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR reference escape should surface as a compile error");
        assert!(
            format!("{}", err).contains("outlives its owner"),
            "expected reference-escape error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("escape_ref")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceEscape),
            "expected MIR reference-escape error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_use_after_explicit_move() {
        let program = shape_ast::parser::parse_program(
            r#"
                function moved_then_read() {
                    let x = "hi"
                    let y = move x
                    let z = x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR use-after-move should surface as a compile error");
        assert!(
            format!("{}", err).contains("after it was moved"),
            "expected use-after-move error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("moved_then_read")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::UseAfterMove),
            "expected MIR use-after-move error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_async_let_exclusive_ref_task_boundary() {
        let program = shape_ast::parser::parse_program(
            r#"
                async function spawn_conflict() {
                    let mut x = 1
                    async let fut = &mut x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR task-boundary error should surface as a compile error");
        assert!(
            format!("{}", err).contains("task boundary"),
            "expected task-boundary error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("spawn_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected MIR task-boundary error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_async_let_nested_task_boundary() {
        let program = shape_ast::parser::parse_program(
            r#"
                async function compute(a, &mut b, c) {
                    return a
                }
                async function spawn_nested_conflict() {
                    let mut x = 1
                    async let fut = compute(1, &mut x, 3)
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        for item in &program.items {
            if let Item::Function(func, _) = item {
                compiler
                    .register_function(func)
                    .expect("function should register");
            }
        }
        let err = compiler
            .compile_function(func)
            .expect_err("nested MIR task-boundary error should surface as a compile error");
        assert!(
            format!("{}", err).contains("task boundary"),
            "expected task-boundary error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("spawn_nested_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected MIR task-boundary error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_join_task_boundary() {
        let program = shape_ast::parser::parse_program(
            r#"
                async function join_conflict() {
                    let mut x = 1
                    await join all {
                        &mut x,
                        2,
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("join MIR task-boundary error should surface as a compile error");
        assert!(
            format!("{}", err).contains("task boundary"),
            "expected task-boundary error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("join_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ExclusiveRefAcrossTaskBoundary),
            "expected MIR task-boundary error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_closure_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function closure_escape() {
                    let x = 1
                    let r = &x
                    let f = || r
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("non-escaping closure ref capture should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("closure_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "non-escaping closure ref capture should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_array_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function array_escape() {
                    let x = 1
                    let arr = [&x]
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local array ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("array_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local array ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_array_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function indirect_array_escape() {
                    let x = 1
                    let r = &x
                    let arr = [r]
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local indirect array ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_array_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local indirect array ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_object_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function object_escape() {
                    let x = 1
                    let obj = { value: &x }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local object ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("object_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local object ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_object_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function indirect_object_escape() {
                    let x = 1
                    let r = &x
                    let obj = { value: r }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local indirect object ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_object_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local indirect object ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_struct_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                type Point { value: int }

                function struct_escape() {
                    let x = 1
                    let point = Point { value: &x }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_item_with_context(&program.items[0], false)
            .expect("struct type should register");
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local struct ref storage should now compile");
        let analysis = compiler
            .mir_borrow_analyses
            .get("struct_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local struct ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_top_level_object_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                let obj = { value: &x }
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level object reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected top-level object-storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_top_level_array_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                let arr = [&x]
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level array reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an array"),
            "expected top-level array-storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_top_level_reference_cannot_escape_into_closure() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                let r = &x
                let f = || r
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level closure capture should reject escaped references");
        assert!(
            format!("{}", err).contains("[B0003]"),
            "expected top-level closure reference escape error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_top_level_struct_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                type Point { value: int }
                let x = 1
                let point = Point { value: &x }
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level struct reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected top-level struct-storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_function_records_mir_enum_tuple_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                enum Maybe { Value(int), Other }

                function enum_tuple_escape() {
                    let x = 1
                    let value = Maybe::Value(&x)
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_item_with_context(&program.items[0], false)
            .expect("enum should register");
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local enum tuple ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("enum_tuple_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local enum tuple ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_enum_tuple_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                enum Maybe { Value(int), Other }

                function indirect_enum_tuple_escape() {
                    let x = 1
                    let r = &x
                    let value = Maybe::Value(r)
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_item_with_context(&program.items[0], false)
            .expect("enum should register");
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local indirect enum tuple ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_enum_tuple_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local indirect enum tuple ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_enum_struct_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                enum Maybe {
                    Err { code: int }
                }

                function enum_struct_escape() {
                    let x = 1
                    let value = Maybe::Err { code: &x }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[1] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_item_with_context(&program.items[0], false)
            .expect("enum should register");
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local enum struct ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("enum_struct_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local enum struct ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_top_level_enum_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                enum Maybe { Value(int), Other }
                let x = 1
                let value = Maybe::Value(&x)
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level enum reference storage should surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an enum payload"),
            "expected top-level enum-payload error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_function_records_mir_property_assignment_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function property_assignment_escape() {
                    var obj = { value: 0 }
                    let x = 1
                    obj.value = &x
                    0
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local property ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("property_assignment_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local property ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_property_assignment_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function indirect_property_assignment_escape() {
                    var obj = { value: 0 }
                    let x = 1
                    let r = &x
                    obj.value = r
                    0
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local indirect property ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_property_assignment_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local indirect property ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_index_assignment_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function index_assignment_escape() {
                    var arr = [0]
                    let x = 1
                    arr[0] = &x
                    0
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local index ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("index_assignment_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local index ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_indirect_index_assignment_reference_escape() {
        let program = shape_ast::parser::parse_program(
            r#"
                function indirect_index_assignment_escape() {
                    var arr = [0]
                    let x = 1
                    let r = &x
                    arr[0] = r
                    0
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("local indirect index ref storage should now compile");

        let analysis = compiler
            .mir_borrow_analyses
            .get("indirect_index_assignment_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "local indirect index ref storage should now be accepted, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_returning_local_array_with_ref_still_errors() {
        let program = shape_ast::parser::parse_program(
            r#"
                function array_escape() {
                    let x = 1
                    let arr = [&x]
                    return arr
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("returned local array ref storage should still surface as a compile error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an array"),
            "expected returned array-storage error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("array_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceStoredInArray),
            "expected returned array ref storage error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_returning_closure_with_ref_still_errors() {
        let program = shape_ast::parser::parse_program(
            r#"
                function closure_escape() {
                    let x = 1
                    let r = &x
                    let f = || r
                    return f
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("returned closure ref capture should still surface as a compile error");
        assert!(
            format!("{}", err).contains("[B0003]"),
            "expected returned closure escape error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("closure_escape")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::ReferenceEscapeIntoClosure),
            "expected returned closure ref capture error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_top_level_property_assignment_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                var obj = { value: 0 }
                obj.value = &x
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level property assignment reference storage should error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an object or struct literal"),
            "expected top-level object-field storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_top_level_index_assignment_direct_reference_storage_rejected() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                var arr = [0]
                arr[0] = &x
            "#,
        )
        .expect("parse failed");

        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("top-level index assignment reference storage should error");
        assert!(
            format!("{}", err).contains("cannot store a reference in an array"),
            "expected top-level array-element storage error, got {}",
            err
        );
    }

    #[test]
    fn test_compile_function_records_mir_owned_closure_capture() {
        let program = shape_ast::parser::parse_program(
            r#"
                function closure_ok() {
                    let x = 1
                    let f = || x
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("owned closure captures should compile cleanly");

        let analysis = compiler
            .mir_borrow_analyses
            .get("closure_ok")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "owned closure capture should stay borrow-clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_assignment_expr_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function nested_write() {
                    let mut x = 1
                    let shared = &x
                    let y = (x = 2)
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR assignment-expression write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("nested_write")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_if_expression_analysis() {
        let program = shape_ast::parser::parse_program(
            r#"
                function branch_write(flag) {
                    let mut x = 1
                    let shared = if flag { &x } else { &x }
                    x = 2
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("if-expression MIR lowering should stay in the supported subset");

        let analysis = compiler
            .mir_borrow_analyses
            .get("branch_write")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "simple if-expression borrow analysis should stay clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_while_expression_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function while_expr_conflict() {
                    let mut x = 1
                    let y = while true {
                        let shared = &x
                        x = 2
                        shared
                        0
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR while-expression write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("while_expr_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR while-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_for_expression_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function for_expr_conflict(items) {
                    let mut x = 1
                    let y = for item in items {
                        let shared = &x
                        x = 2
                        shared
                        0
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR for-expression write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("for_expr_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR for-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_loop_expression_break_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function loop_expr_conflict() {
                    let mut x = 1
                    let y = loop {
                        let shared = &x
                        x = 2
                        shared
                        break 0
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR loop-expression break write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("loop_expr_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR loop-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_continue_expression_analysis() {
        let program = shape_ast::parser::parse_program(
            r#"
                function continue_expr(flag) {
                    let mut x = 1
                    let y = while flag {
                        if flag { continue } else { x }
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("continue inside while-expression should stay in the supported subset");

        let analysis = compiler
            .mir_borrow_analyses
            .get("continue_expr")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "continue-only while-expression analysis should stay clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_destructure_decl_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function destructure_decl_conflict(pair) {
                    var [left, right] = pair
                    let shared = &left
                    left = 2
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR destructuring declaration write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("destructure_decl_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR destructuring-declaration write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_destructure_param_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function destructure_param_conflict([left, right]) {
                    let mut left_copy = left
                    let shared = &left_copy
                    left_copy = 2
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR destructured-parameter write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("destructure_param_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR destructured-parameter write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_destructure_for_loop_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function destructure_for_conflict(items) {
                    for [left, right] in items {
                        let mut left_copy = left
                        let shared = &left_copy
                        left_copy = 2
                        shared
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR destructuring for-loop write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("destructure_for_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR destructuring for-loop write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_match_expression_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function match_expr_conflict(flag) {
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
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR match-expression write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("match_expr_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR match-expression write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_match_expression_identifier_guard_analysis() {
        let program = shape_ast::parser::parse_program(
            r#"
                function guarded_match(v) {
                    let y = match v {
                        x where x > 0 => x
                        _ => 0
                    }
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        compiler
            .compile_function(func)
            .expect("simple guarded match should stay in the MIR-supported subset");

        let analysis = compiler
            .mir_borrow_analyses
            .get("guarded_match")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "guarded match analysis should stay clean, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_match_expression_array_pattern_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function array_match_conflict(pair) {
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
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR array-pattern match write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("array_match_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR array-pattern match write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_match_expression_constructor_pattern_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function constructor_match_conflict(opt) {
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
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR constructor-pattern match write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("constructor_match_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR constructor-pattern match write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_rest_destructure_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function rest_destructure_conflict(items) {
                    var [head, ...tail] = items
                    let shared = &tail
                    tail = items
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR rest-destructure write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("rest_destructure_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR rest-destructure write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_decomposition_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function decomposition_conflict(merged) {
                    var (left: {x}, right: {y}) = merged
                    let shared = &left
                    left = merged
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler.compile_function(func).expect_err(
            "MIR decomposition-pattern write conflict should surface as a compile error",
        );
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("decomposition_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR decomposition-pattern write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_list_comprehension_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function list_comp_conflict() {
                    let mut x = 1
                    let shared = &x
                    let xs = [(x = 2) for y in [1]]
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR list-comprehension write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("list_comp_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR list-comprehension write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_function_records_mir_from_query_write_conflict() {
        let program = shape_ast::parser::parse_program(
            r#"
                function from_query_conflict() {
                    let mut x = 1
                    let shared = &x
                    let rows = from y in [1] where (x = 2) > 0 select y
                    shared
                }
            "#,
        )
        .expect("parse failed");
        let func = match &program.items[0] {
            Item::Function(func, _) => func,
            _ => panic!("expected function item"),
        };

        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(func)
            .expect("function should register");
        let err = compiler
            .compile_function(func)
            .expect_err("MIR from-query write conflict should surface as a compile error");
        assert!(
            format!("{}", err).contains("B0002"),
            "expected B0002-style error, got {}",
            err
        );

        let analysis = compiler
            .mir_borrow_analyses
            .get("from_query_conflict")
            .expect("borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected MIR from-query write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_removed_function_produces_error_not_stack_overflow() {
        // When a comptime annotation handler removes a function via `remove target`,
        // calling that function should produce a clear compile error, not a stack overflow.
        let code = r#"
            annotation remove_me() {
                targets: [function]
                comptime post(target, ctx) {
                    remove target
                }
            }

            @remove_me()
            fn doomed() {
                42
            }

            doomed()
        "#;
        let result = compiles(code);
        assert!(
            result.is_err(),
            "Calling a removed function should produce a compile error"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("removed"),
            "Error should mention function was removed: {}",
            err_msg
        );
    }

    #[test]
    fn test_removed_function_ref_produces_error() {
        // Referencing a removed function (not calling it) should also error.
        let code = r#"
            annotation remove_me() {
                targets: [function]
                comptime post(target, ctx) {
                    remove target
                }
            }

            @remove_me()
            fn doomed() {
                42
            }

            let f = doomed
        "#;
        let result = compiles(code);
        assert!(
            result.is_err(),
            "Referencing a removed function should produce a compile error"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("removed"),
            "Error should mention function was removed: {}",
            err_msg
        );
    }

    #[test]
    fn test_analyze_non_function_items_records_main_context() {
        let program = shape_ast::parser::parse_program(
            r#"
                let x = 1
                x
            "#,
        )
        .expect("parse failed");

        let mut compiler = BytecodeCompiler::new();
        compiler
            .analyze_non_function_items_with_mir("__main__", &program.items)
            .expect("top-level MIR analysis should succeed");

        // MIR is now the sole authority - no need to check authority flag.
        let analysis = compiler
            .mir_borrow_analyses
            .get("__main__")
            .expect("top-level borrow analysis should be recorded");
        assert!(
            analysis.errors.is_empty(),
            "unexpected top-level MIR errors: {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_analyze_non_function_items_reports_top_level_write_while_borrowed() {
        let program = shape_ast::parser::parse_program(
            r#"
                let mut x = [1]
                let r = &x
                x = [2]
                let y = r
            "#,
        )
        .expect("parse failed");

        let mut compiler = BytecodeCompiler::new();
        let err = compiler
            .analyze_non_function_items_with_mir("__main__", &program.items)
            .expect_err("top-level MIR analysis should reject write-while-borrowed");

        assert!(
            format!("{}", err).contains("[B0002]"),
            "expected MIR top-level borrow diagnostic, got {}",
            err
        );
        let analysis = compiler
            .mir_borrow_analyses
            .get("__main__")
            .expect("top-level borrow analysis should be recorded");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.kind == BorrowErrorKind::WriteWhileBorrowed),
            "expected top-level write-while-borrowed error, got {:?}",
            analysis.errors
        );
    }

    #[test]
    fn test_compile_reports_top_level_mir_borrow_error() {
        // Direct call to analyze_non_function_items_with_mir validates that
        // the MIR analysis correctly detects borrow violations in top-level code.
        // (Not yet wired into the compilation pipeline due to false positives
        // on method chains.)
        let source = r#"
                let mut x = [1]
                let r = &x
                x = [2]
                let y = r
            "#;
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let mut compiler = BytecodeCompiler::new();
        let result = compiler.analyze_non_function_items_with_mir("__main__", &program.items);

        assert!(result.is_err(), "expected top-level compile error");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("B0002"),
            "expected top-level MIR borrow diagnostic, got {}",
            err
        );
    }

    #[test]
    fn test_compile_reports_module_body_mir_borrow_error() {
        // Direct call to analyze_non_function_items_with_mir validates that
        // the MIR analysis correctly detects borrow violations in module-level code.
        let source = r#"
                let mut x = [1]
                let r = &x
                x = [2]
                let y = r
            "#;
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let mut compiler = BytecodeCompiler::new();
        let result = compiler.analyze_non_function_items_with_mir("__module__", &program.items);

        assert!(result.is_err(), "expected module-body compile error");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("B0002"),
            "expected module-body MIR borrow diagnostic, got {}",
            err
        );
    }

    #[test]
    fn test_interprocedural_alias_summary_extracted() {
        let code = r#"
            function touch(a, b) {
                a[0] = 1
                return b[0]
            }
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        if let Item::Function(func, _) = &program.items[0] {
            compiler.register_function(func).expect("register");
            compiler.compile_function(func).expect("compile touch");
        }
        let summary = compiler
            .function_borrow_summaries
            .get("touch")
            .expect("touch should have a borrow summary");
        assert!(
            !summary.conflict_pairs.is_empty(),
            "touch should have conflict pairs: mutated param 0 vs read param 1"
        );
    }

    // =========================================================================
    // Composable return reference summary integration tests
    // =========================================================================

    #[test]
    fn test_composable_return_reference_summary() {
        // fn identity(&x) { x }
        // fn wrapper(&y) { identity(y) }
        // wrapper should have return_summary tracing to param 0
        let code = r#"
fn identity(&x) { x }
fn wrapper(&y) { identity(y) }
"#;
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        // Register both functions first (two-pass)
        for item in &program.items {
            if let Item::Function(func, _) = item {
                compiler.register_function(func).expect("register");
            }
        }
        // Compile in order: identity first, then wrapper
        for item in &program.items {
            if let Item::Function(func, _) = item {
                compiler.compile_function(func).expect("compile");
            }
        }

        let summary = compiler
            .function_borrow_summaries
            .get("wrapper")
            .expect("wrapper should have a borrow summary");
        assert!(
            summary.return_summary.is_some(),
            "wrapper should have a return_summary from composed identity call"
        );
        let ret = summary.return_summary.as_ref().unwrap();
        assert_eq!(ret.param_index, 0, "should trace to wrapper's param 0");
    }

    #[test]
    fn test_composable_return_summary_local_shadow_conservative() {
        // Global fn foo(&x) { x }, then bar defines local closure `foo` that
        // shadows the global. The call `foo(y)` in bar should NOT get composed
        // return summary from global foo.
        let code = r#"
fn foo(&x) { x }
fn bar(&y) {
    let foo = |z| { z }
    foo(y)
}
"#;
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        for item in &program.items {
            if let Item::Function(func, _) = item {
                compiler.register_function(func).expect("register");
            }
        }
        for item in &program.items {
            if let Item::Function(func, _) = item {
                compiler.compile_function(func).expect("compile");
            }
        }

        // bar should NOT have a composed return summary from global foo,
        // because the local closure `foo` shadows it
        let has_composed = compiler
            .function_borrow_summaries
            .get("bar")
            .and_then(|s| s.return_summary.as_ref())
            .is_some();
        assert!(
            !has_composed,
            "local shadow should prevent composition with global foo"
        );
    }

    #[test]
    fn test_composable_return_summary_module_binding_shadow() {
        // A module binding named "foo" should prevent composition with a
        // global function "foo". This exercises the
        // resolve_scoped_module_binding_name() check in build_callee_summaries().
        let code = r#"
fn foo(&x) { x }
fn bar(&y) { foo(y) }
"#;
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.allow_internal_builtins = true;
        for item in &program.items {
            if let Item::Function(func, _) = item {
                compiler.register_function(func).expect("register");
            }
        }
        // Compile foo first so it gets a return_summary
        for item in &program.items {
            if let Item::Function(func, _) = item {
                compiler.compile_function(func).expect("compile");
            }
        }
        // Sanity: without module binding shadow, bar DOES get a composed summary
        assert!(
            compiler
                .function_borrow_summaries
                .get("bar")
                .and_then(|s| s.return_summary.as_ref())
                .is_some(),
            "bar should have composed summary before module binding shadow"
        );

        // Now simulate a module binding named "foo" and recompile bar.
        // This mimics `import { foo } from some_module` shadowing global foo.
        compiler.module_bindings.insert("foo".to_string(), 999);
        // Re-register and recompile bar
        if let Item::Function(func, _) = &program.items[1] {
            compiler.register_function(func).expect("re-register bar");
            compiler.compile_function(func).expect("recompile bar");
        }
        let has_composed = compiler
            .function_borrow_summaries
            .get("bar")
            .and_then(|s| s.return_summary.as_ref())
            .is_some();
        assert!(
            !has_composed,
            "module binding shadow should prevent composition with global foo"
        );
    }
}
