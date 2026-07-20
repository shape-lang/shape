//! Pure annotation installation planning.

use std::collections::{BTreeMap, BTreeSet};

use shape_ast::ast::{
    AnnotationDef, AnnotationHandlerType, AnnotationTargetKind, FunctionParameter, Item,
};
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;

use super::annotation_definition;

pub(super) struct AnnotationInstallationPlan {
    pub(super) pending: Vec<PlannedAnnotation>,
}

pub(super) struct PlannedAnnotation {
    pub(super) definition: AnnotationDef,
    pub(super) allowed_targets: Vec<AnnotationTargetKind>,
    pub(super) surface_class: AnnotationSurfaceClass,
    /// ADR-009 C3 #14 (slice 4, S4c): the sugar lowering's artifacts for a
    /// TypedConfig definition with declarative hooks — validated (R3) and
    /// built HERE at planning, stored on `CompiledAnnotation` by the
    /// installer. `None` for Legacy definitions and for TypedConfig
    /// definitions without declarative hooks.
    pub(super) sugar: Option<super::sugar_lowering::SugarLowering>,
}

// ═══════════════════════════════════════════════════════════════════════════
// ADR-009 C3 #14 (slice 4) — THE G7-COMPLIANT TRANSITIONAL CLASSIFICATION
// ═══════════════════════════════════════════════════════════════════════════
//
// This is the ONE ratified transitional duality of C3: until S6 deletes the
// legacy declarative before/after weave, every `annotation` definition is
// classified by a compile-time SYNTACTIC rule into exactly one of two
// surface classes, decided AT THE DECLARATION (before any `@application`):
//
//   - **TypedConfig** — the definition declares >= 1 config parameter
//     carrying a type annotation (`annotation retry(times: int, label:
//     string)`). ALL config parameters must then be typed; a mix of typed
//     and untyped parameters is the declaration-site rejection R2. Every
//     typed parameter's annotation must lie within the C3-G5 ConstLift
//     domain, checked at the declaration by the ONE domain producer
//     `const_lift::annotation_within_lift_domain` (rejection R1) — a
//     non-liftable config type is a named error before any application.
//     TypedConfig definitions' declarative `before`/`after` handlers lower
//     onto the PUBLIC comptime API (CheckedTemplate + install; the S4c
//     sugar lowering — see `sugar_lowering` for the BINDING RULES of the
//     typed hook surface: `before(args)` / `after(result)` polymorphic
//     forms, `before()` / `after()` observers, the R3 shape rejection, and
//     the R3-family lifecycle-hook rejection) and must NEVER engage the
//     legacy weave slots.
//
//   - **Legacy** — zero config parameters, or all parameters untyped. The
//     declarative `before`/`after` handlers keep the BYTE-UNCHANGED legacy
//     runtime-hook weave until S6 deletes it (C3-G7: build new path → flip
//     the sugar → rewrite the 48 annotation pins → pure-deletion capstone).
//
// ZERO-PARAM RULING (resolved in S4 design): NO opt-in marker is minted —
// zero-param definitions classify Legacy until S6, at which point the
// Legacy arm and the untyped param spelling are DELETED and every
// annotation (zero-param included) is new-path for free. C3-G0 forbids
// minting throwaway grammar that S6 must then delete; new-path e2e
// coverage in S4 uses typed-config definitions (the mixed-type shape S4
// exists to prove), while public-API-spelled installs inside zero-param
// definitions are already green (sugar-matrix rows r2/r3/r9).
//
// COMPTIME pre/post handlers are CLASSIFICATION-INDEPENDENT: they execute
// through the comptime mini-VM on both classes (the r-matrix zero-param
// fixtures like `annotation scaled(factor)` keep working untouched); typed
// injection simply gives TypedConfig handler params their declared
// annotations.
//
// PIN-GREENNESS (by construction): the grammar REJECTED typed config
// params before this slice, so no pre-existing green program or pin can
// classify TypedConfig — verified at S4b:
// `rg "annotation \w+\([^)]*:" tools/shape-test/tests/` has zero hits.
//
// NAMED S6 CLOSE: S6 deletes the Legacy arm (and this enum with it, or
// collapses it to the single new-path class), the legacy weave fns
// (`compile_specialized_annotation_handler`,
// `specialize_annotation_runtime_handlers`, `compile_annotation_wrapper`)
// and the untyped-param spelling, then rewrites the 48 legacy pins onto
// the typed surface (c3-decisions.md C3-G7; the S2 F1-F5 ledger carries
// the per-pin arithmetic).
//
// SEALED (the C1 CaptureKind "constructible in ONE file" precedent): each
// variant carries a `SurfaceClassEvidence` token whose field is private to
// THIS module, so `AnnotationSurfaceClass::TypedConfig(..)` is
// unconstructible outside planner.rs — the classification function below
// is the single producer. Consumers match on `TypedConfig(_)` / `Legacy(_)`.

