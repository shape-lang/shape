//! Registration-complete annotation declaration preparation.

use std::collections::{BTreeMap, BTreeSet};

use crate::bytecode::CompiledAnnotation;
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::{
    AnnotationDef, AnnotationHandlerType, ExportItem, FunctionDef, FunctionParameter, Item, Span,
    Statement, TypeAnnotation,
};
use shape_ast::error::{Result, ShapeError};

/// Phase evidence for annotation definitions whose installation completed.
///
/// The map is intentionally opaque outside this module. A declaration enters
/// it only after every runtime/comptime carrier has been installed successfully.
#[derive(Default)]
pub(in crate::compiler) struct AnnotationDeclarationState {
    installed: BTreeMap<String, AnnotationDef>,
}

impl AnnotationDeclarationState {
    fn installation_plan(&self, items: &[Item]) -> Result<Vec<AnnotationDef>> {
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

        let mut conflicts = BTreeSet::new();
        let mut pending = Vec::new();
        for (name, definition) in candidates {
            match self.installed.get(&name) {
                Some(installed) if installed == &definition => {}
                Some(_) => {
                    conflicts.insert(name);
                }
                None => pending.push(definition),
            }
        }

        if let Some(name) = conflicts.into_iter().next() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Conflicting annotation declaration '{}' does not match the declaration already prepared for this qualified name",
                    name
                ),
                location: None,
            });
        }

        Ok(pending)
    }

    fn record_installed(&mut self, definition: AnnotationDef) {
        self.installed.insert(definition.name.clone(), definition);
    }

    fn require(&self, definition: &AnnotationDef) -> Result<()> {
        match self.installed.get(&definition.name) {
            Some(installed) if installed == definition => Ok(()),
            Some(_) => Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal compiler phase-order error: annotation declaration '{}' changed between preparation and pass 2",
                    definition.name
                ),
                location: None,
            }),
            None => Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal compiler phase-order error: annotation declaration '{}' reached pass 2 before declaration preparation",
                    definition.name
                ),
                location: None,
            }),
        }
    }
}

fn annotation_definition(item: &Item) -> Option<&AnnotationDef> {
    match item {
        Item::AnnotationDef(definition, _) => Some(definition),
        Item::Export(export, _) => match &export.item {
            ExportItem::Annotation(definition) => Some(definition),
            _ => None,
        },
        _ => None,
    }
}

impl BytecodeCompiler {
    /// Prepare every annotation declaration in one registration-complete scope.
    pub(in crate::compiler) fn prepare_annotation_scope(&mut self, items: &[Item]) -> Result<()> {
        let plan = self.annotation_declarations.installation_plan(items)?;
        for definition in plan {
            self.install_annotation_definition(&definition)?;
            self.annotation_declarations.record_installed(definition);
        }
        Ok(())
    }

    /// Pass 2 consumes preparation evidence; it never installs or falls back.
    pub(super) fn require_prepared_annotation(&self, definition: &AnnotationDef) -> Result<()> {
        self.annotation_declarations.require(definition)
    }

