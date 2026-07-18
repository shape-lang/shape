use std::collections::BTreeSet;

use shape_ast::parser::parse_program;
use shape_vm::compiler::GeneratedCapturePosition;
use tower_lsp_server::ls_types::{HoverContents, Position, Range, Uri};

use super::{
    CaptureQueryContext, GeneratedCaptureLookup, GeneratedQuerySession, generated_capture_hover,
    generated_capture_references, generated_capture_rename, query,
};
use crate::util::{offset_to_line_col, position_to_offset};

const TWO_APPLICATIONS: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int {
        var total = 5
        let worker = |y: int; share total| y + total
        worker(x)
      }
    }
  }
}

@add_reader()
type Job { id: int }

@add_reader()
type Task { id: int }

let job = Job { id: 1 }
let task = Task { id: 2 }
job.read(2) + task.read(3)
"#;

const NESTED_CAPTURE: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read() -> int {
        var total = 40
        let outer = |; share total| {
          let inner = |; share total| total + 2
          inner()
        }
        outer()
      }
    }
  }
}

@add_reader()
type Job { id: int }
"#;

// ADR-009 E2 #18 5b Part B (companion A): `colliding_reparsed_offsets_never_
// acquire_the_direct_decoys_source_map` RETIRED, closed-under-callers (its inline
// `extend ("__PAYLOAD__")` reparse skeleton + payload machinery go with it). Its
// subject is an adversarial property OF the deleted U03 reparse route (a reparsed
// capture at colliding offsets must not steal a direct decoy's source_map, and
// must carry GENERATED_CAPTURE_SOURCE_UNAVAILABLE_CODE) — post-deletion the
// reparsed half is unconstructible. The surviving half (direct captures get
// correct source maps) is carried by the direct-route fixtures + the no-C0910
// asserts the reachability agent cited.

#[test]
fn one_template_position_returns_every_application_without_conflict() {
    let program = parse_program(TWO_APPLICATIONS).expect("fixture parses");
    let captures = query(&program, TWO_APPLICATIONS);
    let declaration = TWO_APPLICATIONS.find("share total").unwrap() + "share ".len();
    let position = captures
        .capture_at(0, declaration)
        .expect("template declaration maps");
    let GeneratedCapturePosition::Available(site) = position else {
        panic!("valid multi-application site was quarantined")
    };
    let owners: BTreeSet<_> = site
        .captures()
        .iter()
        .map(|capture| capture.owner_display())
        .collect();

    assert_eq!(site.captures().len(), 2);
    assert_eq!(owners, BTreeSet::from(["Job.read", "Task.read"]));
    assert!(
        captures
            .issues()
            .iter()
            .all(|issue| issue.code() != "C0911")
    );

    let hover = match generated_capture_hover(
        &program,
        TWO_APPLICATIONS,
        offset_position(TWO_APPLICATIONS, declaration),
        &GeneratedQuerySession::new(
            &program,
            TWO_APPLICATIONS,
            CaptureQueryContext::unavailable(),
        ),
    ) {
        GeneratedCaptureLookup::Found(hover) => hover,
        _ => panic!("valid template site has aggregate hover"),
    };
    let HoverContents::Markup(markup) = hover.contents else {
        panic!("capture hover is markdown")
    };
    assert!(markup.value.contains("Applications:"));
    assert!(markup.value.contains("Job.read"));
    assert!(markup.value.contains("Task.read"));
}

