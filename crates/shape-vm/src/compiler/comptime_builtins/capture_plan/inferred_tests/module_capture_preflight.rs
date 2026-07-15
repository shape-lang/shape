use super::*;
use crate::bytecode::OpCode;
use crate::type_tracking::BindingStorageClass;

#[derive(Debug, PartialEq, Eq)]
struct ClosurePublicationState {
    counter: u64,
    registry_len: usize,
    function_type_registry_len: usize,
    pack_len: usize,
    closure_function_id_len: usize,
    closure_type_id_len: usize,
    function_type_id_len: usize,
    capture_name_len: usize,
    closure_function_count: usize,
    layout_len: usize,
    published_layout_count: usize,
}

fn closure_publication_state(compiler: &BytecodeCompiler) -> ClosurePublicationState {
    ClosurePublicationState {
        counter: compiler.closure_counter,
        registry_len: compiler.closure_registry.len(),
        function_type_registry_len: compiler.function_type_registry.len(),
        pack_len: compiler.closure_capture_packs.len(),
        closure_function_id_len: compiler.closure_function_ids.len(),
        closure_type_id_len: compiler.closure_type_ids.len(),
        function_type_id_len: compiler.function_type_ids.len(),
        capture_name_len: compiler.closure_capture_names.len(),
        closure_function_count: compiler
            .program
            .functions
            .iter()
            .filter(|function| function.is_closure)
            .count(),
        layout_len: compiler.program.closure_function_layouts.len(),
        published_layout_count: compiler
            .program
            .closure_function_layouts
            .iter()
            .flatten()
            .count(),
    }
}

fn alloc_shared_module_binding_count(compiler: &BytecodeCompiler) -> usize {
    compiler
        .program
        .instructions
        .iter()
        .filter(|instruction| instruction.opcode == OpCode::AllocSharedModuleBinding)
        .count()
}

#[test]
fn callable_unpromoted_module_shared_capture_is_c0912_before_publication() {
    let baseline_source = "var hits = 0\n";
    let baseline_program = shape_ast::parse_program(baseline_source).expect("baseline parses");
    let mut baseline = BytecodeCompiler::new();
    baseline.set_source_with_file(baseline_source, "capture_preflight.shape");
    baseline
        .compile_in_place(&baseline_program)
        .expect("module declaration baseline compiles");
    let baseline_slot = *baseline
        .module_bindings
        .get("hits")
        .expect("baseline module slot exists");
    let baseline_semantics = *baseline
        .type_tracker
        .get_binding_semantics(baseline_slot)
        .expect("baseline module semantics exist");
    let baseline_publication = closure_publication_state(&baseline);
    let baseline_allocations = alloc_shared_module_binding_count(&baseline);

    let source = "var hits = 0\nfn run() -> int {\n  let f = |x: int| { hits = hits + x\n    hits }\n  f(3)\n}\n";
    let program = shape_ast::parse_program(source).expect("negative fixture parses");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source_with_file(source, "capture_preflight.shape");
    let error = compiler
        .compile_in_place(&program)
        .expect_err("callable cannot introduce a module SharedCell");

    let slot = *compiler
        .module_bindings
        .get("hits")
        .expect("negative fixture module slot exists");
    assert_eq!(slot, baseline_slot);
    match error {
        shape_ast::error::ShapeError::SemanticError { message, location } => {
            assert!(message.starts_with(&format!(
                "[C0912] exact reference-flow conflict at callable closure capture for \
                 ModuleBinding({slot}) (name 'hits')"
            )));
            assert!(message.contains("Value [storage=Direct]"));
            assert!(message.contains("shared-module promotion witness is absent"));
            let location = location.expect("module declaration is the diagnostic anchor");
            assert_eq!(location.file.as_deref(), Some("capture_preflight.shape"));
            assert_eq!(location.line, 1);
        }
        other => panic!("expected semantic C0912, got {other:?}"),
    }

    assert_eq!(
        compiler.type_tracker.get_binding_semantics(slot),
        Some(&baseline_semantics),
        "the rejected callable must not change module binding semantics"
    );
    assert_eq!(
        compiler.shared_module_bindings,
        baseline.shared_module_bindings,
        "no module promotion witness may be published"
    );
    assert_eq!(
        alloc_shared_module_binding_count(&compiler),
        baseline_allocations,
        "no module promotion opcode may be emitted"
    );
    assert_eq!(
        closure_publication_state(&compiler),
        baseline_publication,
        "the preflight must run before closure functions, layouts, packs, ids, or counter state"
    );
}

#[test]
fn callable_module_snapshot_capture_needs_no_promotion_effect() {
    let compiler = compile(
        r#"
let base = 10
fn run() -> int {
  let add = |x: int| x + base
  add(2)
}
"#,
    );

    assert_eq!(compiler.closure_capture_packs.len(), 1);
    let descriptor = &compiler.closure_capture_packs[0].descriptors[0];
    assert_eq!(descriptor.name, "base");
    assert!(matches!(
        descriptor.target,
        Some(CaptureTarget::ModuleBinding(_))
    ));
    assert_eq!(descriptor.access, CaptureAccess::Param);
    assert!(compiler.shared_module_bindings.is_empty());
    assert_eq!(alloc_shared_module_binding_count(&compiler), 0);
}

#[test]
fn callable_reuses_already_promoted_module_cell_without_a_second_effect() {
    let compiler = compile(
        r#"
var hits = 0
let promote = |x: int| { hits = hits + x
  hits }
fn run() -> int {
  let reuse = |x: int| { hits = hits + x
    hits }
  reuse(2)
}
"#,
    );

    assert_eq!(compiler.closure_capture_packs.len(), 2);
    let reused = &compiler.closure_capture_packs[1].descriptors[0];
    assert_eq!(reused.name, "hits");
    assert!(matches!(
        reused.target,
        Some(CaptureTarget::ModuleBinding(_))
    ));
    assert_eq!(reused.access, CaptureAccess::SharedCell);
    assert_eq!(reused.storage, Some(BindingStorageClass::SharedCow));
    assert!(!compiler.shared_module_bindings.is_empty());
    assert_eq!(
        alloc_shared_module_binding_count(&compiler),
        1,
        "only the top-level closure may publish the module cell"
    );
}
