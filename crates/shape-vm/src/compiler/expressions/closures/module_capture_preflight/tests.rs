use std::collections::{BTreeSet, HashSet};

use shape_ast::ast::{Expr, Item, Span};
use shape_ast::error::ShapeError;

use super::*;
use crate::bytecode::{Function, OpCode};
use crate::compiler::comptime_builtins::capture_plan::CaptureAccess;
use crate::type_tracking::{
    BindingOwnershipClass, BindingSemantics, BindingStorageClass,
};

const MODULE_SLOT: u16 = 7;
const BINDING_SPAN: Span = Span { start: 4, end: 8 };
const CLOSURE_SPAN: Span = Span { start: 16, end: 24 };

#[derive(Debug, PartialEq, Eq)]
struct PublicationState {
    closure_counter: u64,
    closure_registry_len: usize,
    function_type_registry_len: usize,
    capture_pack_len: usize,
    closure_function_id_len: usize,
    closure_type_id_len: usize,
    function_type_id_len: usize,
    capture_name_len: usize,
    function_len: usize,
    closure_function_count: usize,
    layout_len: usize,
    published_layout_count: usize,
    instruction_len: usize,
    promotion_opcode_count: usize,
    shared_module_bindings: HashSet<String>,
    module_semantics: Option<BindingSemantics>,
    current_function: Option<usize>,
    current_callee_captures: BTreeSet<String>,
}

fn publication_state(compiler: &BytecodeCompiler) -> PublicationState {
    PublicationState {
        closure_counter: compiler.closure_counter,
        closure_registry_len: compiler.closure_registry.len(),
        function_type_registry_len: compiler.function_type_registry.len(),
        capture_pack_len: compiler.closure_capture_packs.len(),
        closure_function_id_len: compiler.closure_function_ids.len(),
        closure_type_id_len: compiler.closure_type_ids.len(),
        function_type_id_len: compiler.function_type_ids.len(),
        capture_name_len: compiler.closure_capture_names.len(),
        function_len: compiler.program.functions.len(),
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
        instruction_len: compiler.program.instructions.len(),
        promotion_opcode_count: compiler
            .program
            .instructions
            .iter()
            .filter(|instruction| instruction.opcode == OpCode::AllocSharedModuleBinding)
            .count(),
        shared_module_bindings: compiler.shared_module_bindings.clone(),
        module_semantics: compiler
            .type_tracker
            .get_binding_semantics(MODULE_SLOT)
            .copied(),
        current_function: compiler.current_function,
        current_callee_captures: compiler.current_closure_callee_captures.clone(),
    }
}

fn active_callable(
    callable_name: &str,
    storage: BindingStorageClass,
    has_shared_witness: bool,
) -> BytecodeCompiler {
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source_with_file("var hits = 0\nlet f = || hits\n", "preflight-unit.shape");
    compiler
        .module_bindings
        .insert("hits".to_string(), MODULE_SLOT);
    compiler
        .module_binding_spans
        .insert(MODULE_SLOT, BINDING_SPAN);
    let mut semantics = BindingSemantics::deferred(BindingOwnershipClass::Flexible);
    semantics.storage_class = storage;
    compiler
        .type_tracker
        .set_binding_semantics(MODULE_SLOT, semantics);
    if has_shared_witness {
        compiler.shared_module_bindings.insert("hits".to_string());
    }
    compiler.program.functions.push(Function {
        name: callable_name.to_string(),
        arity: 0,
        param_names: Vec::new(),
        locals_count: 0,
        entry_point: 0,
        body_length: 0,
        is_closure: false,
        captures_count: 0,
        is_async: false,
        ref_params: Vec::new(),
        ref_mutates: Vec::new(),
        mutable_captures: Vec::new(),
        frame_descriptor: None,
        osr_entry_points: Vec::new(),
        mir_data: None,
    });
    compiler.current_function = Some(0);
    compiler
}

fn canonical_shared_module_plan(compiler: &BytecodeCompiler) -> Vec<PlannedCapture> {
    let captured = vec!["hits".to_string()];
    let mutated = HashSet::from(["hits".to_string()]);
    let plan = compiler
        .plan_captures(
            &captured,
            &mutated,
            None,
            None,
            None,
            CLOSURE_SPAN,
        )
        .expect("canonical module capture plan");
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].plan.access(), CaptureAccess::SharedCell);
    assert_eq!(
        plan[0].facts.target,
        Some(CaptureTarget::ModuleBinding(MODULE_SLOT))
    );
    plan
}

