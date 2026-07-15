//! Structural inference facts for comptime-generated closure nodes.
//!
//! Source spans are not identities for generated syntax: separate expansions
//! can parse from the same byte offsets. This leaf records closure inference
//! under compiler-issued structural provenance and retains conflicts instead
//! of letting hash-map insertion order select a type.

use super::TypeInferenceEngine;
use crate::closure::EnvironmentAnalyzer;
use crate::type_system::{BindingToken, Type};
use shape_ast::ast::{
    Expr, FunctionDef, FunctionParameter, GeneratedNodeOrigin, TypeAnnotation, VariableDecl,
};
use std::collections::HashMap;

mod callsite;
mod declaration;
mod handoff;
mod model;
mod semantic_candidate;
mod state;
mod substitution;
pub(super) use callsite::{
    DeclaredMethodTypeParameters, InferenceMethodDeclarationToken, SemanticCallSiteCandidate,
};
pub use callsite::{
    ExactSemanticCallSiteFact, SemanticCallSiteFact, SemanticCallSiteKey,
    SemanticCalleeDeclaration, SemanticDeclaredParameter, SemanticTypeArgument,
};
pub(super) use model::{
    GeneratedCallableCandidate, InferenceCallableDeclarationToken, SemanticCandidateObservation,
};
pub use model::{
    GeneratedCallableFact, GeneratedCaptureFact, GeneratedCaptureKey, GeneratedNodeKey,
    GeneratedSemanticFactIssue,
};
pub(super) use semantic_candidate::type_is_semantically_resolved;
pub use semantic_candidate::{
    RecursiveCallableShape, SemanticCallableNodeShape, SemanticCallableParameterShape,
    SemanticPassingMode, SemanticTypeCandidate, SemanticTypePathSegment,
};
pub(super) use state::GeneratedInferenceState;
pub(super) use substitution::{SemanticProjectionIssue, project_declared_argument_candidates};

impl TypeInferenceEngine {
    /// Resolve an identifier initializer before its new binding is published.
    /// This preserves the source token through nested shadowing.
    pub(crate) fn binding_semantic_source_token(
        &self,
        decl: &VariableDecl,
    ) -> Option<BindingToken> {
        match decl.value.as_ref() {
            Some(Expr::Identifier(source, _)) => self.env.lookup_binding_token(source),
            _ => None,
        }
    }

    /// Attach recursive callable metadata to the opaque token minted for a
    /// checked lexical binding. This is called after `TypeEnvironment::define`;
    /// the token, not name/depth/span, is the lookup authority.
    pub(crate) fn record_binding_semantic_candidate(
        &mut self,
        token: BindingToken,
        source_token: Option<BindingToken>,
        decl: &VariableDecl,
        ty: &Type,
    ) {
        let Some(_name) = decl.pattern.as_identifier() else {
            return;
        };
        let observations = match decl.value.as_ref() {
            Some(Expr::FunctionExpr {
                params,
                return_type,
                ..
            }) => vec![
                SemanticTypeCandidate::generated_callable(ty.clone(), params, return_type.as_ref())
                    .map(SemanticCandidateObservation::Candidate)
                    .unwrap_or_else(unavailable_observation),
            ],
            _ if decl.type_annotation.is_some() => vec![
                SemanticTypeCandidate::annotated_binding(
                    ty.clone(),
                    decl.type_annotation
                        .as_ref()
                        .expect("guarded declaration annotation"),
                )
                .map(SemanticCandidateObservation::Candidate)
                .unwrap_or_else(unavailable_observation),
            ],
            Some(Expr::Identifier(_, _)) => source_token
                .and_then(|source| self.generated_inference.binding_candidates.get(&source))
                .map(|observations| {
                    observations
                        .iter()
                        .cloned()
                        .map(|observation| observation.with_type(ty.clone()))
                        .collect()
                })
                .unwrap_or_else(|| vec![binding_type_observation(ty.clone())]),
            _ => vec![binding_type_observation(ty.clone())],
        };
        self.record_binding_semantic_observations(token, observations);
    }

    fn record_binding_semantic_observations(
        &mut self,
        token: BindingToken,
        observations: impl IntoIterator<Item = SemanticCandidateObservation>,
    ) {
        self.generated_inference
            .binding_candidates
            .entry(token)
            .or_default()
            .extend(observations);
    }

