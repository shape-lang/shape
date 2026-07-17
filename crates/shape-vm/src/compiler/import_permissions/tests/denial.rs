use super::*;

use crate::bytecode::CompiledAnnotation;
use shape_ast::ast::{AnnotationDef, AnnotationHandlerType, ExportItem, Item};

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

fn generating_annotation_import(file_id: ModuleId) -> ResolvedImport {
    ResolvedImport::Named {
        canonical_path: "std::core::file".to_string(),
        module_id: file_id,
        symbols: vec![NamedImportSymbol {
            original_name: "read_text".to_string(),
            local_name: "read_text".to_string(),
            is_annotation: true,
            kind: ModuleExportKind::Annotation,
        }],
    }
}

#[test]
fn denied_generating_annotation_restores_borrowed_dependency_lifecycle() {
    let root_id = ModuleId(0);
    let file_id = ModuleId(1);
    let dependency_id = ModuleId(2);
    let dependency = parse(
        r#"
from std::core::file use { @read_text }
@read_text()
type Probe { id: int }
"#,
    );
    let graph = graph(
        vec![
            node(
                root_id,
                "__root__",
                ModuleSourceKind::ShapeSource,
                Some(parse("0")),
                Vec::new(),
                vec![dependency_id],
            ),
            node(
                file_id,
                "std::core::file",
                ModuleSourceKind::NativeModule,
                None,
                Vec::new(),
                Vec::new(),
            ),
            node(
                dependency_id,
                "pkg::denied_annotation",
                ModuleSourceKind::ShapeSource,
                Some(dependency),
                vec![generating_annotation_import(file_id)],
                vec![file_id],
            ),
        ],
        // Compile only the borrowing module so no earlier native carrier can
        // obscure the denial's zero-publication proof.
        vec![dependency_id],
        root_id,
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
    install_compiled_annotation(
        &mut compiler,
        "std::core::file::read_text",
        &definition,
    );
    compiler.set_permission_set(Some(PermissionSet::pure()));

    let error = compiler
        .compile_graph_dependencies_for_permission_test(&graph)
        .expect_err("denied annotation import refuses before borrowed state staging");
    let message = error.to_string();
    assert!(message.contains("Permission denied"), "got: {message}");
    assert!(message.contains("fs.read"), "got: {message}");
    assert_eq!(compiler.pending_module_permission_count(), 0);
    assert!(compiler.graph_permission_state.active_owner.is_none());
    assert!(compiler.completed_blobs.is_empty());
    assert!(compiler.blob_name_to_hash.is_empty());
    assert!(compiler.current_blob_builder.is_none());
    assert!(compiler.imported_names.is_empty());
    assert!(compiler.imported_annotations.is_empty());
    assert!(compiler.module_builtin_functions.is_empty());
    assert!(compiler.module_scope_sources.is_empty());
    assert!(compiler.graph_namespace_map.is_empty());
    assert!(compiler.module_bindings.is_empty());
    assert!(compiler.module_scope_stack.is_empty());
    assert!(compiler.program.instructions.is_empty());
    assert_eq!(compiler.generated_symbol_query().len(), 0);
    assert!(
        compiler
            .program
            .compiled_annotations
            .contains_key("std::core::file::read_text"),
        "the installed generating handler makes the denial fixture non-vacuous"
    );
}
