//! Annotation lifecycle and comptime handler compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use shape_ast::ast::{
    DestructurePattern, Expr, FunctionDef, Literal, ObjectEntry, Span, Spanned, Statement,
    TypeAnnotation,
};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::FieldType;
use shape_value::KindedSlot;
use std::collections::{HashMap, HashSet};

use super::BytecodeCompiler;
use super::comptime_builtins::expansion_provenance::{
    ApplicationClaim, ApplicationId, CanonicalHash, ComptimeStage, DeclarationDiscoveryFixedPoint,
    ExpansionIdentity, ExpansionSite, GeneratedNodePath, GeneratedOrigin, GeneratedSymbolTable,
    GeneratorRef, SymbolReservation, TargetIdentity,
};

/// Comptime handlers for one annotation, gathered by the §4.5.1 pre-pass
/// (`materialize_computed_comptime_extends`) from the root program and the
/// module graph.
struct ComptimeAnnotationHandlers {
    handlers: Vec<shape_ast::ast::AnnotationHandler>,
    def_param_names: Vec<String>,
}

/// Canonical label for the comptime handler kind inside a generator
/// descriptor. Total over `AnnotationHandlerType` — only the two comptime
/// kinds reach the expansion path today, but the descriptor never fabricates.
fn annotation_handler_kind_descriptor(
    handler_type: &shape_ast::ast::AnnotationHandlerType,
) -> &'static str {
    use shape_ast::ast::AnnotationHandlerType;
    match handler_type {
        AnnotationHandlerType::ComptimePre => "comptime-pre",
        AnnotationHandlerType::ComptimePost => "comptime-post",
        AnnotationHandlerType::OnDefine => "on-define",
        AnnotationHandlerType::Before => "before",
        AnnotationHandlerType::After => "after",
        AnnotationHandlerType::Metadata => "metadata",
    }
}

/// Canonical label for the annotated target's kind inside a target-identity
/// descriptor.
fn annotation_target_kind_descriptor(
    kind: shape_ast::ast::functions::AnnotationTargetKind,
) -> &'static str {
    use shape_ast::ast::functions::AnnotationTargetKind;
    match kind {
        AnnotationTargetKind::Function => "function",
        AnnotationTargetKind::Type => "type",
        AnnotationTargetKind::Module => "module",
        AnnotationTargetKind::Expression => "expression",
        AnnotationTargetKind::Block => "block",
        AnnotationTargetKind::AwaitExpr => "await-expr",
        AnnotationTargetKind::Binding => "binding",
    }
}

/// Canonical dependency descriptors of one comptime expansion for ticket D1:
/// exactly what the existing path FEEDS the handler — the `ComptimeTarget`
/// the handler receives (fields with their type strings and field
/// annotations, params, return type, applied annotations, captures). The
/// full declaration-discovery dependency graph is ticket D2.
fn comptime_target_dependency_descriptors(
    target: &super::comptime_target::ComptimeTarget,
) -> Vec<String> {
    let mut descriptors = Vec::new();
    for (field_name, field_type, field_annotations) in &target.fields {
        descriptors.push(format!("field:{field_name}:{field_type}"));
        for (ann_name, ann_args) in field_annotations {
            descriptors.push(format!(
                "field-annotation:{field_name}:{ann_name}:{}",
                ann_args.join(",")
            ));
        }
    }
    for (param_name, param_type, is_const) in &target.params {
        descriptors.push(format!("param:{param_name}:{param_type}:{is_const}"));
    }
    if let Some(return_type) = &target.return_type {
        descriptors.push(format!("return:{return_type}"));
    }
    for applied in &target.annotations {
        descriptors.push(format!("applied-annotation:{applied}"));
    }
    for capture in &target.captures {
        descriptors.push(format!("capture-name:{capture}"));
    }
    descriptors
}

/// Canonical structural content encoding of a generated `extend` method
/// (rejection row 3's conflicting-output detector). Taken over the
/// handler-emitted AST — post target-substitution, PRE parameter-annotation
/// enrichment — so the speculative pre-pass and the authoritative pass-2
/// run of one application encode equal output equally.
fn generated_extend_method_content(
    type_name: &shape_ast::ast::TypeName,
    method: &shape_ast::ast::types::MethodDef,
) -> CanonicalHash {
    CanonicalHash::from_canonical_decl_encoding(&format!("extend:{type_name:?}:{method:?}"))
}

/// Canonical structural content encoding of a generated free function.
fn generated_free_fn_content(func_def: &FunctionDef) -> CanonicalHash {
    CanonicalHash::from_canonical_decl_encoding(&format!("fn:{func_def:?}"))
}

/// ADR-009 D1 (S3): re-base a generated declaration's DECL-LEVEL spans to
/// the expansion's application anchor (Decision 68: no `Span::DUMMY`, no
/// spans indexing text that is not the compiling file). Handler-emitted
/// declarations are parsed from synthetic snippet text (`mod
/// __module_probe__ { … }`), built from typed `__ComptimeItemFragment`s
/// (whose scaffolding spans are `Span::default()`), or desugared from
/// handler-body AST — none of those spans resolve inside the file being
/// compiled, so the registered declaration anchors at the real application
/// site instead.
///
/// Scope line (recorded for the wave46 D1 addendum): decl-level anchors
/// only — the name span and synthesized type-param spans. Body-node spans
/// keep their handler-emitted offsets until ticket D2's virtual expansion
/// documents give generated bodies a real per-node mapping; those nodes are
/// covered by the decl's `GeneratedOrigin.node_path` in the meantime.
///
/// MUST be called AFTER the row-3 content fingerprint is taken: both phases
/// fingerprint the RAW handler-emitted AST, so anchoring before hashing in
/// one phase only would fabricate a row-3 "conflicting output" error.
fn anchor_generated_function_decl(func_def: &mut FunctionDef, anchor: Span) {
    func_def.name_span = anchor;
    if let Some(type_params) = func_def.type_params.as_mut() {
        for type_param in type_params {
            match type_param {
                shape_ast::ast::TypeParam::Type { span, .. }
                | shape_ast::ast::TypeParam::Const { span, .. } => *span = anchor,
            }
        }
    }
}

impl BytecodeCompiler {
    pub(super) fn apply_function_comptime_signature_directives_for_analysis(
        &mut self,
        program: &mut shape_ast::ast::Program,
    ) -> Result<()> {
        let handler_map = self.collect_comptime_annotation_handlers(program);
        if handler_map.is_empty() {
            return Ok(());
        }

        let extensions: Vec<_> = self
            .extension_registry
            .as_ref()
            .map(|r| r.as_ref().clone())
            .unwrap_or_default();
        let trait_impls = self.type_inference.env.trait_impl_keys();
        let known_type_symbols: HashSet<String> = self
            .struct_types
            .keys()
            .chain(self.type_aliases.keys())
            .cloned()
            .collect();
        let ctx_module_path = self.module_scope_stack.last().cloned().unwrap_or_default();
        let ctx_file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("")
            .to_string();

        Self::apply_function_comptime_signature_directives_to_items(
            self,
            &handler_map,
            &extensions,
            &trait_impls,
            &known_type_symbols,
            &ctx_module_path,
            &ctx_file,
            &mut program.items,
        )
    }

    fn apply_function_comptime_signature_directives_to_items(
        compiler: &mut BytecodeCompiler,
        handler_map: &HashMap<String, ComptimeAnnotationHandlers>,
        extensions: &[shape_runtime::module_exports::ModuleExports],
        trait_impls: &std::collections::HashSet<String>,
        known_type_symbols: &HashSet<String>,
        ctx_module_path: &str,
        ctx_file: &str,
        items: &mut [shape_ast::ast::Item],
    ) -> Result<()> {
        use shape_ast::ast::{ExportItem, Item};

        for item in items {
            match item {
                Item::Function(func, _) => {
                    compiler.apply_function_comptime_signature_directives_to_function(
                        handler_map,
                        extensions,
                        trait_impls,
                        known_type_symbols,
                        ctx_module_path,
                        ctx_file,
                        func,
                    )?;
                }
                Item::Export(export, _) => {
                    if let ExportItem::Function(func) = &mut export.item {
                        compiler.apply_function_comptime_signature_directives_to_function(
                            handler_map,
                            extensions,
                            trait_impls,
                            known_type_symbols,
                            ctx_module_path,
                            ctx_file,
                            func,
                        )?;
                    }
                }
                Item::Module(module, _) => {
                    Self::apply_function_comptime_signature_directives_to_items(
                        compiler,
                        handler_map,
                        extensions,
                        trait_impls,
                        known_type_symbols,
                        ctx_module_path,
                        ctx_file,
                        &mut module.items,
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_function_comptime_signature_directives_to_function(
        &mut self,
        handler_map: &HashMap<String, ComptimeAnnotationHandlers>,
        extensions: &[shape_runtime::module_exports::ModuleExports],
        trait_impls: &std::collections::HashSet<String>,
        known_type_symbols: &HashSet<String>,
        ctx_module_path: &str,
        ctx_file: &str,
        func_def: &mut FunctionDef,
    ) -> Result<()> {
        use shape_ast::ast::AnnotationHandlerType;

        let annotations = func_def.annotations.clone();
        let phases = [
            AnnotationHandlerType::ComptimePre,
            AnnotationHandlerType::ComptimePost,
        ];
        for phase in phases.iter() {
            for ann in &annotations {
                let Some(entry) = handler_map.get(ann.name.as_str()).or_else(|| {
                    ann.name
                        .rsplit("::")
                        .next()
                        .and_then(|bare| handler_map.get(bare))
                }) else {
                    continue;
                };
                for handler in entry.handlers.iter().filter(|h| &h.handler_type == phase) {
                    let target = super::comptime_target::ComptimeTarget::from_function(func_def);
                    let target_value = target.to_nanboxed()?;
                    let mut helpers = self.collect_comptime_helpers();
                    helpers.extend(self.collect_scoped_helpers_for_expr(&handler.body));
                    helpers.sort_by(|a, b| a.name.cmp(&b.name));
                    helpers.dedup_by(|a, b| a.name == b.name);

                    // S3 pre-pass freeze rule (see `s3_freeze_gate_tests`
                    // module doc): this signature-directive pre-pass runs
                    // AFTER the semantic-freeze barrier and consumes the
                    // real registration-complete freeze handle — the same
                    // one pass-2 uses. A site that cannot obtain it is the
                    // row-3 named compile error; the handle is acquired
                    // before the output-suppression toggle so the error
                    // path cannot leak suppression state.
                    let freeze = self.comptime_freeze_overlay()?;
                    let prev_suppressed =
                        super::comptime_builtins::set_comptime_output_suppressed(true);
                    let execution_result =
                        super::comptime::execute_comptime_with_annotation_handler(
                            &handler.body,
                            &handler.params,
                            target_value,
                            &ann.args,
                            &entry.def_param_names,
                            &[],
                            &helpers,
                            extensions,
                            known_type_symbols.clone(),
                            ctx_module_path,
                            ctx_file,
                            trait_impls.clone(),
                            freeze,
                        );
                    super::comptime_builtins::set_comptime_output_suppressed(prev_suppressed);

                    let execution = match execution_result {
                        Ok(execution) => execution,
                        Err(e) => {
                            if e.to_string().contains("[comptime error]") {
                                let context =
                                    format!("the @{} annotation on {}", ann.name, func_def.name);
                                return Err(self.build_comptime_failure(&e, ann.span, &context));
                            }
                            continue;
                        }
                    };
                    Self::apply_signature_directives_to_analysis_function(
                        func_def,
                        execution.directives,
                    )
                    .map_err(|message| ShapeError::RuntimeError {
                        message: format!(
                            "Comptime handler '{}' directive processing failed: {}",
                            ann.name, message
                        ),
                        location: Some(self.span_to_source_location(handler.span)),
                    })?;
                }
            }
        }
        Ok(())
    }

    fn apply_signature_directives_to_analysis_function(
        func_def: &mut FunctionDef,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
    ) -> std::result::Result<(), String> {
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::SetParamType {
                    param_name,
                    type_annotation,
                } => {
                    let Some(param) = func_def
                        .params
                        .iter_mut()
                        .find(|p| p.simple_name() == Some(param_name.as_str()))
                    else {
                        continue;
                    };
                    if let Some(existing) = &param.type_annotation {
                        if existing != &type_annotation {
                            return Err(format!(
                                "cannot override explicit type of parameter '{}'",
                                param_name
                            ));
                        }
                    } else {
                        param.type_annotation = Some(type_annotation);
                    }
                }
                super::comptime_builtins::ComptimeDirective::SetParamValue {
                    param_name,
                    value,
                } => {
                    let Some(param) = func_def
                        .params
                        .iter_mut()
                        .find(|p| p.simple_name() == Some(param_name.as_str()))
                    else {
                        continue;
                    };
                    param.default_value =
                        Some(Self::scalar_default_expr_from_kinded_slot(&param_name, &value)?);
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { .. } => {}
                _ => {}
            }
        }
        Ok(())
    }

    fn scalar_default_expr_from_kinded_slot(
        param_name: &str,
        value: &KindedSlot,
    ) -> std::result::Result<Expr, String> {
        let coerce_to_f64 = |slot: &KindedSlot| -> Option<f64> {
            match slot.kind() {
                shape_value::NativeKind::Int64 => slot.as_i64().map(|i| i as f64),
                shape_value::NativeKind::Float64 => slot.as_f64(),
                _ => None,
            }
        };
        if let Some(i) = value.as_i64() {
            Ok(Expr::Literal(Literal::Int(i), Span::DUMMY))
        } else if let Some(n) = coerce_to_f64(value) {
            Ok(Expr::Literal(Literal::Number(n), Span::DUMMY))
        } else if let Some(b) = value.as_bool() {
            Ok(Expr::Literal(Literal::Bool(b), Span::DUMMY))
        } else if let Some(s) = value.as_str() {
            Ok(Expr::Literal(Literal::String(s.to_string()), Span::DUMMY))
        } else if matches!(value.kind(), shape_value::NativeKind::Null) {
            Ok(Expr::Literal(Literal::None, Span::DUMMY))
        } else {
            Err(format!(
                "unsupported default value for parameter '{}': set param value only supports int, number, bool, string, and none scalars in this lane (got {:?})",
                param_name,
                value.kind()
            ))
        }
    }

    fn emit_empty_annotation_event_log(&mut self) {
        self.emit(Instruction::new(
            crate::compiler::v2_typed_emission::TypedArrayKind::String.new_opcode(),
            Some(Operand::Count(0)),
        ));
    }

    fn annotation_type_is_unknown(annotation: &TypeAnnotation) -> bool {
        match annotation {
            TypeAnnotation::Basic(name) => name == "unknown",
            TypeAnnotation::Reference(path) => path.as_str() == "unknown",
            TypeAnnotation::Array(inner) | TypeAnnotation::Borrow { inner, .. } => {
                Self::annotation_type_is_unknown(inner)
            }
            TypeAnnotation::Tuple(items)
            | TypeAnnotation::Union(items)
            | TypeAnnotation::Intersection(items) => {
                items.iter().any(Self::annotation_type_is_unknown)
            }
            TypeAnnotation::Object(fields) => fields
                .iter()
                .any(|field| Self::annotation_type_is_unknown(&field.type_annotation)),
            TypeAnnotation::Function { params, returns } => {
                params
                    .iter()
                    .any(|param| Self::annotation_type_is_unknown(&param.type_annotation))
                    || Self::annotation_type_is_unknown(returns)
            }
            TypeAnnotation::Generic { name, args } => {
                name.as_str() == "unknown" || args.iter().any(Self::annotation_type_is_unknown)
            }
            TypeAnnotation::Dyn(paths) => paths.iter().any(|path| path.as_str() == "unknown"),
            _ => false,
        }
    }

    fn annotation_param_type_annotation(
        &self,
        func_def: &FunctionDef,
        param_idx: usize,
        param: &shape_ast::ast::FunctionParameter,
    ) -> Option<TypeAnnotation> {
        if let Some(annotation) = param.type_annotation.as_ref() {
            return (!Self::annotation_type_is_unknown(annotation)).then(|| annotation.clone());
        }

        let shape_runtime::type_system::Type::Function { params, .. } =
            self.inference_facts.function_signature(&func_def.name)?
        else {
            return None;
        };
        let annotation = params.get(param_idx)?.to_annotation()?;
        (!Self::annotation_type_is_unknown(&annotation)).then_some(annotation)
    }

    fn annotation_arg_array_element_annotation(
        &self,
        func_def: &FunctionDef,
    ) -> Result<TypeAnnotation> {
        if func_def.params.is_empty() {
            return Ok(TypeAnnotation::Basic("int".to_string()));
        }

        let mut resolved: Option<TypeAnnotation> = None;
        for (idx, param) in func_def.params.iter().enumerate() {
            let annotation = self
                .annotation_param_type_annotation(func_def, idx, param)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "cannot build annotation args for function '{}': parameter #{} has no \
                         statically proven typed-array element carrier. Add a concrete parameter \
                         annotation or avoid runtime before/after annotations for this function.",
                        func_def.name,
                        idx + 1
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                })?;

            match resolved.as_ref() {
                Some(prev) if prev != &annotation => {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "cannot build annotation args for function '{}': parameters have \
                             heterogeneous element types. Runtime annotation args require a \
                             single statically proven element type.",
                            func_def.name
                        ),
                        location: Some(self.span_to_source_location(param.span())),
                    });
                }
                Some(_) => {}
                None => resolved = Some(annotation),
            }
        }

