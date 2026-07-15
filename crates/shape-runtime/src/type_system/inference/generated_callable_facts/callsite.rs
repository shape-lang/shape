//! Transient structural lookup facts for exact generic call-site arguments.
//!
//! These keys associate inference with bytecode emission. They are never
//! serialized, rendered, or used as semantic type identity; SemanticFreeze
//! supplies that identity after the facts cross into the compiler.

use std::collections::HashMap;

use shape_ast::ast::Span;

use super::{
    GeneratedNodeKey, GeneratedSemanticFactIssue, SemanticCandidateObservation,
    SemanticProjectionIssue, SemanticTypeCandidate, project_declared_argument_candidates,
    type_is_semantically_resolved,
};
use crate::type_system::types::core::DeclaredTypeVarInstantiation;
use crate::type_system::{TypeInferenceEngine, TypeVar};

mod declaration_catalog;
mod method;
pub use declaration_catalog::{SemanticCalleeDeclaration, SemanticDeclaredParameter};
pub(super) use method::{DeclaredMethodTypeParameters, InferenceMethodDeclarationToken};

/// Inference-unit lookup identity for one generic call expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemanticCallSiteKey {
    enclosing_generated_node: Option<GeneratedNodeKey>,
    callee: String,
    call_span: Span,
}

impl SemanticCallSiteKey {
    #[must_use]
    pub fn new(
        enclosing_generated_node: Option<GeneratedNodeKey>,
        callee: impl Into<String>,
        call_span: Span,
    ) -> Self {
        Self {
            enclosing_generated_node,
            callee: callee.into(),
            call_span,
        }
    }

    #[must_use]
    pub fn enclosing_generated_node(&self) -> Option<&GeneratedNodeKey> {
        self.enclosing_generated_node.as_ref()
    }

    #[must_use]
    pub fn callee(&self) -> &str {
        &self.callee
    }

    #[must_use]
    pub fn call_span(&self) -> Span {
        self.call_span
    }
}

/// One declared generic parameter's exact semantic argument at a call site.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticTypeArgument {
    declared: TypeVar,
    ordinal: u32,
    source_name: String,
    candidate: SemanticTypeCandidate,
}

impl SemanticTypeArgument {
    #[must_use]
    pub fn declared(&self) -> &TypeVar {
        &self.declared
    }

