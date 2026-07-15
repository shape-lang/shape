//! LSP projection of compiler-issued ADR-009 generated capture descriptors.
//! Names and spans select verified presentation sites; they never mint identity.

use shape_ast::ast::Program;
#[cfg(test)]
use shape_vm::compiler::GeneratedCaptureQuery;
use shape_vm::compiler::{CaptureSiteRole, GeneratedCapturePosition};
use tower_lsp_server::ls_types::{Diagnostic, GotoDefinitionResponse, Location, Uri};

#[cfg(test)]
mod adversarial_tests;
mod hover;
mod navigation;
mod presentation;
mod rename;
#[cfg(test)]
mod rename_tests;
mod routing;
#[cfg(test)]
mod semantic_tests;
mod session;
#[cfg(test)]
use hover::generated_capture_hover;
pub(crate) use hover::generated_capture_hover_from_source;
use presentation::push_anchor;
pub(crate) use rename::generated_capture_rename;
pub(crate) use routing::GeneratedCaptureLookup;
use routing::{CaptureAnalysis, analyze, analyze_session};
pub(crate) use session::{CaptureQueryContext, GeneratedQuerySession};

#[cfg(test)]
fn query(program: &Program, text: &str) -> GeneratedCaptureQuery {
    match analyze(program, text, CaptureQueryContext::unavailable()) {
        CaptureAnalysis::Ready(query) => query,
        CaptureAnalysis::NotNeeded => GeneratedCaptureQuery::default(),
        CaptureAnalysis::Unavailable => panic!("fixture unexpectedly requires import context"),
    }
}

pub(crate) fn generated_capture_definition(
    program: &Program,
    text: &str,
    offset: usize,
    uri: &Uri,
    session: &GeneratedQuerySession,
) -> GeneratedCaptureLookup<GotoDefinitionResponse> {
    let captures = match analyze_session(session, program) {
        CaptureAnalysis::NotNeeded => return GeneratedCaptureLookup::NotCapture,
        CaptureAnalysis::Unavailable => return GeneratedCaptureLookup::Unavailable,
        CaptureAnalysis::Ready(captures) => captures,
    };
    let site = match captures.capture_at(0, offset) {
        None => return GeneratedCaptureLookup::NotCapture,
        Some(GeneratedCapturePosition::Unavailable) => {
            return GeneratedCaptureLookup::Unavailable;
        }
        Some(GeneratedCapturePosition::Available(site)) => site,
    };
    if site.role() == CaptureSiteRole::Binding {
        return GeneratedCaptureLookup::NotCapture;
    }
    let mut locations = Vec::new();
    for capture in site.captures() {
        let Some(source) = capture.source_map() else {
            continue;
        };
        push_anchor(&mut locations, source.declaration(), text, uri);
        push_anchor(&mut locations, source.binding(), text, uri);
    }
    GeneratedCaptureLookup::Found(GotoDefinitionResponse::Array(locations))
}

pub(crate) fn generated_capture_references(
    program: &Program,
    text: &str,
    offset: usize,
    uri: &Uri,
) -> GeneratedCaptureLookup<Vec<Location>> {
    let session = GeneratedQuerySession::new(program, text, CaptureQueryContext::unavailable());
    generated_capture_references_with_session(program, text, offset, uri, &session)
}

pub(crate) fn generated_capture_references_with_session(
    program: &Program,
    text: &str,
    offset: usize,
    uri: &Uri,
    session: &GeneratedQuerySession,
) -> GeneratedCaptureLookup<Vec<Location>> {
    let captures = match analyze_session(session, program) {
        CaptureAnalysis::NotNeeded => return GeneratedCaptureLookup::NotCapture,
        CaptureAnalysis::Unavailable => return GeneratedCaptureLookup::Unavailable,
        CaptureAnalysis::Ready(captures) => captures,
    };
    let site = match captures.capture_at(0, offset) {
        None => return GeneratedCaptureLookup::NotCapture,
        Some(GeneratedCapturePosition::Unavailable) => {
            return GeneratedCaptureLookup::Unavailable;
        }
        Some(GeneratedCapturePosition::Available(site)) => site,
    };
    let Some(anchors) = navigation::complete_binding_anchors(&captures, &site) else {
        return GeneratedCaptureLookup::Unavailable;
    };
    let mut locations = Vec::new();
    for anchor in anchors {
        push_anchor(&mut locations, anchor, text, uri);
    }
    GeneratedCaptureLookup::Found(locations)
}