        resolved.ok_or_else(|| ShapeError::RuntimeError {
            message: format!(
                "Internal error: annotation arg element type for '{}' was not resolved",
                func_def.name
            ),
            location: None,
        })
    }

    fn annotation_arg_array_kind(
        &self,
        func_def: &FunctionDef,
        impl_idx: u16,
    ) -> Result<crate::compiler::v2_typed_emission::TypedArrayKind> {
        use crate::compiler::v2_typed_emission::{
            TypedArrayKind, should_use_typed_array_from_slot_kind,
        };
        use shape_ast::ast::TypeAnnotation;
        use shape_value::HeapKind;

        if func_def.params.is_empty() {
            return Ok(TypedArrayKind::I64);
        }

        let impl_hints = self
            .program
            .function_local_storage_hints
            .get(impl_idx as usize);
        let mut resolved = None;
        for (idx, param) in func_def.params.iter().enumerate() {
            let kind = impl_hints
                .and_then(|hints| hints.get(idx).copied())
                .and_then(|hint| match hint {
                    shape_value::NativeKind::Ptr(HeapKind::TypedArray) => {
                        Some(TypedArrayKind::TypedArray)
                    }
                    other => should_use_typed_array_from_slot_kind(other),
                })
                .or_else(|| {
                    let ann = self.annotation_param_type_annotation(func_def, idx, param)?;
                    let array_ann = TypeAnnotation::Array(Box::new(ann.clone()));
                    self.resolve_typed_array_kind_from_annotation(&array_ann)
                })
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "cannot build annotation args for function '{}': parameter #{} has no \
                         statically proven typed-array element carrier. Add a concrete parameter \
                         annotation or avoid runtime before/after annotations for this function.",
                        func_def.name,
                        idx + 1
                    ),
                    location: Some(self.span_to_source_location(param.span())),
                })?;

            match resolved {
                Some(prev) if prev != kind => {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "cannot build annotation args for function '{}': parameters have \
                             heterogeneous storage carriers. Runtime annotation args require a \
                             single statically proven typed-array element carrier.",
                            func_def.name
                        ),
                        location: Some(self.span_to_source_location(param.span())),
                    });
                }
                Some(_) => {}
                None => resolved = Some(kind),
            }
        }

        Ok(resolved.expect("non-empty params set a kind"))
    }

    fn emit_annotation_args_array(
        &mut self,
        func_def: &FunctionDef,
        wrapper_ref_params: &[bool],
        impl_idx: u16,
    ) -> Result<()> {
        let kind = self.annotation_arg_array_kind(func_def, impl_idx)?;
        self.emit(Instruction::new(
            kind.new_opcode(),
            Some(Operand::Count(func_def.params.len() as u16)),
        ));
        for (i, _param) in func_def.params.iter().enumerate() {
            self.emit(Instruction::simple(OpCode::Dup));
            if wrapper_ref_params.get(i).copied().unwrap_or(false) {
                self.emit(Instruction::new(
                    OpCode::DerefLoad,
                    Some(Operand::Local(i as u16)),
                ));
            } else {
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(i as u16)),
                ));
            }
            self.emit(Instruction::simple(kind.push_opcode()));
        }
        Ok(())
    }

    pub(super) fn emit_annotation_lifecycle_calls(&mut self, func_def: &FunctionDef) -> Result<()> {
        if self.current_function.is_some() {
            return Ok(());
        }
        if func_def.annotations.is_empty() {
            return Ok(());
        }

        let self_fn_idx =
            self.find_function(&func_def.name)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!(
                        "Internal error: function '{}' not found for annotation lifecycle dispatch",
                        func_def.name
                    ),
                    location: None,
                })? as u16;

        self.emit_annotation_lifecycle_calls_for_target(
            &func_def.annotations,
            &func_def.name,
            shape_ast::ast::functions::AnnotationTargetKind::Function,
            Some(self_fn_idx),
        )
    }

    pub(super) fn emit_annotation_lifecycle_calls_for_type(
        &mut self,
        type_name: &str,
        annotations: &[shape_ast::ast::Annotation],
    ) -> Result<()> {
        if self.current_function.is_some() || annotations.is_empty() {
            return Ok(());
        }
        self.emit_annotation_lifecycle_calls_for_target(
            annotations,
            type_name,
            shape_ast::ast::functions::AnnotationTargetKind::Type,
            Some(0),
        )
    }

    pub(super) fn emit_annotation_lifecycle_calls_for_module(
        &mut self,
        module_name: &str,
        annotations: &[shape_ast::ast::Annotation],
        target_id: Option<u16>,
    ) -> Result<()> {
        if self.current_function.is_some() || annotations.is_empty() {
            return Ok(());
        }
        self.emit_annotation_lifecycle_calls_for_target(
            annotations,
            module_name,
            shape_ast::ast::functions::AnnotationTargetKind::Module,
            target_id,
        )
    }

    fn emit_annotation_lifecycle_calls_for_target(
        &mut self,
        annotations: &[shape_ast::ast::Annotation],
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        for ann in annotations {
            let Some((_, compiled)) = self.lookup_compiled_annotation(ann) else {
                continue;
            };

            if let Some(on_define_id) = compiled.on_define_handler {
                self.emit_annotation_handler_call(
                    on_define_id,
                    ann,
                    target_name,
                    target_kind,
                    target_id,
                )?;
            }
            if let Some(metadata_id) = compiled.metadata_handler {
                self.emit_annotation_handler_call(
                    metadata_id,
                    ann,
                    target_name,
                    target_kind,
                    target_id,
                )?;
            }
        }

        Ok(())
    }

    fn emit_annotation_handler_call(
        &mut self,
        handler_id: u16,
        annotation: &shape_ast::ast::Annotation,
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        let handler = self
            .program
            .functions
            .get(handler_id as usize)
            .cloned()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Internal error: annotation handler function {} not found",
                    handler_id
                ),
                location: None,
            })?;
        let expected_base = 1 + annotation.args.len();
        let arity = handler.arity as usize;
        if arity < expected_base {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal error: annotation handler '{}' arity {} is smaller than required base args {}",
                    handler.name, arity, expected_base
                ),
                location: None,
            });
        }

        match target_kind {
            shape_ast::ast::functions::AnnotationTargetKind::Function => {
                let id = target_id.ok_or_else(|| ShapeError::RuntimeError {
                    message: "Internal error: missing function id for annotation handler call"
                        .to_string(),
                    location: None,
                })?;
                let self_ref = self.program.add_constant(Constant::Number(id as f64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(self_ref)),
                ));
            }
            _ => {
                self.emit_annotation_target_descriptor(target_name, target_kind, target_id)?;
            }
        }

        for ann_arg in &annotation.args {
            self.compile_expr(ann_arg)?;
        }

        for param_idx in expected_base..arity {
            let param_name = handler
                .param_names
                .get(param_idx)
                .map(|s| s.as_str())
                .unwrap_or_default();
            match param_name {
                "fn" | "target" => {
                    self.emit_annotation_target_descriptor(target_name, target_kind, target_id)?
                }
                "ctx" => self.emit_annotation_runtime_ctx()?,
                _ => {
                    self.emit(Instruction::simple(OpCode::PushNull));
                }
            }
        }

        let ac = self.program.add_constant(Constant::Int(arity as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(ac)),
        ));
        self.emit(Instruction::new(
            OpCode::Call,
            Some(Operand::Function(shape_value::FunctionId(handler_id))),
        ));
        self.record_blob_call(handler_id);
        self.emit(Instruction::simple(OpCode::Pop));
        Ok(())
    }

    fn annotation_target_kind_label(
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
    ) -> &'static str {
        match target_kind {
            shape_ast::ast::functions::AnnotationTargetKind::Function => "function",
            shape_ast::ast::functions::AnnotationTargetKind::Type => "type",
            shape_ast::ast::functions::AnnotationTargetKind::Module => "module",
            shape_ast::ast::functions::AnnotationTargetKind::Expression => "expression",
            shape_ast::ast::functions::AnnotationTargetKind::Block => "block",
            shape_ast::ast::functions::AnnotationTargetKind::AwaitExpr => "await_expr",
            shape_ast::ast::functions::AnnotationTargetKind::Binding => "binding",
        }
    }

    fn emit_annotation_runtime_ctx(&mut self) -> Result<()> {
        // W17.2-C §4.D.5 migration: empty-fields case uses the typed
        // variant directly (no Any fallback needed at empty-schema sites).
        let empty_schema_id = self.type_tracker.register_inline_object_schema_typed(&[]);
        if empty_schema_id > u16::MAX as u32 {
            return Err(ShapeError::RuntimeError {
                message: "Internal error: annotation ctx schema id overflow".to_string(),
                location: None,
            });
        }
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: empty_schema_id as u16,
                field_count: 0,
            }),
        ));
        self.emit_empty_annotation_event_log();

        let ctx_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("state", FieldType::Any),
            ("event_log", FieldType::Array(Box::new(FieldType::Any))),
        ]);
        if ctx_schema_id > u16::MAX as u32 {
            return Err(ShapeError::RuntimeError {
                message: "Internal error: annotation ctx schema id overflow".to_string(),
                location: None,
            });
        }
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: ctx_schema_id as u16,
                field_count: 2,
            }),
        ));
        Ok(())
    }

    fn emit_annotation_target_descriptor(
        &mut self,
        target_name: &str,
        target_kind: shape_ast::ast::functions::AnnotationTargetKind,
        target_id: Option<u16>,
    ) -> Result<()> {
        let name_const = self
            .program
            .add_constant(Constant::String(target_name.to_string()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(name_const)),
        ));
        let kind_const = self.program.add_constant(Constant::String(
            Self::annotation_target_kind_label(target_kind).to_string(),
        ));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(kind_const)),
        ));
        if let Some(id) = target_id {
            let id_const = self.program.add_constant(Constant::Number(id as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(id_const)),
            ));
        } else {
            self.emit(Instruction::simple(OpCode::PushNull));
        }

        let fn_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("name", FieldType::String),
            ("kind", FieldType::String),
            ("id", FieldType::I64),
        ]);
        if fn_schema_id > u16::MAX as u32 {
            return Err(ShapeError::RuntimeError {
                message: "Internal error: annotation fn schema id overflow".to_string(),
                location: None,
            });
        }
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: fn_schema_id as u16,
                field_count: 3,
            }),
        ));
        Ok(())
    }

    /// Execute comptime annotation handlers for a function definition.
    ///
    /// When an annotation has a `comptime pre/post(...) { ... }` handler, self builds
    /// a ComptimeTarget from the function definition and executes the handler body
    /// at compile time with the target object bound to the handler parameter.
    pub(super) fn execute_comptime_handlers(&mut self, func_def: &mut FunctionDef) -> Result<bool> {
        let mut removed = false;
        let annotations = func_def.annotations.clone();

        // Phase 1: comptime pre
        for ann in &annotations {
            if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
                if let Some(handler) = compiled.comptime_pre_handler {
                    if self.execute_function_comptime_handler(
                        ann,
                        &handler,
                        &compiled.param_names,
                        func_def,
                    )? {
                        removed = true;
                        break;
                    }
                }
            }
        }

        // Phase 2: comptime post
        if !removed {
            for ann in &annotations {
                if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
                    if let Some(handler) = compiled.comptime_post_handler {
                        if self.execute_function_comptime_handler(
                            ann,
                            &handler,
                            &compiled.param_names,
                            func_def,
                        )? {
                            removed = true;
                            break;
                        }
                    }
                }
            }
        }

        Ok(removed)
    }

    fn execute_function_comptime_handler(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        handler: &shape_ast::ast::AnnotationHandler,
        annotation_def_param_names: &[String],
        func_def: &mut FunctionDef,
    ) -> Result<bool> {
        // Build the target object from the function definition
        let target = super::comptime_target::ComptimeTarget::from_function(func_def);
        // ADR-009 D1 (S2): expansion site for this handler application.
        let expansion_site = self.annotation_expansion_site(annotation, handler, &target);
        // R8 W9 G.2 Step 2 Bucket 7: to_nanboxed now returns Result;
        // surface the V3-S5 ckpt-5 SURFACE through the caller's Result
        // chain instead of panicking.
        let target_value = target.to_nanboxed()?;
        let target_name = func_def.name.clone();
        let handler_span = handler.span;
        let const_bindings = self
            .specialization_const_bindings
            .get(&target_name)
            .cloned()
            .unwrap_or_default();

        let execution = self.execute_comptime_annotation_handler(
            annotation,
            handler,
            target_value,
            annotation_def_param_names,
            &const_bindings,
        )?;

        self.process_comptime_directives_for_function(
            execution.directives,
            &target_name,
            func_def,
            &expansion_site,
        )
        .map_err(|e| {
            // ADR-009 D1 (S4): provenance-carrying generated-decl failures
            // pass through with their location notes intact.
            self.preserve_or_wrap_directive_failure(
                e,
                &format!("Comptime handler '{}'", annotation.name),
                handler_span,
            )
        })
    }

    // ABI flipped to `KindedSlot` per ADR-006 §2.7.10 / Q11 to align
    // with `super::comptime::execute_comptime_with_annotation_handler`
    // (compiler/comptime.rs:486) and the kinded replacement noted in
    // the prior SURFACE comment. The `comptime_builtins::ComptimeDirective::
    // SetParamValue { value: KindedSlot }` migration in
    // `compiler/comptime_builtins.rs:33` is the precedent.
    pub(super) fn execute_comptime_annotation_handler(
        &mut self,
        annotation: &shape_ast::ast::Annotation,
        handler: &shape_ast::ast::AnnotationHandler,
        target_value: KindedSlot,
        annotation_def_param_names: &[String],
        const_bindings: &[(String, KindedSlot)],
    ) -> Result<super::comptime::ComptimeExecutionResult> {
        let handler_span = handler.span;
        let extensions: Vec<_> = self
            .extension_registry
            .as_ref()
            .map(|r| r.as_ref().clone())
            .unwrap_or_default();
        let trait_impls = self.type_inference.env.trait_impl_keys();
        let known_type_symbols: std::collections::HashSet<String> = self
            .struct_types
            .keys()
            .chain(self.type_aliases.keys())
            .cloned()
            .collect();
        let mut comptime_helpers = self.collect_comptime_helpers();
        comptime_helpers.extend(self.collect_scoped_helpers_for_expr(&handler.body));
        // For module-scoped helpers (e.g. "myext::schema_for"), add a bare-name
        // alias so that handler code written inside the module can call them
        // without qualification (e.g. "schema_for(uri)").
        let bare_aliases: Vec<_> = comptime_helpers
            .iter()
            .filter_map(|def| {
                let (_, bare) = def.name.rsplit_once("::")?;
                let mut alias = def.clone();
                alias.name = bare.to_string();
                Some(alias)
            })
            .collect();
        comptime_helpers.extend(bare_aliases);
        comptime_helpers.sort_by(|a, b| a.name.cmp(&b.name));
        comptime_helpers.dedup_by(|a, b| a.name == b.name);

        // §4.4: the comptime `ctx` compile-context (module_path + source file).
        let ctx_module_path = self.module_scope_stack.last().cloned().unwrap_or_default();
        let ctx_file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("")
            .to_string();

        let context = format!("the @{} annotation handler", annotation.name);
        // ADR-009 §4.1 (S2): authoritative handler execution consumes the
        // per-compilation-unit freeze handle — the empty-snapshot defect
        // (`TypeReflectionSnapshot::default()`) is deleted. This runs in
        // pass 2, after the freeze barrier; a handler reached without an
        // installed freeze is a compile error (row 3).
        let freeze = self.comptime_freeze_overlay()?;
        let execution = super::comptime::execute_comptime_with_annotation_handler(
            &handler.body,
            &handler.params,
            target_value,
            &annotation.args,
            annotation_def_param_names,
            const_bindings,
            &comptime_helpers,
            &extensions,
            known_type_symbols,
            &ctx_module_path,
            &ctx_file,
            trait_impls,
            freeze,
        )
        .map_err(|e| self.build_comptime_failure(&e, handler_span, &context))?;
        // §4.4: re-emit any `warning()` output anchored at this handler site.
        self.surface_comptime_warnings(&execution.warnings, handler_span);
        Ok(execution)
    }

    fn collect_scoped_helpers_for_expr(&self, expr: &Expr) -> Vec<FunctionDef> {
        let mut pending_names = Vec::new();
        let mut seed_names = HashSet::new();
        Self::collect_scoped_names_in_expr(expr, &mut seed_names);
        pending_names.extend(seed_names.into_iter());

        let mut visited = HashSet::new();
        let mut helpers = Vec::new();

        while let Some(name) = pending_names.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            let def = if let Some(d) = self.function_defs.get(&name) {
                d.clone()
            } else {
                // Try module-scoped lookup: for bare names like "schema_for",
                // check "module::schema_for" using the current module scope stack.
                let found = self.module_scope_stack.iter().rev().find_map(|module| {
                    let scoped = Self::qualify_module_symbol(module, &name);
                    self.function_defs.get(&scoped).cloned()
                });
                let Some(d) = found else { continue };
                d
            };
            helpers.push(def.clone());
            for stmt in &def.body {
                let mut nested = HashSet::new();
                Self::collect_scoped_names_in_statement(stmt, &mut nested);
                pending_names.extend(nested.into_iter().filter(|n| !visited.contains(n)));
            }
        }

        helpers
    }

    fn collect_scoped_names_in_statement(stmt: &Statement, names: &mut HashSet<String>) {
        match stmt {
            Statement::Return(Some(expr), _) => Self::collect_scoped_names_in_expr(expr, names),
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Statement::Assignment(assign, _) => {
                Self::collect_scoped_names_in_expr(&assign.value, names)
            }
            Statement::Expression(expr, _) => Self::collect_scoped_names_in_expr(expr, names),
            Statement::For(loop_expr, _) => {
                match &loop_expr.init {
                    shape_ast::ast::ForInit::ForIn { iter, .. } => {
                        Self::collect_scoped_names_in_expr(iter, names);
                    }
                    shape_ast::ast::ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        Self::collect_scoped_names_in_statement(init, names);
                        Self::collect_scoped_names_in_expr(condition, names);
                        Self::collect_scoped_names_in_expr(update, names);
                    }
                }
                for body_stmt in &loop_expr.body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
            }
            Statement::While(loop_expr, _) => {
                Self::collect_scoped_names_in_expr(&loop_expr.condition, names);
                for body_stmt in &loop_expr.body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
            }
            Statement::If(if_stmt, _) => {
                Self::collect_scoped_names_in_expr(&if_stmt.condition, names);
                for body_stmt in &if_stmt.then_body {
                    Self::collect_scoped_names_in_statement(body_stmt, names);
                }
                if let Some(else_body) = &if_stmt.else_body {
                    for body_stmt in else_body {
                        Self::collect_scoped_names_in_statement(body_stmt, names);
                    }
                }
            }
            Statement::SetReturnExpr { expression, .. }
            | Statement::SetParamValue { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. } => {
                Self::collect_scoped_names_in_expr(expression, names);
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            _ => {}
        }
    }

    fn collect_scoped_names_in_expr(expr: &Expr, names: &mut HashSet<String>) {
        match expr {
            Expr::MethodCall {
                receiver,
                method,
                args,
                named_args,
                ..
            } => {
                if let Expr::Identifier(namespace, _) = receiver.as_ref() {
                    names.insert(format!("{}::{}", namespace, method));
                }
                Self::collect_scoped_names_in_expr(receiver, names);
                for arg in args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::FunctionCall {
                name,
                args,
                named_args,
                ..
            } => {
                if name.contains("::") {
                    names.insert(name.clone());
                }
                for arg in args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::QualifiedFunctionCall {
                namespace,
                function,
                args,
                named_args,
                ..
            } => {
                names.insert(format!("{}::{}", namespace, function));
                for arg in args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                for (_, value) in named_args {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                Self::collect_scoped_names_in_expr(left, names);
                Self::collect_scoped_names_in_expr(right, names);
            }
            Expr::UnaryOp { operand, .. }
            | Expr::Spread(operand, _)
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::Reference { expr: operand, .. }
            | Expr::AsyncScope(operand, _)
            | Expr::DataRelativeAccess {
                reference: operand, ..
            } => {
                Self::collect_scoped_names_in_expr(operand, names);
            }
            Expr::PropertyAccess { object, .. } => {
                Self::collect_scoped_names_in_expr(object, names)
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                Self::collect_scoped_names_in_expr(object, names);
                Self::collect_scoped_names_in_expr(index, names);
                if let Some(end) = end_index {
                    Self::collect_scoped_names_in_expr(end, names);
                }
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                Self::collect_scoped_names_in_expr(condition, names);
                Self::collect_scoped_names_in_expr(then_expr, names);
                if let Some(else_expr) = else_expr {
                    Self::collect_scoped_names_in_expr(else_expr, names);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } | ObjectEntry::Spread(value) => {
                            Self::collect_scoped_names_in_expr(value, names);
                        }
                    }
                }
            }
            Expr::Array(values, _) => {
                for value in values {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::ListComprehension(comp, _) => {
                Self::collect_scoped_names_in_expr(&comp.element, names);
                for clause in &comp.clauses {
                    Self::collect_scoped_names_in_expr(&clause.iterable, names);
                    if let Some(filter) = &clause.filter {
                        Self::collect_scoped_names_in_expr(filter, names);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            if let Some(value) = &decl.value {
                                Self::collect_scoped_names_in_expr(value, names);
                            }
                        }
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            Self::collect_scoped_names_in_expr(&assign.value, names);
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            Self::collect_scoped_names_in_statement(stmt, names);
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => {
                            Self::collect_scoped_names_in_expr(expr, names);
                        }
                    }
                }
            }
            Expr::TypeAssertion {
                expr,
                meta_param_overrides,
                ..
            } => {
                Self::collect_scoped_names_in_expr(expr, names);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values() {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
            }
            Expr::InstanceOf { expr, .. } => Self::collect_scoped_names_in_expr(expr, names),
            Expr::FunctionExpr { body, .. } => {
                for stmt in body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::If(if_expr, _) => {
                Self::collect_scoped_names_in_expr(&if_expr.condition, names);
                Self::collect_scoped_names_in_expr(&if_expr.then_branch, names);
                if let Some(else_branch) = &if_expr.else_branch {
                    Self::collect_scoped_names_in_expr(else_branch, names);
                }
            }
            Expr::While(while_expr, _) => {
                Self::collect_scoped_names_in_expr(&while_expr.condition, names);
                Self::collect_scoped_names_in_expr(&while_expr.body, names);
            }
            Expr::For(for_expr, _) => {
                Self::collect_scoped_names_in_expr(&for_expr.iterable, names);
                Self::collect_scoped_names_in_expr(&for_expr.body, names);
            }
            Expr::Loop(loop_expr, _) => Self::collect_scoped_names_in_expr(&loop_expr.body, names),
            Expr::Let(let_expr, _) => {
                if let Some(value) = &let_expr.value {
                    Self::collect_scoped_names_in_expr(value, names);
                }
                Self::collect_scoped_names_in_expr(&let_expr.body, names);
            }
            Expr::Assign(assign_expr, _) => {
                Self::collect_scoped_names_in_expr(&assign_expr.target, names);
                Self::collect_scoped_names_in_expr(&assign_expr.value, names);
            }
            Expr::Break(Some(value), _) | Expr::Return(Some(value), _) => {
                Self::collect_scoped_names_in_expr(value, names);
            }
            Expr::Match(match_expr, _) => {
                Self::collect_scoped_names_in_expr(&match_expr.scrutinee, names);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        Self::collect_scoped_names_in_expr(guard, names);
                    }
                    Self::collect_scoped_names_in_expr(&arm.body, names);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    Self::collect_scoped_names_in_expr(start, names);
                }
                if let Some(end) = end {
                    Self::collect_scoped_names_in_expr(end, names);
                }
            }
            Expr::TimeframeContext { expr, .. } | Expr::UsingImpl { expr, .. } => {
                Self::collect_scoped_names_in_expr(expr, names);
            }
            Expr::SimulationCall { params, .. } => {
                for (_, value) in params {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::WindowExpr(window_expr, _) => {
                use shape_ast::ast::WindowFunction;

                match &window_expr.function {
                    WindowFunction::Lag { expr, default, .. }
                    | WindowFunction::Lead { expr, default, .. } => {
                        Self::collect_scoped_names_in_expr(expr, names);
                        if let Some(default) = default {
                            Self::collect_scoped_names_in_expr(default, names);
                        }
                    }
                    WindowFunction::FirstValue(expr)
                    | WindowFunction::LastValue(expr)
                    | WindowFunction::Sum(expr)
                    | WindowFunction::Avg(expr)
                    | WindowFunction::Min(expr)
                    | WindowFunction::Max(expr) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::NthValue(expr, _) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::Count(Some(expr)) => {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                    WindowFunction::Count(None)
                    | WindowFunction::RowNumber
                    | WindowFunction::Rank
                    | WindowFunction::DenseRank
                    | WindowFunction::Ntile(_) => {}
                }

                for expr in &window_expr.over.partition_by {
                    Self::collect_scoped_names_in_expr(expr, names);
                }
                if let Some(order_by) = &window_expr.over.order_by {
                    for (expr, _) in &order_by.columns {
                        Self::collect_scoped_names_in_expr(expr, names);
                    }
                }
            }
            Expr::FromQuery(from_query, _) => {
                Self::collect_scoped_names_in_expr(&from_query.source, names);
                for clause in &from_query.clauses {
                    match clause {
                        shape_ast::ast::QueryClause::Where(expr) => {
                            Self::collect_scoped_names_in_expr(expr, names);
                        }
                        shape_ast::ast::QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                Self::collect_scoped_names_in_expr(&spec.key, names);
                            }
                        }
                        shape_ast::ast::QueryClause::GroupBy { element, key, .. } => {
                            Self::collect_scoped_names_in_expr(element, names);
                            Self::collect_scoped_names_in_expr(key, names);
                        }
                        shape_ast::ast::QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            Self::collect_scoped_names_in_expr(source, names);
                            Self::collect_scoped_names_in_expr(left_key, names);
                            Self::collect_scoped_names_in_expr(right_key, names);
                        }
                        shape_ast::ast::QueryClause::Let { value, .. } => {
                            Self::collect_scoped_names_in_expr(value, names);
                        }
                    }
                }
                Self::collect_scoped_names_in_expr(&from_query.select, names);
            }
            Expr::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    Self::collect_scoped_names_in_expr(value, names);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    Self::collect_scoped_names_in_expr(&branch.expr, names);
                    for ann in &branch.annotations {
                        for arg in &ann.args {
                            Self::collect_scoped_names_in_expr(arg, names);
                        }
                    }
                }
            }
            Expr::Annotated {
                annotation, target, ..
            } => {
                for arg in &annotation.args {
                    Self::collect_scoped_names_in_expr(arg, names);
                }
                Self::collect_scoped_names_in_expr(target, names);
            }
            Expr::AsyncLet(async_let, _) => {
                Self::collect_scoped_names_in_expr(&async_let.expr, names)
            }
            Expr::Comptime(stmts, _) => {
                for stmt in stmts {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::ComptimeFor(comptime_for, _) => {
                Self::collect_scoped_names_in_expr(&comptime_for.iterable, names);
                for stmt in &comptime_for.body {
                    Self::collect_scoped_names_in_statement(stmt, names);
                }
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                shape_ast::ast::EnumConstructorPayload::Unit => {}
                shape_ast::ast::EnumConstructorPayload::Tuple(values) => {
                    for value in values {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
                shape_ast::ast::EnumConstructorPayload::Struct(fields) => {
                    for (_, value) in fields {
                        Self::collect_scoped_names_in_expr(value, names);
                    }
                }
            },
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for elem in row {
                        Self::collect_scoped_names_in_expr(elem, names);
                    }
                }
            }
            Expr::Literal(..)
            | Expr::Identifier(..)
            | Expr::DataRef(..)
            | Expr::DataDateTimeRef(..)
            | Expr::TimeRef(..)
            | Expr::DateTime(..)
            | Expr::PatternRef(..)
            | Expr::Duration(..)
            | Expr::Break(None, _)
            | Expr::Return(None, _)
            | Expr::Continue(..)
            // ADR-009 A2: type syntax is a leaf — no scoped names inside.
            | Expr::TypeSyntax(..)
            | Expr::Unit(..) => {}
        }
    }

    /// ADR-009 D1 (S4): the compiler-owned generated-symbol query surface —
    /// the ONE query API of spec §4.1 for generated declarations. Tooling
    /// (the LSP in slice S5, diagnostics here in S4) resolves generated
    /// symbols to `{SymbolId, checked-decl location, application location,
    /// generator-definition location}` and lists them for workspace-symbol
    /// consumption THROUGH this handle, answered from the S2 identity table
    /// only — never by text scan, never by a second expansion run
    /// (Decision 66 closing rule).
    ///
    /// Query the compiler AFTER compilation (`compile_in_place`) so the
    /// table holds every reserved expansion of the unit.
    pub fn generated_symbol_query(&self) -> &GeneratedSymbolTable {
        &self.generated_symbols
    }

    /// ADR-009 E3 (slice S1): the generated analysis items (`Item::Extend` /
    /// `Item::Function`) materialized by the executed declaration-discovery
    /// pre-pass for this compilation unit. Empty until `compile_in_place`
    /// runs. This is the executed authority that replaced the deleted
    /// non-evaluating static AST scan; static consumers augment their program
    /// view from this slice.
    pub fn generated_analysis_items(&self) -> &[shape_ast::ast::Item] {
        &self.generated_analysis_items
    }

    /// ADR-009 D1 (S2): build the [`ExpansionSite`] for one comptime
    /// annotation-handler application. Called by BOTH phases of the existing
    /// extend/materialization path — the speculative pre-pass
    /// (`materialize_computed_comptime_extends`) and the authoritative
    /// pass-2 handler execution sites — from the SAME AST inputs, so the two
    /// runs of one application agree on one `ExpansionIdentity` (risk 7:
    /// provenance must not double).
    pub(super) fn annotation_expansion_site(
        &self,
        annotation: &shape_ast::ast::Annotation,
        handler: &shape_ast::ast::AnnotationHandler,
        target: &super::comptime_target::ComptimeTarget,
    ) -> ExpansionSite {
        let file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("");
        let generator = GeneratorRef::from_canonical_descriptor(format!(
            "annotation:{}:{}",
            annotation.name,
            annotation_handler_kind_descriptor(&handler.handler_type)
        ));
        let application = ApplicationId::from_canonical_descriptor(format!(
            "application:{}:{}:{}",
            file, annotation.span.start, annotation.span.end
        ));
        let target_identity = TargetIdentity::from_canonical_descriptor(format!(
            "{}:{}",
            annotation_target_kind_descriptor(target.kind),
            target.name
        ));
        let argument_descriptors: Vec<(String, String)> = annotation
            .args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                (
                    index.to_string(),
                    super::comptime_target::expr_to_string_lossy(arg),
                )
            })
            .collect();
        let argument_refs: Vec<(&str, &str)> = argument_descriptors
            .iter()
            .map(|(name, descriptor)| (name.as_str(), descriptor.as_str()))
            .collect();
        let dependency_descriptors = comptime_target_dependency_descriptors(target);
        let dependency_refs: Vec<&str> =
            dependency_descriptors.iter().map(String::as_str).collect();
        ExpansionSite::new(
            ExpansionIdentity::new(
                generator,
                application,
                target_identity,
                ComptimeStage::AnnotationHandler,
                CanonicalHash::from_canonical_argument_descriptors(&argument_refs),
                CanonicalHash::from_canonical_dependency_descriptors(&dependency_refs),
            ),
            self.current_file_id,
            annotation.span,
            // Generator-definition anchor: the annotation's comptime
            // handler span (S4 query surface + row-7 diagnostics answer
            // "generator defined here" from this anchor).
            handler.span,
        )
    }

    /// ADR-009 D1 (S2): build the [`ExpansionSite`] for a `comptime { }`
    /// block emitting directives. The block is its own generator AND its own
    /// application site (there is no separate annotation application).
    pub(super) fn comptime_block_expansion_site(
        &self,
        span: Span,
        module_path: &str,
    ) -> ExpansionSite {
        let file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("");
        let block_descriptor = format!("comptime-block:{}:{}:{}", file, span.start, span.end);
        let no_arguments: [(&str, &str); 0] = [];
        ExpansionSite::new(
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor(block_descriptor.clone()),
                ApplicationId::from_canonical_descriptor(format!(
                    "application:{}:{}:{}",
                    file, span.start, span.end
                )),
                TargetIdentity::from_canonical_descriptor(format!("module:{module_path}")),
                ComptimeStage::ModuleComptimeBlock,
                CanonicalHash::from_canonical_argument_descriptors(&no_arguments),
                CanonicalHash::from_canonical_dependency_descriptors(&[]),
            ),
            self.current_file_id,
            span,
            // The comptime block is its own generator AND its own
            // application site: one span fills both anchor roles.
            span,
        )
    }

    /// Map a reservation-layer rejection (rows 1/2/3, a `String` carrying
    /// the named diagnostic + provenance rendering) into a spanned compile
    /// error anchored at the application site.
    fn expansion_rejection(&self, message: String, site: &ExpansionSite) -> ShapeError {
        ShapeError::SemanticError {
            message,
            location: Some(self.span_to_source_location(site.application_span())),
        }
    }

    pub(super) fn apply_comptime_extend(
        &mut self,
        mut extend: shape_ast::ast::ExtendStatement,
        target_name: &str,
        site: &ExpansionSite,
    ) -> Result<()> {
        match &mut extend.type_name {
            shape_ast::ast::TypeName::Simple(name) if name == "target" => {
                *name = target_name.into();
            }
            shape_ast::ast::TypeName::Generic { name, .. } if name == "target" => {
                *name = target_name.into();
            }
            _ => {}
        }

        // ADR-009 D1 (S2), rejection row 1: a generated declaration must
        // anchor at a real application span — refused HERE, before any
        // registration or compilation (Dec 68 required rejection). The
        // generator-definition anchor is held to the same rule (S4).
        let source_anchor = site
            .source_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;
        let generator_anchor = site
            .generator_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;

        // Row-3 content fingerprints are taken over the handler-emitted
        // method AST (post target-substitution, PRE parameter-annotation
        // enrichment) so the pre-pass and pass-2 encodings agree.
        let method_contents: Vec<CanonicalHash> = extend
            .methods
            .iter()
            .map(|method| generated_extend_method_content(&extend.type_name, method))
            .collect();

        self.annotate_comptime_extend_method_params(&mut extend.methods, target_name);

        let extend_type_str = match &extend.type_name {
            shape_ast::ast::TypeName::Simple(name) => name.clone(),
            shape_ast::ast::TypeName::Generic { name, .. } => name.clone(),
        };

        for (method, content) in extend.methods.iter().zip(method_contents) {
            let mut func_def = self.desugar_extend_method(method, &extend.type_name)?;
            // ADR-009 D1 (S3): the registered decl anchors at the real
            // application span (content fingerprint above is over the raw
            // handler-emitted AST — see `anchor_generated_function_decl`).
            anchor_generated_function_decl(&mut func_def, site.application_span());
            let node_path = GeneratedNodePath::decl_root(format!("extend:{extend_type_str}"))
                .child(format!("method:{}", method.name));
            let origin = GeneratedOrigin {
                expansion: site.identity().clone(),
                node_path: node_path.clone(),
                source_anchor,
            };
            // §4.9.1 + D1 identity-keyed dedup: if the whole-program
            // pre-pass already reserved this identity and registered the
            // method's SIGNATURE (so it is visible to the analyzer, method
            // dispatch, and every user body), the reservation is re-issued —
            // skip re-registering (a second `register_function` would create
            // a duplicate slot). The body is still compiled below, filling
            // the pre-registered slot, so the method is compiled exactly
            // once through the identical path.
            match self.generated_symbols.reserve_generated_decl(
                &func_def.name,
                origin,
                content,
                generator_anchor,
            ) {
                Ok(SymbolReservation::Fresh(_)) => {
                    // ADR-009 D1 (S4), rejection row 7: a diagnostic raised
                    // on a generated declaration carries generated-node +
                    // application + generator locations.
                    self.register_function(&func_def).map_err(|e| {
                        self.build_generated_decl_failure(&e, &func_def.name, &node_path, site)
                    })?;
                }
                Ok(SymbolReservation::Reissued(_)) => {}
                Err(message) => return Err(self.expansion_rejection(message, site)),
            }
            // Wave-38F generated-method JIT parity: hand-written `extend`
            // methods compile through the full driver (`compile_function`),
            // which lowers MIR and back-patches `Function.mir_data` for the
            // JIT. Generated methods need the same path; the signature was
            // already registered above/pre-pass, so this only fills the body
            // and MIR for the existing function slot.
            //
            // ADR-009 D1 (S4), rejection row 7: a body error inside the
            // generated method surfaces with full expansion provenance —
            // generated-node + application-site + generator-definition
            // locations — never as a bare error pointing at handler-emitted
            // offsets.
            self.compile_function(&func_def).map_err(|e| {
                self.build_generated_decl_failure(&e, &func_def.name, &node_path, site)
            })?;
        }
        Ok(())
    }

    /// §4.5.7: apply a computed `extend (expr)` directive — register + compile
    /// the generated items additively at the annotated item's module scope.
    /// Free functions and `extend Type { ... }` blocks are the v1 surface (the
    /// two shapes the derive/LLM showcases emit); other top-level item kinds
    /// surface a clean compile error rather than a partial implementation.
    ///
    /// Function signatures are registered in a first pass so generated items may
    /// reference one another, then bodies are compiled — the same two-phase
    /// shape the top-level pipeline uses.
    /// Comptime-excellence §4.5.1 whole-program pre-pass.
    ///
    /// A computed `extend (f"fn ...")` directive inside a comptime annotation
    /// handler only materializes its generated free function during pass-2,
    /// when the annotated *type* is compiled — which is *after* the analyzer
    /// and after user function bodies resolve their call sites. So a program
    /// with `fn main() { print(User_json_schema()) }` failed with "Undefined
    /// function", even though the same call at top level worked, because the
    /// generated `User_json_schema` was invisible to every earlier phase.
    ///
    /// This pre-pass runs the type-targeting comptime handlers *before* the
    /// analyzer, parses the generated source, and returns the generated free
    /// functions so the driver can insert them as ordinary program items. From
    /// there they flow through function registration, analysis, inference and
    /// pass-2 body compilation exactly like hand-written functions — visible to
    /// `fn main()` and to every user body.
    ///
    /// The pre-pass is speculative: any handler that fails here (missing
    /// helper, `error()` on a non-serializable field, etc.) is silently
    /// skipped — pass-2 re-runs the same handler authoritatively and surfaces
    /// the real diagnostic with its proper span. Every declaration it does
    /// materialize is reserved in the compiler's `GeneratedSymbolTable`
    /// under its `ExpansionIdentity` (ADR-009 D1) so pass-2's
    /// `apply_comptime_extend_items` re-issues the same reservation instead
    /// of registering it a second time.
    ///
    /// Both generated free functions and generated type-extension methods
    /// (`extend Type { method ... }`, §4.9.1) are hoisted: the extend's method
    /// signatures are registered here and the `extend` block is returned so the
    /// analyzer and method-dispatch resolution learn the method on the type
    /// before any user body compiles. Pass-2's `apply_comptime_extend` compiles
    /// each pre-registered method body.
    pub(super) fn materialize_computed_comptime_extends(
        &mut self,
        program: &shape_ast::ast::Program,
    ) -> Result<Vec<shape_ast::ast::Item>> {
        use shape_ast::ast::Item;

        // annotation bare-name -> (comptime handlers, annotation-def param names)
        let handler_map = self.collect_comptime_annotation_handlers(program);
        if handler_map.is_empty() {
            return Ok(Vec::new());
        }

        let extensions: Vec<_> = self
            .extension_registry
            .as_ref()
            .map(|r| r.as_ref().clone())
            .unwrap_or_default();
        let trait_impls = self.type_inference.env.trait_impl_keys();

        // Known type symbols: local struct/type names plus every type symbol
        // already registered (imported modules compiled in graph phase 1).
        let mut known_type_symbols: HashSet<String> = self
            .struct_types
            .keys()
            .chain(self.type_aliases.keys())
            .cloned()
            .collect();
        for item in &program.items {
            if let Item::StructType(sd, _) = item {
                known_type_symbols.insert(sd.name.clone());
            }
        }

        let ctx_module_path = self.module_scope_stack.last().cloned().unwrap_or_default();
        let ctx_file = self
            .program
            .debug_info
            .source_map
            .get_file(self.current_file_id)
            .unwrap_or("")
            .to_string();

        // Snapshot the struct definitions so we can borrow `self` mutably
        // while running the mini-VM.
        let struct_defs: Vec<shape_ast::ast::types::StructTypeDef> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::StructType(sd, _) => Some(sd.clone()),
                _ => None,
            })
            .collect();

        let mut generated: Vec<Item> = Vec::new();

        // ADR-009 D2 (Decision 67): the monotonic declaration-discovery fixed
        // point. The formerly single, unbounded speculative pass is now a
        // bounded worklist that reaches a fixed point BEFORE the analyzer runs
        // (this method is still invoked exactly once — the fixed point is the
        // SINGLE discovery pass, no speculative second evaluation). Each round
        // drains the worklist of struct definitions, runs every not-yet-run
        // annotation application once (run-once memo keyed on the full
        // `ExpansionIdentity` = ApplicationId + dependencies hash), records the
        // generated headers immutably, and enqueues any newly generated
        // annotated type for the next round (additions-only). The v1 directive
        // surface emits only free functions and `extend` methods — never a new
        // annotated type — so real programs converge in one round; the worklist
        // machinery makes multi-level discovery total and its rejections named.
        let mut discovery = DeclarationDiscoveryFixedPoint::new();
        let mut worklist: Vec<shape_ast::ast::types::StructTypeDef> = struct_defs;
        // Generated annotated type → the application whose expansion produced
        // it (the output-triggers edge source for cycle detection).
        let mut type_producer: HashMap<String, ExpansionIdentity> = HashMap::new();

        while !worklist.is_empty() {
            // Round bound (DISCOVERY_UNBOUNDED on overflow).
            discovery
                .begin_round()
                .map_err(|message| self.build_discovery_failure(message, None))?;
            let round_defs = std::mem::take(&mut worklist);
            // Frontier state for the monotone-convergence (oscillation) guard:
            // the sorted set of type names discovered/pending this round.
            let mut frontier: Vec<String> = round_defs.iter().map(|d| d.name.clone()).collect();
            frontier.sort();
            discovery
                .observe_round_state(&frontier)
                .map_err(|message| self.build_discovery_failure(message, None))?;
            // Types generated this round, re-scanned next round (additions-only;
            // discovered headers stay immutable through discovery).
            let mut newly_generated_types: Vec<shape_ast::ast::types::StructTypeDef> = Vec::new();

            for struct_def in &round_defs {
                for ann in &struct_def.annotations {
                    let Some(entry) = handler_map.get(ann.name.as_str()) else {
                        continue;
                    };
                    for handler in &entry.handlers {
                        let fields: Vec<(
                            String,
                            Option<TypeAnnotation>,
                            Vec<shape_ast::ast::functions::Annotation>,
                        )> = struct_def
                            .fields
                            .iter()
                            .map(|f| {
                                (
                                    f.name.clone(),
                                    Some(f.type_annotation.clone()),
                                    f.annotations.clone(),
                                )
                            })
                            .collect();

                        let target = super::comptime_target::ComptimeTarget::from_type(
                            &struct_def.name,
                            &fields,
                        );
                        // ADR-009 D1 (S2): the pre-pass builds the SAME expansion
                        // site pass-2 will build for this application (same ann
                        // node, same handler AST, same ComptimeTarget inputs), so
                        // both phases reserve one identity per generated decl.
                        let expansion_site = self.annotation_expansion_site(ann, handler, &target);
                        // ADR-009 D2 (Decision 67): output-triggers edge for cycle
                        // detection — if this struct was itself generated by an
                        // earlier expansion, record the producing application →
                        // this application edge (DISCOVERY_CYCLE on a closing edge).
                        if let Some(producer) = type_producer.get(&struct_def.name) {
                            discovery
                                .record_trigger(producer, expansion_site.identity())
                                .map_err(|message| {
                                    self.build_discovery_failure(message, Some(&expansion_site))
                                })?;
                        }
                        // ADR-009 D2 run-once memo: run each application exactly
                        // once per (ApplicationId + dependencies hash). A re-claimed
                        // identity (the struct re-enqueued in a later round)
                        // short-circuits — this is memoization, NOT a silent
                        // failure skip (DISCOVERY_UNBOUNDED on the expansion bound).
                        match discovery.claim(expansion_site.identity()) {
                            Ok(ApplicationClaim::Fresh) => {}
                            Ok(ApplicationClaim::AlreadyApplied) => continue,
                            Err(message) => {
                                return Err(
                                    self.build_discovery_failure(message, Some(&expansion_site))
                                );
                            }
                        }
                        let Ok(target_value) = target.to_nanboxed() else {
                            continue;
                        };

                        // Reachable comptime helpers for this handler body. At
                        // pre-pass time `function_defs` already holds every
                        // dependency-module function (graph phase 1); root helpers
                        // that are not yet registered simply fall back to pass-2.
                        let mut helpers = self.collect_comptime_helpers();
                        helpers.extend(self.collect_scoped_helpers_for_expr(&handler.body));
                        helpers.sort_by(|a, b| a.name.cmp(&b.name));
                        helpers.dedup_by(|a, b| a.name == b.name);

                        // §4.5.1: this pre-pass run is speculative (it only
                        // materializes generated function signatures); pass-2
                        // re-runs the same handler authoritatively. Suppress raw
                        // handler output during the speculative run so a handler
                        // that prints does not emit twice.
                        // S3 pre-pass freeze rule (see `s3_freeze_gate_tests`
                        // module doc): this speculative run fires AFTER the
                        // semantic-freeze barrier and consumes the real
                        // registration-complete freeze handle, so reflection-
                        // using handlers materialize their generated functions
                        // here (visible to every user body) instead of
                        // deferring to pass 2. A site that cannot obtain the
                        // handle is the row-3 named compile error; the handle
                        // is acquired before the output-suppression toggle so
                        // the error path cannot leak suppression state.
                        let freeze = self.comptime_freeze_overlay()?;
                        let prev_suppressed =
                            super::comptime_builtins::set_comptime_output_suppressed(true);
                        let execution_result =
                            super::comptime::execute_comptime_with_annotation_handler(
                                &handler.body,
                                &handler.params,
                                target_value,
                                &ann.args,
                                &entry.def_param_names,
                                &[],
                                &helpers,
                                &extensions,
                                known_type_symbols.clone(),
                                &ctx_module_path,
                                &ctx_file,
                                trait_impls.clone(),
                                freeze,
                            );
                        super::comptime_builtins::set_comptime_output_suppressed(prev_suppressed);
                        let execution = match execution_result {
                            Ok(execution) => execution,
                            Err(e) => {
                                // A genuine user `error()` call in the handler is a
                                // deterministic compile error — surface it here with
                                // a clean, spanned, LSDS-routed diagnostic anchored
                                // at the annotation application site (§4.4). If we
                                // swallowed it, the analyzer would instead reject the
                                // never-generated function with a confusing
                                // "Undefined function" and mask the real cause.
                                //
                                // Any other failure is treated as a pre-pass
                                // limitation (e.g. a helper only registered later)
                                // and deferred to pass-2, which re-runs the handler
                                // authoritatively.
                                if e.to_string().contains("[comptime error]") {
                                    let context = format!(
                                        "the @{} annotation on {}",
                                        ann.name, struct_def.name
                                    );
                                    return Err(self.build_comptime_failure(&e, ann.span, &context));
                                }
                                continue;
                            }
                        };

                        for directive in execution.directives {
                            // ADR-009 E3 (slice S1): the executed pre-pass is
                            // now the SINGLE authority for BOTH generated
                            // directive shapes — the computed
                            // `extend (expr)` snippet (`ExtendItems`) AND the
                            // direct `extend target { method }` handler form
                            // (`Extend`). The deleted non-evaluating static AST
                            // scan formerly carried the direct form into the
                            // analysis program; here the direct extend is
                            // target-substituted and normalized to the same
                            // `Item::Extend` the `ExtendItems` path emits, so a
                            // single item-processing loop reserves method
                            // signatures and returns the block for the
                            // analyzer. Pass-2's `apply_comptime_extend`
                            // re-issues the identical reservation (same
                            // `annotation_expansion_site`) and compiles the
                            // bodies.
                            let items: Vec<Item> = match directive {
                                super::comptime_builtins::ComptimeDirective::ExtendItems {
                                    items,
                                } => items,
                                super::comptime_builtins::ComptimeDirective::Extend(mut extend) => {
                                    match &mut extend.type_name {
                                        shape_ast::ast::TypeName::Simple(name)
                                            if name == "target" =>
                                        {
                                            *name = struct_def.name.clone().into();
                                        }
                                        shape_ast::ast::TypeName::Generic { name, .. }
                                            if name == "target" =>
                                        {
                                            *name = struct_def.name.clone().into();
                                        }
                                        _ => {}
                                    }
                                    vec![Item::Extend(
                                        extend,
                                        expansion_site.application_span(),
                                    )]
                                }
                                _ => continue,
                            };
                            // ADR-009 D1 (S2), rejection row 1: generated decls
                            // must anchor at the real application span; the
                            // generator-definition anchor is held to the same
                            // rule (S4).
                            let source_anchor =
                                expansion_site.source_anchor().map_err(|message| {
                                    self.expansion_rejection(message, &expansion_site)
                                })?;
                            let generator_anchor =
                                expansion_site.generator_anchor().map_err(|message| {
                                    self.expansion_rejection(message, &expansion_site)
                                })?;
                            for item in items {
                                match item {
                                    Item::Function(mut func_def, _span) => {
                                        let content = generated_free_fn_content(&func_def);
                                        // ADR-009 D1 (S3): anchor AFTER the raw
                                        // content fingerprint, so pass-2's raw
                                        // hash of the same output agrees.
                                        anchor_generated_function_decl(
                                            &mut func_def,
                                            expansion_site.application_span(),
                                        );
                                        let node_path = GeneratedNodePath::decl_root(format!(
                                            "fn:{}",
                                            func_def.name
                                        ));
                                        let origin = GeneratedOrigin {
                                            expansion: expansion_site.identity().clone(),
                                            node_path: node_path.clone(),
                                            source_anchor,
                                        };
                                        match self.generated_symbols.reserve_generated_decl(
                                            &func_def.name,
                                            origin,
                                            content,
                                            generator_anchor,
                                        ) {
                                            Ok(SymbolReservation::Fresh(_)) => {
                                                // Register the signature NOW so the
                                                // analyzer, function-registration pass, and
                                                // every user body (`fn main`) can resolve
                                                // the call. The BODY is still compiled by
                                                // pass-2's `apply_comptime_extend_items`
                                                // (`compile_function`) when the
                                                // annotated type compiles — the identical
                                                // path as before this pre-pass, so the
                                                // generated function's runtime/JIT
                                                // characteristics are unchanged.
                                                //
                                                // ADR-009 D1 (S4), row 7:
                                                // registration failures on the
                                                // generated decl carry full
                                                // expansion provenance.
                                                self.register_function(&func_def).map_err(|e| {
                                                    self.build_generated_decl_failure(
                                                        &e,
                                                        &func_def.name,
                                                        &node_path,
                                                        &expansion_site,
                                                    )
                                                })?;
                                                // ADR-009 D2: the discovered header
                                                // is immutable through the fixed
                                                // point (DISCOVERY_HEADER_MUTATED on
                                                // a differing re-derivation).
                                                discovery
                                                    .record_header(&func_def.name, content)
                                                    .map_err(|message| {
                                                        self.build_discovery_failure(
                                                            message,
                                                            Some(&expansion_site),
                                                        )
                                                    })?;
                                                generated.push(Item::Function(
                                                    func_def,
                                                    expansion_site.application_span(),
                                                ));
                                            }
                                            Ok(SymbolReservation::Reissued(_)) => {}
                                            Err(message) => {
                                                return Err(self.expansion_rejection(
                                                    message,
                                                    &expansion_site,
                                                ));
                                            }
                                        }
                                    }
                                    Item::Extend(mut extend, _span) => {
                                        // §4.9.1: a comptime-emitted type-extension
                                        // method (`u.to_json()`) must be visible to
                                        // the analyzer, method-dispatch resolution, and
                                        // every user body BEFORE pass-2 — exactly like
                                        // a generated free function. Reserve each
                                        // method's identity and register its SIGNATURE
                                        // now (keyed by its desugared `Type.method`
                                        // name), and return the `extend` block so the
                                        // analyzer learns the method on the type.
                                        // Pass-2's `apply_comptime_extend` re-issues the
                                        // same reservation and fills each pre-registered
                                        // slot through the normal function driver, so
                                        // generated methods get the same MIR/JIT surface
                                        // as hand-written `extend` methods.
                                        let extend_type_str = match &extend.type_name {
                                            shape_ast::ast::TypeName::Simple(name) => name.clone(),
                                            shape_ast::ast::TypeName::Generic { name, .. } => {
                                                name.clone()
                                            }
                                        };
                                        let mut any_new = false;
                                        for method in &extend.methods {
                                            let content = generated_extend_method_content(
                                                &extend.type_name,
                                                method,
                                            );
                                            let mut func_def = self
                                                .desugar_extend_method(method, &extend.type_name)?;
                                            // ADR-009 D1 (S3): anchor AFTER the
                                            // raw content fingerprint (pass-2
                                            // hashes the same raw AST).
                                            anchor_generated_function_decl(
                                                &mut func_def,
                                                expansion_site.application_span(),
                                            );
                                            let node_path = GeneratedNodePath::decl_root(format!(
                                                "extend:{extend_type_str}"
                                            ))
                                            .child(format!("method:{}", method.name));
                                            let origin = GeneratedOrigin {
                                                expansion: expansion_site.identity().clone(),
                                                node_path: node_path.clone(),
                                                source_anchor,
                                            };
                                            match self.generated_symbols.reserve_generated_decl(
                                                &func_def.name,
                                                origin,
                                                content,
                                                generator_anchor,
                                            ) {
                                                Ok(SymbolReservation::Fresh(_)) => {
                                                    // ADR-009 D1 (S4), row 7:
                                                    // provenance on registration
                                                    // failures.
                                                    self.register_function(&func_def).map_err(
                                                        |e| {
                                                            self.build_generated_decl_failure(
                                                                &e,
                                                                &func_def.name,
                                                                &node_path,
                                                                &expansion_site,
                                                            )
                                                        },
                                                    )?;
                                                    // ADR-009 D2: header immutable
                                                    // through the fixed point.
                                                    discovery
                                                        .record_header(&func_def.name, content)
                                                        .map_err(|message| {
                                                            self.build_discovery_failure(
                                                                message,
                                                                Some(&expansion_site),
                                                            )
                                                        })?;
                                                    any_new = true;
                                                }
                                                Ok(SymbolReservation::Reissued(_)) => {}
                                                Err(message) => {
                                                    return Err(self.expansion_rejection(
                                                        message,
                                                        &expansion_site,
                                                    ));
                                                }
                                            }
                                        }
                                        if any_new {
                                            // ADR-009 D1 (S3): the analysis copy
                                            // anchors its decl-level spans at the
                                            // application site too (method body
                                            // spans stay handler-emitted — D2
                                            // scope line, see
                                            // `anchor_generated_function_decl`).
                                            for method in &mut extend.methods {
                                                method.span = expansion_site.application_span();
                                            }
                                            generated.push(Item::Extend(
                                                extend,
                                                expansion_site.application_span(),
                                            ));
                                        }
                                    }
                                    Item::StructType(sd, _span) => {
                                        // ADR-009 D2 additions-only re-scan: a
                                        // generated ANNOTATED type is enqueued for
                                        // the next discovery round so its own
                                        // annotation applications are discovered
                                        // (its header stays immutable once
                                        // discovered). The producing application is
                                        // recorded as the output-triggers edge
                                        // source for cycle detection. The v1
                                        // directive surface never emits a generated
                                        // annotated type, so this arm is dormant on
                                        // real programs — it makes multi-level
                                        // discovery total.
                                        if !sd.annotations.is_empty()
                                            && known_type_symbols.insert(sd.name.clone())
                                        {
                                            type_producer.insert(
                                                sd.name.clone(),
                                                expansion_site.identity().clone(),
                                            );
                                            newly_generated_types.push(sd);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            // ADR-009 D2 additions-only: enqueue this round's newly generated
            // annotated types for re-scan in the next discovery round.
            for sd in newly_generated_types {
                worklist.push(sd);
            }
        }

        // ADR-009 D2: the fixed point converged — the worklist is empty and
        // every reserved generated identity was defined
        // (RESERVED_IDENTITY_UNDEFINED otherwise). Declaration discovery is
        // complete BEFORE the analyzer/inference/body-checking runs below
        // (discovery-before-body ordering, Decision 67).
        discovery
            .converge()
            .map_err(|message| self.build_discovery_failure(message, None))?;

        // The generated free functions are returned so the driver can add them
        // to the ANALYSIS program (the analyzer type-checks their call sites and
        // their bodies). They are NOT added to the compiled program's items:
        // their signatures are already registered above, and their bodies are
        // compiled by pass-2 through the normal function driver.
        Ok(generated)
    }

    /// Collect comptime annotation handlers reachable at pre-pass time, keyed
    /// by the annotation's bare name. Sources: annotation definitions in the
    /// root program plus every dependency-module AST in the module graph
    /// (imported annotations such as `@json_schema` are compiled in graph
    /// phase 1, so their handler AST is already reachable through the graph).
    fn collect_comptime_annotation_handlers(
        &self,
        program: &shape_ast::ast::Program,
    ) -> HashMap<String, ComptimeAnnotationHandlers> {
        use shape_ast::ast::{AnnotationHandlerType, ExportItem, Item};

        let mut map: HashMap<String, ComptimeAnnotationHandlers> = HashMap::new();

        let mut ingest = |ann_def: &shape_ast::ast::AnnotationDef| {
            let handlers: Vec<shape_ast::ast::AnnotationHandler> = ann_def
                .handlers
                .iter()
                .filter(|h| {
                    matches!(
                        h.handler_type,
                        AnnotationHandlerType::ComptimePre | AnnotationHandlerType::ComptimePost
                    )
                })
                .cloned()
                .collect();
            if handlers.is_empty() {
                return;
            }
            let def_param_names: Vec<String> = ann_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect();
            // Vacant-only: root/local definitions are ingested first and win.
            map.entry(ann_def.name.clone())
                .or_insert(ComptimeAnnotationHandlers {
                    handlers,
                    def_param_names,
                });
        };

        let mut ingest_items = |items: &[Item]| {
            for item in items {
                match item {
                    Item::AnnotationDef(ann_def, _) => ingest(ann_def),
                    Item::Export(export, _) => {
                        if let ExportItem::Annotation(ann_def) = &export.item {
                            ingest(ann_def);
                        }
                    }
                    _ => {}
                }
            }
        };

        ingest_items(&program.items);
        if let Some(graph) = &self.module_graph {
            for node in graph.nodes() {
                if let Some(ast) = &node.ast {
                    ingest_items(&ast.items);
                }
            }
        }

        map
    }

    pub(super) fn apply_comptime_extend_items(
        &mut self,
        items: Vec<shape_ast::ast::Item>,
        target_name: &str,
        site: &ExpansionSite,
    ) -> Result<()> {
        use shape_ast::ast::Item;

        let mut functions: Vec<FunctionDef> = Vec::new();
        let mut extends: Vec<shape_ast::ast::ExtendStatement> = Vec::new();
        for item in items {
            match item {
                Item::Function(func_def, _) => functions.push(func_def),
                Item::Extend(extend, _) => extends.push(extend),
                other => {
                    return Err(ShapeError::RuntimeError {
                        message: format!(
                            "comptime `extend (...)` generated an item kind that is not \
                             supported in v1 — only free functions and `extend Type {{ ... }}` \
                             blocks may be generated (got {})",
                            Self::generated_item_kind_name(&other)
                        ),
                        location: None,
                    });
                }
            }
        }

        // ADR-009 D1 (S2), rejection row 1: generated decls must anchor at
        // the real application span — refused before any registration. The
        // generator-definition anchor is held to the same rule (S4).
        let source_anchor = site
            .source_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;
        let generator_anchor = site
            .generator_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;

        // comptime-excellence §4.5.1 + D1 identity-keyed dedup: if the
        // whole-program pre-pass already reserved this generated free
        // function's identity and registered its SIGNATURE (so it is visible
        // to `fn main()` and every user body), the reservation is re-issued —
        // skip re-registering it here (a second `register_function` would
        // create a duplicate slot). The body is still compiled below via
        // `compile_function`, which fills the pre-registered slot, so the
        // generated function is compiled exactly once through the identical
        // path as before the pre-pass existed.
        for func_def in &mut functions {
            let content = generated_free_fn_content(func_def);
            // ADR-009 D1 (S3): anchor AFTER the raw content fingerprint —
            // the pre-pass hashed the same raw AST (see
            // `anchor_generated_function_decl`).
            anchor_generated_function_decl(func_def, site.application_span());
            let node_path = GeneratedNodePath::decl_root(format!("fn:{}", func_def.name));
            let origin = GeneratedOrigin {
                expansion: site.identity().clone(),
                node_path: node_path.clone(),
                source_anchor,
            };
            match self.generated_symbols.reserve_generated_decl(
                &func_def.name,
                origin,
                content,
                generator_anchor,
            ) {
                Ok(SymbolReservation::Fresh(_)) => {
                    // ADR-009 D1 (S4), row 7: provenance on registration
                    // failures.
                    self.register_function(func_def).map_err(|e| {
                        self.build_generated_decl_failure(&e, &func_def.name, &node_path, site)
                    })?;
                }
                Ok(SymbolReservation::Reissued(_)) => {}
                Err(message) => return Err(self.expansion_rejection(message, site)),
            }
        }
        for func_def in &functions {
            // WF-3D generated-fn JIT parity: compile via the FULL driver
            // (`compile_function`), not the bytecode-only `compile_function_body`.
            // The driver lowers the body to MIR and attaches `Function.mir_data`,
            // which the JIT's Phase-4 MirToIR pass requires — a bytecode-only
            // generated function fails Phase-4 ("has no MIR data") and forces a
            // whole-program deopt to the interpreter. A hand-written free function
            // goes through `compile_function`; routing the generated one through
            // the same path gives it native JIT codegen and VM == JIT.
            //
            // ADR-009 D1 (S4), rejection row 7: a body error inside the
            // generated free function surfaces with full expansion
            // provenance (generated-node + application + generator
            // locations).
            self.compile_function(func_def).map_err(|e| {
                let node_path = GeneratedNodePath::decl_root(format!("fn:{}", func_def.name));
                self.build_generated_decl_failure(&e, &func_def.name, &node_path, site)
            })?;
        }
        for extend in extends {
            self.apply_comptime_extend(extend, target_name, site)?;
        }
        Ok(())
    }

    fn generated_item_kind_name(item: &shape_ast::ast::Item) -> &'static str {
        use shape_ast::ast::Item;
        match item {
            Item::Function(..) => "function",
            Item::Extend(..) => "extend",
            Item::StructType(..) => "type",
            Item::Enum(..) => "enum",
            Item::Trait(..) => "trait",
            Item::Impl(..) => "impl",
            _ => "item",
        }
    }

    fn annotate_comptime_extend_method_params(
        &self,
        methods: &mut [shape_ast::ast::types::MethodDef],
        target_name: &str,
    ) {
        let Some(struct_def) = self.comptime_context_struct_defs.get(target_name) else {
            return;
        };
        let field_types: HashMap<&str, &TypeAnnotation> = struct_def
            .fields
            .iter()
            .map(|field| (field.name.as_str(), &field.type_annotation))
            .collect();
        let target_annotation = TypeAnnotation::Basic(target_name.to_string());

        for method in methods {
            for param_idx in 0..method.params.len() {
                if method.params[param_idx].type_annotation.is_some() {
                    continue;
                }
                let Some(param_name) = method.params[param_idx].simple_name() else {
                    continue;
                };

                let inferred = if Self::body_accesses_target_field_on_param(
                    &method.body,
                    param_name,
                    &field_types,
                ) {
                    Some(target_annotation.clone())
                } else {
                    Self::infer_param_type_from_self_field_binary(
                        &method.body,
                        param_name,
                        &field_types,
                    )
                };

                if let Some(type_annotation) = inferred {
                    method.params[param_idx].type_annotation = Some(type_annotation);
                }
            }
        }
    }

    fn body_accesses_target_field_on_param(
        body: &[Statement],
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> bool {
        body.iter().any(|stmt| {
            Self::statement_accesses_target_field_on_param(stmt, param_name, field_types)
        })
    }

    fn statement_accesses_target_field_on_param(
        stmt: &Statement,
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> bool {
        match stmt {
            Statement::Return(Some(expr), _) | Statement::Expression(expr, _) => {
                Self::expr_accesses_target_field_on_param(expr, param_name, field_types)
            }
            Statement::VariableDecl(decl, _) => decl.value.as_ref().is_some_and(|expr| {
                Self::expr_accesses_target_field_on_param(expr, param_name, field_types)
            }),
            Statement::Assignment(assign, _) => {
                Self::expr_accesses_target_field_on_param(&assign.value, param_name, field_types)
            }
            Statement::If(if_stmt, _) => {
                Self::expr_accesses_target_field_on_param(
                    &if_stmt.condition,
                    param_name,
                    field_types,
                ) || if_stmt.then_body.iter().any(|stmt| {
                    Self::statement_accesses_target_field_on_param(stmt, param_name, field_types)
                }) || if_stmt.else_body.as_ref().is_some_and(|body| {
                    body.iter().any(|stmt| {
                        Self::statement_accesses_target_field_on_param(
                            stmt,
                            param_name,
                            field_types,
                        )
                    })
                })
            }
            _ => false,
        }
    }

    fn expr_accesses_target_field_on_param(
        expr: &Expr,
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> bool {
        match expr {
            Expr::PropertyAccess { object, .. } => {
                matches!(&**object, Expr::Identifier(name, _) if name == param_name)
                    || Self::expr_accesses_target_field_on_param(object, param_name, field_types)
            }
            Expr::BinaryOp { left, right, .. } => {
                Self::expr_accesses_target_field_on_param(left, param_name, field_types)
                    || Self::expr_accesses_target_field_on_param(right, param_name, field_types)
            }
            Expr::UnaryOp { operand, .. } => {
                Self::expr_accesses_target_field_on_param(operand, param_name, field_types)
            }
            Expr::FunctionCall { args, .. }
            | Expr::QualifiedFunctionCall { args, .. }
            | Expr::Array(args, _) => args.iter().any(|expr| {
                Self::expr_accesses_target_field_on_param(expr, param_name, field_types)
            }),
            _ => false,
        }
    }

    fn infer_param_type_from_self_field_binary(
        body: &[Statement],
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> Option<TypeAnnotation> {
        body.iter().find_map(|stmt| {
            Self::statement_infer_param_type_from_self_field_binary(stmt, param_name, field_types)
        })
    }

    fn statement_infer_param_type_from_self_field_binary(
        stmt: &Statement,
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> Option<TypeAnnotation> {
        match stmt {
            Statement::Return(Some(expr), _) | Statement::Expression(expr, _) => {
                Self::expr_infer_param_type_from_self_field_binary(expr, param_name, field_types)
            }
            Statement::VariableDecl(decl, _) => decl.value.as_ref().and_then(|expr| {
                Self::expr_infer_param_type_from_self_field_binary(expr, param_name, field_types)
            }),
            Statement::Assignment(assign, _) => Self::expr_infer_param_type_from_self_field_binary(
                &assign.value,
                param_name,
                field_types,
            ),
            Statement::If(if_stmt, _) => Self::expr_infer_param_type_from_self_field_binary(
                &if_stmt.condition,
                param_name,
                field_types,
            )
            .or_else(|| {
                if_stmt.then_body.iter().find_map(|stmt| {
                    Self::statement_infer_param_type_from_self_field_binary(
                        stmt,
                        param_name,
                        field_types,
                    )
                })
            })
            .or_else(|| {
                if_stmt.else_body.as_ref().and_then(|body| {
                    body.iter().find_map(|stmt| {
                        Self::statement_infer_param_type_from_self_field_binary(
                            stmt,
                            param_name,
                            field_types,
                        )
                    })
                })
            }),
            _ => None,
        }
    }

    fn expr_infer_param_type_from_self_field_binary(
        expr: &Expr,
        param_name: &str,
        field_types: &HashMap<&str, &TypeAnnotation>,
    ) -> Option<TypeAnnotation> {
        match expr {
            Expr::BinaryOp { left, right, .. } => {
                if Self::expr_is_identifier(right, param_name) {
                    if let Some(field_type) = Self::self_field_type(left, field_types) {
                        return Some(field_type.clone());
                    }
                }
                if Self::expr_is_identifier(left, param_name) {
                    if let Some(field_type) = Self::self_field_type(right, field_types) {
                        return Some(field_type.clone());
                    }
                }
                Self::expr_infer_param_type_from_self_field_binary(left, param_name, field_types)
                    .or_else(|| {
                        Self::expr_infer_param_type_from_self_field_binary(
                            right,
                            param_name,
                            field_types,
                        )
                    })
            }
            Expr::UnaryOp { operand, .. } => {
                Self::expr_infer_param_type_from_self_field_binary(operand, param_name, field_types)
            }
            Expr::FunctionCall { args, .. }
            | Expr::QualifiedFunctionCall { args, .. }
            | Expr::Array(args, _) => args.iter().find_map(|expr| {
                Self::expr_infer_param_type_from_self_field_binary(expr, param_name, field_types)
            }),
            _ => None,
        }
    }

    fn self_field_type<'a>(
        expr: &Expr,
        field_types: &HashMap<&str, &'a TypeAnnotation>,
    ) -> Option<&'a TypeAnnotation> {
        let Expr::PropertyAccess {
            object, property, ..
        } = expr
        else {
            return None;
        };
        if matches!(&**object, Expr::Identifier(name, _) if name == "self") {
            field_types.get(property.as_str()).copied()
        } else {
            None
        }
    }

    fn expr_is_identifier(expr: &Expr, expected: &str) -> bool {
        matches!(expr, Expr::Identifier(name, _) if name == expected)
    }

    pub(super) fn process_comptime_directives(
        &mut self,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        target_name: &str,
        site: &ExpansionSite,
    ) -> Result<bool> {
        let mut removed = false;
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                    self.apply_comptime_extend(extend, target_name, site)?;
                }
                super::comptime_builtins::ComptimeDirective::ExtendItems { items } => {
                    self.apply_comptime_extend_items(items, target_name, site)?;
                }
                super::comptime_builtins::ComptimeDirective::RemoveTarget => {
                    removed = true;
                    break;
                }
                super::comptime_builtins::ComptimeDirective::SetParamType { .. }
                | super::comptime_builtins::ComptimeDirective::SetParamValue { .. } => {
                    return Err(Self::directive_error(
                        "`set param` directives are only valid when compiling function targets",
                    ));
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { .. } => {
                    return Err(Self::directive_error(
                        "`set return` directives are only valid when compiling function targets",
                    ));
                }
                super::comptime_builtins::ComptimeDirective::ReplaceBody { .. } => {
                    return Err(Self::directive_error(
                        "`replace body` directives are only valid when compiling function targets",
                    ));
                }
                super::comptime_builtins::ComptimeDirective::ReplaceModule { .. } => {
                    return Err(Self::directive_error(
                        "`replace module` directives are only valid when compiling module targets",
                    ));
                }
            }
        }
        Ok(removed)
    }

    pub(super) fn process_comptime_directives_for_function(
        &mut self,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        target_name: &str,
        func_def: &mut FunctionDef,
        site: &ExpansionSite,
    ) -> Result<bool> {
        let mut removed = false;
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                    self.apply_comptime_extend(extend, target_name, site)?;
                }
                super::comptime_builtins::ComptimeDirective::ExtendItems { items } => {
                    self.apply_comptime_extend_items(items, target_name, site)?;
                }
                super::comptime_builtins::ComptimeDirective::RemoveTarget => {
                    removed = true;
                    break;
                }
                super::comptime_builtins::ComptimeDirective::SetParamType {
                    param_name,
                    type_annotation,
                } => {
                    let maybe_param = func_def
                        .params
                        .iter_mut()
                        .find(|p| p.simple_name() == Some(param_name.as_str()));
                    let Some(param) = maybe_param else {
                        return Err(Self::directive_error(format!(
                            "comptime directive referenced unknown parameter '{}'",
                            param_name
                        )));
                    };
                    if let Some(existing) = &param.type_annotation {
                        if existing != &type_annotation {
                            return Err(Self::directive_error(format!(
                                "cannot override explicit type of parameter '{}'",
                                param_name
                            )));
                        }
                    } else {
                        param.type_annotation = Some(type_annotation);
                    }
                }
                super::comptime_builtins::ComptimeDirective::SetParamValue {
                    param_name,
                    value,
                } => {
                    let maybe_param = func_def
                        .params
                        .iter_mut()
                        .find(|p| p.simple_name() == Some(param_name.as_str()));
                    let Some(param) = maybe_param else {
                        return Err(Self::directive_error(format!(
                            "comptime directive referenced unknown parameter '{}'",
                            param_name
                        )));
                    };
                    param.default_value = Some(
                        Self::scalar_default_expr_from_kinded_slot(&param_name, &value)
                            .map_err(Self::directive_error)?,
                    );
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { type_annotation } => {
                    if let Some(existing) = &func_def.return_type {
                        if existing != &type_annotation {
                            return Err(Self::directive_error(
                                "cannot override explicit function return type annotation",
                            ));
                        }
                    } else {
                        func_def.return_type = Some(type_annotation);
                    }
                }
                super::comptime_builtins::ComptimeDirective::ReplaceBody { body } => {
                    // Create a shadow function from the original body so the
                    // replacement can call __original__ to invoke the original
                    // implementation.
                    let shadow_name = format!("__original__{}", func_def.name);
                    let shadow_def = FunctionDef {
                        name: shadow_name.clone(),
                        name_span: func_def.name_span,
                        declaring_module_path: func_def.declaring_module_path.clone(),
                        doc_comment: None,
                        params: func_def.params.clone(),
                        return_type: func_def.return_type.clone(),
                        body: func_def.body.clone(),
                        type_params: func_def.type_params.clone(),
                        annotations: Vec::new(),
                        where_clause: None,
                        is_async: func_def.is_async,
                        is_comptime: func_def.is_comptime,
                    };
                    self.register_function(&shadow_def)?;
                    self.compile_function_body(&shadow_def)?;

                    // Phase 3e: copy the original's inferred return type
                    // onto the shadow so call sites of `__original__` see
                    // the same return-type info. Without this, the
                    // numeric-typed call path treats the shadow's return
                    // as Unknown and `__original__() + 1` falls into
                    // trait dispatch. U4-5b: copied STRUCTURALLY as a
                    // `ConcreteType`.
                    if let Some(rt) = self
                        .type_tracker
                        .get_function_return_concrete_type(&func_def.name)
                        .cloned()
                    {
                        self.type_tracker
                            .register_function_return_concrete_type(&shadow_name, rt);
                    }

                    // Register alias so __original__ resolves to the shadow function.
                    self.function_aliases
                        .insert("__original__".to_string(), shadow_name);

                    // §4.5.5: `__original__` is a direct typed call to the
                    // shadow function, which carries the original's EXACT
                    // signature (params + return type cloned above). Forwarding
                    // is written `__original__(a, b, ...)` with the real
                    // parameter names and is type-checked like any other call.
                    // The previous convention injected a hidden
                    // `let args = [param1, ...]` binding and expected
                    // `__original__(args)` — that passed one array where N
                    // scalars are declared, silently reinterpreting an array
                    // pointer as an int (audit garbage / arity error). No
                    // hidden binding is injected into user scope anymore.
                    func_def.body = body;
                }
                super::comptime_builtins::ComptimeDirective::ReplaceModule { .. } => {
                    return Err(Self::directive_error(
                        "`replace module` directives are only valid when compiling module targets",
                    ));
                }
            }
        }
        Ok(removed)
    }

    /// S3 (design §4.5): a comptime `set return` / `set param` directive
    /// changed `effective_def`'s signature. Re-run the ordinary whole-program
    /// type analysis with the mutated signature patched in, so the directive
    /// re-enters the SAME body-vs-signature checker the explicit-annotation
    /// path uses. This turns the `set return string` on `fn answer() { 42 }`
    /// segfault into an ordinary compile error.
    pub(super) fn recheck_directive_mutated_signature(
        &mut self,
        effective_def: &FunctionDef,
    ) -> Result<()> {
        // Record this function's post-directive signature so later re-analyses
        // (for sibling directive-mutated functions) observe it too.
        self.directive_signature_overrides.insert(
            effective_def.name.clone(),
            (
                effective_def.params.clone(),
                effective_def.return_type.clone(),
            ),
        );
        // Only the strict compiler path executes code, so only it can segfault.
        // In RecoverAll (LSP) mode best-effort analysis already ran and nothing
        // executes; re-running here would double-report diagnostics.
        if !matches!(
            self.type_diagnostic_mode,
            crate::compiler::TypeDiagnosticMode::Strict
        ) {
            return Ok(());
        }
        let Some(base_program) = self.directive_reanalysis_program.clone() else {
            return Ok(());
        };
        // Patch every known directive override into a clone of the analyzed
        // program. Only signature fields (return type + per-param type
        // annotation / default value) are patched; bodies are left as analyzed.
        let mut program = base_program;
        let mut patched_any = false;
        for item in &mut program.items {
            if let shape_ast::ast::Item::Function(func, _) = item
                && let Some((params, return_type)) =
                    self.directive_signature_overrides.get(&func.name)
            {
                func.return_type = return_type.clone();
                for (param, patched) in func.params.iter_mut().zip(params.iter()) {
                    param.type_annotation = patched.type_annotation.clone();
                    param.default_value = patched.default_value.clone();
                }
                patched_any = true;
            }
        }
        if !patched_any {
            return Ok(());
        }
        let known_bindings = self.directive_reanalysis_known_bindings.clone();
        let result = shape_runtime::type_system::analyze_program_with_mode_and_comptime_context(
            &program,
            self.source_text.as_deref(),
            None,
            Some(&known_bindings),
            shape_runtime::type_system::TypeAnalysisMode::FailFast,
            self.comptime_mode,
        );
        if let Err(errors) = result {
            return Err(self.directive_signature_type_error(&effective_def.name, errors));
        }
        Ok(())
    }

    /// Wrap the post-directive analysis failure with an attribution that names
    /// the comptime directive as the source of the incompatible signature.
    fn directive_signature_type_error(
        &self,
        func_name: &str,
        errors: Vec<shape_runtime::type_system::TypeErrorWithLocation>,
    ) -> ShapeError {
        let (detail, location) = match Self::type_errors_to_shape(errors) {
            ShapeError::SemanticError { message, location } => (message, location),
            other => (format!("{}", other), None),
        };
        ShapeError::SemanticError {
            message: format!(
                "a comptime directive set a return/parameter type on '{}' that its body does not satisfy: {}",
                func_name, detail
            ),
            location,
        }
    }

    /// Validate that all annotations on a function are allowed for function targets.
    pub(super) fn validate_annotation_targets(&self, func_def: &FunctionDef) -> Result<()> {
        self.check_duplicate_annotations(&func_def.annotations, func_def.name_span)?;
        for ann in &func_def.annotations {
            self.validate_annotation_target_usage(
                ann,
                shape_ast::ast::functions::AnnotationTargetKind::Function,
                func_def.name_span,
            )?;
        }
        Ok(())
    }

    /// Find ALL compiled annotations with before/after handlers on self function.
    /// Returns them in declaration order (first annotation = outermost wrapper).
    pub(super) fn find_compiled_annotations(
        &self,
        func_def: &FunctionDef,
    ) -> Vec<crate::bytecode::CompiledAnnotation> {
        let mut result = Vec::new();
        for ann in &func_def.annotations {
            if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
                if compiled.before_handler.is_some() || compiled.after_handler.is_some() {
                    result.push(compiled.clone());
                }
            }
        }
        result
    }

    /// Compile a function with multiple chained annotations.
    ///
    /// For `@a @b function foo(x) { body }`:
    /// 1. Compile original body as `foo___impl`
    /// 2. Wrap with `@b`: compile wrapper as `foo___b` calling `foo___impl`
    /// 3. Wrap with `@a`: compile wrapper as `foo` calling `foo___b`
    ///
    /// Annotations are applied inside-out: last annotation wraps first.
    pub(super) fn compile_chained_annotations(
        &mut self,
        func_def: &FunctionDef,
        annotations: Vec<crate::bytecode::CompiledAnnotation>,
    ) -> Result<()> {
        // Step 1: Compile the raw function body as {name}___impl
        let impl_name = format!("{}___impl", func_def.name);
        let impl_def = FunctionDef {
            name: impl_name.clone(),
            name_span: func_def.name_span,
            declaring_module_path: func_def.declaring_module_path.clone(),
            doc_comment: None,
            params: func_def.params.clone(),
            return_type: func_def.return_type.clone(),
            body: func_def.body.clone(),
            type_params: func_def.type_params.clone(),
            annotations: Vec::new(),
            where_clause: None,
            is_async: func_def.is_async,
            is_comptime: func_def.is_comptime,
        };
        self.register_function(&impl_def)?;
        self.compile_function_body(&impl_def)?;

        let mut current_impl_idx =
            self.find_function(&impl_name)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!("Impl function '{}' not found after compilation", impl_name),
                    location: None,
                })? as u16;

        // Step 2: Apply annotations inside-out (last annotation wraps first)
        // For @a @b @c: wrap order is c(impl) -> b(c_wrapper) -> a(b_wrapper)
        let reversed: Vec<_> = annotations.into_iter().rev().collect();
        let total = reversed.len();

        for (i, ann) in reversed.into_iter().enumerate() {
            let is_last = i == total - 1;
            let wrapper_name = if is_last {
                // The outermost annotation gets the original function name
                func_def.name.clone()
            } else {
                // Intermediate wrappers get unique names
                format!("{}___{}", func_def.name, ann.name)
            };

            // Find the annotation arg expressions from the original function def
            let ann_arg_exprs =
                self.annotation_args_for_compiled_name(&func_def.annotations, &ann.name);

            // Register the intermediate wrapper function (outermost already registered)
            let wrapper_func_idx = if is_last {
                self.find_function(&func_def.name)
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: format!("Function '{}' not found", func_def.name),
                        location: None,
                    })?
            } else {
                // Create a placeholder function entry for the intermediate wrapper
                let wrapper_def = FunctionDef {
                    name: wrapper_name.clone(),
                    name_span: func_def.name_span,
                    declaring_module_path: func_def.declaring_module_path.clone(),
                    doc_comment: None,
                    params: func_def.params.clone(),
                    return_type: func_def.return_type.clone(),
                    body: Vec::new(), // placeholder
                    type_params: func_def.type_params.clone(),
                    annotations: Vec::new(),
                    is_async: func_def.is_async,
                    is_comptime: func_def.is_comptime,
                    where_clause: None,
                };
                self.register_function(&wrapper_def)?;
                self.find_function(&wrapper_name)
                    .expect("function was just registered")
            };

            // Compile the wrapper that wraps current_impl_idx with self annotation
            self.compile_annotation_wrapper(
                func_def,
                wrapper_func_idx,
                current_impl_idx,
                &ann,
                &ann_arg_exprs,
            )?;

            current_impl_idx = wrapper_func_idx as u16;
        }

        Ok(())
    }

    /// Compile a function that has a single before/after annotation hook.
    ///
    /// 1. Compile original body as `{name}___impl`
    /// 2. Compile a wrapper under the original name that calls before/impl/after
    pub(super) fn compile_wrapped_function(
        &mut self,
        func_def: &FunctionDef,
        compiled_ann: crate::bytecode::CompiledAnnotation,
    ) -> Result<()> {
        // Find the annotation on the function to get the arg expressions
        let ann = func_def
            .annotations
            .iter()
            .find(|a| self.annotation_matches_compiled_name(a, &compiled_ann.name))
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Annotation '{}' not found on function", compiled_ann.name),
                location: None,
            })?;
        let ann_arg_exprs = ann.args.clone();

        // Step 1: Compile original body as {name}___impl
        let impl_name = format!("{}___impl", func_def.name);
        let impl_def = FunctionDef {
            name: impl_name.clone(),
            name_span: func_def.name_span,
            declaring_module_path: func_def.declaring_module_path.clone(),
            doc_comment: None,
            params: func_def.params.clone(),
            return_type: func_def.return_type.clone(),
            body: func_def.body.clone(),
            type_params: func_def.type_params.clone(),
            annotations: Vec::new(),
            where_clause: None,
            is_async: func_def.is_async,
            is_comptime: func_def.is_comptime,
        };
        self.register_function(&impl_def)?;
        self.compile_function_body(&impl_def)?;

        let impl_idx = self
            .find_function(&impl_name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Impl function '{}' not found after compilation", impl_name),
                location: None,
            })? as u16;

        // Step 2: Compile the wrapper
        let func_idx =
            self.find_function(&func_def.name)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!("Function '{}' not found", func_def.name),
                    location: None,
                })?;

        self.compile_annotation_wrapper(func_def, func_idx, impl_idx, &compiled_ann, &ann_arg_exprs)
    }

    /// §4.1.5 runtime-hook `ctx` type. Field order MUST match the ctx object
    /// schema built in `compile_annotation_wrapper` (`target`, `state`,
    /// `event_log`) so typed field access resolves the right offsets. `target`
    /// is typed as a function value carrying the annotated function's exact
    /// signature, so `ctx.target(...)` is an ordinary typed call and passing
    /// `ctx.target` to a builtin carries its type.
    fn annotation_ctx_type_annotation(&self, func_def: &FunctionDef) -> TypeAnnotation {
        let target_params: Vec<shape_ast::ast::types::FunctionParam> = func_def
            .params
            .iter()
            .map(|p| shape_ast::ast::types::FunctionParam {
                name: p.simple_name().map(|s| s.to_string()),
                optional: false,
                type_annotation: p
                    .type_annotation
                    .clone()
                    .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
            })
            .collect();
        let target_returns = func_def.return_type.clone().unwrap_or(TypeAnnotation::Void);
        TypeAnnotation::Object(vec![
            shape_ast::ast::ObjectTypeField {
                name: "target".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Function {
                    params: target_params,
                    returns: Box::new(target_returns),
                },
                annotations: vec![],
            },
            shape_ast::ast::ObjectTypeField {
                name: "state".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("unknown".to_string()),
                annotations: vec![],
            },
            shape_ast::ast::ObjectTypeField {
                name: "event_log".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                    "unknown".to_string(),
                ))),
                annotations: vec![],
            },
        ])
    }

    fn annotation_result_type_annotation(&self, func_def: &FunctionDef) -> TypeAnnotation {
        if let Some(annotation) = func_def.return_type.as_ref() {
            if !Self::annotation_type_is_unknown(annotation) {
                return annotation.clone();
            }
        }

        if let Some(shape_runtime::type_system::Type::Function { returns, .. }) =
            self.inference_facts.function_signature(&func_def.name)
            && let Some(annotation) = returns.to_annotation()
            && !Self::annotation_type_is_unknown(&annotation)
        {
            return annotation;
        }

        TypeAnnotation::Void
    }

    fn annotation_literal_type_annotation(literal: &Literal) -> Option<TypeAnnotation> {
        let name = match literal {
            Literal::Int(_) => "int",
            Literal::UInt(_) => "u64",
            Literal::TypedInt(_, width) => match width {
                shape_ast::IntWidth::I8 => "i8",
                shape_ast::IntWidth::U8 => "u8",
                shape_ast::IntWidth::I16 => "i16",
                shape_ast::IntWidth::U16 => "u16",
                shape_ast::IntWidth::I32 => "i32",
                shape_ast::IntWidth::U32 => "u32",
                shape_ast::IntWidth::U64 => "u64",
            },
            Literal::Number(_) => "number",
            Literal::Decimal(_) => "decimal",
            Literal::String(_) | Literal::FormattedString { .. } => "string",
            Literal::Char(_) => "char",
            Literal::Bool(_) => "bool",
            Literal::None => "null",
            Literal::Unit => "void",
            Literal::Timeframe(_) => "timeframe",
        };
        Some(TypeAnnotation::Basic(name.to_string()))
    }

    fn annotation_expr_type_annotation(&mut self, expr: &Expr) -> Option<TypeAnnotation> {
        if let Expr::Literal(literal, _) = expr {
            return Self::annotation_literal_type_annotation(literal);
        }

        if let Ok(inferred) = self.infer_expr_type(expr)
            && let Some(annotation) = inferred.to_annotation()
            && !Self::annotation_type_is_unknown(&annotation)
        {
            return Some(annotation);
        }

        let concrete =
            crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(self, expr)?;
        let annotation =
            crate::compiler::expressions::closures::concrete_type_to_type_annotation(&concrete)?;
        (!Self::annotation_type_is_unknown(&annotation)).then_some(annotation)
    }

    fn simple_annotation_parameter(
        name: String,
        type_annotation: Option<TypeAnnotation>,
    ) -> shape_ast::ast::FunctionParameter {
        shape_ast::ast::FunctionParameter {
            pattern: DestructurePattern::Identifier(name, Span::DUMMY),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation,
            default_value: None,
        }
    }

    fn specialized_annotation_handler_name(
        annotation_name: &str,
        wrapper_func_idx: usize,
        handler_type: shape_ast::ast::AnnotationHandlerType,
    ) -> String {
        let sanitized: String = annotation_name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let suffix = match handler_type {
            shape_ast::ast::AnnotationHandlerType::Before => "before",
            shape_ast::ast::AnnotationHandlerType::After => "after",
            _ => "handler",
        };
        format!(
            "__ann_{}_{}_wrapper_{}",
            sanitized, suffix, wrapper_func_idx
        )
    }

    fn compile_specialized_annotation_handler(
        &mut self,
        func_def: &FunctionDef,
        wrapper_func_idx: usize,
        compiled_ann: &crate::bytecode::CompiledAnnotation,
        handler: &shape_ast::ast::AnnotationHandler,
        ann_arg_exprs: &[shape_ast::ast::Expr],
    ) -> Result<u16> {
        let mut params = vec![Self::simple_annotation_parameter(
            "self".to_string(),
            Some(TypeAnnotation::Basic("number".to_string())),
        )];

        let ann_param_count = compiled_ann.param_defs.len().max(ann_arg_exprs.len());
        for idx in 0..ann_param_count {
            let mut param = compiled_ann
                .param_defs
                .get(idx)
                .cloned()
                .unwrap_or_else(|| {
                    let name = compiled_ann
                        .param_names
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| format!("__ann_arg_{}", idx));
                    Self::simple_annotation_parameter(name, None)
                });
            if param.type_annotation.is_none()
                && let Some(expr) = ann_arg_exprs.get(idx)
            {
                param.type_annotation = self.annotation_expr_type_annotation(expr);
            }
            params.push(param);
        }

        let args_annotation = TypeAnnotation::Array(Box::new(
            self.annotation_arg_array_element_annotation(func_def)?,
        ));
        let result_annotation = self.annotation_result_type_annotation(func_def);
        let ctx_annotation = self.annotation_ctx_type_annotation(func_def);

        for handler_param in &handler.params {
            let type_annotation = match handler_param.name.as_str() {
                "args" => Some(args_annotation.clone()),
                "result" => Some(result_annotation.clone()),
                "ctx" => Some(ctx_annotation.clone()),
                _ => None,
            };
            params.push(Self::simple_annotation_parameter(
                handler_param.name.clone(),
                type_annotation,
            ));
        }

        let func_name = Self::specialized_annotation_handler_name(
            &compiled_ann.name,
            wrapper_func_idx,
            handler.handler_type.clone(),
        );
        let declaring_module_path = compiled_ann
            .name
            .rsplit_once("::")
            .map(|(module_path, _)| module_path.to_string());
        let func_def = FunctionDef {
            name: func_name.clone(),
            // ADR-009 D1 (S3): the wrapper IS the compiled handler body, so
            // it anchors at the handler definition — the generator's real
            // span — never Span::DUMMY (Decision 68).
            name_span: handler.span,
            declaring_module_path: declaring_module_path.clone(),
            doc_comment: None,
            params,
            return_type: handler.return_type.clone(),
            body: vec![Statement::Return(
                Some(handler.body.clone()),
                handler.body.span(),
            )],
            type_params: Some(Vec::new()),
            annotations: Vec::new(),
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        self.register_function(&func_def)?;
        let func_idx = self
            .find_function(&func_name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Internal error: specialized annotation handler '{}' was not registered",
                    func_name
                ),
                location: None,
            })?;
        if let Some(module_path) = declaring_module_path {
            self.module_scope_stack.push(module_path);
            let result = self.compile_function(&func_def);
            self.module_scope_stack.pop();
            result?;
        } else {
            self.compile_function(&func_def)?;
        }
        Ok(func_idx as u16)
    }

    fn specialize_annotation_runtime_handlers(
        &mut self,
        func_def: &FunctionDef,
        wrapper_func_idx: usize,
        compiled_ann: &crate::bytecode::CompiledAnnotation,
        ann_arg_exprs: &[shape_ast::ast::Expr],
    ) -> Result<crate::bytecode::CompiledAnnotation> {
        let mut specialized = compiled_ann.clone();

        if let Some(handler) = compiled_ann.before_handler_template.clone() {
            specialized.before_handler = Some(self.compile_specialized_annotation_handler(
                func_def,
                wrapper_func_idx,
                compiled_ann,
                &handler,
                ann_arg_exprs,
            )?);
        }

        if let Some(handler) = compiled_ann.after_handler_template.clone() {
            specialized.after_handler = Some(self.compile_specialized_annotation_handler(
                func_def,
                wrapper_func_idx,
                compiled_ann,
                &handler,
                ann_arg_exprs,
            )?);
        }

        Ok(specialized)
    }

    /// Core annotation wrapper compilation.
    ///
    /// Emits bytecode for a wrapper function at `wrapper_func_idx` that:
    /// - Builds args array from function params
    /// - Calls before(self, ...ann_params, args, ctx) if present
    /// - Calls the impl function at `impl_idx` with (possibly modified) args
    /// - Calls after(self, ...ann_params, args, result, ctx) if present
    /// - Returns result
    pub(super) fn compile_annotation_wrapper(
        &mut self,
        func_def: &FunctionDef,
        wrapper_func_idx: usize,
        impl_idx: u16,
        compiled_ann: &crate::bytecode::CompiledAnnotation,
        ann_arg_exprs: &[shape_ast::ast::Expr],
    ) -> Result<()> {
        let runtime_ann = self.specialize_annotation_runtime_handlers(
            func_def,
            wrapper_func_idx,
            compiled_ann,
            ann_arg_exprs,
        )?;
        let compiled_ann = &runtime_ann;

        let jump_over = if self.current_function.is_none() {
            Some(self.emit_jump(OpCode::Jump, 0))
        } else {
            None
        };

        let saved_function = self.current_function;
        let saved_next_local = self.next_local;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_is_async = self.current_function_is_async;

        self.current_function = Some(wrapper_func_idx);
        self.current_function_is_async = func_def.is_async;
        self.locals = vec![HashMap::new()];
        self.type_tracker.clear_locals();
        self.push_scope();
        self.next_local = 0;

        self.program.functions[wrapper_func_idx].entry_point = self.program.current_offset();

        // Start blob builder for this wrapper function.
        let saved_blob_builder = self.current_blob_builder.take();
        let wrapper_blob_name = self.program.functions[wrapper_func_idx].name.clone();
        self.current_blob_builder = Some(super::FunctionBlobBuilder::new(
            wrapper_blob_name,
            self.program.current_offset(),
            self.program.constants.len(),
            self.program.strings.len(),
        ));

        // Bind original function params as locals
        for param in &func_def.params {
            for name in param.get_identifiers() {
                self.declare_local(&name)?;
            }
        }

        // Declare locals for wrapper internal state
        let args_local = self.declare_local("__args")?;
        let result_local = self.declare_local("__result")?;
        let ctx_local = self.declare_local("__ctx")?;

        // --- Build args array from function params ---
        // The wrapper function may have ref-inferred params (inherited from
        // the original function definition). Callers emit MakeRef for those
        // params, so local slots contain TAG_REF values. We must DerefLoad
        // to get the actual values before putting them in the args array.
        let wrapper_ref_params = self.program.functions[wrapper_func_idx].ref_params.clone();
        self.emit_annotation_args_array(func_def, &wrapper_ref_params, impl_idx)?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(args_local)),
        ));

        // --- Build ctx object: { target: Function, state: {}, event_log: [] } ---
        // Push fields in schema order: target, state, event_log.
        // §4.1.5: `ctx.target` is a typed function value statically bound to
        // the annotated function's ORIGINAL implementation (the same referent
        // as `__original__`). A runtime `before`/`after` hook reads it as an
        // ordinary typed field and may call it or pass it on (WF-2C's `@remote`
        // hard-depends on exactly this — no stringly `ctx["__impl"]` lookup).
        let impl_ref_const = self
            .program
            .add_constant(Constant::Function(impl_idx as u16));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(impl_ref_const)),
        ));
        // W17.2-C §4.D.5 migration: empty-fields case uses the typed
        // variant directly.
        let empty_schema_id = self.type_tracker.register_inline_object_schema_typed(&[]);
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: empty_schema_id as u16,
                field_count: 0,
            }),
        ));

        self.emit_empty_annotation_event_log();

        let ctx_schema_id = self.type_tracker.register_inline_object_schema_typed(&[
            ("target", FieldType::Any),
            ("state", FieldType::Any),
            ("event_log", FieldType::Array(Box::new(FieldType::Any))),
        ]);
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: ctx_schema_id as u16,
                field_count: 3,
            }),
        ));
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(ctx_local)),
        ));

        // --- Call before handler if present ---
        let mut short_circuit_jump: Option<usize> = None;
        if let Some(before_id) = compiled_ann.before_handler {
            let fn_ref = self
                .program
                .add_constant(Constant::Number(wrapper_func_idx as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(fn_ref)),
            ));

            for ann_arg in ann_arg_exprs {
                self.compile_expr(ann_arg)?;
            }

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(args_local)),
            ));
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(ctx_local)),
            ));

            let before_arg_count = 1 + ann_arg_exprs.len() + 2;
            let before_ac = self
                .program
                .add_constant(Constant::Int(before_arg_count as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(before_ac)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(before_id))),
            ));
            self.record_blob_call(before_id);

            let before_result = self.declare_local("__before_result")?;
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(before_result)),
            ));

            // Check if before_result is an array → replace args
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            let one_const = self.program.add_constant(Constant::Int(1));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(one_const)),
            ));
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(crate::bytecode::BuiltinFunction::IsArray)),
            ));

            let skip_array = self.emit_jump(OpCode::JumpIfFalse, 0);

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(args_local)),
            ));
            let skip_obj_check = self.emit_jump(OpCode::Jump, 0);

            self.patch_jump(skip_array);

            // Check if before_result is an object → extract "args" and "state"
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            let one_const2 = self.program.add_constant(Constant::Int(1));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(one_const2)),
            ));
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(crate::bytecode::BuiltinFunction::IsObject)),
            ));

            let skip_obj = self.emit_jump(OpCode::JumpIfFalse, 0);

            // Strict contract: before-handler object form uses typed fields
            // {args, result, state}. The `result` field enables short-circuit:
            // if the before handler returns { result: value }, skip the impl call.
            let before_contract_schema_id =
                self.type_tracker.register_inline_object_schema_typed(&[
                    ("args", FieldType::Any),
                    ("result", FieldType::Any),
                    ("state", FieldType::Any),
                ]);
            if before_contract_schema_id > u16::MAX as u32 {
                return Err(ShapeError::RuntimeError {
                    message: "Internal error: before-handler schema id overflow".to_string(),
                    location: None,
                });
            }
            let (args_operand, state_operand, result_operand) = {
                let schema = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(before_contract_schema_id)
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: "Internal error: missing before-handler schema".to_string(),
                        location: None,
                    })?;
                let args_field =
                    schema
                        .get_field("args")
                        .ok_or_else(|| ShapeError::RuntimeError {
                            message: "Internal error: before-handler schema missing 'args'"
                                .to_string(),
                            location: None,
                        })?;
                let state_field =
                    schema
                        .get_field("state")
                        .ok_or_else(|| ShapeError::RuntimeError {
                            message: "Internal error: before-handler schema missing 'state'"
                                .to_string(),
                            location: None,
                        })?;
                let result_field =
                    schema
                        .get_field("result")
                        .ok_or_else(|| ShapeError::RuntimeError {
                            message: "Internal error: before-handler schema missing 'result'"
                                .to_string(),
                            location: None,
                        })?;
                if args_field.offset > u16::MAX as usize
                    || state_field.offset > u16::MAX as usize
                    || result_field.offset > u16::MAX as usize
                {
                    return Err(ShapeError::RuntimeError {
                        message: "Internal error: before-handler field offset/index overflow"
                            .to_string(),
                        location: None,
                    });
                }
                (
                    Operand::TypedField {
                        type_id: before_contract_schema_id as u16,
                        field_idx: args_field.index as u16,
                        field_type_tag: field_type_to_tag(&args_field.field_type),
                    },
                    Operand::TypedField {
                        type_id: before_contract_schema_id as u16,
                        field_idx: state_field.index as u16,
                        field_type_tag: field_type_to_tag(&state_field.field_type),
                    },
                    Operand::TypedField {
                        type_id: before_contract_schema_id as u16,
                        field_idx: result_field.index as u16,
                        field_type_tag: field_type_to_tag(&result_field.field_type),
                    },
                )
            };

            // Check `result` field for short-circuit: if non-null, skip impl call
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            self.emit(Instruction::new(
                OpCode::GetFieldTyped,
                Some(result_operand),
            ));
            // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::IsNull));
            let skip_short_circuit = self.emit_jump(OpCode::JumpIfTrue, 0);
            // result is non-null → store it and jump past impl call
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(result_local)),
            ));
            short_circuit_jump = Some(self.emit_jump(OpCode::Jump, 0));
            self.patch_jump(skip_short_circuit);
            self.emit(Instruction::simple(OpCode::Pop)); // discard null result

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(args_operand)));
            // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::IsNull));
            let skip_args_replace = self.emit_jump(OpCode::JumpIfTrue, 0);
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(args_local)),
            ));
            let skip_pop_args = self.emit_jump(OpCode::Jump, 0);
            self.patch_jump(skip_args_replace);
            self.emit(Instruction::simple(OpCode::Pop));
            self.patch_jump(skip_pop_args);

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(before_result)),
            ));
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(state_operand)));
            // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::IsNull));
            let skip_state = self.emit_jump(OpCode::JumpIfTrue, 0);
            self.emit_empty_annotation_event_log();
            self.emit(Instruction::new(
                OpCode::NewTypedObject,
                Some(Operand::TypedObjectAlloc {
                    schema_id: ctx_schema_id as u16,
                    field_count: 2,
                }),
            ));
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(ctx_local)),
            ));
            let skip_pop_state = self.emit_jump(OpCode::Jump, 0);
            self.patch_jump(skip_state);
            self.emit(Instruction::simple(OpCode::Pop));
            self.patch_jump(skip_pop_state);

            self.patch_jump(skip_obj);
            self.patch_jump(skip_obj_check);
        }

        // --- Call impl function with (possibly modified) args ---
        // The impl function may have ref-inferred parameters (borrow inference
        // marks unannotated heap-like params as references). We must wrap those
        // args with MakeRef so the impl's DerefLoad/DerefStore opcodes find
        // TAG_REF values in the local slots.
        let impl_ref_params = self.program.functions[impl_idx as usize].ref_params.clone();
        for i in 0..func_def.params.len() {
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(args_local)),
            ));
            let idx_const = self.program.add_constant(Constant::Number(i as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(idx_const)),
            ));
            self.emit(Instruction::simple(OpCode::GetProp));
            if impl_ref_params.get(i).copied().unwrap_or(false) {
                let temp = self.declare_temp_local("__ref_wrap_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                self.emit(Instruction::new(
                    OpCode::MakeRef,
                    Some(Operand::Local(temp)),
                ));
            }
        }
        let impl_ac = self
            .program
            .add_constant(Constant::Int(func_def.params.len() as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(impl_ac)),
        ));
        self.emit(Instruction::new(
            OpCode::Call,
            Some(Operand::Function(shape_value::FunctionId(impl_idx))),
        ));
        self.record_blob_call(impl_idx);

        // For void functions, the impl returns null (the implicit return sentinel).
        // The after handler's `result` parameter would then trip the "missing
        // required argument guard" because null is the sentinel for "parameter not
        // provided". Replace null with Unit so the guard doesn't fire.
        // We only do this for explicitly void functions (return_type: Void) to avoid
        // clobbering valid return values from functions with unspecified return types.
        if compiled_ann.after_handler.is_some() {
            let is_explicit_void = matches!(
                func_def.return_type,
                Some(shape_ast::ast::TypeAnnotation::Void)
            );
            if is_explicit_void {
                // Void function: always replace null with Unit
                self.emit(Instruction::simple(OpCode::Pop));
                self.emit_unit();
            } else if func_def.return_type.is_none() {
                // Unspecified return type: replace null with Unit at runtime
                // (if the function actually returned a value, it won't be null).
                // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::simple(OpCode::IsNull));
                let skip_replace = self.emit_jump(OpCode::JumpIfFalse, 0);
                // Replace the null on stack with Unit
                self.emit(Instruction::simple(OpCode::Pop));
                self.emit_unit();
                self.patch_jump(skip_replace);
            }
        }

        // Store result
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(result_local)),
        ));

        // Patch short-circuit jump: lands here, after impl call + result store
        if let Some(jump_addr) = short_circuit_jump {
            self.patch_jump(jump_addr);
        }

        // --- Call after handler if present ---
        if let Some(after_id) = compiled_ann.after_handler {
            let fn_ref = self
                .program
                .add_constant(Constant::Number(wrapper_func_idx as f64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(fn_ref)),
            ));

            for ann_arg in ann_arg_exprs {
                self.compile_expr(ann_arg)?;
            }

            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(args_local)),
            ));
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(result_local)),
            ));
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(ctx_local)),
            ));

            let after_arg_count = 1 + ann_arg_exprs.len() + 3;
            let after_ac = self
                .program
                .add_constant(Constant::Int(after_arg_count as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(after_ac)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(after_id))),
            ));
            self.record_blob_call(after_id);

            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(result_local)),
            ));
        }

        // Return the result
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(result_local)),
        ));
        self.emit(Instruction::simple(OpCode::ReturnValue));

        // Update function locals count
        self.program.functions[wrapper_func_idx].locals_count = self.next_local;
        self.capture_function_local_storage_hints(wrapper_func_idx);

        // Finalize blob and restore the parent blob builder.
        self.finalize_current_blob(wrapper_func_idx);
        self.current_blob_builder = saved_blob_builder;

        // Restore state
        self.pop_scope();
        self.locals = saved_locals;
        self.current_function = saved_function;
        self.current_function_is_async = saved_is_async;
        self.next_local = saved_next_local;

        if let Some(jump_addr) = jump_over {
            self.patch_jump(jump_addr);
        }

        Ok(())
    }
}

