//! Pre-emission Shared-kind gates for both top-level JIT strategy routes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use shape_ast::ast::Span;
use shape_value::v2::ConcreteType;
use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
use shape_value::{HeapKind, NativeKind};
use shape_vm::bytecode::{BytecodeProgram, MirFunctionData};
use shape_vm::mir::{
    BasicBlock, BasicBlockId, BorrowAnalysis, LocalTypeInfo, MirFunction, MirStatement, Operand,
    Place, Point, SlotId, StatementKind, StoragePlan, Terminator, TerminatorKind,
};

use super::JITCompiler;
use crate::JITConfig;

fn forged_native_scalar_shared_program() -> BytecodeProgram {
    let captured_slot = SlotId(1);
    let mir = MirFunction {
        name: "__main__".to_string(),
        blocks: vec![BasicBlock {
            id: BasicBlockId(0),
            statements: vec![MirStatement {
                kind: StatementKind::ClosureCapture {
                    closure_slot: SlotId(2),
                    operands: vec![Operand::Copy(Place::Local(captured_slot))],
                    function_id: Some(0),
                },
                span: Span::DUMMY,
                point: Point(0),
            }],
            terminator: Terminator {
                kind: TerminatorKind::Return,
                span: Span::DUMMY,
            },
        }],
        num_locals: 3,
        param_slots: Vec::new(),
        param_reference_kinds: Vec::new(),
        local_types: vec![LocalTypeInfo::Unknown; 3],
        span: Span::DUMMY,
        field_name_table: HashMap::new(),
        local_struct_type_names: HashMap::new(),
        local_typed_array_element_types: HashMap::new(),
        local_declared_scalar_types: HashMap::new(),
        binding_slots: HashMap::new(),
        var_binding_slots: HashSet::new(),
    };
    let storage_plan = StoragePlan {
        slot_classes: HashMap::new(),
        slot_semantics: HashMap::new(),
        inline_array_sizes: HashMap::new(),
        non_escaping_closure_slots: HashSet::new(),
        reference_escape_promotion_slots: HashSet::new(),
    };

    // Start from a well-formed layout, then model corrupt/external metadata
    // arriving after publication. Source and generated issuers cannot create
    // this kind, but both JIT entry routes must still reject it defensively.
    let mut forged_layout =
        ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::Shared]);
    forged_layout.capture_native_kinds[0] = NativeKind::Ptr(HeapKind::NativeScalar);

    BytecodeProgram {
        top_level_mir: Some(Arc::new(MirFunctionData {
            mir,
            storage_plan,
            borrow_analysis: BorrowAnalysis::empty(),
        })),
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