pub(crate) fn capture_query_diagnostics(
    compiler: &shape_vm::BytecodeCompiler,
    program: &Program,
    text: &str,
) -> Vec<Diagnostic> {
    presentation::capture_query_diagnostics(compiler, program, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;
    use shape_vm::compiler::CaptureSiteRole;
    use tower_lsp_server::ls_types::{HoverContents, Position};

    use crate::util::offset_to_line_col;

    const GENERATED_CAPTURE: &str = r#"
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

let job = Job { id: 1 }
job.read(2)
"#;

    const GENERATED_CALLABLE_CAPTURE: &str = r#"
annotation add_runner() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method run(x: int) -> int {
        let scale = |value: int| value + 1
        let worker = |y: int; move scale| scale(y)
        worker(x)
      }
    }
  }
}

@add_runner()
type Job { id: int }

let job = Job { id: 1 }
job.run(2)
"#;

    const SIBLING_GENERATED_CAPTURES: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int {
        var total = 5
        let left = |y: int; share total| y + total
        let right = |y: int; share total| y + total
        left(x) + right(x)
      }
    }
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read(2)
"#;

    const REPARSED_GENERATED_CAPTURE: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read() -> int { let base = 40
      let worker = |; move base| base + 2
      worker() } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read()
"#;

    const GENERIC_GENERATED_CAPTURE: &str = r#"
annotation add_echo() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method echo<T>(value: T) -> T {
        let captured = value
        let worker = |; move captured| captured
        worker()
      }
    }
  }
}

@add_echo()
type Job { id: int }

let job = Job { id: 1 }
let number = job.echo(7)
let text = job.echo("shape")
"#;

    const NESTED_GENERATED_CAPTURE: &str = r#"
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

let job = Job { id: 1 }
job.read()
"#;

    const INVALID_IMPLICIT_GENERATED_CAPTURE: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int {
        let base = 40
        let worker = |y: int| y + base
        worker(x)
      }
    }
  }
}