// ADR-009 §4.1 (ticket A1, slice S3) — the annotation-handler freeze gate.
//
// Pre-pass freeze rule (S3, named rule per plan graft 4): the speculative
// annotation pre-passes (`materialize_computed_comptime_extends` and
// `apply_function_comptime_signature_directives_for_analysis`) consume the
// SAME registration-complete freeze handle as the authoritative pass-2
// execution — the freeze barrier runs BEFORE them in `compile()`. A pre-pass
// comptime site that cannot obtain the handle is the row-3 named compile
// error (`NO_FREEZE_HANDLE_DIAGNOSTIC`); exemption-by-suppression, empty
// snapshots and `Option<freeze>` are forbidden shapes. Dec 52 ordering: a
// freeze-boundary rejection fires at the barrier, BEFORE any handler body
// executes.
#[cfg(test)]
mod s3_freeze_gate_tests {
    use super::BytecodeCompiler;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    /// Rejection-matrix row 3, type-target pre-pass: running the speculative
    /// extends pre-pass on a compiler whose freeze barrier has not run is a
    /// compile error with the named diagnostic — the pre-pass consumes the
    /// real handle, it does not fall back to a reflection-rejecting module.
    #[test]
    fn extends_prepass_without_freeze_handle_is_the_named_row3_compile_error() {
        let program = parse(
            r#"
annotation touch() {
  targets: [type]
  comptime post(target, ctx) {
    1
  }
}

@touch()
type Probe { id: int }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        let error = compiler
            .materialize_computed_comptime_extends(&program)
            .expect_err("pre-barrier pre-pass site must be a compile error");
        assert!(
            error.to_string().contains("no semantic freeze handle"),
            "row-3 named diagnostic missing: {error}"
        );
    }

    /// Rejection-matrix row 3, function-target pre-pass (signature
    /// directives): same gate, same named diagnostic.
    #[test]
    fn signature_directive_prepass_without_freeze_handle_is_the_named_row3_compile_error() {
        let mut program = parse(
            r#"
annotation touch() {
  targets: [function]
  comptime post(target, ctx) {
    1
  }
}

@touch()
fn probe() -> int { 2 }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        let error = compiler
            .apply_function_comptime_signature_directives_for_analysis(&mut program)
            .expect_err("pre-barrier pre-pass site must be a compile error");
        assert!(
            error.to_string().contains("no semantic freeze handle"),
            "row-3 named diagnostic missing: {error}"
        );
    }

    /// Dec 52 ordering proof (rejection-matrix row 4): a freeze-boundary
    /// rejection fires at the barrier, BEFORE any annotation handler body
    /// executes. The handler here would leave two observable side effects
    /// (a comptime warning and a hard `error()`); the compile error must be
    /// the freeze rejection and neither side effect may be observed.
    #[test]
    fn freeze_rejection_fires_before_annotation_handler_body_executes() {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::{TypeVar, tyvar_to_annotation};

        // Clear any diagnostics left by other tests on this thread.
        let _ = crate::compiler::comptime_builtins::take_comptime_diagnostics();

        // Poison the unit with partial semantic state: a struct field whose
        // annotation still carries an unresolved inference variable.
        let mut compiler = BytecodeCompiler::new();
        compiler.struct_types.insert(
            "Poisoned".to_string(),
            (vec!["min".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "Poisoned".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [(
                    "min".to_string(),
                    tyvar_to_annotation(&TypeVar("T3".to_string())),
                )]
                .into_iter()
                .collect::<std::collections::HashMap<String, TypeAnnotation>>(),
            },
        );

        let program = parse(
            r#"
annotation marker() {
  targets: [type]
  comptime post(target, ctx) {
    warning("SIDE_EFFECT")
    error("HANDLER_RAN")
  }
}

@marker()
type Probe { id: int }
"#,
        );

        let error = compiler
            .compile(&program)
            .expect_err("partial semantic state must reject compilation at the barrier");
        let message = error.to_string();
        assert!(
            message.contains("unresolved inference variable"),
            "the compile error must be the named freeze rejection, got: {message}"
        );
        assert!(
            !message.contains("HANDLER_RAN"),
            "Dec 52 violated: the handler body executed before the freeze \
             rejection fired: {message}"
        );
        let diagnostics = crate::compiler::comptime_builtins::take_comptime_diagnostics();
        assert!(
            diagnostics
                .iter()
                .all(|d| !d.message.contains("SIDE_EFFECT")),
            "Dec 52 violated: handler side effect observed: {diagnostics:?}"
        );
    }
}

// ADR-009 ticket D1 (slice S2) — provenance stamping on the existing
// two-phase extend path + identity-keyed dedup.
//
// Decision 68 / Decision 67 invariant 5: every generated declaration is
// reserved in the compiler's `GeneratedSymbolTable` under a content-derived
// `SymbolId` with full `ExpansionIdentity` + `GeneratedOrigin`. The
// speculative pre-pass (`materialize_computed_comptime_extends`) and the
// authoritative pass-2 compile (`apply_comptime_extend` /
// `apply_comptime_extend_items`) are the SAME application identity — one
// record, never two, never a doubled diagnostic. Dedup is keyed on the
// expansion identity; name lookups are a derived view into the table.
#[cfg(test)]
mod s2_expansion_provenance_tests {
    use super::BytecodeCompiler;
    use crate::compiler::comptime_builtins::expansion_provenance::{
        ApplicationId, CanonicalHash, ComptimeStage, ExpansionIdentity, ExpansionSite,
        GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC, GENERATED_SYMBOL_CONFLICT_DIAGNOSTIC,
        GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC, GeneratorRef, TargetIdentity,
    };
    use shape_ast::ast::Span;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    fn first_extend(program: &shape_ast::ast::Program) -> shape_ast::ast::ExtendStatement {
        program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend.clone()),
                _ => None,
            })
            .expect("program contains an extend item")
    }

