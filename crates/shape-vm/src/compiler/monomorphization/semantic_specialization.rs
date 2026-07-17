//! Exact semantic specialization authority layered over physical ABI monomorphization.
//!
//! The existing ABI key remains useful for execution layout. It is not a
//! semantic type identity: two nominal or callable types may share that ABI.
//! Exact inference facts therefore enter a disjoint cache domain whose key is
//! extended only by identities issued by SemanticFreeze. Missing, unavailable,
//! or conflicting facts stay in the legacy execution domain and never borrow
//! an exact entry or overlay.

use std::sync::{Arc, Mutex, MutexGuard};

use shape_ast::ast::Span;
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_system::{
    ExactSemanticCallSiteFact, SemanticCallSiteFact, SemanticCallSiteKey, TypeVar,
};

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::semantic_freeze::{
    ClosedSemanticType, SpecializationTypeOverlay,
};

mod keys;
pub(crate) use keys::{
    FrozenLexicalContext, FrozenSemanticArgument, LegacySpecializationKey,
    SemanticSpecializationKey, SpecializationProgressKey,
};

/// Call-site evidence selected before entering the cache.
#[derive(Debug, Clone)]
pub(crate) enum SemanticSpecializationRequest {
    /// No exact fact is available. Only the legacy execution cache is legal.
    Legacy,
    /// Ordered, provenance-bearing arguments from inference.
    Exact(ExactSemanticCallSiteFact),
}

/// Fully frozen cache/overlay authority for one specialization attempt.
#[derive(Debug, Clone)]
pub(crate) enum PreparedSemanticSpecialization {
    Legacy {
        key: LegacySpecializationKey,
    },
    Exact {
        key: SemanticSpecializationKey,
        arguments: Vec<(TypeVar, ClosedSemanticType)>,
    },
}

impl PreparedSemanticSpecialization {
    pub(crate) fn specialized_symbol(&self, legacy_symbol: String) -> String {
        match self {
            Self::Legacy { key } => key.specialized_symbol(legacy_symbol),
            Self::Exact { key, .. } => key.specialized_symbol(),
        }
    }

    fn with_lexical_context(self, lexical_context: FrozenLexicalContext) -> Self {
        match self {
            Self::Legacy { key } => Self::Legacy {
                key: key.with_lexical_context(lexical_context),
            },
            Self::Exact { key, arguments } => Self::Exact {
                key: key.with_lexical_context(lexical_context),
                arguments,
            },
        }
    }

    pub(crate) fn overlay(
        &self,
        parameter_owner: &str,
        declared_names: &[String],
    ) -> Result<SpecializationTypeOverlay> {
        match self {
            Self::Legacy { .. } => Ok(SpecializationTypeOverlay::declaration_only(
                parameter_owner,
                declared_names.to_vec(),
            )),
            Self::Exact { arguments, .. } => {
                if arguments.len() != declared_names.len()
                    || arguments.iter().zip(declared_names).enumerate().any(
                        |(ordinal, ((declared, _), expected_name))| {
                            let Some(provenance) = declared.declared_provenance() else {
                                return true;
                            };
                            usize::try_from(provenance.ordinal()) != Ok(ordinal)
                                || provenance.source_name() != expected_name
                        },
                    )
                {
                    return Err(exact_specialization_error(
                        "exact call-site provenance does not match the callee's declared type parameters",
                    ));
                }
                SpecializationTypeOverlay::exact(
                    parameter_owner,
                    declared_names.to_vec(),
                    arguments.iter().cloned(),
                )
                .map_err(exact_specialization_error)
            }
        }
    }
}