/// Evidence token gating construction of [`AnnotationSurfaceClass`] variants
/// to this file. The unit field is private: only planner.rs can spell
/// `SurfaceClassEvidence(())`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::compiler) struct SurfaceClassEvidence(());

/// The G7-compliant transitional surface class of one `annotation`
/// definition. See the module-level classification doc-comment above —
/// the rule, the zero-param ruling, and the named S6 close live there.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::compiler) enum AnnotationSurfaceClass {
    /// >= 1 typed config param (then ALL must be typed; R2 otherwise), all
    /// annotations inside the ConstLift domain (R1 otherwise). Declarative
    /// before/after handlers lower onto the public comptime API.
    TypedConfig(SurfaceClassEvidence),
    /// Zero config params or all params untyped. Declarative before/after
    /// handlers keep the byte-unchanged legacy weave until S6.
    Legacy(SurfaceClassEvidence),
}

/// THE classification chokepoint (single producer of
/// [`AnnotationSurfaceClass`]). Fires the R2 mixed-params rejection; the
/// R1 ConstLift-domain check runs in [`plan_definition`] on TypedConfig
/// definitions only.
pub(in crate::compiler) fn classify_annotation_surface(
    definition: &AnnotationDef,
) -> std::result::Result<AnnotationSurfaceClass, MixedConfigParams> {
    classify_annotation_params(&definition.params)
}

/// The same classification rule keyed on the PARAM DEFINITIONS alone — for
/// consumers holding a `CompiledAnnotation.param_defs` carrier instead of an
/// AST definition (the C3-G12 nested-fn check). Same single-file sealing;
/// [`classify_annotation_surface`] delegates here.
pub(in crate::compiler) fn classify_annotation_params(
    params: &[FunctionParameter],
) -> std::result::Result<AnnotationSurfaceClass, MixedConfigParams> {
    let typed_count = params
        .iter()
        .filter(|parameter| parameter.type_annotation.is_some())
        .count();
    if typed_count == 0 {
        return Ok(AnnotationSurfaceClass::Legacy(SurfaceClassEvidence(())));
    }
    if let Some(first_untyped) = params
        .iter()
        .find(|parameter| parameter.type_annotation.is_none())
    {
        return Err(MixedConfigParams {
            first_untyped: first_untyped.clone(),
        });
    }
    Ok(AnnotationSurfaceClass::TypedConfig(SurfaceClassEvidence(())))
}

/// R2 payload: the first untyped parameter of a mixed definition, for the
/// rejection sentence and its span anchor.
pub(in crate::compiler) struct MixedConfigParams {
    pub(in crate::compiler) first_untyped: FunctionParameter,
}

