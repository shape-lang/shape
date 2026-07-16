use super::{CaptureQueryContext, GeneratedQuerySession};
use crate::generated_symbols::{
    generated_capture_compile_count, reset_generated_capture_compile_count,
};
use crate::module_cache::ModuleCache;
use crate::rename::{
    GeneratedRenameRequest, generated_rename_request, rename_after_generated_query,
};
use crate::util::offset_to_line_col;
use tower_lsp_server::ls_types::Position;

const NESTED_GENERATED_CAPTURE: &str = r#"
mod generated {
  annotation add_reader() {
    targets: [function]
    comptime post(target, ctx) {
      extend Number {
        method read(x: int) -> int {
          var total = 5
          let worker = |y: int; share total| y + total
          worker(x)
        }
      }
    }
  }

  @add_reader()
  fn marker() -> int { 0 }
}
"#;

const NESTED_POISONED_GENERATION: &str = r#"
mod nested {
  annotation poison() {
    targets: [function]
    comptime post(target, ctx) { error("NESTED_POISON") }
  }

  @poison()
  fn ordinary_target() -> int { 0 }
  ordinary_target()
}
"#;

#[test]
fn ordinary_program_is_not_needed_without_compiler_invocation() {
    reset_generated_capture_compile_count();
    let source = "fn add(left: int, right: int) -> int { left + right }";
    let program = shape_ast::parse_program(source).expect("fixture parses");

    assert!(matches!(
        GeneratedQuerySession::new(&program, source, CaptureQueryContext::unavailable()),
        GeneratedQuerySession::NotNeeded,
    ));
    assert_eq!(generated_capture_compile_count(), 0);
}

#[test]
fn nested_generation_session_is_ready_with_exact_capture_descriptor() {
    reset_generated_capture_compile_count();
    let program = shape_ast::parse_program(NESTED_GENERATED_CAPTURE).expect("fixture parses");
    let session = GeneratedQuerySession::new(
        &program,
        NESTED_GENERATED_CAPTURE,
        CaptureQueryContext::unavailable(),
    );
    let GeneratedQuerySession::Ready(compiler) = session else {
        panic!("nested generation must compile into a ready query session")
    };
    assert_eq!(generated_capture_compile_count(), 1);

    let captures = compiler.generated_capture_query(&program);
    let matching: Vec<_> = captures
        .captures()
        .iter()
        .filter(|capture| capture.display_name() == "total")
        .collect();
    assert_eq!(matching.len(), 1, "one exact generated capture descriptor");
    let capture = matching[0];
    assert_eq!(capture.owner_display(), "Number.read");
    assert_eq!(capture.mode().variant_name(), "Share");
    assert_eq!(
        capture.uniform_capture_type().map(ToString::to_string),
        Some("int".to_string()),
    );
    let declaration = capture
        .source_map()
        .expect("direct generated capture has an authored source map")
        .declaration()
        .span();
    assert_eq!(
        &NESTED_GENERATED_CAPTURE[declaration.start..declaration.end],
        "total",
    );
}

#[test]
fn nested_poison_is_terminal_before_ordinary_rename_fallback() {
    let program = shape_ast::parse_program(NESTED_POISONED_GENERATION).expect("fixture parses");
    let target = NESTED_POISONED_GENERATION
        .find("ordinary_target")
        .expect("target declaration");
    let position = position(NESTED_POISONED_GENERATION, target);
    let uri = "file:///nested-poison.shape".parse().expect("URI");

    assert!(
        rename_after_generated_query(
            NESTED_POISONED_GENERATION,
            &uri,
            position,
            "renamed_target",
            Some(&program),
        )
        .is_some(),
        "ordinary rename would edit this source if terminal quarantine fell through",
    );

    reset_generated_capture_compile_count();
    let cache = ModuleCache::new();
    let request = generated_rename_request(
        NESTED_POISONED_GENERATION,
        &uri,
        position,
        "renamed_target",
        Some(&program),
        None,
        &cache,
        None,
    );
    assert!(matches!(request, GeneratedRenameRequest::Unavailable));
    assert_eq!(generated_capture_compile_count(), 1);
}

fn position(text: &str, offset: usize) -> Position {
    let (line, character) = offset_to_line_col(text, offset);
    Position { line, character }
}
