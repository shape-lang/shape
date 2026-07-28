use super::BytecodeCompiler;
use crate::bytecode::CompiledAnnotation;
use crate::module_graph::{
    ModuleGraph, ModuleId, ModuleInterface, ModuleNode, ModuleSourceKind, NamedImportSymbol,
    ResolvedImport,
};
use shape_ast::ast::{AnnotationDef, AnnotationHandlerType, ExportItem, Item, Program};
use shape_ast::module_utils::ModuleExportKind;
use std::collections::HashMap;

fn parse(source: &str) -> Program {
    shape_ast::parse_program(source).expect("annotation-import pipeline fixture parses")
}

fn annotation_definition(source: &str) -> AnnotationDef {
    parse(source)
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::AnnotationDef(definition, _) => Some(definition),
            Item::Export(export, _) => match export.item {
                ExportItem::Annotation(definition) => Some(definition),
                _ => None,
            },
            _ => None,
        })
        .expect("fixture defines one annotation")
}

fn install_compiled_annotation(
    compiler: &mut BytecodeCompiler,
    exact_name: &str,
    definition: &AnnotationDef,
) {
    compiler.program.compiled_annotations.insert(
        exact_name.to_string(),
        CompiledAnnotation {
            name: exact_name.to_string(),
            param_names: definition
                .params
                .iter()
                .flat_map(|parameter| parameter.get_identifiers())
                .collect(),
            param_defs: definition.params.clone(),
            on_define_handler: None,
            metadata_handler: None,
            comptime_pre_handler: definition
                .handlers
                .iter()
                .find(|handler| handler.handler_type == AnnotationHandlerType::ComptimePre)
                .cloned(),
            comptime_post_handler: definition
                .handlers
                .iter()
                .find(|handler| handler.handler_type == AnnotationHandlerType::ComptimePost)
                .cloned(),
            sugar_post_handler: None,
            sugar_body_fns: Vec::new(),
            allowed_targets: definition.allowed_targets.clone().unwrap_or_default(),
        },
    );
}

fn annotation_import(module: &str, module_id: ModuleId, local_name: &str) -> ResolvedImport {
    ResolvedImport::Named {
        canonical_path: module.to_string(),
        module_id,
        symbols: vec![NamedImportSymbol {
            original_name: local_name.to_string(),
            local_name: local_name.to_string(),
            is_annotation: true,
            kind: ModuleExportKind::Annotation,
        }],
    }
}

#[test]
fn local_annotation_shadows_two_imports_through_the_full_pipeline() {
    let program = parse(
        r#"
from pkg::alpha use { @same }
from pkg::beta use { @same }

annotation same() on type {
  comptime post(target, ctx) {
    extend (extend_method_literal(target.name, "answer", "int", 42))
  }
}

@same()
type Probe { id: int }
"#,
    );
    let mut compiler = BytecodeCompiler::new();

    compiler
        .compile_in_place(&program)
        .expect("one local annotation shadows both imported spellings");

    assert!(
        compiler.imported_annotations.get("same").is_none(),
        "the late import pass must consume the local-shadow decision, not reinsert an alias"
    );
    assert_eq!(
        compiler.generated_symbol_query().len(),
        1,
        "the local handler must materialize exactly one generated declaration"
    );
}

#[test]
fn graph_local_shadow_tombstone_survives_explicit_and_synthetic_rows_in_any_order() {
    let root_id = ModuleId(0);
    let explicit_id = ModuleId(1);
    let synthetic_id = ModuleId(2);
    let root = parse(
        r#"
from pkg::explicit use { @same }
@same()
type Probe { id: int }

annotation same() on type {
  comptime post(target, ctx) {
    extend (extend_method_literal(target.name, "answer", "int", 42))
  }
}
"#,
    );
    let explicit = parse(
        r#"
pub annotation same() on type {
  comptime post(target, ctx) { error("EXPLICIT_HANDLER_MUST_NOT_RUN") }
}
"#,
    );
    let synthetic = parse(
        r#"
pub annotation same() on type {
  comptime post(target, ctx) { error("SYNTHETIC_HANDLER_MUST_NOT_RUN") }
}
"#,
    );
    let orders = [
        vec![
            annotation_import("pkg::explicit", explicit_id, "same"),
            annotation_import("std::prelude", synthetic_id, "same"),
        ],
        vec![
            annotation_import("std::prelude", synthetic_id, "same"),
            annotation_import("pkg::explicit", explicit_id, "same"),
        ],
    ];

    for resolved_imports in orders {
        let graph = ModuleGraph::new(
            vec![
                ModuleNode {
                    id: root_id,
                    canonical_path: "__root__".to_string(),
                    source_kind: ModuleSourceKind::ShapeSource,
                    ast: Some(root.clone()),
                    interface: ModuleInterface::default(),
                    resolved_imports,
                    dependencies: vec![explicit_id, synthetic_id],
                },
                ModuleNode {
                    id: explicit_id,
                    canonical_path: "pkg::explicit".to_string(),
                    source_kind: ModuleSourceKind::ShapeSource,
                    ast: Some(explicit.clone()),
                    interface: ModuleInterface::default(),
                    resolved_imports: Vec::new(),
                    dependencies: Vec::new(),
                },
                ModuleNode {
                    id: synthetic_id,
                    canonical_path: "std::prelude".to_string(),
                    source_kind: ModuleSourceKind::ShapeSource,
                    ast: Some(synthetic.clone()),
                    interface: ModuleInterface::default(),
                    resolved_imports: Vec::new(),
                    dependencies: Vec::new(),
                },
            ],
            HashMap::from([
                ("__root__".to_string(), root_id),
                ("pkg::explicit".to_string(), explicit_id),
                ("std::prelude".to_string(), synthetic_id),
            ]),
            vec![explicit_id, synthetic_id],
            root_id,
        );
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_with_graph_and_prelude_in_place(&root, std::sync::Arc::new(graph), &[])
            .expect("the local tombstone prevents both remote handlers from running");
        assert!(
            compiler.imported_annotations.get("same").is_none(),
            "the completed graph pipeline must retain the local-shadow tombstone"
        );
        assert_eq!(
            compiler
                .program
                .compiled_annotations
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(
                ["pkg::explicit::same", "same", "std::prelude::same",]
            ),
            "all three exact declaration carriers must survive without alias publication"
        );
        assert_eq!(
            compiler.generated_symbol_query().len(),
            1,
            "only the local handler may generate a declaration"
        );
        assert!(
            compiler
                .program
                .functions
                .iter()
                .any(|function| function.name == "Probe.answer"),
            "the local handler must remain authoritative in either resolved order"
        );
    }
}