    #[must_use]
    pub fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    #[must_use]
    pub fn candidate(&self) -> &SemanticTypeCandidate {
        &self.candidate
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SemanticCallSiteFact {
    Exact(ExactSemanticCallSiteFact),
    Unavailable(GeneratedSemanticFactIssue),
    Conflict(GeneratedSemanticFactIssue),
}

/// Sealed exact call-site evidence. Its expected declaration capability and
/// projected arguments are issued together by one inference reduction, so
/// downstream code cannot mix a public argument vector with a foreign owner.
#[derive(Debug, Clone, PartialEq)]
pub struct ExactSemanticCallSiteFact {
    declaration: SemanticCalleeDeclaration,
    arguments: Vec<SemanticTypeArgument>,
}

impl ExactSemanticCallSiteFact {
    fn new(arguments: Vec<SemanticTypeArgument>) -> Self {
        let declaration = SemanticCalleeDeclaration::from_arguments(&arguments);
        Self {
            declaration,
            arguments,
        }
    }

    #[must_use]
    pub fn declaration(&self) -> &SemanticCalleeDeclaration {
        &self.declaration
    }

    #[must_use]
    pub fn arguments(&self) -> &[SemanticTypeArgument] {
        &self.arguments
    }
}

impl SemanticCallSiteFact {
    #[must_use]
    pub fn exact(&self) -> Option<&ExactSemanticCallSiteFact> {
        match self {
            Self::Exact(exact) => Some(exact),
            Self::Unavailable(_) | Self::Conflict(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct SemanticCallSiteCandidate {
    pub(super) instantiations: Vec<SemanticDeclaredInstantiation>,
    pub(super) parameter_types: Option<Vec<crate::type_system::Type>>,
    /// One post-solve-reduced observation history per actual argument.
    pub(super) arguments: Option<Vec<Vec<SemanticCandidateObservation>>>,
}

#[derive(Debug, Clone)]
pub(super) struct SemanticDeclaredInstantiation {
    declared: TypeVar,
    instantiated: TypeVar,
}

impl SemanticDeclaredInstantiation {
    pub(super) fn new(declared: TypeVar, instantiated: TypeVar) -> Self {
        Self {
            declared,
            instantiated,
        }
    }

    fn declared(&self) -> &TypeVar {
        &self.declared
    }

    fn instantiated(&self) -> &TypeVar {
        &self.instantiated
    }
}

impl From<&DeclaredTypeVarInstantiation> for SemanticDeclaredInstantiation {
    fn from(instantiation: &DeclaredTypeVarInstantiation) -> Self {
        Self::new(
            instantiation.declared().clone(),
            instantiation.instantiated().clone(),
        )
    }
}

impl TypeInferenceEngine {
    /// Receiver invoked by named-call inference immediately after a bounded
    /// scheme is instantiated. Resolution is deferred until the solver has
    /// finalized every fresh instance variable.
    pub(crate) fn record_declared_type_instantiations(
        &mut self,
        name: &str,
        call_span: Span,
        instantiations: &[DeclaredTypeVarInstantiation],
    ) {
        if instantiations.is_empty() {
            return;
        }
        let key = SemanticCallSiteKey::new(
            self.generated_inference.active_node_stack.last().cloned(),
            name,
            call_span,
        );
        self.generated_inference
            .callsite_candidates
            .entry(key)
            .or_default()
            .push(SemanticCallSiteCandidate {
                instantiations: instantiations.iter().map(Into::into).collect(),
                parameter_types: None,
                arguments: None,
            });
    }

    /// Attach the checked actual arguments and instantiated parameter shape to
    /// the most recent provenance observation for this exact call node.
    pub(crate) fn record_semantic_call_arguments(
        &mut self,
        name: &str,
        call_span: Span,
        args: &[shape_ast::ast::Expr],
        arg_types: &[crate::type_system::Type],
        instantiated_function: &crate::type_system::Type,
    ) {
        let key = SemanticCallSiteKey::new(
            self.generated_inference.active_node_stack.last().cloned(),
            name,
            call_span,
        );
        let parameter_types = match instantiated_function {
            crate::type_system::Type::Function { params, .. } => Some(params.clone()),
            _ => None,
        };
        let arguments: Vec<_> = args
            .iter()
            .zip(arg_types)
            .map(|(argument, ty)| self.semantic_candidates_for_call_argument(argument, ty))
            .collect();
        let Some(candidate) = self
            .generated_inference
            .callsite_candidates
            .get_mut(&key)
            .and_then(|candidates| candidates.last_mut())
        else {
            return;
        };
        let Some(parameter_types) = parameter_types else {
            candidate.parameter_types = Some(Vec::new());
            candidate.arguments = Some(vec![vec![SemanticCandidateObservation::Unavailable(
                GeneratedSemanticFactIssue::new(
                    "instantiated generic callee did not produce a callable parameter shape",
                ),
            )]]);
            return;
        };
        candidate.parameter_types = Some(parameter_types);
        candidate.arguments = Some(arguments);
    }

    fn semantic_candidates_for_call_argument(
        &self,
        argument: &shape_ast::ast::Expr,
        ty: &crate::type_system::Type,
    ) -> Vec<SemanticCandidateObservation> {
        match argument {
            shape_ast::ast::Expr::FunctionExpr {
                params,
                return_type,
                ..
            } => vec![
                SemanticTypeCandidate::generated_callable(ty.clone(), params, return_type.as_ref())
                    .map(SemanticCandidateObservation::Candidate)
                    .unwrap_or_else(|detail| {
                        SemanticCandidateObservation::Unavailable(GeneratedSemanticFactIssue::new(
                            detail,
                        ))
                    }),
            ],
            shape_ast::ast::Expr::Identifier(name, _) => self
                .env
                .lookup_binding_token(name)
                .and_then(|token| self.generated_inference.binding_candidates.get(&token))
                .map(|observations| {
                    observations
                        .iter()
                        .cloned()
                        .map(|observation| observation.with_type(ty.clone()))
                        .collect()
                })
                .unwrap_or_else(|| vec![semantic_type_observation(ty.clone())]),
            _ => vec![semantic_type_observation(ty.clone())],
        }
    }

    pub(super) fn finalize_semantic_callsite_facts(&mut self) {
        let candidates = std::mem::take(&mut self.generated_inference.callsite_candidates);
        self.generated_inference.callsite_facts = candidates
            .into_iter()
            .map(|(key, candidates)| (key, self.reduce_callsite_candidates(candidates)))
            .collect();
    }

    fn reduce_callsite_candidates(
        &self,
        candidates: Vec<SemanticCallSiteCandidate>,
    ) -> SemanticCallSiteFact {
        let mut exact: Option<ExactSemanticCallSiteFact> = None;
        let mut unavailable = Vec::new();
        let mut conflicts = Vec::new();
        for candidate in candidates {
            let parameter_types = candidate.parameter_types.as_deref();
            let argument_candidates = candidate.arguments.as_deref();
            let mut arguments = Vec::with_capacity(candidate.instantiations.len());
            for instantiation in candidate.instantiations {
                let Some(provenance) = instantiation.declared().declared_provenance() else {
                    unavailable.push(
                        "scheme instantiation omitted declared-parameter provenance".to_string(),
                    );
                    continue;
                };
                let resolved =
                    self.solver
                        .unifier()
                        .apply_substitutions(&crate::type_system::Type::Variable(
                            instantiation.instantiated().clone(),
                        ));
                if !type_is_semantically_resolved(&resolved, false) {
                    unavailable.push(format!(
                        "semantic argument '{}' remained unresolved after solving",
                        provenance.source_name()
                    ));
                    continue;
                }
                let mut projected = Vec::new();
                if let (Some(parameter_types), Some(argument_candidates)) =
                    (parameter_types, argument_candidates)
                {
                    if parameter_types.len() != argument_candidates.len() {
                        unavailable.push(
                            "generic call parameter/argument semantic evidence arity mismatch"
                                .to_string(),
                        );
                        continue;
                    }
                    for (parameter, actual_observations) in
                        parameter_types.iter().zip(argument_candidates)
                    {
                        for actual in actual_observations {
                            match actual {
                                SemanticCandidateObservation::Candidate(actual) => {
                                    match project_declared_argument_candidates(
                                        parameter,
                                        actual,
                                        instantiation.instantiated(),
                                    ) {
                                        Ok(mut occurrences) => projected.append(&mut occurrences),
                                        Err(SemanticProjectionIssue::Unavailable(issue)) => {
                                            unavailable.push(issue);
                                        }
                                        Err(SemanticProjectionIssue::Conflict(issue)) => {
                                            conflicts.push(issue);
                                        }
                                    }
                                }
                                SemanticCandidateObservation::Unavailable(issue) => {
                                    unavailable.push(issue.detail().to_string());
                                }
                                SemanticCandidateObservation::Conflict(issue) => {
                                    conflicts.push(issue.detail().to_string());
                                }
                            }
                        }
                    }
                }
                let semantic = if projected.is_empty() {
                    SemanticTypeCandidate::monomorphic_binding(resolved.clone())
                } else {
                    let mut semantic: Option<SemanticTypeCandidate> = None;
                    let mut projection_conflict = false;
                    for occurrence in projected {
                        let occurrence = match occurrence.with_resolved_type(resolved.clone()) {
                            Ok(occurrence) => occurrence,
                            Err(detail) => {
                                unavailable.push(detail);
                                continue;
                            }
                        };
                        match &semantic {
                            None => semantic = Some(occurrence),
                            Some(previous) if *previous == occurrence => {}
                            Some(_) => projection_conflict = true,
                        }
                    }
                    if projection_conflict {
                        conflicts.push(
                            "repeated generic-parameter occurrences projected different semantic candidates"
                                .to_string(),
                        );
                    }
                    semantic.ok_or_else(|| {
                        "no exact semantic argument occurrence survived projection".to_string()
                    })
                };
                let semantic = match semantic {
                    Ok(semantic) => semantic,
                    Err(detail) => {
                        unavailable.push(format!(
                            "semantic argument '{}' lacks exact recursive callable shape: {detail}",
                            provenance.source_name()
                        ));
                        continue;
                    }
                };
                arguments.push(SemanticTypeArgument {
                    declared: instantiation.declared().clone(),
                    ordinal: provenance.ordinal(),
                    source_name: provenance.source_name().to_string(),
                    candidate: semantic,
                });
            }
            arguments.sort_by_key(SemanticTypeArgument::ordinal);
            if arguments.len() != candidate.instantiations.len() {
                continue;
            }
            if arguments
                .windows(2)
                .any(|pair| pair[0].ordinal == pair[1].ordinal)
            {
                conflicts.push(
                    "declared semantic arguments repeated the same parameter ordinal".to_string(),
                );
                continue;
            }
            let candidate = ExactSemanticCallSiteFact::new(arguments);
            match &exact {
                None => exact = Some(candidate),
                Some(previous) if *previous == candidate => {}
                Some(_) => conflicts.push(
                    "structurally identical call sites resolved different semantic arguments"
                        .to_string(),
                ),
            }
        }
        if !conflicts.is_empty() {
            conflicts.sort();
            conflicts.dedup();
            SemanticCallSiteFact::Conflict(GeneratedSemanticFactIssue::new(conflicts.join("; ")))
        } else if !unavailable.is_empty() {
            unavailable.sort();
            unavailable.dedup();
            SemanticCallSiteFact::Unavailable(GeneratedSemanticFactIssue::new(
                unavailable.join("; "),
            ))
        } else if let Some(exact) = exact {
            SemanticCallSiteFact::Exact(exact)
        } else {
            SemanticCallSiteFact::Unavailable(GeneratedSemanticFactIssue::new(
                "no exact semantic call-site argument was recorded",
            ))
        }
    }

    pub(super) fn take_semantic_callsite_facts(
        &mut self,
    ) -> HashMap<SemanticCallSiteKey, SemanticCallSiteFact> {
        std::mem::take(&mut self.generated_inference.callsite_facts)
    }
}

fn semantic_type_observation(ty: crate::type_system::Type) -> SemanticCandidateObservation {
    SemanticTypeCandidate::monomorphic_binding(ty)
        .map(SemanticCandidateObservation::Candidate)
        .unwrap_or_else(|detail| {
            SemanticCandidateObservation::Unavailable(GeneratedSemanticFactIssue::new(detail))
        })
}

#[cfg(test)]
mod tests;
