//! Mutation phase for a previously validated annotation plan.

use shape_ast::ast::{
    AnnotationHandler, AnnotationHandlerType, DestructurePattern, FunctionDef, FunctionParameter,
    Span, Statement, TypeAnnotation,
};
use shape_ast::error::{Result, ShapeError};

use crate::bytecode::CompiledAnnotation;
use crate::compiler::BytecodeCompiler;

use super::planner::{handler_callable_name, AnnotationSurfaceClass, PlannedAnnotation};

pub(super) fn install(
    compiler: &mut BytecodeCompiler,
    plan: &PlannedAnnotation,
) -> Result<CompiledAnnotation> {
    let definition = &plan.definition;
    let mut compiled = CompiledAnnotation {
        name: definition.name.clone(),
        param_names: definition
            .params
            .iter()
            .flat_map(|parameter| parameter.get_identifiers())
            .collect(),
        param_defs: definition.params.clone(),
        before_handler: None,
        after_handler: None,
        on_define_handler: None,
        metadata_handler: None,
        comptime_pre_handler: None,
        comptime_post_handler: None,
        before_handler_template: None,
        after_handler_template: None,
        // ADR-009 C3 #14 (slice 4, S4c): the sugar lowering's artifacts
        // (planner-built) ride the carrier so BOTH the pass-2 handler driver
        // and the Compiled handler-resolution provenance run the synthesized
        // public-API handler with its minted body fns.
        sugar_post_handler: plan.sugar.as_ref().map(|sugar| sugar.post_handler.clone()),
        sugar_body_fns: plan
            .sugar
            .as_ref()
            .map(|sugar| sugar.body_fns.clone())
            .unwrap_or_default(),
        allowed_targets: plan.allowed_targets.clone(),
    };

    for handler in &definition.handlers {
        match &handler.handler_type {
            AnnotationHandlerType::ComptimePre => {
                compiled.comptime_pre_handler = Some(handler.clone());
            }
            AnnotationHandlerType::ComptimePost => {
                compiled.comptime_post_handler = Some(handler.clone());
            }
            AnnotationHandlerType::Before | AnnotationHandlerType::After => {
                // ADR-009 C3 #14 (slice 4, S4c): the classification fork. A
                // TypedConfig definition's declarative before/after handlers
                // NEVER register on the legacy weave slots (`before_handler`
                // / `before_handler_template` — silent legacy engagement of
                // typed params is forbidden). They are LOWERED onto the
                // public comptime API instead: the planner validated (R3)
                // and built the artifacts (`plan.sugar` — minted body fns +
                // the synthesized public-API `comptime post` handler), which
                // are attached to the carrier below and installed per-target
                // through `specialize_template` + the open
                // InstallTransaction, exactly like a hand-written handler
                // (C3-G2/G3/G4).
                if matches!(plan.surface_class, AnnotationSurfaceClass::TypedConfig(_)) {
                    continue;
                }
                let name = handler_callable_name(&definition.name, &handler.handler_type);
                let function = placeholder_function(name, handler.return_type.clone());
                let function_id = register_exact_function(compiler, &function)?;
                match &handler.handler_type {
                    AnnotationHandlerType::Before => {
                        compiled.before_handler = Some(function_id);
                        compiled.before_handler_template = Some(handler.clone());
                    }
                    AnnotationHandlerType::After => {
                        compiled.after_handler = Some(function_id);
                        compiled.after_handler_template = Some(handler.clone());
                    }
                    _ => unreachable!(),
                }
            }
            AnnotationHandlerType::OnDefine | AnnotationHandlerType::Metadata => {
                let name = handler_callable_name(&definition.name, &handler.handler_type);
                let function = lifecycle_function(name, definition.params.clone(), handler);
                let function_id = register_exact_function(compiler, &function)?;
                let completed_blob_start = compiler.completed_blobs.len();
                compiler.compile_function(&function)?;
                verify_compiled_handler(
                    compiler,
                    usize::from(function_id),
                    &function.name,
                    completed_blob_start,
                )?;
                match &handler.handler_type {
                    AnnotationHandlerType::OnDefine => {
                        compiled.on_define_handler = Some(function_id)
                    }
                    AnnotationHandlerType::Metadata => {
                        compiled.metadata_handler = Some(function_id)
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    Ok(compiled)
}

fn register_exact_function(
    compiler: &mut BytecodeCompiler,
    definition: &FunctionDef,
) -> Result<u16> {
    let expected_index = compiler.program.functions.len();
    let function_id = u16::try_from(expected_index).map_err(|_| function_id_capacity_error())?;
    compiler.register_function(definition)?;
    if compiler.program.functions.len() != expected_index + 1
        || compiler.program.functions[expected_index].name != definition.name
    {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Internal compiler error: annotation handler '{}' did not retain its reserved function identity",
                definition.name
            ),
            location: None,
        });
    }
    Ok(function_id)
}

fn verify_compiled_handler(
    compiler: &BytecodeCompiler,
    function_index: usize,
    name: &str,
    completed_blob_start: usize,
) -> Result<()> {
    if compiler
        .program
        .functions
        .get(function_index)
        .is_none_or(|function| function.name != name)
    {
        return Err(handler_identity_error(name));
    }
    let matching_blobs = compiler.completed_blobs[completed_blob_start..]
        .iter()
        .filter(|blob| blob.name == name)
        .collect::<Vec<_>>();
    if matching_blobs.len() != 1 {
        return Err(handler_identity_error(name));
    }
    let blob = matching_blobs[0];
    if compiler.blob_name_to_hash.get(name) != Some(&blob.content_hash)
        || compiler
            .function_hashes_by_id
            .get(function_index)
            .and_then(|hash| *hash)
            != Some(blob.content_hash)
    {
        return Err(handler_identity_error(name));
    }
    Ok(())
}

fn placeholder_function(name: String, return_type: Option<TypeAnnotation>) -> FunctionDef {
    FunctionDef {
        name,
        name_span: Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        params: Vec::new(),
        return_type,
        body: Vec::new(),
        type_params: Some(Vec::new()),
        annotations: Vec::new(),
        is_async: false,
        is_comptime: false,
        where_clause: None,
    }
}

fn lifecycle_function(
    name: String,
    annotation_params: Vec<FunctionParameter>,
    handler: &AnnotationHandler,
) -> FunctionDef {
    let mut params = vec![FunctionParameter {
        pattern: DestructurePattern::Identifier("self".to_string(), Span::DUMMY),
        is_const: false,
        is_reference: false,
        is_mut_reference: false,
        is_out: false,
        type_annotation: None,
        default_value: None,
    }];
    params.extend(annotation_params);
    params.extend(handler.params.iter().map(|parameter| FunctionParameter {
        pattern: DestructurePattern::Identifier(parameter.name.clone(), Span::DUMMY),
        is_const: false,
        is_reference: false,
        is_mut_reference: false,
        is_out: false,
        type_annotation: inferred_handler_parameter_type(parameter.name.as_str()),
        default_value: None,
    }));
    FunctionDef {
        name,
        name_span: Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        params,
        return_type: handler.return_type.clone(),
        body: vec![Statement::Return(Some(handler.body.clone()), Span::DUMMY)],
        type_params: Some(Vec::new()),
        annotations: Vec::new(),
        is_async: false,
        is_comptime: false,
        where_clause: None,
    }
}

fn inferred_handler_parameter_type(name: &str) -> Option<TypeAnnotation> {
    if name == "ctx" {
        return Some(TypeAnnotation::Object(vec![
            object_field("state", TypeAnnotation::Basic("unknown".to_string())),
            object_field(
                "event_log",
                TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("unknown".to_string()))),
            ),
        ]));
    }
    if name == "fn" || name == "target" {
        return Some(TypeAnnotation::Object(vec![
            object_field("name", TypeAnnotation::Basic("string".to_string())),
            object_field("kind", TypeAnnotation::Basic("string".to_string())),
            object_field("id", TypeAnnotation::Basic("int".to_string())),
        ]));
    }
    None
}

fn object_field(
    name: &str,
    type_annotation: TypeAnnotation,
) -> shape_ast::ast::ObjectTypeField {
    shape_ast::ast::ObjectTypeField {
        name: name.to_string(),
        optional: false,
        type_annotation,
        annotations: Vec::new(),
    }
}

fn handler_identity_error(name: &str) -> ShapeError {
    ShapeError::RuntimeError {
        message: format!(
            "Internal compiler error: annotation handler '{}' lost its reserved function/blob identity",
            name
        ),
        location: None,
    }
}

fn function_id_capacity_error() -> ShapeError {
    ShapeError::RuntimeError {
        message: "Annotation handler installation exceeds the u16 function-id capacity"
            .to_string(),
        location: None,
    }
}
