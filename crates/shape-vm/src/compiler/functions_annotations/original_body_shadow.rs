//! Staging and emission for a `replace body` original-body shadow.
//!
//! Directive execution may still remove the target or reject a later
//! directive. The original body therefore remains an owned pending artifact
//! until the complete handler outcome is known. Only then is it independently
//! analyzed under its semantic owner and emitted under its hygienic identity.

use super::{BytecodeCompiler, FunctionDef, ParamPassMode, Result, ShapeError};

#[derive(Debug, Clone)]
pub(super) struct PendingOriginalBodyShadow {
    semantic_owner: FunctionDef,
    emission: FunctionDef,
    inferred_reference_optimizations: Vec<Option<ParamPassMode>>,
    effective_pass_modes: Vec<ParamPassMode>,
}

impl PendingOriginalBodyShadow {
    pub(super) fn new(
        semantic_owner: &FunctionDef,
        shadow_name: String,
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
            name: shadow_name,
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
        };
        Ok(Self {
            semantic_owner: semantic_owner.clone(),
            emission,
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
    /// Publish and compile a staged shadow after all handlers have completed.
    ///
    /// Registration deliberately follows the compiler's existing quarantine
    /// convention: a later analysis/emission error can leave a rejected
    /// compiler-issued slot behind. This method is not an artifact transaction.
    pub(super) fn finalize_pending_original_body_shadow(
        &mut self,
        pending: PendingOriginalBodyShadow,
    ) -> Result<()> {
        let PendingOriginalBodyShadow {
            semantic_owner,
            emission,
            inferred_reference_optimizations,
            effective_pass_modes,
        } = pending;

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
        self.with_body_analysis_authority(
            emission_id,
            &semantic_owner,
            &emission,
            |compiler| {
                compiler.compile_function_body_with_inferred_reference_optimizations(
                    &emission,
                    &inferred_reference_optimizations,
                )
            },
        )
    }
}

#[cfg(test)]
#[path = "original_body_shadow_tests.rs"]
mod tests;