@add_reader()
type Job { id: int }
"#;

    #[test]
    fn direct_generated_capture_uses_compiler_query_for_hover_and_references() {
        let program = parse_program(GENERATED_CAPTURE).expect("fixture parses");
        let declaration = GENERATED_CAPTURE.find("share total").unwrap() + "share ".len();
        let hover = generated_capture_hover(
            &program,
            GENERATED_CAPTURE,
            offset_position(GENERATED_CAPTURE, declaration),
            &test_session(&program, GENERATED_CAPTURE),
        )
        .found("source-mapped generated capture has hover");
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("capture hover is markdown")
        };
        assert!(markup.value.contains("Canonical mode: `Share`"));
        assert!(markup.value.contains("Exact static type: `int`"));
        assert!(markup.value.contains("Stage: `generated-only`"));

        let uri: Uri = "file:///generated-capture.shape".parse().unwrap();
        let references =
            generated_capture_references(&program, GENERATED_CAPTURE, declaration, &uri)
                .found("capture references");
        assert_eq!(references.len(), 3, "binder + declaration + one use");
    }

    #[test]
    fn cursor_in_captured_callable_argument_is_not_a_capture_site() {
        let program = parse_program(GENERATED_CALLABLE_CAPTURE).expect("fixture parses");
        let argument = GENERATED_CALLABLE_CAPTURE.find("scale(y)").unwrap() + "scale(".len();
        assert!(
            generated_capture_hover(
                &program,
                GENERATED_CALLABLE_CAPTURE,
                offset_position(GENERATED_CALLABLE_CAPTURE, argument),
                &test_session(&program, GENERATED_CALLABLE_CAPTURE),
            )
            .is_not_capture(),
            "the call argument must not inherit the captured callee's use span",
        );
    }

    #[test]
    fn sibling_occurrences_share_binding_identity_without_artifact_conflict() {
        let program = parse_program(SIBLING_GENERATED_CAPTURES).expect("fixture parses");
        let captures = query(&program, SIBLING_GENERATED_CAPTURES);
        let total_captures: Vec<_> = captures
            .captures()
            .iter()
            .filter(|capture| capture.display_name() == "total")
            .collect();

        assert_eq!(
            total_captures.len(),
            2,
            "one occurrence per sibling closure"
        );
        assert_eq!(total_captures[0].identity(), total_captures[1].identity());
        assert_ne!(
            total_captures[0].occurrence_identity(),
            total_captures[1].occurrence_identity(),
        );
        assert!(
            captures
                .issues()
                .iter()
                .all(|issue| issue.code() != "C0911"),
            "valid sibling occurrences must not be diagnosed as conflicting artifacts",
        );

        let first_declaration =
            SIBLING_GENERATED_CAPTURES.find("share total").unwrap() + "share ".len();
        let uri: Uri = "file:///sibling-generated-captures.shape".parse().unwrap();
        let references = generated_capture_references(
            &program,
            SIBLING_GENERATED_CAPTURES,
            first_declaration,
            &uri,
        )
        .found("joined sibling references");
        assert_eq!(
            references.len(),
            5,
            "one binder + two declarations + two resolved uses",
        );
    }

    #[test]
    fn reparsed_capture_reports_honest_source_unavailable_contract() {
        let program = parse_program(REPARSED_GENERATED_CAPTURE).expect("fixture parses");
        let captures = query(&program, REPARSED_GENERATED_CAPTURE);
        let issue = captures
            .issues()
            .iter()
            .find(|issue| issue.code() == "C0910")
            .expect("reparsed generated capture has no exact authored source map");

        assert!(issue.message().contains("compiler capture query"));
        assert!(
            issue
                .message()
                .contains("source hover and navigation are unavailable"),
        );
        assert!(!issue.message().contains("expansion view"));
    }

    #[test]
    fn generic_occurrence_aggregates_exact_specializations_deterministically() {
        let program = parse_program(GENERIC_GENERATED_CAPTURE).expect("fixture parses");
        let captures = query(&program, GENERIC_GENERATED_CAPTURE);
        let captured = captures
            .captures()
            .iter()
            .find(|capture| capture.display_name() == "captured")
            .expect("generic capture occurrence");
        let mut exact_types: Vec<_> = captured
            .specializations()
            .iter()
            .map(|specialization| specialization.capture_type().to_string())
            .collect();
        exact_types.sort();
        exact_types.dedup();

        assert_eq!(exact_types, ["int", "string"]);
        assert!(captured.uniform_capture_type().is_none());
        assert!(
            captures
                .issues()
                .iter()
                .all(|issue| issue.code() != "C0911"),
            "valid monomorphized variants must not conflict",
        );

        let declaration = GENERIC_GENERATED_CAPTURE.find("move captured").unwrap() + "move ".len();
        let hover = generated_capture_hover(
            &program,
            GENERIC_GENERATED_CAPTURE,
            offset_position(GENERIC_GENERATED_CAPTURE, declaration),
            &test_session(&program, GENERIC_GENERATED_CAPTURE),
        )
        .found("aggregated generic hover");
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("capture hover is markdown")
        };
        assert!(markup.value.contains("Exact specialization types (2)"));
        assert!(markup.value.contains("`int`, `string`"));
        assert!(!markup.value.contains("Exact static type:"));
    }

    #[test]
    fn nested_capture_entry_prefers_inner_declaration_over_outer_forwarding_use() {
        let program = parse_program(NESTED_GENERATED_CAPTURE).expect("fixture parses");
        let captures = query(&program, NESTED_GENERATED_CAPTURE);
        let nested_declaration =
            NESTED_GENERATED_CAPTURE.rfind("share total").unwrap() + "share ".len();
        let position = captures
            .capture_at(0, nested_declaration)
            .expect("nested entry is source mapped");
        let GeneratedCapturePosition::Available(site) = position else {
            panic!("valid nested entry is not quarantined")
        };

        assert_eq!(site.role(), CaptureSiteRole::Declaration);
        assert!(
            captures
                .issues()
                .iter()
                .all(|issue| issue.code() != "C0910"),
            "the nested entry is both an inner declaration and exact outer forwarding use",
        );
    }

    #[test]
    fn rejected_implicit_generated_capture_has_no_query_descriptor() {
        let program = parse_program(INVALID_IMPLICIT_GENERATED_CAPTURE).expect("fixture parses");
        let captures = query(&program, INVALID_IMPLICIT_GENERATED_CAPTURE);
        let captured_use = INVALID_IMPLICIT_GENERATED_CAPTURE.find("y + base").unwrap() + 4;

        assert!(captures.captures().is_empty());
        assert!(
            generated_capture_hover(
                &program,
                INVALID_IMPLICIT_GENERATED_CAPTURE,
                offset_position(INVALID_IMPLICIT_GENERATED_CAPTURE, captured_use),
                &test_session(&program, INVALID_IMPLICIT_GENERATED_CAPTURE),
            )
            .is_not_capture(),
        );
    }

    fn offset_position(text: &str, offset: usize) -> Position {
        let (line, character) = offset_to_line_col(text, offset);
        Position { line, character }
    }

    fn test_session(program: &Program, text: &str) -> GeneratedQuerySession {
        GeneratedQuerySession::new(program, text, CaptureQueryContext::unavailable())
    }
}
