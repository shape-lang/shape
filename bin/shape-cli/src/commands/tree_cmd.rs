use anyhow::{Context, Result};
use shape_runtime::dependency_resolver::DependencyResolver;
use shape_runtime::package_bundle::PackageBundle;
use shape_runtime::project::{ShapeProject, parse_shape_project_toml};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub async fn run_tree(show_native: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let project = shape_runtime::project::find_project_root(&cwd).ok_or_else(|| {
        anyhow::anyhow!("No shape.toml found. Run `shape tree` from within a Shape project.")
    })?;

    let root_name = if project.config.project.name.trim().is_empty() {
        project
            .root_path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("root")
            .to_string()
    } else {
        project.config.project.name.trim().to_string()
    };
    let root_version = if project.config.project.version.trim().is_empty() {
        "0.0.0".to_string()
    } else {
        project.config.project.version.trim().to_string()
    };

    println!("{}@{}", root_name, root_version);

    let mut visited = HashSet::new();
    let canonical_root = project
        .root_path
        .canonicalize()
        .unwrap_or_else(|_| project.root_path.clone());
    visited.insert(canonical_root);

    print_project_tree(
        &project.root_path,
        &project.config,
        "",
        show_native,
        &mut visited,
    )
}

fn print_project_tree(
    root: &Path,
    project: &ShapeProject,
    prefix: &str,
    show_native: bool,
    visited: &mut HashSet<PathBuf>,
) -> Result<()> {
    let mut dep_names: Vec<String> = project.dependencies.keys().cloned().collect();
    dep_names.sort();
    if dep_names.is_empty() {
        return Ok(());
    }

    let resolver = DependencyResolver::new(root.to_path_buf()).ok_or_else(|| {
        anyhow::anyhow!(
            "failed to initialize dependency resolver (unable to determine cache directory)"
        )
    })?;
    let resolved = resolver.resolve(&project.dependencies).map_err(|e| {
        anyhow::anyhow!(
            "failed to resolve dependencies for '{}': {}",
            root.display(),
            e
        )
    })?;

    for (idx, dep_name) in dep_names.iter().enumerate() {
        let is_last = idx + 1 == dep_names.len();
        let branch = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}│   ", prefix)
        };

        let resolved_dep = resolved
            .iter()
            .find(|dep| dep.name == *dep_name)
            .ok_or_else(|| {
                anyhow::anyhow!("resolved dependency entry missing for '{}'", dep_name)
            })?;

        let dep_kind = match &resolved_dep.source {
            shape_runtime::dependency_resolver::ResolvedDependencySource::Path => "source",
            shape_runtime::dependency_resolver::ResolvedDependencySource::Bundle => "bundle",
            shape_runtime::dependency_resolver::ResolvedDependencySource::Git { .. } => "git",
            shape_runtime::dependency_resolver::ResolvedDependencySource::Registry { .. } => {
                "registry"
            }
        };
        let is_bundle_path = resolved_dep
            .path
            .extension()
            .is_some_and(|ext| ext == "shapec");

        println!(
            "{}{}{}@{} [{}]",
            prefix, branch, dep_name, resolved_dep.version, dep_kind
        );

        if dep_kind == "bundle" || is_bundle_path {
            print_bundle_tree(&resolved_dep.path, &child_prefix, show_native)?;
            continue;
        }

        let dep_root = resolved_dep.path.clone();
        let canonical = dep_root.canonicalize().unwrap_or(dep_root.clone());
        if !visited.insert(canonical) {
            println!("{}└── (cycle)", child_prefix);
            continue;
        }

        let dep_toml = dep_root.join("shape.toml");
        if !dep_toml.is_file() {
            println!("{}└── (missing shape.toml)", child_prefix);
            continue;
        }

        let source = std::fs::read_to_string(&dep_toml)
            .with_context(|| format!("failed to read {}", dep_toml.display()))?;
        let dep_project = parse_shape_project_toml(&source)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {}", dep_toml.display(), e))?;

        print_project_tree(&dep_root, &dep_project, &child_prefix, show_native, visited)?;
    }

    Ok(())
}

fn print_bundle_tree(bundle_path: &Path, prefix: &str, show_native: bool) -> Result<()> {
    let bundle = PackageBundle::read_from_file(bundle_path)
        .map_err(|e| anyhow::anyhow!("failed to read bundle '{}': {}", bundle_path.display(), e))?;

    if !bundle.metadata.bundle_kind.is_empty() && bundle.metadata.bundle_kind != "portable-bytecode"
    {
        println!(
            "{}└── ! unsupported bundle_kind '{}'",
            prefix, bundle.metadata.bundle_kind
        );
        return Ok(());
    }

    if !bundle.dependencies.is_empty() {
        let mut deps: Vec<_> = bundle.dependencies.iter().collect();
        deps.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (idx, (name, version)) in deps.iter().enumerate() {
            let branch = if idx + 1 == deps.len()
                && !(show_native && !bundle.native_dependency_scopes.is_empty())
            {
                "└── "
            } else {
                "├── "
            };
            println!("{}{}{}@{} [bundle-meta]", prefix, branch, name, version);
        }
    }

    if show_native {
        if bundle.native_dependency_scopes.is_empty() {
            println!("{}└── [native] none", prefix);
        } else {
            let mut scopes = bundle.native_dependency_scopes.clone();
            scopes.sort_by(|a, b| a.package_key.cmp(&b.package_key));
            for (scope_idx, scope) in scopes.iter().enumerate() {
                let mut aliases: Vec<_> = scope.dependencies.keys().cloned().collect();
                aliases.sort();
                let branch = if scope_idx + 1 == scopes.len() {
                    "└── "
                } else {
                    "├── "
                };
                println!(
                    "{}{}[native] {} => {}",
                    prefix,
                    branch,
                    scope.package_key,
                    aliases.join(", ")
                );
            }
        }

        if !bundle.metadata.native_portable {
            let host = if bundle.metadata.build_host.trim().is_empty() {
                "unknown"
            } else {
                bundle.metadata.build_host.as_str()
            };
            println!("{}└── [native] host-bound (built on {})", prefix, host);
        }
    }

    Ok(())
}