    /// A hand-built expansion site for driving the pass-2 registration entry
    /// point directly (the real enforcement point for rows 1 and 3).
    fn test_site(application_span: Span) -> ExpansionSite {
        let no_args: [(&str, &str); 0] = [];
        ExpansionSite::new(
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor("annotation:test_gen:comptime-post"),
                ApplicationId::from_canonical_descriptor("application:test:10:20"),
                TargetIdentity::from_canonical_descriptor("type:UserRow"),
                ComptimeStage::AnnotationHandler,
                CanonicalHash::from_canonical_argument_descriptors(&no_args),
                CanonicalHash::from_canonical_dependency_descriptors(&[]),
            ),
            0,
            application_span,
            // Generator-definition span: a distinct real span so S4 tests
            // can tell the generator anchor apart from the application.
            Span::new(2, 8),
        )
    }

    /// Risk-7 agreement proof, extend-method shape: the generated method is
    /// registered by the speculative pre-pass and re-seen by the
    /// authoritative pass-2 compile under the SAME `ExpansionIdentity` —
    /// exactly ONE record in the generated-symbol table (a disagreement
    /// would either double the table or trip the row-2 conflict error and
    /// fail compilation).
    #[test]
    fn prepass_and_pass2_agree_on_one_expansion_identity_for_generated_extend_method() {
        let program = parse(
            r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ 42 \} \}")
  }
}

