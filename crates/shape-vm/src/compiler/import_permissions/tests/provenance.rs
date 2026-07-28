use super::*;

use shape_runtime::module_exports::ModuleExports;
use shape_runtime::module_loader::{ModuleArtifactOrigin, ModuleCode, ModuleLoader};

const PROBE_PATH: &str = "std::core::origin_probe";

fn root() -> AstProgram {
    parse("use std::core::origin_probe\n0")
}

fn source(code: &'static str) -> ModuleCode {
    ModuleCode::Source(Arc::from(code))
}

fn build(loader: &mut ModuleLoader, native: bool) -> ModuleGraph {
    let extensions = if native {
        vec![ModuleExports::new(PROBE_PATH)]
    } else {
        Vec::new()
    };
    crate::module_graph::build_module_graph(&root(), loader, &extensions, &[])
        .expect("origin-probe graph builds")
}

fn probe_id(graph: &ModuleGraph) -> ModuleId {
    graph
        .id_for_path(PROBE_PATH)
        .expect("origin-probe module identity")
}

#[test]
fn hybrid_bootstrap_authority_follows_the_winning_resolver_only() {
    let mut embedded = ModuleLoader::new();
    embedded.register_embedded_stdlib_module(
        PROBE_PATH,
        source("pub fn embedded_winner() -> int { 1 }"),
    );
    let embedded_graph = build(&mut embedded, true);
    let embedded_id = probe_id(&embedded_graph);
    assert_eq!(
        embedded_graph.node(embedded_id).source_kind,
        ModuleSourceKind::Hybrid
    );
    assert!(embedded_graph.is_stdlib_bootstrap(embedded_id));
    assert!(
        embedded_graph
            .node(embedded_id)
            .interface
            .exports
            .contains_key("embedded_winner")
    );

    let mut extension = ModuleLoader::new();
    extension.register_embedded_stdlib_module(
        PROBE_PATH,
        source("pub fn embedded_loser() -> int { 1 }"),
    );
    extension.register_extension_module(
        PROBE_PATH,
        source("pub fn extension_winner() -> int { 2 }"),
    );
    let loaded = extension
        .load_module(PROBE_PATH)
        .expect("extension shadow resolves");
    assert_eq!(loaded.artifact_origin(), ModuleArtifactOrigin::Extension);
    let extension_graph = build(&mut extension, true);
    let extension_id = probe_id(&extension_graph);
    assert_eq!(
        extension_graph.node(extension_id).source_kind,
        ModuleSourceKind::Hybrid
    );
    assert!(!extension_graph.is_stdlib_bootstrap(extension_id));
    assert!(
        extension_graph
            .node(extension_id)
            .interface
            .exports
            .contains_key("extension_winner")
    );
    assert!(
        !extension_graph
            .node(extension_id)
            .interface
            .exports
            .contains_key("embedded_loser")
    );
}

#[test]
fn bundle_shadow_of_embedded_path_remains_untrusted() {
    let mut loader = ModuleLoader::new();
    loader.register_embedded_stdlib_module(
        PROBE_PATH,
        source("pub fn embedded_loser() -> int { 1 }"),
    );
    loader.register_bundle_modules(vec![(
        PROBE_PATH.to_string(),
        source("pub fn bundle_winner() -> int { 3 }"),
    )]);
    let loaded = loader.load_module(PROBE_PATH).expect("bundle shadow resolves");
    assert_eq!(loaded.artifact_origin(), ModuleArtifactOrigin::Bundle);

    let graph = build(&mut loader, false);
    let id = probe_id(&graph);
    assert!(!graph.is_stdlib_bootstrap(id));
    assert!(
        graph
            .node(id)
            .interface
            .exports
            .contains_key("bundle_winner")
    );
    assert!(
        !graph
            .node(id)
            .interface
            .exports
            .contains_key("embedded_loser")
    );
}
