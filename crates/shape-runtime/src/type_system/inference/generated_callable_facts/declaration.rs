//! Exact declaration capabilities for named and synthetic callables.

use std::collections::{HashMap, HashSet};

use shape_ast::ast::FunctionDef;

use super::{
    InferenceCallableDeclarationToken, SemanticCandidateObservation, SemanticTypeCandidate,
    TypeInferenceEngine, unavailable_observation,
};
use crate::type_system::{BindingToken, Type, TypeError, TypeResult, TypeScheme, TypeVar};

fn declared_quantifiers(scheme: &TypeScheme) -> TypeResult<Vec<TypeVar>> {
    let mut declared = Vec::new();
    let mut owner = None;
    let mut source_names = HashSet::new();
    for variable in &scheme.quantified {
        let Some(provenance) = variable.declared_provenance() else {
            continue;
        };
        let ordinal = u32::try_from(declared.len()).map_err(|_| {
            TypeError::ConstraintViolation(
                "generic scheme declares more parameters than the provenance range supports"
                    .to_string(),
            )
        })?;
        if provenance.ordinal() != ordinal {
            return Err(TypeError::ConstraintViolation(
                "generic scheme parameter order disagrees with declared provenance".to_string(),
            ));
        }
        if owner.is_some_and(|expected| expected != provenance.owner()) {
            return Err(TypeError::ConstraintViolation(
                "generic scheme combines parameters from different declaration owners"
                    .to_string(),
            ));
        }
        owner = Some(provenance.owner());
        if !source_names.insert(provenance.source_name()) {
            return Err(TypeError::ConstraintViolation(format!(
                "generic scheme declares type parameter '{}' more than once",
                provenance.source_name()
            )));
        }
        declared.push(variable.clone());
    }
    Ok(declared)
}

fn same_declared_quantifiers(expected: &[TypeVar], actual: &[TypeVar]) -> bool {
    expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(expected, actual)| {
            let (Some(expected), Some(actual)) = (
                expected.declared_provenance(),
                actual.declared_provenance(),
            ) else {
                return false;
            };
            expected.owner() == actual.owner()
                && expected.ordinal() == actual.ordinal()
                && expected.source_name() == actual.source_name()
        })
}

impl TypeInferenceEngine {
    pub(crate) fn declared_parameter_tokens(
        scheme: &TypeScheme,
    ) -> TypeResult<HashMap<String, TypeVar>> {
        let declared = declared_quantifiers(scheme)?;
        let mut parameters = HashMap::with_capacity(declared.len());
        for variable in declared {
            let provenance = variable
                .declared_provenance()
                .expect("declared_quantifiers returns only declared capabilities");
            if parameters
                .insert(provenance.source_name().to_string(), variable.clone())
                .is_some()
            {
                return Err(TypeError::ConstraintViolation(format!(
                    "generic scheme declares type parameter '{}' more than once",
                    provenance.source_name()
                )));
            }
        }
        Ok(parameters)
    }

