use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use crate::executor::{PermissionError, VMConfig, VirtualMachine};
use crate::module_graph::{ModuleInterface, ModuleNode, ModuleSourceKind, NamedImportSymbol};
use shape_abi_v1::{Permission, PermissionSet};
use shape_ast::ast::Program as AstProgram;
use shape_ast::module_utils::ModuleExportKind;

mod denial;
mod integration;
mod provenance;

fn parse(source: &str) -> AstProgram {
    shape_ast::parse_program(source).expect("permission-carrier fixture parses")
}

fn node(
    id: ModuleId,
    canonical_path: &str,
    source_kind: ModuleSourceKind,
    ast: Option<AstProgram>,
    resolved_imports: Vec<ResolvedImport>,
    dependencies: Vec<ModuleId>,
) -> ModuleNode {
    ModuleNode {
        id,
        canonical_path: canonical_path.to_string(),
        source_kind,
        ast,
        interface: ModuleInterface::default(),
        resolved_imports,
        dependencies,
    }
}

fn graph(nodes: Vec<ModuleNode>, topo_order: Vec<ModuleId>, root_id: ModuleId) -> ModuleGraph {
    let path_to_id = nodes
        .iter()
        .map(|node| (node.canonical_path.clone(), node.id))
        .collect::<HashMap<_, _>>();
    ModuleGraph::new(nodes, path_to_id, topo_order, root_id)
}

fn capability_import(module_id: ModuleId, function: &str) -> ResolvedImport {
    ResolvedImport::Named {
        canonical_path: "std::core::file".to_string(),
        module_id,
        symbols: vec![NamedImportSymbol {
            original_name: function.to_string(),
            local_name: function.to_string(),
            is_annotation: false,
            kind: ModuleExportKind::BuiltinFunction,
        }],
    }
}

fn permission_blob<'a>(
    program: &'a crate::bytecode::Program,
    module_path: &str,
) -> &'a crate::bytecode::FunctionBlob {
    let name = BytecodeCompiler::module_permission_blob_name(module_path);
    program
        .function_store
        .values()
        .find(|blob| blob.name == name)
        .expect("authenticated module-permission carrier")
}

#[test]
fn annotation_only_dependency_carrier_is_hashed_linked_and_refused_at_load() {
    let root_id = ModuleId(0);
    let file_id = ModuleId(1);
    let dependency_id = ModuleId(2);
    let root = parse("0");
    let dependency = parse("pub annotation marker() on type { }");
    let graph = graph(
        vec![
            node(
                root_id,
                "__root__",
                ModuleSourceKind::ShapeSource,
                Some(root.clone()),
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
                "pkg::annotation_only",
                ModuleSourceKind::ShapeSource,
                Some(dependency),
                vec![capability_import(file_id, "read_text")],
                vec![file_id],
            ),
        ],
        vec![file_id, dependency_id],
        root_id,
    );
    let expected = PermissionSet::from([Permission::FsRead]);
    let mut compiler = BytecodeCompiler::new();
    compiler.set_permission_set(Some(expected.clone()));

    let bytecode = compiler
        .compile_with_graph(&root, Arc::new(graph))
        .expect("granted annotation-only graph compiles");
    let content_addressed = bytecode
        .content_addressed
        .expect("graph compilation produces a content-addressed program");
    let carrier = permission_blob(&content_addressed, "pkg::annotation_only");

    assert_eq!(carrier.required_permissions, expected);
    assert!(
        carrier.instructions.is_empty(),
        "carrier code must be empty"
    );
    assert!(carrier.constants.is_empty());
    assert!(carrier.strings.is_empty());
    assert!(carrier.dependencies.is_empty());
    assert_eq!(carrier.content_hash, carrier.compute_hash());
    assert!(
        content_addressed
            .function_store
            .contains_key(&carrier.content_hash),
        "the recomputable carrier hash must authenticate store membership"
    );
    let main = content_addressed
        .function_store
        .get(&content_addressed.entry)
        .expect("entry resolves to __main__");
    assert_eq!(main.required_permissions, PermissionSet::pure());

    let linked = crate::linker::link(&content_addressed).expect("carrier graph links");
    assert_eq!(linked.total_required_permissions, expected);
    let mut vm = VirtualMachine::new(VMConfig::default());
    let error = vm
        .load_linked_program_with_permissions(linked, &PermissionSet::pure())
        .expect_err("a pure receiver refuses the dependency carrier at load");
    match error {
        PermissionError::InsufficientPermissions { missing, .. } => {
            assert_eq!(missing, PermissionSet::from([Permission::FsRead]));
        }
        other => panic!("expected InsufficientPermissions, got {other:?}"),
    }
}

