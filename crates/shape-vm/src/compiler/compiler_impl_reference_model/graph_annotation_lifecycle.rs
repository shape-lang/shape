//! Graph dependency compilation under module-scoped annotation semantics.

use crate::compiler::BytecodeCompiler;
use crate::module_graph::{ModuleGraph, ModuleSourceKind};
use shape_ast::ast::{Item, Program};
use shape_ast::error::{Result, ShapeError};
use std::sync::Arc;

impl BytecodeCompiler {
    /// Borrowed graph driver shared by production and state-inspecting tests.
    pub(in crate::compiler) fn compile_with_graph_and_prelude_in_place(
        &mut self,
        root_program: &Program,
        graph: Arc<ModuleGraph>,
        _prelude_paths: &[String],
    ) -> Result<()> {
        self.ensure_annotation_compiler_usable()?;
        self.module_graph = Some(graph.clone());

        // Stage the root view before dependency compilation mutates scoped
        // import semantics; restore this exact view for root discovery.
        let root_annotation_semantics =
            self.stage_graph_annotation_imports_for_module(root_program, graph.root_id(), &graph)?;

        // The graph unit owns one registration-complete semantic freeze. Run
        // both freeze-input passes across every qualified dependency and then
        // the root before any dependency body or comptime site executes.
        for pass in [
            crate::compiler::statements::SemanticFreezePredeclarePass::TypesAndTraits,
            crate::compiler::statements::SemanticFreezePredeclarePass::Impls,
        ] {
            for &dependency_id in graph.topo_order() {
                let node = graph.node(dependency_id);
                if !matches!(
                    node.source_kind,
                    ModuleSourceKind::ShapeSource | ModuleSourceKind::Hybrid
                ) {
                    continue;
                }
                let Some(ast) = node.ast.clone() else {
                    continue;
                };
                let module_path = node.canonical_path.clone();
                self.module_scope_stack.push(module_path.clone());
                let result = (|| -> Result<()> {
                    for item in &ast.items {
                        if matches!(item, Item::Import(..))
                            || !Self::item_can_carry_semantic_freeze_inputs(item)
                        {
                            continue;
                        }
                        let qualified = self.qualify_module_item(item, &module_path)?;
                        if pass
                            == crate::compiler::statements::SemanticFreezePredeclarePass::TypesAndTraits
                        {
                            self.predeclare_item_struct_schemas(&qualified);
                        }
                        self.predeclare_item_semantic_freeze_inputs(&qualified, pass)?;
                    }
                    Ok(())
                })();
                self.module_scope_stack.pop();
                result?;
            }
            for item in &root_program.items {
                if pass
                    == crate::compiler::statements::SemanticFreezePredeclarePass::TypesAndTraits
                {
                    self.predeclare_item_struct_schemas(item);
                }
                self.predeclare_item_semantic_freeze_inputs(item, pass)?;
            }
        }
        self.install_semantic_freeze()?;

        self.compile_graph_dependency_modules(&graph)?;

        let mut stripped_program = root_program.clone();
        stripped_program
            .items
            .retain(|item| !matches!(item, Item::Import(..)));
        self.restore_annotation_import_semantics(&root_annotation_semantics);
        self.compile_in_place(&stripped_program)
    }

    pub(super) fn compile_graph_dependency_modules(&mut self, graph: &ModuleGraph) -> Result<()> {
        for &module_id in graph.topo_order() {
            let node = graph.node(module_id);
            let permission_owner = self.enter_graph_permission_owner(module_id, graph)?;
            let compilation = match node.source_kind {
                ModuleSourceKind::NativeModule => {
                    let saved = self.annotation_import_semantic_snapshot();
                    let registration = self.register_graph_imports_for_module(module_id, graph);
                    self.restore_annotation_import_semantics(&saved);
                    registration
                }
                ModuleSourceKind::ShapeSource | ModuleSourceKind::Hybrid => {
                    self.compile_module_from_graph(module_id, graph)
                }
                ModuleSourceKind::CompiledBytecode => Err(ShapeError::ModuleError {
                    message: format!(
                        "Module '{}' is only available as pre-compiled bytecode",
                        node.canonical_path
                    ),
                    module_path: None,
                }),
            };
            let permission_completion = if compilation.is_ok() {
                self.complete_graph_import_permissions(module_id, graph)
            } else {
                self.discard_graph_import_permissions(module_id, graph)
            };
            let leave = self.leave_graph_permission_owner(permission_owner);
            match (compilation, permission_completion, leave) {
                (Err(error), _, _)
                | (Ok(()), Err(error), _)
                | (Ok(()), Ok(()), Err(error)) => return Err(error),
                (Ok(()), Ok(()), Ok(())) => {}
            }
        }
        Ok(())
    }
}
