//! ADR-009 C3 #14 (slice 1) — template specialization: the INSTALL-side twin of
//! the `CheckedTemplate` construction chokepoint.
//!
//! # The construction/install split (mirrors `comptime_fragments/checked_body.rs:8-35`)
//!
//! The C2/E1 checked-body surface established the binding split this module
//! inherits: `comptime_fragments::checked_template` is the CONSTRUCTION
//! chokepoint (typestate builder, `finish()` only on complete states, no string
//! constructor); THIS module is where a constructed template meets a frozen
//! target — per-specialization checking (C3-G4/G10: the template body fn
//! type-checked against the bound Sig, riding
//! `ensure_monomorphic_function_for_callsite`, EMISSION tier + MIR battery) and
//! the G9 pseudo-tuple resolution (constant `args[i]` → the i-th typed
//! parameter slot; `args.length` → a constant; mutation-return → a
//! compiler-internal per-target aggregate at the weave boundary).
//!
//! Slice-1 staging: S1b landed the module home + the single pseudo-tuple
//! traversal core ([`pseudo_tuple`]). S1c stage 3 landed the target-side glue
//! ([`SpecializationTarget`]), the C3-G4 concrete degenerate case
//! (match-or-error, [`BytecodeCompiler::specialize_template`]), and the
//! application-site attribution producer
//! ([`BytecodeCompiler::template_application_error`], C3-G10). S1c stage 4
//! landed the POLYMORPHIC arms: the G9 pseudo-tuple resolution
//! ([`pseudo_tuple::resolve_pseudo_tuple`]) riding
//! `ensure_monomorphic_template_specialization` — the ONE monomorphization
//! pipeline extended by an explicit plan parameter (no new pipeline, no
//! ambient state, no new `SemanticSpecializationRequest` variant: a template
//! plan is not call-site inference evidence).
//!
//! # Sig-source constraint (slice-0 report §7.4 — binding)
//!
//! Sig TYPES bind from the target `FunctionDef`'s AST-side annotations
//! (`annotation_param_type_annotation`, declared annotation first, guarded
//! inference fallback second) — NEVER from the freeze round-trip. The reason:
//! `reconstruct_type_annotation` (`comptime_builtins.rs:521-624`, the E1-D7
//! total inverse) NAMED-REJECTS Nominal/Record/Parameter identities until
//! B4/B5, so freeze-round-tripped TYPES would spuriously reject every
//! struct-param target. AST-side types + `declared_annotation_concrete_type`
//! handle structs today. The frozen [`CallableDescriptor`] on
//! [`SpecializationTarget`] is kept for IDENTITY/equality ONLY (derived
//! `PartialEq` over 128-bit `FrozenTypeIdentity`) — the match-or-error fast
//! path compares identities; it never reconstructs a type from one.
//!
//! # The S1 template contract is MINIMAL
//!
//! No `ctx` parameter (the E4 fence keeps HookDecision / failure-retry state /
//! `ctx.state` out of C3, and the S0 §2 ctx-object native smoke is unproven —
//! S2 owns any ctx growth, with its own native smoke). No config parameters
//! (config enters ONLY as ConstLift'd DECLARED captures — S3). A `before`
//! template binds the target's typed parameters; an `after` template binds the
//! target's typed result. Nothing else.
//!
//! # Transaction composition (E1-D6b — never a second transaction)
//!
//! [`BytecodeCompiler::specialize_template`] REQUIRES the ALREADY-OPEN C2
//! `InstallTransaction` (`begin_checked_body_install`,
//! `checked_body/mod.rs:207-212`, sets `install_journal = Some`): its first
//! statement rejects when no journal is live. It never opens a transaction of
//! its own — specialization output composes with the one open install exactly
//! as the `CheckedReplaceBody` shadow_export journaling precedent does.
//!
//! # Not a foundation: the legacy weave
//!
//! The legacy hook machinery (`compile_specialized_annotation_handler`,
//! `specialize_annotation_runtime_handlers`, `compile_annotation_wrapper`, the
//! homogeneous args array) is a C3-G7 DELETION target. Nothing in this module
//! may call into it, extend it, or depend on its carriers; the new path is
//! built BESIDE it and the S6 capstone deletes it whole. Error attribution
//! here deliberately inverts the legacy failure mode (the S0 g3 finding):
//! every rejection anchors at the `@application` site and names the
//! USER-SPELLED target and template signatures — never `handler.span`, never
//! a mangled mono-key name.

pub(in crate::compiler) mod pseudo_tuple;

