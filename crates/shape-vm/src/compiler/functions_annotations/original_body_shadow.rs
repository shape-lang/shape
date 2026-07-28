//! Staging and emission for a `replace body` original-body shadow.
//!
//! Directive execution may still remove the target or reject a later
//! directive. The original body therefore remains an owned pending artifact
//! until the complete handler outcome is known. Only then is it independently
//! analyzed under its semantic owner and emitted under its hygienic identity.

use super::{
    BytecodeCompiler, FunctionDef, OriginalCapability, ParamPassMode, Result, ShapeError,
};
use crate::compiler::comptime_builtins::{
    FreezeOverlay, FrozenTypeCategory, FrozenTypeIdentity,
};
use shape_ast::ast::{FunctionParam, TypeAnnotation};

const SHADOW_IDENTITY_DIAGNOSTIC: &str =
    "internal original-body capability invariant: shadow identity does not match the staged semantic owner";
const CALLABLE_IDENTITY_DIAGNOSTIC: &str =
    "internal original-body capability invariant: frozen callable identity does not match the staged semantic owner and shadow emission";
const CALLABLE_PAYLOAD_DIAGNOSTIC: &str =
    "internal original-body capability invariant: frozen callable identity has no complete Callable payload";

pub(super) fn canonical_original_callable(
    freeze: &FreezeOverlay,
    function: &FunctionDef,
) -> std::result::Result<FrozenTypeIdentity, String> {
    let mut params = Vec::with_capacity(function.params.len());
    for parameter in &function.params {
        let Some(type_annotation) = parameter.type_annotation.clone() else {
            let parameter_name = match parameter.simple_name() {
                Some(name) => name,
                None => "<pattern>",
            };
            return Err(format!(
                "`ctx.original` capability requires a typed parameter, but parameter '{}' of '{}' has no type annotation",
                parameter_name,
                function.name
            ));
        };
        let type_annotation = if parameter.is_reference {
            TypeAnnotation::Borrow {
                mutable: parameter.is_mut_reference,
                inner: Box::new(type_annotation),
            }
        } else {
            type_annotation
        };
        params.push(FunctionParam {
            name: parameter.simple_name().map(str::to_string),
            optional: parameter.default_value.is_some(),
            type_annotation,
        });
    }
    let return_type = match &function.return_type {
        Some(type_annotation) => type_annotation.clone(),
        None => TypeAnnotation::Void,
    };
    let projection = freeze.canonicalize_type_projection(&TypeAnnotation::Function {
        params,
        returns: Box::new(return_type),
        effects: None,
    })?;
    if projection.category() != FrozenTypeCategory::Callable {
        return Err("ctx.original signature did not canonicalize as a Callable".to_string());
    }
    Ok(projection.identity())
}

#[derive(Debug, Clone)]
pub(super) struct PendingOriginalBodyShadow {
    semantic_owner: FunctionDef,
    emission: FunctionDef,
    capability: OriginalCapability,
    inferred_reference_optimizations: Vec<Option<ParamPassMode>>,
    effective_pass_modes: Vec<ParamPassMode>,
}

impl PendingOriginalBodyShadow {
    pub(super) fn new(
        semantic_owner: &FunctionDef,
        capability: OriginalCapability,
        inferred_reference_optimizations: &[Option<ParamPassMode>],
        effective_pass_modes: &[ParamPassMode],
    ) -> Result<Self> {
        Self::validate_cardinality(
            semantic_owner,
            inferred_reference_optimizations.len(),
            "inferred-reference provenance",
        )?;
        Self::validate_cardinality(
            semantic_owner,
            effective_pass_modes.len(),
            "effective pass modes",
        )?;

        let emission = FunctionDef {
            name: capability.shadow_name().to_string(),
            name_span: semantic_owner.name_span,
            declaring_module_path: semantic_owner.declaring_module_path.clone(),
            doc_comment: None,
            params: semantic_owner.params.clone(),
            return_type: semantic_owner.return_type.clone(),
            body: semantic_owner.body.clone(),
            type_params: semantic_owner.type_params.clone(),
            annotations: Vec::new(),
            where_clause: semantic_owner.where_clause.clone(),
            is_async: semantic_owner.is_async,
            is_comptime: semantic_owner.is_comptime,
            effect_row: None,
        };
        Ok(Self {
            semantic_owner: semantic_owner.clone(),
            emission,
            capability,
            inferred_reference_optimizations: inferred_reference_optimizations.to_vec(),
            effective_pass_modes: effective_pass_modes.to_vec(),
        })
    }