@gen()
type Point { id: int }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&program)
            .expect("generated extend method compiles through both phases");
        assert_eq!(
            compiler.generated_symbols.len(),
            1,
            "pre-pass and pass-2 must agree on ONE identity for one generated decl"
        );
        let id = compiler
            .generated_symbols
            .symbol_for_name("Point.answer")
            .expect("generated method resolves through the derived name view");
        let origin = compiler
            .generated_symbols
            .origin_of(id)
            .expect("reserved identity has full provenance");
        assert!(
            origin
                .expansion
                .target
                .canonical_descriptor()
                .contains("Point"),
            "target identity must name the annotated type, got {:?}",
            origin.expansion.target
        );
        assert!(
            origin
                .expansion
                .generator
                .canonical_descriptor()
                .contains("gen"),
            "generator identity must name the annotation, got {:?}",
            origin.expansion.generator
        );
        assert_ne!(
            origin.source_anchor.span(),
            Span::DUMMY,
            "generated decls anchor at the real application span, never DUMMY"
        );
    }

    /// Risk-7 agreement proof, free-function shape (the §4.5.1 pre-pass
    /// visibility case): `fn main` resolves the generated free function AND
    /// the table holds exactly one record for it after pass-2 re-runs the
    /// same handler.
    #[test]
    fn prepass_and_pass2_agree_on_one_expansion_identity_for_generated_free_function() {
        let program = parse(
            r#"
annotation gen2() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_flag() -> int { 7 }")
  }
}

