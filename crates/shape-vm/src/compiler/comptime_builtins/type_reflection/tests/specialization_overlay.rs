//! Declared generic-parameter identity across specialized-body compilation.

use super::*;
use crate::compiler::comptime_builtins::semantic_freeze::SpecializationTypeOverlay;

#[test]
fn specialized_generic_body_resolves_own_type_param_through_comptime() {
    let control = r#"
let control_label = comptime {
  match type_category(type_ref(int)) {
    FrozenTypeCategory::Primitive => "primitive"
    _ => "other"
  }
}
"#;
    let control_program = shape_ast::parse_program(control).expect("control parses");
    let control_compiler = BytecodeCompiler::new();
    control_compiler
        .compile(&control_program)
        .expect("module-scope comptime reflection compiles with a bare compiler");

    let source = r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(T)) {
      FrozenTypeCategory::Parameter => "parameter"
      _ => "other"
    }
  }
  label
}

let result = describe(1)
"#;
    let program = shape_ast::parse_program(source).expect("generic program parses");
    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("specialized generic body with type_ref(T) compiles");
    assert!(
        bytecode
            .functions
            .iter()
            .any(|function| function.name.starts_with("describe::")),
        "expected a registered specialization of 'describe', got: {:?}",
        bytecode
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn specialization_overlay_supplies_base_scoped_parameter_identity() {
    let program = shape_ast::parse_program("fn describe(value: int) -> int { value }")
        .expect("mono-shaped function parses");
    let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
        panic!("expected function item");
    };
    let mut mono_def = definition.clone();
    mono_def.name = "describe::i64".to_string();
    assert!(mono_def.type_params.is_none());
    let mut compiler = BytecodeCompiler::new();
    compiler
        .register_function(&mono_def)
        .expect("mono def registers");
    compiler.current_function = compiler
        .program
        .functions
        .iter()
        .position(|function| function.name == "describe::i64");
    compiler
        .install_semantic_freeze()
        .expect("registration-complete state freezes");

    let bare = compiler
        .comptime_freeze_overlay()
        .expect("post-barrier site obtains the handle");
    assert_eq!(bare.identity_of("T"), None);

    let _specialization =
        compiler
            .specialization_type_overlays
            .enter(SpecializationTypeOverlay::declaration_only(
                "describe",
                vec!["T".to_string()],
            ));
    let overlay = compiler
        .comptime_freeze_overlay()
        .expect("post-barrier site obtains the handle");
    let identity = overlay.identity_of("T").expect("overlay T identity");
    assert_eq!(
        overlay.category_of(identity),
        Ok(FrozenTypeCategory::Parameter)
    );
    assert_eq!(
        identity,
        FrozenTypeIdentity::from_canonical_descriptor("parameter:describe:T"),
        "overlay Parameter identity must be scoped to the BASE fn name"
    );
    assert_ne!(
        identity,
        FrozenTypeIdentity::from_canonical_descriptor("parameter:describe::i64:T"),
        "overlay Parameter identity must NOT be scoped to the mono key"
    );
}

#[test]
fn specialization_overlay_identity_is_stable_across_instantiations() {
    let overlay_identity = |mono_name: &str, base_name: &str| {
        let program = shape_ast::parse_program("fn placeholder(value: int) -> int { value }")
            .expect("mono-shaped function parses");
        let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
            panic!("expected function item");
        };
        let mut mono_def = definition.clone();
        mono_def.name = mono_name.to_string();
        assert!(mono_def.type_params.is_none());
        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(&mono_def)
            .expect("mono def registers");
        compiler.current_function = compiler
            .program
            .functions
            .iter()
            .position(|function| function.name == mono_name);
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");
        let _specialization = compiler.specialization_type_overlays.enter(
            SpecializationTypeOverlay::declaration_only(base_name, vec!["T".to_string()]),
        );
        let overlay = compiler
            .comptime_freeze_overlay()
            .expect("post-barrier site obtains the handle");
        overlay.identity_of("T").expect("overlay T identity")
    };

    assert_eq!(
        overlay_identity("identity::i64", "identity"),
        overlay_identity("identity::string", "identity"),
        "Parameter identity must be stable across instantiations of one base fn"
    );
    assert_ne!(
        overlay_identity("identity::i64", "identity"),
        overlay_identity("filter::i64", "filter"),
        "Parameter identities must be distinct across owning functions"
    );
}