    /// Record one generated closure's synthesized pre-substitution type.
    pub(super) fn record_generated_callable_candidate(&mut self, expr: &Expr, ty: &Type) {
        let Expr::FunctionExpr {
            params,
            return_type,
            generated_origin: Some(origin),
            ..
        } = expr
        else {
            return;
        };
        let observation =
            SemanticTypeCandidate::generated_callable(ty.clone(), params, return_type.as_ref())
                .map(SemanticCandidateObservation::Candidate)
                .unwrap_or_else(|detail| {
                    SemanticCandidateObservation::Unavailable(GeneratedSemanticFactIssue::new(
                        detail,
                    ))
                });
        self.generated_inference
            .callable_candidates
            .entry(GeneratedNodeKey::from_origin(origin))
            .or_default()
            .push(GeneratedCallableCandidate { observation });
    }

    /// Record the generated closure's outer binding types before its own
    /// parameter scope is pushed, then make its structural node key available
    /// to generic calls nested in the body.
    pub(super) fn enter_generated_function_fact_scope(
        &mut self,
        origin: Option<&GeneratedNodeOrigin>,
        params: &[FunctionParameter],
        return_type: Option<&TypeAnnotation>,
        body: &[shape_ast::ast::Statement],
    ) -> crate::type_system::TypeResult<Option<GeneratedNodeKey>> {
        let Some(origin) = origin else {
            return Ok(None);
        };
        let node = GeneratedNodeKey::from_origin(origin);
        self.record_generated_capture_candidates(&node, params, return_type, body)?;
        self.generated_inference
            .active_node_stack
            .push(node.clone());
        Ok(Some(node))
    }

    pub(super) fn exit_generated_function_fact_scope(
        &mut self,
        entered: Option<&GeneratedNodeKey>,
    ) -> crate::type_system::TypeResult<()> {
        let Some(entered) = entered else {
            return Ok(());
        };
        let popped = self.generated_inference.active_node_stack.pop();
        if popped.as_ref() != Some(entered) {
            self.generated_inference.active_node_stack.clear();
            return Err(crate::type_system::TypeError::ConstraintViolation(
                "internal inference error: generated semantic-fact scope stack is unbalanced"
                    .to_string(),
            ));
        }
        Ok(())
    }

    fn record_generated_capture_candidates(
        &mut self,
        node: &GeneratedNodeKey,
        params: &[FunctionParameter],
        return_type: Option<&TypeAnnotation>,
        body: &[shape_ast::ast::Statement],
    ) -> crate::type_system::TypeResult<()> {
        let function = FunctionDef {
            name: "__generated_capture_fact_subject".to_string(),
            name_span: shape_ast::ast::Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: return_type.cloned(),
            body: body.to_vec(),
            annotations: Vec::new(),
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };
        let outer_names = self.env.visible_binding_names();
        let analysis = EnvironmentAnalyzer::analyze_function_captures(&function, &outer_names);

        for (ordinal, name) in analysis.captured_vars().iter().enumerate() {
            let ordinal = u16::try_from(ordinal).map_err(|_| {
                crate::type_system::TypeError::ConstraintViolation(
                    "generated closure capture ordinal exceeds the semantic-fact range".to_string(),
                )
            })?;
            let observations = match self.env.lookup(name) {
                None => vec![SemanticCandidateObservation::Unavailable(
                    GeneratedSemanticFactIssue::new(format!(
                        "captured binding '{name}' has no outer inference scheme"
                    )),
                )],
                Some(scheme) if !scheme.quantified.is_empty() => {
                    vec![SemanticCandidateObservation::Unavailable(
                        GeneratedSemanticFactIssue::new(format!(
                            "captured binding '{name}' is polymorphic and has no monomorphic value type"
                        )),
                    )]
                }
                Some(scheme) => self
                    .env
                    .lookup_binding_token(name)
                    .and_then(|token| self.generated_inference.binding_candidates.get(&token))
                    .cloned()
                    .unwrap_or_else(|| vec![binding_type_observation(scheme.ty.clone())]),
            };
            self.generated_inference
                .capture_candidates
                .entry(GeneratedCaptureKey::new(node.clone(), ordinal))
                .or_default()
                .extend(observations);
        }
        Ok(())
    }

    pub(super) fn clear_generated_callable_facts(&mut self) {
        self.generated_inference = Default::default();
    }