@gen2()
type Point { id: int }

fn main() -> int { generated_flag() }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&program)
            .expect("generated free function compiles through both phases");
        assert_eq!(
            compiler.generated_symbols.len(),
            1,
            "pre-pass and pass-2 must agree on ONE identity for one generated decl"
        );
        compiler
            .generated_symbols
            .symbol_for_name("generated_flag")
            .expect("generated free function resolves through the derived name view");
    }

    /// Rejection-matrix row 2: a second, conflicting definition for one
    /// generated symbol name — two DIFFERENT applications each generating
    /// `fn clash()` — is the named compile error carrying expansion
    /// provenance, not a silent first-wins dedup.
    #[test]
    fn conflicting_generated_name_across_applications_is_the_named_row2_compile_error() {
        let program = parse(
            r#"
annotation dup() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn clash() -> int { 1 }")
  }
}

@dup()
type A { id: int }

@dup()
type B { id: int }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        let error = compiler
            .compile_in_place(&program)
            .expect_err("two applications generating one symbol name must conflict");
        let message = error.to_string();
        assert!(
            message.contains(GENERATED_SYMBOL_CONFLICT_DIAGNOSTIC),
            "row-2 named diagnostic missing: {message}"
        );
        assert!(
            message.contains("clash"),
            "conflict diagnostic must name the generated symbol: {message}"
        );
        assert!(
            message.contains("annotation:dup"),
            "conflict diagnostic must carry generator provenance: {message}"
        );
    }

    /// Rejection-matrix row 3: the SAME full application identity expanded
    /// twice with CONFLICTING output (same generated method name, different
    /// body) is the named duplicate-identity compile error at the real
    /// registration entry point.
    #[test]
    fn same_application_identity_with_conflicting_output_is_the_named_row3_compile_error() {
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&parse("type UserRow { id: int }"))
            .expect("target type compiles");

        let first = first_extend(&parse("extend UserRow { method row() -> int { 1 } }"));
        let second = first_extend(&parse("extend UserRow { method row() -> int { 2 } }"));
        let site = test_site(Span::new(10, 20));

        compiler
            .apply_comptime_extend(first, "UserRow", &site)
            .expect("first expansion of the identity reserves and compiles");
        let error = compiler
            .apply_comptime_extend(second, "UserRow", &site)
            .expect_err("conflicting output for one reserved identity must be refused");
        let message = error.to_string();
        assert!(
            message.contains(GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC),
            "row-3 named diagnostic missing: {message}"
        );
    }

    /// Rejection-matrix row 1 (Dec 68 required rejection): a generated
    /// declaration whose application anchor is `Span::DUMMY` — the named
    /// `UserRow` + dummy-span node — is refused at the registration entry
    /// point with the named diagnostic, BEFORE any registration or compile.
    #[test]
    fn generated_decl_anchored_at_dummy_span_is_the_named_row1_compile_error() {
        let mut compiler = BytecodeCompiler::new();
        let extend = first_extend(&parse("extend UserRow { method row() -> int { 1 } }"));
        let site = test_site(Span::DUMMY);

        let error = compiler
            .apply_comptime_extend(extend, "UserRow", &site)
            .expect_err("a dummy-anchored generated decl must be refused");
        let message = error.to_string();
        assert!(
            message.contains(GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC),
            "row-1 named diagnostic missing: {message}"
        );
        assert_eq!(
            compiler.generated_symbols.len(),
            0,
            "nothing may be reserved for an unanchorable generated decl"
        );
    }
}