#[test]
fn binder_references_join_every_template_application_and_nested_forwarder() {
    let uri: Uri = "file:///generated-capture.shape".parse().unwrap();
    let program = parse_program(TWO_APPLICATIONS).expect("fixture parses");
    let binder = TWO_APPLICATIONS.find("var total").unwrap() + "var ".len();
    let references = match generated_capture_references(&program, TWO_APPLICATIONS, binder, &uri) {
        GeneratedCaptureLookup::Found(references) => references,
        _ => panic!("binder participates in generated reference graph"),
    };
    assert_eq!(
        references.len(),
        3,
        "binder + template clause + template use"
    );

    let nested_program = parse_program(NESTED_CAPTURE).expect("nested fixture parses");
    let nested_query = query(&nested_program, NESTED_CAPTURE);
    let total_captures: Vec<_> = nested_query
        .captures()
        .iter()
        .filter(|capture| capture.display_name() == "total")
        .collect();
    assert_eq!(total_captures.len(), 2);
    assert_eq!(total_captures[0].identity(), total_captures[1].identity());

    let nested_binder = NESTED_CAPTURE.find("var total").unwrap() + "var ".len();
    let nested_references =
        match generated_capture_references(&nested_program, NESTED_CAPTURE, nested_binder, &uri) {
            GeneratedCaptureLookup::Found(references) => references,
            _ => panic!("nested binder participates in generated reference graph"),
        };
    assert_eq!(
        nested_references.len(),
        4,
        "binder + outer clause + inner clause + final body use",
    );
    let reference_ranges: Vec<_> = nested_references
        .iter()
        .map(|location| location.range)
        .collect();
    let reference_offsets = exact_token_offsets(NESTED_CAPTURE, &reference_ranges, "total");
    assert!(
        reference_offsets
            .windows(2)
            .all(|pair| pair[0].1 <= pair[1].0),
        "identity-joined reference anchors are exact, ordered, and nonoverlapping",
    );

    let mode_offsets: Vec<_> = NESTED_CAPTURE
        .match_indices("share total")
        .map(|(offset, _)| offset)
        .collect();
    assert_eq!(mode_offsets.len(), 2, "outer and inner capture modes");
    let declaration_offsets: Vec<_> = mode_offsets
        .iter()
        .map(|offset| offset + "share ".len())
        .collect();
    let final_use = NESTED_CAPTURE.find("total + 2").unwrap();
    let session = GeneratedQuerySession::new(
        &nested_program,
        NESTED_CAPTURE,
        CaptureQueryContext::unavailable(),
    );

    for mode in mode_offsets {
        assert!(
            generated_capture_references(&nested_program, NESTED_CAPTURE, mode, &uri)
                .is_not_capture(),
            "capture mode text never selects the identity graph",
        );
        assert!(
            generated_capture_rename(
                &nested_program,
                NESTED_CAPTURE,
                mode,
                &uri,
                "amount",
                &session,
            )
            .is_not_capture(),
            "capture mode text never authorizes a rename edit",
        );
    }

    for offset in [
        nested_binder,
        declaration_offsets[0],
        declaration_offsets[1],
        final_use,
    ] {
        let edit = generated_capture_rename(
            &nested_program,
            NESTED_CAPTURE,
            offset,
            &uri,
            "amount",
            &session,
        )
        .found("every identity-bearing site renames the complete nested graph");
        let changes = edit.changes.expect("rename returns document changes");
        let edit_ranges: Vec<_> = changes[&uri].iter().map(|edit| edit.range).collect();
        assert_eq!(edit_ranges, reference_ranges);
        assert_eq!(
            exact_token_offsets(NESTED_CAPTURE, &edit_ranges, "total"),
            reference_offsets,
            "rename admits neither whole-entry nor overlapping edits",
        );
    }
}

#[test]
fn frontmatter_preserves_generated_capture_hover_definition_and_reference_offsets() {
    let source = format!("---\nname = \"capture-fixture\"\n---\n{TWO_APPLICATIONS}");
    let declaration = source.find("share total").unwrap() + "share ".len();
    let position = offset_position(&source, declaration);
    let uri: Uri = "file:///frontmatter-capture.shape".parse().unwrap();

    let hover = crate::hover::get_hover(&source, position, None, None, None)
        .expect("frontmatter capture hover");
    let HoverContents::Markup(markup) = hover.contents else {
        panic!("capture hover is markdown")
    };
    assert!(markup.value.contains("Generated Capture"));

    let definition = crate::definition::get_definition(&source, position, &uri, None, None, None);
    assert!(definition.is_some(), "frontmatter capture definition");

    let references = crate::definition::get_references(&source, position, &uri)
        .expect("frontmatter capture references");
    assert_eq!(references.len(), 3);
}

#[test]
fn cross_file_reference_request_compiles_the_capture_query_once() {
    crate::generated_symbols::reset_generated_capture_compile_count();
    let declaration = TWO_APPLICATIONS.find("share total").unwrap() + "share ".len();
    let uri: Uri = "file:///single-capture-query.shape".parse().unwrap();
    let references = crate::definition::get_references_cross_file(
        TWO_APPLICATIONS,
        offset_position(TWO_APPLICATIONS, declaration),
        &uri,
        None,
        None,
        None,
        None,
    )
    .expect("capture references");

    assert_eq!(references.len(), 3);
    assert_eq!(
        crate::generated_symbols::generated_capture_compile_count(),
        1,
    );
}

#[test]
fn separate_requests_do_not_cache_session_scoped_binding_identities() {
    crate::generated_symbols::reset_generated_capture_compile_count();
    let declaration = TWO_APPLICATIONS.find("share total").unwrap() + "share ".len();
    let position = offset_position(TWO_APPLICATIONS, declaration);
    let uri: Uri = "file:///request-scoped-capture-query.shape"
        .parse()
        .unwrap();

    for _ in 0..2 {
        let references = crate::definition::get_references(TWO_APPLICATIONS, position, &uri)
            .expect("each request resolves capture references");
        assert_eq!(references.len(), 3);
    }

    assert_eq!(
        crate::generated_symbols::generated_capture_compile_count(),
        2,
        "each LSP request must issue a fresh query-local binding identity",
    );
}

