use super::{
    CaptureQueryContext, GeneratedCaptureLookup, GeneratedQuerySession, generated_capture_hover,
    generated_capture_rename,
};
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
  mod decoy {
    pub annotation add_reader() {
      targets: [function]
      comptime post(target, ctx) {
        extend Number {
          method read(x: int) -> int {
            var decoy_total = 99
            let worker = |y: int; share decoy_total| y + decoy_total
            worker(x)
          }
        }
      }
    }
  }

  pub annotation add_reader() {
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

  extend Number {
    @add_reader()
    method marker() { self }
  }
}

let force = 2.0.read(0)
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

const EXPRESSION_GENERATED_CAPTURE: &str = r#"
annotation add_reader() {
  targets: [expression]
  comptime post(target, ctx) {
    extend Job {
      method expression_read(x: int) -> int {
        var total = 5
        let worker = |y: int; share total| y + total
        worker(x)
      }
    }
  }
}

type Job { id: int }
let trigger = @add_reader() 0
"#;

#[test]
fn ordinary_program_is_not_needed_without_compiler_invocation() {
    reset_generated_capture_compile_count();
    let source = r#"
mod arithmetic {
  fn add(left: int, right: int) -> int { left + right }
}
"#;
    let program = shape_ast::parse_program(source).expect("fixture parses");

    assert!(matches!(
        GeneratedQuerySession::new(&program, source, CaptureQueryContext::unavailable()),
        GeneratedQuerySession::NotNeeded,
    ));
    assert_eq!(generated_capture_compile_count(), 0);
}

#[test]
fn nested_annotated_method_session_is_ready_with_exact_capture_descriptor() {
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
    let source_map = capture
        .source_map()
        .expect("direct generated capture has an authored source map");
    let binding = source_map.binding().span();
    let declaration = source_map.declaration().span();
    let [use_site] = source_map.uses() else {
        panic!("direct generated capture has one exact authored use")
    };
    assert_eq!(
        &NESTED_GENERATED_CAPTURE[binding.start..binding.end],
        "total",
    );
    assert_eq!(
        &NESTED_GENERATED_CAPTURE[declaration.start..declaration.end],
        "total",
    );
    let use_site = use_site.span();
    assert_eq!(
        &NESTED_GENERATED_CAPTURE[use_site.start..use_site.end],
        "total",
    );
    assert!(captures.issues().is_empty());
}

#[test]
fn expression_annotation_missing_inference_is_terminally_quarantined() {
    reset_generated_capture_compile_count();
    let program = shape_ast::parse_program(EXPRESSION_GENERATED_CAPTURE).expect("fixture parses");
    let session = GeneratedQuerySession::new(
        &program,
        EXPRESSION_GENERATED_CAPTURE,
        CaptureQueryContext::unavailable(),
    );
    let GeneratedQuerySession::Ready(compiler) = &session else {
        panic!("expression-level generation must compile into a ready query session")
    };
    assert_eq!(generated_capture_compile_count(), 1);

    let generated_symbols = compiler
        .generated_symbol_query()
        .symbols_named("expression_read");
    assert_eq!(
        generated_symbols.len(),
        1,
        "the expression handler publishes one generated method symbol",
    );
    assert_eq!(generated_symbols[0].decl_name, "Job.expression_read");
    assert_eq!(
        compiler.function_type_ids().len(),
        1,
        "Job.expression_read compiles its one closure body before capture projection",
    );

    let captures = compiler.generated_capture_query(&program);
    assert!(
        captures.captures().is_empty(),
        "missing inference evidence cannot publish a capture descriptor",
    );
    let [issue] = captures.issues() else {
        panic!("expression capture must produce one exact semantic-evidence refusal")
    };
    assert_eq!(issue.code(), "C0911");
    assert_eq!(
        issue.message(),
        "[C0911] generated capture 'total' cannot establish a structural specialization identity: capture descriptor 0 semantic evidence is unavailable (MissingInferenceFact): capture 'total' has no structural inference fact at ordinal 0: occurrence:0ba24839b354aaf7857a2f7dfedaf0f6:node:extend:Job/method:expression_read/closure:0:descriptor:0",
    );

    let declaration = EXPRESSION_GENERATED_CAPTURE.find("share total").unwrap() + "share ".len();
    assert!(matches!(
        captures.capture_at(0, declaration),
        Some(shape_vm::compiler::GeneratedCapturePosition::Unavailable),
    ));
    assert!(matches!(
        generated_capture_hover(
            &program,
            EXPRESSION_GENERATED_CAPTURE,
            position(EXPRESSION_GENERATED_CAPTURE, declaration),
            &session,
        ),
        GeneratedCaptureLookup::Unavailable,
    ));
    let uri = "file:///expression-capture.shape".parse().expect("URI");
    assert!(matches!(
        generated_capture_rename(
            &program,
            EXPRESSION_GENERATED_CAPTURE,
            declaration,
            &uri,
            "renamed_total",
            &session,
        ),
        GeneratedCaptureLookup::Unavailable,
    ));
    assert_eq!(
        generated_capture_compile_count(),
        1,
        "hover and rename reuse the request's quarantined compiler query",
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
