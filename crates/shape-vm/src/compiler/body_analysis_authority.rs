use super::{BytecodeCompiler, ParamPassMode};
use shape_ast::ast::FunctionDef;
use shape_ast::error::{Result, ShapeError};

/// Owned authority for semantic facts produced for one function body and
/// consumed while that identical body is emitted under another function id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveBodyAnalysisAuthority {
    pub(super) emission_function_id: usize,
    pub(super) semantic_owner_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MirRelevantStructureField {
    Parameters,
    Body,
    ReturnType,
    Async,
    Comptime,
    TypeParameters,
    WhereClause,
    DeclaringModule,
}

impl MirRelevantStructureField {
    const fn label(self) -> &'static str {
        match self {
            Self::Parameters => "parameters",
            Self::Body => "body and spans",
            Self::ReturnType => "return type",
            Self::Async => "async marker",
            Self::Comptime => "comptime marker",
            Self::TypeParameters => "type parameters",
            Self::WhereClause => "where clause",
            Self::DeclaringModule => "declaring module",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BodyAnalysisAuthorityError {
    EmissionIdentityMismatch {
        emission_function_id: usize,
        expected_name: String,
        registered_name: Option<String>,
    },
    StructureMismatch {
        semantic_owner: String,
        emission_name: String,
        field: MirRelevantStructureField,
    },
    PassModeCardinality {
        emission_name: String,
        parameter_count: usize,
        pass_mode_count: usize,
    },
}

impl std::fmt::Display for BodyAnalysisAuthorityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmissionIdentityMismatch {
                emission_function_id,
                expected_name,
                registered_name,
            } => write!(
                f,
                "emission id {emission_function_id} must identify '{expected_name}', found {}",
                registered_name.as_deref().unwrap_or("<missing>")
            ),
            Self::StructureMismatch {
                semantic_owner,
                emission_name,
                field,
            } => write!(
                f,
                "semantic owner '{semantic_owner}' and emission '{emission_name}' differ in MIR-relevant {}",
                field.label()
            ),
            Self::PassModeCardinality {
                emission_name,
                parameter_count,
                pass_mode_count,
            } => write!(
                f,
                "emission '{emission_name}' has {parameter_count} parameters but {pass_mode_count} authoritative pass modes"
            ),
        }
    }
}

impl From<BodyAnalysisAuthorityError> for ShapeError {
    fn from(error: BodyAnalysisAuthorityError) -> Self {
        ShapeError::RuntimeError {
            message: format!("internal body-analysis authority invariant: {error}"),
            location: None,
        }
    }
}

fn validate_mir_relevant_structure(
    semantic_owner: &FunctionDef,
    emission: &FunctionDef,
) -> std::result::Result<(), BodyAnalysisAuthorityError> {
    let mismatch = if semantic_owner.params != emission.params {
        Some(MirRelevantStructureField::Parameters)
    } else if semantic_owner.body != emission.body {
        Some(MirRelevantStructureField::Body)
    } else if semantic_owner.return_type != emission.return_type {
        Some(MirRelevantStructureField::ReturnType)
    } else if semantic_owner.is_async != emission.is_async {
        Some(MirRelevantStructureField::Async)
    } else if semantic_owner.is_comptime != emission.is_comptime {
        Some(MirRelevantStructureField::Comptime)
    } else if semantic_owner.type_params != emission.type_params {
        Some(MirRelevantStructureField::TypeParameters)
    } else if semantic_owner.where_clause != emission.where_clause {
        Some(MirRelevantStructureField::WhereClause)
    } else if semantic_owner.declaring_module_path != emission.declaring_module_path {
        Some(MirRelevantStructureField::DeclaringModule)
    } else {
        None
    };

    if let Some(field) = mismatch {
        return Err(BodyAnalysisAuthorityError::StructureMismatch {
            semantic_owner: semantic_owner.name.clone(),
            emission_name: emission.name.clone(),
            field,
        });
    }
    Ok(())
}

impl BytecodeCompiler {
    /// Resolve the semantic owner for facts about the body currently being
    /// emitted. The override is deliberately target-id checked: nested closure
    /// and generic compilation observes its own registered identity.
    pub(super) fn current_body_semantic_owner_key(&self) -> Option<&str> {
        let current_function = self.current_function?;
        if let Some(authority) = self.active_body_analysis_authority.as_ref()
            && authority.emission_function_id == current_function
        {
            return Some(authority.semantic_owner_key.as_str());
        }
        self.program
            .functions
            .get(current_function)
            .map(|function| function.name.as_str())
    }

    /// Apply the exact pass-mode vector to a just-registered hygienic emission.
    /// This updates only the emitted function's typed call metadata; it does
    /// not create a hygienic-name alias in inference maps.
    pub(super) fn refresh_authoritative_emission_metadata(
        &mut self,
        emission_function_id: usize,
        emission: &FunctionDef,
        semantic_owner_key: &str,
        pass_modes: &[ParamPassMode],
    ) -> Result<()> {
        let registered_name = self
            .program
            .functions
            .get(emission_function_id)
            .map(|function| function.name.clone());
        if registered_name.as_deref() != Some(emission.name.as_str()) {
            return Err(BodyAnalysisAuthorityError::EmissionIdentityMismatch {
                emission_function_id,
                expected_name: emission.name.clone(),
                registered_name,
            }
            .into());
        }
        if pass_modes.len() != emission.params.len() {
            return Err(BodyAnalysisAuthorityError::PassModeCardinality {
                emission_name: emission.name.clone(),
                parameter_count: emission.params.len(),
                pass_mode_count: pass_modes.len(),
            }
            .into());
        }

        let function = &mut self.program.functions[emission_function_id];
        function.ref_params = pass_modes.iter().map(|mode| mode.is_reference()).collect();
        function.ref_mutates = pass_modes.iter().map(|mode| mode.is_exclusive()).collect();

        // A wrapper calls the impl by its emission id. Project only the
        // resolved return carrier needed by that direct-call boundary; MIR,
        // inference, borrow, and storage maps remain source-keyed.
        if let Some(return_type) = self
            .type_tracker
            .get_function_return_concrete_type(semantic_owner_key)
            .cloned()
        {
            self.type_tracker
                .register_function_return_concrete_type(&emission.name, return_type);
        }
        Ok(())
    }

    /// Install body-analysis authority for one raw impl emission and restore
    /// the previous owned scope after either success or failure.
    pub(super) fn with_body_analysis_authority<T>(
        &mut self,
        emission_function_id: usize,
        semantic_owner: &FunctionDef,
        emission: &FunctionDef,
        compile: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let registered_name = self
            .program
            .functions
            .get(emission_function_id)
            .map(|function| function.name.clone());
        if registered_name.as_deref() != Some(emission.name.as_str()) {
            return Err(BodyAnalysisAuthorityError::EmissionIdentityMismatch {
                emission_function_id,
                expected_name: emission.name.clone(),
                registered_name,
            }
            .into());
        }
        validate_mir_relevant_structure(semantic_owner, emission)?;

        let previous = self
            .active_body_analysis_authority
            .replace(ActiveBodyAnalysisAuthority {
                emission_function_id,
                semantic_owner_key: semantic_owner.name.clone(),
            });
        let result = compile(self);
        self.active_body_analysis_authority = previous;
        result
    }
}

#[cfg(test)]
#[path = "body_analysis_authority_tests.rs"]
mod tests;
