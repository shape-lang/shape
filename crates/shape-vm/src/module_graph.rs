//! Canonical module graph for dependency-ordered compilation.
//!
//! Replaces AST import inlining with a directed acyclic graph where each
//! module is a node with its own resolved imports and public interface.
//! Modules compile in topological order using the graph for cross-module
//! name resolution.

use std::collections::{HashMap, HashSet};
use shape_value::ValueWordExt;
use std::sync::Arc;

use shape_ast::ast::FunctionDef;
use shape_ast::module_utils::ModuleExportKind;
use shape_ast::Program;

// ---------------------------------------------------------------------------
// Core identifiers
// ---------------------------------------------------------------------------

/// Opaque module identity — index into the graph's node array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u32);

// ---------------------------------------------------------------------------
// Source classification
// ---------------------------------------------------------------------------

/// How a module's implementation is provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleSourceKind {
    /// Has `.shape` source, compiles to bytecode.
    ShapeSource,
    /// Rust-backed `ModuleExports`, runtime dispatch only.
    NativeModule,
    /// Both native exports AND Shape source overlay.
    Hybrid,
    /// Pre-compiled, no source available (deferred — emits hard error).
    CompiledBytecode,
}

// ---------------------------------------------------------------------------
// Export visibility
// ---------------------------------------------------------------------------

/// Visibility of a module export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleExportVisibility {
    /// Available at both compile time and runtime.
    Public,
    /// Available only during comptime evaluation.
    ComptimeOnly,
}

// ---------------------------------------------------------------------------
// Module interface
// ---------------------------------------------------------------------------

/// Metadata for a single exported symbol.
#[derive(Debug, Clone)]
pub struct ExportedSymbol {
    /// What kind of symbol this is (function, type, annotation, etc.).
    pub kind: ModuleExportKind,
    /// The full function definition, if this export is a Shape function.
    pub function_def: Option<Arc<FunctionDef>>,
    /// Visibility level.
    pub visibility: ModuleExportVisibility,
}

/// The public interface of a module — what it exports.
#[derive(Debug, Clone, Default)]
pub struct ModuleInterface {
    /// Exported symbols keyed by their public name (after alias resolution).
    pub exports: HashMap<String, ExportedSymbol>,
}

// ---------------------------------------------------------------------------
// Resolved imports (per-node)
// ---------------------------------------------------------------------------

/// A single symbol from a named import (`from m use { a, b as c }`).
#[derive(Debug, Clone)]
pub struct NamedImportSymbol {
    /// Name as it appears in the source module.
    pub original_name: String,
    /// Name bound in the importing module (may differ via `as` alias).
    pub local_name: String,
    /// Whether this import targets an annotation definition.
    pub is_annotation: bool,
    /// Resolved kind from the dependency's interface.
    pub kind: ModuleExportKind,
}

/// A resolved import — how the importing module accesses a dependency.
#[derive(Debug, Clone)]
pub enum ResolvedImport {
    /// Namespace import: `use std::core::math` or `use std::core::math as m`
    Namespace {
        /// Local name bound in the importing module (e.g. `math` or `m`).
        local_name: String,
        /// Canonical module path (e.g. `std::core::math`).
        canonical_path: String,
        /// Graph node of the imported module.
        module_id: ModuleId,
    },
    /// Named import: `from std::core::math use { sqrt, PI }`
    Named {
        /// Canonical module path.
        canonical_path: String,
        /// Graph node of the imported module.
        module_id: ModuleId,
        /// Individual symbols being imported.
        symbols: Vec<NamedImportSymbol>,
    },
}

// ---------------------------------------------------------------------------
// Graph nodes
// ---------------------------------------------------------------------------

/// A single module in the dependency graph.
#[derive(Debug, Clone)]
pub struct ModuleNode {
    /// Unique identity within this graph.
    pub id: ModuleId,
    /// Canonical module path (e.g. `std::core::math`, `mypackage::utils`).
    pub canonical_path: String,
    /// How this module is implemented.
    pub source_kind: ModuleSourceKind,
    /// Parsed AST (present for `ShapeSource` and `Hybrid`, absent for
    /// `NativeModule` and `CompiledBytecode`).
    pub ast: Option<Program>,
    /// Public interface (exports).
    pub interface: ModuleInterface,
    /// Resolved imports for this module.
    pub resolved_imports: Vec<ResolvedImport>,
    /// Direct dependencies (modules this one imports).
    pub dependencies: Vec<ModuleId>,
}