impl BytecodeCompiler {
    pub(crate) fn specialization_type_param_names(
        base_def: &shape_ast::ast::FunctionDef,
    ) -> Vec<String> {
        base_def
            .type_params
            .as_ref()
            .map(|parameters| {
                parameters
                    .iter()
                    .filter(|parameter| !parameter.is_const())
                    .map(|parameter| parameter.name().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn declaration_only_specialization_overlay(
        base_name: &str,
        base_def: &shape_ast::ast::FunctionDef,
    ) -> SpecializationTypeOverlay {
        SpecializationTypeOverlay::declaration_only(
            base_name,
            Self::specialization_type_param_names(base_def),
        )
    }

    /// Read the one inference-owned fact for this exact structural call site.
    pub(crate) fn semantic_specialization_request(
        &self,
        callee: &str,
        call_span: Span,
    ) -> SemanticSpecializationRequest {
        let key = SemanticCallSiteKey::new(
            self.active_generated_node_stack.last().cloned(),
            callee,
            call_span,
        );
        match self.inference_facts.semantic_callsite_fact(&key) {
            Some(SemanticCallSiteFact::Exact(exact)) => {
                SemanticSpecializationRequest::Exact(exact.clone())
            }
            Some(SemanticCallSiteFact::Unavailable(_))
            | Some(SemanticCallSiteFact::Conflict(_))
            | None => SemanticSpecializationRequest::Legacy,
        }
    }

    /// Authenticate exact arguments against the inference-owned declaration.
    ///
    /// A forged, stale, or declaration-mismatched fact is quarantined as a
    /// hard compiler diagnostic. It never degrades into the ABI-only cache.
    pub(crate) fn validate_semantic_specialization_request(
        &self,
        callee: &str,
        expected_type_argument_count: usize,
        declared_names: Option<&[String]>,
        request: &SemanticSpecializationRequest,
    ) -> Result<()> {
        let SemanticSpecializationRequest::Exact(exact) = request else {
            return Ok(());
        };
        let arguments = exact.arguments();
        if arguments.is_empty() || arguments.len() != expected_type_argument_count {
            return Err(exact_specialization_error(format!(
                "exact call-site fact carries {} semantic arguments for {} ABI type arguments",
                arguments.len(),
                expected_type_argument_count,
            )));
        }
        let mut declared_owner = None;
        for (expected_ordinal, argument) in arguments.iter().enumerate() {
            let Some(provenance) = argument.declared().declared_provenance() else {
                return Err(exact_specialization_error(
                    "exact call-site argument omitted declared-TypeVar provenance",
                ));
            };
            if declared_owner.is_some_and(|owner| owner != provenance.owner()) {
                return Err(exact_specialization_error(
                    "exact call-site arguments do not share one declared-parameter owner",
                ));
            }
            declared_owner = Some(provenance.owner());
            if argument.ordinal() != provenance.ordinal()
                || usize::try_from(argument.ordinal()) != Ok(expected_ordinal)
                || argument.source_name() != provenance.source_name()
            {
                return Err(exact_specialization_error(
                    "exact call-site argument provenance does not match declaration order",
                ));
            }
        }
        let expected_declaration = self
            .inference_facts
            .semantic_callee_declaration(callee)
            .ok_or_else(|| {
                exact_specialization_error(format!(
                    "exact call-site fact for '{callee}' has no inference-issued callee declaration capability"
                ))
            })?;
        if !expected_declaration.matches_exact(exact) {
            return Err(exact_specialization_error(format!(
                "exact call-site provenance does not belong to the active declaration of '{callee}'"
            )));
        }
        if declared_names.is_some_and(|names| {
            arguments.len() != names.len()
                || arguments
                    .iter()
                    .zip(names)
                    .any(|(argument, name)| argument.source_name() != name)
        }) {
            return Err(exact_specialization_error(
                "exact call-site provenance does not match the callee's declared type parameters",
            ));
        }
        Ok(())
    }

    /// Authenticate and freeze exact arguments through the runtime authority.
    ///
    /// An allegedly exact candidate that cannot close is quarantined as a
    /// hard compiler diagnostic. It never degrades into the ABI-only cache.
    pub(crate) fn prepare_semantic_specialization(
        &self,
        callee: &str,
        abi_mono_key: String,
        expected_type_argument_count: usize,
        request: SemanticSpecializationRequest,
    ) -> Result<PreparedSemanticSpecialization> {
        self.validate_semantic_specialization_request(
            callee,
            expected_type_argument_count,
            None,
            &request,
        )?;
        let SemanticSpecializationRequest::Exact(exact) = request else {
            return Ok(PreparedSemanticSpecialization::Legacy {
                key: LegacySpecializationKey::new(abi_mono_key),
            });
        };
        let arguments = exact.arguments();
        let freeze = self.comptime_freeze_overlay().map_err(|error| {
            exact_specialization_error(format!(
                "SemanticFreeze handle could not be obtained: {error:?}"
            ))
        })?;
        let mut ordered_arguments = Vec::with_capacity(arguments.len());
        let mut overlay_arguments = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let closed = freeze
                .close_semantic_candidate(argument.candidate())
                .map_err(exact_specialization_error)?;
            let projection = closed.projection();
            ordered_arguments.push(FrozenSemanticArgument::new(
                projection.category(),
                projection.identity(),
            ));
            overlay_arguments.push((argument.declared().clone(), closed));
        }
        Ok(PreparedSemanticSpecialization::Exact {
            key: SemanticSpecializationKey::new(abi_mono_key, ordered_arguments),
            arguments: overlay_arguments,
        })
    }

    /// Prepare a closure-inlined specialization under the exact lexical
    /// Parameter environment whose AST is being spliced into the callee.
    pub(crate) fn prepare_lexical_inline_specialization(
        &self,
        callee: &str,
        abi_mono_key: String,
        expected_type_argument_count: usize,
        request: SemanticSpecializationRequest,
    ) -> Result<PreparedSemanticSpecialization> {
        let prepared = self.prepare_semantic_specialization(
            callee,
            abi_mono_key,
            expected_type_argument_count,
            request,
        )?;
        let lexical_context = if self.specialization_type_overlays.current().is_some() {
            let freeze = self.comptime_freeze_overlay().map_err(|error| {
                exact_specialization_error(format!(
                    "lexical Parameter context could not be obtained: {error:?}"
                ))
            })?;
            FrozenLexicalContext::new(freeze.lexical_parameter_identities().to_vec())
        } else {
            FrozenLexicalContext::default()
        };
        Ok(prepared.with_lexical_context(lexical_context))
    }
}

fn exact_specialization_error(detail: impl Into<String>) -> ShapeError {
    ShapeError::TypeError(format!(
        "C0911: exact semantic specialization evidence is unavailable: {}",
        detail.into()
    ))
}

/// Safe, drop-restored stack for nested specialization-body compilation.
///
/// The guard owns an `Arc` rather than borrowing the compiler field, so the
/// compiler may recursively compile while the frame is active. Poisoned locks
/// are recovered without panicking; a non-LIFO internal mismatch clears all
/// frames so stale exact evidence can never escape.
#[derive(Debug, Clone, Default)]
pub(crate) struct SpecializationTypeOverlayStack {
    frames: Arc<Mutex<Vec<SpecializationTypeOverlay>>>,
}

impl SpecializationTypeOverlayStack {
    pub(crate) fn enter(
        &self,
        overlay: SpecializationTypeOverlay,
    ) -> SpecializationTypeOverlayGuard {
        self.push(overlay)
    }

    /// Enter a body produced by lexically splicing caller-owned AST into a
    /// specialization (currently the closure-inlining route).
    ///
    /// Recursive compilation alone is not lexical nesting: ordinary callees
    /// must not see their caller's declared names or exact TypeVar map. This
    /// explicit route is the only place where those lexical capabilities are
    /// composed into the inner frame.
    pub(crate) fn enter_lexical_inline(
        &self,
        mut overlay: SpecializationTypeOverlay,
    ) -> SpecializationTypeOverlayGuard {
        let depth = {
            let mut frames = lock_unpoisoned(&self.frames);
            if let Some(outer) = frames.last() {
                overlay.inherit_for_lexical_inline(outer);
            }
            let depth = frames.len();
            frames.push(overlay);
            depth
        };
        SpecializationTypeOverlayGuard {
            frames: Arc::clone(&self.frames),
            depth,
        }
    }

    fn push(&self, overlay: SpecializationTypeOverlay) -> SpecializationTypeOverlayGuard {
        let depth = {
            let mut frames = lock_unpoisoned(&self.frames);
            let depth = frames.len();
            frames.push(overlay);
            depth
        };
        SpecializationTypeOverlayGuard {
            frames: Arc::clone(&self.frames),
            depth,
        }
    }

    pub(crate) fn current(&self) -> Option<SpecializationTypeOverlay> {
        lock_unpoisoned(&self.frames).last().cloned()
    }

    #[cfg(test)]
    pub(crate) fn depth(&self) -> usize {
        lock_unpoisoned(&self.frames).len()
    }
}

pub(crate) struct SpecializationTypeOverlayGuard {
    frames: Arc<Mutex<Vec<SpecializationTypeOverlay>>>,
    depth: usize,
}

impl Drop for SpecializationTypeOverlayGuard {
    fn drop(&mut self) {
        let mut frames = lock_unpoisoned(&self.frames);
        if frames.len() == self.depth + 1 {
            frames.pop();
        } else {
            // Fail closed: no stale outer or exact frame may remain visible.
            frames.clear();
        }
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod lexical_tests;
#[cfg(test)]
mod provenance_tests;
#[cfg(test)]
mod tests;
