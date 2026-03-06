use shape_runtime::engine::ShapeEngine;
use shape_vm::BytecodeExecutor;
use std::path::Path;

/// Wire module loading/import pre-resolution for VM execution.
///
/// This keeps CLI command paths aligned with runtime module resolution semantics.
pub fn wire_vm_executor_module_loading(
    engine: &mut ShapeEngine,
    executor: &mut BytecodeExecutor,
    context_file: Option<&Path>,
    source: Option<&str>,
) {
    let dep_paths = engine.get_runtime_mut().get_dependency_paths().clone();
    if !dep_paths.is_empty() {
        executor.set_dependency_paths(dep_paths);
    }

    let mut loader = engine.get_runtime_mut().configured_module_loader();
    if let Some(context_file) = context_file {
        loader.configure_for_context(context_file, None);
    }
    executor.set_module_loader(loader);

    if let Some(source) = source {
        let context_dir = context_file.and_then(Path::parent);
        executor.resolve_file_imports_from_source(source, context_dir);
    }
}