// ---------------------------------------------------------------------------
// The graph
// ---------------------------------------------------------------------------

/// Canonical module graph — the single source of truth for import resolution.
///
/// Built before compilation; modules compile in `topo_order` so that
/// dependencies are always available when a module is compiled.
#[derive(Debug, Clone)]
pub struct ModuleGraph {
    /// All module nodes, indexed by `ModuleId`.
    nodes: Vec<ModuleNode>,
    /// Canonical path → node id lookup.
    path_to_id: HashMap<String, ModuleId>,
    /// Topological compilation order (dependencies before dependents).
    topo_order: Vec<ModuleId>,
    /// The root module (entry point / user script).
    root_id: ModuleId,
}

impl ModuleGraph {
    /// Create a new graph from pre-built components.
    ///
    /// Used by the graph builder after all nodes, interfaces, and edges
    /// have been constructed and topologically sorted.
    pub fn new(
        nodes: Vec<ModuleNode>,
        path_to_id: HashMap<String, ModuleId>,
        topo_order: Vec<ModuleId>,
        root_id: ModuleId,
    ) -> Self {
        Self {
            nodes,
            path_to_id,
            topo_order,
            root_id,
        }
    }

    /// Look up a module by its canonical path.
    pub fn id_for_path(&self, path: &str) -> Option<ModuleId> {
        self.path_to_id.get(path).copied()
    }

    /// Get a module node by id.
    pub fn node(&self, id: ModuleId) -> &ModuleNode {
        &self.nodes[id.0 as usize]
    }

    /// Get a mutable module node by id.
    pub fn node_mut(&mut self, id: ModuleId) -> &mut ModuleNode {
        &mut self.nodes[id.0 as usize]
    }

    /// Topological compilation order (dependencies before dependents).
    /// Does NOT include the root module — that is compiled separately.
    pub fn topo_order(&self) -> &[ModuleId] {
        &self.topo_order
    }

    /// The root module id.
    pub fn root_id(&self) -> ModuleId {
        self.root_id
    }

    /// All nodes in the graph.
    pub fn nodes(&self) -> &[ModuleNode] {
        &self.nodes
    }

    /// Number of modules in the graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Check if a canonical path is registered in the graph.
    pub fn contains(&self, path: &str) -> bool {
        self.path_to_id.contains_key(path)
    }
}

// ---------------------------------------------------------------------------
// Graph builder
// ---------------------------------------------------------------------------

/// Errors that can occur during graph construction.
#[derive(Debug, Clone)]
pub enum GraphBuildError {
    /// Circular dependency detected.
    CyclicDependency {
        /// The cycle path, e.g. `["a", "b", "c", "a"]`.
        cycle: Vec<String>,
    },
    /// A module is only available as pre-compiled bytecode.
    CompiledBytecodeNotSupported {
        module_path: String,
    },
    /// Module not found.
    ModuleNotFound {
        module_path: String,
        requested_by: String,
    },
    /// Other error during graph construction.
    Other {
        message: String,
    },
}

impl std::fmt::Display for GraphBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphBuildError::CyclicDependency { cycle } => {
                write!(
                    f,
                    "Circular dependency detected: {}",
                    cycle.join(" → ")
                )
            }
            GraphBuildError::CompiledBytecodeNotSupported { module_path } => {
                write!(
                    f,
                    "Module '{}' is only available as pre-compiled bytecode. \
                     Graph-mode compilation requires source modules. Use \
                     `shape bundle --include-source` to include source in the \
                     package, or compile the dependency from source.",
                    module_path
                )
            }
            GraphBuildError::ModuleNotFound {
                module_path,
                requested_by,
            } => {
                write!(
                    f,
                    "Module '{}' not found (imported by '{}')",
                    module_path, requested_by
                )
            }
            GraphBuildError::Other { message } => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for GraphBuildError {}

/// Classification hint for how a module path should be resolved.
///
/// Used during graph construction to decide how to handle each dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleSourceKindHint {
    /// Module is backed by a native extension (Rust `ModuleExports`).
    NativeExtension,
    /// Module has `.shape` source code available.
    ShapeSource,
    /// Module is an embedded stdlib module.
    EmbeddedStdlib,
    /// Module is available only as a pre-compiled bundle.
    CompiledBundle,
    /// Module could not be found.
    NotFound,
}

