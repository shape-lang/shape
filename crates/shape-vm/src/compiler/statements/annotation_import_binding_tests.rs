use super::BytecodeCompiler;
use crate::module_graph::{
    ModuleGraph, ModuleId, ModuleInterface, ModuleNode, ModuleSourceKind, NamedImportSymbol,
    ResolvedImport,
};
use shape_ast::ast::Program;
use shape_ast::module_utils::ModuleExportKind;
use std::collections::HashMap;

fn parse(source: &str) -> Program {
    shape_ast::parse_program(source).expect("annotation-import fixture parses")
}

fn root_graph(program: &Program, resolved_imports: Vec<ResolvedImport>) -> ModuleGraph {
    let root_id = ModuleId(0);
    ModuleGraph::new(
        vec![ModuleNode {
            id: root_id,
            canonical_path: "__root__".to_string(),
            source_kind: ModuleSourceKind::ShapeSource,
            ast: Some(program.clone()),
            interface: ModuleInterface::default(),
            resolved_imports,
            dependencies: Vec::new(),
        }],
        HashMap::new(),
        Vec::new(),
        root_id,
    )
}

fn annotation_import(module: &str, local: &str) -> ResolvedImport {
    ResolvedImport::Named {
        canonical_path: module.to_string(),
        module_id: ModuleId(0),
        symbols: vec![NamedImportSymbol {
            original_name: local.to_string(),
            local_name: local.to_string(),
            is_annotation: true,
            kind: ModuleExportKind::Annotation,
        }],
    }
}

#[test]
fn standalone_annotation_alias_conflict_is_order_independent_and_transactional() {
    let first = parse(
        r#"
from pkg::ok use { @ok }
from pkg::alpha use { @same }
from pkg::beta use { @same }
"#,
    );
    let reversed = parse(
        r#"
from pkg::beta use { @same }
from pkg::alpha use { @same }
from pkg::ok use { @ok }
"#,
    );

    let mut messages = Vec::new();
    for program in [&first, &reversed] {
        let mut compiler = BytecodeCompiler::new();
        let error = compiler
            .pre_register_root_annotation_imports(program)
            .expect_err("two annotation identities cannot own one local spelling");
        messages.push(error.to_string());
        assert!(
            compiler.imported_annotations.is_empty(),
            "a conflict must commit none of the otherwise-valid aliases"
        );
        assert!(
            compiler.module_scope_sources.is_empty(),
            "a conflict must commit no annotation module scopes"
        );
        assert_eq!(
            compiler.generated_symbol_query().len(),
            0,
            "alias refusal happens before any handler can publish a symbol"
        );
    }

    assert_eq!(messages[0], messages[1]);
    assert!(messages[0].contains("pkg::alpha::same"));
    assert!(messages[0].contains("pkg::beta::same"));
}

#[test]
fn exact_repeated_annotation_alias_is_idempotent() {
    let program = parse(
        r#"
from pkg::alpha use { @same }
from pkg::alpha use { @same }
"#,
    );
    let mut compiler = BytecodeCompiler::new();

    compiler
        .pre_register_root_annotation_imports(&program)
        .expect("an exact repeated import is one binding");
    compiler
        .pre_register_root_annotation_imports(&program)
        .expect("re-running the semantic prepass is also idempotent");

    assert_eq!(compiler.imported_annotations.len(), 1);
    let binding = compiler
        .imported_annotations
        .get("same")
        .expect("the exact alias is installed");
    assert_eq!(binding.original_name, "same");
    assert_eq!(binding._module_path, "pkg::alpha");
}

#[test]
fn root_local_annotation_keeps_lexical_precedence_over_imports_and_prelude() {
    let standalone = parse(
        r#"
annotation same() { targets: [type] }
from pkg::support use { @same }
"#,
    );
    let mut standalone_compiler = BytecodeCompiler::new();
    standalone_compiler
        .pre_register_root_annotation_imports(&standalone)
        .expect("local annotation makes the imported bare spelling non-vacant");
    assert!(standalone_compiler.imported_annotations.is_empty());

    let graph_program = parse("annotation same() { targets: [type] }");
    let graph = root_graph(
        &graph_program,
        vec![annotation_import("std::prelude", "same")],
    );
    let mut graph_compiler = BytecodeCompiler::new();
    graph_compiler
        .pre_register_root_graph_annotation_imports(&graph_program, &graph)
        .expect("local annotation keeps precedence over a synthetic prelude alias");
    assert!(graph_compiler.imported_annotations.is_empty());
}

#[test]
fn graph_explicit_annotation_import_wins_without_resolved_vector_order() {
    let program = parse("from pkg::support use { @same }");
    let orders = [
        vec![
            annotation_import("std::prelude", "same"),
            annotation_import("pkg::support", "same"),
        ],
        vec![
            annotation_import("pkg::support", "same"),
            annotation_import("std::prelude", "same"),
        ],
    ];

    for resolved in orders {
        let graph = root_graph(&program, resolved);
        let mut compiler = BytecodeCompiler::new();
        compiler
            .pre_register_root_graph_annotation_imports(&program, &graph)
            .expect("source-proven explicit import owns its local spelling");
        let binding = compiler
            .imported_annotations
            .get("same")
            .expect("explicit annotation alias is installed");
        assert_eq!(binding._module_path, "pkg::support");
        assert_eq!(binding.original_name, "same");
    }
}

#[test]
fn graph_synthetic_annotation_conflict_is_order_independent_and_transactional() {
    let program = parse("0");
    let orders = [
        vec![
            annotation_import("std::alpha", "same"),
            annotation_import("std::beta", "same"),
        ],
        vec![
            annotation_import("std::beta", "same"),
            annotation_import("std::alpha", "same"),
        ],
    ];

    let mut messages = Vec::new();
    for resolved in orders {
        let graph = root_graph(&program, resolved);
        let mut compiler = BytecodeCompiler::new();
        let error = compiler
            .pre_register_root_graph_annotation_imports(&program, &graph)
            .expect_err("unproven synthetic precedence must reject");
        messages.push(error.to_string());
        assert!(compiler.imported_annotations.is_empty());
        assert!(compiler.graph_namespace_map.is_empty());
        assert_eq!(compiler.generated_symbol_query().len(), 0);
    }

    assert_eq!(messages[0], messages[1]);
    assert!(messages[0].contains("std::alpha::same"));
    assert!(messages[0].contains("std::beta::same"));
}

#[test]
fn graph_namespace_prepass_registers_only_qualified_prefix() {
    let program = parse("use std::core::remote as remote");
    let graph = root_graph(
        &program,
        vec![ResolvedImport::Namespace {
            local_name: "remote".to_string(),
            canonical_path: "std::core::remote".to_string(),
            module_id: ModuleId(0),
        }],
    );
    let mut compiler = BytecodeCompiler::new();

    compiler
        .pre_register_root_graph_annotation_imports(&program, &graph)
        .expect("qualified namespace scope stages without runtime emission");

    assert_eq!(
        compiler
            .graph_namespace_map
            .get("remote")
            .map(String::as_str),
        Some("std::core::remote")
    );
    assert!(
        compiler.imported_annotations.is_empty(),
        "namespace imports must not sweep exported annotations into bare aliases"
    );
    assert!(
        compiler.module_bindings.is_empty(),
        "the semantic prepass must not emit or allocate runtime namespace bindings"
    );
}