use shape_ast::ast::{FunctionDef, Span, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use shape_value::v2::ConcreteType;

use self::pseudo_tuple::TemplateSpecializationPlan;
use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::{CallableDescriptor, FrozenTypeIdentity};
use crate::compiler::comptime_fragments::checked_body::BodySignature;
use crate::compiler::comptime_fragments::checked_template::{
    CheckedTemplate, TemplateHookKind, TemplateSig,
};
use crate::compiler::comptime_target::type_annotation_to_string;
use crate::compiler::monomorphization::cache::SpecializationFailure;
use crate::compiler::monomorphization::semantic_specialization::SemanticSpecializationRequest;
use crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type;

/// The frozen target a template specializes against: the USER-SPELLED name,
/// the AST-side parameter/return types (slice-0 §7.4 — never the freeze
/// round-trip), the frozen [`CallableDescriptor`] for IDENTITY/equality only,
/// and the `@application` span every rejection anchors at.
#[derive(Debug)]
pub(in crate::compiler) struct SpecializationTarget {
    /// The target's user-spelled function name (never a mangled mono-key —
    /// the S0 g3 failure mode is forbidden here).
    name: String,
    /// Per-parameter display name + AST-side type, in signature order.
    params: Vec<(String, TypeAnnotation)>,
    /// The target's DECLARED return annotation. Declared-only by design: a
    /// user-spelled annotation can never be the inference-loss `"unknown"`
    /// (that arises only from `Type::to_annotation` round-trips, which this
    /// carrier does not perform).
    return_type: Option<TypeAnnotation>,
    /// The frozen structural descriptor — IDENTITY/equality ONLY (module
    /// docs, slice-0 §7.4). Never a type source.
    descriptor: Option<CallableDescriptor>,
    /// The `@application` anchor.
    application_span: Span,
}

/// How a specialized `before` handler returns its typed args mutation
/// (C3-G9). `after` handlers flow the typed result directly and carry no
/// mutation carrier.
#[derive(Debug, Clone)]
pub(in crate::compiler) enum MutationCarrier {
    /// Arity-1 target: the mutated argument flows back as the bare typed
    /// value — no aggregate exists.
    Single { annotation: TypeAnnotation },
    /// Arity-n target (n > 1): the COMPILER-INTERNAL per-target aggregate at
    /// the weave boundary (C3-G9 — never user-visible, never user-spellable;
    /// reachable only through the polymorphic pseudo-tuple form, which lands
    /// in the next stage).
    Aggregate {
        fields: Vec<(String, TypeAnnotation)>,
    },
}

/// A successfully specialized handler: the template body fn's index in the
/// current compilation, plus the mutation carrier (`Some` for `before`,
/// `None` for `after`).
#[derive(Debug)]
pub(in crate::compiler) struct SpecializedHandler {
    function_index: u16,
    carrier: Option<MutationCarrier>,
}

impl SpecializedHandler {
    /// The specialized handler's function index.
    pub(in crate::compiler) fn function_index(&self) -> u16 {
        self.function_index
    }

    /// The mutation carrier (`Some` for `before` handlers, `None` for
    /// `after`).
    pub(in crate::compiler) fn carrier(&self) -> Option<&MutationCarrier> {
        self.carrier.as_ref()
    }
}

impl BytecodeCompiler {
    /// Build the [`SpecializationTarget`] glue from a target `FunctionDef`.
    ///
    /// Per-parameter types come from the EXISTING
    /// `annotation_param_type_annotation` (declared annotation guarded by
    /// `annotation_type_is_unknown`, else the `inference_facts`
    /// function-signature fallback under the same guard — the guard is
    /// load-bearing against the documented `TypeVar` → `"unknown"` loss,
    /// CLAUDE.md / `core.rs:218`). A parameter with NO resolvable type is a
    /// NAMED rejection at the `@application` site naming the parameter —
    /// surface-and-stop, never a guess (and never the S0 g3 mangled-name
    /// error).
    pub(in crate::compiler) fn specialization_target_from_def(
        &self,
        func_def: &FunctionDef,
        descriptor: Option<CallableDescriptor>,
        application_span: Span,
    ) -> Result<SpecializationTarget> {
        let mut params = Vec::with_capacity(func_def.params.len());
        for (idx, param) in func_def.params.iter().enumerate() {
            let display_name = param
                .simple_name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("<parameter {}>", idx + 1));
            let annotation = self
                .annotation_param_type_annotation(func_def, idx, param)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "cannot specialize an annotation template for `{}`: parameter `{}` \
                         (position {}) has no statically known type — neither a declared \
                         annotation nor an inferred signature provides one. Annotate the \
                         parameter with a concrete type to make the target specializable.",
                        func_def.name,
                        display_name,
                        idx + 1
                    ),
                    location: Some(self.span_to_source_location(application_span)),
                })?;
            params.push((display_name, annotation));
        }
        Ok(SpecializationTarget {
            name: func_def.name.clone(),
            params,
            return_type: func_def.return_type.clone(),
            descriptor,
            application_span,
        })
    }

    /// Specialize a [`CheckedTemplate`] against a frozen target (C3-G4/G10).
    ///
    /// COMPOSES with the ALREADY-OPEN C2 `InstallTransaction` (E1-D6b
    /// atomicity-by-composition) — the first statement rejects when
    /// `begin_checked_body_install` has not opened the journal; this function
    /// NEVER opens a second transaction.
    ///
    /// This stage implements the CONCRETE degenerate case (match-or-error at
    /// the `@application` site naming both signatures; the concrete body was
    /// already checked at definition under its own signature, so no re-check
    /// runs — `function_index` resolves the definition-compiled body). The
    /// polymorphic arms are named placeholders until the next stage.
    pub(in crate::compiler) fn specialize_template(
        &mut self,
        template: &CheckedTemplate,
        target: &SpecializationTarget,
    ) -> Result<SpecializedHandler> {
        if self.install_journal.is_none() {
            return Err(ShapeError::RuntimeError {
                message: "internal error: specialize_template requires the open checked-body \
                          install transaction (E1-D6b); call begin_checked_body_install before \
                          specializing — never a second transaction"
                    .to_string(),
                location: None,
            });
        }

        match template.sig() {
            // The C3-G4 polymorphic arms: the G9 pseudo-tuple resolution +
            // per-specialization checking riding the monomorphization
            // pipeline (EMISSION tier + MIR battery, C3-G10). The classifier
            // derives the variant FROM the hook kind, so PolymorphicArgs is
            // Before-only and PolymorphicResult is After-only by
            // construction.
            TemplateSig::PolymorphicArgs {
                type_param,
                args_param,
            } => {
                debug_assert_eq!(template.hook_kind(), TemplateHookKind::Before);
                let type_param = type_param.clone();
                let args_param = args_param.clone();
                self.specialize_polymorphic_before(template, target, &type_param, &args_param)
            }
            TemplateSig::PolymorphicResult { .. } => {
                debug_assert_eq!(template.hook_kind(), TemplateHookKind::After);
                self.specialize_polymorphic_after(template, target)
            }
            TemplateSig::Concrete(signature) => match template.hook_kind() {
                TemplateHookKind::Before => {
                    self.specialize_concrete_before(template, target, signature)
                }
                TemplateHookKind::After => {
                    self.specialize_concrete_after(template, target, signature)
                }
            },
        }
    }

    /// The C3-G4 polymorphic `before` case: build the
    /// [`TemplateSpecializationPlan`] (per-param carrier + G9 mutation
    /// carrier — `Single` over the one parameter type for a 1-ary target,
    /// the compiler-internal `Aggregate` otherwise) and ride
    /// `ensure_monomorphic_template_specialization`. The
    /// `ConcreteType::Tuple` type argument is the honest Sig identity for
    /// the cache: the same template applied at the same target Sig SHARES
    /// one specialization; a distinct Sig gets a distinct one. `Legacy` is
    /// the correct semantic request — `Exact` facts are inference-owned
    /// per-call-site evidence, which template application does not have.
    ///
    /// EVERY `SpecializationFailure` (Soft AND Hard — there is no generic
    /// fallback for templates, the C3-G10 hard-fail posture) wraps through
    /// [`Self::template_application_error`], preserving the inner detail
    /// text.
    fn specialize_polymorphic_before(
        &mut self,
        template: &CheckedTemplate,
        target: &SpecializationTarget,
        type_param: &str,
        args_param: &str,
    ) -> Result<SpecializedHandler> {
        if target.params.is_empty() {
            return Err(self.template_application_error(
                template,
                target,
                "the target declares no parameters, so a `before` template has no arguments to \
                 receive or mutate; remove the `before` template or use an `after` template on \
                 the target's result",
            ));
        }
        let mut param_concretes = Vec::with_capacity(target.params.len());
        for (position, (param_name, annotation)) in target.params.iter().enumerate() {
            let Some(concrete) = declared_annotation_concrete_type(self, annotation) else {
                return Err(self.template_application_error(
                    template,
                    target,
                    &format!(
                        "parameter {} (`{param_name}`): the required specialization's type `{}` \
                         (from the target's signature) does not resolve to a concrete type in \
                         this compilation; annotate the target with a concrete resolvable type",
                        position + 1,
                        type_annotation_to_string(annotation)
                    ),
                ));
            };
            param_concretes.push(concrete);
        }
        let carrier = if target.params.len() == 1 {
            MutationCarrier::Single {
                annotation: target.params[0].1.clone(),
            }
        } else {
            MutationCarrier::Aggregate {
                fields: target
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, (_, annotation))| (format!("a{index}"), annotation.clone()))
                    .collect(),
            }
        };
        let plan = TemplateSpecializationPlan {
            args_param: args_param.to_string(),
            type_param: type_param.to_string(),
            target_params: target.params.clone(),
            carrier: carrier.clone(),
        };
        let function_index = self
            .ensure_monomorphic_template_specialization(
                template.body_fn(),
                &[ConcreteType::Tuple(param_concretes)],
                SemanticSpecializationRequest::Legacy,
                &plan,
            )
            .map_err(|failure| {
                let detail = specialization_failure_detail(failure);
                self.template_application_error(template, target, &detail)
            })?;
        Ok(SpecializedHandler {
            function_index,
            carrier: Some(carrier),
        })
    }

    /// The C3-G4 polymorphic `after` case: the specialized def IS plain
    /// substitution of the template body at the target's result type, so it
    /// rides the plain, UNCHANGED
    /// `ensure_monomorphic_function_for_callsite` — no plan, no salt
    /// (sharing a cache entry/symbol with an ordinary generic instantiation
    /// at `R` is correct). Same hard-fail wrap as the `before` case.
    fn specialize_polymorphic_after(
        &mut self,
        template: &CheckedTemplate,
        target: &SpecializationTarget,
    ) -> Result<SpecializedHandler> {
        let required = match &target.return_type {
            Some(TypeAnnotation::Void) | None => {
                return Err(self.template_application_error(
                    template,
                    target,
                    "the target returns no value, so an `after` template has no typed result to \
                     receive; remove the `after` template or give the target a return type",
                ));
            }
            Some(annotation) => annotation,
        };
        let Some(result_concrete) = declared_annotation_concrete_type(self, required) else {
            return Err(self.template_application_error(
                template,
                target,
                &format!(
                    "the result type: the required specialization's type `{}` (from the \
                     target's signature) does not resolve to a concrete type in this \
                     compilation; annotate the target with a concrete resolvable type",
                    type_annotation_to_string(required)
                ),
            ));
        };
        let function_index = self
            .ensure_monomorphic_function_for_callsite(
                template.body_fn(),
                &[result_concrete],
                SemanticSpecializationRequest::Legacy,
            )
            .map_err(|failure| {
                let detail = specialization_failure_detail(failure);
                self.template_application_error(template, target, &detail)
            })?;
        Ok(SpecializedHandler {
            function_index,
            carrier: None,
        })
    }

    /// The C3-G4 concrete `before` case. Expected specialization signature:
    /// parameters positionally equal to the target's parameters; return type
    /// equal to the mutation carrier — [`MutationCarrier::Single`] over the
    /// target's one parameter type iff the target is 1-ary. A target of arity
    /// > 1 has NO user-spellable concrete return (the G9 aggregate is
    /// compiler-internal BY RULING) — named rejection with the polymorphic
    /// positive twin.
    fn specialize_concrete_before(
        &self,
        template: &CheckedTemplate,
        target: &SpecializationTarget,
        signature: &BodySignature,
    ) -> Result<SpecializedHandler> {
        match target.params.len() {
            0 => Err(self.template_application_error(
                template,
                target,
                "the target declares no parameters, so a `before` template has no arguments to \
                 receive or mutate; remove the `before` template or use an `after` template on \
                 the target's result",
            )),
            1 => {
                let (param_name, required) = &target.params[0];
                if signature.params().len() != 1 {
                    return Err(self.template_application_error(
                        template,
                        target,
                        &format!(
                            "the template declares {} value parameters but the required \
                             specialization declares exactly 1; declare the template with \
                             exactly the required specialization signature",
                            signature.params().len()
                        ),
                    ));
                }
                let required_identity = target.descriptor.as_ref().and_then(|descriptor| {
                    // Identity is usable only when the descriptor is
                    // positionally consistent with the AST-side params —
                    // otherwise ignore it (identity/equality only; never a
                    // guess).
                    (descriptor.params.len() == target.params.len())
                        .then(|| descriptor.params[0].type_identity)
                });
                self.require_specialization_position_match(
                    template,
                    target,
                    template_param_annotation(template, target, signature, 0, self)?,
                    required,
                    required_identity,
                    &format!("parameter 1 (`{param_name}`)"),
                )?;
                match signature.return_type() {
                    Some(template_return) => self.require_specialization_position_match(
                        template,
                        target,
                        template_return,
                        required,
                        required_identity,
                        "the return type (the typed args mutation carrier)",
                    )?,
                    None => {
                        return Err(self.template_application_error(
                            template,
                            target,
                            &format!(
                                "the template declares no return type but the required \
                                 specialization returns `{}` (a `before` template returns the \
                                 typed args mutation); declare the return type",
                                type_annotation_to_string(required)
                            ),
                        ));
                    }
                }
                Ok(SpecializedHandler {
                    function_index: self.template_body_function_index(template)?,
                    carrier: Some(MutationCarrier::Single {
                        annotation: required.clone(),
                    }),
                })
            }
            arity => Err(self.template_application_error(
                template,
                target,
                &format!(
                    "the target declares {arity} parameters and the {arity}-ary mutation \
                     carrier is compiler-internal (never user-spellable, C3-G9), so no concrete \
                     template body can match; declare the template polymorphic over the args \
                     pseudo-tuple (fn t<Args>(args: Args) -> Args)"
                ),
            )),
        }
    }

    /// The C3-G4 concrete `after` case. Expected specialization signature:
    /// `(R) -> R` where `R` is the target's declared return type. A target
    /// with no return value cannot host an `after` template — named rejection
    /// (surface-and-stop; S2 revisits).
    fn specialize_concrete_after(
        &self,
        template: &CheckedTemplate,
        target: &SpecializationTarget,
        signature: &BodySignature,
    ) -> Result<SpecializedHandler> {
        let required = match &target.return_type {
            Some(TypeAnnotation::Void) | None => {
                return Err(self.template_application_error(
                    template,
                    target,
                    "the target returns no value, so an `after` template has no typed result to \
                     receive; remove the `after` template or give the target a return type",
                ));
            }
            Some(annotation) => annotation,
        };
        if signature.params().len() != 1 {
            return Err(self.template_application_error(
                template,
                target,
                &format!(
                    "the template declares {} value parameters but the required specialization \
                     declares exactly 1 (the target's typed result); declare the template with \
                     exactly the required specialization signature",
                    signature.params().len()
                ),
            ));
        }
        let required_identity = target
            .descriptor
            .as_ref()
            .map(|descriptor| descriptor.returns);
        self.require_specialization_position_match(
            template,
            target,
            template_param_annotation(template, target, signature, 0, self)?,
            required,
            required_identity,
            "parameter 1 (the target's typed result)",
        )?;
        match signature.return_type() {
            Some(template_return) => self.require_specialization_position_match(
                template,
                target,
                template_return,
                required,
                required_identity,
                "the return type (the typed result flowing onward)",
            )?,
            None => {
                return Err(self.template_application_error(
                    template,
                    target,
                    &format!(
                        "the template declares no return type but the required specialization \
                         returns `{}` (an `after` template returns the typed result); declare \
                         the return type",
                        type_annotation_to_string(required)
                    ),
                ));
            }
        }
        Ok(SpecializedHandler {
            function_index: self.template_body_function_index(template)?,
            carrier: None,
        })
    }

    /// One compared position of the concrete match-or-error rule.
    ///
    /// FAST PATH — frozen-identity equality (slice-0 §7.4: the descriptor is
    /// for IDENTITY/equality ONLY): when the target side carries a frozen
    /// identity AND the template-side annotation canonicalizes under the
    /// installed freeze, derived `PartialEq` over `FrozenTypeIdentity` is the
    /// authority (the freeze is the ONE semantic type authority; aliases
    /// resolve, nominals intern). STRUCTURAL PATH otherwise — per-position
    /// [`declared_annotation_concrete_type`] on BOTH sides and `ConcreteType`
    /// `PartialEq`; a `None` resolution on EITHER side is a named rejection
    /// naming side + position, never a guess.
    fn require_specialization_position_match(
        &self,
        template: &CheckedTemplate,
        target: &SpecializationTarget,
        template_annotation: &TypeAnnotation,
        required_annotation: &TypeAnnotation,
        required_identity: Option<FrozenTypeIdentity>,
        position: &str,
    ) -> Result<()> {
        let mismatch = |compiler: &Self| {
            compiler.template_application_error(
                template,
                target,
                &format!(
                    "{position}: the template declares `{}` but the required specialization \
                     declares `{}`",
                    type_annotation_to_string(template_annotation),
                    type_annotation_to_string(required_annotation)
                ),
            )
        };

        if let Some(required_identity) = required_identity
            && let Ok(overlay) = self.comptime_freeze_overlay()
            && let Ok(template_identity) = overlay.canonicalize_type(template_annotation)
        {
            if template_identity == required_identity {
                return Ok(());
            }
            return Err(mismatch(self));
        }

        let Some(template_concrete) = declared_annotation_concrete_type(self, template_annotation)
        else {
            return Err(self.template_application_error(
                template,
                target,
                &format!(
                    "{position}: the template's declared type `{}` does not resolve to a \
                     concrete type in this compilation; declare the position with a concrete \
                     resolvable type",
                    type_annotation_to_string(template_annotation)
                ),
            ));
        };
        let Some(required_concrete) = declared_annotation_concrete_type(self, required_annotation)
        else {
            return Err(self.template_application_error(
                template,
                target,
                &format!(
                    "{position}: the required specialization's type `{}` (from the target's \
                     signature) does not resolve to a concrete type in this compilation; \
                     annotate the target with a concrete resolvable type",
                    type_annotation_to_string(required_annotation)
                ),
            ));
        };
        if template_concrete == required_concrete {
            Ok(())
        } else {
            Err(mismatch(self))
        }
    }

    /// Resolve the template body fn's index in the current compilation.
    /// Concrete template bodies compile at definition through the ordinary
    /// pipeline, so a registered template ALWAYS has an entry — a miss is an
    /// internal invariant error, never a user rejection.
    fn template_body_function_index(&self, template: &CheckedTemplate) -> Result<u16> {
        let index = self
            .find_function(template.body_fn())
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "internal error: template body fn `{}` is not registered in the current \
                     compilation (a CheckedTemplate is compiler-session-local; its body fn must \
                     be registered before specialization)",
                    template.body_fn()
                ),
                location: None,
            })?;
        u16::try_from(index).map_err(|_| ShapeError::RuntimeError {
            message: format!(
                "internal error: function index {index} for template body fn `{}` exceeds the \
                 u16 function-index space",
                template.body_fn()
            ),
            location: None,
        })
    }

    /// The SINGLE application-site attribution producer (C3-G10 — the
    /// genuinely-new piece). Every user-facing rejection of the
    /// specialization flow routes through here: a `SemanticError` anchored at
    /// the `@application` span (never `handler.span`, never a mangled
    /// mono-key name — the S0 g3 failure mode), naming BOTH signatures — the
    /// template's declared form and the required specialization signature
    /// rendered from the target. Precedent: `directive_signature_type_error`
    /// (`functions_annotations.rs:3644-3660`), re-anchored at the
    /// application site.
    fn template_application_error(
        &self,
        template: &CheckedTemplate,
        target: &SpecializationTarget,
        detail: &str,
    ) -> ShapeError {
        ShapeError::SemanticError {
            message: format!(
                "annotation template `{}` (declared `{}`) cannot specialize for target `{}` \
                 (required specialization signature `{}`): {}",
                template.body_fn(),
                render_template_declared_signature(template),
                target.name,
                render_required_specialization_signature(template.hook_kind(), target),
                detail
            ),
            location: Some(self.span_to_source_location(target.application_span)),
        }
    }
}

