//! Active generic-callee declaration capabilities in the finalized handoff.

use std::collections::HashMap;

use super::{ExactSemanticCallSiteFact, SemanticTypeArgument};
use crate::type_system::{TypeInferenceEngine, TypeVar};

/// One ordered declaration parameter. Token equality proves the opaque owner;
/// explicit ordinal and spelling remain load-bearing because [`TypeVar`]
/// identity deliberately ignores presentation-only source names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDeclaredParameter {
    token: TypeVar,
    ordinal: u32,
    source_name: String,
}

impl SemanticDeclaredParameter {
    #[must_use]
    pub fn token(&self) -> &TypeVar {
        &self.token
    }

    #[must_use]
    pub fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    fn matches_argument(&self, argument: &SemanticTypeArgument) -> bool {
        self.token == *argument.declared()
            && self.ordinal == argument.ordinal()
            && self.source_name == argument.source_name()
    }
}

/// Inference-owned declaration capability expected for one compiler-qualified
/// generic callee.
///
/// The ordered opaque tokens are validation authority only. Downstream code
/// compares them for exact equality before cache lookup; their transient owner
/// identity is never rendered or included in a frozen specialization key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticCalleeDeclaration {
    parameters: Vec<SemanticDeclaredParameter>,
}

impl SemanticCalleeDeclaration {
    pub(super) fn from_tokens(parameters: &[TypeVar]) -> Option<Self> {
        let parameters = parameters
            .iter()
            .map(|token| {
                let provenance = token.declared_provenance()?;
                Some(SemanticDeclaredParameter {
                    token: token.clone(),
                    ordinal: provenance.ordinal(),
                    source_name: provenance.source_name().to_string(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { parameters })
    }

    pub(super) fn from_arguments(arguments: &[SemanticTypeArgument]) -> Self {
        Self {
            parameters: arguments
                .iter()
                .map(|argument| SemanticDeclaredParameter {
                    token: argument.declared().clone(),
                    ordinal: argument.ordinal(),
                    source_name: argument.source_name().to_string(),
                })
                .collect(),
        }
    }

    #[must_use]
    pub fn parameters(&self) -> &[SemanticDeclaredParameter] {
        &self.parameters
    }

    /// Validate one sealed exact fact against this active callee capability.
    ///
    /// This is the compiler-facing provenance authority: the active catalog,
    /// the fact's sealed declaration, and every argument must agree on opaque
    /// token identity, ordinal, and authored spelling before any cache lookup.
    #[must_use]
    pub fn matches_exact(&self, exact: &ExactSemanticCallSiteFact) -> bool {
        let sealed = exact.declaration().parameters();
        let arguments = exact.arguments();
        self.parameters.len() == arguments.len()
            && sealed.len() == arguments.len()
            && self.parameters == sealed
            && self
                .parameters
                .iter()
                .zip(arguments)
                .all(|(parameter, argument)| parameter.matches_argument(argument))
            && sealed
                .iter()
                .zip(arguments)
                .all(|(parameter, argument)| parameter.matches_argument(argument))
    }
}

impl TypeInferenceEngine {
    /// Project active AST declaration capabilities into the finalized handoff
    /// before transient inference maps are cleared. Same-named declarations
    /// publish only the active lexical declaration, matching compiler lookup;
    /// qualified method aliases may intentionally share one capability.
    pub(in crate::type_system::inference) fn finalize_semantic_callee_declarations(&mut self) {
        let mut declarations = HashMap::new();
        for (name, declaration) in &self.generated_inference.active_callable_declarations {
            if let Some(parameters) = self
                .generated_inference
                .callable_declared_parameters
                .get(declaration)
                .filter(|parameters| !parameters.is_empty())
            {
                if let Some(declaration) = SemanticCalleeDeclaration::from_tokens(parameters) {
                    declarations.insert(name.clone(), declaration);
                }
            }
        }
        for (qualified_name, declaration) in &self.generated_inference.active_method_declarations {
            if let Some(parameters) = self
                .generated_inference
                .method_declared_parameters
                .get(declaration)
            {
                let mut ordered = parameters.receiver.clone();
                ordered.extend(parameters.method.iter().cloned());
                if !ordered.is_empty() {
                    if let Some(declaration) = SemanticCalleeDeclaration::from_tokens(&ordered) {
                        declarations.insert(qualified_name.clone(), declaration);
                    }
                }
            }
        }
        self.generated_inference.callee_declarations = declarations;
    }

    pub(in crate::type_system::inference) fn take_semantic_callee_declarations(
        &mut self,
    ) -> HashMap<String, SemanticCalleeDeclaration> {
        std::mem::take(&mut self.generated_inference.callee_declarations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_system::{Type, TypeVarGen};
    use shape_ast::ast::TypeAnnotation;

    #[test]
    fn sealed_exact_fact_rejects_active_declaration_with_renamed_parameter() {
        let mut generator = TypeVarGen::new();
        let owner = generator.fresh_declared_owner();
        let authored_t = TypeVar::declared(owner, 0, "T");
        let authored_u = TypeVar::declared(owner, 0, "U");
        let argument = SemanticTypeArgument {
            declared: authored_t.clone(),
            ordinal: 0,
            source_name: "T".to_string(),
            candidate: crate::type_system::SemanticTypeCandidate::monomorphic_binding(
                Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            )
            .expect("primitive argument must be exact"),
        };
        let exact = ExactSemanticCallSiteFact::new(vec![argument]);
        let active = SemanticCalleeDeclaration::from_tokens(std::slice::from_ref(&authored_u))
            .expect("declared parameter must produce a catalog capability");

        assert_eq!(authored_t, authored_u, "TypeVar identity ignores spelling");
        assert!(
            !active.matches_exact(&exact),
            "compiler-facing validation must retain authored spelling independently",
        );
    }
}
