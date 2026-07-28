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
    /// ADR-009 C3 #14 (slice 4, S4c; S6 completion): the sugar lowering's
    /// artifacts for a definition with declarative hooks — validated (R3)
    /// and built HERE at planning, stored on `CompiledAnnotation` by the
    /// installer. `None` for definitions without declarative hooks.
    pub(super) sugar: Option<super::sugar_lowering::SugarLowering>,
}

// ═══════════════════════════════════════════════════════════════════════════
// ADR-009 C3 #14 (S6 completion) — THE COLLAPSED CLASSIFICATION (one surface)
// ═══════════════════════════════════════════════════════════════════════════
//
// The S4 transitional TypedConfig/Legacy fork is DELETED (C3-G7/S6): there
// is exactly ONE annotation surface. Decided AT THE DECLARATION (before any
// `@application` exists):
//
//   - Every config parameter DECLARES ITS TYPE. An untyped config parameter
//     is the named declaration-site rejection in `plan_definition` below
//     (single producer; the former R2 mixed-params refinement folds into
//     it — "mixed" is just "an untyped parameter is present"). Every
//     declared type must lie within the C3-G5 ConstLift domain, checked by
//     the ONE domain producer `const_lift::annotation_within_lift_domain`
//     (rejection R1) — a non-liftable config type is a named error before
//     any application.
//
//   - Declarative `before`/`after` handlers LOWER onto the PUBLIC comptime
//     API (CheckedTemplate + install; the S4c sugar lowering — see
//     `sugar_lowering` for the BINDING RULES: `before(args)` /
//     `after(result)` polymorphic forms, `before()` / `after()` observers,
//     and the R3 shape rejection). Zero-param definitions route this same
//     path — there is no other.
//
//   - COMPTIME pre/post handlers run through the comptime mini-VM; typed
//     injection gives handler params their declared annotations.
//
//   - `on_define`/`metadata` LIFECYCLE handlers register through the
//     installer's lifecycle arm (compile-time-fired definition hooks — not
//     part of the deleted runtime before/after weave). They reserve the
//     `{name}___{kind}` callables below. S6-completion disposition
//     (Risk-1): lifecycle survives the collapse and now accepts typed
//     config params (the former R3-family rejection existed to keep typed
//     params off the legacy surface, which no longer exists).
//
// The legacy weave slots and their machinery are GONE (deleted from
// `CompiledAnnotation` and the compiler at the S6 capstone) — declarative
// hooks travel exclusively on the sugar carrier above.

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

    // ADR-009 C3 #14 (S6 completion): only LIFECYCLE handlers
    // (`on_define`/`metadata`) register handler function slots and reserve
    // `{name}___{kind}` callables. Declarative before/after handlers lower
    // onto the public comptime API (installed per-target through
    // `specialize_template`) — they consume NO reserved slot and no callable
    // name.
    let lifecycle_handler_count = pending
        .iter()
        .map(|plan| lifecycle_handler_count(&plan.definition))
        .sum::<usize>();
    let end = compiler
        .program
        .functions
        .len()
        .checked_add(lifecycle_handler_count)
        .ok_or_else(function_id_capacity_error)?;
    if end > usize::from(u16::MAX) + 1 {
        return Err(function_id_capacity_error());
    }

    let mut planned_callable_names = BTreeSet::new();
    for plan in &pending {
        for handler in &plan.definition.handlers {
            if is_lifecycle_handler(&handler.handler_type) {
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
    // ADR-009 C3 #14 (S6 completion): the collapsed declaration-site checks,
    // BEFORE any application can exist — an untyped config parameter (the
    // named rejection below, which the former R2 mixed-params refinement
    // folds into) and R1 (config type outside the ConstLift domain) are
    // DECLARATION-SITE named rejections (the G5 sentence precedent from
    // S3's finish()-time check).
    if let Some(untyped) = definition
        .params
        .iter()
        .find(|parameter| parameter.type_annotation.is_none())
    {
        let untyped_name = untyped.simple_name().unwrap_or_default().to_string();
        return Err(ShapeError::SemanticError {
            message: format!(
                "annotation `{}` declares config parameter `{}` without a type; every annotation config parameter declares its type (the untyped config surface is deleted, C3-G7/S6) — annotate `{}` with a ConstLift-liftable type",
                definition.name, untyped_name, untyped_name
            ),
            location: Some(compiler.span_to_source_location(untyped.span())),
        });
    }
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

    // ADR-009 C3 #14 (slice 4, S4c; S6 completion): the sugar lowering —
    // validated (R3, exact sentences from the ONE producer in
    // `sugar_lowering`) and BUILT at the declaration, before any
    // `@application` exists, for EVERY definition (zero-param included).
    // The installer stores the artifacts on the `CompiledAnnotation`
    // carrier.
    let sugar =
        match super::sugar_lowering::lower_typed_config_declarative_hooks(compiler, &definition) {
            Ok(sugar) => sugar,
            Err(rejection) => {
                return Err(ShapeError::SemanticError {
                    message: rejection.message,
                    location: Some(compiler.span_to_source_location(rejection.span)),
                });
            }
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
            // ADR-009 E4-D2 ctx-removal (slice S3): the `on_define`/`metadata`
            // lifecycle handler receives ONLY the `target`/`fn` descriptor. The
            // always-empty `ctx` carrier was DELETED in E4-D2; a lingering `ctx`
            // — or any other non-descriptor name — now degrades to a silent-null
            // `PushNull` param (functions_annotations.rs `match param_name` →
            // `_ => PushNull`), the silent-no-op / cryptic-runtime footgun the
            // standing user ruling forbids. Reject it LOUD, pre-inference, once
            // per declaration, at the handler signature span (LSP-visible). The
            // accepted set is exactly the names the descriptor emission arm +
            // `inferred_handler_parameter_type` still recognize (`target`/`fn`).
            let kind = handler_kind_name(&handler.handler_type);
            if let Some(parameter) = handler
                .params
                .iter()
                .find(|parameter| parameter.name != "target" && parameter.name != "fn")
            {
                let message = if parameter.name == "ctx" {
                    // E4-D2 ctx-removal: specific sub-message names the removed
                    // lifecycle-ctx surface + the per-invocation State deferral (#83).
                    // The HookDecision protocol itself landed in E4 (#68 closed).
                    format!(
                        "Annotation '{}': the '{}' lifecycle handler takes only '(target)'. \
                         The 'ctx' parameter was removed in E4-D2 — the always-empty lifecycle \
                         ctx ({{state: {{}}, event_log: []}}) had no reader. The HookDecision \
                         protocol landed in E4; the typed per-invocation context (State \
                         threading) is a first-cut deferral tracked in issue #83.",
                        definition.name, kind
                    )
                } else {
                    format!(
                        "Annotation '{}': unknown '{}' lifecycle handler parameter '{}'. \
                         Lifecycle handlers receive only the 'target' descriptor.",
                        definition.name, kind, parameter.name
                    )
                };
                return Err(ShapeError::SemanticError {
                    message,
                    location: Some(compiler.span_to_source_location(handler.span)),
                });
            }
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

    // ADR-009 C3 #14 (slice 5, S5b): the declaration-tier non-function-target
    // rejection (S4 residual 7) — a TypedConfig-with-hooks definition whose
    // explicit targets exclude `function` can never fire its hooks (the
    // non-function consumer seams run only comptime pre/post handlers).
    // Empty `allowed_targets` means "no restriction" (function applications
    // stay legal), so only a non-empty function-less set rejects. ONE
    // sentence producer in `sugar_lowering`.
    if sugar.is_some()
        && !allowed_targets.is_empty()
        && !allowed_targets.contains(&AnnotationTargetKind::Function)
    {
        return Err(ShapeError::SemanticError {
            message: super::sugar_lowering::non_function_targets_declaration_rejection(
                &definition.name,
                &allowed_targets,
            ),
            location: Some(compiler.span_to_source_location(definition.span)),
        });
    }

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

/// The handlers that register compiled handler functions and reserve
/// `{name}___{kind}` callables post-collapse: the `on_define`/`metadata`
/// lifecycle pair. Declarative before/after handlers lower onto the public
/// comptime API and consume no slot (S6 completion).
fn is_lifecycle_handler(handler: &AnnotationHandlerType) -> bool {
    matches!(
        handler,
        AnnotationHandlerType::OnDefine | AnnotationHandlerType::Metadata
    )
}

fn lifecycle_handler_count(definition: &AnnotationDef) -> usize {
    definition
        .handlers
        .iter()
        .filter(|handler| is_lifecycle_handler(&handler.handler_type))
        .count()
}

fn callable_name_is_vacant(compiler: &BytecodeCompiler, name: &str) -> bool {
    !compiler
        .program
        .functions
        .iter()
        .any(|row| row.name == name)
        && !compiler.function_defs.contains_key(name)
        && !compiler.foreign_function_defs.contains_key(name)
        && !compiler.module_builtin_functions.contains_key(name)
        && !compiler.function_arity_bounds.contains_key(name)
        && !compiler.function_const_params.contains_key(name)
        && !compiler
            .function_return_reference_summaries
            .contains_key(name)
        && !compiler.inferred_ref_params.contains_key(name)
        && !compiler.inferred_ref_mutates.contains_key(name)
        && !compiler.inferred_param_pass_modes.contains_key(name)
        && !compiler.blob_name_to_hash.contains_key(name)
        && !compiler
            .completed_blobs
            .iter()
            .any(|blob| blob.name == name)
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
        message: "Annotation handler installation exceeds the u16 function-id capacity".to_string(),
        location: None,
    }
}