/// The template-side annotation at parameter `index`, or a named rejection: a
/// concrete template body declares its parameter types explicitly (an
/// unannotated parameter is a `None` resolution on the template side — named,
/// never guessed).
fn template_param_annotation<'sig>(
    template: &CheckedTemplate,
    target: &SpecializationTarget,
    signature: &'sig BodySignature,
    index: usize,
    compiler: &BytecodeCompiler,
) -> Result<&'sig TypeAnnotation> {
    signature.params()[index].type_annotation.as_ref().ok_or_else(|| {
        compiler.template_application_error(
            template,
            target,
            &format!(
                "parameter {} of the template has no declared type annotation; a concrete \
                 template body annotates every parameter explicitly",
                index + 1
            ),
        )
    })
}

/// Extract the inner detail text of a monomorphization-ride failure so the
/// two-signature application-site attribution PRESERVES it (C3-G10). Soft
/// and Hard both surface — there is no generic fallback for templates.
fn specialization_failure_detail(failure: SpecializationFailure) -> String {
    match failure.into_error() {
        ShapeError::SemanticError { message, .. } => message,
        ShapeError::TypeError(message) => message,
        ShapeError::RuntimeError { message, .. } => message,
        other => other.to_string(),
    }
}

/// Render the template's DECLARED form for attribution messages. The
/// polymorphic forms render literally in their `<T>(p: T) -> T` shape (with
/// the template's own spellings); the concrete form renders per-parameter via
/// the existing `type_annotation_to_string`.
fn render_template_declared_signature(template: &CheckedTemplate) -> String {
    match template.sig() {
        TemplateSig::PolymorphicArgs {
            type_param,
            args_param,
        } => format!("<{type_param}>({args_param}: {type_param}) -> {type_param}"),
        TemplateSig::PolymorphicResult {
            type_param,
            result_param,
        } => format!("<{type_param}>({result_param}: {type_param}) -> {type_param}"),
        TemplateSig::Concrete(signature) => {
            let params = signature
                .params()
                .iter()
                .map(|param| match &param.type_annotation {
                    Some(annotation) => type_annotation_to_string(annotation),
                    None => "<unannotated>".to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            match signature.return_type() {
                Some(annotation) => {
                    format!("({params}) -> {}", type_annotation_to_string(annotation))
                }
                None => format!("({params})"),
            }
        }
    }
}

/// Render the REQUIRED specialization signature from the target. For a
/// `before` hook over an arity-n target (n > 1) the aggregate return renders
/// as the tuple NOTATION `(T0, ..., Tn-1)` in MESSAGE TEXT ONLY — no tuple
/// value or tuple surface is implied (C3-G9: the aggregate is
/// compiler-internal; #63 tracks a first-class tuple surface).
fn render_required_specialization_signature(
    hook_kind: TemplateHookKind,
    target: &SpecializationTarget,
) -> String {
    match hook_kind {
        TemplateHookKind::Before => {
            let params: Vec<String> = target
                .params
                .iter()
                .map(|(_, annotation)| type_annotation_to_string(annotation))
                .collect();
            let rendered_params = params.join(", ");
            let carrier = match params.len() {
                0 => "()".to_string(),
                1 => params[0].clone(),
                _ => format!("({})", params.join(", ")),
            };
            format!("({rendered_params}) -> {carrier}")
        }
        TemplateHookKind::After => {
            let result = target
                .return_type
                .as_ref()
                .filter(|annotation| !matches!(annotation, TypeAnnotation::Void))
                .map(type_annotation_to_string)
                .unwrap_or_else(|| "void".to_string());
            format!("({result}) -> {result}")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use shape_ast::ast::{CaptureClause, FunctionDef, Item};
    use shape_value::{KindedSlot, NativeKind, ValueSlot};

    use super::*;
    use crate::compiler::comptime_builtins::ParamDescriptor;
    use crate::compiler::comptime_fragments::checked_template::CheckedTemplateBuilder;
    use crate::compiler::monomorphization::cache::TEMPLATE_SPECIALIZATION_KEY_SALT;
    use crate::executor::{VMConfig, VirtualMachine};

    /// The route_tests harness pattern (`monomorphization/cache/route_tests.rs:40-83`):
    /// parse → `infer_reference_model_with_comptime_context` → fresh compiler
    /// + `inference_facts` + `register_function` + `install_semantic_freeze`.
    struct Fixture {
        compiler: BytecodeCompiler,
        defs: HashMap<String, FunctionDef>,
    }

    fn fixture(source: &str) -> Fixture {
        let program = shape_ast::parse_program(source).expect("fixture parses");
        let (_, _, _, facts) =
            BytecodeCompiler::infer_reference_model_with_comptime_context(&program, false);
        let mut compiler = BytecodeCompiler::new();
        compiler.inference_facts = facts;
        compiler.source_text = Some(source.to_string());
        let mut defs = HashMap::new();
        for item in &program.items {
            if let Item::Function(def, _) = item {
                compiler
                    .register_function(def)
                    .expect("fixture fn registers");
                defs.insert(def.name.clone(), def.clone());
            }
        }
        compiler
            .install_semantic_freeze()
            .expect("fixture freeze installs");
        Fixture { compiler, defs }
    }

    fn empty_captures() -> CaptureClause {
        CaptureClause {
            entries: Vec::new(),
            span: Span::default(),
        }
    }

    fn template(fixture: &Fixture, hook_kind: TemplateHookKind, name: &str) -> CheckedTemplate {
        CheckedTemplateBuilder::new(hook_kind)
            .body_fn(fixture.defs.get(name).expect("template fn in fixture"))
            .expect("template classifies")
            .captures(empty_captures())
            .finish()
            .expect("template finishes")
    }

    /// Build the target via the production glue, anchored at the target fn's
    /// `name_span` (a real span into the fixture source, standing in for the
    /// `@application` span the S2 caller threads).
    fn target_for(
        fixture: &Fixture,
        name: &str,
        descriptor: Option<CallableDescriptor>,
    ) -> SpecializationTarget {
        let def = fixture.defs.get(name).expect("target fn in fixture");
        fixture
            .compiler
            .specialization_target_from_def(def, descriptor, def.name_span)
            .expect("target glue builds")
    }

    /// Open the C2 install transaction around the specialization (E1-D6b
    /// composition: commit = drop token + clear journal; failure = rollback)
    /// — the production discipline, mirrored.
    fn specialize(
        compiler: &mut BytecodeCompiler,
        template: &CheckedTemplate,
        target: &SpecializationTarget,
    ) -> Result<SpecializedHandler> {
        let transaction = compiler.begin_checked_body_install();
        match compiler.specialize_template(template, target) {
            Ok(handler) => {
                drop(transaction);
                compiler.install_journal = None;
                Ok(handler)
            }
            Err(err) => {
                compiler.rollback_checked_body_install(transaction);
                Err(err)
            }
        }
    }

    /// Assert the G10 attribution contract: a `SemanticError` whose message
    /// names BOTH rendered signatures and whose location anchors at the
    /// `@application` span's line.
    fn assert_names_both_signatures_at_line(
        err: &ShapeError,
        template_rendered: &str,
        required_rendered: &str,
        expected_line: usize,
    ) {
        let ShapeError::SemanticError { message, location } = err else {
            panic!("expected a SemanticError attribution, got: {err}");
        };
        assert!(
            message.contains(template_rendered),
            "message must name the template's declared signature {template_rendered:?}, got: \
             {message}"
        );
        assert!(
            message.contains(required_rendered),
            "message must name the required specialization signature {required_rendered:?}, \
             got: {message}"
        );
        let location = location
            .as_ref()
            .expect("attribution must anchor at the application site");
        assert_eq!(
            location.line, expected_line,
            "attribution must anchor at the @application span's line"
        );
    }

    // HAPPY PATH: concrete `before` on a 1-ary int target — index + Single
    // carrier.
    #[test]
    fn concrete_before_unary_match_yields_index_and_single_carrier() {
        let src = "fn tmpl(x: int) -> int { return x * 2 }\n\
                   fn target_fn(a: int) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let handler = specialize(&mut fx.compiler, &template, &target)
            .expect("matching concrete before specializes");
        let expected_index = fx
            .compiler
            .find_function("tmpl")
            .expect("template body fn registered");
        assert_eq!(handler.function_index() as usize, expected_index);
        match handler.carrier() {
            Some(MutationCarrier::Single { annotation }) => {
                assert_eq!(type_annotation_to_string(annotation), "int");
            }
            other => panic!("expected the Single mutation carrier, got {:?}", other.is_some()),
        }
    }

    // HAPPY PATH: concrete `after` (R) -> R against the target's return.
    #[test]
    fn concrete_after_result_match_specializes_with_no_carrier() {
        let src = "fn post(r: number) -> number { return r + 1.0 }\n\
                   fn target_fn(a: int) -> number { return 2.0 }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::After, "post");
        let target = target_for(&fx, "target_fn", None);

        let handler = specialize(&mut fx.compiler, &template, &target)
            .expect("matching concrete after specializes");
        assert!(handler.carrier().is_none(), "after handlers carry no mutation carrier");
        let expected_index = fx.compiler.find_function("post").expect("post registered");
        assert_eq!(handler.function_index() as usize, expected_index);
    }

    // NEGATIVE (G10 attribution): parameter-count mismatch names BOTH
    // signatures and anchors at the application span.
    #[test]
    fn param_count_mismatch_names_both_signatures_at_the_application_site() {
        let src = "fn tmpl(x: int, y: int) -> int { return x + y }\n\
                   fn target_fn(a: int) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("2-ary template cannot match a 1-ary required specialization");
        // target_fn is declared on line 2 of the fixture source.
        assert_names_both_signatures_at_line(&err, "(int, int) -> int", "(int) -> int", 2);
        assert!(
            err.to_string().contains("declares 2 value parameters"),
            "detail must name the count mismatch: {err}"
        );
    }

    // NEGATIVE (G10 attribution): per-position type mismatch names BOTH
    // signatures, the position, and anchors at the application span.
    #[test]
    fn per_position_type_mismatch_names_both_signatures() {
        let src = "fn tmpl(x: string) -> string { return x }\n\
                   fn target_fn(a: int) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("string template cannot match an int target");
        assert_names_both_signatures_at_line(&err, "(string) -> string", "(int) -> int", 2);
        assert!(
            err.to_string().contains("parameter 1"),
            "detail must name the mismatching position: {err}"
        );
    }

    // NEGATIVE (G9): a multi-param target has no user-spellable concrete
    // return — rejection carries the polymorphic positive twin and renders
    // the aggregate in tuple NOTATION (message text only).
    #[test]
    fn multi_param_concrete_before_rejects_with_the_polymorphic_twin() {
        let src = "fn tmpl(x: int) -> int { return x }\n\
                   fn target_fn(a: int, b: number) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("multi-param concrete before must reject");
        assert_names_both_signatures_at_line(
            &err,
            "(int) -> int",
            "(int, number) -> (int, number)",
            2,
        );
        assert!(
            err.to_string()
                .contains("declare the template polymorphic over the args pseudo-tuple \
                           (fn t<Args>(args: Args) -> Args)"),
            "rejection must carry the polymorphic positive twin: {err}"
        );
    }

    // NEGATIVE: a position that resolves on neither path (no descriptor, and
    // `declared_annotation_concrete_type` has no Function projection) is a
    // NAMED rejection naming side + position — never a guess.
    #[test]
    fn unresolvable_template_position_without_descriptor_is_a_named_rejection() {
        let src = "fn tmpl(f: (int) => int) -> (int) => int { return f }\n\
                   fn target_fn(g: (int) => int) -> int { return g(1) }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("a structurally unresolvable position must reject without a descriptor");
        assert!(
            err.to_string()
                .contains("does not resolve to a concrete type"),
            "expected the named unresolvable rejection: {err}"
        );
        assert!(
            err.to_string().contains("parameter 1"),
            "rejection must name the position: {err}"
        );
    }

    // NEGATIVE (required side): the template side resolves but the target's
    // type does not — the rejection names the REQUIRED side + position.
    #[test]
    fn unresolvable_required_position_names_the_target_side() {
        let src = "fn tmpl(r: int) -> int { return r }\n\
                   fn target_fn(a: int) -> (int) => int { return |v: int| v }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::After, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("an unresolvable required position must reject");
        assert!(
            err.to_string()
                .contains("the required specialization's type"),
            "rejection must name the required side: {err}"
        );
    }

    // NEGATIVE: `after` on a void-returning target — surface-and-stop (S2
    // revisits).
    #[test]
    fn void_return_target_rejects_an_after_template() {
        let src = "fn post(r: int) -> int { return r }\n\
                   fn target_fn(a: int) { let b = a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::After, "post");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("an after template on a void target must reject");
        assert!(
            err.to_string().contains("no typed result to receive"),
            "expected the named void-return rejection: {err}"
        );
        assert!(
            err.to_string().contains("give the target a return type"),
            "rejection must carry the positive twin: {err}"
        );
    }

    // NEGATIVE: `before` on a zero-param target — nothing to bind or mutate.
    #[test]
    fn zero_param_target_rejects_a_before_template() {
        let src = "fn tmpl(x: int) -> int { return x }\n\
                   fn target_fn() -> int { return 7 }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("a before template on a zero-param target must reject");
        assert!(
            err.to_string().contains("declares no parameters"),
            "expected the named zero-param rejection: {err}"
        );
    }

    // TRANSACTION COMPOSITION (E1-D6b): specialize_template REQUIRES the
    // already-open C2 InstallTransaction — never a second transaction, never
    // a silent run outside one.
    #[test]
    fn specialize_template_requires_the_open_install_transaction() {
        let src = "fn tmpl(x: int) -> int { return x }\n\
                   fn target_fn(a: int) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        assert!(fx.compiler.install_journal.is_none(), "control: no journal open");
        let err = fx
            .compiler
            .specialize_template(&template, &target)
            .expect_err("specialization outside the install transaction must reject");
        assert!(
            err.to_string()
                .contains("requires the open checked-body install transaction"),
            "expected the named internal transaction error: {err}"
        );
    }

    // INTERNAL INVARIANT: a template naming an unregistered body fn is an
    // internal error at index resolution (construction is session-local; the
    // registry is the ONE function source).
    #[test]
    fn unregistered_template_body_fn_is_an_internal_error() {
        let src = "fn target_fn(a: int) -> int { return a }\n";
        let mut fx = fixture(src);
        // Parse the template fn OUTSIDE the fixture program so it is never
        // registered with the compiler.
        let orphan = shape_ast::parse_program("fn orphan(x: int) -> int { return x }")
            .expect("orphan parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                Item::Function(def, _) => Some(def),
                _ => None,
            })
            .expect("orphan fn");
        let template = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&orphan)
            .expect("orphan classifies")
            .captures(empty_captures())
            .finish()
            .expect("orphan finishes");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("an unregistered body fn must be an internal error");
        assert!(
            err.to_string().contains("is not registered"),
            "expected the named internal registry error: {err}"
        );
    }

    // FAST PATH (positive proof): the SAME shape the structural path rejects
    // (function-typed positions have no ConcreteType projection) is ACCEPTED
    // when the target carries its frozen descriptor and the template side
    // canonicalizes — identity equality (derived PartialEq) is the authority.
    // Paired with `unresolvable_template_position_without_descriptor...`,
    // this proves the descriptor fast path FIRED (the structural path cannot
    // accept this fixture).
    #[test]
    fn descriptor_fast_path_identity_equality_accepts_frozen_positions() {
        use shape_runtime::comptime_reflection::PassingMode;

        let src = "fn tmpl(f: (int) => int) -> (int) => int { return f }\n\
                   fn target_fn(g: (int) => int) -> int { return g(1) }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");

        let overlay = fx
            .compiler
            .comptime_freeze_overlay()
            .expect("fixture freeze installed");
        let target_def = fx.defs.get("target_fn").expect("target in fixture");
        let param_annotation = target_def.params[0]
            .type_annotation
            .as_ref()
            .expect("target param annotated");
        let param_identity = overlay
            .canonicalize_type(param_annotation)
            .expect("function type canonicalizes");
        let int_identity = overlay.identity_of("int").expect("int identity");
        let descriptor = CallableDescriptor {
            params: vec![ParamDescriptor {
                name: Some("g".to_string()),
                type_identity: param_identity,
                optional: false,
                mode: PassingMode::Move,
            }],
            returns: int_identity,
        };
        let target = target_for(&fx, "target_fn", Some(descriptor));

        let handler = specialize(&mut fx.compiler, &template, &target)
            .expect("frozen-identity fast path must accept the matching function-typed position");
        assert!(matches!(
            handler.carrier(),
            Some(MutationCarrier::Single { .. })
        ));
    }

    // FAST PATH (negative proof): identity INEQUALITY produces the same
    // two-signature application-site attribution as the structural path.
    #[test]
    fn descriptor_fast_path_identity_mismatch_names_both_signatures() {
        use shape_runtime::comptime_reflection::PassingMode;

        let src = "fn tmpl(x: number) -> number { return x }\n\
                   fn target_fn(a: int) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");

        let overlay = fx
            .compiler
            .comptime_freeze_overlay()
            .expect("fixture freeze installed");
        let int_identity = overlay.identity_of("int").expect("int identity");
        let descriptor = CallableDescriptor {
            params: vec![ParamDescriptor {
                name: Some("a".to_string()),
                type_identity: int_identity,
                optional: false,
                mode: PassingMode::Move,
            }],
            returns: int_identity,
        };
        let target = target_for(&fx, "target_fn", Some(descriptor));

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("number template cannot match the frozen int identity");
        assert_names_both_signatures_at_line(&err, "(number) -> number", "(int) -> int", 2);
    }

    // TARGET GLUE: a target parameter with no statically known type is a
    // NAMED rejection at the application site naming the parameter (the S0
    // g3 mangled-name error must never recur).
    #[test]
    fn target_param_without_static_type_is_a_named_application_site_rejection() {
        let src = "fn helper(a: int) -> int { return a }\n";
        let fx = fixture(src);
        // Build a def with a deliberately unresolvable parameter: strip the
        // annotation from a clone so neither the declared annotation nor the
        // (absent) inference signature provides a type.
        let mut def = fx.defs.get("helper").expect("helper in fixture").clone();
        def.name = "stripped".to_string();
        def.params[0].type_annotation = None;
        let err = fx
            .compiler
            .specialization_target_from_def(&def, None, def.name_span)
            .expect_err("a type-less target param must reject");
        let ShapeError::SemanticError { message, location } = &err else {
            panic!("expected a SemanticError, got: {err}");
        };
        assert!(
            message.contains("parameter `a` (position 1) has no statically known type"),
            "rejection must name the parameter: {message}"
        );
        assert!(
            message.contains("`stripped`"),
            "rejection must name the user-spelled target, never a mangled name: {message}"
        );
        assert!(location.is_some(), "rejection must anchor at the application site");
    }

    // =====================================================================
    // S1c stage 4 — the polymorphic arms: G9 resolution riding the
    // monomorphization pipeline (C3-G10), execution-proven per the S0 §2
    // named uncertainty ("compile-proof alone is insufficient").
    // =====================================================================

    /// Route-tests-style clean-state pin (route_tests.rs:89-92).
    fn assert_clean_specialization_state(compiler: &BytecodeCompiler) {
        assert!(compiler.monomorphization_in_progress.is_empty());
        assert_eq!(compiler.specialization_type_overlays.depth(), 0);
    }

    fn int_arg(value: i64) -> KindedSlot {
        KindedSlot::new(ValueSlot::from_raw(value as u64), NativeKind::Int64)
    }

    fn number_arg(value: f64) -> KindedSlot {
        KindedSlot::new(ValueSlot::from_raw(value.to_bits()), NativeKind::Float64)
    }

    /// EXECUTE a specialized handler in the VM (never compile-proof alone):
    /// load the compiler's program into a fresh VM and call the handler by
    /// its function index with typed args.
    fn execute_specialized(
        compiler: &BytecodeCompiler,
        function_index: u16,
        args: Vec<KindedSlot>,
    ) -> KindedSlot {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(compiler.program.clone());
        vm.execute_function_by_id(function_index, args, None)
            .expect("the specialized handler must execute in the VM")
    }

    /// Read the executed aggregate result POSITIONALLY through the typed
    /// storage carrier (`KindedSlot::as_typed_object_storage`): slot `i` is
    /// field `a{i}` — the positional weave contract (C3-G9). The field
    /// NAMES are pinned at the AST level by the pseudo_tuple rewrite tests;
    /// here the execution pin reads per-slot kind + raw bits directly (no
    /// schema-registry projection — the fresh test VM has no comptime
    /// registry to consult).
    fn aggregate_slots(result: &KindedSlot) -> Vec<(NativeKind, u64)> {
        let storage = result
            .as_typed_object_storage()
            .expect("the aggregate result must be the ordinary inline-schema TypedObject");
        storage
            .field_kinds
            .iter()
            .copied()
            .zip(storage.slots().iter().map(|slot| slot.raw()))
            .collect()
    }

    fn registered_specialized_def<'c>(
        compiler: &'c BytecodeCompiler,
        function_index: u16,
    ) -> &'c FunctionDef {
        let name = &compiler.program.functions[function_index as usize].name;
        compiler
            .function_defs
            .get(name)
            .expect("the specialized def must be registered")
    }

    // END-TO-END (2-ary aggregate): before-template mutates slot 0, passes
    // slot 1 through; the executed result is the compiler-internal typed
    // aggregate with a0 mutated and a1 untouched.
    #[test]
    fn polymorphic_before_two_ary_executes_the_aggregate_mutation() {
        let src = "fn tmpl<Args>(args: Args) -> Args {\n\
                   \x20   args[0] = args[0] + 1\n\
                   \x20   return args\n\
                   }\n\
                   fn target_fn(a: int, b: number) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let handler = specialize(&mut fx.compiler, &template, &target)
            .expect("the polymorphic before template specializes");
        match handler.carrier() {
            Some(MutationCarrier::Aggregate { fields }) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0, "a0");
                assert_eq!(fields[1].0, "a1");
                assert_eq!(type_annotation_to_string(&fields[0].1), "int");
                assert_eq!(type_annotation_to_string(&fields[1].1), "number");
            }
            other => panic!("expected the aggregate carrier, got {:?}", other),
        }

        // The registered symbol carries the template salt; the registered
        // def is fully resolved — the transient post-substitution Tuple
        // annotation must never survive to checking or emission (C3-G9).
        let index = handler.function_index();
        let spec_def = registered_specialized_def(&fx.compiler, index);
        assert!(
            spec_def.name.contains("c3_before_hook"),
            "the specialized symbol must carry the template salt: {}",
            spec_def.name
        );
        assert!(
            spec_def
                .params
                .iter()
                .all(|p| !matches!(p.type_annotation, Some(TypeAnnotation::Tuple(_)))),
            "no Tuple parameter annotation may survive resolution"
        );
        assert!(
            !matches!(spec_def.return_type, Some(TypeAnnotation::Tuple(_))),
            "no Tuple return annotation may survive resolution"
        );

        let result = execute_specialized(&fx.compiler, index, vec![int_arg(3), number_arg(4.5)]);
        let slots = aggregate_slots(&result);
        assert_eq!(slots.len(), 2, "the aggregate carries one slot per target parameter");
        assert_eq!(slots[0].0, NativeKind::Int64, "a0 is the typed int slot");
        assert_eq!(slots[0].1 as i64, 4, "a0 must carry the mutated int");
        assert_eq!(slots[1].0, NativeKind::Float64, "a1 is the typed number slot");
        assert_eq!(
            f64::from_bits(slots[1].1),
            4.5,
            "a1 must pass the number through untouched"
        );
        assert_clean_specialization_state(&fx.compiler);
    }

    // END-TO-END (1-ary Single carrier): the mutated argument flows back as
    // the bare typed value — no aggregate exists.
    #[test]
    fn polymorphic_before_unary_executes_with_the_single_carrier() {
        let src = "fn tmpl<Args>(args: Args) -> Args {\n\
                   \x20   args[0] = args[0] * 3\n\
                   \x20   return args\n\
                   }\n\
                   fn target_fn(a: int) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let handler = specialize(&mut fx.compiler, &template, &target)
            .expect("the unary polymorphic before template specializes");
        match handler.carrier() {
            Some(MutationCarrier::Single { annotation }) => {
                assert_eq!(type_annotation_to_string(annotation), "int");
            }
            other => panic!("expected the Single carrier, got {:?}", other),
        }

        let result =
            execute_specialized(&fx.compiler, handler.function_index(), vec![int_arg(3)]);
        assert_eq!(
            result.as_i64(),
            Some(9),
            "the single-carrier handler must return the mutated bare int"
        );
        assert_clean_specialization_state(&fx.compiler);
    }

    // `args.length` resolves to the target-arity CONSTANT — executed, not
    // just compiled: a0 = 3 + length = 5 proves the constant was 2.
    #[test]
    fn args_length_resolves_to_the_target_arity_constant() {
        let src = "fn tmpl<Args>(args: Args) -> Args {\n\
                   \x20   args[0] = args[0] + args.length\n\
                   \x20   return args\n\
                   }\n\
                   fn target_fn(a: int, b: number) -> number { return b }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let handler = specialize(&mut fx.compiler, &template, &target)
            .expect("the length-using template specializes");
        let result = execute_specialized(
            &fx.compiler,
            handler.function_index(),
            vec![int_arg(3), number_arg(4.5)],
        );
        let slots = aggregate_slots(&result);
        assert_eq!(slots[0].0, NativeKind::Int64);
        assert_eq!(
            slots[0].1 as i64,
            5,
            "a0 = 3 + args.length proves length resolved to the constant 2"
        );
        assert_clean_specialization_state(&fx.compiler);
    }

    // Polymorphic AFTER at two distinct R types: distinct specializations,
    // both executed; the plain (unsalted) monomorphization ride is correct
    // for After — the specialized def IS plain substitution.
    #[test]
    fn polymorphic_after_executes_at_two_distinct_result_types() {
        let src = "fn post<R>(result: R) -> R { return result }\n\
                   fn target_int(a: int) -> int { return a }\n\
                   fn target_num(a: int) -> number { return 2.0 }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::After, "post");

        let target_int = target_for(&fx, "target_int", None);
        let handler_int = specialize(&mut fx.compiler, &template, &target_int)
            .expect("after template specializes at int");
        assert!(handler_int.carrier().is_none());

        let target_num = target_for(&fx, "target_num", None);
        let handler_num = specialize(&mut fx.compiler, &template, &target_num)
            .expect("after template specializes at number");

        assert_ne!(
            handler_int.function_index(),
            handler_num.function_index(),
            "distinct result types get distinct specializations"
        );
        let int_name = &fx.compiler.program.functions[handler_int.function_index() as usize].name;
        assert!(
            !int_name.contains("c3_before_hook"),
            "the After ride is unsalted — plain substitution shares with ordinary generic \
             instantiation: {int_name}"
        );

        let result_int =
            execute_specialized(&fx.compiler, handler_int.function_index(), vec![int_arg(7)]);
        assert_eq!(result_int.as_i64(), Some(7));
        let result_num = execute_specialized(
            &fx.compiler,
            handler_num.function_index(),
            vec![number_arg(2.5)],
        );
        assert_eq!(result_num.as_f64(), Some(2.5));
        assert_clean_specialization_state(&fx.compiler);
    }

    // CACHE identity (the honest Sig identity): same template + same target
    // Sig ⇒ ONE shared specialization; a distinct Sig ⇒ its own. The
    // specialized symbol carries the c3_before_hook salt.
    #[test]
    fn same_sig_shares_one_specialization_distinct_sig_gets_its_own() {
        let src = "fn tmpl<Args>(args: Args) -> Args {\n\
                   \x20   args[0] = args[0] + 1\n\
                   \x20   return args\n\
                   }\n\
                   fn target_one(a: int, b: number) -> int { return a }\n\
                   fn target_two(x: int, y: number) -> number { return y }\n\
                   fn target_three(a: int, b: string) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");

        let target_one = target_for(&fx, "target_one", None);
        let first = specialize(&mut fx.compiler, &template, &target_one)
            .expect("first target specializes");
        assert_eq!(fx.compiler.monomorphization_cache.legacy_len(), 1);
        assert!(
            fx.compiler.program.functions[first.function_index() as usize]
                .name
                .contains(TEMPLATE_SPECIALIZATION_KEY_SALT.trim_start_matches("::")),
            "the specialized symbol must carry the salt"
        );
        assert_clean_specialization_state(&fx.compiler);

        let target_two = target_for(&fx, "target_two", None);
        let second = specialize(&mut fx.compiler, &template, &target_two)
            .expect("same-Sig target specializes");
        assert_eq!(
            first.function_index(),
            second.function_index(),
            "the same template + same Sig must share ONE specialization"
        );
        assert_eq!(
            fx.compiler.monomorphization_cache.legacy_len(),
            1,
            "a same-Sig application is a cache hit"
        );
        assert_clean_specialization_state(&fx.compiler);

        let target_three = target_for(&fx, "target_three", None);
        let third = specialize(&mut fx.compiler, &template, &target_three)
            .expect("distinct-Sig target specializes");
        assert_ne!(
            first.function_index(),
            third.function_index(),
            "a distinct Sig must get its own specialization"
        );
        assert_eq!(fx.compiler.monomorphization_cache.legacy_len(), 2);
        assert_clean_specialization_state(&fx.compiler);
    }

    // THE g6 FAILURE MODE FIXED (C3-G10): a body type error at
    // specialization is a HARD error wrapped with BOTH signatures at the
    // `@application` span — never an anonymous error deep in the hook body,
    // never a generic fallback. Fixture = the emission-tier strict-proof
    // class (arithmetic on a bool-typed pseudo-slot — exactly the g6
    // per-instantiation error family). Two MEASURED near-miss fixtures are
    // surfaced in the stage report as observations about the G10
    // emission-tier boundary: `args[0].trim()` on an int slot and
    // `args[0] = "boom"` into an int slot BOTH compile at this seam today
    // (unknown-method dispatch defers to runtime; a local re-assignment
    // re-stamps the slot kind at emission) — battery row 1 (the
    // whole-program analyzer) does not re-run per specialization BY RULING
    // (slice-0 §7.3), so the emission tier's strict-proof classes are the
    // per-specialization checking surface.
    #[test]
    fn body_type_error_wraps_with_both_signatures_at_the_application_site() {
        let src = "fn tmpl<Args>(args: Args) -> Args {\n\
                   \x20   args[0] = args[0] + 1\n\
                   \x20   return args\n\
                   }\n\
                   fn target_fn(a: bool) -> bool { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("arithmetic on a bool-typed pseudo-slot must hard-fail");
        // target_fn is declared on line 5 of the fixture source.
        assert_names_both_signatures_at_line(
            &err,
            "<Args>(args: Args) -> Args",
            "(bool) -> bool",
            5,
        );
        assert!(
            err.to_string()
                .contains("Cannot infer types for binary operation `Add`"),
            "the inner detail text must be preserved verbatim: {err}"
        );
        assert_eq!(
            fx.compiler.monomorphization_cache.legacy_len(),
            0,
            "a failed specialization must not leave a cache entry"
        );
        assert_clean_specialization_state(&fx.compiler);
    }

    // G9 out-of-range constant index: rejected at specialization naming the
    // index, the target arity + signature, AND both signatures at the
    // application site.
    #[test]
    fn out_of_range_index_rejects_naming_index_arity_and_both_signatures() {
        let src = "fn tmpl<Args>(args: Args) -> Args {\n\
                   \x20   args[7] = 1\n\
                   \x20   return args\n\
                   }\n\
                   fn target_fn(a: int, b: number) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("an out-of-range constant index must reject at specialization");
        assert_names_both_signatures_at_line(
            &err,
            "<Args>(args: Args) -> Args",
            "(int, number) -> (int, number)",
            5,
        );
        assert!(
            err.to_string().contains("index 7 is out of range"),
            "must quote the index: {err}"
        );
        assert!(
            err.to_string().contains("declares 2 parameters"),
            "must quote the target arity: {err}"
        );
        assert_eq!(
            fx.compiler.monomorphization_cache.legacy_len(),
            0,
            "the rejection fires before any cache publication"
        );
        assert_clean_specialization_state(&fx.compiler);
    }

    // ROLLBACK PROBE (the stage-named hazard): the C2 rollback set
    // (checked_body/mod.rs) truncates `program.functions` to the watermark
    // but did NOT include `monomorphization_cache` — a pre-rollback cache
    // entry would survive pointing at a truncated index, and a re-run of the
    // SAME specialization would cache-hit the dangling index. PROBED
    // 2026-07-20: stale-index reuse CONFIRMED (the re-specialize returned
    // the truncated index against a shrunk function table). Fold-in applied
    // per the disclosed protocol: `rollback_checked_body_install` now evicts
    // every cache entry (both domains) whose index is at/above the
    // functions watermark, so the re-run below re-registers FRESH at the
    // same (freed) index and the cache stays index-consistent.
    #[test]
    fn rollback_evicts_the_specialization_cache_and_a_rerun_reregisters_fresh() {
        let src = "fn tmpl<Args>(args: Args) -> Args {\n\
                   \x20   args[0] = args[0] + 1\n\
                   \x20   return args\n\
                   }\n\
                   fn target_fn(a: int, b: number) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let functions_before = fx.compiler.program.functions.len();
        let transaction = fx.compiler.begin_checked_body_install();
        let first = fx
            .compiler
            .specialize_template(&template, &target)
            .expect("the first specialization succeeds inside the transaction");
        let first_index = first.function_index();
        assert!(fx.compiler.program.functions.len() > functions_before);
        assert_eq!(fx.compiler.monomorphization_cache.legacy_len(), 1);

        fx.compiler.rollback_checked_body_install(transaction);
        assert_eq!(
            fx.compiler.program.functions.len(),
            functions_before,
            "rollback truncates the function table to the watermark"
        );
        assert_eq!(
            fx.compiler.monomorphization_cache.legacy_len(),
            0,
            "rollback must evict the at/above-watermark cache entry (the fold-in)"
        );

        // Re-run the same specialization in the same compiler: a fresh
        // registration at the freed index, executed to prove it is real.
        let handler = specialize(&mut fx.compiler, &template, &target)
            .expect("the re-run re-specializes fresh after rollback");
        assert_eq!(
            handler.function_index(),
            first_index,
            "the re-run re-registers at the freed watermark index"
        );
        assert!(fx.compiler.program.functions.len() > functions_before);
        assert_eq!(fx.compiler.monomorphization_cache.legacy_len(), 1);
        let result = execute_specialized(
            &fx.compiler,
            handler.function_index(),
            vec![int_arg(3), number_arg(4.5)],
        );
        let slots = aggregate_slots(&result);
        assert_eq!(slots[0].1 as i64, 4, "the re-specialized handler executes correctly");
        assert_clean_specialization_state(&fx.compiler);
    }

    // Missing template return annotation on a concrete before body: named
    // rejection carrying the required carrier type.
    #[test]
    fn missing_template_return_annotation_is_a_named_rejection() {
        let src = "fn tmpl(x: int) { let y = x }\n\
                   fn target_fn(a: int) -> int { return a }\n";
        let mut fx = fixture(src);
        let template = template(&fx, TemplateHookKind::Before, "tmpl");
        let target = target_for(&fx, "target_fn", None);

        let err = specialize(&mut fx.compiler, &template, &target)
            .expect_err("a return-less concrete before template must reject");
        assert!(
            err.to_string().contains("declares no return type"),
            "expected the named missing-return rejection: {err}"
        );
        assert!(
            err.to_string().contains("typed args mutation"),
            "rejection must explain the before-carrier requirement: {err}"
        );
    }
}