impl AnnotationInstallationPlan {
    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

pub(super) fn build(
    compiler: &BytecodeCompiler,
    items: &[Item],
    installed: &BTreeMap<String, AnnotationDef>,
) -> Result<AnnotationInstallationPlan> {
    let mut candidates = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for definition in items.iter().filter_map(annotation_definition) {
        if candidates
            .insert(definition.name.clone(), definition.clone())
            .is_some()
        {
            duplicates.insert(definition.name.clone());
        }
    }
    if let Some(name) = duplicates.into_iter().next() {
        return Err(ShapeError::SemanticError {
            message: format!(
                "Duplicate annotation declaration '{}' in one declaration scope",
                name
            ),
            location: None,
        });
    }

    let mut pending = Vec::new();
    for (name, definition) in candidates {
        match installed.get(&name) {
            Some(existing) if existing == &definition => continue,
            Some(_) => {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Conflicting annotation declaration '{}' does not match the declaration already prepared for this qualified name",
                        name
                    ),
                    location: None,
                });
            }
            None => pending.push(plan_definition(compiler, definition)?),
        }
    }

    // ADR-009 C3 #14 (slice 4, S4c): only LEGACY-classified declarative
    // handlers register legacy runtime-handler function slots and reserve
    // `{name}___{kind}` callables. TypedConfig before/after handlers lower
    // onto the public comptime API (installed per-target through
    // `specialize_template`) — they consume NO reserved slot and no callable
    // name; TypedConfig lifecycle handlers were R3-family-rejected in
    // `plan_definition` before reaching this loop.
    let runtime_handler_count = pending
        .iter()
        .filter(|plan| matches!(plan.surface_class, AnnotationSurfaceClass::Legacy(_)))
        .map(|plan| runtime_handler_count(&plan.definition))
        .sum::<usize>();
    let end = compiler
        .program
        .functions
        .len()
        .checked_add(runtime_handler_count)
        .ok_or_else(function_id_capacity_error)?;
    if end > usize::from(u16::MAX) + 1 {
        return Err(function_id_capacity_error());
    }

    let mut planned_callable_names = BTreeSet::new();
    for plan in &pending {
        if !matches!(plan.surface_class, AnnotationSurfaceClass::Legacy(_)) {
            continue;
        }
        for handler in &plan.definition.handlers {
            if is_runtime_handler(&handler.handler_type) {
                let name = handler_callable_name(&plan.definition.name, &handler.handler_type);
                if !planned_callable_names.insert(name.clone())
                    || !callable_name_is_vacant(compiler, &name)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Annotation handler callable '{}' conflicts with an existing compiler callable",
                            name
                        ),
                        location: Some(compiler.span_to_source_location(handler.span)),
                    });
                }
            }
        }
    }

    Ok(AnnotationInstallationPlan { pending })
}