// ADR-009 ticket D1 (slice S3) — real source anchors on generated
// declarations.
//
// Decision 68: generated text and dummy spans are not semantic
// representations. Every generated declaration the compiler registers must
// carry spans that resolve (via `span_to_source_location`) to a REAL
// location in the compiling file — the annotation-application site for
// expansion-emitted decls, the handler definition for annotation-handler
// wrappers. `Span::DUMMY` numerically equals a legitimate offset-0 span, so
// every assertion here is on the RESOLVED line, never a `{0,0}` comparison.
#[cfg(test)]
mod s3_source_anchor_tests {
    use super::BytecodeCompiler;
    use shape_ast::ast::Span;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    /// Compile `source` with source text installed so spans resolve to real
    /// line/column locations. `compile_in_place` moves `source_text` into
    /// the program's debug info at the end of compilation, so the source is
    /// re-installed afterwards for the test's own resolutions.
    fn compiled_with_source(source: &str) -> BytecodeCompiler {
        let program = parse(source);
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        compiler
            .compile_in_place(&program)
            .expect("test program compiles");
        compiler.set_source(source);
        compiler
    }

    /// 1-indexed line of the first occurrence of `needle` in `source`.
    fn line_of(source: &str, needle: &str) -> usize {
        let offset = source.find(needle).expect("needle present in source");
        source[..offset].chars().filter(|c| *c == '\n').count() + 1
    }