    /// Apply the run's final substitutions to every structural observation.
    /// Duplicate keys are reduced only after substitution, so fresh variables
    /// that resolve identically do not create false conflicts.
    pub(super) fn finalize_generated_callable_facts(&mut self) {
        let candidates = std::mem::take(&mut self.generated_inference.callable_candidates);
        self.generated_inference.callable_facts = candidates
            .into_iter()
            .map(|(key, candidates)| {
                let observations = candidates
                    .into_iter()
                    .map(|candidate| candidate.observation)
                    .collect();
                let fact = match self.reduce_semantic_observations(observations) {
                    ReducedSemanticFact::Exact(candidate)
                        if matches!(candidate.ty(), Type::Function { .. }) =>
                    {
                        GeneratedCallableFact::Exact(candidate)
                    }
                    ReducedSemanticFact::Exact(_) => {
                        GeneratedCallableFact::Unavailable(GeneratedSemanticFactIssue::new(
                            "generated callable fact finalized to a non-callable type",
                        ))
                    }
                    ReducedSemanticFact::Unavailable(issue) => {
                        GeneratedCallableFact::Unavailable(issue)
                    }
                    ReducedSemanticFact::Conflict(issue) => GeneratedCallableFact::Conflict(issue),
                };
                (key, fact)
            })
            .collect();

        let candidates = std::mem::take(&mut self.generated_inference.capture_candidates);
        self.generated_inference.capture_facts = candidates
            .into_iter()
            .map(|(key, observations)| {
                let fact = match self.reduce_semantic_observations(observations) {
                    ReducedSemanticFact::Exact(candidate) => GeneratedCaptureFact::Exact(candidate),
                    ReducedSemanticFact::Unavailable(issue) => {
                        GeneratedCaptureFact::Unavailable(issue)
                    }
                    ReducedSemanticFact::Conflict(issue) => GeneratedCaptureFact::Conflict(issue),
                };
                (key, fact)
            })
            .collect();
        self.generated_inference.binding_candidates.clear();
        self.generated_inference.callable_binding_tokens.clear();
        self.generated_inference
            .callable_declared_parameters
            .clear();
        self.generated_inference.active_node_stack.clear();
    }

    fn reduce_semantic_observations(
        &self,
        observations: Vec<SemanticCandidateObservation>,
    ) -> ReducedSemanticFact {
        let mut exact: Option<SemanticTypeCandidate> = None;
        let mut unavailable = Vec::new();
        let mut conflict = false;
        for observation in observations {
            let candidate = match observation {
                SemanticCandidateObservation::Unavailable(issue) => {
                    unavailable.push(issue.detail);
                    continue;
                }
                SemanticCandidateObservation::Conflict(issue) => {
                    return ReducedSemanticFact::Conflict(issue);
                }
                SemanticCandidateObservation::Candidate(candidate) => candidate,
            };
            let resolved = self.solver.unifier().apply_substitutions(candidate.ty());
            if !type_is_semantically_resolved(&resolved, true) {
                unavailable.push(
                    "semantic type retained an unresolved inference variable after solving"
                        .to_string(),
                );
                continue;
            }
            let candidate = match candidate.with_resolved_type(resolved) {
                Ok(candidate) => candidate,
                Err(detail) => {
                    unavailable.push(detail);
                    continue;
                }
            };
            match &exact {
                None => exact = Some(candidate),
                Some(previous) if *previous == candidate => {}
                Some(_) => conflict = true,
            }
        }
        if conflict {
            ReducedSemanticFact::Conflict(GeneratedSemanticFactIssue::new(
                "structurally identical observations finalized to different semantic candidates",
            ))
        } else if !unavailable.is_empty() {
            unavailable.sort();
            unavailable.dedup();
            ReducedSemanticFact::Unavailable(GeneratedSemanticFactIssue::new(
                unavailable.join("; "),
            ))
        } else if let Some(candidate) = exact {
            ReducedSemanticFact::Exact(candidate)
        } else {
            ReducedSemanticFact::Unavailable(GeneratedSemanticFactIssue::new(
                "no semantic inference observation was recorded",
            ))
        }
    }

    pub(super) fn take_generated_callable_facts(
        &mut self,
    ) -> HashMap<GeneratedNodeKey, GeneratedCallableFact> {
        std::mem::take(&mut self.generated_inference.callable_facts)
    }

    pub(super) fn take_generated_capture_facts(
        &mut self,
    ) -> HashMap<GeneratedCaptureKey, GeneratedCaptureFact> {
        std::mem::take(&mut self.generated_inference.capture_facts)
    }
}

enum ReducedSemanticFact {
    Exact(SemanticTypeCandidate),
    Unavailable(GeneratedSemanticFactIssue),
    Conflict(GeneratedSemanticFactIssue),
}

fn unavailable_observation(detail: String) -> SemanticCandidateObservation {
    SemanticCandidateObservation::Unavailable(GeneratedSemanticFactIssue::new(detail))
}

fn binding_type_observation(ty: Type) -> SemanticCandidateObservation {
    SemanticTypeCandidate::monomorphic_binding(ty)
        .map(SemanticCandidateObservation::Candidate)
        .unwrap_or_else(unavailable_observation)
}

#[cfg(test)]
mod tests;