#[test]
fn dependency_namespace_cannot_replace_root_namespace_before_materialization() {
    let dir = tempfile::tempdir().expect("temp module tree");
    let package = dir.path().join("pkg");
    std::fs::create_dir_all(&package).expect("package directory");
    std::fs::write(
        package.join("root_support.shape"),
        r#"
pub annotation mark() on type {
  comptime post(target, ctx) { error("ROOT_NAMESPACE_HANDLER") }
}
"#,
    )
    .expect("root support module");
    std::fs::write(
        package.join("dep_support.shape"),
        r#"
pub annotation mark() on type {
  comptime post(target, ctx) { error("DEPENDENCY_NAMESPACE_HANDLER") }
}
"#,
    )
    .expect("dependency support module");
    std::fs::write(
        package.join("consumer.shape"),
        r#"
use pkg::dep_support as shared
pub fn touch() -> int { 0 }
"#,
    )
    .expect("dependency consumer module");

    let root = parse(
        r#"
from pkg::consumer use { touch }
use pkg::root_support as shared
@shared::mark()
type Probe { id: int }
touch()
"#,
    );
    let mut loader = shape_runtime::module_loader::ModuleLoader::new();
    loader.add_module_path(dir.path().to_path_buf());
    let (graph, stdlib_names, prelude_imports) =
        crate::module_resolution::build_graph_and_stdlib_names(&root, &mut loader, &[])
            .expect("graph resolves both same-spelled namespace prefixes");
    let mut compiler = BytecodeCompiler::new();
    compiler.stdlib_function_names = stdlib_names;

    let error = compiler
        .compile_with_graph_and_prelude(&root, graph, &prelude_imports)
        .expect_err("the root-selected handler deliberately refuses");
    let message = error.to_string();
    assert!(message.contains("ROOT_NAMESPACE_HANDLER"), "got: {message}");
    assert!(
        !message.contains("DEPENDENCY_NAMESPACE_HANDLER"),
        "dependency-phase namespace state must not select the root handler: {message}"
    );
}

#[test]
fn denied_annotation_import_stops_before_handler_publication() {
    let program = parse(
        r#"
from std::core::file use { @read_text }
@read_text()
type Probe { id: int }
"#,
    );
    let definition = annotation_definition(
        r#"
annotation read_text() on type {
  comptime post(target, ctx) {
    extend (item_fn("forbidden_generated", "int", 1))
  }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    install_compiled_annotation(&mut compiler, "std::core::file::read_text", &definition);
    compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::pure()));

    let error = compiler
        .compile_in_place(&program)
        .expect_err("fs.read must be denied before handler discovery");
    let message = error.to_string();
    assert!(message.contains("Permission denied"), "got: {message}");
    assert!(message.contains("fs.read"), "got: {message}");
    assert_eq!(
        compiler.generated_symbol_query().len(),
        0,
        "a denied import cannot execute its installed generating handler"
    );
}

#[test]
fn allowed_graph_annotation_import_records_exact_permission_on_main_blob() {
    let root_id = ModuleId(0);
    let file_id = ModuleId(1);
    let root = parse("from std::core::file use { @read_text }\n0");
    let file = parse("pub annotation read_text() on type { }");
    let graph = ModuleGraph::new(
        vec![
            ModuleNode {
                id: root_id,
                canonical_path: "__root__".to_string(),
                source_kind: ModuleSourceKind::ShapeSource,
                ast: Some(root.clone()),
                interface: ModuleInterface::default(),
                resolved_imports: vec![annotation_import("std::core::file", file_id, "read_text")],
                dependencies: vec![file_id],
            },
            ModuleNode {
                id: file_id,
                canonical_path: "std::core::file".to_string(),
                source_kind: ModuleSourceKind::ShapeSource,
                ast: Some(file),
                interface: ModuleInterface::default(),
                resolved_imports: Vec::new(),
                dependencies: Vec::new(),
            },
        ],
        HashMap::from([
            ("__root__".to_string(), root_id),
            ("std::core::file".to_string(), file_id),
        ]),
        vec![file_id],
        root_id,
    );
    let expected = shape_abi_v1::PermissionSet::from([shape_abi_v1::Permission::FsRead]);
    let mut compiler = BytecodeCompiler::new();
    compiler.set_permission_set(Some(expected.clone()));

    let bytecode = compiler
        .compile_with_graph(&root, std::sync::Arc::new(graph))
        .expect("the granted graph annotation import compiles");
    let content_addressed = bytecode
        .content_addressed
        .expect("compilation produces content-addressed blobs");
    let main = content_addressed
        .function_store
        .get(&content_addressed.entry)
        .expect("entry identifies the __main__ blob");
    assert_eq!(main.required_permissions, expected);
}
