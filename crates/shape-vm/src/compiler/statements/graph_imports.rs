//! Late runtime registration for resolved module-graph imports.

use super::annotation_imports::AnnotationImportSemanticSnapshot;
use super::{
    BytecodeCompiler, ImportedAnnotationSymbol, ImportedSymbol, Instruction, ModuleBuiltinFunction,
    OpCode, Operand, Result,
};
use crate::module_graph::{ModuleGraph, ModuleId, ModuleSourceKind, ResolvedImport};

impl BytecodeCompiler {
    pub(super) fn register_graph_imports_for_module(
        &mut self,
        module_id: ModuleId,
        graph: &ModuleGraph,
    ) -> Result<()> {
        let semantics = self.annotation_import_semantic_snapshot();
        self.register_graph_imports_with_annotation_semantics(module_id, graph, &semantics)
    }

    pub(super) fn register_graph_imports_with_annotation_semantics(
        &mut self,
        module_id: ModuleId,
        graph: &ModuleGraph,
        semantics: &AnnotationImportSemanticSnapshot,
    ) -> Result<()> {
        let node = graph.node(module_id);
        let resolved_imports = node.resolved_imports.clone();

        // The complete import set is authorized before any imported symbol,
        // annotation, module binding, instruction, or carrier is published.
        self.authorize_and_stage_graph_import_permissions(module_id, graph, &resolved_imports)?;

        for resolved in &resolved_imports {
            match resolved {
                ResolvedImport::Namespace {
                    local_name,
                    canonical_path,
                    module_id: dependency_id,
                } => {
                    let dependency = graph.node(*dependency_id);
                    let canonical_idx = self.get_or_create_module_binding(canonical_path);
                    if matches!(
                        dependency.source_kind,
                        ModuleSourceKind::NativeModule | ModuleSourceKind::Hybrid
                    ) {
                        self.register_extension_module_schema(canonical_path);
                        let schema_name = format!("__mod_{canonical_path}");
                        if self
                            .type_tracker
                            .schema_registry()
                            .get(&schema_name)
                            .is_some()
                        {
                            self.set_module_binding_type_info(canonical_idx, &schema_name);
                        }
                    }

                    if local_name != canonical_path {
                        let alias_idx = self.get_or_create_module_binding(local_name);
                        if let Some(type_info) =
                            self.type_tracker.get_binding_type(canonical_idx).cloned()
                        {
                            self.type_tracker.set_binding_type(alias_idx, type_info);
                        } else {
                            let schema_name = format!("__mod_{canonical_path}");
                            if self
                                .type_tracker
                                .schema_registry()
                                .get(&schema_name)
                                .is_some()
                            {
                                self.set_module_binding_type_info(alias_idx, &schema_name);
                            }
                        }
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(canonical_idx)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::StoreModuleBinding,
                            Some(Operand::ModuleBinding(alias_idx)),
                        ));
                    }

                    self.module_namespace_bindings.insert(local_name.clone());
                    self.graph_namespace_map
                        .entry(local_name.clone())
                        .or_insert_with(|| canonical_path.clone());
                }
                ResolvedImport::Named {
                    canonical_path,
                    module_id: dependency_id,
                    symbols,
                } => {
                    let dependency = graph.node(*dependency_id);
                    for symbol in symbols {
                        if symbol.is_annotation {
                            if Self::annotation_import_is_shadowed(semantics, &symbol.local_name) {
                                continue;
                            }
                            self.module_scope_sources
                                .entry(canonical_path.clone())
                                .or_insert_with(|| canonical_path.clone());
                            self.imported_annotations
                                .entry(symbol.local_name.clone())
                                .or_insert_with(|| ImportedAnnotationSymbol {
                                    original_name: symbol.original_name.clone(),
                                    _module_path: canonical_path.clone(),
                                    hidden_module_name: canonical_path.clone(),
                                });
                            continue;
                        }

                        self.imported_names
                            .entry(symbol.local_name.clone())
                            .or_insert_with(|| ImportedSymbol {
                                original_name: symbol.original_name.clone(),
                                module_path: canonical_path.clone(),
                                kind: Some(symbol.kind),
                            });
                        if matches!(
                            dependency.source_kind,
                            ModuleSourceKind::NativeModule | ModuleSourceKind::Hybrid
                        ) && matches!(
                            symbol.kind,
                            shape_ast::module_utils::ModuleExportKind::Function
                                | shape_ast::module_utils::ModuleExportKind::BuiltinFunction
                        ) {
                            self.module_builtin_functions
                                .entry(symbol.local_name.clone())
                                .or_insert_with(|| ModuleBuiltinFunction {
                                    export_name: symbol.original_name.clone(),
                                    source_module_path: canonical_path.clone(),
                                });
                        }
                        if matches!(
                            symbol.kind,
                            shape_ast::module_utils::ModuleExportKind::Value
                        ) {
                            self.register_imported_const_initializer(
                                dependency,
                                &symbol.original_name,
                                &symbol.local_name,
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn register_imported_const_initializer(
        &mut self,
        dependency: &crate::module_graph::ModuleNode,
        original_name: &str,
        local_name: &str,
    ) {
        let Some(ast) = dependency.ast.as_ref() else {
            return;
        };
        for item in &ast.items {
            let shape_ast::ast::Item::Export(export, _) = item else {
                continue;
            };
            let Some(declaration) = export.source_decl.as_ref() else {
                continue;
            };
            if declaration.kind != shape_ast::ast::VarKind::Const
                || declaration.pattern.as_identifier() != Some(original_name)
            {
                continue;
            }
            if let Some(initializer) = declaration.value.as_ref() {
                self.imported_consts
                    .entry(local_name.to_string())
                    .or_insert_with(|| initializer.clone());
            }
        }
    }
}
