use shape_ast::parser::parse_program;
use tower_lsp_server::ls_types::{Position, Range, Uri};

use super::{
    CaptureQueryContext, GeneratedCaptureLookup, GeneratedQuerySession,
    generated_capture_references, generated_capture_rename,
};
use crate::util::{offset_to_line_col, position_to_offset};

const DIRECT_CAPTURE: &str = r#"
annotation add_reader() on type {
  comptime post(target, ctx) {
    extend target {
      method read() -> int {
        var total = 40
        let worker = |; share total| total + 2
        worker()
      }
    }
  }
}

@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read()
"#;

const COLLIDING_BINDING_NAMES: &str = r#"
annotation add_readers() on type {
  comptime post(target, ctx) {
    extend target {
      method left() -> int {
        var total = 40
        let worker = |; share total| total + 1
        worker()
      }
      method right() -> int {
        var total = 50
        let worker = |; share total| total + 2
        worker()
      }
    }
  }
}

@add_readers()
type Job { id: int }
"#;

// ADR-009 E2 #18 5b Part B (companion A): REPARSED_CAPTURE fixture RETIRED with
// its `unmapped_generated_string_is_quarantined_from_name_fallback` test —
// subject = a reparsed (source-string) generated capture quarantined from name
// fallback under an unavailable source map; the deleted U03 route was its sole
// producer. (`unavailable_query_context_never_falls_through_as_an_ordinary_rename`
// below constructs CaptureQueryContext::unavailable() DIRECTLY and stays.)

#[test]
fn rename_matches_the_complete_identity_joined_reference_graph() {
    let program = parse_program(DIRECT_CAPTURE).expect("fixture parses");
    let uri: Uri = "file:///capture-rename.shape".parse().unwrap();
    let session =
        GeneratedQuerySession::new(&program, DIRECT_CAPTURE, CaptureQueryContext::unavailable());
    let binder = DIRECT_CAPTURE.find("var total").unwrap() + "var ".len();
    let mode = DIRECT_CAPTURE.find("share total").unwrap();
    let declaration = mode + "share ".len();
    let use_site = DIRECT_CAPTURE.find("total + 2").unwrap();
    let references = generated_capture_references(&program, DIRECT_CAPTURE, declaration, &uri)
        .found("compiler-issued capture references");
    let expected: Vec<_> = references.iter().map(|location| location.range).collect();
    assert_eq!(expected.len(), 3, "binder + declaration + body use");

    for offset in [binder, declaration, use_site] {
        let edit =
            generated_capture_rename(&program, DIRECT_CAPTURE, offset, &uri, "amount", &session)
                .found("binding, declaration, and use select the same capture identity");
        let ranges: Vec<_> = edit.changes.unwrap()[&uri]
            .iter()
            .map(|edit| edit.range)
            .collect();
        assert_eq!(ranges, expected);
        assert!(
            ranges
                .iter()
                .all(|range| source_at(DIRECT_CAPTURE, *range) == "total")
        );
    }

    assert!(
        generated_capture_references(&program, DIRECT_CAPTURE, mode, &uri).is_not_capture(),
        "the capture mode is not part of the identity-bearing declaration site"
    );
    assert!(
        generated_capture_rename(&program, DIRECT_CAPTURE, mode, &uri, "amount", &session)
            .is_not_capture(),
        "the capture mode never authorizes an edit"
    );
}

#[test]
fn same_spelling_in_another_owner_is_not_renamed() {
    let program = parse_program(COLLIDING_BINDING_NAMES).expect("fixture parses");
    let uri: Uri = "file:///capture-collision.shape".parse().unwrap();
    let session = GeneratedQuerySession::new(
        &program,
        COLLIDING_BINDING_NAMES,
        CaptureQueryContext::unavailable(),
    );
    let left = COLLIDING_BINDING_NAMES.find("share total").unwrap() + "share ".len();
    let right_method = COLLIDING_BINDING_NAMES.find("method right").unwrap();
    let edit = generated_capture_rename(
        &program,
        COLLIDING_BINDING_NAMES,
        left,
        &uri,
        "amount",
        &session,
    )
    .found("left capture is descriptor-identified");
    let edits = &edit.changes.unwrap()[&uri];

    assert_eq!(edits.len(), 3, "left binder + declaration + body use");
    assert!(edits.iter().all(|edit| {
        position_to_offset(COLLIDING_BINDING_NAMES, edit.range.start)
            .is_some_and(|offset| offset < right_method)
    }));
}

// unmapped_generated_string_is_quarantined_from_name_fallback RETIRED — see the
// REPARSED_CAPTURE fixture-retirement note above (reparse route deleted).

#[test]
fn unavailable_query_context_never_falls_through_as_an_ordinary_rename() {
    let source = "from support use { helper }\n@derive()\ntype Job { id: int }";
    let program = parse_program(source).expect("fixture parses");
    let session = GeneratedQuerySession::new(&program, source, CaptureQueryContext::unavailable());
    let uri: Uri = "file:///unavailable-capture.shape".parse().unwrap();

    assert!(matches!(
        generated_capture_rename(
            &program,
            source,
            source.find("Job").unwrap(),
            &uri,
            "Task",
            &session,
        ),
        GeneratedCaptureLookup::Unavailable,
    ));
}

#[test]
fn capture_fallthrough_reuses_the_request_compiler_for_generated_symbols() {
    crate::generated_symbols::reset_generated_capture_compile_count();
    let program = parse_program(DIRECT_CAPTURE).expect("fixture parses");
    let session =
        GeneratedQuerySession::new(&program, DIRECT_CAPTURE, CaptureQueryContext::unavailable());
    let uri: Uri = "file:///capture-rename-session.shape".parse().unwrap();
    let method = DIRECT_CAPTURE.find("job.read").unwrap() + "job.".len();
    assert!(matches!(
        generated_capture_rename(&program, DIRECT_CAPTURE, method, &uri, "load", &session,),
        GeneratedCaptureLookup::NotCapture,
    ));
    let compiler = session
        .compiler()
        .expect("generating fixture has a compiler");
    assert!(
        crate::rename::generated_rename_from_compiler(
            DIRECT_CAPTURE,
            &uri,
            position(DIRECT_CAPTURE, method),
            "load",
            &program,
            compiler,
        )
        .is_some()
    );
    assert_eq!(
        crate::generated_symbols::generated_capture_compile_count(),
        1,
    );
}

fn source_at(source: &str, range: Range) -> &str {
    let start = position_to_offset(source, range.start).unwrap();
    let end = position_to_offset(source, range.end).unwrap();
    &source[start..end]
}

fn position(source: &str, offset: usize) -> Position {
    let (line, character) = offset_to_line_col(source, offset);
    Position { line, character }
}