#[test]
fn module_id_pending_state_is_sibling_isolated_hash_sensitive_and_no_overwrite() {
    let root_id = ModuleId(0);
    let file_id = ModuleId(1);
    let reader_id = ModuleId(2);
    let writer_id = ModuleId(3);
    let graph = graph(
        vec![
            node(
                root_id,
                "__root__",
                ModuleSourceKind::ShapeSource,
                Some(parse("0")),
                Vec::new(),
                vec![reader_id, writer_id],
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
                reader_id,
                "pkg::reader",
                ModuleSourceKind::ShapeSource,
                Some(parse("0")),
                vec![capability_import(file_id, "read_text")],
                vec![file_id],
            ),
            node(
                writer_id,
                "pkg::writer",
                ModuleSourceKind::ShapeSource,
                Some(parse("0")),
                vec![capability_import(file_id, "write_text")],
                vec![file_id],
            ),
        ],
        vec![file_id, reader_id, writer_id],
        root_id,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler.set_permission_set(Some(PermissionSet::full()));

    for module_id in [reader_id, writer_id] {
        let token = compiler
            .enter_graph_permission_owner(module_id, &graph)
            .expect("module owns its permission derivation");
        let imports = graph.node(module_id).resolved_imports.clone();
        compiler
            .authorize_and_stage_graph_import_permissions(module_id, &graph, &imports)
            .expect("authorized imports stage by ModuleId");
        assert_eq!(compiler.pending_module_permission_count(), 1);
        assert!(
            compiler.current_blob_builder.is_none(),
            "staged permissions must not hold a function builder open"
        );
        compiler
            .complete_graph_import_permissions(module_id, &graph)
            .expect("module close publishes one zero-code carrier");
        assert_eq!(compiler.pending_module_permission_count(), 0);
        assert!(compiler.current_blob_builder.is_none());
        compiler
            .leave_graph_permission_owner(token)
            .expect("exact owner leaves");
    }

    let reader_name = BytecodeCompiler::module_permission_blob_name("pkg::reader");
    let writer_name = BytecodeCompiler::module_permission_blob_name("pkg::writer");
    let reader = compiler
        .completed_blobs
        .iter()
        .find(|blob| blob.name == reader_name)
        .expect("reader carrier");
    let writer = compiler
        .completed_blobs
        .iter()
        .find(|blob| blob.name == writer_name)
        .expect("writer carrier");
    assert_eq!(
        reader.required_permissions,
        PermissionSet::from([Permission::FsRead])
    );
    assert_eq!(
        writer.required_permissions,
        PermissionSet::from([Permission::FsWrite])
    );
    assert_eq!(reader.content_hash, reader.compute_hash());
    assert_eq!(writer.content_hash, writer.compute_hash());
    let mut permission_tamper = reader.clone();
    permission_tamper.required_permissions = PermissionSet::from([Permission::FsWrite]);
    assert_ne!(permission_tamper.compute_hash(), reader.content_hash);

    let completed_before_duplicate = compiler.completed_blobs.len();
    let token = compiler
        .enter_graph_permission_owner(reader_id, &graph)
        .expect("reader re-entry is typed");
    let imports = graph.node(reader_id).resolved_imports.clone();
    compiler
        .authorize_and_stage_graph_import_permissions(reader_id, &graph, &imports)
        .expect("authorization itself remains valid");
    let error = compiler
        .complete_graph_import_permissions(reader_id, &graph)
        .expect_err("an existing carrier name cannot be overwritten");
    assert!(error.to_string().contains("duplicate authenticated"));
    assert_eq!(compiler.pending_module_permission_count(), 0);
    assert_eq!(compiler.completed_blobs.len(), completed_before_duplicate);
    compiler
        .leave_graph_permission_owner(token)
        .expect("duplicate refusal preserves owner state");
}

#[test]
fn denied_dependency_import_publishes_no_pending_blob_symbol_or_instruction() {
    let root_id = ModuleId(0);
    let file_id = ModuleId(1);
    let dependency_id = ModuleId(2);
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
                "pkg::denied",
                ModuleSourceKind::ShapeSource,
                Some(parse("0")),
                vec![capability_import(file_id, "read_text")],
                vec![file_id],
            ),
        ],
        vec![file_id, dependency_id],
        root_id,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler.set_permission_set(Some(PermissionSet::pure()));
    let token = compiler
        .enter_graph_permission_owner(dependency_id, &graph)
        .expect("dependency owner enters");

    let imports = graph.node(dependency_id).resolved_imports.clone();
    let error = compiler
        .authorize_and_stage_graph_import_permissions(dependency_id, &graph, &imports)
        .expect_err("fs.read is refused at the registrar's first mutation seam");
    let message = error.to_string();
    assert!(message.contains("Permission denied"), "got: {message}");
    assert!(message.contains("fs.read"), "got: {message}");
    assert_eq!(compiler.pending_module_permission_count(), 0);
    assert!(compiler.completed_blobs.is_empty());
    assert!(compiler.current_blob_builder.is_none());
    assert!(compiler.imported_names.is_empty());
    assert!(compiler.imported_annotations.is_empty());
    assert!(compiler.module_bindings.is_empty());
    assert!(compiler.program.instructions.is_empty());
    compiler
        .discard_graph_import_permissions(dependency_id, &graph)
        .expect("pre-staging refusal has nothing to discard");
    compiler
        .leave_graph_permission_owner(token)
        .expect("denial preserves exact owner cleanup");
}