/// Classify a module path by probing the module loader's resolvers.
pub fn resolve_module_source_kind(
    loader: &shape_runtime::module_loader::ModuleLoader,
    module_path: &str,
) -> ModuleSourceKindHint {
    // Check if it's a registered extension module (native)
    if loader.has_extension_module(module_path) {
        return ModuleSourceKindHint::NativeExtension;
    }
    // Check if it's an embedded stdlib module
    if loader.embedded_stdlib_module_paths().contains(&module_path.to_string()) {
        return ModuleSourceKindHint::EmbeddedStdlib;
    }
    // Check if we can resolve a file path for it
    if loader.resolve_module_path(module_path).is_ok() {
        return ModuleSourceKindHint::ShapeSource;
    }
    ModuleSourceKindHint::NotFound
}

/// Intermediate builder state used during graph construction.
pub struct GraphBuilder {
    nodes: Vec<ModuleNode>,
    path_to_id: HashMap<String, ModuleId>,
    /// Tracks modules currently being visited for cycle detection.
    visiting: HashSet<String>,
    /// Tracks modules that have been fully processed.
    visited: HashSet<String>,
}

impl GraphBuilder {
    /// Create a new empty graph builder.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            path_to_id: HashMap::new(),
            visiting: HashSet::new(),
            visited: HashSet::new(),
        }
    }

    /// Allocate a new node with the given canonical path and return its id.
    /// If a node with this path already exists, returns its existing id.
    pub fn get_or_create_node(&mut self, canonical_path: &str) -> ModuleId {
        if let Some(&id) = self.path_to_id.get(canonical_path) {
            return id;
        }
        let id = ModuleId(self.nodes.len() as u32);
        self.nodes.push(ModuleNode {
            id,
            canonical_path: canonical_path.to_string(),
            source_kind: ModuleSourceKind::ShapeSource, // default, overwritten later
            ast: None,
            interface: ModuleInterface::default(),
            resolved_imports: Vec::new(),
            dependencies: Vec::new(),
        });
        self.path_to_id.insert(canonical_path.to_string(), id);
        id
    }

    /// Mark a module as currently being visited (for cycle detection).
    /// Returns `false` if the module is already being visited (cycle!).
    pub fn begin_visit(&mut self, canonical_path: &str) -> bool {
        self.visiting.insert(canonical_path.to_string())
    }

    /// Mark a module as fully visited.
    pub fn end_visit(&mut self, canonical_path: &str) {
        self.visiting.remove(canonical_path);
        self.visited.insert(canonical_path.to_string());
    }

    /// Check if a module has been fully visited.
    pub fn is_visited(&self, canonical_path: &str) -> bool {
        self.visited.contains(canonical_path)
    }

    /// Check if a module is currently being visited (would form a cycle).
    pub fn is_visiting(&self, canonical_path: &str) -> bool {
        self.visiting.contains(canonical_path)
    }

    /// Get the cycle path when a cycle is detected.
    pub fn get_cycle_path(&self, target: &str) -> Vec<String> {
        // The visiting set doesn't preserve order, so we just report
        // the modules involved. The caller can provide more context.
        let mut cycle: Vec<String> = self.visiting.iter().cloned().collect();
        cycle.push(target.to_string());
        cycle
    }

    /// Compute topological order via DFS post-order.
    /// The root module is excluded from the topo order (compiled separately).
    pub fn compute_topo_order(&self, root_id: ModuleId) -> Vec<ModuleId> {
        let mut order = Vec::new();
        let mut visited = HashSet::new();
        for node in &self.nodes {
            self.topo_dfs(node.id, root_id, &mut visited, &mut order);
        }
        order
    }

    fn topo_dfs(
        &self,
        current: ModuleId,
        root_id: ModuleId,
        visited: &mut HashSet<ModuleId>,
        order: &mut Vec<ModuleId>,
    ) {
        if !visited.insert(current) {
            return;
        }
        let node = &self.nodes[current.0 as usize];
        for &dep in &node.dependencies {
            self.topo_dfs(dep, root_id, visited, order);
        }
        // Exclude root from topo order — it is compiled separately
        if current != root_id {
            order.push(current);
        }
    }

    /// Finalize into a `ModuleGraph`.
    pub fn build(self, root_id: ModuleId) -> ModuleGraph {
        let topo_order = self.compute_topo_order(root_id);
        ModuleGraph::new(self.nodes, self.path_to_id, topo_order, root_id)
    }
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Graph building
// ---------------------------------------------------------------------------

