//! Exact frozen callable evidence carried by generated capture packs.

use super::super::semantic_freeze::{FreezeOverlay, FrozenSemanticTypeProjection};
use crate::compiler::{BytecodeCompiler, ParamPassMode};
use shape_ast::ast::{FunctionParameter, GeneratedNodeOrigin};
use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::type_system::{
    GeneratedCallableFact, GeneratedNodeKey, SemanticPassingMode, SemanticTypeCandidate, Type,
};

/// Frozen semantic identity of the closure value described by one pack.
#[derive(Debug, Clone)]
pub(crate) struct CallableSemanticType(FrozenSemanticTypeProjection);

impl CallableSemanticType {
    fn from_projection(
        projection: FrozenSemanticTypeProjection,
    ) -> std::result::Result<Self, String> {
        if projection.category() != FrozenTypeCategory::Callable {
            return Err(format!(
                "semantic freeze returned {:?} for a callable projection",
                projection.category()
            ));
        }
        Ok(Self(projection))
    }

    pub(crate) fn category(&self) -> FrozenTypeCategory {
        self.0.category()
    }

    pub(crate) fn identity_components(&self) -> (i64, i64) {
        let identity = self.0.identity();
        (identity.high, identity.low)
    }

    pub(crate) fn presentation(&self) -> &str {
        self.0.presentation()
    }
}

impl PartialEq for CallableSemanticType {
    fn eq(&self, other: &Self) -> bool {
        self.category() == other.category()
            && self.identity_components() == other.identity_components()
    }
}

impl Eq for CallableSemanticType {}

impl std::hash::Hash for CallableSemanticType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.category().catalog_ordinal(), state);
        std::hash::Hash::hash(&self.identity_components(), state);
    }
}

impl PartialOrd for CallableSemanticType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CallableSemanticType {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.category().catalog_ordinal(),
            self.identity_components(),
        )
            .cmp(&(
                other.category().catalog_ordinal(),
                other.identity_components(),
            ))
    }
}

/// Stable reason why a pack cannot expose exact callable semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CallableSemanticIssueKind {
    OrdinarySource,
    MissingInferenceFact,
    InferenceUnavailable,
    InferenceConflict,
    MissingSemanticFreeze,
    NotCallable,
    ArityMismatch,
    PassingModeConflict,
    OptionalityConflict,
    CallableShapeUnavailable,
    FreezeRejected,
}

/// Deterministic typed evidence failure. `detail` is diagnostic-only and never
/// participates in specialization identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallableSemanticIssue {
    kind: CallableSemanticIssueKind,
    detail: String,
}

impl CallableSemanticIssue {
    fn new(kind: CallableSemanticIssueKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    pub(crate) fn kind(&self) -> CallableSemanticIssueKind {
        self.kind
    }

    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }
}

/// A pack never collapses conflict and unavailability into `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallableSemanticEvidence {
    Exact(CallableSemanticType),
    Unavailable(CallableSemanticIssue),
    Conflict(CallableSemanticIssue),
}

impl CallableSemanticEvidence {
    pub(crate) fn unavailable(kind: CallableSemanticIssueKind, detail: impl Into<String>) -> Self {
        Self::Unavailable(CallableSemanticIssue::new(kind, detail))
    }
}

impl BytecodeCompiler {
    /// Project one validated generated closure's finalized inference fact into
    /// a frozen callable identity. Failure affects compiler queries only;
    /// ordinary closure emission continues through its existing ABI path.
    pub(crate) fn callable_semantic_evidence(
        &self,
        origin: Option<&GeneratedNodeOrigin>,
        params: &[FunctionParameter],
        pass_modes: &[ParamPassMode],
    ) -> CallableSemanticEvidence {
        let Some(origin) = origin else {
            return CallableSemanticEvidence::unavailable(
                CallableSemanticIssueKind::OrdinarySource,
                "ordinary source closure has no generated-capture query occurrence",
            );
        };
        let key = GeneratedNodeKey::from_origin(origin);
        let Some(fact) = self.inference_facts.generated_callable_fact(&key) else {
            return CallableSemanticEvidence::unavailable(
                CallableSemanticIssueKind::MissingInferenceFact,
                "generated closure has no structural callable inference fact",
            );
        };
        let candidate = match fact {
            GeneratedCallableFact::Exact(candidate) => candidate,
            GeneratedCallableFact::Unavailable(issue) => {
                return CallableSemanticEvidence::unavailable(
                    CallableSemanticIssueKind::InferenceUnavailable,
                    issue.detail(),
                );
            }
            GeneratedCallableFact::Conflict(issue) => {
                return CallableSemanticEvidence::Conflict(CallableSemanticIssue::new(
                    CallableSemanticIssueKind::InferenceConflict,
                    issue.detail(),
                ));
            }
        };
        let freeze = match self.comptime_freeze_overlay() {
            Ok(freeze) => freeze,
            Err(error) => {
                return CallableSemanticEvidence::unavailable(
                    CallableSemanticIssueKind::MissingSemanticFreeze,
                    format!("generated closure has no semantic-freeze projection: {error}"),
                );
            }
        };
        match freeze_exact_callable(candidate, params, pass_modes, &freeze) {
            Ok(callable) => CallableSemanticEvidence::Exact(callable),
            Err(issue) => CallableSemanticEvidence::Unavailable(issue),
        }
    }
}

