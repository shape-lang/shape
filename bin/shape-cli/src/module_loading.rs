use anyhow::Result;
use shape_runtime::engine::ShapeEngine;
use shape_vm::BytecodeExecutor;
use std::path::{Path, PathBuf};

/// Wire module loading/import pre-resolution for VM execution.
///
/// This keeps CLI command paths aligned with runtime module resolution semantics.
pub fn wire_vm_executor_module_loading(
    engine: &mut ShapeEngine,
    executor: &mut BytecodeExecutor,
    context_file: Option<&Path>,
    source: Option<&str>,
) -> Result<()> {
    let dep_paths = engine.get_runtime_mut().get_dependency_paths().clone();
    if !dep_paths.is_empty() {
        executor.set_dependency_paths(dep_paths);
    }

    if let Some(resolutions) = resolve_native_libraries_for_context(context_file, source)? {
        executor.set_native_library_overrides(resolutions.alias_load_targets()?);
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
    Ok(())
}

fn resolve_native_libraries_for_context(
    context_file: Option<&Path>,
    source: Option<&str>,
) -> Result<Option<shape_runtime::native_resolution::NativeResolutionSet>> {
    let Some(context_file) = context_file else {
        return Ok(None);
    };

    let base_dir = context_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    if let Some(project) = shape_runtime::project::find_project_root(&base_dir) {
        let lock_path = project.root_path.join("shape.lock");
        let resolutions = shape_runtime::native_resolution::resolve_native_dependencies_for_project(
            &project,
            &lock_path,
            project.config.build.external.mode,
        )?;
        return Ok(Some(resolutions));
    }

    let Some(source) = source else {
        return Ok(None);
    };

    let (frontmatter, _) = shape_runtime::frontmatter::parse_frontmatter(source);
    let Some(frontmatter) = frontmatter else {
        return Ok(None);
    };
    let scopes =
        shape_runtime::native_resolution::collect_native_dependency_scopes(&base_dir, &frontmatter)?;
    if scopes.is_empty() {
        return Ok(None);
    }
    let lock_path = standalone_script_lock_path(context_file);
    let resolutions = shape_runtime::native_resolution::resolve_native_dependency_scopes(
        &scopes,
        Some(&lock_path),
        shape_runtime::project::ExternalLockMode::Update,
        true,
    )?;
    Ok(Some(resolutions))
}

fn standalone_script_lock_path(script_path: &Path) -> PathBuf {
    let mut lock_path = script_path.to_path_buf();
    lock_path.set_extension("lock");
    lock_path
}