    pub(crate) fn install_callable_declared_parameters(
        &mut self,
        function: &FunctionDef,
        parameters: Vec<TypeVar>,
    ) -> TypeResult<()> {
        let declaration = InferenceCallableDeclarationToken::of(function);
        match self
            .generated_inference
            .callable_declared_parameters
            .insert(declaration, parameters.clone())
        {
            Some(previous) if previous != parameters => {
                self.generated_inference
                    .callable_declared_parameters
                    .insert(declaration, previous);
                Err(TypeError::ConstraintViolation(
                    "internal inference error: callable declaration received conflicting generic-token capabilities"
                        .to_string(),
                ))
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn remove_callable_declared_parameters(&mut self, function: &FunctionDef) {
        self.generated_inference
            .callable_declared_parameters
            .remove(&InferenceCallableDeclarationToken::of(function));
    }

    /// Named callable declarations have the same exact syntax evidence as a
    /// closure literal. Recording it on their lexical token makes direct
    /// captures and identifier aliases preserve nested callable shape.
    pub(crate) fn record_named_callable_semantic_candidate(
        &mut self,
        token: BindingToken,
        function: &FunctionDef,
        ty: &Type,
    ) {
        let observation = SemanticTypeCandidate::generated_callable(
            ty.clone(),
            &function.params,
            function.return_type.as_ref(),
        )
        .map(SemanticCandidateObservation::Candidate)
        .unwrap_or_else(unavailable_observation);
        self.record_binding_semantic_observations(token, [observation]);
    }

    /// Publish one named callable across predeclare/infer/rewalk passes while
    /// preserving the lexical token of that exact in-memory AST declaration.
    /// The pointer is a transient pass capability only; it is never serialized
    /// or included in semantic identity.
    pub(crate) fn predeclare_named_callable_scheme(
        &mut self,
        function: &FunctionDef,
        scheme: TypeScheme,
        ty: &Type,
    ) -> TypeResult<()> {
        let declaration = InferenceCallableDeclarationToken::of(function);
        if self
            .generated_inference
            .callable_binding_tokens
            .contains_key(&declaration)
        {
            return Err(TypeError::ConstraintViolation(
                "internal inference error: callable declaration was predeclared twice".to_string(),
            ));
        }
        let declared = declared_quantifiers(&scheme)?;
        let token = self.env.define_with_token(&function.name, scheme);
        self.generated_inference
            .callable_binding_tokens
            .insert(declaration, token);
        self.generated_inference
            .active_callable_declarations
            .insert(function.name.clone(), declaration);
        self.generated_inference
            .callable_declared_parameters
            .insert(declaration, declared);
        self.record_named_callable_semantic_candidate(token, function, ty);
        Ok(())
    }

    pub(crate) fn declared_type_parameters_for_callable(
        &self,
        function: &FunctionDef,
    ) -> TypeResult<Vec<TypeVar>> {
        self.generated_inference
            .callable_declared_parameters
            .get(&InferenceCallableDeclarationToken::of(function))
            .cloned()
            .ok_or_else(|| {
                TypeError::ConstraintViolation(
                    "internal inference error: callable declaration has no predeclared generic-token capability"
                        .to_string(),
                )
            })
    }

    pub(crate) fn republish_named_callable_scheme(
        &mut self,
        function: &FunctionDef,
        scheme: TypeScheme,
        ty: &Type,
    ) -> TypeResult<()> {
        let republished_declared = declared_quantifiers(&scheme)?;
        let declaration = InferenceCallableDeclarationToken::of(function);
        let token = self
            .generated_inference
            .callable_binding_tokens
            .get(&declaration)
            .copied()
            .ok_or_else(|| {
                TypeError::ConstraintViolation(
                    "internal inference error: callable re-publication has no predeclared AST capability"
                        .to_string(),
                )
            })?;
        let declared = self
            .generated_inference
            .callable_declared_parameters
            .get(&declaration)
            .ok_or_else(|| {
                TypeError::ConstraintViolation(
                    "internal inference error: callable re-publication lost its declared generic-token capability"
                        .to_string(),
                )
            })?;
        if !same_declared_quantifiers(declared, &republished_declared) {
            return Err(TypeError::ConstraintViolation(
                "internal inference error: callable re-publication changed its declared generic tokens"
                    .to_string(),
            ));
        }
        match self.env.lookup_binding_token(&function.name) {
            Some(current) if current == token => self
                .env
                .redefine_with_token(&function.name, token, scheme)
                .map_err(TypeError::ConstraintViolation)?,
            // A later same-named AST declaration is the current lexical
            // binding. Keep this declaration's observation on its own opaque
            // token without replacing the later binding's environment entry.
            Some(_) => {}
            None => {
                return Err(TypeError::ConstraintViolation(
                    "internal inference error: callable re-publication has no live lexical binding"
                        .to_string(),
                ));
            }
        }
        self.record_named_callable_semantic_candidate(token, function, ty);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
