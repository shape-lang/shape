//! Transient inference-run state for structural semantic facts.

use std::collections::HashMap;

use crate::type_system::{BindingToken, TypeVar};

use super::{
    DeclaredMethodTypeParameters, GeneratedCallableCandidate, GeneratedCallableFact,
    GeneratedCaptureFact, GeneratedCaptureKey, GeneratedNodeKey, InferenceCallableDeclarationToken,
    InferenceMethodDeclarationToken, SemanticCallSiteCandidate, SemanticCallSiteFact,
    SemanticCallSiteKey, SemanticCalleeDeclaration, SemanticCandidateObservation,
};

#[derive(Default)]
pub(super) struct GeneratedInferenceState {
    pub(super) callable_candidates: HashMap<GeneratedNodeKey, Vec<GeneratedCallableCandidate>>,
    pub(super) callable_facts: HashMap<GeneratedNodeKey, GeneratedCallableFact>,
    pub(super) capture_candidates: HashMap<GeneratedCaptureKey, Vec<SemanticCandidateObservation>>,
    pub(super) capture_facts: HashMap<GeneratedCaptureKey, GeneratedCaptureFact>,
    pub(super) active_node_stack: Vec<GeneratedNodeKey>,
    pub(super) binding_candidates: HashMap<BindingToken, Vec<SemanticCandidateObservation>>,
    pub(super) callable_binding_tokens: HashMap<InferenceCallableDeclarationToken, BindingToken>,
    pub(super) active_callable_declarations: HashMap<String, InferenceCallableDeclarationToken>,
    pub(super) callable_declared_parameters:
        HashMap<InferenceCallableDeclarationToken, Vec<TypeVar>>,
    pub(super) method_declared_parameters:
        HashMap<InferenceMethodDeclarationToken, DeclaredMethodTypeParameters>,
    pub(super) active_method_declarations: HashMap<String, InferenceMethodDeclarationToken>,
    pub(super) callsite_candidates: HashMap<SemanticCallSiteKey, Vec<SemanticCallSiteCandidate>>,
    pub(super) callsite_facts: HashMap<SemanticCallSiteKey, SemanticCallSiteFact>,
    pub(super) callee_declarations: HashMap<String, SemanticCalleeDeclaration>,
}
