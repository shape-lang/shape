use super::*;

use shape_ast::ast::{DestructurePattern, Span};

fn parameter(is_reference: bool, is_mut_reference: bool) -> FunctionParameter {
    FunctionParameter {
        pattern: DestructurePattern::Identifier("value".to_string(), Span::DUMMY),
        is_const: false,
        is_reference,
        is_mut_reference,
        is_out: false,
        type_annotation: None,
        default_value: None,
    }
}

#[test]
fn original_flags_distinguish_inferred_and_explicit_reference_modes_by_slot() {
    let params = [
        parameter(false, false),
        parameter(true, false),
        parameter(false, false),
        parameter(true, true),
        parameter(false, false),
    ];
    let modes = [
        ParamPassMode::ByRefShared,
        ParamPassMode::ByRefShared,
        ParamPassMode::ByRefExclusive,
        ParamPassMode::ByRefExclusive,
        ParamPassMode::ByValue,
    ];

    assert_eq!(
        BytecodeCompiler::inferred_reference_optimizations(&params, &modes),
        vec![
            Some(ParamPassMode::ByRefShared),
            None,
            Some(ParamPassMode::ByRefExclusive),
            None,
            None,
        ]
    );
}

#[test]
#[should_panic(expected = "parameter provenance must stay slot-aligned")]
fn missing_effective_mode_is_a_structural_error() {
    let params = [parameter(false, false), parameter(false, false)];

    let _ = BytecodeCompiler::inferred_reference_optimizations(
        &params,
        &[ParamPassMode::ByRefShared],
    );
}