    fn resolved_line(compiler: &BytecodeCompiler, span: Span) -> usize {
        compiler.span_to_source_location(span).line
    }

    /// A generated `extend target { method }` declaration (the DIRECT
    /// handler-AST directive shape) registers with its name span anchored at
    /// the annotation-application site — not `Span::DUMMY` resolving to
    /// line 1 of the wrong text.
    #[test]
    fn generated_extend_target_method_name_span_anchors_at_the_application_site() {
        let source = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

@gen()
type Point { id: int }
"#;
        let compiler = compiled_with_source(source);
        let func_def = compiler
            .function_defs
            .get("Point.answer")
            .expect("generated method is registered");
        let application_line = line_of(source, "@gen()");
        assert_eq!(
            resolved_line(&compiler, func_def.name_span),
            application_line,
            "generated method name span must resolve to the @gen() application line"
        );
    }

    /// A generated method parsed from a SNIPPET (`extend (f"extend …")`) has
    /// snippet-relative spans; the registered declaration must be re-based to
    /// the application span, not left pointing into synthetic snippet text.
    #[test]
    fn generated_snippet_extend_method_name_span_anchors_at_the_application_site() {
        let source = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ 42 \} \}")
  }
}

@gen()
type Point { id: int }
"#;
        let compiler = compiled_with_source(source);
        let func_def = compiler
            .function_defs
            .get("Point.answer")
            .expect("generated method is registered");
        let application_line = line_of(source, "@gen()");
        assert_eq!(
            resolved_line(&compiler, func_def.name_span),
            application_line,
            "snippet-parsed generated method must anchor at the application line"
        );
    }

    /// A generated FREE function parsed from the `mod __module_probe__`
    /// snippet anchors at the application site, and its `GeneratedOrigin`
    /// source anchor resolves to the SAME location — the identity table and
    /// the registered declaration agree on one real anchor.
    #[test]
    fn generated_free_function_anchors_at_the_application_site_not_the_snippet() {
        let source = r#"
annotation gen2() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_flag() -> int { 7 }")
  }
}

@gen2()
type Point { id: int }

fn main() -> int { generated_flag() }
"#;
        let compiler = compiled_with_source(source);
        let application_line = line_of(source, "@gen2()");

        let func_def = compiler
            .function_defs
            .get("generated_flag")
            .expect("generated free function is registered");
        assert_eq!(
            resolved_line(&compiler, func_def.name_span),
            application_line,
            "generated free-function name span must anchor at the application line, \
             not at snippet-relative offsets"
        );

        let id = compiler
            .generated_symbols
            .symbol_for_name("generated_flag")
            .expect("generated free function has an issued SymbolId");
        let origin = compiler
            .generated_symbols
            .origin_of(id)
            .expect("issued SymbolId has full provenance");
        assert_eq!(
            resolved_line(&compiler, origin.source_anchor.span()),
            application_line,
            "GeneratedOrigin source anchor must resolve to the same application line"
        );
    }

    /// `desugar_extend_method` carries the METHOD's own span onto the
    /// desugared FunctionDef (name span + every synthesized type-param
    /// span) — the hand-written extend path's real anchor.
    #[test]
    fn desugared_extend_method_carries_the_method_span_not_dummy() {
        let source = "extend Vec<T> {\n  method always_one() -> int { 1 }\n}";
        let program = parse(source);
        let extend = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend.clone()),
                _ => None,
            })
            .expect("program contains an extend item");
        let method = &extend.methods[0];
        assert_ne!(method.span, Span::DUMMY, "parser anchors the method");

        let compiler = BytecodeCompiler::new();
        let func_def = compiler
            .desugar_extend_method(method, &extend.type_name)
            .expect("method desugars");
        assert_eq!(
            func_def.name_span, method.span,
            "desugared extend method must carry the method's own span"
        );
        let type_params = func_def
            .type_params
            .expect("generic target has type params");
        assert!(!type_params.is_empty(), "extend Vec<T> synthesizes T");
        for tp in &type_params {
            let span = match tp {
                shape_ast::ast::TypeParam::Type { span, .. } => *span,
                shape_ast::ast::TypeParam::Const { span, .. } => *span,
            };
            assert_eq!(
                span,
                method.span,
                "synthesized type param `{}` must anchor at the method span",
                tp.name()
            );
        }
    }

    /// The specialized annotation-handler WRAPPER function (the compiled
    /// `before`/`after` handler body) anchors at the handler definition —
    /// the generator's real span — for both its name span and its
    /// synthesized `Return` statement.
    #[test]
    fn annotation_handler_wrapper_anchors_at_the_handler_definition() {
        let source = r#"
annotation logged() {
  before(args, ctx) {
    args
  }
}

@logged()
fn work(x: int) -> int { x + 1 }

let out = work(2)
"#;
        let compiler = compiled_with_source(source);
        let handler_line = line_of(source, "before(args, ctx)");
        let (name, wrapper) = compiler
            .function_defs
            .iter()
            .find(|(name, _)| name.contains("before_wrapper"))
            .expect("before-handler wrapper is registered");
        assert_eq!(
            resolved_line(&compiler, wrapper.name_span),
            handler_line,
            "wrapper `{name}` name span must resolve to the handler definition line"
        );
        let return_span = wrapper
            .body
            .iter()
            .find_map(|statement| match statement {
                shape_ast::ast::Statement::Return(_, span) => Some(*span),
                _ => None,
            })
            .expect("wrapper body is a synthesized Return");
        assert_eq!(
            resolved_line(&compiler, return_span),
            handler_line,
            "wrapper Return statement must anchor at the handler body"
        );
    }
}

// ADR-009 ticket D1 (slice S4) — the shared compiler query surface for
// generated symbols + provenance-carrying diagnostics.
//
// Decision 66 closing rule: tooling resolves generated declarations through
// COMPILER QUERY RESULTS — {SymbolId, checked-decl location, application
// location, generator-definition location} — answered from the S2 identity
// table only, never by text scan and never by a second expansion run.
// Rejection row 7: a diagnostic raised on a generated declaration carries
// generated-node + application-site + generator-definition locations.
#[cfg(test)]
mod s4_query_surface_and_diagnostics_tests {
    use super::BytecodeCompiler;
    use crate::compiler::comptime_builtins::expansion_provenance::{
        ApplicationId, CanonicalHash, ComptimeStage, ExpansionIdentity, ExpansionSite,
        GeneratorRef, TargetIdentity,
    };
    use shape_ast::ast::Span;
    use shape_ast::error::ShapeError;

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("test program parses")
    }

    /// Compile `source` with source text installed so spans resolve to real
    /// line/column locations (see the S3 test-mod helper: `compile_in_place`
    /// moves `source_text` into debug info, so it is re-installed for the
    /// test's own resolutions).
    fn compiled_with_source(source: &str) -> BytecodeCompiler {
        let program = parse(source);
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        compiler
            .compile_in_place(&program)
            .expect("test program compiles");
        compiler.set_source(source);
        compiler
    }

    /// 1-indexed line of the first occurrence of `needle` in `source`.
    fn line_of(source: &str, needle: &str) -> usize {
        let offset = source.find(needle).expect("needle present in source");
        source[..offset].chars().filter(|c| *c == '\n').count() + 1
    }

    fn resolved_line(compiler: &BytecodeCompiler, span: Span) -> usize {
        compiler.span_to_source_location(span).line
    }

    const GENERATED_METHOD_FIXTURE: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
      method double() -> int { 84 }
    }
  }
}

@gen()
type Point { id: int }
"#;

    /// The query surface resolves a generated declaration NAME to its full
    /// provenance: SymbolId + checked-decl + application + generator
    /// locations, each resolving to the right REAL source line — answered
    /// from the identity table via `generated_symbol_query()` alone.
    #[test]
    fn query_surface_resolves_generated_method_provenance_to_real_lines() {
        let source = GENERATED_METHOD_FIXTURE;
        let compiler = compiled_with_source(source);
        let application_line = line_of(source, "@gen()");
        let generator_line = line_of(source, "comptime post(target, ctx)");

        let provenance = compiler
            .generated_symbol_query()
            .provenance_for_name("Point.answer")
            .expect("generated method resolves through the query surface");
        assert_eq!(provenance.decl_name, "Point.answer");
        assert_eq!(
            provenance.node_path.render(),
            "extend:Point/method:answer",
            "node path identifies the generated node"
        );
        assert_eq!(
            resolved_line(&compiler, provenance.checked_decl.span()),
            application_line,
            "checked-decl location resolves to the application line (S3 anchoring)"
        );
        assert_eq!(
            resolved_line(&compiler, provenance.application.span()),
            application_line,
            "application location resolves to the @gen() line"
        );
        assert_eq!(
            resolved_line(&compiler, provenance.generator.span()),
            generator_line,
            "generator-definition location resolves to the comptime handler line"
        );

        let by_id = compiler
            .generated_symbol_query()
            .provenance_of(provenance.symbol)
            .expect("issued SymbolId resolves to the same provenance");
        assert_eq!(by_id, provenance, "name view and SymbolId view agree");
    }

    /// The query surface lists every generated symbol (workspace-symbol
    /// consumption) in deterministic order, and resolves a position inside
    /// the checked-decl anchor to the generated declarations anchored there.
    #[test]
    fn query_surface_lists_and_position_resolves_generated_symbols() {
        let source = GENERATED_METHOD_FIXTURE;
        let compiler = compiled_with_source(source);
        let query = compiler.generated_symbol_query();

        let names: Vec<&str> = query
            .generated_symbols()
            .iter()
            .map(|provenance| provenance.decl_name)
            .collect();
        assert_eq!(
            names,
            vec!["Point.answer", "Point.double"],
            "workspace-symbol listing enumerates every generated decl deterministically"
        );

        let anchor = query
            .provenance_for_name("Point.answer")
            .expect("generated method resolves")
            .application;
        let at_application = query.symbols_at(anchor.file_id(), anchor.span().start);
        assert_eq!(
            at_application
                .iter()
                .map(|provenance| provenance.decl_name)
                .collect::<Vec<_>>(),
            vec!["Point.answer", "Point.double"],
            "a position on the application resolves every decl anchored there"
        );

        let type_offset = source.find("type Point").expect("fixture has the type");
        assert!(
            query.symbols_at(anchor.file_id(), type_offset).is_empty(),
            "a position outside every generated anchor resolves to nothing"
        );
    }

    /// Rejection row 7: an error raised INSIDE a generated method body
    /// (here: the generated body calls an undefined function) surfaces as
    /// the C0003 diagnostic carrying THREE location-bearing notes —
    /// generated node, application site, generator definition — each
    /// resolving to its real line.
    #[test]
    fn generated_body_failure_carries_three_provenance_note_locations() {
        let source = "type UserRow { id: int }\n// application line\n// generator line\n";
        let program = parse(source);
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        compiler
            .compile_in_place(&program)
            .expect("target type compiles");
        compiler.set_source(source);

        let application_offset = source.find("// application line").expect("marker");
        let generator_offset = source.find("// generator line").expect("marker");
        let application_span = Span::new(application_offset, application_offset + 4);
        let generator_span = Span::new(generator_offset, generator_offset + 4);
        let application_line = line_of(source, "// application line");
        let generator_line = line_of(source, "// generator line");

        let no_args: [(&str, &str); 0] = [];
        let site = ExpansionSite::new(
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor("annotation:broken_gen:comptime-post"),
                ApplicationId::from_canonical_descriptor("application:test:row7"),
                TargetIdentity::from_canonical_descriptor("type:UserRow"),
                ComptimeStage::AnnotationHandler,
                CanonicalHash::from_canonical_argument_descriptors(&no_args),
                CanonicalHash::from_canonical_dependency_descriptors(&[]),
            ),
            0,
            application_span,
            generator_span,
        );

        let extend_program =
            parse("extend UserRow { method broken() -> int { missing_helper() } }");
        let extend = extend_program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend.clone()),
                _ => None,
            })
            .expect("program contains an extend item");

        let err = compiler
            .apply_comptime_extend(extend, "UserRow", &site)
            .expect_err("a generated body calling an undefined function must fail");
        let ShapeError::SemanticError {
            message,
            location: Some(location),
        } = &err
        else {
            panic!("row-7 failure must be a located SemanticError, got {err:?}");
        };
        assert!(
            message.contains("error in generated declaration `UserRow.broken`"),
            "row-7 message must name the generated declaration: {message}"
        );
        assert_eq!(
            location.line, application_line,
            "row-7 primary location anchors at the application site"
        );
        assert_eq!(
            location.notes.len(),
            3,
            "row-7 diagnostic carries exactly the three provenance notes: {:?}",
            location.notes
        );
        let generated_note = &location.notes[0];
        assert!(
            generated_note
                .message
                .contains("generated node extend:UserRow/method:broken"),
            "generated-node note must carry the node path: {}",
            generated_note.message
        );
        assert_eq!(
            generated_note
                .location
                .as_ref()
                .expect("generated-node note has a location")
                .line,
            application_line,
            "generated-node note resolves to the checked-decl (application) line"
        );
        let application_note = &location.notes[1];
        assert!(
            application_note
                .message
                .contains("generated from this application site"),
            "application note missing: {}",
            application_note.message
        );
        assert_eq!(
            application_note
                .location
                .as_ref()
                .expect("application note has a location")
                .line,
            application_line,
        );
        let generator_note = &location.notes[2];
        assert!(
            generator_note.message.contains("generator defined here"),
            "generator note missing: {}",
            generator_note.message
        );
        assert_eq!(
            generator_note
                .location
                .as_ref()
                .expect("generator note has a location")
                .line,
            generator_line,
            "generator note resolves to the generator-definition line"
        );
    }

    /// End-to-end row 7: the SAME provenance-carrying diagnostic surfaces
    /// through the full two-phase pipeline when an annotation handler
    /// generates a method whose body fails to compile. Runs under the
    /// RecoverAll diagnostic modes (the LSP configuration) so the pipeline
    /// reaches pass-2's generated-body compile after the analyzer has
    /// already recorded its own view of the broken body; the row-7
    /// diagnostic must be among the surfaced errors WITH its three
    /// location-bearing notes intact (the outer directive-processing wrap
    /// must not flatten them to a string).
    #[test]
    fn end_to_end_generated_body_failure_carries_provenance() {
        let source = r#"
annotation bad_gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method broken() -> int { missing_helper() }
    }
  }
}

@bad_gen()
type Point { id: int }
"#;
        let program = parse(source);
        let mut compiler = BytecodeCompiler::new();
        compiler.set_type_diagnostic_mode(crate::compiler::TypeDiagnosticMode::RecoverAll);
        compiler.set_compile_diagnostic_mode(crate::compiler::CompileDiagnosticMode::RecoverAll);
        compiler.set_source(source);
        let err = compiler
            .compile_in_place(&program)
            .expect_err("generated body calling an undefined function must fail the compile");

        fn flatten<'a>(e: &'a ShapeError, out: &mut Vec<&'a ShapeError>) {
            if let ShapeError::MultiError(errors) = e {
                for inner in errors {
                    flatten(inner, out);
                }
            } else {
                out.push(e);
            }
        }
        let mut flat = Vec::new();
        flatten(&err, &mut flat);
        let provenance_error = flat
            .iter()
            .find_map(|e| match e {
                ShapeError::SemanticError {
                    message,
                    location: Some(location),
                } if message.contains("error in generated declaration `Point.broken`") => {
                    Some(location)
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "end-to-end failure must include the provenance-carrying \
                     generated-decl diagnostic, got: {flat:?}"
                )
            });
        assert_eq!(
            provenance_error.notes.len(),
            3,
            "the three provenance notes must survive the full pipeline: {:?}",
            provenance_error.notes
        );
    }
}
