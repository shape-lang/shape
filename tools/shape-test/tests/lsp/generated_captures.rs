//! ADR-009 C1 slice 4: strictly typed LSP projection of explicit generated
//! capture descriptors. Hover and navigation consume the compiler query;
//! ordinary inferred closures keep their existing source-only behavior.

use shape_test::shape_test::{ShapeTest, pos};
use tower_lsp_server::ls_types::Position;

const SIBLING_CAPTURES: &str = r#"
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

const COLLIDING_SCOPES: &str = r#"
annotation add_readers() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method first(x: int) -> int {
        let total = 10
        let worker = |y: int; move total| y + total
        worker(x)
      }
      method second(x: int) -> int {
        let total = 20
        let worker = |y: int; move total| y + total
        worker(x)
      }
    }
  }
}

@add_readers()
type Job { id: int }

let job = Job { id: 1 }
job.first(1) + job.second(2)
"#;

const ORDINARY_INFERRED_CAPTURE: &str = r#"
let total = 5
let worker = |y: int| y + total
worker(2)
"#;

// REPARSED_GENERATED_CAPTURE fixture RETIRED with its C0910 test (see note below,
// at the retired reparsed_generated_capture test) — the U03 reparse route it fed
// is deleted this slice.

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

const GENERIC_CAPTURE_SPECIALIZATIONS: &str = r#"
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

const INVALID_UNUSED_GENERATED_CAPTURE: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int {
        let unused = 7
        let worker = |y: int; move unused| y + 1
        worker(x)
      }
    }
  }
}

@add_reader()
type Job { id: int }
"#;

#[test]
fn generated_capture_hover_reports_mode_type_stage_owner_and_identity() {
    ShapeTest::new(SIBLING_CAPTURES)
        .at(nth_position(SIBLING_CAPTURES, "share total", 0, 6))
        .expect_hover_contains("Generated Capture")
        .expect_hover_contains("share total: int")
        .expect_hover_contains("Canonical mode: `Share`")
        .expect_hover_contains("Exact static type: `int`")
        .expect_hover_contains("Stage: `generated-only`")
        .expect_hover_contains("Owner:")
        .expect_hover_contains("Binding identity: `capture:")
        .expect_hover_contains("Capture occurrence:");
}

#[test]
fn definition_links_one_occurrence_to_its_declaration_and_binding() {
    let binding_line = nth_line(SIBLING_CAPTURES, "var total", 0);
    let declaration_line = nth_line(SIBLING_CAPTURES, "share total", 0);
    let sibling_declaration_line = nth_line(SIBLING_CAPTURES, "share total", 1);

    ShapeTest::new(SIBLING_CAPTURES)
        .at(nth_position(SIBLING_CAPTURES, "y + total", 0, 4))
        .expect_definition_includes_lines(&[declaration_line, binding_line])
        .expect_definition_excludes_line(sibling_declaration_line);
}

#[test]
fn references_join_sibling_occurrences_of_the_same_binding() {
    let expected = [
        nth_line(SIBLING_CAPTURES, "var total", 0),
        nth_line(SIBLING_CAPTURES, "share total", 0),
        nth_line(SIBLING_CAPTURES, "y + total", 0),
        nth_line(SIBLING_CAPTURES, "share total", 1),
        nth_line(SIBLING_CAPTURES, "y + total", 1),
    ];

    ShapeTest::new(SIBLING_CAPTURES)
        .at(nth_position(SIBLING_CAPTURES, "share total", 0, 6))
        .expect_references_include_lines(&expected)
        .expect_no_semantic_diagnostic_contains("[C0911]");
}

#[test]
fn colliding_names_in_distinct_generated_owners_do_not_cross_link() {
    let first_lines = [
        nth_line(COLLIDING_SCOPES, "let total", 0),
        nth_line(COLLIDING_SCOPES, "move total", 0),
        nth_line(COLLIDING_SCOPES, "y + total", 0),
    ];
    let second_lines = [
        nth_line(COLLIDING_SCOPES, "let total", 1),
        nth_line(COLLIDING_SCOPES, "move total", 1),
        nth_line(COLLIDING_SCOPES, "y + total", 1),
    ];

    let test = ShapeTest::new(COLLIDING_SCOPES)
        .at(nth_position(COLLIDING_SCOPES, "move total", 0, 5))
        .expect_references_include_lines(&first_lines);
    second_lines
        .into_iter()
        .fold(test, |test, line| test.expect_references_exclude_line(line));
}

#[test]
fn ordinary_source_closure_capture_remains_inference_only() {
    ShapeTest::new(ORDINARY_INFERRED_CAPTURE)
        .at(nth_position(ORDINARY_INFERRED_CAPTURE, "y + total", 0, 4))
        .expect_hover_exists()
        .expect_hover_not_contains("Generated Capture")
        .expect_no_semantic_diagnostic_contains("[C0910]")
        .expect_no_semantic_diagnostic_contains("[C0911]");
}

// ADR-009 E2 #18 5b Part B (companion A): `reparsed_generated_capture_reports_
// honest_source_unavailable_diagnostic` RETIRED — subject = the [C0910]
// source-unavailable diagnostic of the deleted U03 reparse route (deletion
// replaces the emission with a loud invariant error). Surviving capture-query
// coverage stays in the ordinary/generic/invalid fixtures around it.

#[test]
fn generic_capture_hover_lists_every_exact_specialization_without_picking_one() {
    ShapeTest::new(GENERIC_CAPTURE_SPECIALIZATIONS)
        .at(nth_position(
            GENERIC_CAPTURE_SPECIALIZATIONS,
            "move captured",
            0,
            5,
        ))
        .expect_hover_contains("Generated Capture")
        .expect_hover_contains("Exact specialization types (2): `int`, `string`")
        .expect_hover_contains("Structural specializations:")
        .expect_hover_not_contains("Exact static type:")
        .expect_no_semantic_diagnostic_contains("[C0911]");
}

#[test]
fn invalid_generated_implicit_capture_has_no_invented_descriptor_hover() {
    ShapeTest::new(INVALID_IMPLICIT_GENERATED_CAPTURE).expect_semantic_diagnostic_contains(
        "generated closure implicitly captures 'base'; generated captures must be explicit",
    );
}

#[test]
fn unused_explicit_capture_is_rejected_before_query_mapping() {
    ShapeTest::new(INVALID_UNUSED_GENERATED_CAPTURE)
        .expect_semantic_diagnostic_contains(
            "[C0901] declared capture 'unused' is never used by the closure body",
        )
        .expect_no_semantic_diagnostic_contains("[C0910]");
}

fn nth_position(source: &str, needle: &str, occurrence: usize, within: usize) -> Position {
    let offset = nth_offset(source, needle, occurrence) + within;
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let character = prefix
        .rsplit_once('\n')
        .map_or(prefix.len(), |(_, tail)| tail.len()) as u32;
    pos(line, character)
}

fn nth_line(source: &str, needle: &str, occurrence: usize) -> u32 {
    nth_position(source, needle, occurrence, 0).line
}

fn nth_offset(source: &str, needle: &str, occurrence: usize) -> usize {
    source
        .match_indices(needle)
        .nth(occurrence)
        .unwrap_or_else(|| panic!("missing occurrence {occurrence} of {needle:?}"))
        .0
}