/// Extract import paths from a program's AST (same logic as
/// `shape_runtime::module_loader::resolution::extract_dependencies`).
fn extract_import_paths(ast: &Program) -> Vec<String> {
    ast.items
        .iter()
        .filter_map(|item| {
            if let shape_ast::ast::Item::Import(import_stmt, _) = item {
                Some(import_stmt.from.clone())
            } else {
                None
            }
        })
        .collect()
}

/// Build a module interface from a Shape source AST.
fn build_shape_interface(ast: &Program) -> ModuleInterface {
    let symbols = match shape_ast::module_utils::collect_exported_symbols(ast) {
        Ok(syms) => syms,
        Err(_) => return ModuleInterface::default(),
    };

    let mut exports = HashMap::new();
    for sym in symbols {
        let name = sym.alias.unwrap_or(sym.name);
        exports.insert(
            name,
            ExportedSymbol {
                kind: sym.kind,
                function_def: None,
                visibility: ModuleExportVisibility::Public,
            },
        );
    }

    ModuleInterface { exports }
}

/// Build a module interface from a native `ModuleExports`.
fn build_native_interface(
    module: &shape_runtime::module_exports::ModuleExports,
) -> ModuleInterface {
    let mut exports = HashMap::new();
    for name in module.export_names() {
        let visibility = match module.export_visibility(name) {
            shape_runtime::module_exports::ModuleExportVisibility::Public => {
                ModuleExportVisibility::Public
            }
            shape_runtime::module_exports::ModuleExportVisibility::ComptimeOnly => {
                ModuleExportVisibility::ComptimeOnly
            }
            shape_runtime::module_exports::ModuleExportVisibility::Internal => {
                ModuleExportVisibility::Public
            }
        };
        exports.insert(
            name.to_string(),
            ExportedSymbol {
                kind: ModuleExportKind::Function,
                function_def: None,
                visibility,
            },
        );
    }
    ModuleInterface { exports }
}

/// Resolve imports for a module node against the graph's dependency interfaces.
fn resolve_imports_for_node(
    ast: &Program,
    builder: &GraphBuilder,
) -> Vec<ResolvedImport> {
    let mut resolved = Vec::new();

    for item in &ast.items {
        let shape_ast::ast::Item::Import(import_stmt, _) = item else {
            continue;
        };
        let module_path = &import_stmt.from;
        let Some(&dep_id) = builder.path_to_id.get(module_path) else {
            continue;
        };
        let dep_node = &builder.nodes[dep_id.0 as usize];

        match &import_stmt.items {
            shape_ast::ast::ImportItems::Namespace { name, alias } => {
                let local_name = alias
                    .as_ref()
                    .or(Some(name))
                    .cloned()
                    .unwrap_or_else(|| {
                        module_path
                            .split("::")
                            .last()
                            .unwrap_or(module_path)
                            .to_string()
                    });
                resolved.push(ResolvedImport::Namespace {
                    local_name,
                    canonical_path: module_path.clone(),
                    module_id: dep_id,
                });
            }
            shape_ast::ast::ImportItems::Named(specs) => {
                let mut symbols = Vec::new();
                for spec in specs {
                    let kind = dep_node
                        .interface
                        .exports
                        .get(&spec.name)
                        .map(|e| e.kind)
                        .unwrap_or(ModuleExportKind::Function);
                    symbols.push(NamedImportSymbol {
                        original_name: spec.name.clone(),
                        local_name: spec.alias.clone().unwrap_or_else(|| spec.name.clone()),
                        is_annotation: spec.is_annotation,
                        kind,
                    });
                }
                resolved.push(ResolvedImport::Named {
                    canonical_path: module_path.clone(),
                    module_id: dep_id,
                    symbols,
                });
            }
        }
    }

    resolved
}

