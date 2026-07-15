use super::*;
use crate::module_graph::{
    ModuleGraph, ModuleId, ModuleInterface, ModuleNode, ModuleSourceKind, NamedImportSymbol,
    ResolvedImport,
};
use shape_ast::module_utils::ModuleExportKind;
use std::collections::HashMap;

#[test]
fn metadata_and_on_define_ids_survive_nested_closure_registration() {
    for (kind, suffix) in [("metadata", "metadata"), ("on_define", "on_define")] {
        let program = parse(&format!(
            r#"
annotation nested() {{
  {kind}(target) {{
    let identity = |value| value
    identity(target)
  }}
}}
"#,
        ));
        let mut compiler = BytecodeCompiler::new();
        compiler
            .prepare_annotation_scope(&program.items)
            .expect("lifecycle handler with closure installs");
        let carrier = compiler
            .program
            .compiled_annotations
            .get("nested")
            .expect("carrier published");
        let handler_id = usize::from(match kind {
            "metadata" => carrier.metadata_handler,
            "on_define" => carrier.on_define_handler,
            _ => unreachable!(),
        }
        .expect("handler id"));
        let handler_name = format!("nested___{suffix}");
        assert_eq!(compiler.program.functions[handler_id].name, handler_name);
        assert!(
            compiler
                .program
                .functions
                .iter()
                .skip(handler_id + 1)
                .any(|function| function.is_closure),
            "closure rows may follow the reserved handler row without stealing its id"
        );
        let blobs = compiler
            .completed_blobs
            .iter()
            .filter(|blob| blob.name == handler_name)
            .collect::<Vec<_>>();
        assert_eq!(blobs.len(), 1, "one exact lifecycle-handler blob");
        let hash = compiler.function_hashes_by_id[handler_id].expect("handler hash");
        assert_eq!(blobs[0].content_hash, hash);
        assert_eq!(compiler.blob_name_to_hash.get(&handler_name), Some(&hash));
    }
}

#[test]
fn bare_and_qualified_callable_collisions_refuse_before_installation() {
    for module_path in [None, Some("pkg")] {
        let mut compiler = BytecodeCompiler::new();
        let occupied = parse("fn blocked___metadata() -> int { 1 }");
        let Item::Function(mut function, _) = occupied.items[0].clone() else {
            panic!("fixture function")
        };
        let callable_name = module_path
            .map(|module| format!("{}::blocked___metadata", module))
            .unwrap_or_else(|| "blocked___metadata".to_string());
        function.name = callable_name.clone();
        compiler
            .register_function(&function)
            .expect("occupy callable name");
        let definition = parse("annotation blocked() { metadata(target) { 1 } }");
        let items = if let Some(module) = module_path {
            vec![compiler
                .qualify_module_item(&definition.items[0], module)
                .expect("definition qualifies")]
        } else {
            vec![definition.items[0].clone()]
        };
        let before = compiler.program.functions.len();
        let error = compiler
            .prepare_annotation_scope(&items)
            .expect_err("planner must reject every occupied exact callable name");
        assert!(error.to_string().contains(&callable_name));
        assert_eq!(compiler.program.functions.len(), before);
        assert!(compiler.program.compiled_annotations.is_empty());
    }
}

#[test]
fn root_imported_callable_alias_blocks_handler_name_but_type_alias_does_not() {
    for (kind, must_block) in [
        (ModuleExportKind::Function, true),
        (ModuleExportKind::TypeAlias, false),
    ] {
        let graph = root_import_graph(kind);
        let mut compiler = BytecodeCompiler::new();
        compiler.current_blob_builder = Some(crate::compiler::FunctionBlobBuilder::new(
            "__main__".to_string(),
            0,
            0,
            0,
        ));
        compiler
            .register_graph_imports_for_module(ModuleId(0), &graph)
            .expect("resolved root import publishes its typed alias");
        let imported = compiler
            .imported_names
            .get("blocked___metadata")
            .expect("exact local alias");
        assert_eq!(imported.kind, Some(kind));

        let definition = parse("annotation blocked() { metadata(target) { return 1 } }");
        let result = compiler.prepare_annotation_scope(&definition.items);
        if must_block {
            let error = result.expect_err("callable import alias occupies the handler name");
            assert!(error.to_string().contains("blocked___metadata"));
            assert!(compiler.program.compiled_annotations.is_empty());
        } else {
            result.expect("a type-only imported alias is a separate namespace");
            assert!(compiler.program.compiled_annotations.contains_key("blocked"));
        }
    }
}

fn root_import_graph(kind: ModuleExportKind) -> ModuleGraph {
    let root_id = ModuleId(0);
    let dependency_id = ModuleId(1);
    let dependency = match kind {
        ModuleExportKind::Function => parse("pub fn helper() -> int { 1 }"),
        ModuleExportKind::TypeAlias => parse("pub type helper = int;"),
        _ => unreachable!(),
    };
    ModuleGraph::new(
        vec![
            ModuleNode {
                id: root_id,
                canonical_path: "__root__".to_string(),
                source_kind: ModuleSourceKind::ShapeSource,
                ast: Some(parse("0")),
                interface: ModuleInterface::default(),
                resolved_imports: vec![ResolvedImport::Named {
                    canonical_path: "pkg::dep".to_string(),
                    module_id: dependency_id,
                    symbols: vec![NamedImportSymbol {
                        original_name: "helper".to_string(),
                        local_name: "blocked___metadata".to_string(),
                        is_annotation: false,
                        kind,
                    }],
                }],
                dependencies: vec![dependency_id],
            },
            ModuleNode {
                id: dependency_id,
                canonical_path: "pkg::dep".to_string(),
                source_kind: ModuleSourceKind::ShapeSource,
                ast: Some(dependency),
                interface: ModuleInterface::default(),
                resolved_imports: Vec::new(),
                dependencies: Vec::new(),
            },
        ],
        HashMap::from([
            ("__root__".to_string(), root_id),
            ("pkg::dep".to_string(), dependency_id),
        ]),
        vec![dependency_id],
        root_id,
    )
}
