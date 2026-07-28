//! Pre-emission Shared-kind gates for both top-level JIT strategy routes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use shape_ast::ast::Span;
use shape_value::v2::ConcreteType;
use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
use shape_value::{HeapKind, NativeKind};
use shape_vm::bytecode::{BytecodeProgram, Function, MirFunctionData};
use shape_vm::mir::{
    BasicBlock, BasicBlockId, BorrowAnalysis, LocalTypeInfo, MirFunction, MirStatement, Operand,
    Place, Point, SlotId, StatementKind, StoragePlan, Terminator, TerminatorKind,
};

use super::JITCompiler;
use crate::JITConfig;

fn return_only_mir_data(
    name: &str,
    num_locals: u16,
    param_slots: Vec<SlotId>,
    statements: Vec<MirStatement>,
) -> Arc<MirFunctionData> {
    let param_reference_kinds = vec![None; param_slots.len()];
    Arc::new(MirFunctionData {
        mir: MirFunction {
            name: name.to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements,
                terminator: Terminator {
                    kind: TerminatorKind::Return,
                    span: Span::DUMMY,
                },
            }],
            num_locals,
            param_slots,
            param_reference_kinds,
            local_types: vec![LocalTypeInfo::Unknown; usize::from(num_locals)],
            span: Span::DUMMY,
            field_name_table: HashMap::new(),
            local_struct_type_names: HashMap::new(),
            local_typed_array_element_types: HashMap::new(),
            local_declared_scalar_types: HashMap::new(),
            binding_slots: HashSet::new(),
            var_binding_slots: HashSet::new(),
        },
        storage_plan: StoragePlan {
            slot_classes: HashMap::new(),
            slot_semantics: HashMap::new(),
            inline_array_sizes: HashMap::new(),
            non_escaping_closure_slots: HashSet::new(),
            reference_escape_promotion_slots: HashSet::new(),
            escape: Default::default(),
        },
        borrow_analysis: BorrowAnalysis::empty(),
    })
}

fn forged_native_scalar_shared_program() -> BytecodeProgram {
    let captured_slot = SlotId(1);
    let mir_data = return_only_mir_data(
        "__main__",
        3,
        Vec::new(),
        vec![MirStatement {
            kind: StatementKind::ClosureCapture {
                closure_slot: SlotId(2),
                operands: vec![Operand::Copy(Place::Local(captured_slot))],
                function_id: Some(0),
            },
            span: Span::DUMMY,
            point: Point(0),
        }],
    );

    // Start from a well-formed layout, then model corrupt/external metadata
    // arriving after publication. Source and generated issuers cannot create
    // this kind, but both JIT entry routes must still reject it defensively.
    let mut forged_layout =
        ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::Shared]);
    forged_layout.capture_native_kinds[0] = NativeKind::Ptr(HeapKind::NativeScalar);

    BytecodeProgram {
        top_level_mir: Some(mir_data),
        closure_function_layouts: vec![Some(Arc::new(forged_layout))],
        ..Default::default()
    }
}

fn forged_native_scalar_inherited_capture_program() -> BytecodeProgram {
    let capture_slot = SlotId(1);
    let closure_mir = return_only_mir_data("__closure_0", 2, vec![capture_slot], Vec::new());
    let mut forged_layout =
        ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::Shared]);
    forged_layout.capture_native_kinds[0] = NativeKind::Ptr(HeapKind::NativeScalar);

    BytecodeProgram {
        functions: vec![Function {
            name: "__closure_0".to_string(),
            arity: 1,
            param_names: vec!["captured".to_string()],
            locals_count: 2,
            entry_point: 0,
            body_length: 0,
            is_closure: true,
            captures_count: 1,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: vec![true],
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: Some(closure_mir),
        }],
        top_level_mir: Some(return_only_mir_data("__main__", 1, Vec::new(), Vec::new())),
        closure_function_layouts: vec![Some(Arc::new(forged_layout))],
        ..Default::default()
    }
}

fn assert_pre_emission_native_scalar_refusal(error: &str) {
    assert!(
        error.contains("Ptr(NativeScalar)"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("rejected before JIT emission or allocation"),
        "refusal must identify the pre-emission boundary: {error}"
    );
}

#[test]
fn direct_strategy_route_rejects_forged_shared_kind_before_publication() {
    let program = forged_native_scalar_shared_program();
    let mut compiler = JITCompiler::new(JITConfig::default()).expect("JIT setup must succeed");
    let symbol = "forged_shared_direct";

    let error = compiler
        .compile_strategy(symbol, &program)
        .expect_err("the direct strategy route must reject forged Shared metadata");

    assert_pre_emission_native_scalar_refusal(&error);
    assert!(!compiler.compiled_functions.contains_key(symbol));
}

#[test]
fn user_function_strategy_route_rejects_forged_shared_kind_before_publication() {
    let program = forged_native_scalar_shared_program();
    let mut compiler = JITCompiler::new(JITConfig::default()).expect("JIT setup must succeed");
    let symbol = "forged_shared_with_user_funcs";

    let error = compiler
        .compile_strategy_with_user_funcs(
            symbol,
            &program,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect_err("the user-function strategy route must reject forged Shared metadata");

    assert_pre_emission_native_scalar_refusal(&error);
    assert!(!compiler.compiled_functions.contains_key(symbol));
}

#[test]
fn user_function_body_rejects_forged_inherited_shared_capture_before_publication() {
    let program = forged_native_scalar_inherited_capture_program();
    let closure_mir = program.functions[0]
        .mir_data
        .as_ref()
        .expect("the forged closure body must carry MIR");
    assert_eq!(closure_mir.mir.param_slots, [SlotId(1)]);
    assert!(closure_mir.storage_plan.slot_classes.is_empty());
    assert!(closure_mir.mir.blocks.iter().all(|block| {
        block
            .statements
            .iter()
            .all(|statement| !matches!(statement.kind, StatementKind::ClosureCapture { .. }))
    }));

    let mut compiler = JITCompiler::new(JITConfig::default()).expect("JIT setup must succeed");
    let error = compiler
        .compile_program("forged_inherited_shared", &program)
        .expect_err("the closure-body route must reject inherited Shared metadata");

    assert_eq!(
        error,
        "INTERNAL SharedCell kind invariant: inherited Shared capture parameter slot _1 \
         resolved to unsupported Ptr(NativeScalar). The exhaustive ConcreteType capture-kind \
         issuer cannot produce a carrier-less kind; forged or external layout metadata must \
         be rejected before JIT emission or allocation."
    );
    assert!(compiler.compiled_functions.is_empty());
}
