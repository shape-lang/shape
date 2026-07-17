//! Non-emitting closure-layout peek used by monomorphization.

use std::collections::BTreeSet;

use shape_ast::ast::{
    CaptureClause, FunctionDef, FunctionParameter, GeneratedNodeOrigin, Span, Statement,
};
use shape_runtime::closure::EnvironmentAnalyzer;
use shape_value::v2::concrete_type::ClosureTypeId;

use crate::compiler::BytecodeCompiler;

use super::collect_static_mut_self_container_captures;

impl BytecodeCompiler {
    /// Peek and intern a closure capture signature without emitting bytecode or
    /// adding an entry to `closure_type_ids`.
    ///
    /// The canonical surface gate and capture planner are shared with ordinary
    /// emission. An invalid declaration returns `None`, allowing emission to
    /// report the diagnostic without minting an inferred stand-in layout.
    pub(crate) fn mint_closure_type_id_peek(
        &mut self,
        params: &[FunctionParameter],
        body: &[Statement],
        declared: Option<&CaptureClause>,
        generated_origin: Option<&GeneratedNodeOrigin>,
        closure_span: Span,
    ) -> Option<ClosureTypeId> {
        let proto_def = FunctionDef {
            name: "__peek_closure__".to_string(),
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
        let analysis = EnvironmentAnalyzer::analyze_function_captures(&proto_def, &outer_vars);
        let mut captured_vars = analysis.captured_vars().to_vec();
        let mut mutated_captures = analysis.mutated_captures().clone();
        captured_vars.sort();
        let param_names: BTreeSet<String> = params
            .iter()
            .flat_map(|param| param.get_identifiers())
            .collect();
        captured_vars.retain(|name| !param_names.contains(name));

        // A peek is not permission to bypass the generated-only surface gate.
        let generated_origin = self
            .validate_capture_surface(declared, generated_origin, &captured_vars, closure_span)
            .ok()?;
        let captured_var_set: BTreeSet<String> = captured_vars.iter().cloned().collect();
        mutated_captures.extend(collect_static_mut_self_container_captures(
            self,
            body,
            &captured_var_set,
        ));

        // Match emission's callable-capture classification exactly.
        let saved_callee_captures = std::mem::replace(
            &mut self.current_closure_callee_captures,
            Self::collect_callee_identifier_names(body),
        );
        let plan = match self.plan_captures(
            &captured_vars,
            &mutated_captures,
            Some(&analysis),
            declared,
            generated_origin,
            closure_span,
        ) {
            Ok(plan) => plan,
            Err(_) => {
                self.current_closure_callee_captures = saved_callee_captures;
                return None;
            }
        };
        // Speculative specialization is not permission to pre-publish the
        // layout for a module-cell effect that real callable emission will
        // reject. Decline the peek; the ordinary compile path reports C0912.
        if self
            .preflight_callable_module_shared_captures(&plan, closure_span)
            .is_err()
        {
            self.current_closure_callee_captures = saved_callee_captures;
            return None;
        }

        let user_pass_modes = self.effective_function_like_pass_modes(None, params, Some(body));
        let callable_semantic_evidence =
            self.callable_semantic_evidence(generated_origin, params, &user_pass_modes);
        let pack = match self.build_capture_pack(
            u16::MAX,
            &plan,
            generated_origin,
            callable_semantic_evidence,
        ) {
            Ok(pack) => pack,
            Err(_) => {
                self.current_closure_callee_captures = saved_callee_captures;
                return None;
            }
        };
        let id = self.intern_closure_type_id_for_pack(&pack);
        self.current_closure_callee_captures = saved_callee_captures;
        Some(id)
    }
}