fn plan_definition(
    compiler: &BytecodeCompiler,
    definition: AnnotationDef,
) -> Result<PlannedAnnotation> {
    // ADR-009 C3 #14 (slice 4): classify the surface BEFORE any application
    // can exist — R2 (mixed typed/untyped) and R1 (config type outside the
    // ConstLift domain) are DECLARATION-SITE named rejections (the G5
    // sentence precedent from S3's finish()-time check).
    let surface_class = match classify_annotation_surface(&definition) {
        Ok(surface_class) => surface_class,
        Err(mixed) => {
            let untyped_name = mixed
                .first_untyped
                .simple_name()
                .unwrap_or_default()
                .to_string();
            return Err(ShapeError::SemanticError {
                message: format!(
                    "annotation `{}` mixes typed and untyped config parameters; a typed-config annotation declares a type on every config parameter — annotate `{}`",
                    definition.name, untyped_name
                ),
                location: Some(compiler.span_to_source_location(mixed.first_untyped.span())),
            });
        }
    };
    if matches!(surface_class, AnnotationSurfaceClass::TypedConfig(_)) {
        for parameter in &definition.params {
            let Some(annotation) = parameter.type_annotation.as_ref() else {
                continue;
            };
            // R1: the ONE domain producer, reused at the declaration site —
            // never re-implemented (S3's `annotation_within_lift_domain`).
            if let Err(reason) =
                crate::compiler::template_specialization::const_lift::annotation_within_lift_domain(
                    annotation,
                )
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "annotation `{}` declares config parameter `{}: {}`, whose type is outside the ConstLift domain ({}); {} — declare the config parameter with a liftable type",
                        definition.name,
                        parameter.simple_name().unwrap_or_default(),
                        annotation.to_type_string(),
                        reason,
                        crate::compiler::template_specialization::const_lift::CONST_LIFT_DOMAIN_SENTENCE,
                    ),
                    location: Some(compiler.span_to_source_location(parameter.span())),
                });
            }
        }
    }

    // ADR-009 C3 #14 (slice 4, S4c): the sugar lowering — validated (R3 /
    // R3-family, exact sentences from the ONE producer in `sugar_lowering`)
    // and BUILT at the declaration, before any `@application` exists. The
    // installer stores the artifacts on the `CompiledAnnotation` carrier.
    let sugar = if matches!(surface_class, AnnotationSurfaceClass::TypedConfig(_)) {
        match super::sugar_lowering::lower_typed_config_declarative_hooks(compiler, &definition) {
            Ok(sugar) => sugar,
            Err(rejection) => {
                return Err(ShapeError::SemanticError {
                    message: rejection.message,
                    location: Some(compiler.span_to_source_location(rejection.span)),
                });
            }
        }
    } else {
        None
    };

    let mut handler_kinds = BTreeSet::new();
    for handler in &definition.handlers {
        let kind = handler_kind_name(&handler.handler_type);
        if !handler_kinds.insert(kind) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Annotation '{}' declares more than one '{}' handler",
                    definition.name, kind
                ),
                location: Some(compiler.span_to_source_location(handler.span)),
            });
        }
        if is_runtime_handler(&handler.handler_type)
            && handler.params.iter().any(|parameter| parameter.is_variadic)
        {
            return Err(ShapeError::SemanticError {
                message: "Variadic annotation handler params (`...args`) are only supported on comptime handlers"
                    .to_string(),
                location: Some(compiler.span_to_source_location(handler.span)),
            });
        }
        if matches!(
            &handler.handler_type,
            AnnotationHandlerType::OnDefine | AnnotationHandlerType::Metadata
        ) {
            let count = 1usize
                .checked_add(definition.params.len())
                .and_then(|count| count.checked_add(handler.params.len()))
                .ok_or_else(function_id_capacity_error)?;
            if count > usize::from(u16::MAX) {
                return Err(function_id_capacity_error());
            }
            validate_default_parameter_order(compiler, &definition.params)?;
            if !handler.params.is_empty()
                && definition
                    .params
                    .iter()
                    .any(|parameter| parameter.default_value.is_some())
            {
                return Err(ShapeError::SemanticError {
                    message: "Required parameter cannot follow a parameter with a default value"
                        .to_string(),
                    location: Some(compiler.span_to_source_location(handler.span)),
                });
            }
        }
    }

    let has_before_after = definition.handlers.iter().any(|handler| {
        matches!(
            &handler.handler_type,
            AnnotationHandlerType::Before
                | AnnotationHandlerType::After
                | AnnotationHandlerType::ComptimePre
                | AnnotationHandlerType::ComptimePost
        )
    });
    let has_lifecycle = definition.handlers.iter().any(|handler| {
        matches!(
            &handler.handler_type,
            AnnotationHandlerType::OnDefine | AnnotationHandlerType::Metadata
        )
    });
    let allowed_targets = definition.allowed_targets.clone().unwrap_or_else(|| {
        if has_before_after {
            vec![AnnotationTargetKind::Function]
        } else if has_lifecycle {
            vec![
                AnnotationTargetKind::Function,
                AnnotationTargetKind::Type,
                AnnotationTargetKind::Module,
            ]
        } else {
            Vec::new()
        }
    });

    if has_lifecycle {
        if allowed_targets.is_empty() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Annotation '{}' uses `on_define`/`metadata` and cannot have unrestricted targets. Allowed targets are: function, type, module",
                    definition.name
                ),
                location: Some(compiler.span_to_source_location(definition.span)),
            });
        }
        if let Some(invalid) = allowed_targets
            .iter()
            .find(|target| !BytecodeCompiler::is_definition_annotation_target(**target))
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Annotation '{}' uses `on_define`/`metadata`, but target '{}' is not a definition target. Allowed targets are: function, type, module",
                    definition.name,
                    format!("{:?}", invalid).to_lowercase()
                ),
                location: Some(compiler.span_to_source_location(definition.span)),
            });
        }
    }

    Ok(PlannedAnnotation {
        definition,
        allowed_targets,
        surface_class,
        sugar,
    })
}

fn validate_default_parameter_order(
    compiler: &BytecodeCompiler,
    parameters: &[FunctionParameter],
) -> Result<()> {
    let mut saw_default = false;
    for parameter in parameters {
        if parameter.default_value.is_some() {
            saw_default = true;
        } else if saw_default {
            return Err(ShapeError::SemanticError {
                message: "Required parameter cannot follow a parameter with a default value"
                    .to_string(),
                location: Some(compiler.span_to_source_location(parameter.span())),
            });
        }
    }
    Ok(())
}

