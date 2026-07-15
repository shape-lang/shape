//! Finalized facts handed from one inference pass to compiler consumers.

use std::collections::HashMap;

use shape_ast::ast::Span;

use super::{
    GeneratedCallableFact, GeneratedCaptureFact, GeneratedCaptureKey, GeneratedNodeKey,
    SemanticCallSiteFact, SemanticCallSiteKey, SemanticCalleeDeclaration,
};
use crate::type_system::Type;

/// Finalized type fact for one source binding.
///
/// The key in `InferenceFacts::binding_facts` is the binder/name span. The
/// duplicated `binder_span` keeps the record self-describing for consumers that
/// collect facts by name.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingFact {
    pub name: String,
    pub binder_span: Span,
    pub initializer_span: Option<Span>,
    pub ty: Type,
}

/// Canonical type-inference facts produced by one best-effort program pass.
///
/// Both top-level and structural maps come from the same inference run;
/// constructing this carrier must not trigger a second inference pass.
#[derive(Debug, Clone, Default)]
pub struct InferenceFacts {
    pub(super) top_level_types: HashMap<String, Type>,
    pub(super) expression_types: HashMap<Span, Type>,
    pub(super) binding_facts: HashMap<Span, BindingFact>,
    pub(super) generated_callable_facts: HashMap<GeneratedNodeKey, GeneratedCallableFact>,
    pub(super) generated_capture_facts: HashMap<GeneratedCaptureKey, GeneratedCaptureFact>,
    pub(super) semantic_callsite_facts: HashMap<SemanticCallSiteKey, SemanticCallSiteFact>,
    pub(super) semantic_callee_declarations: HashMap<String, SemanticCalleeDeclaration>,
}

impl InferenceFacts {
    pub fn new(
        top_level_types: HashMap<String, Type>,
        expression_types: HashMap<Span, Type>,
    ) -> Self {
        Self {
            top_level_types,
            expression_types,
            ..Self::default()
        }
    }

    pub fn with_binding_facts(
        top_level_types: HashMap<String, Type>,
        expression_types: HashMap<Span, Type>,
        binding_facts: HashMap<Span, BindingFact>,
    ) -> Self {
        Self {
            top_level_types,
            expression_types,
            binding_facts,
            ..Self::default()
        }
    }

    pub fn expression_type(&self, span: Span) -> Option<&Type> {
        if span.is_dummy() {
            return None;
        }
        self.expression_types.get(&span)
    }

    pub fn function_signature(&self, name: &str) -> Option<&Type> {
        match self.top_level_types.get(name) {
            Some(ty @ Type::Function { .. }) => Some(ty),
            _ => None,
        }
    }

    pub fn top_level_type(&self, name: &str) -> Option<&Type> {
        self.top_level_types.get(name)
    }

    pub fn top_level_types(&self) -> &HashMap<String, Type> {
        &self.top_level_types
    }

    pub fn expression_types(&self) -> &HashMap<Span, Type> {
        &self.expression_types
    }

    pub fn binding_fact(&self, span: Span) -> Option<&BindingFact> {
        if span.is_dummy() {
            return None;
        }
        self.binding_facts.get(&span)
    }

    pub fn binding_type(&self, span: Span) -> Option<&Type> {
        self.binding_fact(span).map(|fact| &fact.ty)
    }

    pub fn binding_facts(&self) -> &HashMap<Span, BindingFact> {
        &self.binding_facts
    }

    pub fn bindings_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a BindingFact> {
        self.binding_facts
            .values()
            .filter(move |fact| fact.name == name)
    }

    pub fn into_expression_types(self) -> HashMap<Span, Type> {
        self.expression_types
    }

    pub fn into_parts(self) -> (HashMap<String, Type>, HashMap<Span, Type>) {
        (self.top_level_types, self.expression_types)
    }

    pub fn into_parts_with_bindings(
        self,
    ) -> (
        HashMap<String, Type>,
        HashMap<Span, Type>,
        HashMap<Span, BindingFact>,
    ) {
        (
            self.top_level_types,
            self.expression_types,
            self.binding_facts,
        )
    }
}