#[test]
fn only_embedded_resolver_provenance_suppresses_bootstrap_call_stamping() {
    let root = parse("use std::core::bootstrap_probe\n0");
    let mut loader = shape_runtime::module_loader::ModuleLoader::new();
    loader.register_embedded_stdlib_module(
        "std::core::bootstrap_probe",
        shape_runtime::module_loader::ModuleCode::Source(Arc::from("pub fn probe() -> int { 1 }")),
    );
    let authentic = crate::module_graph::build_module_graph(&root, &mut loader, &[], &[])
        .expect("embedded module graph builds");
    let authentic_id = authentic
        .id_for_path("std::core::bootstrap_probe")
        .expect("embedded module identity");
    assert!(authentic.is_stdlib_bootstrap(authentic_id));

    let root_id = ModuleId(0);
    let forged_id = ModuleId(1);
    let forged = graph(
        vec![
            node(
                root_id,
                "__root__",
                ModuleSourceKind::ShapeSource,
                Some(parse("0")),
                Vec::new(),
                vec![forged_id],
            ),
            node(
                forged_id,
                "std::core::bootstrap_probe",
                ModuleSourceKind::ShapeSource,
                Some(parse("0")),
                Vec::new(),
                Vec::new(),
            ),
        ],
        vec![forged_id],
        root_id,
    );
    assert!(!forged.is_stdlib_bootstrap(forged_id));

    let stamped = |graph: &ModuleGraph, module_id: ModuleId| {
        let mut compiler = BytecodeCompiler::new();
        compiler.current_blob_builder = Some(FunctionBlobBuilder::new(
            "owner".to_string(),
            compiler.program.current_offset(),
            compiler.program.constants.len(),
            compiler.program.strings.len(),
        ));
        let token = compiler
            .enter_graph_permission_owner(module_id, graph)
            .expect("typed graph owner enters");
        compiler.record_owned_capability_call_permissions("std::core::file", "read_text");
        let permissions = compiler
            .current_blob_builder
            .as_ref()
            .expect("test owner builder")
            .required_permissions
            .clone();
        compiler
            .leave_graph_permission_owner(token)
            .expect("typed graph owner leaves");
        permissions
    };

    assert_eq!(stamped(&authentic, authentic_id), PermissionSet::pure());
    assert_eq!(
        stamped(&forged, forged_id),
        PermissionSet::from([Permission::FsRead]),
        "a user-chosen std spelling is not bootstrap authority"
    );
}