pub(super) fn handler_callable_name(name: &str, handler: &AnnotationHandlerType) -> String {
    format!("{}___{}", name, handler_kind_name(handler))
}

fn handler_kind_name(handler: &AnnotationHandlerType) -> &'static str {
    match handler {
        AnnotationHandlerType::Before => "before",
        AnnotationHandlerType::After => "after",
        AnnotationHandlerType::OnDefine => "on_define",
        AnnotationHandlerType::Metadata => "metadata",
        AnnotationHandlerType::ComptimePre => "comptime_pre",
        AnnotationHandlerType::ComptimePost => "comptime_post",
    }
}

fn is_runtime_handler(handler: &AnnotationHandlerType) -> bool {
    !matches!(
        handler,
        AnnotationHandlerType::ComptimePre | AnnotationHandlerType::ComptimePost
    )
}

fn runtime_handler_count(definition: &AnnotationDef) -> usize {
    definition
        .handlers
        .iter()
        .filter(|handler| is_runtime_handler(&handler.handler_type))
        .count()
}

fn callable_name_is_vacant(compiler: &BytecodeCompiler, name: &str) -> bool {
    !compiler.program.functions.iter().any(|row| row.name == name)
        && !compiler.function_defs.contains_key(name)
        && !compiler.foreign_function_defs.contains_key(name)
        && !compiler.module_builtin_functions.contains_key(name)
        && !compiler.function_arity_bounds.contains_key(name)
        && !compiler.function_const_params.contains_key(name)
        && !compiler.function_return_reference_summaries.contains_key(name)
        && !compiler.inferred_ref_params.contains_key(name)
        && !compiler.inferred_ref_mutates.contains_key(name)
        && !compiler.inferred_param_pass_modes.contains_key(name)
        && !compiler.blob_name_to_hash.contains_key(name)
        && !compiler.completed_blobs.iter().any(|blob| blob.name == name)
        && !compiler
            .current_blob_builder
            .as_ref()
            .is_some_and(|builder| builder.name == name)
        && !compiler.mir_functions.contains_key(name)
        && !compiler.mir_borrow_analyses.contains_key(name)
        && !compiler.mir_storage_plans.contains_key(name)
        && !compiler.function_borrow_summaries.contains_key(name)
        && !compiler.mir_span_to_point.contains_key(name)
        && !compiler.mir_field_analyses.contains_key(name)
        && !compiler.const_specializations.contains_key(name)
        && !compiler.specialization_const_bindings.contains_key(name)
        && !compiler.failed_call_site_specializations.contains(name)
        && !compiler.removed_functions.contains(name)
        && !compiler.stdlib_function_names.contains(name)
        && !compiler.generated_symbols.contains_name(name)
        && !compiler
            .imported_names
            .get(name)
            .is_some_and(|imported| imported_symbol_is_callable(compiler, imported))
}

fn imported_symbol_is_callable(
    compiler: &BytecodeCompiler,
    imported: &crate::compiler::ImportedSymbol,
) -> bool {
    use shape_ast::module_utils::ModuleExportKind;

    match imported.kind {
        Some(ModuleExportKind::Function | ModuleExportKind::BuiltinFunction) => true,
        Some(
            ModuleExportKind::TypeAlias
            | ModuleExportKind::BuiltinType
            | ModuleExportKind::Trait
            | ModuleExportKind::Enum
            | ModuleExportKind::Annotation
            | ModuleExportKind::Value,
        ) => false,
        None => {
            let qualified = if imported.module_path.is_empty() {
                imported.original_name.clone()
            } else {
                format!("{}::{}", imported.module_path, imported.original_name)
            };
            callable_registry_contains(compiler, &qualified)
                || callable_registry_contains(compiler, &imported.original_name)
        }
    }
}

fn callable_registry_contains(compiler: &BytecodeCompiler, name: &str) -> bool {
    compiler.function_defs.contains_key(name)
        || compiler.foreign_function_defs.contains_key(name)
        || compiler.module_builtin_functions.contains_key(name)
        || compiler
            .program
            .functions
            .iter()
            .any(|function| function.name == name)
}

fn function_id_capacity_error() -> ShapeError {
    ShapeError::RuntimeError {
        message: "Annotation handler installation exceeds the u16 function-id capacity"
            .to_string(),
        location: None,
    }
}