#[test]
fn capture_not_capture_reuses_the_session_for_generated_definition() {
    crate::generated_symbols::reset_generated_capture_compile_count();
    let offset = TWO_APPLICATIONS.find("job.read").unwrap() + "job.".len();
    let uri: Uri = "file:///shared-query-session.shape".parse().unwrap();
    let definition = crate::definition::get_definition(
        TWO_APPLICATIONS,
        offset_position(TWO_APPLICATIONS, offset),
        &uri,
        None,
        None,
        None,
    );

    assert!(
        definition.is_some(),
        "legacy generated-symbol provider answers"
    );
    assert_eq!(
        crate::generated_symbols::generated_capture_compile_count(),
        1,
        "capture NotCapture must reuse the request compiler",
    );
}

#[test]
fn imported_annotation_registration_survives_capture_fallthrough() {
    let temp = tempfile::tempdir().expect("temporary module workspace");
    let support_path = temp.path().join("support.shape");
    let decoy_path = temp.path().join("decoy.shape");
    let main_path = temp.path().join("main.shape");
    let manifest_path = temp.path().join("shape.toml");
    let support = r#"
pub annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target { method read() -> int { 42 } }
  }
}
"#;
    // This module is loaded before `support` and exports the same annotation
    // spelling. A bare-name, first-wins registration would retain this decoy
    // even though root import resolution selects `support::add_reader`, making
    // the `read` definition disappear.
    let decoy = r#"
pub annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target { method decoy() -> int { 0 } }
  }
}
pub fn helper() { 0 }
"#;
    // `decoy` and `support` are declared as shape.toml path dependencies whose
    // roots resolve to the sibling `decoy.shape`/`support.shape` files through
    // the `<dep_root>.shape` fallback in module resolution. This is the shipped
    // route for importing local files by bare module name — the grammar has no
    // `./` module-path syntax, so the previous `from ./support` spelling died at
    // parse before any of the registration logic below could run.
    let manifest = r#"[dependencies]
decoy = { path = "decoy" }
support = { path = "support" }
"#;
    // `decoy` is imported first so it is registered ahead of `support`,
    // preserving the same-name collision the test probes: a first-wins bug would
    // retain the decoy's `add_reader` even though `@add_reader` is explicitly
    // imported from `support`.
    let source = r#"
from decoy use { helper }
from support use { @add_reader }
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read()
"#;
    std::fs::write(&support_path, support).expect("write imported annotation module");
    std::fs::write(&decoy_path, decoy).expect("write same-named decoy annotation module");
    std::fs::write(&manifest_path, manifest).expect("write shape.toml path dependencies");
    std::fs::write(&main_path, source).expect("write importing source");
    let cache = crate::module_cache::ModuleCache::new();
    let uri = Uri::from_file_path(&main_path).expect("file URI");
    let offset = source.find("job.read").unwrap() + "job.".len();

    crate::generated_symbols::reset_generated_capture_compile_count();
    let definition = crate::definition::get_definition(
        source,
        offset_position(source, offset),
        &uri,
        Some(&cache),
        None,
        None,
    );

    assert!(
        definition.is_some(),
        "the explicitly imported annotation generates its method without aliasing the same-named decoy"
    );
    assert_eq!(
        crate::generated_symbols::generated_capture_compile_count(),
        1,
        "import-registered compiler must feed both providers",
    );
}

#[test]
fn import_gated_session_is_unavailable_not_an_ordinary_fallthrough() {
    let source = "from support use { helper }\n@derive()\ntype Job { id: int }";
    let program = parse_program(source).expect("fixture parses");
    crate::generated_symbols::reset_generated_capture_compile_count();
    let session = GeneratedQuerySession::new(&program, source, CaptureQueryContext::unavailable());
    let uri: Uri = "file:///import-gated.shape".parse().unwrap();
    let offset = source.find("Job").unwrap();

    assert!(matches!(
        super::generated_capture_definition(&program, source, offset, &uri, &session),
        GeneratedCaptureLookup::Unavailable,
    ));
    assert_eq!(
        crate::generated_symbols::generated_capture_compile_count(),
        0,
        "missing import context must refuse before compiling",
    );
}

fn offset_position(text: &str, offset: usize) -> Position {
    let (line, character) = offset_to_line_col(text, offset);
    Position { line, character }
}

fn exact_token_offsets(source: &str, ranges: &[Range], expected: &str) -> Vec<(usize, usize)> {
    ranges
        .iter()
        .map(|range| {
            let start = position_to_offset(source, range.start).expect("range starts in source");
            let end = position_to_offset(source, range.end).expect("range ends in source");
            assert_eq!(&source[start..end], expected);
            (start, end)
        })
        .collect()
}