fn semantic_message(error: ShapeError) -> String {
    match error {
        ShapeError::SemanticError { message, location } => {
            let location = location.expect("binding declaration anchors C0912");
            assert_eq!(location.file.as_deref(), Some("preflight-unit.shape"));
            assert_eq!(location.line, 1);
            assert!(message.contains("ModuleBinding(7) (name 'hits')"));
            message
        }
        other => panic!("expected semantic C0912, got {other:?}"),
    }
}

fn closure_parts(
    source: &str,
) -> (
    Vec<shape_ast::ast::FunctionParameter>,
    Vec<shape_ast::ast::Statement>,
    Span,
) {
    let program = shape_ast::parse_program(source).expect("closure fixture parses");
    let Item::VariableDecl(decl, _) = program.items.into_iter().next().expect("one declaration")
    else {
        panic!("fixture must be a variable declaration");
    };
    let Some(Expr::FunctionExpr {
        params, body, span, ..
    }) = decl.value
    else {
        panic!("fixture initializer must be a closure");
    };
    (params, body, span)
}

#[test]
fn shared_cow_without_promotion_witness_rejects_without_publication() {
    let compiler = active_callable("risk_model", BindingStorageClass::SharedCow, false);
    let plan = canonical_shared_module_plan(&compiler);
    let before = publication_state(&compiler);

    let error = compiler
        .preflight_callable_module_shared_captures(&plan, CLOSURE_SPAN)
        .expect_err("SharedCow storage alone is not a published cell");
    let message = semantic_message(error);
    assert!(message.contains("callable 'risk_model' closure capture"));
    assert!(message.contains("Value [storage=SharedCow]"));
    assert!(message.contains("shared-module promotion witness is absent"));
    assert_eq!(publication_state(&compiler), before);
}

#[test]
fn promotion_witness_without_shared_cow_storage_rejects_without_publication() {
    let compiler = active_callable("risk_model", BindingStorageClass::Direct, true);
    let plan = canonical_shared_module_plan(&compiler);
    let before = publication_state(&compiler);

    let error = compiler
        .preflight_callable_module_shared_captures(&plan, CLOSURE_SPAN)
        .expect_err("a witness cannot substitute for SharedCow storage");
    let message = semantic_message(error);
    assert!(message.contains("callable 'risk_model' closure capture"));
    assert!(message.contains("Value [storage=Direct]"));
    assert!(message.contains("shared-module promotion witness is present"));
    assert_eq!(publication_state(&compiler), before);
}

#[test]
fn active_callable_peek_declines_before_registry_layout_or_id_publication() {
    let mut compiler = active_callable("peek_owner", BindingStorageClass::Direct, false);
    let (params, body, span) =
        closure_parts("let worker = || { hits = hits + 1\n hits }");
    let plan = canonical_shared_module_plan(&compiler);
    assert!(compiler
        .preflight_callable_module_shared_captures(&plan, span)
        .is_err());
    let before = publication_state(&compiler);

    assert!(compiler
        .mint_closure_type_id_peek(&params, &body, None, None, span)
        .is_none());
    assert_eq!(
        publication_state(&compiler),
        before,
        "a rejected peek must not intern a registry key or publish layout/id state"
    );
}

#[test]
fn generated_named_callable_uses_the_same_structural_preflight() {
    // Generated extend methods cannot yet resolve module bindings through the
    // real ingress. This focused unit therefore supplies only the stable name
    // that such a method owns; the predicate itself remains provenance-free
    // and consumes the same canonical structural capture plan as source code.
    let compiler = active_callable("Job::read", BindingStorageClass::Direct, false);
    let plan = canonical_shared_module_plan(&compiler);
    let before = publication_state(&compiler);

    let error = compiler
        .preflight_callable_module_shared_captures(&plan, CLOSURE_SPAN)
        .expect_err("generated-named callable cannot introduce module storage");
    let message = semantic_message(error);
    assert!(message.contains("callable 'Job::read' closure capture"));
    assert!(message.contains("ModuleBinding(7) (name 'hits')"));
    assert_eq!(publication_state(&compiler), before);
}
