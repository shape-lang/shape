use super::*;

#[test]
fn unresolved_import_maps_to_unavailable_not_empty_capture_query() {
    let directory = tempfile::tempdir().expect("module directory");
    let file_path = directory.path().join("main.shape");
    let source = r#"
from missing use { helper }
annotation derive() on type { }
@derive()
type Probe { id: int }
"#;
    let program = shape_ast::parse_program(source).expect("fixture parses");
    let cache = crate::module_cache::ModuleCache::new();
    let session = GeneratedQuerySession::new(
        &program,
        source,
        CaptureQueryContext {
            file_path: Some(&file_path),
            module_cache: Some(&cache),
            workspace_root: None,
        },
    );
    assert!(matches!(session, GeneratedQuerySession::Unavailable));
}

#[test]
fn poisoned_imported_annotation_maps_to_unavailable() {
    let directory = tempfile::tempdir().expect("module directory");
    let file_path = directory.path().join("main.shape");
    std::fs::write(
        directory.path().join("shape.toml"),
        "[modules]\npaths = []\n",
    )
    .expect("write project manifest");
    std::fs::write(
        directory.path().join("support.shape"),
        "pub annotation broken() { metadata(target) { missing_handler_value } }",
    )
    .expect("write dependency");
    let source = "from support use { @broken }\n@broken()\ntype Probe { id: int }";
    let program = shape_ast::parse_program(source).expect("fixture parses");
    let cache = crate::module_cache::ModuleCache::new();
    let session = GeneratedQuerySession::new(
        &program,
        source,
        CaptureQueryContext {
            file_path: Some(&file_path),
            module_cache: Some(&cache),
            workspace_root: None,
        },
    );
    assert!(matches!(session, GeneratedQuerySession::Unavailable));
}

#[test]
fn hard_compile_error_is_unavailable_and_capture_routing_does_not_fall_through() {
    let source = r#"
annotation derive() on type { }
@derive()
type Probe { id: int }
__intrinsic_std([1, 2, 3])
"#;
    let program = shape_ast::parse_program(source).expect("fixture parses");
    let session = GeneratedQuerySession::new(
        &program,
        source,
        CaptureQueryContext::unavailable(),
    );
    assert!(matches!(session, GeneratedQuerySession::Unavailable));

    let uri = "file:///probe.shape".parse().expect("URI");
    let lookup = generated_capture_definition(&program, source, 0, &uri, &session);
    assert!(matches!(lookup, GeneratedCaptureLookup::Unavailable));
}
