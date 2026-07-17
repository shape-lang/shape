//! Canonical MIR, borrow, and storage analysis for one function body.
//!
//! Normal function compilation and compiler-issued hygienic emissions must
//! consume the same analysis pipeline. Keeping that pipeline here makes it
//! possible to analyze an untouched semantic body before emitting an identical
//! body under a different, unspellable function identity.

use super::*;

impl BytecodeCompiler {
    /// Analyze `function` under its semantic identity and publish the complete
    /// MIR-derived fact bundle keyed by that identity.
    pub(in crate::compiler) fn analyze_function_body(
        &mut self,
        function: &FunctionDef,
    ) -> Result<()> {
        let fn_return_types = self.build_fn_return_type_seed();
        let unit_variant_names = self.build_unit_variant_name_seed();
        let lowering = crate::mir::lowering::lower_function_detailed_with_returns_and_variants(
            &function.name,
            &function.params,
            &function.body,
            function.name_span,
            fn_return_types,
            unit_variant_names,
        );
        let callee_summaries =
            self.build_callee_summaries(Some(&function.name), &lowering.all_local_names);
        let options = crate::mir::solver::BorrowAnalysisOptions {
            allow_return_slot_local_escape_promotion: function
                .return_type
                .as_ref()
                .is_some_and(|ann| matches!(ann, shape_ast::ast::TypeAnnotation::Borrow { .. })),
        };
        let mut analysis =
            crate::mir::solver::analyze_with_options(&lowering.mir, &callee_summaries, options);
        analysis.mutability_errors = crate::mir::lowering::compute_mutability_errors(&lowering);
        crate::mir::repair::attach_repairs(&mut analysis, &lowering.mir);

        // MIR is the sole authority. When lowering used a fallback, suppress
        // only diagnostics whose own span overlaps that fallback.
        let first_mutability_error = if lowering.fallback_spans.is_empty() {
            analysis.mutability_errors.first().cloned()
        } else {
            analysis
                .mutability_errors
                .iter()
                .find(|error| !Self::span_overlaps_any(&error.span, &lowering.fallback_spans))
                .cloned()
        };
        let first_borrow_error = if lowering.fallback_spans.is_empty() {
            analysis.errors.first().cloned()
        } else {
            analysis
                .errors
                .iter()
                .find(|error| !Self::span_overlaps_any(&error.span, &lowering.fallback_spans))
                .cloned()
        };

        if let Some(summary) = analysis.return_reference_summary.clone() {
            self.function_return_reference_summaries
                .insert(function.name.clone(), summary.into());
        } else {
            self.function_return_reference_summaries
                .remove(&function.name);
        }

        let (closure_captures, mutable_captures) =
            crate::mir::storage_planning::collect_closure_captures(&lowering.mir);
        let mut binding_semantics = HashMap::new();
        for slot_idx in 0..lowering.mir.num_locals {
            if let Some(semantics) = self.type_tracker.get_local_binding_semantics(slot_idx) {
                binding_semantics.insert(slot_idx, *semantics);
            }
        }
        let storage_plan = crate::mir::storage_planning::plan_storage(
            &crate::mir::storage_planning::StoragePlannerInput {
                mir: &lowering.mir,
                analysis: &analysis,
                binding_semantics: &binding_semantics,
                closure_captures: &closure_captures,
                mutable_captures: &mutable_captures,
                had_fallbacks: lowering.had_fallbacks,
                callee_summaries: Some(&self.function_borrow_summaries),
            },
        );
        self.mir_storage_plans
            .insert(function.name.clone(), storage_plan);

        let field_cfg = crate::mir::cfg::ControlFlowGraph::build(&lowering.mir);
        let mut field_analysis = crate::mir::field_analysis::analyze_fields(
            &crate::mir::field_analysis::FieldAnalysisInput {
                mir: &lowering.mir,
                cfg: &field_cfg,
            },
        );
        for (slot_id, field_indices) in &field_analysis.hoisted_fields {
            let recommendations: Vec<(crate::mir::FieldIdx, String)> = field_indices
                .iter()
                .filter(|idx| !field_analysis.dead_fields.contains(&(*slot_id, **idx)))
                .filter_map(|idx| {
                    lowering
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
        for (slot_id, field_indices) in &field_analysis.hoisted_fields {
            if let Some(binding) = lowering
                .binding_infos
                .iter()
                .find(|binding| binding.slot == *slot_id)
            {
                let field_names: Vec<String> = field_indices
                    .iter()
                    .filter(|idx| !field_analysis.dead_fields.contains(&(*slot_id, **idx)))
                    .filter_map(|idx| lowering.field_names.get(idx))
                    .cloned()
                    .collect();
                if !field_names.is_empty() {
                    self.hoisted_fields
                        .insert(binding.name.clone(), field_names);
                }
            }
        }
        self.mir_field_analyses
            .insert(function.name.clone(), field_analysis);

        let mut span_to_point = HashMap::new();
        for block in lowering.mir.iter_blocks() {
            for statement in &block.statements {
                span_to_point
                    .entry(statement.span)
                    .or_insert(statement.point);
            }
        }
        self.mir_span_to_point
            .insert(function.name.clone(), span_to_point);

        let callee_return_modes = self.build_callee_return_modes(Some(&function.name));
        let borrow_summary = crate::mir::solver::extract_borrow_summary_with_callees(
            &lowering.mir,
            analysis.return_reference_summary.clone(),
            &callee_return_modes,
        );
        let has_informative_summary = !borrow_summary.conflict_pairs.is_empty()
            || borrow_summary.return_summary.is_some()
            || borrow_summary.return_ownership_mode != crate::mir::ReturnOwnershipMode::Unknown
            || borrow_summary
                .closure_param_escapes
                .iter()
                .any(|escapes| !escapes);
        if has_informative_summary {
            self.function_borrow_summaries
                .insert(function.name.clone(), borrow_summary);
        } else {
            self.function_borrow_summaries.remove(&function.name);
        }

        let alias_errors = self.check_call_site_aliasing(&lowering.mir, &lowering.fallback_spans);
        let first_alias_error = alias_errors.first().cloned();
        analysis.errors.extend(alias_errors);

        self.mir_functions
            .insert(function.name.clone(), lowering.mir);
        self.mir_borrow_analyses
            .insert(function.name.clone(), analysis);
        if let Some(error) = first_mutability_error.as_ref() {
            return Err(self.mir_mutability_error(error));
        }
        if let Some(error) = first_borrow_error.as_ref() {
            return Err(self.mir_borrow_error(error));
        }
        if let Some(error) = first_alias_error.as_ref() {
            return Err(self.mir_borrow_error(error));
        }
        Ok(())
    }
}