fn freeze_exact_callable(
    candidate: &SemanticTypeCandidate,
    params: &[FunctionParameter],
    pass_modes: &[ParamPassMode],
    freeze: &FreezeOverlay,
) -> std::result::Result<CallableSemanticType, CallableSemanticIssue> {
    let Type::Function {
        params: inferred_params,
        returns: _,
    } = candidate.ty()
    else {
        return Err(CallableSemanticIssue::new(
            CallableSemanticIssueKind::NotCallable,
            "structural generated-callable fact is not a function type",
        ));
    };
    if inferred_params.len() != params.len() || params.len() != pass_modes.len() {
        return Err(CallableSemanticIssue::new(
            CallableSemanticIssueKind::ArityMismatch,
            format!(
                "callable evidence arity mismatch: inference={}, syntax={}, modes={}",
                inferred_params.len(),
                params.len(),
                pass_modes.len()
            ),
        ));
    }
    let shape = candidate
        .recursive_callable_shape()
        .callable_at(&[])
        .ok_or_else(|| {
            CallableSemanticIssue::new(
                CallableSemanticIssueKind::CallableShapeUnavailable,
                "generated callable fact omitted its root callable-shape evidence",
            )
        })?;
    if shape.parameters().len() != params.len() {
        return Err(CallableSemanticIssue::new(
            CallableSemanticIssueKind::ArityMismatch,
            format!(
                "callable shape arity mismatch: shape={}, syntax={}",
                shape.parameters().len(),
                params.len()
            ),
        ));
    }
    for (index, ((semantic, syntax), mode)) in shape
        .parameters()
        .iter()
        .zip(params)
        .zip(pass_modes)
        .enumerate()
    {
        if semantic.optional() != syntax.default_value.is_some() {
            return Err(CallableSemanticIssue::new(
                CallableSemanticIssueKind::OptionalityConflict,
                format!(
                    "callable parameter {index} optionality disagrees between inference and syntax"
                ),
            ));
        }
        if semantic.passing_mode() != semantic_mode(*mode) {
            return Err(CallableSemanticIssue::new(
                CallableSemanticIssueKind::PassingModeConflict,
                format!(
                    "callable parameter {index} passing mode disagrees between inference and effective compiler mode"
                ),
            ));
        }
    }

    let annotation = freeze
        .semantic_candidate_annotation(candidate)
        .map_err(|detail| {
            CallableSemanticIssue::new(CallableSemanticIssueKind::FreezeRejected, detail)
        })?;
    let projection = freeze
        .canonicalize_type_projection(&annotation)
        .map_err(|detail| {
            CallableSemanticIssue::new(CallableSemanticIssueKind::FreezeRejected, detail)
        })?;
    CallableSemanticType::from_projection(projection).map_err(|detail| {
        CallableSemanticIssue::new(CallableSemanticIssueKind::FreezeRejected, detail)
    })
}