    fn validate_cardinality(
        semantic_owner: &FunctionDef,
        actual: usize,
        fact: &str,
    ) -> Result<()> {
        let expected = semantic_owner.params.len();
        if actual == expected {
            return Ok(());
        }
        Err(ShapeError::RuntimeError {
            message: format!(
                "internal pending original-body shadow invariant: function '{}' has {expected} parameters but {actual} {fact} entries",
                semantic_owner.name
            ),
            location: None,
        })
    }
}

impl BytecodeCompiler {
    fn original_capability_invariant(message: &'static str) -> ShapeError {
        ShapeError::RuntimeError {
            message: message.to_string(),
            location: None,
        }
    }

    fn validate_pending_original_body_capability(
        &self,
        pending: &PendingOriginalBodyShadow,
    ) -> Result<()> {
        let expected_shadow = self.original_body_shadow_name(&pending.semantic_owner.name);
        if pending.capability.shadow_name() != expected_shadow.as_str()
            || pending.emission.name.as_str() != expected_shadow.as_str()
        {
            return Err(Self::original_capability_invariant(
                SHADOW_IDENTITY_DIAGNOSTIC,
            ));
        }

        let freeze = self
            .comptime_freeze_overlay()
            .map_err(|_| Self::original_capability_invariant(CALLABLE_IDENTITY_DIAGNOSTIC))?;
        let owner_callable = canonical_original_callable(freeze.as_ref(), &pending.semantic_owner)
            .map_err(|_| Self::original_capability_invariant(CALLABLE_IDENTITY_DIAGNOSTIC))?;
        let emission_callable = canonical_original_callable(freeze.as_ref(), &pending.emission)
            .map_err(|_| Self::original_capability_invariant(CALLABLE_IDENTITY_DIAGNOSTIC))?;
        let stored_callable = pending.capability.callable();
        if stored_callable != owner_callable || owner_callable != emission_callable {
            return Err(Self::original_capability_invariant(
                CALLABLE_IDENTITY_DIAGNOSTIC,
            ));
        }
        // Canonicalizing both equal signatures on this same overlay memoizes
        // their complete Callable payload. The check remains a defensive
        // refusal for internal freeze corruption; no partial payload is usable.
        let payload = freeze
            .payload_of(stored_callable)
            .map_err(|_| Self::original_capability_invariant(CALLABLE_PAYLOAD_DIAGNOSTIC))?;
        if payload.category() != FrozenTypeCategory::Callable {
            return Err(Self::original_capability_invariant(
                CALLABLE_PAYLOAD_DIAGNOSTIC,
            ));
        }
        Ok(())
    }

    /// Publish and compile a staged shadow after all handlers have completed.
    ///
    /// Registration deliberately follows the compiler's existing quarantine
    /// convention: a later analysis/emission error can leave a rejected
    /// compiler-issued slot behind. This method is not an artifact transaction.
    pub(super) fn finalize_pending_original_body_shadow(
        &mut self,
        pending: PendingOriginalBodyShadow,
    ) -> Result<()> {
        self.validate_pending_original_body_capability(&pending)?;
        let semantic_owner = pending.semantic_owner;
        let emission = pending.emission;
        let inferred_reference_optimizations = pending.inferred_reference_optimizations;
        let effective_pass_modes = pending.effective_pass_modes;

        self.register_function(&emission)?;
        let emission_id = self.find_function(&emission.name).ok_or_else(|| {
            ShapeError::RuntimeError {
                message: format!(
                    "Original-body shadow '{}' was not registered",
                    emission.name
                ),
                location: None,
            }
        })?;

        // The untouched body is analyzed under its original semantic identity.
        // The replacement body has not entered its normal analysis path yet.
        self.refresh_function_signature_metadata(&semantic_owner)?;
        self.analyze_function_body(&semantic_owner)?;
        self.refresh_authoritative_emission_metadata(
            emission_id,
            &emission,
            &semantic_owner.name,
            &effective_pass_modes,
        )?;
        // `closure_function_ids` is an ephemeral MIR-backpatch queue owned by
        // the function currently being compiled. Shadow closures remain
        // persistently registered under the compiler's existing quarantine
        // convention, but their transient IDs must never backpatch the later
        // replacement body. Scope this queue across both success and error.
        let saved_closure_function_ids = std::mem::take(&mut self.closure_function_ids);
        let shadow_result = self.with_body_analysis_authority(
            emission_id,
            &semantic_owner,
            &emission,
            |compiler| {
                compiler.compile_function_body_with_inferred_reference_optimizations(
                    &emission,
                    &inferred_reference_optimizations,
                )
            },
        );
        self.closure_function_ids.clear();
        self.closure_function_ids = saved_closure_function_ids;
        shadow_result
    }
}

#[cfg(test)]
#[path = "original_body_shadow_tests.rs"]
mod tests;
