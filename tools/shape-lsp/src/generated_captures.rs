//! LSP projection of ADR-009 C1 generated capture descriptors.
//!
//! Every answer comes from `BytecodeCompiler::generated_capture_query`.
//! Source names and spans only select a presentation site already attached to
//! a compiler-issued binding identity; they never reconstruct that identity.

use shape_ast::ast::Program;
use shape_vm::compiler::GeneratedCaptureQuery;
use tower_lsp_server::ls_types::{Diagnostic, GotoDefinitionResponse, Location, Uri};

use crate::generated_symbols::compile_for_generated_symbol_queries;

mod hover;
mod presentation;
pub(crate) use hover::generated_capture_hover;
use presentation::push_anchor;

fn query(program: &Program, text: &str) -> GeneratedCaptureQuery {
    compile_for_generated_symbol_queries(program, text).generated_capture_query(program)
}

/// Go-to-definition over the identity-bearing capture graph. The declaration
/// and originating binder are both returned; neither is found by spelling.
pub(crate) fn generated_capture_definition(
    program: &Program,
    text: &str,
    offset: usize,
    uri: &Uri,
) -> Option<GotoDefinitionResponse> {
    let captures = query(program, text);
    let capture = captures.capture_at(0, offset)?.capture();
    let source = capture.source_map()?;
    let mut locations = Vec::new();
    push_anchor(&mut locations, source.declaration(), text, uri);
    push_anchor(&mut locations, source.binding(), text, uri);
    Some(GotoDefinitionResponse::Array(locations))
}

/// Find references over the same identity-bearing capture graph: originating
/// binder, explicit capture declaration, and every analyzer-resolved use.
pub(crate) fn generated_capture_references(
    program: &Program,
    text: &str,
    offset: usize,
    uri: &Uri,
) -> Option<Vec<Location>> {
    let captures = query(program, text);
    let capture = captures.capture_at(0, offset)?.capture();
    let identity = capture.identity().clone();
    let mut locations = Vec::new();
    for occurrence in captures.captures_for_binding(&identity) {
        let Some(source) = occurrence.source_map() else {
            continue;
        };
        push_anchor(&mut locations, source.binding(), text, uri);
        push_anchor(&mut locations, source.declaration(), text, uri);
        for use_site in source.uses() {
            push_anchor(&mut locations, *use_site, text, uri);
        }
    }
    (!locations.is_empty()).then_some(locations)
}

/// Stable informational diagnostics for typed capture descriptors whose
/// generated offsets have no exact authored-source map. The application
/// anchor is compiler-issued; an issue without one is omitted rather than
/// assigned an invented range.
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

    /// Real authored annotation IR: the compiler materializes/stamps this
    /// `extend target` method, while the original AST remains the exact source
    /// map for the capture declaration and use.
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
        )
        .expect("source-mapped generated capture has hover");
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("capture hover is markdown")
        };
        assert!(markup.value.contains("Canonical mode: `Share`"));
        assert!(markup.value.contains("Exact static type: `int`"));
        assert!(markup.value.contains("Stage: `generated-only`"));

        let uri: Uri = "file:///generated-capture.shape".parse().unwrap();
        let references =
            generated_capture_references(&program, GENERATED_CAPTURE, declaration, &uri)
                .expect("capture references");
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
            )
            .is_none(),
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
        .expect("joined sibling references");
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
        )
        .expect("aggregated generic hover");
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
        let site = captures
            .capture_at(0, nested_declaration)
            .expect("nested entry is source mapped");

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
            )
            .is_none(),
        );
    }

    fn offset_position(text: &str, offset: usize) -> Position {
        let (line, character) = offset_to_line_col(text, offset);
        Position { line, character }
    }
}
