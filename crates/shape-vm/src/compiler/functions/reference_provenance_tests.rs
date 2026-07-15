use super::*;

use shape_ast::ast::{CaptureMode, DestructurePattern, Span};

use crate::type_tracking::BindingStorageClass;

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

#[derive(Clone, Copy)]
enum AnnotationRoute {
    SingleRuntime,
    ChainedRuntime,
    ReplaceBody,
}

impl AnnotationRoute {
    fn source(self, parameter: &str) -> String {
        let source = match self {
            Self::SingleRuntime => {
                r#"
annotation first() {
  targets: [function]
  before(args, ctx) { args }
}

@first()
fn probe($PARAM) -> int {
  let worker = |x: int; move value| x + value
  worker(1)
}
"#
            }
            Self::ChainedRuntime => {
                r#"
annotation first() {
  targets: [function]
  before(args, ctx) { args }
}

annotation second() {
  targets: [function]
  before(args, ctx) { args }
}

@first()
@second()
fn probe($PARAM) -> int {
  let worker = |x: int; move value| x + value
  worker(1)
}
"#
            }
            Self::ReplaceBody => {
                r#"
annotation replace_with_original() {
  targets: [function]
  comptime post(target, ctx) {
    replace body {
      return ctx.original(value)
    }
  }
}

@replace_with_original()
fn probe($PARAM) -> int {
  let worker = |x: int; move value| x + value
  worker(1)
}
"#
            }
        };
        source.replace("$PARAM", parameter)
    }
}

fn compile_stamped_probe(
    route: AnnotationRoute,
    parameter: &str,
    mode: ParamPassMode,
) -> (BytecodeCompiler, std::result::Result<(), String>) {
    let mut program = shape_ast::parse_program(&route.source(parameter)).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();

    for item in &program.items {
        if matches!(item, Item::AnnotationDef(..)) {
            compiler
                .compile_item_with_context(item, false)
                .expect("annotation definition compiles through the real registration path");
        }
    }

    let root = compiler.generated_node_issuer.issue(
        shape_ast::ast::GeneratedExpansionFingerprint::from_components(17, 29),
        shape_ast::ast::GeneratedNodePath::decl_root("fn:probe"),
        0,
        Span::DUMMY,
        "probe".to_string(),
    );
    let probe = program
        .items
        .iter_mut()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == "probe" => Some(function),
            _ => None,
        })
        .expect("fixture has probe");
    shape_ast::transform::stamp_generated_closures(&mut probe.body, &root);

    let facts = BytecodeCompiler::infer_reference_model(&program).3;
    compiler.resolved_expr_types = facts.expression_types().clone();
    compiler.inference_facts = facts;
    compiler
        .inferred_param_pass_modes
        .insert("probe".to_string(), vec![mode]);

    let probe = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == "probe" => Some(function.clone()),
            _ => None,
        })
        .expect("fixture has probe");
    compiler
        .register_function(&probe)
        .expect("probe registers with the injected pass mode");
    compiler
        .install_semantic_freeze()
        .expect("registration-complete fixture freezes");
    let outcome = compiler
        .compile_function(&probe)
        .map_err(|error| error.to_string());
    (compiler, outcome)
}

fn assert_inferred_reference_is_not_true_reference(
    route: AnnotationRoute,
    mode: ParamPassMode,
) {
    let (compiler, outcome) = compile_stamped_probe(route, "value: int", mode);
    outcome.expect("an inferred reference optimization is not a true reference capture");

    let descriptors: Vec<_> = compiler
        .closure_capture_packs
        .iter()
        .flat_map(|pack| &pack.descriptors)
        .filter(|descriptor| descriptor.name == "value")
        .collect();
    assert_eq!(descriptors.len(), 1, "fixture has one value capture");
    assert_eq!(descriptors[0].declared, Some(CaptureMode::Move));
    assert_ne!(
        descriptors[0].storage,
        Some(BindingStorageClass::Reference),
        "inferred pass mode must not become true-reference evidence"
    );
}

fn assert_explicit_reference_is_c0902(
    route: AnnotationRoute,
    parameter: &str,
    mode: ParamPassMode,
) {
    let (_, outcome) = compile_stamped_probe(route, parameter, mode);
    let error = outcome.expect_err("an explicit reference must remain true-reference evidence");
    assert!(
        error.contains(
            "[C0902] ReferenceEscapeIntoClosure: declared capture 'move value' carries reference binding 'value'"
        ),
        "explicit reference control must use exact C0902: {error}"
    );
}

#[test]
fn single_runtime_annotation_preserves_shared_reference_provenance() {
    assert_inferred_reference_is_not_true_reference(
        AnnotationRoute::SingleRuntime,
        ParamPassMode::ByRefShared,
    );
    assert_explicit_reference_is_c0902(
        AnnotationRoute::SingleRuntime,
        "&value: int",
        ParamPassMode::ByRefShared,
    );
}

#[test]
fn chained_runtime_annotations_preserve_exclusive_reference_provenance() {
    assert_inferred_reference_is_not_true_reference(
        AnnotationRoute::ChainedRuntime,
        ParamPassMode::ByRefExclusive,
    );
    assert_explicit_reference_is_c0902(
        AnnotationRoute::ChainedRuntime,
        "&mut value: int",
        ParamPassMode::ByRefExclusive,
    );
}

#[test]
fn replace_body_ctx_original_preserves_inferred_reference_provenance() {
    assert_inferred_reference_is_not_true_reference(
        AnnotationRoute::ReplaceBody,
        ParamPassMode::ByRefShared,
    );
}
