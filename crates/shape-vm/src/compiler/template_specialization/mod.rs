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
//! traversal core ([`pseudo_tuple`]). THIS stage (S1c stage 3) lands the
//! target-side glue ([`SpecializationTarget`]), the C3-G4 concrete degenerate
//! case (match-or-error, [`BytecodeCompiler::specialize_template`]), and the
//! application-site attribution producer
//! ([`BytecodeCompiler::template_application_error`], C3-G10). The POLYMORPHIC
//! arms (pseudo-tuple rewrite + `ensure_monomorphic_function_for_callsite`
//! ride) land in the next stage — until then they are named placeholders,
//! never a silent partial.
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

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::{CallableDescriptor, FrozenTypeIdentity};
use crate::compiler::comptime_fragments::checked_body::BodySignature;
use crate::compiler::comptime_fragments::checked_template::{
    CheckedTemplate, TemplateHookKind, TemplateSig,
};
use crate::compiler::comptime_target::type_annotation_to_string;
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
#[derive(Debug)]
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
            // Stage 4 territory: the pseudo-tuple rewrite + the
            // `ensure_monomorphic_function_for_callsite` ride (EMISSION tier +
            // MIR battery, C3-G10). Named placeholder — never a silent
            // partial.
            TemplateSig::PolymorphicArgs { .. } | TemplateSig::PolymorphicResult { .. } => {
                Err(self.template_application_error(
                    template,
                    target,
                    "polymorphic specialization lands in the next stage; only concrete template \
                     bodies specialize today",
                ))
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

    use super::*;
    use crate::compiler::comptime_builtins::ParamDescriptor;
    use crate::compiler::comptime_fragments::checked_template::CheckedTemplateBuilder;

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
