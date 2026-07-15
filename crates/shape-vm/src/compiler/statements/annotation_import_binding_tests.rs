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
            before_handler: None,
            after_handler: None,
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
            before_handler_template: None,
            after_handler_template: None,
            allowed_targets: definition.allowed_targets.clone().unwrap_or_default(),
        },
    );
}

#[test]
fn standalone_annotation_alias_conflict_is_order_independent_and_transactional() {
    let first = parse(
        r#"
from ./ok use { @ok }
from ./alpha use { @same }
from ./beta use { @same }
"#,
    );
    let reversed = parse(
        r#"
from ./beta use { @same }
from ./alpha use { @same }
from ./ok use { @ok }
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
    assert!(messages[0].contains("./alpha::same"));
    assert!(messages[0].contains("./beta::same"));
}

#[test]
fn exact_repeated_annotation_alias_is_idempotent() {
    let program = parse(
        r#"
from ./alpha use { @same }
from ./alpha use { @same }
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
    assert_eq!(binding._module_path, "./alpha");
}

#[test]
fn root_local_annotation_keeps_lexical_precedence_over_imports_and_prelude() {
    let standalone = parse(
        r#"
annotation same() { targets: [type] }
from ./support use { @same }
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
    let program = parse("from ./support use { @same }");
    let orders = [
        vec![
            annotation_import("std::prelude", "same"),
            annotation_import("./support", "same"),
        ],
        vec![
            annotation_import("./support", "same"),
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
        assert_eq!(binding._module_path, "./support");
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

#[test]
fn local_annotation_shadows_two_imports_through_the_full_pipeline() {
    let program = parse(
        r#"
from ./alpha use { @same }
from ./beta use { @same }

annotation same() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ 42 \} \}")
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
fn dependency_namespace_cannot_replace_root_namespace_before_materialization() {
    let dir = tempfile::tempdir().expect("temp module tree");
    let package = dir.path().join("pkg");
    std::fs::create_dir_all(&package).expect("package directory");
    std::fs::write(
        package.join("root_support.shape"),
        r#"
pub annotation mark() {
  targets: [type]
  comptime post(target, ctx) { error("ROOT_NAMESPACE_HANDLER") }
}
"#,
    )
    .expect("root support module");
    std::fs::write(
        package.join("dep_support.shape"),
        r#"
pub annotation mark() {
  targets: [type]
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
annotation read_text() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn forbidden_generated() -> int { 1 }")
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
    let file = parse("pub annotation read_text() { targets: [type] }");
    let graph = ModuleGraph::new(
        vec![
            ModuleNode {
                id: root_id,
                canonical_path: "__root__".to_string(),
                source_kind: ModuleSourceKind::ShapeSource,
                ast: Some(root.clone()),
                interface: ModuleInterface::default(),
                resolved_imports: vec![ResolvedImport::Named {
                    canonical_path: "std::core::file".to_string(),
                    module_id: file_id,
                    symbols: vec![NamedImportSymbol {
                        original_name: "read_text".to_string(),
                        local_name: "read_text".to_string(),
                        is_annotation: true,
                        kind: ModuleExportKind::Annotation,
                    }],
                }],
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