fn semantic_mode(mode: ParamPassMode) -> SemanticPassingMode {
    match mode {
        ParamPassMode::ByValue => SemanticPassingMode::ByValue,
        ParamPassMode::ByRefShared => SemanticPassingMode::SharedBorrow,
        ParamPassMode::ByRefExclusive => SemanticPassingMode::ExclusiveBorrow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::{DestructurePattern, Span};
    use shape_ast::parser::parse_program;
    use shape_runtime::type_system::{
        SemanticCallSiteFact, SemanticTypeCandidate, TypeInferenceEngine,
    };

    fn parameter(name: &str, optional: bool, mode: ParamPassMode) -> FunctionParameter {
        FunctionParameter {
            pattern: DestructurePattern::Identifier(name.to_string(), Span::new(1, 2)),
            is_const: false,
            is_reference: !matches!(mode, ParamPassMode::ByValue),
            is_mut_reference: matches!(mode, ParamPassMode::ByRefExclusive),
            is_out: false,
            type_annotation: None,
            default_value: optional.then(|| {
                shape_ast::ast::Expr::Literal(shape_ast::ast::Literal::Int(1), Span::new(3, 4))
            }),
        }
    }

    fn callable_candidate(closure: &str) -> SemanticTypeCandidate {
        let program = parse_program(&format!(
            "fn retain<F>(value: F) -> F {{ value }}\nlet retained = retain({closure})"
        ))
        .expect("callable candidate fixture parses");
        let mut inference = TypeInferenceEngine::new();
        let (facts, errors) = inference.infer_program_facts_best_effort(&program);
        assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
        facts
            .semantic_callsite_facts()
            .values()
            .find_map(|fact| match fact {
                SemanticCallSiteFact::Exact(exact) => exact
                    .arguments()
                    .first()
                    .map(|argument| argument.candidate().clone()),
                SemanticCallSiteFact::Unavailable(_) | SemanticCallSiteFact::Conflict(_) => None,
            })
            .expect("generic call publishes an exact callable candidate")
    }

    #[test]
    fn optionality_and_passing_mode_are_semantic_identity() {
        let compiler = BytecodeCompiler::new();
        let overlay = super::super::super::semantic_freeze::overlay_for_tests(&compiler);
        let required_candidate = callable_candidate("fn(value: int) { return \"ok\" }");
        let optional_candidate = callable_candidate("fn(value: int = 1) { return \"ok\" }");
        let borrowed_candidate = callable_candidate("fn(&value: int) { return \"ok\" }");
        let exclusive_candidate = callable_candidate("fn(&mut value: int) { return \"ok\" }");
        let required = freeze_exact_callable(
            &required_candidate,
            &[parameter("value", false, ParamPassMode::ByValue)],
            &[ParamPassMode::ByValue],
            &overlay,
        )
        .expect("required callable must freeze");
        let optional = freeze_exact_callable(
            &optional_candidate,
            &[parameter("value", true, ParamPassMode::ByValue)],
            &[ParamPassMode::ByValue],
            &overlay,
        )
        .expect("optional callable must freeze");
        let borrowed = freeze_exact_callable(
            &borrowed_candidate,
            &[parameter("value", false, ParamPassMode::ByRefShared)],
            &[ParamPassMode::ByRefShared],
            &overlay,
        )
        .expect("borrowed callable must freeze");
        let exclusive = freeze_exact_callable(
            &exclusive_candidate,
            &[parameter("value", false, ParamPassMode::ByRefExclusive)],
            &[ParamPassMode::ByRefExclusive],
            &overlay,
        )
        .expect("exclusive callable must freeze");

        assert_ne!(required, optional);
        assert_ne!(required, borrowed);
        assert_ne!(optional, borrowed);
        assert_ne!(borrowed, exclusive);
    }

    #[test]
    fn parameter_names_do_not_change_callable_identity() {
        let compiler = BytecodeCompiler::new();
        let overlay = super::super::super::semantic_freeze::overlay_for_tests(&compiler);
        let candidate = callable_candidate("|value: int| \"ok\"");
        let left = freeze_exact_callable(
            &candidate,
            &[parameter("left", false, ParamPassMode::ByValue)],
            &[ParamPassMode::ByValue],
            &overlay,
        )
        .expect("left callable must freeze");
        let right = freeze_exact_callable(
            &candidate,
            &[parameter("right", false, ParamPassMode::ByValue)],
            &[ParamPassMode::ByValue],
            &overlay,
        )
        .expect("right callable must freeze");

        assert_eq!(left, right);
    }

    #[test]
    fn nested_callable_passing_mode_is_semantic_identity() {
        let compiler = BytecodeCompiler::new();
        let overlay = super::super::super::semantic_freeze::overlay_for_tests(&compiler);
        let value = callable_candidate("|callback: (value: int) => string| \"ok\"");
        let borrowed = callable_candidate("|callback: (value: &int) => string| \"ok\"");
        let syntax = [parameter("callback", false, ParamPassMode::ByValue)];
        let value = freeze_exact_callable(&value, &syntax, &[ParamPassMode::ByValue], &overlay)
            .expect("nested by-value callable freezes");
        let borrowed =
            freeze_exact_callable(&borrowed, &syntax, &[ParamPassMode::ByValue], &overlay)
                .expect("nested borrowed callable freezes");

        assert_ne!(value, borrowed);
    }
}