/// Build a complete module graph from a root program.
///
/// Algorithm:
/// 1. Pre-register native modules from `extensions`
/// 2. Create root node from the user's program
/// 3. Walk imports recursively (DFS), loading Shape sources via `loader`
/// 4. Build interfaces per node
/// 5. Resolve per-node imports against dependency interfaces
/// 6. Cycle detection via visiting set
/// 7. Topological sort
///
/// Prelude modules are included as synthetic low-priority imports on
/// each module node.
pub fn build_module_graph(
    root_program: &Program,
    loader: &mut shape_runtime::module_loader::ModuleLoader,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    prelude_imports: &[String],
) -> Result<ModuleGraph, GraphBuildError> {
    // Collect structured prelude imports from the loader
    let structured = collect_prelude_imports(loader);
    build_module_graph_with_prelude_structure(
        root_program,
        loader,
        extensions,
        prelude_imports,
        &structured,
    )
}

/// Build a module graph with pre-collected structured prelude import data.
fn build_module_graph_with_prelude_structure(
    root_program: &Program,
    loader: &mut shape_runtime::module_loader::ModuleLoader,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    prelude_imports: &[String],
    structured_prelude: &[PreludeImport],
) -> Result<ModuleGraph, GraphBuildError> {
    let mut builder = GraphBuilder::new();

    // Step 1: Pre-register native extension modules.
    // These have no source artifact in the loader; they are Rust-backed.
    for ext in extensions {
        let ext_id = builder.get_or_create_node(&ext.name);
        let node = &mut builder.nodes[ext_id.0 as usize];
        node.source_kind = ModuleSourceKind::NativeModule;
        node.interface = build_native_interface(ext);
        builder.visited.insert(ext.name.clone());

        // Also check if the extension provides Shape source overlays
        // (module_artifacts or shape_sources), making it Hybrid.
        let has_shape_source = ext
            .module_artifacts
            .iter()
            .any(|a| a.source.is_some() && a.module_path == ext.name)
            || !ext.shape_sources.is_empty();

        if has_shape_source {
            // Try to load the Shape overlay AST
            if let Ok(module) = loader.load_module(&ext.name) {
                let shape_interface = build_shape_interface(&module.ast);
                // Merge: Shape exports take priority over native
                let node = &mut builder.nodes[ext_id.0 as usize];
                node.source_kind = ModuleSourceKind::Hybrid;
                node.ast = Some(module.ast.clone());
                // Merge interfaces: Shape exports override native ones
                for (name, sym) in shape_interface.exports {
                    node.interface.exports.insert(name, sym);
                }
                // Hybrid modules need their Shape source dependencies walked
                builder.visited.remove(&ext.name);
            }
        }
    }

    // Step 2: Create root node.
    let root_id = builder.get_or_create_node("__root__");
    {
        let node = &mut builder.nodes[root_id.0 as usize];
        node.source_kind = ModuleSourceKind::ShapeSource;
        node.ast = Some(root_program.clone());
        node.interface = build_shape_interface(root_program);
    }

    // Step 3: Walk imports recursively.
    // Collect the root's direct imports plus prelude imports.
    let mut root_deps = extract_import_paths(root_program);
    for prelude_path in prelude_imports {
        if !root_deps.contains(prelude_path) {
            root_deps.push(prelude_path.clone());
        }
    }

    visit_module(
        "__root__",
        &root_deps,
        &mut builder,
        loader,
        extensions,
        prelude_imports,
    )?;

    // Step 4: Resolve imports per node (build ResolvedImport entries).
    // Must be done after all nodes are created so dependency lookups work.
    let node_count = builder.nodes.len();
    for i in 0..node_count {
        let ast = builder.nodes[i].ast.clone();
        if let Some(ast) = &ast {
            let resolved = resolve_imports_for_node(ast, &builder);
            builder.nodes[i].resolved_imports = resolved;

            // Also set dependencies from resolved imports
            let deps: Vec<ModuleId> = builder.nodes[i]
                .resolved_imports
                .iter()
                .map(|ri| match ri {
                    ResolvedImport::Namespace { module_id, .. } => *module_id,
                    ResolvedImport::Named { module_id, .. } => *module_id,
                })
                .collect();
            builder.nodes[i].dependencies = deps;
        }
    }

    // Step 5: Add prelude as synthetic low-priority imports to each module
    // node that doesn't already have explicit imports for them.
    for i in 0..node_count {
        let node_path = builder.nodes[i].canonical_path.clone();
        // Skip prelude modules themselves to avoid circular dependencies
        if node_path.starts_with("std::core::prelude")
            || prelude_imports.contains(&node_path)
        {
            continue;
        }
        for pi in structured_prelude {
            let Some(&dep_id) = builder.path_to_id.get(pi.canonical_path.as_str()) else {
                continue;
            };

            if pi.is_namespace || pi.named_symbols.is_empty() {
                // Namespace import: skip only if there is already an explicit
                // Namespace import for this path. A Named import from the same
                // module is not conflicting — it provides bare names while the
                // namespace provides qualified access.
                let has_namespace_import = builder.nodes[i].resolved_imports.iter().any(|ri| {
                    matches!(ri, ResolvedImport::Namespace { canonical_path, .. }
                        if canonical_path == &pi.canonical_path)
                });
                if has_namespace_import {
                    continue;
                }

                let local_name = pi
                    .canonical_path
                    .split("::")
                    .last()
                    .unwrap_or(&pi.canonical_path)
                    .to_string();
                builder.nodes[i]
                    .resolved_imports
                    .push(ResolvedImport::Namespace {
                        local_name,
                        canonical_path: pi.canonical_path.clone(),
                        module_id: dep_id,
                    });
            } else {
                // Named import: per-symbol merge.
                // Collect which local names already exist from explicit Named imports
                // on this node for this module path.
                let existing_names: HashSet<String> = builder.nodes[i]
                    .resolved_imports
                    .iter()
                    .filter_map(|ri| match ri {
                        ResolvedImport::Named {
                            canonical_path,
                            symbols,
                            ..
                        } if canonical_path == &pi.canonical_path => {
                            Some(symbols.iter().map(|s| s.local_name.clone()))
                        }
                        _ => None,
                    })
                    .flatten()
                    .collect();

                // If already imported as namespace, the symbols are accessible
                // via qualified names — but we still add named imports so bare
                // names resolve. Only skip symbols whose bare name already exists.

                let dep_node = &builder.nodes[dep_id.0 as usize];
                let mut symbols = Vec::new();
                for sym in &pi.named_symbols {
                    // Skip if this specific name is already imported explicitly
                    if existing_names.contains(&sym.name) {
                        continue;
                    }
                    // Resolve the export kind from the dependency's interface
                    let kind = dep_node
                        .interface
                        .exports
                        .get(&sym.name)
                        .map(|e| e.kind)
                        .unwrap_or(ModuleExportKind::Function);

                    symbols.push(NamedImportSymbol {
                        original_name: sym.name.clone(),
                        local_name: sym.name.clone(),
                        is_annotation: sym.is_annotation,
                        kind,
                    });
                }

                if !symbols.is_empty() {
                    // Check if we already have a Named import for this path to merge into
                    let existing_named_idx = builder.nodes[i]
                        .resolved_imports
                        .iter()
                        .position(|ri| matches!(ri,
                            ResolvedImport::Named { canonical_path, .. }
                            if canonical_path == &pi.canonical_path
                        ));

                    if let Some(idx) = existing_named_idx {
                        // Merge symbols into existing Named import
                        if let ResolvedImport::Named {
                            symbols: ref mut existing_symbols,
                            ..
                        } = builder.nodes[i].resolved_imports[idx]
                        {
                            existing_symbols.extend(symbols);
                        }
                    } else {
                        builder.nodes[i]
                            .resolved_imports
                            .push(ResolvedImport::Named {
                                canonical_path: pi.canonical_path.clone(),
                                module_id: dep_id,
                                symbols,
                            });
                    }
                }

                // Don't add a namespace binding for Named prelude imports.
                // A namespace binding with the last segment (e.g., "snapshot" from
                // "std::core::snapshot") would shadow the bare named symbol when
                // module_binding_name resolution runs before find_function.
            }

            if !builder.nodes[i].dependencies.contains(&dep_id) {
                builder.nodes[i].dependencies.push(dep_id);
            }
        }
    }

    // Step 6: Build final graph with topological order.
    Ok(builder.build(root_id))
}

