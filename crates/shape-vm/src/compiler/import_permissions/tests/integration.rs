use super::*;

fn dependency_graph(source: &str) -> (AstProgram, ModuleGraph) {
    let root_id = ModuleId(0);
    let file_id = ModuleId(1);
    let dependency_id = ModuleId(2);
    let root = parse("0");
    let dependency = parse(source);
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
                "pkg::dep",
                ModuleSourceKind::ShapeSource,
                Some(dependency),
                vec![capability_import(file_id, "read_text")],
                vec![file_id],
            ),
        ],
        vec![file_id, dependency_id],
        root_id,
    );
    (root, graph)
}

fn blob_named<'a>(
    program: &'a crate::bytecode::Program,
    name: &str,
) -> &'a crate::bytecode::FunctionBlob {
    program
        .function_store
        .values()
        .find(|blob| blob.name == name)
        .unwrap_or_else(|| panic!("finalized blob '{name}'"))
}

#[test]
fn unbound_graph_stamps_real_dependency_call_in_both_declaration_orders() {
    let sources = [
        r#"
from std::core::file use { read_text }
pub fn pure() -> int { 7 }
pub fn capability() -> int {
  read_text("/tmp/shape-permission-probe")
  1
}
"#,
        r#"
from std::core::file use { read_text }
pub fn capability() -> int {
  read_text("/tmp/shape-permission-probe")
  1
}
pub fn pure() -> int { 7 }
"#,
    ];
    let expected = PermissionSet::from([Permission::FsRead]);

    for source in sources {
        let (root, graph) = dependency_graph(source);
        let compiler = BytecodeCompiler::new()
            .with_extensions(vec![shape_runtime::stdlib::file::create_file_module()]);
        assert!(
            compiler.permission_set.is_none(),
            "fixture must exercise the default unbound policy"
        );

        let bytecode = compiler
            .compile_with_graph(&root, Arc::new(graph))
            .expect("unbound graph compiles without capability refusal");
        let content_addressed = bytecode
            .content_addressed
            .expect("graph compilation produces content-addressed blobs");
        let pure = blob_named(&content_addressed, "pkg::dep::pure");
        let capability = blob_named(&content_addressed, "pkg::dep::capability");
        let carrier = permission_blob(&content_addressed, "pkg::dep");

        assert_eq!(pure.required_permissions, PermissionSet::pure());
        assert_eq!(capability.required_permissions, expected);
        assert_eq!(carrier.required_permissions, expected);
        for blob in [pure, capability, carrier] {
            assert_eq!(blob.content_hash, blob.compute_hash());
            assert!(
                content_addressed
                    .function_store
                    .contains_key(&blob.content_hash),
                "recomputed hash must authenticate '{}' store membership",
                blob.name
            );
        }
        assert!(
            carrier.instructions.is_empty(),
            "carrier code must be empty"
        );
        assert!(carrier.constants.is_empty());
        assert!(carrier.strings.is_empty());

        let main = content_addressed
            .function_store
            .get(&content_addressed.entry)
            .expect("entry resolves to __main__");
        assert_eq!(main.required_permissions, PermissionSet::pure());
        let linked = crate::linker::link(&content_addressed).expect("dependency graph links");
        assert_eq!(linked.total_required_permissions, expected);
        let mut vm = VirtualMachine::new(VMConfig::default());
        let error = vm
            .load_linked_program_with_permissions(linked, &PermissionSet::pure())
            .expect_err("pure receiver refuses unbound-compiled capability metadata");
        match error {
            PermissionError::InsufficientPermissions { missing, .. } => {
                assert_eq!(missing, expected);
            }
            other => panic!("expected InsufficientPermissions, got {other:?}"),
        }
    }
}
