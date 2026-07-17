//! Finalized structural-fact handoff carried with ordinary inference results.

use std::collections::HashMap;

use super::{
    GeneratedCallableFact, GeneratedCaptureFact, GeneratedCaptureKey, GeneratedNodeKey,
    SemanticCallSiteFact, SemanticCallSiteKey, SemanticCalleeDeclaration,
};
use crate::type_system::Type;
use crate::type_system::inference::{BindingFact, InferenceFacts};
use shape_ast::ast::Span;

impl InferenceFacts {
    pub fn with_all_facts(
        top_level_types: HashMap<String, Type>,
        expression_types: HashMap<Span, Type>,
        binding_facts: HashMap<Span, BindingFact>,
        generated_callable_facts: HashMap<GeneratedNodeKey, GeneratedCallableFact>,
        generated_capture_facts: HashMap<GeneratedCaptureKey, GeneratedCaptureFact>,
        semantic_callsite_facts: HashMap<SemanticCallSiteKey, SemanticCallSiteFact>,
        semantic_callee_declarations: HashMap<String, SemanticCalleeDeclaration>,
    ) -> Self {
        Self {
            top_level_types,
            expression_types,
            binding_facts,
            generated_callable_facts,
            generated_capture_facts,
            semantic_callsite_facts,
            semantic_callee_declarations,
        }
    }

    pub fn generated_callable_fact(
        &self,
        key: &GeneratedNodeKey,
    ) -> Option<&GeneratedCallableFact> {
        self.generated_callable_facts.get(key)
    }

    pub fn generated_callable_facts(&self) -> &HashMap<GeneratedNodeKey, GeneratedCallableFact> {
        &self.generated_callable_facts
    }

    pub fn generated_capture_fact(
        &self,
        key: &GeneratedCaptureKey,
    ) -> Option<&GeneratedCaptureFact> {
        self.generated_capture_facts.get(key)
    }

    pub fn generated_capture_facts(&self) -> &HashMap<GeneratedCaptureKey, GeneratedCaptureFact> {
        &self.generated_capture_facts
    }

    pub fn semantic_callsite_fact(
        &self,
        key: &SemanticCallSiteKey,
    ) -> Option<&SemanticCallSiteFact> {
        self.semantic_callsite_facts.get(key)
    }

    pub fn semantic_callsite_facts(&self) -> &HashMap<SemanticCallSiteKey, SemanticCallSiteFact> {
        &self.semantic_callsite_facts
    }

    pub fn semantic_callee_declaration(&self, callee: &str) -> Option<&SemanticCalleeDeclaration> {
        self.semantic_callee_declarations.get(callee)
    }

    pub fn semantic_callee_declarations(&self) -> &HashMap<String, SemanticCalleeDeclaration> {
        &self.semantic_callee_declarations
    }
}