/// Recursively visit a module's dependencies and add them to the graph.
fn visit_module(
    current_path: &str,
    dep_paths: &[String],
    builder: &mut GraphBuilder,
    loader: &mut shape_runtime::module_loader::ModuleLoader,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    prelude_imports: &[String],
) -> Result<(), GraphBuildError> {
    if !builder.begin_visit(current_path) {
        return Err(GraphBuildError::CyclicDependency {
            cycle: builder.get_cycle_path(current_path),
        });
    }

    for dep_path in dep_paths {
        // Already fully processed?
        if builder.is_visited(dep_path) {
            continue;
        }

        // Already has a node (pre-registered native module)?
        if builder.path_to_id.contains_key(dep_path.as_str()) && builder.is_visited(dep_path) {
            continue;
        }

        // Classify the module
        let kind_hint = resolve_module_source_kind(loader, dep_path);

        match kind_hint {
            ModuleSourceKindHint::NativeExtension => {
                // Should have been caught in pre-registration step.
                // If not, it's an extension module registered via loader
                // but not in the extensions list. Create a native node.
                if !builder.path_to_id.contains_key(dep_path.as_str()) {
                    let ext = extensions.iter().find(|e| e.name == *dep_path);
                    let dep_id = builder.get_or_create_node(dep_path);
                    let node = &mut builder.nodes[dep_id.0 as usize];
                    node.source_kind = ModuleSourceKind::NativeModule;
                    if let Some(ext) = ext {
                        node.interface = build_native_interface(ext);
                    }
                    builder.visited.insert(dep_path.clone());
                }
            }
            ModuleSourceKindHint::ShapeSource
            | ModuleSourceKindHint::EmbeddedStdlib => {
                // Load Shape source
                let module = loader
                    .load_module(dep_path)
                    .map_err(|e| GraphBuildError::Other {
                        message: format!(
                            "Failed to load module '{}': {}",
                            dep_path, e
                        ),
                    })?;

                let dep_id = builder.get_or_create_node(dep_path);
                let node = &mut builder.nodes[dep_id.0 as usize];

                // Check if this is also a native extension (Hybrid)
                let is_native = extensions.iter().any(|e| e.name == *dep_path);
                if is_native {
                    node.source_kind = ModuleSourceKind::Hybrid;
                    // Build merged interface
                    let shape_iface = build_shape_interface(&module.ast);
                    let native_ext = extensions.iter().find(|e| e.name == *dep_path).unwrap();
                    let mut native_iface = build_native_interface(native_ext);
                    // Shape exports take priority
                    for (name, sym) in shape_iface.exports {
                        native_iface.exports.insert(name, sym);
                    }
                    node.interface = native_iface;
                } else {
                    node.source_kind = ModuleSourceKind::ShapeSource;
                    node.interface = build_shape_interface(&module.ast);
                }
                node.ast = Some(module.ast.clone());

                // Recurse into this module's dependencies
                let mut sub_deps = extract_import_paths(&module.ast);
                // Also add prelude imports, but skip for prelude modules
                // themselves to avoid circular dependencies among them.
                if !prelude_imports.contains(dep_path) {
                    for pp in prelude_imports {
                        if !sub_deps.contains(pp) {
                            sub_deps.push(pp.clone());
                        }
                    }
                }

                visit_module(
                    dep_path,
                    &sub_deps,
                    builder,
                    loader,
                    extensions,
                    prelude_imports,
                )?;
            }
            ModuleSourceKindHint::CompiledBundle => {
                return Err(GraphBuildError::CompiledBytecodeNotSupported {
                    module_path: dep_path.clone(),
                });
            }
            ModuleSourceKindHint::NotFound => {
                // Module not found — might be a prelude module or just missing.
                // We skip silently here; the compiler will emit proper errors.
                // For prelude modules that don't resolve, this is expected
                // (e.g., they may be virtual stdlib modules handled at compile time).
            }
        }
    }

    builder.end_visit(current_path);
    Ok(())
}