    /// Install the runtime and comptime carriers for one annotation definition.
    fn install_annotation_definition(&mut self, ann_def: &AnnotationDef) -> Result<()> {
        let mut compiled = CompiledAnnotation {
            name: ann_def.name.clone(),
            param_names: ann_def
                .params
                .iter()
                .flat_map(|parameter| parameter.get_identifiers())
                .collect(),
            param_defs: ann_def.params.clone(),
            before_handler: None,
            after_handler: None,
            on_define_handler: None,
            metadata_handler: None,
            comptime_pre_handler: None,
            comptime_post_handler: None,
            before_handler_template: None,
            after_handler_template: None,
            allowed_targets: Vec::new(),
        };

        for handler in &ann_def.handlers {
            match handler.handler_type {
                AnnotationHandlerType::ComptimePre => {
                    compiled.comptime_pre_handler = Some(handler.clone());
                    continue;
                }
                AnnotationHandlerType::ComptimePost => {
                    compiled.comptime_post_handler = Some(handler.clone());
                    continue;
                }
                _ => {}
            }

            if handler.params.iter().any(|parameter| parameter.is_variadic) {
                return Err(ShapeError::SemanticError {
                    message: "Variadic annotation handler params (`...args`) are only supported on comptime handlers"
                        .to_string(),
                    location: Some(self.span_to_source_location(handler.span)),
                });
            }

            let handler_type = match handler.handler_type {
                AnnotationHandlerType::Before => "before",
                AnnotationHandlerType::After => "after",
                AnnotationHandlerType::OnDefine => "on_define",
                AnnotationHandlerType::Metadata => "metadata",
                AnnotationHandlerType::ComptimePre | AnnotationHandlerType::ComptimePost => {
                    unreachable!()
                }
            };
            let function_name = format!("{}___{}", ann_def.name, handler_type);

            if matches!(
                handler.handler_type,
                AnnotationHandlerType::Before | AnnotationHandlerType::After
            ) {
                let placeholder = FunctionDef {
                    name: function_name.clone(),
                    name_span: Span::DUMMY,
                    declaring_module_path: None,
                    doc_comment: None,
                    params: Vec::new(),
                    return_type: handler.return_type.clone(),
                    body: Vec::new(),
                    type_params: Some(Vec::new()),
                    annotations: Vec::new(),
                    is_async: false,
                    is_comptime: false,
                    where_clause: None,
                };
                self.register_function(&placeholder)?;
                let function_id = self.find_function(&function_name).ok_or_else(|| {
                    ShapeError::RuntimeError {
                        message: format!(
                            "Internal error: annotation handler function '{}' was not registered",
                            function_name
                        ),
                        location: None,
                    }
                })? as u16;
                match handler.handler_type {
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
                continue;
            }

            let mut params = vec![FunctionParameter {
                pattern: shape_ast::ast::DestructurePattern::Identifier(
                    "self".to_string(),
                    Span::DUMMY,
                ),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            }];
            params.extend(ann_def.params.iter().cloned());
            for parameter in &handler.params {
                let inferred_type = if parameter.name == "ctx" {
                    Some(TypeAnnotation::Object(vec![
                        shape_ast::ast::ObjectTypeField {
                            name: "state".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("unknown".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "event_log".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Array(Box::new(
                                TypeAnnotation::Basic("unknown".to_string()),
                            )),
                            annotations: vec![],
                        },
                    ]))
                } else if matches!(
                    handler.handler_type,
                    AnnotationHandlerType::OnDefine | AnnotationHandlerType::Metadata
                ) && (parameter.name == "fn" || parameter.name == "target")
                {
                    Some(TypeAnnotation::Object(vec![
                        shape_ast::ast::ObjectTypeField {
                            name: "name".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("string".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "kind".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("string".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "id".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("int".to_string()),
                            annotations: vec![],
                        },
                    ]))
                } else {
                    None
                };

                params.push(FunctionParameter {
                    pattern: shape_ast::ast::DestructurePattern::Identifier(
                        parameter.name.clone(),
                        Span::DUMMY,
                    ),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: inferred_type,
                    default_value: None,
                });
            }

            let function = FunctionDef {
                name: function_name,
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
            };

            self.register_function(&function)?;
            self.compile_function(&function)?;
            let function_index = self.program.functions.len() - 1;
            self.program.functions[function_index].locals_count = self.next_local;
            self.capture_function_local_storage_hints(function_index);
            let function_id = function_index as u16;

            match handler.handler_type {
                AnnotationHandlerType::Before => compiled.before_handler = Some(function_id),
                AnnotationHandlerType::After => compiled.after_handler = Some(function_id),
                AnnotationHandlerType::OnDefine => compiled.on_define_handler = Some(function_id),
                AnnotationHandlerType::Metadata => compiled.metadata_handler = Some(function_id),
                AnnotationHandlerType::ComptimePre | AnnotationHandlerType::ComptimePost => {}
            }
        }

        if let Some(explicit) = &ann_def.allowed_targets {
            compiled.allowed_targets = explicit.clone();
        } else if compiled.before_handler.is_some()
            || compiled.after_handler.is_some()
            || compiled.comptime_pre_handler.is_some()
            || compiled.comptime_post_handler.is_some()
        {
            compiled.allowed_targets = vec![shape_ast::ast::AnnotationTargetKind::Function];
        } else if compiled.on_define_handler.is_some() || compiled.metadata_handler.is_some() {
            compiled.allowed_targets = vec![
                shape_ast::ast::AnnotationTargetKind::Function,
                shape_ast::ast::AnnotationTargetKind::Type,
                shape_ast::ast::AnnotationTargetKind::Module,
            ];
        }

        if compiled.on_define_handler.is_some() || compiled.metadata_handler.is_some() {
            if compiled.allowed_targets.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Annotation '{}' uses `on_define`/`metadata` and cannot have unrestricted targets. Allowed targets are: function, type, module",
                        ann_def.name
                    ),
                    location: Some(self.span_to_source_location(ann_def.span)),
                });
            }
            if let Some(invalid) = compiled
                .allowed_targets
                .iter()
                .find(|target| !Self::is_definition_annotation_target(**target))
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Annotation '{}' uses `on_define`/`metadata`, but target '{}' is not a definition target. Allowed targets are: function, type, module",
                        ann_def.name,
                        format!("{:?}", invalid).to_lowercase()
                    ),
                    location: Some(self.span_to_source_location(ann_def.span)),
                });
            }
        }

        self.program
            .compiled_annotations
            .insert(ann_def.name.clone(), compiled);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
