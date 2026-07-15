//! Pure annotation installation planning.

use std::collections::{BTreeMap, BTreeSet};

use shape_ast::ast::{
    AnnotationDef, AnnotationHandlerType, AnnotationTargetKind, FunctionParameter, Item, Spanned,
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

    let runtime_handler_count = pending
        .iter()
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
}

fn function_id_capacity_error() -> ShapeError {
    ShapeError::RuntimeError {
        message: "Annotation handler installation exceeds the u16 function-id capacity"
            .to_string(),
        location: None,
    }
}