/// A single symbol imported by the prelude.
#[derive(Debug, Clone)]
pub struct PreludeNamedSymbol {
    pub name: String,
    pub is_annotation: bool,
}

/// A prelude import preserving the named/namespace structure from prelude.shape.
#[derive(Debug, Clone)]
pub struct PreludeImport {
    pub canonical_path: String,
    pub named_symbols: Vec<PreludeNamedSymbol>,
    pub is_namespace: bool,
}

/// Collect structured prelude imports by loading `std::core::prelude`.
///
/// Preserves the named/namespace import structure so the graph builder
/// can generate appropriate `ResolvedImport::Named` entries (not just
/// `ResolvedImport::Namespace` for everything).
pub fn collect_prelude_imports(
    loader: &mut shape_runtime::module_loader::ModuleLoader,
) -> Vec<PreludeImport> {
    let prelude = match loader.load_module("std::core::prelude") {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let mut imports = Vec::new();
    for item in &prelude.ast.items {
        if let shape_ast::ast::Item::Import(import_stmt, _) = item {
            // Check for duplicate module path
            if imports
                .iter()
                .any(|i: &PreludeImport| i.canonical_path == import_stmt.from)
            {
                continue;
            }

            match &import_stmt.items {
                shape_ast::ast::ImportItems::Named(specs) => {
                    let symbols = specs
                        .iter()
                        .map(|spec| PreludeNamedSymbol {
                            name: spec.name.clone(),
                            is_annotation: spec.is_annotation,
                        })
                        .collect();
                    imports.push(PreludeImport {
                        canonical_path: import_stmt.from.clone(),
                        named_symbols: symbols,
                        is_namespace: false,
                    });
                }
                shape_ast::ast::ImportItems::Namespace { .. } => {
                    imports.push(PreludeImport {
                        canonical_path: import_stmt.from.clone(),
                        named_symbols: Vec::new(),
                        is_namespace: true,
                    });
                }
            }
        }
    }
    imports
}

/// Collect prelude import paths by loading `std::core::prelude`.
///
/// Returns the list of module paths that the prelude imports,
/// which should be added as synthetic imports to each module.
/// This is a thin wrapper over `collect_prelude_imports` for backward compatibility.
pub fn collect_prelude_import_paths(
    loader: &mut shape_runtime::module_loader::ModuleLoader,
) -> Vec<String> {
    collect_prelude_imports(loader)
        .into_iter()
        .map(|pi| pi.canonical_path)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_builder_basic() {
        let mut builder = GraphBuilder::new();
        let a = builder.get_or_create_node("a");
        let b = builder.get_or_create_node("b");
        let c = builder.get_or_create_node("c");

        // a depends on b, b depends on c
        builder.nodes[a.0 as usize].dependencies.push(b);
        builder.nodes[b.0 as usize].dependencies.push(c);

        let graph = builder.build(a);

        assert_eq!(graph.len(), 3);
        assert_eq!(graph.root_id(), a);
        // Topo order: c, b (root excluded)
        assert_eq!(graph.topo_order(), &[c, b]);
    }

    #[test]
    fn test_graph_builder_dedup() {
        let mut builder = GraphBuilder::new();
        let id1 = builder.get_or_create_node("std::core::math");
        let id2 = builder.get_or_create_node("std::core::math");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_cycle_detection() {
        let mut builder = GraphBuilder::new();
        assert!(builder.begin_visit("a"));
        assert!(builder.begin_visit("b"));
        assert!(builder.is_visiting("a"));
        assert!(!builder.begin_visit("a")); // cycle!
    }

    #[test]
    fn test_graph_lookup() {
        let mut builder = GraphBuilder::new();
        let math_id = builder.get_or_create_node("std::core::math");
        builder.nodes[math_id.0 as usize].source_kind = ModuleSourceKind::NativeModule;

        let graph = builder.build(math_id);
        assert_eq!(graph.id_for_path("std::core::math"), Some(math_id));
        assert_eq!(graph.id_for_path("nonexistent"), None);
        assert_eq!(
            graph.node(math_id).source_kind,
            ModuleSourceKind::NativeModule
        );
    }

    #[test]
    fn test_diamond_dependency() {
        let mut builder = GraphBuilder::new();
        let root = builder.get_or_create_node("root");
        let a = builder.get_or_create_node("a");
        let b = builder.get_or_create_node("b");
        let c = builder.get_or_create_node("c");

        // root -> a, root -> b, a -> c, b -> c
        builder.nodes[root.0 as usize].dependencies.push(a);
        builder.nodes[root.0 as usize].dependencies.push(b);
        builder.nodes[a.0 as usize].dependencies.push(c);
        builder.nodes[b.0 as usize].dependencies.push(c);

        let graph = builder.build(root);

        // c must come before a and b
        let order = graph.topo_order();
        assert_eq!(order.len(), 3); // root excluded
        let c_pos = order.iter().position(|&id| id == c).unwrap();
        let a_pos = order.iter().position(|&id| id == a).unwrap();
        let b_pos = order.iter().position(|&id| id == b).unwrap();
        assert!(c_pos < a_pos);
        assert!(c_pos < b_pos);
    }
}
