//! Graph dependency compilation under module-scoped annotation semantics.

use crate::compiler::BytecodeCompiler;
use crate::module_graph::{ModuleGraph, ModuleSourceKind};
use shape_ast::error::{Result, ShapeError};

impl BytecodeCompiler {
    pub(super) fn compile_graph_dependency_modules(&mut self, graph: &ModuleGraph) -> Result<()> {
        for &module_id in graph.topo_order() {
            let node = graph.node(module_id);
            match node.source_kind {
                ModuleSourceKind::NativeModule => {
                    let saved = self.annotation_import_semantic_snapshot();
                    let registration = self.register_graph_imports_for_module(module_id, graph);
                    self.restore_annotation_import_semantics(&saved);
                    registration?;
                }
                ModuleSourceKind::ShapeSource | ModuleSourceKind::Hybrid => {
                    self.compile_module_from_graph(module_id, graph)?;
                }
                ModuleSourceKind::CompiledBytecode => {
                    return Err(ShapeError::ModuleError {
                        message: format!(
                            "Module '{}' is only available as pre-compiled bytecode",
                            node.canonical_path
                        ),
                        module_path: None,
                    });
                }
            }
        }
        Ok(())
    }
}
